//! Keep SVG paint-server inheritance local and bound filter work before rasterization.
use std::collections::{HashMap, HashSet};

use resvg::{tiny_skia::Transform, usvg};
use roxmltree::Node;

use super::{fragment_id, unqualified_attribute, XLINK};
use crate::{
    epub::{charge, css, invalid, limited, unsupported},
    EngineError,
};

const MAX_TEMPLATE_DEPTH: usize = 32;
const MAX_LAYER_PIXELS: usize = 8 * 1024 * 1024;
const MAX_TOTAL_PIXELS: usize = 32 * 1024 * 1024;
const MAX_DEVICE_SIGMA: f32 = 1024.0;
const MAX_COLOR_COEFFICIENT: f32 = 1024.0;
const MAX_SHADOW_OFFSET: f32 = wordcraft_render::MAX_SIDE as f32;

pub(in crate::epub) fn is_gradient(tag: &str) -> bool {
    matches!(tag, "linearGradient" | "radialGradient")
}

pub(in crate::epub) fn is_filter_primitive(tag: &str) -> bool {
    matches!(tag, "feGaussianBlur" | "feDropShadow" | "feColorMatrix")
}

pub(super) struct Effects<'a, 'input> {
    ids: HashMap<&'a str, Option<Node<'a, 'input>>>,
}

impl<'a, 'input> Effects<'a, 'input> {
    pub(super) fn new(root: Node<'a, 'input>) -> Self {
        let mut ids = HashMap::new();
        for node in root.descendants().filter(Node::is_element) {
            if let Some(id) = unqualified_attribute(node, "id") {
                ids.entry(id)
                    .and_modify(|entry| *entry = None)
                    .or_insert(Some(node));
            }
        }
        Self { ids }
    }

    fn target(&self, id: &str) -> Result<Node<'a, 'input>, EngineError> {
        match self.ids.get(id) {
            Some(Some(node)) => Ok(*node),
            Some(None) => Err(invalid("SVG effect references a duplicate ID")),
            None => Err(invalid("SVG effect references a missing ID")),
        }
    }

    pub(super) fn validate_filter<'v>(
        &self,
        node: Node<'_, '_>,
        value: &'v str,
    ) -> Result<Option<&'v str>, EngineError> {
        let value = value.trim();
        if value == "none" {
            return Ok(None);
        }
        if node
            .ancestors()
            .filter(Node::is_element)
            .any(|ancestor| matches!(ancestor.tag_name().name(), "pattern" | "mask" | "clipPath"))
        {
            return Err(unsupported(
                "SVG filters inside a pattern, mask or clip path",
            ));
        }
        let url = value
            .strip_prefix("url(")
            .and_then(|url| url.strip_suffix(')'))
            .map(str::trim)
            .ok_or_else(|| unsupported("SVG filter requires one local filter reference"))?;
        let url = ['\'', '"']
            .into_iter()
            .find_map(|quote| url.strip_prefix(quote)?.strip_suffix(quote))
            .unwrap_or(url);
        let id = fragment_id(url)
            .ok_or_else(|| unsupported("SVG filter requires a same-document fragment"))?;
        if self.target(id)?.tag_name().name() != "filter" {
            return Err(invalid("SVG filter references a non-filter element"));
        }
        Ok(Some(id))
    }

    fn validate_gradient(&self, mut node: Node<'a, 'input>) -> Result<(), EngineError> {
        let mut visited = HashSet::new();
        while let Some(href) =
            unqualified_attribute(node, "href").or_else(|| node.attribute((XLINK, "href")))
        {
            if !visited.insert(node.id()) {
                return Err(invalid("cyclic SVG gradient template reference"));
            }
            if visited.len() > MAX_TEMPLATE_DEPTH {
                return Err(limited("SVG gradient template depth exceeds 32"));
            }
            let id = fragment_id(href)
                .ok_or_else(|| unsupported("SVG gradient requires a same-document template"))?;
            node = self.target(id)?;
            if !is_gradient(node.tag_name().name()) {
                return Err(invalid("SVG gradient template must reference a gradient"));
            }
        }
        Ok(())
    }
}

