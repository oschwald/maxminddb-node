use crate::{
    cache::PropertyNameCache,
    decode::{
        lookup_result_path_to_js, lookup_result_record_uncached_to_js, string_bytes_to_js,
        JsDecodeEnvGuard,
    },
    errors::{lookup_error, napi_error},
};
use arrayvec::ArrayString;
use maxminddb::{
    LookupResult, MaxMindDbError, Mmap, Reader as MaxMindReader, Within, WithinOptions,
};
use napi::{
    bindgen_prelude::{Env, ToNapiValue, Unknown},
    Result,
};
use rustc_hash::FxHashMap;
use std::fmt::Write;

pub(crate) enum NetworkIter<'de> {
    Mmap(Within<'de, Mmap>),
    Memory(Within<'de, Vec<u8>>),
}

macro_rules! dispatch_network_iter {
    ($iter:expr, $inner:ident => $body:expr) => {
        match $iter {
            Self::Mmap($inner) => $body,
            Self::Memory($inner) => $body,
        }
    };
}

impl<'de> NetworkIter<'de> {
    pub(crate) fn from_mmap(
        reader: &'de MaxMindReader<Mmap>,
        cidr: Option<ipnetwork::IpNetwork>,
        options: WithinOptions,
    ) -> std::result::Result<Self, MaxMindDbError> {
        within_iter(reader, cidr, options).map(Self::Mmap)
    }

    pub(crate) fn from_memory(
        reader: &'de MaxMindReader<Vec<u8>>,
        cidr: Option<ipnetwork::IpNetwork>,
        options: WithinOptions,
    ) -> std::result::Result<Self, MaxMindDbError> {
        within_iter(reader, cidr, options).map(Self::Memory)
    }

    fn next_record_to_js<'env>(
        &mut self,
        env: &'env Env,
        guard: &JsDecodeEnvGuard<'_>,
        records_by_offset: &mut Option<FxHashMap<usize, Unknown<'env>>>,
        path: Option<&[maxminddb::PathElement<'_>]>,
    ) -> Result<Option<Unknown<'env>>> {
        dispatch_network_iter!(self, iter => {
            iter.next()
                .map(|result| network_lookup_to_js(env, result, guard, records_by_offset, path))
                .transpose()
        })
    }
}

fn within_iter<'de, S: AsRef<[u8]>>(
    reader: &'de MaxMindReader<S>,
    cidr: Option<ipnetwork::IpNetwork>,
    options: WithinOptions,
) -> std::result::Result<Within<'de, S>, MaxMindDbError> {
    match cidr {
        Some(cidr) => reader.within(cidr, options),
        None => reader.networks(options),
    }
}

fn network_lookup_to_js<'env, 'de, S: AsRef<[u8]>>(
    env: &'env Env,
    result: std::result::Result<LookupResult<'de, S>, MaxMindDbError>,
    guard: &JsDecodeEnvGuard<'_>,
    records_by_offset: &mut Option<FxHashMap<usize, Unknown<'env>>>,
    path: Option<&[maxminddb::PathElement<'_>]>,
) -> Result<Unknown<'env>> {
    let lookup = result.map_err(lookup_error)?;
    let network = lookup.network().map_err(lookup_error)?;
    let mut network_string = ArrayString::<64>::new();
    write!(&mut network_string, "{network}")
        .map_err(|_| napi_error("network string exceeds expected length"))?;
    let network = string_bytes_to_js(env, network_string.as_bytes())?;
    let record = if let (Some(records_by_offset), Some(offset)) =
        (records_by_offset.as_mut(), lookup.offset())
    {
        if let Some(record) = records_by_offset.get(&offset) {
            *record
        } else {
            let record = network_record_to_js(env, &lookup, guard, path)?;
            records_by_offset.insert(offset, record);
            record
        }
    } else {
        network_record_to_js(env, &lookup, guard, path)?
    };
    let mut pair = env.create_array(2)?;
    pair.set(0, network)?;
    pair.set(1, record)?;
    pair.into_unknown(env)
}

pub(crate) fn collect_next_networks_page_to_js<'env, 'de>(
    env: &'env Env,
    iter: &mut NetworkIter<'de>,
    limit: usize,
    property_names: &mut PropertyNameCache,
    cache_records: bool,
    path: Option<&[maxminddb::PathElement<'_>]>,
) -> Result<(Unknown<'env>, bool)> {
    let mut records = env.create_array(0)?;
    let mut records_by_offset = cache_records.then(FxHashMap::default);
    let guard = JsDecodeEnvGuard::enter(env.raw(), property_names);
    let mut count: u32 = 0;
    for _ in 0..limit {
        let Some(record) = iter.next_record_to_js(env, &guard, &mut records_by_offset, path)?
        else {
            break;
        };
        records.set(count, record)?;
        count = count
            .checked_add(1)
            .ok_or_else(|| napi_error("page size exceeds u32 range"))?;
    }
    let is_empty = count == 0;
    Ok((records.into_unknown(env)?, is_empty))
}

fn network_record_to_js<'env, S: AsRef<[u8]>>(
    env: &'env Env,
    lookup: &LookupResult<'_, S>,
    guard: &JsDecodeEnvGuard<'_>,
    path: Option<&[maxminddb::PathElement<'_>]>,
) -> Result<Unknown<'env>> {
    if let Some(path) = path {
        lookup_result_path_to_js(env, lookup, path, guard)
    } else {
        lookup_result_record_uncached_to_js(env, lookup, guard)
    }
}

pub(crate) fn make_within_options(
    include_aliased_networks: Option<bool>,
    include_networks_without_data: Option<bool>,
    skip_empty_values: Option<bool>,
) -> WithinOptions {
    let mut options = WithinOptions::default();
    if include_aliased_networks.unwrap_or(false) {
        options = options.include_aliased_networks();
    }
    if include_networks_without_data.unwrap_or(false) {
        options = options.include_networks_without_data();
    }
    if skip_empty_values.unwrap_or(false) {
        options = options.skip_empty_values();
    }
    options
}
