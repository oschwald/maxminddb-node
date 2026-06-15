mod cache;
mod decode;
mod errors;
mod ip;
mod metadata;
mod networks;
mod paths;

use crate::{
    cache::{cache_stats_to_js, PropertyNameCache, RecordCache},
    decode::{lookup_result_record_to_js, lookup_to_js, MmdbValue},
    errors::{invalid_arg, lookup_error, napi_error, open_error},
    ip::{parse_ip, parse_network, prefix_len_for_lookup},
    metadata::metadata_to_js,
    networks::{
        collect_networks_for_reader, collect_networks_page_for_reader, make_within_options,
        network_record_page_to_js, network_records_to_js, NetworkRecord, NetworkRecordPage,
    },
    paths::{compiled_path, parse_path, path_elements_from_owned, OwnedPathElement},
};
use maxminddb::{MaxMindDbError, Mmap, Reader as MaxMindReader, WithinOptions};
use napi::{
    bindgen_prelude::{Array, Buffer, Either, Env, Object, ObjectFinalize, ToNapiValue, Unknown},
    Result,
};
use napi_derive::napi;
use std::{cell::RefCell, net::IpAddr, num::NonZeroUsize, path::Path};

const ERR_CLOSED_DB: &str = "Attempt to read from a closed MaxMind DB.";

enum ReaderSource {
    Mmap(MaxMindReader<Mmap>),
    Memory(MaxMindReader<Vec<u8>>),
}

impl ReaderSource {
    fn lookup_record_to_js<'env>(
        &self,
        env: &'env Env,
        ip: IpAddr,
        cache: &RefCell<Option<RecordCache>>,
        property_names: &RefCell<PropertyNameCache>,
    ) -> Result<Unknown<'env>> {
        match self {
            ReaderSource::Mmap(reader) => {
                let result = reader.lookup(ip).map_err(lookup_error)?;
                lookup_result_record_to_js(env, &result, cache, property_names)
            }
            ReaderSource::Memory(reader) => {
                let result = reader.lookup(ip).map_err(lookup_error)?;
                lookup_result_record_to_js(env, &result, cache, property_names)
            }
        }
    }

    fn lookup_record_with_prefix_to_js<'env>(
        &self,
        env: &'env Env,
        ip: IpAddr,
        cache: &RefCell<Option<RecordCache>>,
        property_names: &RefCell<PropertyNameCache>,
    ) -> Result<(Unknown<'env>, usize)> {
        match self {
            ReaderSource::Mmap(reader) => {
                let result = reader.lookup(ip).map_err(lookup_error)?;
                let network = result.network().map_err(lookup_error)?;
                let prefix = prefix_len_for_lookup(ip, network);
                Ok((
                    lookup_result_record_to_js(env, &result, cache, property_names)?,
                    prefix,
                ))
            }
            ReaderSource::Memory(reader) => {
                let result = reader.lookup(ip).map_err(lookup_error)?;
                let network = result.network().map_err(lookup_error)?;
                let prefix = prefix_len_for_lookup(ip, network);
                Ok((
                    lookup_result_record_to_js(env, &result, cache, property_names)?,
                    prefix,
                ))
            }
        }
    }

    fn lookup_path(
        &self,
        ip: IpAddr,
        path: &[maxminddb::PathElement<'_>],
    ) -> std::result::Result<Option<MmdbValue<'_>>, MaxMindDbError> {
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

    fn collect_networks(
        &self,
        cidr: Option<ipnetwork::IpNetwork>,
        options: WithinOptions,
    ) -> std::result::Result<Vec<NetworkRecord<'_>>, MaxMindDbError> {
        match self {
            ReaderSource::Mmap(reader) => collect_networks_for_reader(reader, cidr, options),
            ReaderSource::Memory(reader) => collect_networks_for_reader(reader, cidr, options),
        }
    }

    fn collect_networks_page(
        &self,
        cidr: Option<ipnetwork::IpNetwork>,
        options: WithinOptions,
        limit: usize,
        offset: usize,
    ) -> std::result::Result<NetworkRecordPage<'_>, MaxMindDbError> {
        match self {
            ReaderSource::Mmap(reader) => {
                collect_networks_page_for_reader(reader, cidr, options, limit, offset)
            }
            ReaderSource::Memory(reader) => {
                collect_networks_page_for_reader(reader, cidr, options, limit, offset)
            }
        }
    }
}

#[napi(js_name = "NativeReader", custom_finalize)]
pub struct NativeReader {
    reader: Option<ReaderSource>,
    cache: RefCell<Option<RecordCache>>,
    property_names: RefCell<PropertyNameCache>,
    paths: RefCell<Vec<Vec<OwnedPathElement>>>,
    ip_version: u16,
}

#[napi]
impl NativeReader {
    #[napi(constructor)]
    pub fn new(database: Buffer, cache_capacity: Option<u32>) -> Result<Self> {
        Self::from_bytes(database.as_ref().to_vec(), cache_capacity)
    }

