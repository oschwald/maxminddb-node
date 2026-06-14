use maxminddb::{MaxMindDbError, Mmap, Reader as MaxMindReader};
use napi::{
    bindgen_prelude::{
        Array, Buffer, Either, Env, JsObjectValue, Null, Object, ToNapiValue, Unknown,
    },
    Error, Result, Status,
};
use napi_derive::napi;
use serde::de::{
    self, Deserialize, DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor,
};
use std::{
    fmt,
    net::{IpAddr, Ipv4Addr},
    path::Path,
    str::FromStr,
    sync::RwLock,
};

const ERR_CLOSED_DB: &str = "Attempt to read from a closed MaxMind DB.";
const ERR_BAD_DATA: &str =
    "The MaxMind DB file's data section contains bad data (unknown data type or corrupt data)";

enum ReaderSource {
    Mmap(MaxMindReader<Mmap>),
    Memory(MaxMindReader<Vec<u8>>),
}

impl ReaderSource {
    fn lookup(&self, ip: IpAddr) -> std::result::Result<Option<MmdbValue>, MaxMindDbError> {
        match self {
            ReaderSource::Mmap(reader) => reader.lookup(ip)?.decode(),
            ReaderSource::Memory(reader) => reader.lookup(ip)?.decode(),
        }
    }

    fn lookup_prefix(
        &self,
        ip: IpAddr,
    ) -> std::result::Result<(Option<MmdbValue>, usize), MaxMindDbError> {
        match self {
            ReaderSource::Mmap(reader) => {
                let result = reader.lookup(ip)?;
                let network = result.network()?;
                let prefix = prefix_len_for_lookup(ip, network);
                Ok((result.decode()?, prefix))
            }
            ReaderSource::Memory(reader) => {
                let result = reader.lookup(ip)?;
                let network = result.network()?;
                let prefix = prefix_len_for_lookup(ip, network);
                Ok((result.decode()?, prefix))
            }
        }
    }

    fn lookup_path(
        &self,
        ip: IpAddr,
        path: &[maxminddb::PathElement<'_>],
    ) -> std::result::Result<Option<MmdbValue>, MaxMindDbError> {
        match self {
            ReaderSource::Mmap(reader) => reader.lookup(ip)?.decode_path(path),
            ReaderSource::Memory(reader) => reader.lookup(ip)?.decode_path(path),
        }
    }

    fn metadata(&self) -> &maxminddb::Metadata {
        match self {
            ReaderSource::Mmap(reader) => &reader.metadata,
            ReaderSource::Memory(reader) => &reader.metadata,
        }
    }
}

#[derive(Debug)]
enum MmdbValue {
    Bool(bool),
    I32(i32),
    I64(i64),
    U16(u16),
    U32(u32),
    U64(u64),
    U128(u128),
    F64(f64),
    String(String),
    Bytes(Vec<u8>),
    Array(Vec<MmdbValue>),
    Object(Vec<(String, MmdbValue)>),
}

enum OwnedPathElement {
    Key(String),
    Index(usize),
    IndexFromEnd(usize),
}

impl<'de> Deserialize<'de> for MmdbValue {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(MmdbValueVisitor)
    }
}

struct MmdbValueVisitor;

impl<'de> Visitor<'de> for MmdbValueVisitor {
    type Value = MmdbValue;

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
        Ok(MmdbValue::String(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> std::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(MmdbValue::String(value))
    }

    fn visit_bytes<E>(self, value: &[u8]) -> std::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(MmdbValue::Bytes(value.to_vec()))
    }

    fn visit_byte_buf<E>(self, value: Vec<u8>) -> std::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(MmdbValue::Bytes(value))
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
        while let Some(key) = map.next_key::<String>()? {
            let value = map.next_value_seed(MmdbValueSeed)?;
            values.push((key, value));
        }
        Ok(MmdbValue::Object(values))
    }
}

struct MmdbValueSeed;

impl<'de> DeserializeSeed<'de> for MmdbValueSeed {
    type Value = MmdbValue;

    fn deserialize<D>(self, deserializer: D) -> std::result::Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        MmdbValue::deserialize(deserializer)
    }
}

#[napi(js_name = "NativeReader")]
pub struct NativeReader {
    reader: RwLock<Option<ReaderSource>>,
    ip_version: u16,
}

#[napi]
impl NativeReader {
    #[napi(constructor)]
    pub fn new(database: Buffer) -> Result<Self> {
        Self::from_bytes(database.as_ref().to_vec())
    }

