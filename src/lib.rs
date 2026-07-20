mod cache;
mod decode;
mod errors;
mod ip;
mod metadata;
mod networks;
mod paths;

use crate::{
    cache::{cache_stats_to_js, PropertyNameCache, RecordCache},
    decode::{lookup_result_path_to_js, lookup_result_paths_to_js, lookup_result_record_to_js},
    errors::{invalid_arg, lookup_error, napi_error, open_error},
    ip::{parse_js_ip, parse_network, prefix_len_for_lookup},
    metadata::metadata_to_js,
    networks::{collect_next_networks_page_to_js, make_within_options, NetworkIter},
    paths::{compiled_path, parse_path, path_elements_from_owned, OwnedPathElement, PathElements},
};
use maxminddb::{MaxMindDbError, Mmap, Reader as MaxMindReader, WithinOptions};
use memmap2::MmapOptions;
use napi::{
    bindgen_prelude::{
        Array, AsyncTask, Buffer, Either, Env, Object, ObjectFinalize, ToNapiValue, Unknown,
    },
    JsString, Result, Task,
};
use napi_derive::napi;
use std::{
    cell::{Ref, RefCell, RefMut},
    collections::HashMap,
    fs::File,
    io::Read,
    net::IpAddr,
    num::NonZeroUsize,
    path::Path,
    sync::Arc,
};

const ERR_CLOSED_DB: &str = "Attempt to read from a closed MaxMind DB.";
const ERR_REENTRANT_DB: &str = "MaxMind DB reader is already in use.";
const ERR_GZIP_DB: &str =
    "Looks like you are passing in a file in gzip format, please use mmdb database instead.";

enum ReaderSource {
    Mmap(MaxMindReader<Mmap>),
    Memory(MaxMindReader<Vec<u8>>),
}

pub struct OpenReaderTask {
    path: String,
    mode: String,
    cache_capacity: Option<u32>,
}

pub struct OpenReaderOutput(ReaderSource);

impl Task for OpenReaderTask {
    type Output = OpenReaderOutput;
    type JsValue = NativeReader;

    fn compute(&mut self) -> Result<Self::Output> {
        open_source(&self.path, Some(&self.mode)).map(OpenReaderOutput)
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> Result<Self::JsValue> {
        Ok(create_reader(output.0, self.cache_capacity))
    }
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
        cache: &mut Option<RecordCache>,
        property_names: &mut PropertyNameCache,
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
        cache: &mut Option<RecordCache>,
        property_names: &mut PropertyNameCache,
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
        property_names: &mut PropertyNameCache,
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

    fn lookup_paths_to_js<'env>(
        &self,
        env: &'env Env,
        ip: IpAddr,
        paths: &[PathElements<'_>],
        property_names: &mut PropertyNameCache,
    ) -> Result<Unknown<'env>> {
        match self {
            ReaderSource::Mmap(reader) => {
                let result = reader.lookup(ip).map_err(lookup_error)?;
                lookup_result_paths_to_js(env, &result, paths, property_names)
            }
            ReaderSource::Memory(reader) => {
                let result = reader.lookup(ip).map_err(lookup_error)?;
                lookup_result_paths_to_js(env, &result, paths, property_names)
            }
        }
    }

