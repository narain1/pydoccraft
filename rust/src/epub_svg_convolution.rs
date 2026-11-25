//! Validate bounded SVG convolution kernels before passing them to resvg.
use resvg::usvg::filter::{ConvolveMatrix, EdgeMode};
use roxmltree::Node;

use super::{finite_number, number_pair, MAX_COLOR_COEFFICIENT};
use crate::{
    epub::{invalid, limited, unsupported},
    EngineError,
};

const MAX_ORDER: f32 = 9.0;
const MIN_DIVISOR: f32 = 1e-6;

pub(super) fn validate(node: Node<'_, '_>) -> Result<(), EngineError> {
    if node.has_attribute("kernelUnitLength") {
        return Err(unsupported(
            "SVG convolution kernelUnitLength is unsupported",
        ));
    }
    if !matches!(
        node.attribute("edgeMode"),
        None | Some("duplicate" | "wrap" | "none")
    ) {
        return Err(unsupported("unsupported SVG convolution edge mode"));
    }
    if !matches!(
        node.attribute("preserveAlpha"),
        None | Some("true" | "false")
    ) {
        return Err(invalid("invalid SVG convolution preserveAlpha value"));
    }
    let [columns, rows] = node
        .attribute("order")
        .map(|raw| number_pair(raw, "invalid SVG convolution order"))
        .transpose()?
        .unwrap_or([3.0; 2])
        .map(f32::trunc);
    if columns < 1.0 || rows < 1.0 {
        return Err(invalid("invalid SVG convolution order"));
    }
    if columns > MAX_ORDER || rows > MAX_ORDER {
        return Err(limited("SVG convolution order exceeds 9"));
    }
    for (key, order) in [("targetX", columns), ("targetY", rows)] {
        if let Some(raw) = node.attribute(key) {
            let target = raw.trim_ascii().parse::<u32>().ok();
            if target.is_none_or(|target| target >= order as u32) {
                return Err(invalid("invalid SVG convolution target"));
            }
        }
    }
    let expected = (columns * rows) as usize;
    let message = "invalid SVG convolution kernel matrix";
    let raw = node
        .attribute("kernelMatrix")
        .ok_or_else(|| invalid(message))?;
    if !raw.is_ascii() {
        return Err(invalid(message));
    }
    let mut count = 0;
    for segment in raw.split(',') {
        if segment.trim_ascii().is_empty() {
            return Err(invalid(message));
        }
        for value in segment.split_ascii_whitespace() {
            count += 1;
            if count > expected {
                return Err(invalid(message));
            }
            coefficient(value, message)?;
        }
    }
    if count != expected {
        return Err(invalid(message));
    }
    if let Some(raw) = node.attribute("divisor") {
        let divisor = coefficient(raw, "invalid SVG convolution divisor")?;
        // The locked parser drops the primitive for zero instead of using
        // the default sum, which can be requested by omitting the attribute.
        if divisor.abs() < MIN_DIVISOR {
            return Err(unsupported(
                "SVG convolution divisor is zero or below 0.000001",
            ));
        }
    }
    if let Some(raw) = node.attribute("bias") {
        coefficient(raw, "invalid SVG convolution bias")?;
    }
    Ok(())
}

fn coefficient(raw: &str, message: &'static str) -> Result<f32, EngineError> {
    if !raw.is_ascii() {
        return Err(invalid(message));
    }
    let value = finite_number(raw, message)?;
    if value.abs() > MAX_COLOR_COEFFICIENT {
        return Err(limited("SVG convolution coefficient exceeds 1024"));
    }
    Ok(value)
}

pub(super) struct AlphaCoverage {
    pub(super) retains_ink: bool,
    pub(super) keeps_transparency: bool,
    pub(super) needs_neighbors: bool,
}

pub(super) fn alpha_coverage(filter: &ConvolveMatrix) -> AlphaCoverage {
    if filter.preserve_alpha() {
        return AlphaCoverage {
            retains_ink: true,
            keeps_transparency: true,
            needs_neighbors: false,
        };
    }
    let matrix = filter.matrix();
    // SVG rotates the kernel 180 degrees before applying the authored target.
    let x = matrix.columns() - 1 - matrix.target_x();
    let y = matrix.rows() - 1 - matrix.target_y();
    let center = (y * matrix.columns() + x) as usize;
    let divisor = filter.divisor().get();
    let previous = matrix.data()[..center]
        .iter()
        .chain(&matrix.data()[center + 1..]);
    let mut coverage = AlphaCoverage {
        retains_ink: matrix.data()[center] / divisor >= 1.0 && filter.bias() >= 0.0,
        keeps_transparency: filter.bias() <= 0.0,
        needs_neighbors: false,
    };
    for coefficient in previous {
        let gain = coefficient / divisor;
        coverage.retains_ink &= gain >= 0.0;
        coverage.keeps_transparency &= gain <= 0.0;
        coverage.needs_neighbors |= gain != 0.0;
    }
    coverage
}

pub(super) fn work(filter: &ConvolveMatrix) -> usize {
    let matrix = filter.matrix();
    let taps = (matrix.columns() * matrix.rows()) as usize;
    // In wrap mode the native kernel repeatedly adds the input size to a
    // negative coordinate. A one-pixel input is the worst case.
    let wrap = if filter.edge_mode() == EdgeMode::Wrap {
        (matrix.target_x() + matrix.target_y()) as usize
    } else {
        0
    };
    8 + taps * (4 + wrap)
}
