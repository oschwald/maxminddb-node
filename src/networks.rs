use crate::decode::{value_to_js, MmdbValue};
use maxminddb::{MaxMindDbError, Reader as MaxMindReader, WithinOptions};
use napi::{
    bindgen_prelude::{Array, Env, JsObjectValue, Null, Object, ToNapiValue, Unknown},
    Result,
};

pub(crate) struct NetworkRecord<'de> {
    network: String,
    record: Option<MmdbValue<'de>>,
}

pub(crate) struct NetworkRecordPage<'de> {
    records: Vec<NetworkRecord<'de>>,
    next_offset: Option<usize>,
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

pub(crate) fn collect_networks_page_for_reader<'de, S: AsRef<[u8]>>(
    reader: &'de MaxMindReader<S>,
    cidr: Option<ipnetwork::IpNetwork>,
    options: WithinOptions,
    limit: usize,
    offset: usize,
) -> std::result::Result<NetworkRecordPage<'de>, MaxMindDbError> {
    let mut iter = match cidr {
        Some(cidr) => reader.within(cidr, options)?,
        None => reader.networks(options)?,
    };

    for _ in 0..offset {
        if let Some(result) = iter.next() {
            result?;
        } else {
            return Ok(NetworkRecordPage {
                records: Vec::new(),
                next_offset: None,
            });
        }
    }

    let mut records = Vec::with_capacity(limit);
    for _ in 0..limit {
        let Some(result) = iter.next() else {
            return Ok(NetworkRecordPage {
                records,
                next_offset: None,
            });
        };
        let lookup = result?;
        let network = lookup.network()?.to_string();
        let record = lookup.decode::<MmdbValue<'_>>()?;
        records.push(NetworkRecord { network, record });
    }

    let next_offset = match iter.next() {
        Some(result) => {
            result?;
            Some(offset + records.len())
        }
        None => None,
    };

    Ok(NetworkRecordPage {
        records,
        next_offset,
    })
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

pub(crate) fn network_record_page_to_js<'env>(
    env: &'env Env,
    page: NetworkRecordPage<'_>,
) -> Result<Unknown<'env>> {
    let records = network_records_to_js(env, page.records)?;
    let next_offset = match page.next_offset {
        Some(offset) => (offset as f64).into_unknown(env)?,
        None => Null.into_unknown(env)?,
    };

    let mut object = Object::new(env)?;
    object.set_named_property("records", records)?;
    object.set_named_property("nextOffset", next_offset)?;
    object.into_unknown(env)
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
