mod bound;
mod cache;
mod decode;
mod errors;
mod ip;
mod metadata;
mod networks;
mod paths;

use crate::{
    cache::{cache_stats_to_js, PropertyNameCache, RecordCache},
    decode::{
        lookup_result_path_to_js, lookup_result_paths_to_js, lookup_result_record_to_js,
        JsDecodeEnvGuard,
    },
    errors::{invalid_arg, lookup_error, napi_error, open_error, FileError, OpenError, OpenResult},
    ip::{parse_js_ip, parse_network, prefix_len_for_lookup},
    metadata::metadata_to_js,
    networks::{collect_next_networks_page_to_js, make_within_options, NetworkIter},
    paths::{compiled_path, parse_path, path_elements_from_owned, OwnedPathElement, PathElements},
};
use maxminddb::{MaxMindDbError, Mmap, Reader as MaxMindReader, WithinOptions};
use memmap2::MmapOptions;
use napi::{
    bindgen_prelude::{
        Array, AsyncTask, Buffer, Either, Env, JsObjectValue, Object, ObjectFinalize, Reference,
        ToNapiValue, Unknown,
    },
    JsString, JsValue, Result, Task,
};
use napi_derive::napi;
use std::{
    cell::{Ref, RefCell, RefMut},
    collections::HashMap,
    fs::File,
    io::Read,
    net::IpAddr,
    num::NonZeroUsize,
    ops::Deref,
    path::Path,
    rc::Rc,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex, PoisonError,
    },
};

/// How many batch results or network-page entries to convert before closing
/// the handle scope that holds their temporary handles. Opening a scope has a
/// fixed cost, so one scope per value slows small batches, while a scope per
/// call lets handles pile up and slows garbage collection in large ones.
pub(crate) const VALUES_PER_HANDLE_SCOPE: usize = 64;

const ERR_CLOSED_DB: &str = "Attempt to read from a closed MaxMind DB.";
const ERR_REENTRANT_DB: &str = "MaxMind DB reader is already in use.";
const ERR_GZIP_DB: &str =
    "Looks like you are passing in a file in gzip format, please use mmdb database instead.";

// Process-wide counter taken when a database read begins (when the file is
// opened, or when a buffer is loaded). The JavaScript Reader compares these
// numbers so an asynchronous reload that read its file earlier never replaces
// a database that was read later.
static READ_SEQUENCE: AtomicU64 = AtomicU64::new(0);

// Held across taking a sequence number and opening the file, so sequence
// order matches the order in which files are opened, and therefore which
// version of an atomically replaced file each reader sees. Only the open
// itself runs under the lock; reading and mapping the file do not.
static OPEN_LOCK: Mutex<()> = Mutex::new(());

fn next_read_sequence() -> u64 {
    READ_SEQUENCE.fetch_add(1, Ordering::SeqCst) + 1
}

// A database read's result, with the read sequence taken before it failed
// when the failure happened after the read began.
type ReadResult<T> = std::result::Result<(T, u64), ReadFailure>;
type ReadFailure = (OpenError, Option<u64>);

fn open_file_with_sequence(path: &str) -> ReadResult<File> {
    let _guard = OPEN_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    let read_sequence = next_read_sequence();
    let file = File::open(Path::new(path))
        .map_err(|error| (open_file_error(&error, path).into(), Some(read_sequence)))?;
    Ok((file, read_sequence))
}

// Converts a failed database read into a JavaScript error. File system
// failures keep `code`, `syscall` and `path`; both kinds carry the read
// sequence when there is one.
fn read_failure_error(env: &Env, (error, read_sequence): ReadFailure) -> napi::Error {
    match error {
        OpenError::Other(error) => error_with_read_sequence(env, error, read_sequence),
        OpenError::File(file_error) => file_error.into_napi(env, read_sequence),
    }
}

// Attaches the failed read's sequence to the JavaScript error as
// `readSequence`, so the Reader can order the failure against other reloads.
fn error_with_read_sequence(
    env: &Env,
    error: napi::Error,
    read_sequence: Option<u64>,
) -> napi::Error {
    let Some(read_sequence) = read_sequence else {
        return error;
    };
    match env.create_error(error) {
        Ok(mut object) => {
            // If the property cannot be set, the Reader falls back to ordering
            // the failure by when the attempt began.
            let _ = object.set_named_property("readSequence", read_sequence as f64);
            napi::Error::from(object.to_unknown())
        }
        Err(error) => error,
    }
}

