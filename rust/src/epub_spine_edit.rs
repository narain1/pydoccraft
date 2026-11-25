//! Patch only the authored attributes of existing OPF spine occurrences.
use std::{collections::HashMap, ops::Range};

use roxmltree::Node;

use super::{apply_edits, argument, editor, unsupported, METADATA_BYTES};
use crate::{limits::validate_member_count, EngineError};

pub(crate) type SpinePropertyEdit = (
    i64,
    Option<bool>,
    Option<String>,
    Option<String>,
    Option<String>,
);

const LAYOUT: &[(&str, &str)] = &[
    ("reflowable", "rendition:layout-reflowable"),
    ("pre-paginated", "rendition:layout-pre-paginated"),
];
const PLACEMENT: &[(&str, &str)] = &[
    ("left", "page-spread-left"),
    ("right", "page-spread-right"),
    ("center", "rendition:page-spread-center"),
    ("left", "rendition:page-spread-left"),
    ("right", "rendition:page-spread-right"),
];
const SPREAD: &[(&str, &str)] = &[
    ("none", "rendition:spread-none"),
    ("landscape", "rendition:spread-landscape"),
    ("portrait", "rendition:spread-portrait"),
    ("both", "rendition:spread-both"),
    ("auto", "rendition:spread-auto"),
];

pub(super) struct Item {
    pub linear: bool,
    pub markup: Option<String>,
    pub fixed: Option<bool>,
}

pub(super) fn index(value: i64, len: usize) -> Result<usize, EngineError> {
    let len = len as i64;
    let index = if value < 0 { len + value } else { value };
    usize::try_from(index)
        .ok()
        .filter(|&index| index < len as usize)
        .ok_or_else(|| EngineError::PageIndex("EPUB chapter is out of range".into()))
}

pub(super) fn stage(
    source: &str,
    references: &[Node<'_, '_>],
    changes: &[SpinePropertyEdit],
    epub3: bool,
    package_fixed: bool,
) -> Result<HashMap<usize, Item>, EngineError> {
    validate_member_count(changes.len())?;
    let mut staged = HashMap::new();
    for (index_value, linear, layout, placement, spread) in changes {
        let index = index(*index_value, references.len())?;
        if !epub3 && (layout.is_some() || placement.is_some() || spread.is_some()) {
            return Err(unsupported(
                "editing spine rendition properties requires EPUB 3",
            ));
        }
        let node = references[index];
        let range = node.range();
        let mut edits = Vec::new();
        let current_linear = node.attribute("linear") != Some("no");
        if let Some(linear) = linear.filter(|&linear| linear != current_linear) {
            let value = if linear { "yes" } else { "no" };
            patch_attribute(
                source,
                node,
                "linear",
                format!(" linear=\"{value}\""),
                &mut edits,
            )?;
        }
        let current: Vec<_> = node
            .attribute("properties")
            .unwrap_or_default()
            .split_ascii_whitespace()
            .collect();
        let mut properties = current.clone();
        for (requested, choices, name) in [
            (layout, LAYOUT, "layout"),
            (placement, PLACEMENT, "page spread placement"),
            (spread, SPREAD, "synthetic spread"),
        ] {
            if let Some(requested) = requested {
                set_property(&mut properties, choices, requested, name)?;
            }
        }
        if properties != current {
            patch_attribute(
                source,
                node,
                "properties",
                editor::properties_attribute(&properties),
                &mut edits,
            )?;
        }
        let markup = if edits.is_empty() {
            None
        } else {
            Some(apply_edits(&source[range], edits, METADATA_BYTES)?)
        };
        if staged
            .insert(
                index,
                Item {
                    linear: linear.unwrap_or(current_linear),
                    markup,
                    fixed: layout.as_deref().map(|layout| {
                        layout == "pre-paginated" || (layout == "inherit" && package_fixed)
                    }),
                },
            )
            .is_some()
        {
            return Err(argument("duplicate EPUB spine property edit"));
        }
    }
    Ok(staged)
}

fn set_property(
    properties: &mut Vec<&str>,
    choices: &[(&str, &'static str)],
    requested: &str,
    kind: &str,
) -> Result<(), EngineError> {
    let replacement = if requested == "inherit" {
        None
    } else {
        Some(
            choices
                .iter()
                .find(|(value, _)| *value == requested)
                .map(|(_, token)| *token)
                .ok_or_else(|| argument(&format!("invalid EPUB spine {kind}")))?,
        )
    };
    let belongs = |token: &str| choices.iter().any(|(_, candidate)| token == *candidate);
    if properties
        .iter()
        .copied()
        .filter(|token| belongs(token))
        .eq(replacement)
    {
        return Ok(());
    }
    properties.retain(|token| !belongs(token));
    properties.extend(replacement);
    Ok(())
}

fn patch_attribute(
    source: &str,
    node: Node<'_, '_>,
    name: &str,
    mut encoded: String,
    edits: &mut Vec<(Range<usize>, String)>,
) -> Result<(), EngineError> {
    let range = if let Some(attribute) = node.attribute_node(name) {
        if encoded.starts_with(' ') {
            encoded.remove(0);
        }
        attribute.range()
    } else {
        let position = editor::before_attributes(source, node)?;
        position..position
    };
    let start = node.range().start;
    edits.push((range.start - start..range.end - start, encoded));
    Ok(())
}
