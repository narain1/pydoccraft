//! Validate bounded SVG lighting before calling the native raster kernel.
use resvg::{
    tiny_skia::Transform,
    usvg::{
        filter::{ColorInterpolation, Kind, LightSource, Primitive},
        NonZeroRect,
    },
};
use roxmltree::Node;

use super::{finite_number, is_light_source, is_lighting, positive_axes, MAX_OFFSET};
use crate::{
    epub::{css, invalid, limited, unsupported},
    EngineError,
};

const NUMBER: &str = "invalid SVG lighting number";

pub(super) fn validate(node: Node<'_, '_>) -> Result<(), EngineError> {
    let mut sources = node.children().filter(Node::is_element);
    if !sources
        .next()
        .is_some_and(|light| is_light_source(light.tag_name().name()))
        || sources.next().is_some()
    {
        return Err(invalid(
            "SVG lighting requires one distant light or point light",
        ));
    }
    if node.has_attribute("kernelUnitLength") {
        return Err(unsupported("SVG lighting kernelUnitLength"));
    }
    if node
        .parent_element()
        .and_then(|filter| filter.attribute("primitiveUnits"))
        == Some("objectBoundingBox")
    {
        return Err(unsupported(
            "SVG lighting requires user-space primitive units",
        ));
    }
    if let Some(value) = node.attribute("surfaceScale") {
        bounded_number(value, 1024.0)?;
    }
    for property in ["diffuseConstant", "specularConstant"] {
        if let Some(value) = node.attribute(property) {
            if bounded_number(value, 64.0)? < 0.0 {
                return Err(invalid("SVG lighting constant cannot be negative"));
            }
        }
    }
    if let Some(value) = node.attribute("specularExponent") {
        if !(1.0..=128.0).contains(&finite_number(value, NUMBER)?) {
            return Err(invalid(
                "SVG lighting specular exponent must be between 1 and 128",
            ));
        }
    }
    Ok(())
}

pub(super) fn validate_source(node: Node<'_, '_>) -> Result<(), EngineError> {
    if !node
        .parent_element()
        .is_some_and(|parent| is_lighting(parent.tag_name().name()))
        || node.children().any(|child| child.is_element())
    {
        return Err(invalid("misplaced SVG light source"));
    }
    let (coordinates, max): (&[_], _) = if node.has_tag_name("fePointLight") {
        (&["x", "y", "z"], MAX_OFFSET)
    } else {
        (&["azimuth", "elevation"], 360.0)
    };
    for &property in coordinates {
        if let Some(value) = node.attribute(property) {
            bounded_number(value, max)?;
        }
    }
    Ok(())
}

fn bounded_number(value: &str, max: f32) -> Result<f32, EngineError> {
    let value = finite_number(value, NUMBER)?;
    if value.abs() > max {
        return Err(limited("SVG lighting exceeds its numeric budget"));
    }
    Ok(value)
}

pub(super) fn color(value: &str) -> Result<String, EngineError> {
    let ([red, green, blue], _) = css::color(value.trim())
        .ok_or_else(|| unsupported("SVG lighting requires an absolute color"))?;
    // Lighting uses the color channels and ignores the declared color alpha.
    Ok(format!("rgb({red},{green},{blue})"))
}

pub(super) fn work(
    primitive: &Primitive,
    filter: NonZeroRect,
    device: Transform,
    [width, height]: [u32; 2],
) -> Result<usize, EngineError> {
    if primitive.color_interpolation() != ColorInterpolation::SRGB {
        return Err(unsupported(
            "SVG lighting requires sRGB color interpolation",
        ));
    }
    if !positive_axes(device) {
        return Err(unsupported(
            "SVG lighting requires an axis-aligned raster transform",
        ));
    }
    let filter = filter
        .transform(device)
        .ok_or_else(|| invalid("invalid SVG lighting region"))?;
    if filter.width() < 3.0 || filter.height() < 3.0 {
        return Err(unsupported(
            "SVG lighting requires at least three pixels on each raster axis",
        ));
    }
    // Clamping the layer changes the samples used by the normal kernel.
    if filter.x() < 0.0
        || filter.y() < 0.0
        || filter.right() > width as f32
        || filter.bottom() > height as f32
    {
        return Err(unsupported(
            "SVG lighting region must fit within the raster viewport",
        ));
    }
    // Charge the neighboring alpha samples, buffers and color work.
    // Specular also evaluates an exponent.
    let (source, surface, passes) = match primitive.kind() {
        Kind::DiffuseLighting(light) => (light.light_source(), light.surface_scale(), 24),
        Kind::SpecularLighting(light) => (light.light_source(), light.surface_scale(), 32),
        _ => return Err(invalid("invalid SVG lighting primitive")),
    };
    if let LightSource::PointLight(point) = source {
        if device.sx != device.sy {
            return Err(unsupported(
                "SVG point lighting requires uniform device scale",
            ));
        }
        // Native resvg scales the light's z coordinate with the device,
        // while surface heights remain in the original units.
        if surface != 0.0 && device.sx != 1.0 {
            return Err(unsupported(
                "SVG raised point lighting requires unit device scale",
            ));
        }
        if (point.z - surface.max(0.0)) * device.sx < 1.0 / 256.0 {
            return Err(unsupported(
                "SVG point light height must be above the sampled surface",
            ));
        }
        let position = [
            point.x * device.sx + device.tx,
            point.y * device.sy + device.ty,
            point.z * device.sx,
        ];
        if position
            .iter()
            .any(|v| !v.is_finite() || v.abs() > MAX_OFFSET)
        {
            return Err(limited(
                "SVG point lighting exceeds its device position budget",
            ));
        }
        // A point light normalizes a new light vector at every sample.
        return Ok(passes + 8);
    }
    Ok(passes)
}
