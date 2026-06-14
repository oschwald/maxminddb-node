use crate::{
    cache::{PropertyNameCache, RecordCache},
    errors::{lookup_error, napi_error},
};
use maxminddb::MaxMindDbError;
use napi::{
    bindgen_prelude::{Array, Buffer, Env, Null, ToNapiValue, Unknown},
    check_status, sys, Error, JsValue, Result,
};
use serde::de::{self, Deserialize, DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor};
use std::{
    borrow::Cow,
    cell::{Cell, RefCell},
    ffi::c_char,
    fmt, ptr,
};

thread_local! {
    static JS_DECODE_ENV: Cell<sys::napi_env> = Cell::new(ptr::null_mut());
    static JS_DECODE_NAPI_ERROR: RefCell<Option<Error>> = const { RefCell::new(None) };
    static JS_PROPERTY_NAME_CACHE: Cell<*const RefCell<PropertyNameCache>> = Cell::new(ptr::null());
}

#[derive(Debug)]
pub(crate) enum MmdbValue<'de> {
    Bool(bool),
    I32(i32),
    I64(i64),
    U16(u16),
    U32(u32),
    U64(u64),
    U128(u128),
    F64(f64),
    String(Cow<'de, str>),
    Bytes(Cow<'de, [u8]>),
    Array(Vec<MmdbValue<'de>>),
    Object(Vec<(Cow<'de, str>, MmdbValue<'de>)>),
}

struct RawJsValue(sys::napi_value);

impl RawJsValue {
    fn into_unknown<'env>(self, env: &'env Env) -> Unknown<'env> {
        unsafe { Unknown::from_raw_unchecked(env.raw(), self.0) }
    }
}

struct JsDecodeEnvGuard {
    previous_env: sys::napi_env,
    previous_error: Option<Error>,
    previous_property_name_cache: *const RefCell<PropertyNameCache>,
}

impl JsDecodeEnvGuard {
    fn enter(env: sys::napi_env, property_name_cache: &RefCell<PropertyNameCache>) -> Self {
        let previous_env = JS_DECODE_ENV.with(|cell| cell.replace(env));
        let previous_error = JS_DECODE_NAPI_ERROR.with(|cell| cell.replace(None));
        let previous_property_name_cache =
            JS_PROPERTY_NAME_CACHE.with(|cell| cell.replace(property_name_cache));
        Self {
            previous_env,
            previous_error,
            previous_property_name_cache,
        }
    }
}

impl Drop for JsDecodeEnvGuard {
    fn drop(&mut self) {
        JS_DECODE_ENV.with(|cell| cell.set(self.previous_env));
        JS_DECODE_NAPI_ERROR.with(|cell| {
            cell.replace(self.previous_error.take());
        });
        JS_PROPERTY_NAME_CACHE.with(|cell| cell.set(self.previous_property_name_cache));
    }
}

impl<'de> Deserialize<'de> for MmdbValue<'de> {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(MmdbValueVisitor)
    }
}

struct MmdbValueVisitor;

impl<'de> Visitor<'de> for MmdbValueVisitor {
    type Value = MmdbValue<'de>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("any valid MaxMind DB value")
    }

    fn visit_bool<E>(self, value: bool) -> std::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(MmdbValue::Bool(value))
    }

    fn visit_i32<E>(self, value: i32) -> std::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(MmdbValue::I32(value))
    }

    fn visit_i64<E>(self, value: i64) -> std::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(MmdbValue::I64(value))
    }

    fn visit_u16<E>(self, value: u16) -> std::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(MmdbValue::U16(value))
    }

    fn visit_u32<E>(self, value: u32) -> std::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(MmdbValue::U32(value))
    }

    fn visit_u64<E>(self, value: u64) -> std::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(MmdbValue::U64(value))
    }

    fn visit_u128<E>(self, value: u128) -> std::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(MmdbValue::U128(value))
    }

    fn visit_f32<E>(self, value: f32) -> std::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(MmdbValue::F64(f64::from(value)))
    }

    fn visit_f64<E>(self, value: f64) -> std::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(MmdbValue::F64(value))
    }

    fn visit_str<E>(self, value: &str) -> std::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(MmdbValue::String(Cow::Owned(value.to_owned())))
    }

    fn visit_borrowed_str<E>(self, value: &'de str) -> std::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(MmdbValue::String(Cow::Borrowed(value)))
    }

    fn visit_string<E>(self, value: String) -> std::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(MmdbValue::String(Cow::Owned(value)))
    }

    fn visit_bytes<E>(self, value: &[u8]) -> std::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(MmdbValue::Bytes(Cow::Owned(value.to_vec())))
    }

    fn visit_borrowed_bytes<E>(self, value: &'de [u8]) -> std::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(MmdbValue::Bytes(Cow::Borrowed(value)))
    }

    fn visit_byte_buf<E>(self, value: Vec<u8>) -> std::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(MmdbValue::Bytes(Cow::Owned(value)))
    }

    fn visit_seq<A>(self, mut seq: A) -> std::result::Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::with_capacity(seq.size_hint().unwrap_or(0));
        while let Some(value) = seq.next_element_seed(MmdbValueSeed)? {
            values.push(value);
        }
        Ok(MmdbValue::Array(values))
    }

    fn visit_map<A>(self, mut map: A) -> std::result::Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut values = Vec::with_capacity(map.size_hint().unwrap_or(0));
        while let Some(key) = map.next_key::<Cow<'de, str>>()? {
            let value = map.next_value_seed(MmdbValueSeed)?;
            values.push((key, value));
        }
        Ok(MmdbValue::Object(values))
    }
}