pub(in crate::epub) fn validate_markup(root: Node<'_, '_>) -> Result<(), EngineError> {
    let effects = Effects::new(root);
    for node in root.descendants().filter(Node::is_element) {
        match node.tag_name().name() {
            tag if is_gradient(tag) => effects.validate_gradient(node)?,
            "filter" => {
                let mut children = node.children().filter(Node::is_element);
                if !children
                    .next()
                    .is_some_and(|child| is_filter_primitive(child.tag_name().name()))
                    || children.next().is_some()
                {
                    return Err(unsupported("SVG filter requires one supported primitive"));
                }
            }
            tag if is_filter_primitive(tag) => validate_primitive(node)?,
            _ => {}
        }
        if let Some(value) = node.attribute("filter") {
            effects.validate_filter(node, value)?;
        }
    }
    Ok(())
}

fn validate_primitive(node: Node<'_, '_>) -> Result<(), EngineError> {
    if !node
        .parent_element()
        .is_some_and(|parent| parent.has_tag_name("filter"))
        || node.children().any(|child| child.is_element())
    {
        return Err(invalid("misplaced SVG filter primitive"));
    }
    if node
        .attribute("in")
        .is_some_and(|value| !matches!(value, "SourceGraphic" | "SourceAlpha"))
    {
        return Err(unsupported(
            "SVG filters require SourceGraphic or SourceAlpha",
        ));
    }
    if node.has_tag_name("feColorMatrix") {
        return validate_color_matrix(node);
    }
    if let Some(value) = node.attribute("stdDeviation") {
        let numbers = if let Some((x, y)) = value.split_once(',') {
            [Some(x.trim()), Some(y.trim()), None]
        } else {
            let mut parts = value.split_ascii_whitespace();
            [parts.next(), parts.next(), parts.next()]
        };
        let mut count = 0;
        for number in numbers.into_iter().flatten() {
            let number = finite_number(number, "invalid SVG blur deviation")?;
            if number < 0.0 {
                return Err(invalid("invalid SVG blur deviation"));
            }
            if number > MAX_DEVICE_SIGMA {
                return Err(limited("SVG blur deviation exceeds 1024"));
            }
            count += 1;
        }
        if !(1..=2).contains(&count) {
            return Err(invalid("invalid SVG blur deviation"));
        }
    }
    if node.has_tag_name("feDropShadow") {
        for key in ["dx", "dy"] {
            if let Some(value) = node.attribute(key) {
                validate_offset(finite_number(value, "invalid SVG drop shadow offset")?)?;
            }
        }
        for property in ["flood-color", "flood-opacity"] {
            if let Some(value) = node.attribute(property) {
                flood_paint(property, value)?;
            }
        }
    }
    Ok(())
}

fn validate_color_matrix(node: Node<'_, '_>) -> Result<(), EngineError> {
    let mode = node.attribute("type").unwrap_or("matrix");
    let expected = match mode {
        "matrix" => 20,
        "saturate" | "hueRotate" => 1,
        _ => return Err(unsupported("unsupported SVG color matrix mode")),
    };
    let Some(raw) = node.attribute("values") else {
        return Ok(());
    };
    let message = "invalid SVG color matrix values";
    let mut values = [0.0; 20];
    let mut count = 0;
    for segment in raw.split(',') {
        if segment.trim().is_empty() {
            return Err(invalid(message));
        }
        for number in segment.split_ascii_whitespace() {
            let value = values.get_mut(count).ok_or_else(|| invalid(message))?;
            *value = finite_number(number, message)?;
            count += 1;
        }
    }
    if count != expected {
        return Err(invalid(message));
    }
    if mode == "matrix" {
        if values[15..] != [0.0, 0.0, 0.0, 1.0, 0.0] {
            return Err(unsupported("SVG color matrix must preserve alpha"));
        }
        if values
            .iter()
            .any(|value| value.abs() > MAX_COLOR_COEFFICIENT)
        {
            return Err(limited("SVG color matrix coefficient exceeds 1024"));
        }
    }
    if mode == "saturate" && !(0.0..=1.0).contains(&values[0]) {
        return Err(unsupported("SVG saturation must be between 0 and 1"));
    }
    Ok(())
}

fn finite_number(value: &str, reason: &'static str) -> Result<f32, EngineError> {
    value
        .trim()
        .parse::<f32>()
        .ok()
        .filter(|value| value.is_finite())
        .ok_or_else(|| invalid(reason))
}

