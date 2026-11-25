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
const MAX_FILTER_PRIMITIVES: usize = 16;
const MAX_RESULT_BYTES: usize = 128;
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
            "filter" => validate_chain(node)?,
            tag if is_filter_primitive(tag) => validate_primitive(node)?,
            _ => {}
        }
        if let Some(value) = node.attribute("filter") {
            effects.validate_filter(node, value)?;
        }
    }
    Ok(())
}

fn validate_chain(node: Node<'_, '_>) -> Result<(), EngineError> {
    for attribute in ["filterUnits", "primitiveUnits"] {
        if node
            .attribute(attribute)
            .is_some_and(|value| !matches!(value, "userSpaceOnUse" | "objectBoundingBox"))
        {
            return Err(invalid("invalid SVG filter units"));
        }
    }
    validate_region(node)?;
    let mut results = HashSet::new();
    let mut children = node.children().filter(Node::is_element).peekable();
    if children.peek().is_none() {
        return Err(invalid("SVG filter requires a primitive"));
    }
    for (index, child) in children.enumerate() {
        if index >= MAX_FILTER_PRIMITIVES {
            return Err(limited("SVG filter exceeds 16 primitives"));
        }
        if !is_filter_primitive(child.tag_name().name()) {
            return Err(unsupported("unsupported SVG filter primitive"));
        }
        match child.attribute("in") {
            None | Some("SourceGraphic" | "SourceAlpha") => {}
            Some("BackgroundImage" | "BackgroundAlpha" | "FillPaint" | "StrokePaint") => {
                return Err(unsupported(
                    "SVG filters require SourceGraphic or SourceAlpha or a prior named result",
                ));
            }
            Some(input) if results.contains(input) => {}
            Some(_) => {
                return Err(invalid(
                    "SVG filter input must reference a prior named result",
                ))
            }
        }
        if let Some(result) = child.attribute("result") {
            if !result_name(result) {
                return Err(invalid("invalid SVG filter result name"));
            }
            if !results.insert(result) {
                return Err(invalid("duplicate SVG filter result name"));
            }
        }
    }
    Ok(())
}

fn result_name(value: &str) -> bool {
    let mut chars = value.chars();
    value.len() <= MAX_RESULT_BYTES
        && !matches!(
            value,
            "SourceGraphic"
                | "SourceAlpha"
                | "BackgroundImage"
                | "BackgroundAlpha"
                | "FillPaint"
                | "StrokePaint"
                | "inherit"
                | "initial"
                | "unset"
                | "revert"
                | "revert-layer"
                | "default"
        )
        && chars
            .next()
            .is_some_and(|ch| ch == '_' || ch == '-' || ch.is_alphabetic())
        && chars.all(|ch| ch == '_' || ch == '-' || ch.is_alphanumeric())
}

