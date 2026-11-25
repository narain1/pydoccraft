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
const MIN_DEVICE_DISTANCE: f32 = 1.0 / 256.0;

pub(super) fn validate(node: Node<'_, '_>) -> Result<(), EngineError> {
    let mut sources = node.children().filter(Node::is_element);
    if !sources
        .next()
        .is_some_and(|light| is_light_source(light.tag_name().name()))
        || sources.next().is_some()
    {
        return Err(invalid(
            "SVG lighting requires one distant light, point light or spot light",
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
    let (coordinates, max): (&[_], _) = match node.tag_name().name() {
        "feSpotLight" => {
            if let Some(value) = node.attribute("specularExponent") {
                if !(0.0..=128.0).contains(&finite_number(value, NUMBER)?) {
                    return Err(invalid("SVG spot exponent must be between 0 and 128"));
                }
            }
            if let Some(value) = node.attribute("limitingConeAngle") {
                let angle = finite_number(value, NUMBER)?;
                if angle <= 0.0 || angle > 90.0 {
                    return Err(invalid(
                        "SVG spot cone angle must be positive and at most 90 degrees",
                    ));
                }
            }
            (
                &["x", "y", "z", "pointsAtX", "pointsAtY", "pointsAtZ"],
                MAX_OFFSET,
            )
        }
        "fePointLight" => (&["x", "y", "z"], MAX_OFFSET),
        _ => (&["azimuth", "elevation"], 360.0),
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
    let (position, target, kind, spatial_work) = match source {
        LightSource::DistantLight(_) => return Ok(passes),
        LightSource::PointLight(point) => ([point.x, point.y, point.z], None, "point", 8),
        LightSource::SpotLight(spot) => (
            [spot.x, spot.y, spot.z],
            Some([spot.points_at_x, spot.points_at_y, spot.points_at_z]),
            "spot",
            24,
        ),
    };
    if device.sx != device.sy {
        return Err(unsupported(&format!(
            "SVG {kind} lighting requires uniform device scale"
        )));
    }
    // Native resvg scales light depth with the device, while sampled surface
    // heights remain in the original units.
    if surface != 0.0 && device.sx != 1.0 {
        return Err(unsupported(&format!(
            "SVG raised {kind} lighting requires unit device scale"
        )));
    }
    if (position[2] - surface.max(0.0)) * device.sx < MIN_DEVICE_DISTANCE {
        return Err(unsupported(&format!(
            "SVG {kind} light height must be above the sampled surface"
        )));
    }
    let position = device_position(position, device, kind)?;
    if let Some(target) = target {
        let target = device_position(target, device, kind)?;
        let length_squared = position
            .into_iter()
            .zip(target)
            .map(|(light, aim)| (light - aim).powi(2))
            .sum::<f32>();
        if length_squared < MIN_DEVICE_DISTANCE.powi(2) {
            return Err(invalid("SVG spot light requires a nonzero direction"));
        }
    }
    // Positional lights normalize a vector per sample. A spot light also
    // evaluates the cone and beam exponent before shading.
    Ok(passes + spatial_work)
}

fn device_position(
    [x, y, z]: [f32; 3],
    device: Transform,
    kind: &str,
) -> Result<[f32; 3], EngineError> {
    let position = [
        x * device.sx + device.tx,
        y * device.sy + device.ty,
        z * device.sx,
    ];
    if position
        .iter()
        .any(|v| !v.is_finite() || v.abs() > MAX_OFFSET)
    {
        return Err(limited(&format!(
            "SVG {kind} lighting exceeds its device position budget"
        )));
    }
    Ok(position)
}
