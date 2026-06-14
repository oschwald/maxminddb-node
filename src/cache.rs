use crate::errors::napi_error;
use lru::LruCache;
use napi::{
    bindgen_prelude::{Env, JsObjectValue, Object, Unknown},
    Result, UnknownRef, ValueType,
};
use std::{
    cell::RefCell,
    collections::HashMap,
    ffi::{c_char, CString},
    num::NonZeroUsize,
};

pub(crate) struct RecordCache {
    pub(crate) values: LruCache<usize, UnknownRef>,
    pub(crate) hits: u64,
    pub(crate) misses: u64,
    pub(crate) inserts: u64,
    pub(crate) evictions: u64,
}

pub(crate) struct PropertyNameCache {
    values: HashMap<String, CString>,
}

impl PropertyNameCache {
    pub(crate) fn new() -> Self {
        Self {
            values: HashMap::new(),
        }
    }

    pub(crate) fn get(&mut self, name: &str) -> Option<*const c_char> {
        if let Some(reference) = self.values.get(name) {
            return Some(reference.as_ptr());
        }

        let reference = CString::new(name).ok()?;
        let pointer = reference.as_ptr();
        self.values.insert(name.to_owned(), reference);
        Some(pointer)
    }

    pub(crate) fn clear(&mut self) {
        self.values.clear();
    }
}

impl RecordCache {
    pub(crate) fn new(capacity: NonZeroUsize) -> Self {
        Self {
            values: LruCache::new(capacity),
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
        let Some(value) = self.values.get(&offset) else {
            self.misses += 1;
            return Ok(None);
        };

        self.hits += 1;
        value.get_value(env).map(Some)
    }

    pub(crate) fn put(&mut self, env: &Env, offset: usize, value: &Unknown<'_>) -> Result<()> {
        if value.get_type()? != ValueType::Object {
            return Ok(());
        }

        let reference = value.create_ref()?;
        self.inserts += 1;
        if let Some((old_offset, old_reference)) = self.values.push(offset, reference) {
            if old_offset != offset {
                self.evictions += 1;
            }
            old_reference.unref(env)?;
        }
        Ok(())
    }

    pub(crate) fn clear(&mut self, env: &Env) -> Result<()> {
        while let Some((_offset, reference)) = self.values.pop_lru() {
            reference.unref(env)?;
        }
        Ok(())
    }
}

pub(crate) fn cache_stats_to_js<'env>(
    env: &'env Env,
    cache: &RefCell<Option<RecordCache>>,
) -> Result<Object<'env>> {
    let cache = cache
        .try_borrow()
        .map_err(|_| napi_error("cache already borrowed"))?;
    let mut object = Object::new(env)?;

    if let Some(cache) = cache.as_ref() {
        object.set_named_property("enabled", true)?;
        object.set_named_property("size", cache.values.len() as f64)?;
        object.set_named_property("capacity", cache.values.cap().get() as f64)?;
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