    fn metadata(&self) -> &maxminddb::Metadata {
        match self {
            ReaderSource::Mmap(reader) => reader.metadata(),
            ReaderSource::Memory(reader) => reader.metadata(),
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
    state: RefCell<NativeReaderState>,
}

struct NativeReaderState {
    reader: Option<Arc<ReaderSource>>,
    cache: Option<RecordCache>,
    property_names: PropertyNameCache,
    paths: HashMap<u32, Vec<OwnedPathElement>>,
    next_path_id: u32,
    ip_version: u16,
}

#[napi(js_name = "NativeNetworkCursor")]
pub struct NativeNetworkCursor {
    iter: Option<NetworkCursorCell>,
    property_names: PropertyNameCache,
    cache_records: bool,
    path: Option<Vec<OwnedPathElement>>,
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

        let path_elements = self.path.as_deref().map(path_elements_from_owned);
        let Some(iter) = self.iter.as_mut() else {
            return Array::from_vec(env, Vec::<Unknown<'env>>::new())?.into_unknown(env);
        };
        let (page, is_empty) = iter.with_dependent_mut(|_reader, iter| {
            collect_next_networks_page_to_js(
                env,
                iter,
                limit as usize,
                &mut self.property_names,
                self.cache_records,
                path_elements.as_deref(),
            )
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
    pub fn load(&self, env: &Env, database: Buffer) -> Result<()> {
        let new_reader = Self::reader_from_bytes(database.as_ref().to_vec())?;
        self.replace_reader(env, new_reader)
    }

    #[napi(js_name = "reloadFromFile")]
    pub fn reload_from_file(&self, env: &Env, path: String, mode: Option<String>) -> Result<()> {
        let new_reader = open_source(&path, mode.as_deref())?;
        self.replace_reader(env, new_reader)
    }

    #[napi(getter)]
    pub fn closed(&self) -> Result<bool> {
        Ok(self.state()?.reader.is_none())
    }

    #[napi]
    pub fn close(&self, env: &Env) -> Result<()> {
        let mut state = self.state_mut()?;
        state.clear_record_cache(env)?;
        state.clear_property_names();
        state.paths.clear();
        state.reader = None;
        Ok(())
    }

    #[napi(js_name = "clearCache")]
    pub fn clear_cache(&self, env: &Env) -> Result<()> {
        self.state_mut()?.clear_record_cache(env)
    }

    #[napi(js_name = "cacheStats")]
    pub fn cache_stats<'env>(&self, env: &'env Env) -> Result<Object<'env>> {
        cache_stats_to_js(env, &self.state()?.cache)
    }

    #[napi]
    pub fn get<'env>(&self, env: &'env Env, ip_address: JsString<'env>) -> Result<Unknown<'env>> {
        let ip = parse_js_ip(env, ip_address)?;
        let mut state = self.lookup_state(ip)?;
        let state = &mut *state;
        let reader = state
            .reader
            .as_ref()
            .ok_or_else(|| invalid_arg(ERR_CLOSED_DB))?;
        reader.lookup_record_to_js(env, ip, &mut state.cache, &mut state.property_names)
    }

    #[napi(js_name = "getPath")]
    pub fn get_path<'env>(
        &self,
        env: &'env Env,
        ip_address: JsString<'env>,
        path: Vec<Either<String, f64>>,
    ) -> Result<Unknown<'env>> {
        let ip = parse_js_ip(env, ip_address)?;
        let owned_path = parse_path(path)?;
        let path_elements = path_elements_from_owned(&owned_path);
        let mut state = self.lookup_state(ip)?;
        let state = &mut *state;
        let reader = state
            .reader
            .as_ref()
            .ok_or_else(|| invalid_arg(ERR_CLOSED_DB))?;
        reader.lookup_path_to_js(env, ip, &path_elements, &mut state.property_names)
    }

    #[napi(js_name = "getPaths")]
    pub fn get_paths<'env>(
        &self,
        env: &'env Env,
        ip_address: JsString<'env>,
        paths: Vec<Vec<Either<String, f64>>>,
    ) -> Result<Unknown<'env>> {
        let ip = parse_js_ip(env, ip_address)?;
        let owned_paths = parse_paths(paths)?;
        let path_elements = path_elements_from_paths(&owned_paths);
        let mut state = self.lookup_state(ip)?;
        let state = &mut *state;
        let reader = state
            .reader
            .as_ref()
            .ok_or_else(|| invalid_arg(ERR_CLOSED_DB))?;
        reader.lookup_paths_to_js(env, ip, &path_elements, &mut state.property_names)
    }

    #[napi(js_name = "compilePath")]
    pub fn compile_path(&self, path: Vec<Either<String, f64>>) -> Result<u32> {
        let mut state = self.state_mut()?;
        if state.reader.is_none() {
            return Err(invalid_arg(ERR_CLOSED_DB));
        }
        let path = parse_path(path)?;
        let path_id = state.next_path_id;
        let next_path_id = path_id
            .checked_add(1)
            .ok_or_else(|| napi_error("too many compiled paths"))?;
        state.paths.insert(path_id, path);
        state.next_path_id = next_path_id;
        Ok(path_id)
    }

    #[napi(js_name = "releasePath")]
    pub fn release_path(&self, path_id: u32) -> Result<()> {
        self.state_mut()?.paths.remove(&path_id);
        Ok(())
    }

    #[napi(js_name = "getCompiledPath")]
    pub fn get_compiled_path<'env>(
        &self,
        env: &'env Env,
        ip_address: JsString<'env>,
        path_id: u32,
    ) -> Result<Unknown<'env>> {
        let ip = parse_js_ip(env, ip_address)?;
        let mut state = self.lookup_state(ip)?;
        let state = &mut *state;
        let reader = state
            .reader
            .as_ref()
            .ok_or_else(|| invalid_arg(ERR_CLOSED_DB))?;
        let owned_path = compiled_path(&state.paths, path_id)?;
        let path_elements = path_elements_from_owned(owned_path);
        reader.lookup_path_to_js(env, ip, &path_elements, &mut state.property_names)
    }

    #[napi(js_name = "getWithPrefixLength")]
    pub fn get_with_prefix_length<'env>(
        &self,
        env: &'env Env,
        ip_address: JsString<'env>,
    ) -> Result<Unknown<'env>> {
        let ip = parse_js_ip(env, ip_address)?;
        let mut state = self.lookup_state(ip)?;
        let state = &mut *state;
        let reader = state
            .reader
            .as_ref()
            .ok_or_else(|| invalid_arg(ERR_CLOSED_DB))?;
        let (js_value, prefix_len) = reader.lookup_record_with_prefix_to_js(
            env,
            ip,
            &mut state.cache,
            &mut state.property_names,
        )?;
        let js_prefix = (prefix_len as u32).into_unknown(env)?;
        Array::from_vec(env, vec![js_value, js_prefix])?.into_unknown(env)
    }

    #[napi(js_name = "getMany")]
    pub fn get_many<'env>(&self, env: &'env Env, ips: Array<'env>) -> Result<Unknown<'env>> {
        let parsed_ips = self.parse_lookup_ips(env, &ips)?;
        let mut state = self.lookup_state_for_ips(&parsed_ips)?;
        let state = &mut *state;
        let reader = state
            .reader
            .as_ref()
            .ok_or_else(|| invalid_arg(ERR_CLOSED_DB))?;
        collect_lookup_results(env, parsed_ips, |ip| {
            reader.lookup_record_to_js(env, ip, &mut state.cache, &mut state.property_names)
        })
    }

    #[napi(js_name = "getManyCompiledPath")]
    pub fn get_many_compiled_path<'env>(
        &self,
        env: &'env Env,
        ips: Array<'env>,
        path_id: u32,
    ) -> Result<Unknown<'env>> {
        let parsed_ips = self.parse_lookup_ips(env, &ips)?;
        let mut state = self.lookup_state_for_ips(&parsed_ips)?;
        let state = &mut *state;
        let reader = state
            .reader
            .as_ref()
            .ok_or_else(|| invalid_arg(ERR_CLOSED_DB))?;
        let owned_path = compiled_path(&state.paths, path_id)?;
        let path_elements = path_elements_from_owned(owned_path);
        collect_lookup_results(env, parsed_ips, |ip| {
            reader.lookup_path_to_js(env, ip, &path_elements, &mut state.property_names)
        })
    }

    #[napi(js_name = "getManyPath")]
    pub fn get_many_path<'env>(
        &self,
        env: &'env Env,
        ips: Array<'env>,
        path: Vec<Either<String, f64>>,
    ) -> Result<Unknown<'env>> {
        let parsed_ips = self.parse_lookup_ips(env, &ips)?;
        let owned_path = parse_path(path)?;
        let path_elements = path_elements_from_owned(&owned_path);
        let mut state = self.lookup_state_for_ips(&parsed_ips)?;
        let state = &mut *state;
        let reader = state
            .reader
            .as_ref()
            .ok_or_else(|| invalid_arg(ERR_CLOSED_DB))?;
        collect_lookup_results(env, parsed_ips, |ip| {
            reader.lookup_path_to_js(env, ip, &path_elements, &mut state.property_names)
        })
    }

    #[napi(js_name = "getManyPaths")]
    pub fn get_many_paths<'env>(
        &self,
        env: &'env Env,
        ips: Array<'env>,
        paths: Vec<Vec<Either<String, f64>>>,
    ) -> Result<Unknown<'env>> {
        let parsed_ips = self.parse_lookup_ips(env, &ips)?;
        let owned_paths = parse_paths(paths)?;
        let path_elements = path_elements_from_paths(&owned_paths);
        let mut state = self.lookup_state_for_ips(&parsed_ips)?;
        let state = &mut *state;
        let reader = state
            .reader
            .as_ref()
            .ok_or_else(|| invalid_arg(ERR_CLOSED_DB))?;
        collect_lookup_results(env, parsed_ips, |ip| {
            reader.lookup_paths_to_js(env, ip, &path_elements, &mut state.property_names)
        })
    }

    #[napi(js_name = "networkCursor")]
    pub fn network_cursor(
        &self,
        cidr: Option<String>,
        include_aliased_networks: Option<bool>,
        include_networks_without_data: Option<bool>,
        skip_empty_values: Option<bool>,
        path: Option<Vec<Either<String, f64>>>,
    ) -> Result<NativeNetworkCursor> {
        let cidr = cidr.as_deref().map(parse_network).transpose()?;
        let path = path.map(parse_path).transpose()?;
        let options = make_within_options(
            include_aliased_networks,
            include_networks_without_data,
            skip_empty_values,
        );
        let state = self.state()?;
        let reader = Arc::clone(
            state
                .reader
                .as_ref()
                .ok_or_else(|| invalid_arg(ERR_CLOSED_DB))?,
        );
        let iter = NetworkCursorCell::try_new(reader, |reader| reader.network_iter(cidr, options))
            .map_err(lookup_error)?;
        Ok(NativeNetworkCursor {
            iter: Some(iter),
            property_names: PropertyNameCache::new(),
            cache_records: state.cache.is_some(),
            path,
        })
    }

    #[napi]
    pub fn metadata<'env>(&self, env: &'env Env) -> Result<Object<'env>> {
        let state = self.state()?;
        let reader = state
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
        reader_from_bytes(bytes).map(ReaderSource::Memory)
    }

    fn parse_lookup_ips(&self, env: &Env, ips: &Array<'_>) -> Result<Vec<IpAddr>> {
        (0..ips.len())
            .map(|index| {
                let ip = ips
                    .get::<JsString<'_>>(index)?
                    .ok_or_else(|| invalid_arg("missing IP address array element"))?;
                parse_js_ip(env, ip)
            })
            .collect()
    }

    fn lookup_state(&self, ip: IpAddr) -> Result<RefMut<'_, NativeReaderState>> {
        let state = self.state_mut()?;
        validate_lookup_ip(state.ip_version, ip)?;
        Ok(state)
    }

    fn lookup_state_for_ips(&self, ips: &[IpAddr]) -> Result<RefMut<'_, NativeReaderState>> {
        let state = self.state_mut()?;
        for &ip in ips {
            validate_lookup_ip(state.ip_version, ip)?;
        }
        Ok(state)
    }

    fn replace_reader(&self, env: &Env, new_reader: ReaderSource) -> Result<()> {
        self.state_mut()?.replace_reader(env, new_reader)
    }

    fn state(&self) -> Result<Ref<'_, NativeReaderState>> {
        self.state
            .try_borrow()
            .map_err(|_| napi_error(ERR_REENTRANT_DB))
    }

    fn state_mut(&self) -> Result<RefMut<'_, NativeReaderState>> {
        self.state
            .try_borrow_mut()
            .map_err(|_| napi_error(ERR_REENTRANT_DB))
    }
}

