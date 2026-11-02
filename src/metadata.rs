use napi::{
    bindgen_prelude::{Array, Env, JsObjectValue, Object},
    Result,
};

pub(crate) fn metadata_to_js<'env>(
    env: &'env Env,
    meta: &maxminddb::Metadata,
) -> Result<Object<'env>> {
    let mut object = Object::new(env)?;
    object.set_named_property("binaryFormatMajorVersion", meta.binary_format_major_version)?;
    object.set_named_property("binaryFormatMinorVersion", meta.binary_format_minor_version)?;
    object.set_named_property("buildEpoch", meta.build_epoch as f64)?;
    object.set_named_property("databaseType", meta.database_type.as_str())?;

    let mut description = Object::new(env)?;
    for (key, value) in &meta.description {
        description.set_named_property(key, value.as_str())?;
    }
    object.set_named_property("description", description)?;

    object.set_named_property("ipVersion", meta.ip_version)?;
    object.set_named_property(
        "languages",
        Array::from_ref_vec_string(env, &meta.languages)?,
    )?;
    object.set_named_property("nodeCount", meta.node_count)?;
    object.set_named_property("recordSize", meta.record_size)?;
    object.set_named_property("nodeByteSize", meta.record_size / 4)?;
    object.set_named_property(
        "searchTreeSize",
        search_tree_size(meta.node_count, meta.record_size),
    )?;
    object.set_named_property(
        "treeDepth",
        if meta.ip_version == 4 {
            32_u32
        } else {
            128_u32
        },
    )?;
    Ok(object)
}

fn search_tree_size(node_count: u32, record_size: u16) -> f64 {
    f64::from(node_count) * f64::from(record_size / 4)
}

#[cfg(test)]
mod tests {
    use super::search_tree_size;

    #[test]
    fn search_tree_size_preserves_values_across_u32_boundary() {
        for (record_size, boundary) in [(24, 715_827_883), (28, 613_566_757), (32, 536_870_912)] {
            for node_count in [0, 1, 1000, boundary - 1, boundary, boundary + 1, u32::MAX] {
                let expected = u64::from(node_count) * u64::from(record_size / 4);
                assert_eq!(
                    search_tree_size(node_count, record_size),
                    expected as f64,
                    "node_count={node_count}, record_size={record_size}"
                );
            }
        }
    }
}
