use lru::LruCache;
use napi::{
    bindgen_prelude::{Env, JsObjectValue, Object, Unknown},
    Result, UnknownRef, ValueType,
};
use std::{
    borrow::Borrow,
    collections::HashSet,
    ffi::{c_char, CString},
    hash::{Hash, Hasher},
    num::NonZeroUsize,
    ptr::NonNull,
    slice,
};

// Cap cached NUL-terminated map keys by both entry count and key length so
// databases with high-cardinality or oversized dynamic keys cannot grow the
// cache without bound (worst case 4,096 * 64 bytes = 256 KiB per reader/cursor).
// Keys beyond these bounds fall back to allocating a JS string directly.
const MAX_PROPERTY_NAME_CACHE_ENTRIES: usize = 4_096;
const MAX_PROPERTY_NAME_KEY_BYTES: usize = 64;

// Use a two-segment SLRU (20% protected, 80% probationary) for smaller caches
// (including the default 10,000-entry cache) to resist scan pollution. Larger
// caches already absorb working-set churn and avoid the extra segment lookup.
const SLRU_MAX_CAPACITY: usize = 10_000;

pub(crate) struct RecordCache {
    probationary: LruCache<usize, UnknownRef>,
    protected: Option<LruCache<usize, UnknownRef>>,
    capacity: NonZeroUsize,
    pub(crate) hits: u64,
    pub(crate) misses: u64,
    pub(crate) inserts: u64,
    pub(crate) evictions: u64,
}

// Owns a NUL-terminated byte buffer detached from `Box`/`CString` via
// `CString::into_raw`. Moving this handle (e.g. during HashSet insertion or
// rehashing) never retags or invalidates pointers previously returned by
// `as_ptr()` while the entry remains in the cache.
struct CachedCString {
    ptr: NonNull<c_char>,
    len: usize,
}

impl CachedCString {
    fn new(bytes: &[u8]) -> Option<Self> {
        let cstring = CString::new(bytes).ok()?;
        let len = cstring.as_bytes().len();
        let ptr = NonNull::new(cstring.into_raw())?;
        Some(Self { ptr, len })
    }

    fn as_bytes(&self) -> &[u8] {
        // SAFETY: `self.ptr` points to `self.len` initialized bytes followed by
        // a NUL byte from `CString::into_raw` and remains immutable until Drop.
        unsafe { slice::from_raw_parts(self.ptr.as_ptr().cast::<u8>(), self.len) }
    }

    fn as_ptr(&self) -> *const c_char {
        self.ptr.as_ptr()
    }
}

impl Drop for CachedCString {
    fn drop(&mut self) {
        // SAFETY: `self.ptr` was created by `CString::into_raw` and is
        // reclaimed exactly once here.
        unsafe {
            drop(CString::from_raw(self.ptr.as_ptr()));
        }
    }
}

impl Borrow<[u8]> for CachedCString {
    fn borrow(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl PartialEq for CachedCString {
    fn eq(&self, other: &Self) -> bool {
        self.as_bytes() == other.as_bytes()
    }
}

impl Eq for CachedCString {}

impl Hash for CachedCString {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_bytes().hash(state);
    }
}

pub(crate) struct PropertyNameCache {
    values: HashSet<CachedCString>,
}

impl PropertyNameCache {
    pub(crate) fn new() -> Self {
        Self {
            values: HashSet::new(),
        }
    }

    pub(crate) fn get(&mut self, name: &[u8]) -> Option<*const c_char> {
        if name.len() > MAX_PROPERTY_NAME_KEY_BYTES {
            return None;
        }

        if let Some(cached) = self.values.get(name) {
            return Some(cached.as_ptr());
        }

        if self.values.len() >= MAX_PROPERTY_NAME_CACHE_ENTRIES {
            return None;
        }

        let cached = CachedCString::new(name)?;
        let pointer = cached.as_ptr();
        self.values.insert(cached);
        Some(pointer)
    }

    pub(crate) fn clear(&mut self) {
        self.values.clear();
    }
}

impl RecordCache {
    pub(crate) fn new(capacity: NonZeroUsize) -> Self {
        let protected_capacity = if capacity.get() <= SLRU_MAX_CAPACITY {
            NonZeroUsize::new(capacity.get() / 5)
        } else {
            None
        };
        let probationary_capacity = NonZeroUsize::new(
            capacity.get() - protected_capacity.map(NonZeroUsize::get).unwrap_or(0),
        )
        .unwrap();
        Self {
            probationary: LruCache::new(probationary_capacity),
            protected: protected_capacity.map(LruCache::new),
            capacity,
            hits: 0,
            misses: 0,
            inserts: 0,
            evictions: 0,
        }
    }