struct MmdbValueSeed;

impl<'de> DeserializeSeed<'de> for MmdbValueSeed {
    type Value = MmdbValue<'de>;

    fn deserialize<D>(self, deserializer: D) -> std::result::Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        MmdbValue::deserialize(deserializer)
    }
}

impl<'de> Deserialize<'de> for RawJsValue {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(RawJsValueVisitor)
    }
}

struct RawJsValueVisitor;

impl<'de> Visitor<'de> for RawJsValueVisitor {
    type Value = RawJsValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("any valid MaxMind DB value")
    }

    fn visit_bool<E>(self, value: bool) -> std::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        let env = js_decode_env()?;
        napi_result_to_de(raw_bool(env, value))
    }

    fn visit_i32<E>(self, value: i32) -> std::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        let env = js_decode_env()?;
        napi_result_to_de(raw_i32(env, value))
    }

    fn visit_i64<E>(self, value: i64) -> std::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        let env = js_decode_env()?;
        napi_result_to_de(raw_i64(env, value))
    }

    fn visit_u16<E>(self, value: u16) -> std::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        let env = js_decode_env()?;
        napi_result_to_de(raw_u32(env, u32::from(value)))
    }

    fn visit_u32<E>(self, value: u32) -> std::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        let env = js_decode_env()?;
        napi_result_to_de(raw_u32(env, value))
    }

    fn visit_u64<E>(self, value: u64) -> std::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        let env = js_decode_env()?;
        napi_result_to_de(raw_to_napi_value(env, value))
    }

    fn visit_u128<E>(self, value: u128) -> std::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        let env = js_decode_env()?;
        napi_result_to_de(raw_to_napi_value(env, value))
    }

    fn visit_f32<E>(self, value: f32) -> std::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        let env = js_decode_env()?;
        napi_result_to_de(raw_f64(env, f64::from(value)))
    }

    fn visit_f64<E>(self, value: f64) -> std::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        let env = js_decode_env()?;
        napi_result_to_de(raw_f64(env, value))
    }

    fn visit_str<E>(self, value: &str) -> std::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        let env = js_decode_env()?;
        napi_result_to_de(raw_string(env, value))
    }

    fn visit_borrowed_str<E>(self, value: &'de str) -> std::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        let env = js_decode_env()?;
        napi_result_to_de(raw_string(env, value))
    }

    fn visit_string<E>(self, value: String) -> std::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        let env = js_decode_env()?;
        napi_result_to_de(raw_string(env, &value))
    }

    fn visit_bytes<E>(self, value: &[u8]) -> std::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        let env = js_decode_env()?;
        napi_result_to_de(raw_buffer(env, value))
    }

    fn visit_borrowed_bytes<E>(self, value: &'de [u8]) -> std::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        let env = js_decode_env()?;
        napi_result_to_de(raw_buffer(env, value))
    }

    fn visit_byte_buf<E>(self, value: Vec<u8>) -> std::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        let env = js_decode_env()?;
        napi_result_to_de(raw_buffer(env, &value))
    }

    fn visit_none<E>(self) -> std::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        let env = js_decode_env()?;
        napi_result_to_de(raw_null(env))
    }

    fn visit_unit<E>(self) -> std::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        let env = js_decode_env()?;
        napi_result_to_de(raw_null(env))
    }

    fn visit_some<D>(self, deserializer: D) -> std::result::Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        RawJsValue::deserialize(deserializer)
    }

    fn visit_seq<A>(self, mut seq: A) -> std::result::Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let env = js_decode_env()?;
        let mut values = Vec::with_capacity(seq.size_hint().unwrap_or(0));
        while let Some(value) = seq.next_element_seed(RawJsValueSeed)? {
            values.push(value);
        }
        napi_result_to_de(raw_array(env, values))
    }

    fn visit_map<A>(self, mut map: A) -> std::result::Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let env = js_decode_env()?;
        let mut values = Vec::with_capacity(map.size_hint().unwrap_or(0));
        while let Some(key) = map.next_key::<Cow<'de, str>>()? {
            let value = map.next_value_seed(RawJsValueSeed)?;
            values.push((key, value));
        }
        napi_result_to_de(raw_object_entries(env, values))
    }
}