    #[napi]
    pub fn load(&mut self, database: Buffer) -> Result<()> {
        let new_reader = Self::reader_from_bytes(database.as_ref().to_vec())?;
        self.ip_version = new_reader.metadata().ip_version;
        *self
            .reader
            .write()
            .map_err(|_| napi_error("reader lock poisoned"))? = Some(new_reader);
        Ok(())
    }

    #[napi(js_name = "reloadFromFile")]
    pub fn reload_from_file(&mut self, path: String, mode: Option<String>) -> Result<()> {
        let new_reader = open_source(&path, mode.as_deref())?;
        self.ip_version = new_reader.metadata().ip_version;
        *self
            .reader
            .write()
            .map_err(|_| napi_error("reader lock poisoned"))? = Some(new_reader);
        Ok(())
    }

    #[napi(getter)]
    pub fn closed(&self) -> Result<bool> {
        Ok(self
            .reader
            .read()
            .map_err(|_| napi_error("reader lock poisoned"))?
            .is_none())
    }

    #[napi]
    pub fn close(&mut self) -> Result<()> {
        *self
            .reader
            .write()
            .map_err(|_| napi_error("reader lock poisoned"))? = None;
        Ok(())
    }

    #[napi]
    pub fn get<'env>(&self, env: &'env Env, ip_address: String) -> Result<Unknown<'env>> {
        let ip = self.parse_lookup_ip(&ip_address)?;
        let guard = self
            .reader
            .read()
            .map_err(|_| napi_error("reader lock poisoned"))?;
        let reader = guard.as_ref().ok_or_else(|| invalid_arg(ERR_CLOSED_DB))?;
        lookup_to_js(env, reader.lookup(ip))
    }

    #[napi(js_name = "getPath")]
    pub fn get_path<'env>(
        &self,
        env: &'env Env,
        ip_address: String,
        path: Vec<Either<String, i64>>,
    ) -> Result<Unknown<'env>> {
        let ip = self.parse_lookup_ip(&ip_address)?;
        let owned_path = parse_path(path)?;
        let path_elements = path_elements_from_owned(&owned_path);
        let guard = self
            .reader
            .read()
            .map_err(|_| napi_error("reader lock poisoned"))?;
        let reader = guard.as_ref().ok_or_else(|| invalid_arg(ERR_CLOSED_DB))?;
        lookup_to_js(env, reader.lookup_path(ip, &path_elements))
    }

    #[napi(js_name = "getWithPrefixLength")]
    pub fn get_with_prefix_length<'env>(
        &self,
        env: &'env Env,
        ip_address: String,
    ) -> Result<Unknown<'env>> {
        let ip = self.parse_lookup_ip(&ip_address)?;
        let guard = self
            .reader
            .read()
            .map_err(|_| napi_error("reader lock poisoned"))?;
        let reader = guard.as_ref().ok_or_else(|| invalid_arg(ERR_CLOSED_DB))?;
        let (value, prefix_len) = reader.lookup_prefix(ip).map_err(lookup_error)?;
        let js_value = match value {
            Some(value) => value_to_js(env, value)?,
            None => Null.into_unknown(env)?,
        };
        let js_prefix = (prefix_len as u32).into_unknown(env)?;
        Array::from_vec(env, vec![js_value, js_prefix])?.into_unknown(env)
    }

    #[napi(js_name = "getMany")]
    pub fn get_many<'env>(&self, env: &'env Env, ips: Vec<String>) -> Result<Unknown<'env>> {
        let parsed_ips = self.parse_lookup_ips(ips)?;
        let guard = self
            .reader
            .read()
            .map_err(|_| napi_error("reader lock poisoned"))?;
        let reader = guard.as_ref().ok_or_else(|| invalid_arg(ERR_CLOSED_DB))?;
        let values = parsed_ips
            .into_iter()
            .map(|ip| lookup_to_js(env, reader.lookup(ip)))
            .collect::<Result<Vec<_>>>()?;
        Array::from_vec(env, values)?.into_unknown(env)
    }

    #[napi(js_name = "getManyPath")]
    pub fn get_many_path<'env>(
        &self,
        env: &'env Env,
        ips: Vec<String>,
        path: Vec<Either<String, i64>>,
    ) -> Result<Unknown<'env>> {
        let parsed_ips = self.parse_lookup_ips(ips)?;
        let owned_path = parse_path(path)?;
        let path_elements = path_elements_from_owned(&owned_path);
        let guard = self
            .reader
            .read()
            .map_err(|_| napi_error("reader lock poisoned"))?;
        let reader = guard.as_ref().ok_or_else(|| invalid_arg(ERR_CLOSED_DB))?;
        let values = parsed_ips
            .into_iter()
            .map(|ip| lookup_to_js(env, reader.lookup_path(ip, &path_elements)))
            .collect::<Result<Vec<_>>>()?;
        Array::from_vec(env, values)?.into_unknown(env)
    }

    #[napi]
    pub fn metadata<'env>(&self, env: &'env Env) -> Result<Object<'env>> {
        let guard = self
            .reader
            .read()
            .map_err(|_| napi_error("reader lock poisoned"))?;
        let reader = guard.as_ref().ok_or_else(|| invalid_arg(ERR_CLOSED_DB))?;
        metadata_to_js(env, reader.metadata())
    }
}