    pub(crate) fn get<'env>(
        &mut self,
        env: &'env Env,
        offset: usize,
    ) -> Result<Option<Unknown<'env>>> {
        if let Some(protected) = self.protected.as_mut() {
            if let Some(value) = protected.get(&offset) {
                self.hits += 1;
                return value.get_value(env).map(Some);
            }

            if let Some(value) = self.probationary.peek(&offset) {
                let result = value.get_value(env)?;
                if let Some(value) = self.probationary.pop(&offset) {
                    if let Some((demoted_offset, demoted)) = protected.push(offset, value) {
                        debug_assert_ne!(demoted_offset, offset);
                        let evicted = self.probationary.push(demoted_offset, demoted);
                        debug_assert!(evicted.is_none());
                    }
                }
                self.hits += 1;
                return Ok(Some(result));
            }
        } else if let Some(value) = self.probationary.get(&offset) {
            self.hits += 1;
            return value.get_value(env).map(Some);
        }

        self.misses += 1;
        Ok(None)
    }

    pub(crate) fn put(&mut self, env: &Env, offset: usize, value: &Unknown<'_>) -> Result<()> {
        if value.get_type()? != ValueType::Object {
            return Ok(());
        }

        let reference = value.create_ref()?;
        self.inserts += 1;
        if let Some((old_offset, old_reference)) = self.probationary.push(offset, reference) {
            if old_offset != offset {
                self.evictions += 1;
            }
            old_reference.unref(env)?;
        }
        Ok(())
    }

    pub(crate) fn clear(&mut self, env: &Env) -> Result<()> {
        while let Some((_offset, reference)) = self.probationary.pop_lru() {
            reference.unref(env)?;
        }
        if let Some(protected) = self.protected.as_mut() {
            while let Some((_offset, reference)) = protected.pop_lru() {
                reference.unref(env)?;
            }
        }
        Ok(())
    }

    fn len(&self) -> usize {
        self.probationary.len() + self.protected.as_ref().map(LruCache::len).unwrap_or(0)
    }
}

pub(crate) fn cache_stats_to_js<'env>(
    env: &'env Env,
    cache: &Option<RecordCache>,
) -> Result<Object<'env>> {
    let mut object = Object::new(env)?;

    if let Some(cache) = cache.as_ref() {
        object.set_named_property("enabled", true)?;
        object.set_named_property("size", cache.len() as f64)?;
        object.set_named_property("capacity", cache.capacity.get() as f64)?;
        object.set_named_property("hits", cache.hits as f64)?;
        object.set_named_property("misses", cache.misses as f64)?;
        object.set_named_property("inserts", cache.inserts as f64)?;
        object.set_named_property("evictions", cache.evictions as f64)?;
    } else {
        object.set_named_property("enabled", false)?;
        object.set_named_property("size", 0_f64)?;
        object.set_named_property("capacity", 0_f64)?;
        object.set_named_property("hits", 0_f64)?;
        object.set_named_property("misses", 0_f64)?;
        object.set_named_property("inserts", 0_f64)?;
        object.set_named_property("evictions", 0_f64)?;
    }

    Ok(object)
}

#[cfg(test)]
mod tests {
    use super::{PropertyNameCache, MAX_PROPERTY_NAME_CACHE_ENTRIES, MAX_PROPERTY_NAME_KEY_BYTES};
    use std::ffi::CStr;

    #[test]
    fn caches_property_names_with_stable_pointers_across_rehashes() {
        let mut cache = PropertyNameCache::new();
        let first_ptr = cache.get(b"country").expect("valid key should be cached");
        assert_eq!(cache.get(b"country"), Some(first_ptr));

        for index in 0..512 {
            let key = format!("key_{index}");
            let ptr = cache.get(key.as_bytes()).expect("key should fit in cache");
            assert_eq!(unsafe { CStr::from_ptr(ptr) }.to_bytes(), key.as_bytes());
        }

        assert_eq!(cache.get(b"country"), Some(first_ptr));
        assert_eq!(unsafe { CStr::from_ptr(first_ptr) }.to_bytes(), b"country");
    }

    #[test]
    fn rejects_interior_nul_oversized_keys_and_enforces_capacity_bound() {
        let mut cache = PropertyNameCache::new();
        assert_eq!(cache.get(b"bad\0key"), None);
        assert!(cache.get(&[b'a'; MAX_PROPERTY_NAME_KEY_BYTES]).is_some());
        assert_eq!(cache.get(&[b'a'; MAX_PROPERTY_NAME_KEY_BYTES + 1]), None);
        cache.clear();

        for index in 0..MAX_PROPERTY_NAME_CACHE_ENTRIES {
            let key = format!("k{index}");
            assert!(cache.get(key.as_bytes()).is_some());
        }

        // Existing entries still hit, while new entries beyond the cap return None.
        assert!(cache.get(b"k0").is_some());
        assert_eq!(cache.get(b"overflow_key"), None);

        cache.clear();
        assert!(cache.get(b"overflow_key").is_some());
    }
}