pub(super) fn flood_paint(property: &str, value: &str) -> Result<String, EngineError> {
    let value = value.trim();
    if matches!(
        value,
        "inherit" | "initial" | "unset" | "revert" | "revert-layer"
    ) {
        return Ok(value.to_owned());
    }
    if property == "flood-color" {
        if value.eq_ignore_ascii_case("currentcolor") {
            return Ok("currentColor".into());
        }
        let ([red, green, blue], alpha) =
            css::color(value).ok_or_else(|| invalid("invalid SVG flood color"))?;
        // Canonical sRGB avoids the native SVG parser defaulting for CSS Color
        // syntax that it does not understand.
        Ok(format!("rgba({red},{green},{blue},{alpha})"))
    } else {
        let (value, divisor) = value
            .strip_suffix('%')
            .map_or((value, 1.0), |value| (value, 100.0));
        let opacity = finite_number(value, "invalid SVG flood opacity")? / divisor;
        Ok(opacity.clamp(0.0, 1.0).to_string())
    }
}

fn validate_offset(offset: f32) -> Result<(), EngineError> {
    if !offset.is_finite() || offset.abs() > MAX_SHADOW_OFFSET {
        return Err(limited("SVG drop shadow offset exceeds 16000 pixels"));
    }
    Ok(())
}

pub(super) fn validate_raster(tree: &usvg::Tree, scale: Transform) -> Result<(), EngineError> {
    for root in tree
        .patterns()
        .iter()
        .map(|pattern| pattern.root())
        .chain(tree.masks().iter().map(|mask| mask.root()))
        .chain(tree.clip_paths().iter().map(|clip| clip.root()))
    {
        if !root.filters().is_empty() || groups(root).any(contains_filter) {
            return Err(unsupported(
                "SVG filters inside a pattern, mask or clip path",
            ));
        }
    }
    let mut pixels = 0;
    for group in groups(tree.root()) {
        measure_group(group, scale, &mut pixels)?;
    }
    Ok(())
}

fn groups(root: &usvg::Group) -> impl Iterator<Item = &usvg::Group> {
    root.children().iter().filter_map(|node| match node {
        usvg::Node::Group(group) => Some(group.as_ref()),
        usvg::Node::Text(text) => Some(text.flattened()),
        _ => None,
    })
}

fn contains_filter(group: &usvg::Group) -> bool {
    !group.filters().is_empty() || groups(group).any(contains_filter)
}

fn measure_group(
    group: &usvg::Group,
    parent: Transform,
    pixels: &mut usize,
) -> Result<(), EngineError> {
    let mut transform = parent.pre_concat(group.transform());
    if !group.filters().is_empty() {
        // resvg isolates a filtered group in an axis-aligned layer. Its column
        // lengths become the effective scales, and translations change only its origin.
        let sx = transform.sx.hypot(transform.ky);
        let sy = transform.kx.hypot(transform.sy);
        transform = Transform::from_scale(sx, sy);
        let bounds = group
            .layer_bounding_box()
            .ok_or_else(|| invalid("invalid SVG filter bounds"))?;
        let width = bounds.width() * sx;
        let height = bounds.height() * sy;
        let area = width.ceil().max(1.0) * height.ceil().max(1.0);
        if !width.is_finite() || !height.is_finite() || area > MAX_LAYER_PIXELS as f32 {
            return Err(limited("SVG filter layer exceeds 8 Mi pixels"));
        }
        for filter in group.filters() {
            for primitive in filter.primitives() {
                let passes = match primitive.kind() {
                    usvg::filter::Kind::GaussianBlur(blur) => {
                        validate_blur(blur.std_dev_x().get() * sx, blur.std_dev_y().get() * sy)?;
                        1
                    }
                    usvg::filter::Kind::DropShadow(shadow) => {
                        validate_offset(shadow.dx() * sx)?;
                        validate_offset(shadow.dy() * sy)?;
                        validate_blur(
                            shadow.std_dev_x().get() * sx,
                            shadow.std_dev_y().get() * sy,
                        )?;
                        // Gaussian blur, flood, offset, and compositing each
                        // process the entire layer.
                        4
                    }
                    // Account for two color-space conversions, two alpha
                    // conversions, and the matrix multiplication.
                    usvg::filter::Kind::ColorMatrix(_) => 5,
                    _ => return Err(unsupported("unsupported SVG filter primitive")),
                };
                charge(
                    pixels,
                    (area as usize).saturating_mul(passes),
                    MAX_TOTAL_PIXELS,
                    "SVG filter work exceeds 32 Mi pixels",
                )?;
            }
        }
    }
    for child in groups(group) {
        measure_group(child, transform, pixels)?;
    }
    Ok(())
}

fn validate_blur(x: f32, y: f32) -> Result<(), EngineError> {
    if x > MAX_DEVICE_SIGMA || y > MAX_DEVICE_SIGMA {
        return Err(limited("SVG blur exceeds 1024 pixels after scaling"));
    }
    Ok(())
}