fn validate_region(node: Node<'_, '_>) -> Result<(), EngineError> {
    for (attribute, positive) in [
        ("x", false),
        ("y", false),
        ("width", true),
        ("height", true),
    ] {
        if let Some(length) = node.attribute(attribute) {
            let length = length.trim();
            let value = ["%", "px", "em", "ex", "in", "cm", "mm", "pt", "pc"]
                .into_iter()
                .find_map(|suffix| length.strip_suffix(suffix))
                .unwrap_or(length);
            let value = finite_number(value, "invalid SVG filter region")?;
            if positive && value <= 0.0 {
                return Err(invalid("invalid SVG filter region"));
            }
            if value.abs() > 1_000_000.0 {
                return Err(limited("SVG filter region length exceeds 1000000"));
            }
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
    validate_region(node)?;
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
        "luminanceToAlpha" => 0,
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
    if mode == "matrix"
        && values
            .iter()
            .any(|value| value.abs() > MAX_COLOR_COEFFICIENT)
    {
        return Err(limited("SVG color matrix coefficient exceeds 1024"));
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
    measure_group(tree.root(), scale, &mut pixels)
}

// A filtered layer can crop the source and then change its alpha at each result.
// Follow the final result's actual input chain. Other branches do not reach the canvas.
pub(super) fn unproven_geometry(tree: &usvg::Tree) -> bool {
    fn proof_failed(group: &usvg::Group) -> bool {
        group.filters().iter().any(|filter| {
            let Some(bounds) = group.stroke_bounding_box() else {
                return true;
            };
            let covers = |region: usvg::NonZeroRect| {
                region.x() <= bounds.x()
                    && region.y() <= bounds.y()
                    && region.right() >= bounds.right()
                    && region.bottom() >= bounds.bottom()
            };
            if !covers(filter.rect()) {
                return true;
            }
            let mut safe = HashMap::new();
            for primitive in filter.primitives() {
                use usvg::filter::{ColorMatrixKind, Input, Kind};
                let (input, preserves) = match primitive.kind() {
                    Kind::ColorMatrix(matrix) => {
                        let preserves = match matrix.kind() {
                            ColorMatrixKind::Matrix(values) => values[15..] == [0., 0., 0., 1., 0.],
                            ColorMatrixKind::LuminanceToAlpha => false,
                            ColorMatrixKind::Saturate(_) | ColorMatrixKind::HueRotate(_) => true,
                        };
                        (matrix.input(), preserves)
                    }
                    Kind::GaussianBlur(blur) => (
                        blur.input(),
                        blur.std_dev_x().get() == 0.0 && blur.std_dev_y().get() == 0.0,
                    ),
                    // The native implementation paints the original input over the shadow.
                    Kind::DropShadow(shadow) => (shadow.input(), true),
                    _ => return true,
                };
                let input_safe = match input {
                    Input::SourceGraphic | Input::SourceAlpha => true,
                    Input::Reference(name) => safe.get(name.as_str()).copied().unwrap_or(false),
                };
                safe.insert(
                    primitive.result(),
                    preserves && input_safe && covers(primitive.rect()),
                );
            }
            filter
                .primitives()
                .last()
                .is_some_and(|last| safe.get(last.result()) != Some(&true))
        }) || groups(group).any(proof_failed)
    }

    proof_failed(tree.root())
}

pub(super) fn geometry_error() -> EngineError {
    unsupported(
        "SVG filter can change the painted source geometry used for PDF text and annotations",
    )
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
        let area = area as usize;
        // Charge the isolated layer and the native filter's input/output canvas copies.
        charge_work(pixels, area, 3)?;
        for filter in group.filters() {
            for primitive in filter.primitives() {
                let region = primitive.rect();
                if [
                    region.x() * sx,
                    region.y() * sy,
                    region.right() * sx,
                    region.bottom() * sy,
                ]
                .iter()
                .any(|value| !value.is_finite() || value.abs() > i32::MAX as f32 / 8.0)
                {
                    return Err(limited("SVG filter subregion exceeds device coordinates"));
                }
                // The native filter keeps prior results alive. Charge even unused
                // results for the full layer, input copies and per-primitive output.
                let passes = match primitive.kind() {
                    usvg::filter::Kind::GaussianBlur(blur) => {
                        3 + blur_work(blur.std_dev_x().get() * sx, blur.std_dev_y().get() * sy)?
                    }
                    usvg::filter::Kind::DropShadow(shadow) => {
                        validate_offset(shadow.dx() * sx)?;
                        validate_offset(shadow.dy() * sy)?;
                        8 + blur_work(shadow.std_dev_x().get() * sx, shadow.std_dev_y().get() * sy)?
                    }
                    usvg::filter::Kind::ColorMatrix(_) => 8,
                    _ => return Err(unsupported("unsupported SVG filter primitive")),
                };
                charge_work(pixels, area, passes)?;
            }
        }
    }
    for child in groups(group) {
        measure_group(child, transform, pixels)?;
    }
    Ok(())
}

fn charge_work(pixels: &mut usize, area: usize, passes: usize) -> Result<(), EngineError> {
    charge(
        pixels,
        area.saturating_mul(passes),
        MAX_TOTAL_PIXELS,
        "SVG filter work exceeds 32 Mi pixels",
    )
}

fn blur_work(x: f32, y: f32) -> Result<usize, EngineError> {
    if !x.is_finite() || !y.is_finite() || x > MAX_DEVICE_SIGMA || y > MAX_DEVICE_SIGMA {
        return Err(limited("SVG blur exceeds 1024 pixels after scaling"));
    }
    // Both native kernels make multiple sweeps over the layer and convert colors.
    Ok(if x > 0.0 || y > 0.0 { 18 } else { 0 })
}
