use lru::LruCache;
use napi::{
    bindgen_prelude::{Env, JsObjectValue, Object, Unknown},
    Result, UnknownRef, ValueType,
};
use std::{
    collections::HashMap,
    ffi::{c_char, CString},
    num::NonZeroUsize,
};

pub(crate) struct RecordCache {
    probationary: LruCache<usize, UnknownRef>,
    protected: Option<LruCache<usize, UnknownRef>>,
    capacity: NonZeroUsize,
    pub(crate) hits: u64,
    pub(crate) misses: u64,
    pub(crate) inserts: u64,
    pub(crate) evictions: u64,
}

pub(crate) struct PropertyNameCache {
    values: HashMap<Vec<u8>, CString>,
}

impl PropertyNameCache {
    pub(crate) fn new() -> Self {
        Self {
            values: HashMap::new(),
        }
    }

    pub(crate) fn get(&mut self, name: &[u8]) -> Option<*const c_char> {
        if let Some(reference) = self.values.get(name) {
            return Some(reference.as_ptr());
        }

        let reference = CString::new(name).ok()?;
        let pointer = reference.as_ptr();
        self.values.insert(name.to_vec(), reference);
        Some(pointer)
    }

    pub(crate) fn clear(&mut self) {
        self.values.clear();
    }
}

impl RecordCache {
    pub(crate) fn new(capacity: NonZeroUsize) -> Self {
        let protected_capacity = if capacity.get() <= 10_000 {
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

            if let Some(value) = self.probationary.pop(&offset) {
                let result = value.get_value(env)?;
                if let Some((demoted_offset, demoted)) = protected.push(offset, value) {
                    debug_assert_ne!(demoted_offset, offset);
                    let evicted = self.probationary.push(demoted_offset, demoted);
                    debug_assert!(evicted.is_none());
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