enum ReaderSource {
    Mmap(MaxMindReader<Mmap>),
    /// A database copied onto the Rust heap, with the size of its allocation.
    Memory(MaxMindReader<Vec<u8>>, usize),
}

macro_rules! dispatch_reader {
    ($source:expr, $reader:ident => $body:expr) => {
        match $source {
            ReaderSource::Mmap($reader) => $body,
            ReaderSource::Memory($reader, _) => $body,
        }
    };
}

/// Native memory reported to V8 with `napi_adjust_external_memory`, so the
/// garbage collector sees the true cost of a small JavaScript object that owns
/// a large native allocation. Dropping it reverses the adjustment.
///
/// It keeps the `Env` it was created with, so it must be dropped on that
/// environment's JavaScript thread. Owners share it through `Rc`, which keeps
/// it on that thread.
struct ExternalMemory {
    env: Env,
    bytes: i64,
}

impl ExternalMemory {
    fn report(env: &Env, bytes: usize) -> Option<Self> {
        let bytes = i64::try_from(bytes).ok().filter(|&bytes| bytes > 0)?;
        // The adjustment is only a hint to the garbage collector. If it fails,
        // keep the reader usable and skip the matching release.
        env.adjust_external_memory(bytes).ok()?;
        Some(Self { env: *env, bytes })
    }
}

impl Drop for ExternalMemory {
    fn drop(&mut self) {
        let _ = self.env.adjust_external_memory(-self.bytes);
    }
}

/// A reader source shared by a `NativeReader` and the network cursors created
/// from it. The last owner to drop it frees the database and releases any
/// external memory it reported.
struct SharedSource {
    source: ReaderSource,
    // Declared after `source`, so the database is freed before V8 is told.
    _external_memory: Option<ExternalMemory>,
}

impl SharedSource {
    fn new(env: &Env, source: ReaderSource) -> Rc<Self> {
        let external_memory = ExternalMemory::report(env, source.heap_bytes());
        Rc::new(Self {
            source,
            _external_memory: external_memory,
        })
    }
}

impl Deref for SharedSource {
    type Target = ReaderSource;

    fn deref(&self) -> &ReaderSource {
        &self.source
    }
}

pub struct OpenReaderTask {
    path: String,
    mode: String,
    cache_capacity: Option<u32>,
    failed_read_sequence: Option<u64>,
    // A file error found off the JS thread; `reject` turns it into a JS error
    // with `code`, `syscall` and `path` set.
    file_error: Option<FileError>,
}

pub struct OpenReaderOutput(ReaderSource, u64);

impl Task for OpenReaderTask {
    type Output = OpenReaderOutput;
    type JsValue = NativeReader;

    fn compute(&mut self) -> Result<Self::Output> {
        match open_source(&self.path, Some(&self.mode)) {
            Ok((source, read_sequence)) => Ok(OpenReaderOutput(source, read_sequence)),
            Err((error, read_sequence)) => {
                self.failed_read_sequence = read_sequence;
                match error {
                    OpenError::Other(error) => Err(error),
                    OpenError::File(file_error) => {
                        let message = file_error.message();
                        self.file_error = Some(file_error);
                        Err(napi_error(message))
                    }
                }
            }
        }
    }

    fn resolve(&mut self, env: Env, output: Self::Output) -> Result<Self::JsValue> {
        Ok(create_reader(&env, output.0, output.1, self.cache_capacity))
    }

    fn reject(&mut self, env: Env, error: napi::Error) -> Result<Self::JsValue> {
        Err(match self.file_error.take() {
            Some(file_error) => file_error.into_napi(&env, self.failed_read_sequence),
            None => error_with_read_sequence(&env, error, self.failed_read_sequence),
        })
    }
}

// The network iterator borrows from its reader. Keep the Rc-owned reader and
// the borrowing iterator together so cursor snapshots remain valid after the
// parent reader reloads or closes.
self_cell::self_cell!(
    struct NetworkCursorCell {
        owner: Rc<SharedSource>,

        #[covariant]
        dependent: NetworkIter,
    }
);

impl ReaderSource {
    /// Bytes this source holds on the Rust heap. Memory-mapped databases are
    /// backed by the file and are not counted.
    fn heap_bytes(&self) -> usize {
        match self {
            ReaderSource::Mmap(_) => 0,
            ReaderSource::Memory(_, bytes) => *bytes,
        }
    }