impl NativeReaderState {
    fn replace_reader(&mut self, env: &Env, new_reader: ReaderSource) -> Result<()> {
        self.clear_record_cache(env)?;
        self.ip_version = new_reader.metadata().ip_version;
        self.reader = Some(Arc::new(new_reader));
        Ok(())
    }

    fn clear_record_cache(&mut self, env: &Env) -> Result<()> {
        if let Some(cache) = self.cache.as_mut() {
            cache.clear(env)?;
        }
        Ok(())
    }

    fn clear_property_names(&mut self) {
        self.property_names.clear();
    }
}

fn validate_lookup_ip(ip_version: u16, ip: IpAddr) -> Result<()> {
    if ip_version == 4 && matches!(ip, IpAddr::V6(_)) {
        return Err(invalid_arg(format!(
            "Error looking up {ip}. You attempted to look up an IPv6 address in an IPv4-only database"
        )));
    }
    Ok(())
}

fn collect_lookup_results<'env>(
    env: &'env Env,
    ips: Vec<IpAddr>,
    mut lookup: impl FnMut(IpAddr) -> Result<Unknown<'env>>,
) -> Result<Unknown<'env>> {
    let length = u32::try_from(ips.len()).map_err(|_| invalid_arg("too many IP addresses"))?;
    let mut values = env.create_array(length)?;
    for (index, ip) in ips.into_iter().enumerate() {
        values.set(index as u32, lookup(ip)?)?;
    }
    values.into_unknown(env)
}

