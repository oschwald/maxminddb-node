use crate::errors::invalid_arg;
use arrayvec::ArrayVec;
use napi::{bindgen_prelude::Either, Result};
use std::{collections::HashMap, ops::Deref};

const INLINE_PATH_ELEMENTS: usize = 8;
const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;
const ERR_PATH_INDEX: &str = "path indexes must be finite safe integers";

pub(crate) enum OwnedPathElement {
    Key(String),
    Index(usize),
    IndexFromEnd(usize),
}

pub(crate) fn parse_path(path: Vec<Either<String, f64>>) -> Result<Vec<OwnedPathElement>> {
    path.into_iter()
        .map(|element| match element {
            Either::A(key) => Ok(OwnedPathElement::Key(key)),
            Either::B(index) => number_to_path_element(index),
        })
        .collect()
}

fn number_to_path_element(index: f64) -> Result<OwnedPathElement> {
    if !index.is_finite() || index.fract() != 0.0 || index.abs() > MAX_SAFE_INTEGER {
        return Err(invalid_arg(ERR_PATH_INDEX));
    }

    if index >= 0.0 {
        Ok(OwnedPathElement::Index(
            usize::try_from(index as u64).unwrap_or(usize::MAX),
        ))
    } else {
        Ok(OwnedPathElement::IndexFromEnd(
            usize::try_from((-index - 1.0) as u64).unwrap_or(usize::MAX),
        ))
    }
}

pub(crate) enum PathElements<'a> {
    Inline(ArrayVec<maxminddb::PathElement<'a>, INLINE_PATH_ELEMENTS>),
    Heap(Vec<maxminddb::PathElement<'a>>),
}

impl<'a> Deref for PathElements<'a> {
    type Target = [maxminddb::PathElement<'a>];

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Inline(elements) => elements,
            Self::Heap(elements) => elements,
        }
    }
}

pub(crate) fn path_elements_from_owned(path: &[OwnedPathElement]) -> PathElements<'_> {
    if path.len() <= INLINE_PATH_ELEMENTS {
        PathElements::Inline(path.iter().map(path_element_from_owned).collect())
    } else {
        PathElements::Heap(path.iter().map(path_element_from_owned).collect())
    }
}

fn path_element_from_owned(element: &OwnedPathElement) -> maxminddb::PathElement<'_> {
    match element {
        OwnedPathElement::Key(key) => maxminddb::PathElement::Key(key.as_str()),
        OwnedPathElement::Index(index) => maxminddb::PathElement::Index(*index),
        OwnedPathElement::IndexFromEnd(index) => maxminddb::PathElement::IndexFromEnd(*index),
    }
}

pub(crate) fn compiled_path(
    paths: &HashMap<u32, Vec<OwnedPathElement>>,
    path_id: u32,
) -> Result<&[OwnedPathElement]> {
    paths
        .get(&path_id)
        .map(Vec::as_slice)
        .ok_or_else(|| invalid_arg(format!("Invalid compiled path id: {path_id}")))
}

#[cfg(test)]
mod tests {
    use super::{
        number_to_path_element, parse_path, path_elements_from_owned, OwnedPathElement,
        PathElements, INLINE_PATH_ELEMENTS, MAX_SAFE_INTEGER,
    };
    use napi::bindgen_prelude::Either;

    #[test]
    fn maps_numeric_indexes() {
        assert!(matches!(
            number_to_path_element(0.0),
            Ok(OwnedPathElement::Index(0))
        ));
        assert!(matches!(
            number_to_path_element(-1.0),
            Ok(OwnedPathElement::IndexFromEnd(0))
        ));
        assert!(matches!(
            number_to_path_element(-2.0),
            Ok(OwnedPathElement::IndexFromEnd(1))
        ));
    }

    #[cfg(target_pointer_width = "64")]
    #[test]
    fn maps_maximum_safe_indexes() {
        assert!(matches!(
            number_to_path_element(MAX_SAFE_INTEGER),
            Ok(OwnedPathElement::Index(index)) if index == MAX_SAFE_INTEGER as usize
        ));
        assert!(matches!(
            number_to_path_element(-MAX_SAFE_INTEGER),
            Ok(OwnedPathElement::IndexFromEnd(index))
                if index == MAX_SAFE_INTEGER as usize - 1
        ));
    }

    #[test]
    fn rejects_invalid_numeric_indexes() {
        for index in [
            1.5,
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            MAX_SAFE_INTEGER + 1.0,
            -MAX_SAFE_INTEGER - 1.0,
        ] {
            let err = parse_path(vec![Either::B(index)])
                .err()
                .expect("invalid numeric path index should fail");
            assert!(err.reason.contains("finite safe integers"));
        }
    }

    #[test]
    fn stores_only_common_paths_inline() {
        let inline = (0..INLINE_PATH_ELEMENTS)
            .map(OwnedPathElement::Index)
            .collect::<Vec<_>>();
        let heap = (0..=INLINE_PATH_ELEMENTS)
            .map(OwnedPathElement::Index)
            .collect::<Vec<_>>();

        assert!(matches!(
            path_elements_from_owned(&inline),
            PathElements::Inline(_)
        ));
        assert!(matches!(
            path_elements_from_owned(&heap),
            PathElements::Heap(_)
        ));
    }
}