    fn lookup_record_to_js<'env>(
        &self,
        env: &'env Env,
        ip: IpAddr,
        cache: &mut Option<RecordCache>,
        guard: &JsDecodeEnvGuard<'_>,
    ) -> Result<Unknown<'env>> {
        dispatch_reader!(self, reader => {
            let result = reader.lookup(ip).map_err(lookup_error)?;
            lookup_result_record_to_js(env, &result, cache, guard)
        })
    }

    fn lookup_record_with_prefix_to_js<'env>(
        &self,
        env: &'env Env,
        ip: IpAddr,
        cache: &mut Option<RecordCache>,
        guard: &JsDecodeEnvGuard<'_>,
    ) -> Result<(Unknown<'env>, usize)> {
        dispatch_reader!(self, reader => {
            let result = reader.lookup(ip).map_err(lookup_error)?;
            let network = result.network().map_err(lookup_error)?;
            let prefix = prefix_len_for_lookup(ip, network);
            Ok((
                lookup_result_record_to_js(env, &result, cache, guard)?,
                prefix,
            ))
        })
    }

    fn lookup_path_to_js<'env>(
        &self,
        env: &'env Env,
        ip: IpAddr,
        path: &[maxminddb::PathElement<'_>],
        guard: &JsDecodeEnvGuard<'_>,
    ) -> Result<Unknown<'env>> {
        dispatch_reader!(self, reader => {
            let result = reader.lookup(ip).map_err(lookup_error)?;
            lookup_result_path_to_js(env, &result, path, guard)
        })
    }

    fn lookup_paths_to_js<'env>(
        &self,
        env: &'env Env,
        ip: IpAddr,
        paths: &[PathElements<'_>],
        guard: &JsDecodeEnvGuard<'_>,
    ) -> Result<Unknown<'env>> {
        dispatch_reader!(self, reader => {
            let result = reader.lookup(ip).map_err(lookup_error)?;
            lookup_result_paths_to_js(env, &result, paths, guard)
        })
    }

    fn metadata(&self) -> &maxminddb::Metadata {
        dispatch_reader!(self, reader => reader.metadata())
    }

    fn network_iter(
        &self,
        cidr: Option<ipnetwork::IpNetwork>,
        options: WithinOptions,
    ) -> std::result::Result<NetworkIter<'_>, MaxMindDbError> {
        match self {
            ReaderSource::Mmap(reader) => NetworkIter::from_mmap(reader, cidr, options),
            ReaderSource::Memory(reader, _) => NetworkIter::from_memory(reader, cidr, options),
        }
    }
}

#[napi(js_name = "NativeReader", custom_finalize)]
pub struct NativeReader {
    state: RefCell<NativeReaderState>,
}

struct NativeReaderState {
    reader: Option<Rc<SharedSource>>,
    cache: Option<RecordCache>,
    property_names: PropertyNameCache,
    paths: HashMap<u32, Vec<OwnedPathElement>>,
    next_path_id: u32,
    ip_version: u16,
    read_sequence: u64,
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
    pub fn new(env: Env, database: Buffer, cache_capacity: Option<u32>) -> Result<Self> {
        let read_sequence = next_read_sequence();
        let source = memory_source(database.as_ref().to_vec())?;
        Ok(create_reader(&env, source, read_sequence, cache_capacity))
    }

    #[napi]
    pub fn load(&self, env: &Env, database: Buffer) -> Result<()> {
        if self.state()?.reader.is_none() {
            return Err(invalid_arg(ERR_CLOSED_DB));
        }
        let read_sequence = next_read_sequence();
        let new_reader = memory_source(database.as_ref().to_vec())
            .map_err(|error| error_with_read_sequence(env, error, Some(read_sequence)))?;
        self.replace_reader(env, new_reader, read_sequence)
    }

    #[napi(js_name = "reloadFromFile")]
    pub fn reload_from_file(&self, env: &Env, path: String, mode: Option<String>) -> Result<()> {
        if self.state()?.reader.is_none() {
            return Err(invalid_arg(ERR_CLOSED_DB));
        }
        let (new_reader, read_sequence) = open_source(&path, mode.as_deref())
            .map_err(|failure| read_failure_error(env, failure))?;
        self.replace_reader(env, new_reader, read_sequence)
    }

    #[napi(getter)]
    pub fn closed(&self) -> Result<bool> {
        Ok(self.state()?.reader.is_none())
    }

    /// Order in which the current database's read began, relative to every
    /// other database read in the process. Larger numbers were read later.
    #[napi(getter, js_name = "readSequence")]
    pub fn read_sequence(&self) -> Result<f64> {
        Ok(self.state()?.read_sequence as f64)
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
        let guard = JsDecodeEnvGuard::enter(env.raw(), &mut state.property_names);
        reader.lookup_record_to_js(env, ip, &mut state.cache, &guard)
    }