struct RawJsValueSeed;

impl<'de> DeserializeSeed<'de> for RawJsValueSeed {
    type Value = RawJsValue;

    fn deserialize<D>(self, deserializer: D) -> std::result::Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        RawJsValue::deserialize(deserializer)
    }
}

pub(crate) fn lookup_to_js<'env>(
    env: &'env Env,
    result: std::result::Result<Option<MmdbValue<'_>>, MaxMindDbError>,
) -> Result<Unknown<'env>> {
    match result.map_err(lookup_error)? {
        Some(value) => value_to_js(env, value),
        None => Null.into_unknown(env),
    }
}

pub(crate) fn lookup_result_record_to_js<'env, S: AsRef<[u8]>>(
    env: &'env Env,
    result: &maxminddb::LookupResult<'_, S>,
    cache: &RefCell<Option<RecordCache>>,
    property_names: &RefCell<PropertyNameCache>,
) -> Result<Unknown<'env>> {
    let Some(offset) = result.offset() else {
        return Null.into_unknown(env);
    };

    {
        let mut cache_guard = cache
            .try_borrow_mut()
            .map_err(|_| napi_error("cache already borrowed"))?;
        let Some(record_cache) = cache_guard.as_mut() else {
            return lookup_result_record_uncached_to_js(env, result, property_names);
        };

        if let Some(value) = record_cache.get(env, offset)? {
            return Ok(value);
        }
    }

    let value = lookup_result_record_uncached_to_js(env, result, property_names)?;
    if let Some(record_cache) = cache
        .try_borrow_mut()
        .map_err(|_| napi_error("cache already borrowed"))?
        .as_mut()
    {
        record_cache.put(env, offset, &value)?;
    }
    Ok(value)
}

fn lookup_result_record_uncached_to_js<'env, S: AsRef<[u8]>>(
    env: &'env Env,
    result: &maxminddb::LookupResult<'_, S>,
    property_names: &RefCell<PropertyNameCache>,
) -> Result<Unknown<'env>> {
    let _guard = JsDecodeEnvGuard::enter(env.raw(), property_names);
    match result.decode::<RawJsValue>() {
        Ok(Some(value)) => Ok(value.into_unknown(env)),
        Ok(None) => Null.into_unknown(env),
        Err(err) => Err(take_js_decode_napi_error().unwrap_or_else(|| lookup_error(err))),
    }
}

fn js_decode_env<E>() -> std::result::Result<sys::napi_env, E>
where
    E: de::Error,
{
    let env = JS_DECODE_ENV.with(Cell::get);
    if env.is_null() {
        return Err(E::custom(
            "JavaScript decode environment was not initialized",
        ));
    }
    Ok(env)
}

fn napi_result_to_de<T, E>(result: Result<T>) -> std::result::Result<T, E>
where
    E: de::Error,
{
    result.map_err(|err| {
        let message = err.to_string();
        JS_DECODE_NAPI_ERROR.with(|cell| {
            cell.replace(Some(err));
        });
        E::custom(message)
    })
}

fn take_js_decode_napi_error() -> Option<Error> {
    JS_DECODE_NAPI_ERROR.with(|cell| cell.take())
}

