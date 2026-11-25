//! Validate bounded SVG lighting before calling the native raster kernel.
use std::collections::HashMap;

use resvg::{
    tiny_skia::Transform,
    usvg::{
        filter::{
            ColorInterpolation, Composite, CompositeOperator, Input, Kind, LightSource, Primitive,
        },
        NonZeroRect,
    },
};
use roxmltree::{Node, NodeId};

use super::{
    super::{style_insertion, use_refs},
    finite_number, is_light_source, is_lighting, positive_axes, MAX_OFFSET,
};
use crate::{
    author_epub::escape_attribute,
    epub::{
        charge, css, invalid, limited, parse_xml, structure::apply_edits, unsupported, XML_BYTES,
    },
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

const INTERPOLATION: &str = "color-interpolation-filters";
type FilterSpace = (ColorInterpolation, Option<NodeId>);

/// usvg passes CSS color bytes to the lighting kernel without decoding them.
/// Project the already cascaded paint into the selected raster color space.
pub(in crate::epub::svg) fn normalize(
    styled: Vec<u8>,
    has_uses: bool,
) -> Result<Vec<u8>, EngineError> {
    let source = std::str::from_utf8(&styled).map_err(invalid)?;
    let xml = parse_xml(&styled)?;
    let root = xml.root_element();
    // A root-only declaration, or the initial value alone, is common to
    // all use instances. Local overrides can vary across native clones.
    let scoped_color = has_uses
        && root
            .descendants()
            .skip(1)
            .filter(Node::is_element)
            .any(|node| {
                node.has_attribute(INTERPOLATION)
                    || node
                        .attribute("style")
                        .is_some_and(|style| style.contains(INTERPOLATION))
            });
    let used = if scoped_color {
        use_refs::used_sources(root)
    } else {
        Vec::new()
    };
    let mut spaces = HashMap::new();
    let mut edits = Vec::new();
    let mut bytes = 0;
    for light in root
        .descendants()
        .filter(|node| node.is_element() && is_lighting(node.tag_name().name()))
    {
        let (space, declared_by) = filter_space(light, &mut spaces)?;
        let declaration = light.attribute_node("style");
        let inline = declaration.map_or("", |style| style.value());
        let properties = css::svg_declarations(inline)?;
        let color = properties
            .get("lighting-color")
            .map(String::as_str)
            .or_else(|| light.attribute("lighting-color"));
        let rgb = color
            .map(|value| {
                css::color(value)
                    .map(|(rgb, _)| rgb)
                    .ok_or_else(|| unsupported("SVG lighting requires an absolute color"))
            })
            .transpose()?;

        // An inherited mode from outside a native-use clone can depend on
        // the instance. A color with only endpoint channels needs no decoding.
        if use_refs::scope_contains(&used, light)
            && declared_by != Some(light.id())
            && declared_by != light.parent_element().map(|filter| filter.id())
        {
            if rgb.is_some_and(|rgb| rgb.iter().any(|&channel| channel != 0 && channel != 255)) {
                return Err(unsupported("SVG lighting color interpolation inside a use instance needs a local declaration"));
            }
            continue;
        }

        let mut style = inline.trim_end_matches(';').to_owned();
        if !style.is_empty() {
            style.push(';');
        }
        style.push_str(INTERPOLATION);
        match space {
            ColorInterpolation::SRGB => style.push_str(":sRGB;"),
            ColorInterpolation::LinearRGB => {
                style.push_str(":linearRGB;");
                if let Some(rgb) = rgb {
                    let [r, g, b] = rgb.map(linear_channel);
                    style.push_str(&format!("lighting-color:rgb({r},{g},{b});"));
                }
            }
        }
        let range = declaration.map_or_else(
            || style_insertion(source.as_bytes(), light),
            |attribute| Ok(attribute.range()),
        )?;
        let replacement = format!(r#" style="{}""#, escape_attribute(&style));
        charge(
            &mut bytes,
            replacement.len(),
            XML_BYTES,
            "EPUB SVG lighting styles exceed 8 MiB",
        )?;
        edits.push((range, replacement));
    }
    Ok(apply_edits(source, edits, XML_BYTES)?.into_bytes())
}

fn filter_space(
    node: Node<'_, '_>,
    cache: &mut HashMap<NodeId, FilterSpace>,
) -> Result<FilterSpace, EngineError> {
    let mut visited = Vec::new();
    let mut result = (ColorInterpolation::LinearRGB, None);
    for ancestor in node.ancestors().filter(Node::is_element) {
        if let Some(&space) = cache.get(&ancestor.id()) {
            result = space;
            break;
        }
        visited.push(ancestor.id());
        let declarations = ancestor
            .attribute("style")
            .filter(|style| style.contains(INTERPOLATION))
            .map(css::svg_declarations)
            .transpose()?;
        let value = declarations
            .as_ref()
            .and_then(|style| style.get(INTERPOLATION))
            .map(String::as_str)
            .or_else(|| ancestor.attribute(INTERPOLATION));
        let Some(value) = value else {
            continue;
        };
        let space = match value.trim().to_ascii_lowercase().as_str() {
            "srgb" => ColorInterpolation::SRGB,
            "linearrgb" | "initial" | "auto" => ColorInterpolation::LinearRGB,
            "inherit" | "unset" => continue,
            _ => return Err(unsupported("unsupported SVG lighting color interpolation")),
        };
        result = (space, Some(ancestor.id()));
        break;
    }
    cache.extend(visited.into_iter().map(|id| (id, result)));
    Ok(result)
}

fn linear_channel(channel: u8) -> u8 {
    let encoded = f64::from(channel) / 255.0;
    let linear = if encoded <= 0.04045 {
        encoded / 12.92
    } else {
        ((encoded + 0.055) / 1.055).powf(2.4)
    };
    (255.0 * linear).round() as u8
}

pub(super) fn masked_input<'a>(
    previous: &'a [Primitive],
    composite: &Composite,
) -> Option<&'a Kind> {
    if !matches!(composite.operator(), CompositeOperator::In)
        || !matches!(composite.input2(), Input::SourceAlpha)
    {
        return None;
    }
    let Input::Reference(name) = composite.input1() else {
        return None;
    };
    let input = previous
        .iter()
        .rfind(|primitive| primitive.result() == name)?
        .kind();
    matches!(input, Kind::DiffuseLighting(_) | Kind::SpecularLighting(_)).then_some(input)
}

pub(super) fn flat_alpha_mask(primitives: &[Primitive]) -> bool {
    let [light, output] = primitives else {
        return false;
    };
    let Kind::Composite(composite) = output.kind() else {
        return false;
    };
    // The light field of a flat surface does not depend on any source
    // samples. The final composite can only retain the original alpha.
    match masked_input(std::slice::from_ref(light), composite) {
        Some(Kind::DiffuseLighting(light)) => light.surface_scale() == 0.0,
        Some(Kind::SpecularLighting(light)) => light.surface_scale() == 0.0,
        _ => false,
    }
}

pub(super) fn work(
    primitive: &Primitive,
    filter: NonZeroRect,
    device: Transform,
    [width, height]: [u32; 2],
) -> Result<usize, EngineError> {
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
    // Linear output can require unpremultiplication, transfer, and
    // premultiplication before compositing on the rendered canvas.
    let color_work = if primitive.color_interpolation() == ColorInterpolation::LinearRGB {
        6
    } else {
        0
    };
    let passes = passes + color_work;
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