fn parse_paths(paths: Vec<Vec<Either<String, f64>>>) -> Result<Vec<Vec<OwnedPathElement>>> {
    paths.into_iter().map(parse_path).collect()
}

fn path_elements_from_paths(paths: &[Vec<OwnedPathElement>]) -> Vec<PathElements<'_>> {
    paths
        .iter()
        .map(|path| path_elements_from_owned(path))
        .collect()
}

impl ObjectFinalize for NativeReader {
    fn finalize(mut self, env: Env) -> Result<()> {
        let state = self.state.get_mut();
        state.clear_record_cache(&env)?;
        state.clear_property_names();
        Ok(())
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

#[napi(js_name = "openReaderAsync")]
pub fn open_reader_async(
    path: String,
    mode: String,
    cache_capacity: Option<u32>,
) -> AsyncTask<OpenReaderTask> {
    AsyncTask::new(OpenReaderTask {
        path,
        mode,
        cache_capacity,
    })
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
    let state = NativeReaderState {
        reader: Some(Arc::new(source)),
        cache,
        property_names: PropertyNameCache::new(),
        paths: HashMap::new(),
        next_path_id: 0,
        ip_version,
    };
    NativeReader {
        state: RefCell::new(state),
    }
}

fn open_source(path: &str, mode: Option<&str>) -> Result<ReaderSource> {
    match mode.unwrap_or("mmap") {
        "auto" | "mmap" => open_mmap_reader(Path::new(path)).map(ReaderSource::Mmap),
        "memory" | "buffer" => open_memory_reader(Path::new(path)).map(ReaderSource::Memory),
        other => Err(invalid_arg(format!("Unsupported open mode: {other}"))),
    }
}

fn open_mmap_reader(path: &Path) -> Result<MaxMindReader<Mmap>> {
    let file = File::open(path)
        .map_err(MaxMindDbError::Io)
        .map_err(open_error)?;
    // SAFETY: The mapping is read-only. Callers should replace database files
    // atomically rather than mutating an open file in place.
    let mmap = unsafe { MmapOptions::new().map(&file) }
        .map_err(MaxMindDbError::Mmap)
        .map_err(open_error)?;
    reject_gzip(mmap.as_ref())?;
    MaxMindReader::from_source(mmap).map_err(open_error)
}

fn open_memory_reader(path: &Path) -> Result<MaxMindReader<Vec<u8>>> {
    let mut file = File::open(path)
        .map_err(MaxMindDbError::Io)
        .map_err(open_error)?;
    let mut prefix = [0_u8; 2];
    let mut prefix_len = 0;
    while prefix_len < prefix.len() {
        let read = file
            .read(&mut prefix[prefix_len..])
            .map_err(MaxMindDbError::Io)
            .map_err(open_error)?;
        if read == 0 {
            break;
        }
        prefix_len += read;
    }
    reject_gzip(&prefix[..prefix_len])?;

    let capacity = file
        .metadata()
        .ok()
        .and_then(|metadata| usize::try_from(metadata.len()).ok())
        .unwrap_or(prefix_len);
    let mut bytes = Vec::with_capacity(capacity);
    bytes.extend_from_slice(&prefix[..prefix_len]);
    file.read_to_end(&mut bytes)
        .map_err(MaxMindDbError::Io)
        .map_err(open_error)?;
    reader_from_bytes(bytes)
}

fn reader_from_bytes(bytes: Vec<u8>) -> Result<MaxMindReader<Vec<u8>>> {
    reject_gzip(&bytes)?;
    MaxMindReader::from_source(bytes).map_err(open_error)
}

fn reject_gzip(bytes: &[u8]) -> Result<()> {
    if bytes.starts_with(&[0x1f, 0x8b]) {
        return Err(napi_error(ERR_GZIP_DB));
    }
    Ok(())
}