fn raw_to_napi_value<T>(env: sys::napi_env, value: T) -> Result<RawJsValue>
where
    T: ToNapiValue,
{
    unsafe { ToNapiValue::to_napi_value(env, value) }.map(RawJsValue)
}

fn raw_null(env: sys::napi_env) -> Result<RawJsValue> {
    let mut value = ptr::null_mut();
    check_status!(
        unsafe { sys::napi_get_null(env, &mut value) },
        "Failed to create null",
    )?;
    Ok(RawJsValue(value))
}

fn raw_bool(env: sys::napi_env, bool_value: bool) -> Result<RawJsValue> {
    let mut value = ptr::null_mut();
    check_status!(
        unsafe { sys::napi_get_boolean(env, bool_value, &mut value) },
        "Failed to create boolean",
    )?;
    Ok(RawJsValue(value))
}

fn raw_i32(env: sys::napi_env, int_value: i32) -> Result<RawJsValue> {
    let mut value = ptr::null_mut();
    check_status!(
        unsafe { sys::napi_create_int32(env, int_value, &mut value) },
        "Failed to create integer",
    )?;
    Ok(RawJsValue(value))
}

fn raw_i64(env: sys::napi_env, int_value: i64) -> Result<RawJsValue> {
    if int_value >= i64::from(i32::MIN) && int_value <= i64::from(i32::MAX) {
        return raw_i32(env, int_value as i32);
    }

    let mut value = ptr::null_mut();
    check_status!(
        unsafe { sys::napi_create_int64(env, int_value, &mut value) },
        "Failed to create integer",
    )?;
    Ok(RawJsValue(value))
}

fn raw_u32(env: sys::napi_env, int_value: u32) -> Result<RawJsValue> {
    let mut value = ptr::null_mut();
    check_status!(
        unsafe { sys::napi_create_uint32(env, int_value, &mut value) },
        "Failed to create unsigned integer",
    )?;
    Ok(RawJsValue(value))
}

fn raw_f64(env: sys::napi_env, float_value: f64) -> Result<RawJsValue> {
    let mut value = ptr::null_mut();
    check_status!(
        unsafe { sys::napi_create_double(env, float_value, &mut value) },
        "Failed to create double",
    )?;
    Ok(RawJsValue(value))
}

fn raw_string(env: sys::napi_env, string_value: &str) -> Result<RawJsValue> {
    raw_js_string(env, string_value).map(RawJsValue)
}

fn raw_js_string(env: sys::napi_env, string_value: &str) -> Result<sys::napi_value> {
    let mut value = ptr::null_mut();
    if string_value.is_ascii() {
        check_status!(
            unsafe {
                sys::napi_create_string_latin1(
                    env,
                    string_value.as_ptr().cast(),
                    string_value.len() as isize,
                    &mut value,
                )
            },
            "Failed to create string",
        )?;
    } else {
        check_status!(
            unsafe {
                sys::napi_create_string_utf8(
                    env,
                    string_value.as_ptr().cast(),
                    string_value.len() as isize,
                    &mut value,
                )
            },
            "Failed to create string",
        )?;
    }
    Ok(value)
}

fn raw_property_descriptor_name(
    env: sys::napi_env,
    property_name: &str,
) -> Result<(*const c_char, sys::napi_value)> {
    let cache = JS_PROPERTY_NAME_CACHE.with(Cell::get);
    if cache.is_null() {
        return raw_js_string(env, property_name).map(|name| (ptr::null(), name));
    }

    if let Some(utf8name) = unsafe { &*cache }
        .try_borrow_mut()
        .map_err(|_| napi_error("property name cache already borrowed"))?
        .get(property_name)
    {
        return Ok((utf8name, ptr::null_mut()));
    }

    raw_js_string(env, property_name).map(|name| (ptr::null(), name))
}

fn raw_buffer(env: sys::napi_env, bytes: &[u8]) -> Result<RawJsValue> {
    let mut value = ptr::null_mut();
    let data = if bytes.is_empty() {
        ptr::null()
    } else {
        bytes.as_ptr().cast()
    };
    check_status!(
        unsafe {
            sys::napi_create_buffer_copy(env, bytes.len(), data, ptr::null_mut(), &mut value)
        },
        "Failed to create buffer",
    )?;
    Ok(RawJsValue(value))
}