impl NativeReader {
    fn from_bytes(bytes: Vec<u8>) -> Result<Self> {
        let source = Self::reader_from_bytes(bytes)?;
        Ok(create_reader(source))
    }

    fn reader_from_bytes(bytes: Vec<u8>) -> Result<ReaderSource> {
        MaxMindReader::from_source(bytes)
            .map(ReaderSource::Memory)
            .map_err(open_error)
    }

    fn parse_lookup_ip(&self, ip_address: &str) -> Result<IpAddr> {
        let ip = parse_ip(ip_address)?;
        if self.ip_version == 4 && matches!(ip, IpAddr::V6(_)) {
            return Err(invalid_arg(format!(
                "Error looking up {ip}. You attempted to look up an IPv6 address in an IPv4-only database"
            )));
        }
        Ok(ip)
    }

    fn parse_lookup_ips(&self, ips: Vec<String>) -> Result<Vec<IpAddr>> {
        ips.iter().map(|ip| self.parse_lookup_ip(ip)).collect()
    }
}

#[napi(js_name = "openReader")]
pub fn open_reader(path: String, mode: Option<String>) -> Result<NativeReader> {
    open_source(&path, mode.as_deref()).map(create_reader)
}

#[napi(js_name = "nativeVersion")]
pub fn native_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

fn create_reader(source: ReaderSource) -> NativeReader {
    let ip_version = source.metadata().ip_version;
    NativeReader {
        reader: RwLock::new(Some(source)),
        ip_version,
    }
}

fn open_source(path: &str, mode: Option<&str>) -> Result<ReaderSource> {
    match mode.unwrap_or("mmap") {
        "auto" | "mmap" => {
            // SAFETY: The mapping is read-only. Callers should replace database files
            // atomically rather than mutating an open file in place.
            unsafe { MaxMindReader::open_mmap(Path::new(path)) }
                .map(ReaderSource::Mmap)
                .map_err(open_error)
        }
        "memory" | "buffer" => MaxMindReader::open_readfile(Path::new(path))
            .map(ReaderSource::Memory)
            .map_err(open_error),
        other => Err(invalid_arg(format!("Unsupported open mode: {other}"))),
    }
}

fn lookup_to_js<'env>(
    env: &'env Env,
    result: std::result::Result<Option<MmdbValue>, MaxMindDbError>,
) -> Result<Unknown<'env>> {
    match result.map_err(lookup_error)? {
        Some(value) => value_to_js(env, value),
        None => Null.into_unknown(env),
    }
}

fn value_to_js<'env>(env: &'env Env, value: MmdbValue) -> Result<Unknown<'env>> {
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
        MmdbValue::String(value) => value.into_unknown(env),
        MmdbValue::Bytes(value) => Buffer::from(value).into_unknown(env),
        MmdbValue::Array(values) => {
            let js_values = values
                .into_iter()
                .map(|value| value_to_js(env, value))
                .collect::<Result<Vec<_>>>()?;
            Array::from_vec(env, js_values)?.into_unknown(env)
        }
        MmdbValue::Object(values) => {
            let mut object = Object::new(env)?;
            for (key, value) in values {
                object.set_named_property(&key, value_to_js(env, value)?)?;
            }
            object.into_unknown(env)
        }
    }
}

