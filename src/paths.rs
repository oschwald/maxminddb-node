use crate::errors::invalid_arg;
use arrayvec::ArrayVec;
use napi::{bindgen_prelude::Either, Result};
use std::{collections::HashMap, ops::Deref};

const INLINE_PATH_ELEMENTS: usize = 8;

pub(crate) enum OwnedPathElement {
    Key(String),
    Index(usize),
    IndexFromEnd(usize),
}

pub(crate) fn parse_path(path: Vec<Either<String, i64>>) -> Result<Vec<OwnedPathElement>> {
    path.into_iter()
        .map(|element| match element {
            Either::A(key) => Ok(OwnedPathElement::Key(key)),
            Either::B(index) => Ok(signed_index_to_path_element(index)),
        })
        .collect()
}

fn signed_index_to_path_element(index: i64) -> OwnedPathElement {
    if index >= 0 {
        OwnedPathElement::Index(index as usize)
    } else {
        let index_from_end = index
            .checked_neg()
            .and_then(|n| n.checked_sub(1))
            .map(|n| n as usize)
            .unwrap_or(usize::MAX);
        OwnedPathElement::IndexFromEnd(index_from_end)
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
