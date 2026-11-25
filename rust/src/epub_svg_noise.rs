//! Bound the native turbulence and displacement kernels.
use resvg::usvg::{filter::Turbulence, NonZeroRect};
use roxmltree::Node;

use super::{finite_number, number_pair, validate_offset};
use crate::{
    epub::{invalid, limited, unsupported},
    EngineError,
};

const MAX_OCTAVES: u32 = 8;
const MAX_FREQUENCY: f32 = 1024.0;
const MAX_LATTICE_COORDINATE: f64 = (1 << 23) as f64;
pub(super) const TURBULENCE_SETUP_WORK: usize = 4096;

fn require_user_units(node: Node<'_, '_>) -> Result<(), EngineError> {
    if node
        .parent_element()
        .and_then(|filter| filter.attribute("primitiveUnits"))
        == Some("objectBoundingBox")
    {
        return Err(unsupported(
            "SVG turbulence and displacement require userSpaceOnUse primitive units",
        ));
    }
    Ok(())
}

pub(super) fn validate_turbulence(node: Node<'_, '_>) -> Result<(), EngineError> {
    require_user_units(node)?;
    if !matches!(
        node.attribute("type"),
        None | Some("turbulence" | "fractalNoise")
    ) {
        return Err(unsupported("unsupported SVG turbulence type"));
    }
    if !matches!(node.attribute("stitchTiles"), None | Some("noStitch")) {
        return Err(unsupported("SVG turbulence stitching is unsupported"));
    }
    if let Some(value) = node.attribute("baseFrequency") {
        for frequency in number_pair(value, "invalid SVG turbulence base frequency")? {
            if frequency.is_sign_negative() {
                return Err(invalid("invalid SVG turbulence base frequency"));
            }
            if frequency > MAX_FREQUENCY {
                return Err(limited("SVG turbulence base frequency exceeds 1024"));
            }
        }
    }
    if let Some(value) = node.attribute("numOctaves") {
        let octaves = value
            .trim_ascii()
            .parse::<u32>()
            .map_err(|_| invalid("invalid SVG turbulence octave count"))?;
        if octaves > MAX_OCTAVES {
            return Err(limited("SVG turbulence exceeds 8 octaves"));
        }
    }
    if let Some(value) = node.attribute("seed") {
        let seed = number(value, "invalid SVG turbulence seed")?;
        // An i32::MIN seed overflows the native generator's negation.
        if seed.abs() >= i32::MAX as f32 {
            return Err(limited(
                "SVG turbulence seed exceeds the signed generator range",
            ));
        }
    }
    Ok(())
}

pub(super) fn validate_displacement(node: Node<'_, '_>) -> Result<(), EngineError> {
    require_user_units(node)?;
    for selector in ["xChannelSelector", "yChannelSelector"] {
        if !matches!(node.attribute(selector), None | Some("R" | "G" | "B" | "A")) {
            return Err(unsupported("unsupported SVG displacement channel selector"));
        }
    }
    if let Some(scale) = node.attribute("scale") {
        validate_offset(
            number(scale, "invalid SVG displacement scale")?,
            "SVG displacement scale exceeds 16000",
        )?;
    }
    Ok(())
}

fn number(value: &str, error: &'static str) -> Result<f32, EngineError> {
    if !value.is_ascii() {
        return Err(invalid(error));
    }
    finite_number(value, error)
}

pub(super) fn turbulence_work(
    turbulence: &Turbulence,
    layer: NonZeroRect,
    [sx, sy]: [f32; 2],
) -> Result<usize, EngineError> {
    let octaves = turbulence.num_octaves();
    if octaves > MAX_OCTAVES || turbulence.seed() == i32::MIN || turbulence.stitch_tiles() {
        return Err(limited("SVG turbulence exceeds its native kernel limits"));
    }
    let gain = f64::from(1u32 << octaves);
    for (extent, scale, frequency) in [
        (
            layer.x().abs().max(layer.right().abs()),
            sx,
            turbulence.base_frequency_x().get(),
        ),
        (
            layer.y().abs().max(layer.bottom().abs()),
            sy,
            turbulence.base_frequency_y().get(),
        ),
    ] {
        if octaves == 0 || frequency == 0.0 {
            continue;
        }
        // The isolated raster origin and filter crop are rounded in device
        // space. Include a device-pixel margin in the lattice bound before
        // native noise doubles its coordinates at each octave.
        let coordinate = (f64::from(extent) + 8.0 / f64::from(scale)) * f64::from(frequency) * gain;
        if !scale.is_finite()
            || scale <= 0.0
            || !coordinate.is_finite()
            || coordinate > MAX_LATTICE_COORDINATE
        {
            return Err(limited(
                "SVG turbulence lattice exceeds its device coordinate budget",
            ));
        }
    }
    // Each octave samples four lattice neighbors for each RGBA channel.
    Ok(8 + 16 * octaves as usize)
}