    #[napi]
    pub fn load(&mut self, env: &Env, database: Buffer) -> Result<()> {
        let new_reader = Self::reader_from_bytes(database.as_ref().to_vec())?;
        self.replace_reader(env, new_reader)
    }

    #[napi(js_name = "reloadFromFile")]
    pub fn reload_from_file(
        &mut self,
        env: &Env,
        path: String,
        mode: Option<String>,
    ) -> Result<()> {
        let new_reader = open_source(&path, mode.as_deref())?;
        self.replace_reader(env, new_reader)
    }

    #[napi(getter)]
    pub fn closed(&self) -> Result<bool> {
        Ok(self.reader.is_none())
    }

    #[napi]
    pub fn close(&mut self, env: &Env) -> Result<()> {
        self.clear_record_cache(env)?;
        self.clear_property_names(env)?;
        self.reader = None;
        Ok(())
    }

    #[napi(js_name = "clearCache")]
    pub fn clear_cache(&self, env: &Env) -> Result<()> {
        self.clear_record_cache(env)
    }

    #[napi(js_name = "cacheStats")]
    pub fn cache_stats<'env>(&self, env: &'env Env) -> Result<Object<'env>> {
        cache_stats_to_js(env, &self.cache)
    }

    #[napi]
    pub fn get<'env>(&self, env: &'env Env, ip_address: String) -> Result<Unknown<'env>> {
        let ip = self.parse_lookup_ip(&ip_address)?;
        let reader = self
            .reader
            .as_ref()
            .ok_or_else(|| invalid_arg(ERR_CLOSED_DB))?;
        reader.lookup_record_to_js(env, ip, &self.cache, &self.property_names)
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
        let reader = self
            .reader
            .as_ref()
            .ok_or_else(|| invalid_arg(ERR_CLOSED_DB))?;
        lookup_to_js(env, reader.lookup_path(ip, &path_elements))
    }

    #[napi(js_name = "compilePath")]
    pub fn compile_path(&self, path: Vec<Either<String, i64>>) -> Result<u32> {
        let path = parse_path(path)?;
        let mut paths = self
            .paths
            .try_borrow_mut()
            .map_err(|_| napi_error("path cache already borrowed"))?;
        let path_id =
            u32::try_from(paths.len()).map_err(|_| napi_error("too many compiled paths"))?;
        paths.push(path);
        Ok(path_id)
    }

    #[napi(js_name = "getCompiledPath")]
    pub fn get_compiled_path<'env>(
        &self,
        env: &'env Env,
        ip_address: String,
        path_id: u32,
    ) -> Result<Unknown<'env>> {
        let ip = self.parse_lookup_ip(&ip_address)?;
        let paths = self
            .paths
            .try_borrow()
            .map_err(|_| napi_error("path cache already borrowed"))?;
        let owned_path = compiled_path(&paths, path_id)?;
        let path_elements = path_elements_from_owned(owned_path);
        let reader = self
            .reader
            .as_ref()
            .ok_or_else(|| invalid_arg(ERR_CLOSED_DB))?;
        lookup_to_js(env, reader.lookup_path(ip, &path_elements))
    }

    #[napi(js_name = "getWithPrefixLength")]
    pub fn get_with_prefix_length<'env>(
        &self,
        env: &'env Env,
        ip_address: String,
    ) -> Result<Unknown<'env>> {
        let ip = self.parse_lookup_ip(&ip_address)?;
        let reader = self
            .reader
            .as_ref()
            .ok_or_else(|| invalid_arg(ERR_CLOSED_DB))?;
        let (js_value, prefix_len) =
            reader.lookup_record_with_prefix_to_js(env, ip, &self.cache, &self.property_names)?;
        let js_prefix = (prefix_len as u32).into_unknown(env)?;
        Array::from_vec(env, vec![js_value, js_prefix])?.into_unknown(env)
    }

    #[napi(js_name = "getMany")]
    pub fn get_many<'env>(&self, env: &'env Env, ips: Vec<String>) -> Result<Unknown<'env>> {
        let parsed_ips = self.parse_lookup_ips(ips)?;
        let reader = self
            .reader
            .as_ref()
            .ok_or_else(|| invalid_arg(ERR_CLOSED_DB))?;
        let values = parsed_ips
            .into_iter()
            .map(|ip| reader.lookup_record_to_js(env, ip, &self.cache, &self.property_names))
            .collect::<Result<Vec<_>>>()?;
        Array::from_vec(env, values)?.into_unknown(env)
    }

    #[napi(js_name = "getManyCompiledPath")]
    pub fn get_many_compiled_path<'env>(
        &self,
        env: &'env Env,
        ips: Vec<String>,
        path_id: u32,
    ) -> Result<Unknown<'env>> {
        let parsed_ips = self.parse_lookup_ips(ips)?;
        let paths = self
            .paths
            .try_borrow()
            .map_err(|_| napi_error("path cache already borrowed"))?;
        let owned_path = compiled_path(&paths, path_id)?;
        let path_elements = path_elements_from_owned(owned_path);
        let reader = self
            .reader
            .as_ref()
            .ok_or_else(|| invalid_arg(ERR_CLOSED_DB))?;
        let values = parsed_ips
            .into_iter()
            .map(|ip| lookup_to_js(env, reader.lookup_path(ip, &path_elements)))
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
        let reader = self
            .reader
            .as_ref()
            .ok_or_else(|| invalid_arg(ERR_CLOSED_DB))?;
        let values = parsed_ips
            .into_iter()
            .map(|ip| lookup_to_js(env, reader.lookup_path(ip, &path_elements)))
            .collect::<Result<Vec<_>>>()?;
        Array::from_vec(env, values)?.into_unknown(env)
    }

    #[napi]
    pub fn networks<'env>(
        &self,
        env: &'env Env,
        cidr: Option<String>,
        include_aliased_networks: Option<bool>,
        include_networks_without_data: Option<bool>,
        skip_empty_values: Option<bool>,
    ) -> Result<Unknown<'env>> {
        let cidr = cidr.as_deref().map(parse_network).transpose()?;
        let options = make_within_options(
            include_aliased_networks,
            include_networks_without_data,
            skip_empty_values,
        );
        let reader = self
            .reader
            .as_ref()
            .ok_or_else(|| invalid_arg(ERR_CLOSED_DB))?;
        let records = reader
            .collect_networks(cidr, options)
            .map_err(lookup_error)?;
        network_records_to_js(env, records)
    }

    #[napi(js_name = "networksPage")]
    #[allow(clippy::too_many_arguments)]
    pub fn networks_page<'env>(
        &self,
        env: &'env Env,
        cidr: Option<String>,
        include_aliased_networks: Option<bool>,
        include_networks_without_data: Option<bool>,
        skip_empty_values: Option<bool>,
        limit: u32,
        offset: u32,
    ) -> Result<Unknown<'env>> {
        let cidr = cidr.as_deref().map(parse_network).transpose()?;
        let options = make_within_options(
            include_aliased_networks,
            include_networks_without_data,
            skip_empty_values,
        );
        let reader = self
            .reader
            .as_ref()
            .ok_or_else(|| invalid_arg(ERR_CLOSED_DB))?;
        let page = reader
            .collect_networks_page(cidr, options, limit as usize, offset as usize)
            .map_err(lookup_error)?;
        network_record_page_to_js(env, page)
    }

    #[napi]
    pub fn metadata<'env>(&self, env: &'env Env) -> Result<Object<'env>> {
        let reader = self
            .reader
            .as_ref()
            .ok_or_else(|| invalid_arg(ERR_CLOSED_DB))?;
        metadata_to_js(env, reader.metadata())
    }
}