fn raw_array(env: sys::napi_env, values: Vec<RawJsValue>) -> Result<RawJsValue> {
    let mut array = ptr::null_mut();
    check_status!(
        unsafe { sys::napi_create_array_with_length(env, values.len(), &mut array) },
        "Failed to create array",
    )?;

    for (index, value) in values.into_iter().enumerate() {
        let index =
            u32::try_from(index).map_err(|_| napi_error("array index exceeds u32 range"))?;
        check_status!(
            unsafe { sys::napi_set_element(env, array, index, value.0) },
            "Failed to set array element",
        )?;
    }

    Ok(RawJsValue(array))
}

fn raw_object_entries(
    env: sys::napi_env,
    values: Vec<(Cow<'_, str>, RawJsValue)>,
) -> Result<RawJsValue> {
    let mut object = ptr::null_mut();
    check_status!(
        unsafe { sys::napi_create_object(env, &mut object) },
        "Failed to create object",
    )?;

    let mut descriptors = Vec::with_capacity(values.len());
    for (key, value) in values {
        let key = key.as_ref();
        let (utf8name, name) = raw_property_descriptor_name(env, key)?;
        descriptors.push(sys::napi_property_descriptor {
            utf8name,
            name,
            method: None,
            getter: None,
            setter: None,
            value: value.0,
            attributes: sys::PropertyAttributes::writable
                | sys::PropertyAttributes::enumerable
                | sys::PropertyAttributes::configurable,
            data: ptr::null_mut(),
        });
    }

    if !descriptors.is_empty() {
        check_status!(
            unsafe {
                sys::napi_define_properties(env, object, descriptors.len(), descriptors.as_ptr())
            },
            "Failed to define properties",
        )?;
    }

    Ok(RawJsValue(object))
}

pub(crate) fn value_to_js<'env>(env: &'env Env, value: MmdbValue<'_>) -> Result<Unknown<'env>> {
    match value {
        MmdbValue::Bool(value) => value.into_unknown(env),
        MmdbValue::I32(value) => value.into_unknown(env),
        MmdbValue::I64(value) if value >= i32::MIN as i64 && value <= i32::MAX as i64 => {
            (value as i32).into_unknown(env)
        }
        MmdbValue::I64(value) => value.into_unknown(env),
        MmdbValue::U16(value) => value.into_unknown(env),
        MmdbValue::U32(value) => value.into_unknown(env),
        MmdbValue::U64(value) => value.into_unknown(env),
        MmdbValue::U128(value) => value.into_unknown(env),
        MmdbValue::F64(value) => value.into_unknown(env),
        MmdbValue::String(value) => value.as_ref().into_unknown(env),
        MmdbValue::Bytes(value) => Buffer::from(value.into_owned()).into_unknown(env),
        MmdbValue::Array(values) => {
            let js_values = values
                .into_iter()
                .map(|value| value_to_js(env, value))
                .collect::<Result<Vec<_>>>()?;
            Array::from_vec(env, js_values)?.into_unknown(env)
        }
        MmdbValue::Object(values) => object_entries_to_js(env, values),
    }
}

fn object_entries_to_js<'env>(
    env: &'env Env,
    values: Vec<(Cow<'_, str>, MmdbValue<'_>)>,
) -> Result<Unknown<'env>> {
    let raw_env = env.raw();
    let mut object = ptr::null_mut();
    check_status!(
        unsafe { sys::napi_create_object(raw_env, &mut object) },
        "Failed to create object",
    )?;

    let mut descriptors = Vec::with_capacity(values.len());
    for (key, value) in values {
        let key = key.as_ref();
        let name = raw_js_string(raw_env, key)?;
        let value = value_to_js(env, value)?;
        descriptors.push(sys::napi_property_descriptor {
            utf8name: ptr::null(),
            name,
            method: None,
            getter: None,
            setter: None,
            value: value.raw(),
            attributes: sys::PropertyAttributes::writable
                | sys::PropertyAttributes::enumerable
                | sys::PropertyAttributes::configurable,
            data: ptr::null_mut(),
        });
    }

    if !descriptors.is_empty() {
        check_status!(
            unsafe {
                sys::napi_define_properties(
                    raw_env,
                    object,
                    descriptors.len(),
                    descriptors.as_ptr(),
                )
            },
            "Failed to define properties",
        )?;
    }

    Ok(unsafe { Unknown::from_raw_unchecked(raw_env, object) })
}
