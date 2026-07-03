use crate::{
    cache::PropertyNameCache, decode::lookup_result_record_uncached_to_js, errors::lookup_error,
};
use maxminddb::{
    LookupResult, MaxMindDbError, Mmap, Reader as MaxMindReader, Within, WithinOptions,
};
use napi::{
    bindgen_prelude::{Array, Env, ToNapiValue, Unknown},
    Result,
};

const MAX_INITIAL_NETWORK_PAGE_CAPACITY: usize = 1024;

pub(crate) enum NetworkIter<'de> {
    Mmap(Within<'de, Mmap>),
    Memory(Within<'de, Vec<u8>>),
}

pub(crate) fn collect_networks_for_reader_to_js<'env, S: AsRef<[u8]>>(
    env: &'env Env,
    reader: &MaxMindReader<S>,
    cidr: Option<ipnetwork::IpNetwork>,
    options: WithinOptions,
    property_names: &std::cell::RefCell<PropertyNameCache>,
) -> Result<Unknown<'env>> {
    let iter = match cidr {
        Some(cidr) => reader.within(cidr, options).map_err(lookup_error)?,
        None => reader.networks(options).map_err(lookup_error)?,
    };
    let mut records = Vec::new();
    for result in iter {
        records.push(network_lookup_to_js(env, result, property_names)?);
    }
    Array::from_vec(env, records)?.into_unknown(env)
}

impl<'de> NetworkIter<'de> {
    pub(crate) fn from_mmap(
        reader: &'de MaxMindReader<Mmap>,
        cidr: Option<ipnetwork::IpNetwork>,
        options: WithinOptions,
    ) -> std::result::Result<Self, MaxMindDbError> {
        let iter = match cidr {
            Some(cidr) => reader.within(cidr, options)?,
            None => reader.networks(options)?,
        };
        Ok(Self::Mmap(iter))
    }

    pub(crate) fn from_memory(
        reader: &'de MaxMindReader<Vec<u8>>,
        cidr: Option<ipnetwork::IpNetwork>,
        options: WithinOptions,
    ) -> std::result::Result<Self, MaxMindDbError> {
        let iter = match cidr {
            Some(cidr) => reader.within(cidr, options)?,
            None => reader.networks(options)?,
        };
        Ok(Self::Memory(iter))
    }
}

impl<'de> NetworkIter<'de> {
    fn next_record_to_js<'env>(
        &mut self,
        env: &'env Env,
        property_names: &std::cell::RefCell<PropertyNameCache>,
    ) -> Result<Option<Unknown<'env>>> {
        match self {
            Self::Mmap(iter) => iter
                .next()
                .map(|result| network_lookup_to_js(env, result, property_names))
                .transpose(),
            Self::Memory(iter) => iter
                .next()
                .map(|result| network_lookup_to_js(env, result, property_names))
                .transpose(),
        }
    }
}

fn network_lookup_to_js<'env, 'de, S: AsRef<[u8]>>(
    env: &'env Env,
    result: std::result::Result<LookupResult<'de, S>, MaxMindDbError>,
    property_names: &std::cell::RefCell<PropertyNameCache>,
) -> Result<Unknown<'env>> {
    let lookup = result.map_err(lookup_error)?;
    let network = lookup
        .network()
        .map_err(lookup_error)?
        .to_string()
        .into_unknown(env)?;
    let record = lookup_result_record_uncached_to_js(env, &lookup, property_names)?;
    Array::from_vec(env, vec![network, record])?.into_unknown(env)
}

pub(crate) fn collect_next_networks_page_to_js<'env, 'de>(
    env: &'env Env,
    iter: &mut NetworkIter<'de>,
    limit: usize,
    property_names: &std::cell::RefCell<PropertyNameCache>,
) -> Result<(Unknown<'env>, bool)> {
    let mut records = Vec::with_capacity(limit.min(MAX_INITIAL_NETWORK_PAGE_CAPACITY));
    for _ in 0..limit {
        let Some(record) = iter.next_record_to_js(env, property_names)? else {
            break;
        };
        records.push(record);
    }
    let is_empty = records.is_empty();
    Ok((Array::from_vec(env, records)?.into_unknown(env)?, is_empty))
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
