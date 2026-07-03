mod cache;
mod decode;
mod errors;
mod ip;
mod metadata;
mod networks;
mod paths;

use crate::{
    cache::{cache_stats_to_js, PropertyNameCache, RecordCache},
    decode::{lookup_result_path_to_js, lookup_result_record_to_js},
    errors::{invalid_arg, lookup_error, napi_error, open_error},
    ip::{parse_ip, parse_network, prefix_len_for_lookup},
    metadata::metadata_to_js,
    networks::{
        collect_networks_for_reader_to_js, collect_next_networks_page_to_js, make_within_options,
        NetworkIter,
    },
    paths::{compiled_path, parse_path, path_elements_from_owned, OwnedPathElement},
};
use maxminddb::{MaxMindDbError, Mmap, Reader as MaxMindReader, WithinOptions};
use napi::{
    bindgen_prelude::{Array, Buffer, Either, Env, Object, ObjectFinalize, ToNapiValue, Unknown},
    Result,
};
use napi_derive::napi;
use std::{cell::RefCell, net::IpAddr, num::NonZeroUsize, path::Path, sync::Arc};

const ERR_CLOSED_DB: &str = "Attempt to read from a closed MaxMind DB.";

enum ReaderSource {
    Mmap(MaxMindReader<Mmap>),
    Memory(MaxMindReader<Vec<u8>>),
}

// The network iterator borrows from its reader. Keep the Arc-owned reader and
// the borrowing iterator together so cursor snapshots remain valid after the
// parent reader reloads or closes.
self_cell::self_cell!(
    struct NetworkCursorCell {
        owner: Arc<ReaderSource>,

        #[covariant]
        dependent: NetworkIter,
    }
);

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

    fn lookup_path_to_js<'env>(
        &self,
        env: &'env Env,
        ip: IpAddr,
        path: &[maxminddb::PathElement<'_>],
        property_names: &RefCell<PropertyNameCache>,
    ) -> Result<Unknown<'env>> {
        match self {
            ReaderSource::Mmap(reader) => {
                let result = reader.lookup(ip).map_err(lookup_error)?;
                lookup_result_path_to_js(env, &result, path, property_names)
            }
            ReaderSource::Memory(reader) => {
                let result = reader.lookup(ip).map_err(lookup_error)?;
                lookup_result_path_to_js(env, &result, path, property_names)
            }
        }
    }

    fn metadata(&self) -> &maxminddb::Metadata {
        match self {
            ReaderSource::Mmap(reader) => reader.metadata(),
            ReaderSource::Memory(reader) => reader.metadata(),
        }
    }

    fn collect_networks_to_js<'env>(
        &self,
        env: &'env Env,
        cidr: Option<ipnetwork::IpNetwork>,
        options: WithinOptions,
        property_names: &RefCell<PropertyNameCache>,
    ) -> Result<Unknown<'env>> {
        match self {
            ReaderSource::Mmap(reader) => {
                collect_networks_for_reader_to_js(env, reader, cidr, options, property_names)
            }
            ReaderSource::Memory(reader) => {
                collect_networks_for_reader_to_js(env, reader, cidr, options, property_names)
            }
        }
    }

    fn network_iter(
        &self,
        cidr: Option<ipnetwork::IpNetwork>,
        options: WithinOptions,
    ) -> std::result::Result<NetworkIter<'_>, MaxMindDbError> {
        match self {
            ReaderSource::Mmap(reader) => NetworkIter::from_mmap(reader, cidr, options),
            ReaderSource::Memory(reader) => NetworkIter::from_memory(reader, cidr, options),
        }
    }
}

#[napi(js_name = "NativeReader", custom_finalize)]
pub struct NativeReader {
    reader: Option<Arc<ReaderSource>>,
    cache: RefCell<Option<RecordCache>>,
    property_names: RefCell<PropertyNameCache>,
    paths: RefCell<Vec<Vec<OwnedPathElement>>>,
    ip_version: u16,
}

#[napi(js_name = "NativeNetworkCursor")]
pub struct NativeNetworkCursor {
    iter: Option<NetworkCursorCell>,
    property_names: RefCell<PropertyNameCache>,
}

impl Drop for NativeNetworkCursor {
    fn drop(&mut self) {
        self.iter.take();
    }
}

#[napi]
impl NativeNetworkCursor {
    #[napi(js_name = "nextPage")]
    pub fn next_page<'env>(&mut self, env: &'env Env, limit: u32) -> Result<Unknown<'env>> {
        if limit == 0 {
            return Err(invalid_arg("page size should be a positive 32-bit integer"));
        }

        let Some(iter) = self.iter.as_mut() else {
            return Array::from_vec(env, Vec::<Unknown<'env>>::new())?.into_unknown(env);
        };
        let (page, is_empty) = iter.with_dependent_mut(|_reader, iter| {
            collect_next_networks_page_to_js(env, iter, limit as usize, &self.property_names)
        })?;
        if is_empty {
            self.iter = None;
        }
        Ok(page)
    }

    #[napi]
    pub fn close(&mut self) -> Result<()> {
        self.iter = None;
        Ok(())
    }
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
        reader.lookup_path_to_js(env, ip, &path_elements, &self.property_names)
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
        reader.lookup_path_to_js(env, ip, &path_elements, &self.property_names)
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
            .map(|ip| reader.lookup_path_to_js(env, ip, &path_elements, &self.property_names))
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
            .map(|ip| reader.lookup_path_to_js(env, ip, &path_elements, &self.property_names))
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
        reader.collect_networks_to_js(env, cidr, options, &self.property_names)
    }

    #[napi(js_name = "networkCursor")]
    pub fn network_cursor(
        &self,
        cidr: Option<String>,
        include_aliased_networks: Option<bool>,
        include_networks_without_data: Option<bool>,
        skip_empty_values: Option<bool>,
    ) -> Result<NativeNetworkCursor> {
        let cidr = cidr.as_deref().map(parse_network).transpose()?;
        let options = make_within_options(
            include_aliased_networks,
            include_networks_without_data,
            skip_empty_values,
        );
        let reader = Arc::clone(
            self.reader
                .as_ref()
                .ok_or_else(|| invalid_arg(ERR_CLOSED_DB))?,
        );
        let iter = NetworkCursorCell::try_new(reader, |reader| reader.network_iter(cidr, options))
            .map_err(lookup_error)?;
        Ok(NativeNetworkCursor {
            iter: Some(iter),
            property_names: RefCell::new(PropertyNameCache::new()),
        })
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
        self.reader = Some(Arc::new(new_reader));
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
        reader: Some(Arc::new(source)),
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