impl NativeReader {
    fn from_bytes(bytes: Vec<u8>, cache_capacity: Option<u32>) -> Result<Self> {
        let source = Self::reader_from_bytes(bytes)?;
        Ok(create_reader(source, cache_capacity))
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

    fn replace_reader(&mut self, env: &Env, new_reader: ReaderSource) -> Result<()> {
        self.clear_record_cache(env)?;
        self.ip_version = new_reader.metadata().ip_version;
        self.reader = Some(new_reader);
        Ok(())
    }

    fn clear_record_cache(&self, env: &Env) -> Result<()> {
        if let Some(cache) = self
            .cache
            .try_borrow_mut()
            .map_err(|_| napi_error("cache already borrowed"))?
            .as_mut()
        {
            cache.clear(env)?;
        }
        Ok(())
    }

    fn clear_property_names(&self, env: &Env) -> Result<()> {
        let _ = env;
        self.property_names
            .try_borrow_mut()
            .map_err(|_| napi_error("property name cache already borrowed"))?
            .clear();
        Ok(())
    }
}

impl ObjectFinalize for NativeReader {
    fn finalize(self, env: Env) -> Result<()> {
        self.clear_record_cache(&env)?;
        self.clear_property_names(&env)
    }
}

#[napi(js_name = "openReader")]
pub fn open_reader(
    path: String,
    mode: Option<String>,
    cache_capacity: Option<u32>,
) -> Result<NativeReader> {
    open_source(&path, mode.as_deref()).map(|source| create_reader(source, cache_capacity))
}

#[napi(js_name = "nativeVersion")]
pub fn native_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

fn create_reader(source: ReaderSource, cache_capacity: Option<u32>) -> NativeReader {
    let ip_version = source.metadata().ip_version;
    let cache = cache_capacity
        .and_then(|capacity| NonZeroUsize::new(capacity as usize))
        .map(RecordCache::new);
    NativeReader {
        reader: Some(source),
        cache: RefCell::new(cache),
        property_names: RefCell::new(PropertyNameCache::new()),
        paths: RefCell::new(Vec::new()),
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
