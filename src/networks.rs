use crate::decode::{value_to_js, MmdbValue};
use maxminddb::{
    LookupResult, MaxMindDbError, Mmap, Reader as MaxMindReader, Within, WithinOptions,
};
use napi::{
    bindgen_prelude::{Array, Env, Null, ToNapiValue, Unknown},
    Result,
};

pub(crate) struct NetworkRecord<'de> {
    network: String,
    record: Option<MmdbValue<'de>>,
}

pub(crate) enum NetworkIter<'de> {
    Mmap(Within<'de, Mmap>),
    Memory(Within<'de, Vec<u8>>),
}

pub(crate) fn collect_networks_for_reader<'de, S: AsRef<[u8]>>(
    reader: &'de MaxMindReader<S>,
    cidr: Option<ipnetwork::IpNetwork>,
    options: WithinOptions,
) -> std::result::Result<Vec<NetworkRecord<'de>>, MaxMindDbError> {
    let iter = match cidr {
        Some(cidr) => reader.within(cidr, options)?,
        None => reader.networks(options)?,
    };
    let mut records = Vec::new();
    for result in iter {
        let lookup = result?;
        let network = lookup.network()?.to_string();
        let record = lookup.decode::<MmdbValue<'_>>()?;
        records.push(NetworkRecord { network, record });
    }
    Ok(records)
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

impl<'de> Iterator for NetworkIter<'de> {
    type Item = std::result::Result<NetworkRecord<'de>, MaxMindDbError>;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Mmap(iter) => iter.next().map(network_record_from_lookup),
            Self::Memory(iter) => iter.next().map(network_record_from_lookup),
        }
    }
}

fn network_record_from_lookup<'de, S: AsRef<[u8]>>(
    result: std::result::Result<LookupResult<'de, S>, MaxMindDbError>,
) -> std::result::Result<NetworkRecord<'de>, MaxMindDbError> {
    result.and_then(|lookup| {
        let network = lookup.network()?.to_string();
        let record = lookup.decode::<MmdbValue<'_>>()?;
        Ok(NetworkRecord { network, record })
    })
}

pub(crate) fn collect_next_networks_page<'de>(
    iter: &mut NetworkIter<'de>,
    limit: usize,
) -> std::result::Result<Vec<NetworkRecord<'de>>, MaxMindDbError> {
    let mut records = Vec::with_capacity(limit);
    for _ in 0..limit {
        let Some(result) = iter.next() else {
            return Ok(records);
        };
        records.push(result?);
    }
    Ok(records)
}

pub(crate) fn network_records_to_js<'env>(
    env: &'env Env,
    records: Vec<NetworkRecord<'_>>,
) -> Result<Unknown<'env>> {
    let values = records
        .into_iter()
        .map(|record| {
            let js_record = match record.record {
                Some(value) => value_to_js(env, value)?,
                None => Null.into_unknown(env)?,
            };
            let network = record.network.into_unknown(env)?;
            Array::from_vec(env, vec![network, js_record])?.into_unknown(env)
        })
        .collect::<Result<Vec<_>>>()?;
    Array::from_vec(env, values)?.into_unknown(env)
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
