//! Validate component transfer syntax and classify native alpha coverage.
use std::collections::HashSet;

use resvg::usvg::filter::{ComponentTransfer, TransferFunction};
use roxmltree::Node;

use super::{finite_number, MAX_COLOR_COEFFICIENT};
use crate::{
    epub::{invalid, limited, unsupported},
    EngineError,
};

const MAX_TABLE_ENTRIES: usize = 64;

pub(in crate::epub) fn is_channel(tag: &str) -> bool {
    matches!(tag, "feFuncR" | "feFuncG" | "feFuncB" | "feFuncA")
}

pub(super) fn validate_channels(node: Node<'_, '_>) -> Result<(), EngineError> {
    let mut channels = HashSet::new();
    for child in node.children().filter(Node::is_element) {
        let channel = child.tag_name().name();
        if !is_channel(channel) {
            return Err(invalid(
                "SVG component transfer requires feFunc channel children",
            ));
        }
        if !channels.insert(channel) {
            return Err(unsupported("repeated SVG component transfer channel"));
        }
    }
    Ok(())
}

pub(super) fn validate_channel(node: Node<'_, '_>) -> Result<(), EngineError> {
    if !node
        .parent_element()
        .is_some_and(|parent| parent.has_tag_name("feComponentTransfer"))
        || node.children().any(|child| child.is_element())
    {
        return Err(invalid("misplaced SVG component transfer channel"));
    }
    let allowed: &[_] = match node.attribute("type") {
        None | Some("identity") => &[],
        Some("table" | "discrete") => &["tableValues"],
        Some("linear") => &["slope", "intercept"],
        Some("gamma") => &["amplitude", "exponent", "offset"],
        _ => return Err(unsupported("unsupported SVG component transfer function")),
    };
    for key in [
        "tableValues",
        "slope",
        "intercept",
        "amplitude",
        "exponent",
        "offset",
    ] {
        let Some(raw) = node.attribute(key) else {
            continue;
        };
        if !allowed.contains(&key) {
            return Err(invalid("unexpected SVG component transfer parameter"));
        }
        if !raw.is_ascii() {
            return Err(invalid("invalid SVG component transfer parameter"));
        }
        if key == "tableValues" {
            validate_table(raw)?;
        } else {
            let value = coefficient(raw)?;
            if key == "exponent" && value < 0.0 {
                return Err(unsupported(
                    "SVG component transfer gamma requires a nonnegative exponent",
                ));
            }
        }
    }
    Ok(())
}

fn validate_table(raw: &str) -> Result<(), EngineError> {
    if raw.trim().is_empty() {
        return Ok(());
    }
    let mut count = 0;
    for segment in raw.split(',') {
        if segment.trim().is_empty() {
            return Err(invalid("invalid SVG component transfer table"));
        }
        for value in segment.split_ascii_whitespace() {
            count += 1;
            if count > MAX_TABLE_ENTRIES {
                return Err(limited("SVG component transfer table exceeds 64 entries"));
            }
            coefficient(value)?;
        }
    }
    Ok(())
}

fn coefficient(raw: &str) -> Result<f32, EngineError> {
    let value = finite_number(raw, "invalid SVG component transfer coefficient")?;
    if value.abs() > MAX_COLOR_COEFFICIENT {
        return Err(limited("SVG component transfer coefficient exceeds 1024"));
    }
    Ok(value)
}

pub(super) struct AlphaCoverage {
    pub(super) retains_ink: bool,
    pub(super) keeps_transparency: bool,
}

/// Prove coverage properties without replacing native transfer evaluation.
pub(super) fn alpha_coverage(function: &TransferFunction) -> AlphaCoverage {
    const BYTE_ALPHA: f32 = 1.0 / 255.0;
    let (zero, retains_ink) = match function {
        TransferFunction::Identity => (0.0, true),
        TransferFunction::Table(values) if values.is_empty() => (0.0, true),
        TransferFunction::Discrete(values) if values.is_empty() => (0.0, true),
        TransferFunction::Table(values) => {
            // Linear interpolation stays above a threshold shared by all
            // endpoints. Leave one alpha level for floating-point rounding.
            (
                values[0],
                values.iter().all(|value| *value >= 2.0 * BYTE_ALPHA),
            )
        }
        TransferFunction::Discrete(values) => {
            (values[0], values.iter().all(|value| *value >= BYTE_ALPHA))
        }
        TransferFunction::Linear { slope, intercept } => {
            let least = (slope * BYTE_ALPHA + intercept).min(slope + intercept);
            (*intercept, least >= BYTE_ALPHA)
        }
        TransferFunction::Gamma {
            amplitude,
            exponent,
            offset,
        } => {
            let zero = amplitude * 0.0_f32.powf(*exponent) + offset;
            let least = (amplitude * BYTE_ALPHA.powf(*exponent) + offset).min(amplitude + offset);
            (zero, least >= 2.0 * BYTE_ALPHA)
        }
    };
    AlphaCoverage {
        retains_ink,
        keeps_transparency: zero <= 0.0,
    }
}

pub(super) fn work(transfer: &ComponentTransfer) -> usize {
    let gamma = [
        transfer.func_r(),
        transfer.func_g(),
        transfer.func_b(),
        transfer.func_a(),
    ]
    .into_iter()
    .filter(|function| matches!(function, TransferFunction::Gamma { .. }))
    .count();
    // Native transfer converts color spaces, removes/restores premultiplication,
    // and calls powf per pixel for each gamma channel.
    12 + gamma * 8
}