    #[napi(js_name = "bindGet")]
    pub fn bind_get<'env>(
        &self,
        env: &'env Env,
        reader: Reference<NativeReader>,
    ) -> Result<Unknown<'env>> {
        bound::bind_get(env, reader)
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
        let guard = JsDecodeEnvGuard::enter(env.raw(), &mut state.property_names);
        reader.lookup_path_to_js(env, ip, &path_elements, &guard)
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
        let guard = JsDecodeEnvGuard::enter(env.raw(), &mut state.property_names);
        reader.lookup_paths_to_js(env, ip, &path_elements, &guard)
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
        let guard = JsDecodeEnvGuard::enter(env.raw(), &mut state.property_names);
        reader.lookup_path_to_js(env, ip, &path_elements, &guard)
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
        let guard = JsDecodeEnvGuard::enter(env.raw(), &mut state.property_names);
        let (js_value, prefix_len) =
            reader.lookup_record_with_prefix_to_js(env, ip, &mut state.cache, &guard)?;
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
        let guard = JsDecodeEnvGuard::enter(env.raw(), &mut state.property_names);
        collect_lookup_results(env, parsed_ips, |ip| {
            reader.lookup_record_to_js(env, ip, &mut state.cache, &guard)
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
        let guard = JsDecodeEnvGuard::enter(env.raw(), &mut state.property_names);
        collect_lookup_results(env, parsed_ips, |ip| {
            reader.lookup_path_to_js(env, ip, &path_elements, &guard)
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
        let guard = JsDecodeEnvGuard::enter(env.raw(), &mut state.property_names);
        collect_lookup_results(env, parsed_ips, |ip| {
            reader.lookup_path_to_js(env, ip, &path_elements, &guard)
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
        let guard = JsDecodeEnvGuard::enter(env.raw(), &mut state.property_names);
        collect_lookup_results(env, parsed_ips, |ip| {
            reader.lookup_paths_to_js(env, ip, &path_elements, &guard)
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
        let reader = Rc::clone(
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
        if state.reader.is_none() {
            return Err(invalid_arg(ERR_CLOSED_DB));
        }
        validate_lookup_ip(state.ip_version, ip)?;
        Ok(state)
    }

    fn lookup_state_for_ips(&self, ips: &[IpAddr]) -> Result<RefMut<'_, NativeReaderState>> {
        let state = self.state_mut()?;
        if state.reader.is_none() {
            return Err(invalid_arg(ERR_CLOSED_DB));
        }
        for &ip in ips {
            validate_lookup_ip(state.ip_version, ip)?;
        }
        Ok(state)
    }

    fn replace_reader(
        &self,
        env: &Env,
        new_reader: ReaderSource,
        read_sequence: u64,
    ) -> Result<()> {
        self.state_mut()?
            .replace_reader(env, new_reader, read_sequence)
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
    fn replace_reader(
        &mut self,
        env: &Env,
        new_reader: ReaderSource,
        read_sequence: u64,
    ) -> Result<()> {
        if self.reader.is_none() {
            return Err(invalid_arg(ERR_CLOSED_DB));
        }
        self.clear_record_cache(env)?;
        self.clear_property_names();
        self.ip_version = new_reader.metadata().ip_version;
        self.reader = Some(SharedSource::new(env, new_reader));
        self.read_sequence = read_sequence;
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
    // Decoded values are reachable from `values` once stored, so their handles
    // (and the intermediate handles created while decoding them) can be
    // released. Without nested scopes they all stay live until the call
    // returns, and every GC during a large batch has to scan them.
    let mut index: u32 = 0;
    for chunk in ips.chunks(VALUES_PER_HANDLE_SCOPE) {
        env.run_in_scope(|| {
            for &ip in chunk {
                values.set(index, lookup(ip)?)?;
                index += 1;
            }
            Ok(())
        })?;
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
    env: Env,
    path: String,
    mode: Option<String>,
    cache_capacity: Option<u32>,
) -> Result<NativeReader> {
    open_source(&path, mode.as_deref())
        .map(|(source, read_sequence)| create_reader(&env, source, read_sequence, cache_capacity))
        .map_err(|(error, _)| read_failure_error(&env, (error, None)))
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
        failed_read_sequence: None,
        file_error: None,
    })
}

/// The most recently taken read sequence number. Any database read that
/// begins after this call gets a larger number.
#[napi(js_name = "currentReadSequence")]
pub fn current_read_sequence() -> f64 {
    READ_SEQUENCE.load(Ordering::SeqCst) as f64
}

#[napi(js_name = "nativeVersion")]
pub fn native_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

fn create_reader(
    env: &Env,
    source: ReaderSource,
    read_sequence: u64,
    cache_capacity: Option<u32>,
) -> NativeReader {
    let ip_version = source.metadata().ip_version;
    let cache = cache_capacity
        .and_then(|capacity| NonZeroUsize::new(capacity as usize))
        .map(RecordCache::new);
    let state = NativeReaderState {
        reader: Some(SharedSource::new(env, source)),
        cache,
        property_names: PropertyNameCache::new(),
        paths: HashMap::new(),
        next_path_id: 0,
        ip_version,
        read_sequence,
    };
    NativeReader {
        state: RefCell::new(state),
    }
}

fn open_source(path: &str, mode: Option<&str>) -> ReadResult<ReaderSource> {
    let mode = mode.unwrap_or("mmap");
    if !matches!(mode, "auto" | "mmap" | "memory" | "buffer") {
        return Err((
            invalid_arg(format!("Unsupported open mode: {mode}")).into(),
            None,
        ));
    }
    let (file, read_sequence) = open_file_with_sequence(path)?;
    let source = read_database(file, path, matches!(mode, "auto" | "mmap"))
        .map_err(|error| (error, Some(read_sequence)))?;
    Ok((source, read_sequence))
}

fn read_database(file: File, path: &str, mmap: bool) -> OpenResult<ReaderSource> {
    let len = database_len(&file, path)?;
    if mmap {
        open_mmap_reader(&file, len, path).map(ReaderSource::Mmap)
    } else {
        open_memory_reader(file, len, path)
    }
}

/// Returns the length of an opened database file and rejects directories, so
/// every mode reports a directory the same way.
fn database_len(file: &File, path: &str) -> OpenResult<u64> {
    let metadata = file
        .metadata()
        .map_err(|err| FileError::from_io(&err, "fstat", path))?;
    if metadata.is_dir() {
        return Err(FileError::is_a_directory("read", path).into());
    }
    Ok(metadata.len())
}

#[cfg(windows)]
fn open_file_error(err: &std::io::Error, path: &str) -> FileError {
    // Windows refuses to open a directory as a file with "access denied".
    if err.kind() == std::io::ErrorKind::PermissionDenied && Path::new(path).is_dir() {
        FileError::is_a_directory("open", path)
    } else {
        FileError::from_io(err, "open", path)
    }
}

#[cfg(not(windows))]
fn open_file_error(err: &std::io::Error, path: &str) -> FileError {
    FileError::from_io(err, "open", path)
}

fn open_mmap_reader(file: &File, len: u64, path: &str) -> OpenResult<MaxMindReader<Mmap>> {
    let len = usize::try_from(len).map_err(|_| FileError::too_large("mmap", path))?;
    // SAFETY: The mapping is read-only. Callers should replace database files
    // atomically rather than mutating an open file in place.
    let mmap = unsafe { MmapOptions::new().len(len).map(file) }
        .map_err(|err| FileError::from_io(&err, "mmap", path))?;
    reject_gzip(mmap.as_ref())?;
    Ok(MaxMindReader::from_source(mmap).map_err(open_error)?)
}

fn open_memory_reader(mut file: File, len: u64, path: &str) -> OpenResult<ReaderSource> {
    let read_error = |err: std::io::Error| FileError::from_io(&err, "read", path);
    let mut prefix = [0_u8; 2];
    let mut prefix_len = 0;
    while prefix_len < prefix.len() {
        let read = file.read(&mut prefix[prefix_len..]).map_err(read_error)?;
        if read == 0 {
            break;
        }
        prefix_len += read;
    }
    reject_gzip(&prefix[..prefix_len])?;

    let capacity = usize::try_from(len).unwrap_or(prefix_len);
    let mut bytes = Vec::with_capacity(capacity);
    bytes.extend_from_slice(&prefix[..prefix_len]);
    file.read_to_end(&mut bytes).map_err(read_error)?;
    Ok(memory_source(bytes)?)
}

fn memory_source(bytes: Vec<u8>) -> Result<ReaderSource> {
    reject_gzip(&bytes)?;
    let heap_bytes = bytes.capacity();
    let reader = MaxMindReader::from_source(bytes).map_err(open_error)?;
    Ok(ReaderSource::Memory(reader, heap_bytes))
}

fn reject_gzip(bytes: &[u8]) -> Result<()> {
    if bytes.starts_with(&[0x1f, 0x8b]) {
        return Err(napi_error(ERR_GZIP_DB));
    }
    Ok(())
}