fn metadata_to_js<'env>(env: &'env Env, meta: &maxminddb::Metadata) -> Result<Object<'env>> {
    let mut object = Object::new(env)?;
    object.set_named_property(
        "binaryFormatMajorVersion",
        meta.binary_format_major_version,
    )?;
    object.set_named_property(
        "binaryFormatMinorVersion",
        meta.binary_format_minor_version,
    )?;
    object.set_named_property("buildEpoch", meta.build_epoch as f64)?;
    object.set_named_property("databaseType", meta.database_type.as_str())?;

    let mut description = Object::new(env)?;
    for (key, value) in &meta.description {
        description.set_named_property(key, value.as_str())?;
    }
    object.set_named_property("description", description)?;

    object.set_named_property("ipVersion", meta.ip_version)?;
    object.set_named_property("languages", Array::from_ref_vec_string(env, &meta.languages)?)?;
    object.set_named_property("nodeCount", meta.node_count)?;
    object.set_named_property("recordSize", meta.record_size)?;
    object.set_named_property("nodeByteSize", meta.record_size / 4)?;
    object.set_named_property("searchTreeSize", meta.node_count * (meta.record_size as u32 / 4))?;
    object.set_named_property("treeDepth", if meta.ip_version == 4 { 32_u32 } else { 128_u32 })?;
    Ok(object)
}

fn parse_path(path: Vec<Either<String, i64>>) -> Result<Vec<OwnedPathElement>> {
    path.into_iter()
        .map(|element| match element {
            Either::A(key) => Ok(OwnedPathElement::Key(key)),
            Either::B(index) => Ok(signed_index_to_path_element(index)),
        })
        .collect()
}

fn signed_index_to_path_element(index: i64) -> OwnedPathElement {
    if index >= 0 {
        OwnedPathElement::Index(index as usize)
    } else {
        let index_from_end = index
            .checked_neg()
            .and_then(|n| n.checked_sub(1))
            .map(|n| n as usize)
            .unwrap_or(usize::MAX);
        OwnedPathElement::IndexFromEnd(index_from_end)
    }
}

fn path_elements_from_owned(path: &[OwnedPathElement]) -> Vec<maxminddb::PathElement<'_>> {
    path.iter()
        .map(|element| match element {
            OwnedPathElement::Key(key) => maxminddb::PathElement::Key(key.as_str()),
            OwnedPathElement::Index(index) => maxminddb::PathElement::Index(*index),
            OwnedPathElement::IndexFromEnd(index) => {
                maxminddb::PathElement::IndexFromEnd(*index)
            }
        })
        .collect()
}

fn parse_ip(ip: &str) -> Result<IpAddr> {
    if let Some(ip) = parse_ipv4(ip.as_bytes()) {
        return Ok(IpAddr::V4(ip));
    }
    IpAddr::from_str(ip).map_err(|_| invalid_arg(format!("Invalid IP address: {ip}")))
}

fn parse_ipv4(bytes: &[u8]) -> Option<Ipv4Addr> {
    let mut octets = [0_u8; 4];
    let mut octet_index = 0;
    let mut value: u16 = 0;
    let mut digits = 0;

    for &byte in bytes {
        if byte == b'.' {
            if digits == 0 || octet_index == 3 {
                return None;
            }
            octets[octet_index] = value as u8;
            octet_index += 1;
            value = 0;
            digits = 0;
            continue;
        }
        if !byte.is_ascii_digit() {
            return None;
        }
        if digits == 1 && value == 0 {
            return None;
        }
        digits += 1;
        if digits > 3 {
            return None;
        }
        value = value * 10 + u16::from(byte - b'0');
        if value > u16::from(u8::MAX) {
            return None;
        }
    }

    if octet_index != 3 || digits == 0 {
        return None;
    }
    octets[octet_index] = value as u8;
    Some(Ipv4Addr::from(octets))
}

fn prefix_len_for_lookup(ip: IpAddr, network: ipnetwork::IpNetwork) -> usize {
    if ip.is_ipv4() && network.is_ipv6() {
        (network.prefix() as usize).saturating_sub(96)
    } else {
        network.prefix() as usize
    }
}

fn open_error(err: MaxMindDbError) -> Error {
    match err {
        MaxMindDbError::Io(io_err) => Error::new(Status::GenericFailure, io_err.to_string()),
        MaxMindDbError::InvalidDatabase { .. } | MaxMindDbError::Decoding { .. } => {
            Error::new(Status::GenericFailure, ERR_BAD_DATA)
        }
        other => Error::new(Status::GenericFailure, other.to_string()),
    }
}

fn lookup_error(err: MaxMindDbError) -> Error {
    match err {
        MaxMindDbError::InvalidDatabase { .. } | MaxMindDbError::Decoding { .. } => {
            Error::new(Status::GenericFailure, ERR_BAD_DATA)
        }
        other => Error::new(Status::GenericFailure, other.to_string()),
    }
}

fn invalid_arg(message: impl Into<String>) -> Error {
    Error::new(Status::InvalidArg, message.into())
}

fn napi_error(message: impl Into<String>) -> Error {
    Error::new(Status::GenericFailure, message.into())
}
