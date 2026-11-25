//! Keep SVG paint-server inheritance local and bound filter work before rasterization.
use std::collections::{HashMap, HashSet};

use resvg::{tiny_skia::Transform, usvg};
use roxmltree::Node;

use super::{fragment_id, unqualified_attribute, XLINK};
use crate::{
    epub::{charge, css, invalid, limited, unsupported},
    EngineError,
};

#[path = "epub_svg_morphology.rs"]
mod morphology;
pub(super) use morphology::normalize as normalize_morphology;

#[path = "epub_svg_noise.rs"]
mod noise;

#[path = "epub_svg_transfer.rs"]
mod transfer;
pub(in crate::epub) use transfer::is_channel as is_transfer_channel;

const MAX_TEMPLATE_DEPTH: usize = 32;
const MAX_FILTER_PRIMITIVES: usize = 16;
const MAX_MERGE_INPUTS: usize = 16;
const MAX_RESULT_BYTES: usize = 128;
const MAX_LAYER_PIXELS: usize = 8 * 1024 * 1024;
const MAX_TOTAL_PIXELS: usize = 32 * 1024 * 1024;
const MAX_DEVICE_SIGMA: f32 = 1024.0;
const MAX_COLOR_COEFFICIENT: f32 = 1024.0;
const MAX_OFFSET: f32 = wordcraft_render::MAX_SIDE as f32;

pub(in crate::epub) fn is_gradient(tag: &str) -> bool {
    matches!(tag, "linearGradient" | "radialGradient")
}

pub(in crate::epub) fn is_filter_primitive(tag: &str) -> bool {
    matches!(
        tag,
        "feGaussianBlur"
            | "feDropShadow"
            | "feColorMatrix"
            | "feOffset"
            | "feFlood"
            | "feComposite"
            | "feMerge"
            | "feBlend"
            | "feComponentTransfer"
            | "feMorphology"
            | "feTurbulence"
            | "feDisplacementMap"
    )
}

pub(in crate::epub) fn has_flood_paint(tag: &str) -> bool {
    matches!(tag, "feDropShadow" | "feFlood")
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
            tag if is_transfer_channel(tag) => transfer::validate_channel(node)?,
            "feMergeNode"
                if !node
                    .parent_element()
                    .is_some_and(|parent| parent.has_tag_name("feMerge"))
                    || node.children().any(|child| child.is_element()) =>
            {
                return Err(invalid("misplaced SVG filter merge input"));
            }
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
        match child.tag_name().name() {
            "feFlood" | "feMerge" | "feTurbulence" if child.has_attribute("in") => {
                return Err(invalid("unexpected SVG filter input"));
            }
            "feMerge" => {
                let mut count = 0;
                for input in child.children().filter(Node::is_element) {
                    if !input.has_tag_name("feMergeNode") {
                        return Err(invalid("SVG filter merge requires feMergeNode children"));
                    }
                    count += 1;
                    if count > MAX_MERGE_INPUTS {
                        return Err(limited("SVG filter merge exceeds 16 inputs"));
                    }
                    validate_input(input.attribute("in"), &results)?;
                }
                if count == 0 {
                    return Err(invalid("SVG filter merge requires an input"));
                }
            }
            "feComposite" | "feBlend" | "feDisplacementMap" => {
                validate_input(child.attribute("in"), &results)?;
                validate_input(child.attribute("in2"), &results)?;
            }
            _ => validate_input(child.attribute("in"), &results)?,
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

fn validate_input(input: Option<&str>, results: &HashSet<&str>) -> Result<(), EngineError> {
    match input {
        None | Some("SourceGraphic" | "SourceAlpha") => Ok(()),
        Some("BackgroundImage" | "BackgroundAlpha" | "FillPaint" | "StrokePaint") => Err(
            unsupported("SVG filters require SourceGraphic or SourceAlpha or a prior named result"),
        ),
        Some(input) if results.contains(input) => Ok(()),
        Some(_) => Err(invalid(
            "SVG filter input must reference a prior named result",
        )),
    }
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
        || (!matches!(node.tag_name().name(), "feMerge" | "feComponentTransfer")
            && node.children().any(|child| child.is_element()))
    {
        return Err(invalid("misplaced SVG filter primitive"));
    }
    validate_region(node)?;
    match node.tag_name().name() {
        "feColorMatrix" => return validate_color_matrix(node),
        "feComposite" => return validate_composite(node),
        "feComponentTransfer" => return transfer::validate_channels(node),
        "feMorphology" => return morphology::validate(node),
        "feTurbulence" => return noise::validate_turbulence(node),
        "feDisplacementMap" => return noise::validate_displacement(node),
        "feBlend" => {
            if !matches!(
                node.attribute("mode").unwrap_or("normal"),
                "normal"
                    | "multiply"
                    | "screen"
                    | "overlay"
                    | "darken"
                    | "lighten"
                    | "color-dodge"
                    | "color-burn"
                    | "hard-light"
                    | "soft-light"
                    | "difference"
                    | "exclusion"
                    | "hue"
                    | "saturation"
                    | "color"
                    | "luminosity"
            ) {
                return Err(unsupported("unsupported SVG filter blend mode"));
            }
            return Ok(());
        }
        "feOffset" => {
            // resvg translates the input raster and retains its subregion.
            if ["x", "y", "width", "height"]
                .iter()
                .any(|key| node.has_attribute(*key))
            {
                return Err(unsupported("SVG feOffset primitive subregions"));
            }
            return validate_offsets(
                node,
                "invalid SVG filter offset",
                "SVG filter offset exceeds 16000 pixels",
            );
        }
        _ => {}
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
        validate_offsets(
            node,
            "invalid SVG drop shadow offset",
            "SVG drop shadow offset exceeds 16000 pixels",
        )?;
    }
    if has_flood_paint(node.tag_name().name()) {
        for property in ["flood-color", "flood-opacity"] {
            if let Some(value) = node.attribute(property) {
                flood_paint(property, value)?;
            }
        }
    }
    Ok(())
}

fn validate_offsets(
    node: Node<'_, '_>,
    invalid_value: &'static str,
    budget: &'static str,
) -> Result<(), EngineError> {
    for key in ["dx", "dy"] {
        if let Some(value) = node.attribute(key) {
            validate_offset(finite_number(value, invalid_value)?, budget)?;
        }
    }
    Ok(())
}

fn validate_composite(node: Node<'_, '_>) -> Result<(), EngineError> {
    let operator = node.attribute("operator").unwrap_or("over");
    if !matches!(
        operator,
        "over" | "in" | "out" | "atop" | "xor" | "lighter" | "arithmetic"
    ) {
        return Err(unsupported("unsupported SVG filter compositing operator"));
    }
    for key in ["k1", "k2", "k3", "k4"] {
        if let Some(value) = node.attribute(key) {
            if operator != "arithmetic" {
                return Err(invalid("SVG composite coefficients require arithmetic"));
            }
            if finite_number(value, "invalid SVG filter composite coefficient")?.abs()
                > MAX_COLOR_COEFFICIENT
            {
                return Err(limited("SVG filter composite coefficient exceeds 1024"));
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

fn number_pair(raw: &str, error: &'static str) -> Result<[f32; 2], EngineError> {
    if !raw.is_ascii() {
        return Err(invalid(error));
    }
    if let Some((x, y)) = raw.split_once(',') {
        return Ok([finite_number(x, error)?, finite_number(y, error)?]);
    }
    let mut values = raw.split_ascii_whitespace();
    let x = finite_number(values.next().ok_or_else(|| invalid(error))?, error)?;
    let y = values.next().map_or(Ok(x), |y| finite_number(y, error))?;
    if values.next().is_some() {
        return Err(invalid(error));
    }
    Ok([x, y])
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

fn validate_offset(offset: f32, message: &'static str) -> Result<(), EngineError> {
    if !offset.is_finite() || offset.abs() > MAX_OFFSET {
        return Err(limited(message));
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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum FilterGeometry {
    #[default]
    Retained,
    Sample,
    Moved,
    Unproven,
}

#[derive(Clone, Copy, Default)]
struct Support {
    retains_source: bool,
    within_source: bool,
    procedural: bool,
}

impl Support {
    fn union(self, other: Self) -> Self {
        Self {
            retains_source: self.retains_source || other.retains_source,
            within_source: self.within_source && other.within_source,
            procedural: self.procedural || other.procedural,
        }
    }

    fn subset(within_source: bool) -> Self {
        Self {
            retains_source: false,
            within_source,
            procedural: false,
        }
    }
}

// Results are classified in the native filter's raster coordinates. A branch
// can retain all source locations, stay within them, or satisfy neither proof.
fn filter_support(filter: &usvg::filter::Filter, bounds: usvg::Rect) -> Support {
    use usvg::filter::{ColorMatrixKind, CompositeOperator, Input, Kind};
    let covers = |region: usvg::NonZeroRect| {
        region.x() <= bounds.x()
            && region.y() <= bounds.y()
            && region.right() >= bounds.right()
            && region.bottom() >= bounds.bottom()
    };
    let mut results = HashMap::<&str, Support>::new();
    let source = Support {
        retains_source: covers(filter.rect()),
        within_source: true,
        procedural: false,
    };
    let mut output = Support::default();
    for primitive in filter.primitives() {
        let get = |input: &Input| match input {
            Input::SourceGraphic | Input::SourceAlpha => source,
            Input::Reference(name) => results.get(name.as_str()).copied().unwrap_or_default(),
        };
        output = match primitive.kind() {
            Kind::ColorMatrix(matrix) => {
                let input = get(matrix.input());
                match matrix.kind() {
                    ColorMatrixKind::Matrix(values) => Support {
                        // A gain of at least one cannot quantize nonzero alpha away.
                        retains_source: input.retains_source
                            && values[15..18] == [0.; 3]
                            && values[18] >= 1.0
                            && values[19] == 0.0,
                        // The native input is premultiplied and transparent pixels have
                        // zero color. A nonpositive bias cannot create coverage there.
                        within_source: input.within_source && values[19] <= 0.0,
                        procedural: input.procedural,
                    },
                    ColorMatrixKind::LuminanceToAlpha => Support {
                        retains_source: false,
                        ..input
                    },
                    ColorMatrixKind::Saturate(_) | ColorMatrixKind::HueRotate(_) => input,
                }
            }
            Kind::GaussianBlur(blur)
                if blur.std_dev_x().get() == 0.0 && blur.std_dev_y().get() == 0.0 =>
            {
                get(blur.input())
            }
            Kind::GaussianBlur(blur) => Support {
                procedural: get(blur.input()).procedural,
                ..Support::default()
            },
            Kind::DropShadow(shadow) => Support {
                within_source: false,
                ..get(shadow.input())
            },
            Kind::Offset(offset) if offset.dx() == 0.0 && offset.dy() == 0.0 => get(offset.input()),
            Kind::DisplacementMap(map) if map.scale() == 0.0 => get(map.input1()),
            Kind::DisplacementMap(_) | Kind::Turbulence(_) => Support {
                procedural: true,
                ..Support::default()
            },
            Kind::Flood(flood) if flood.opacity().get() == 0.0 => Support::subset(true),
            // Native blend modes use the same source-over alpha as normal.
            Kind::Blend(blend) => get(blend.input1()).union(get(blend.input2())),
            Kind::ComponentTransfer(transfer) => {
                let input = get(transfer.input());
                let alpha = transfer::alpha_coverage(transfer.func_a());
                Support {
                    retains_source: input.retains_source && alpha.retains_ink,
                    within_source: input.within_source && alpha.keeps_transparency,
                    procedural: input.procedural,
                }
            }
            Kind::Morphology(morphology) => {
                let input = get(morphology.input());
                match morphology.operator() {
                    usvg::filter::MorphologyOperator::Erode => Support {
                        retains_source: false,
                        ..input
                    },
                    usvg::filter::MorphologyOperator::Dilate => Support {
                        within_source: false,
                        ..input
                    },
                }
            }
            Kind::Composite(composite) => {
                let (first, second) = (get(composite.input1()), get(composite.input2()));
                let support = match composite.operator() {
                    CompositeOperator::Over | CompositeOperator::Lighter => first.union(second),
                    CompositeOperator::In => {
                        Support::subset(first.within_source || second.within_source)
                    }
                    CompositeOperator::Out => Support::subset(first.within_source),
                    CompositeOperator::Atop => Support::subset(second.within_source),
                    CompositeOperator::Xor => {
                        Support::subset(first.within_source && second.within_source)
                    }
                    CompositeOperator::Arithmetic { k4, .. } => {
                        Support::subset(k4 <= 0.0 && first.within_source && second.within_source)
                    }
                };
                Support {
                    procedural: first.procedural || second.procedural,
                    ..support
                }
            }
            Kind::Merge(merge) => merge
                .inputs()
                .iter()
                .fold(Support::subset(true), |output, input| {
                    output.union(get(input))
                }),
            _ => Support::default(),
        };
        output.retains_source &= covers(primitive.rect());
        results.insert(primitive.result(), output);
    }
    output
}

// Follow only the input of the final result. Discarded branches do not
// transport ink. A nonzero alpha bias, blend, merge or blur is not a
// one-to-one translation of the source raster.
fn offset_chain(filter: &usvg::filter::Filter) -> Option<(Vec<[f32; 2]>, bool)> {
    use usvg::filter::{ColorMatrixKind, Input, Kind};
    let primitives = filter.primitives();
    let mut current = primitives.len().checked_sub(1)?;
    let mut offsets = Vec::new();
    let mut preserves_alpha = true;
    loop {
        let input = match primitives[current].kind() {
            Kind::Offset(offset) => {
                offsets.push([offset.dx(), offset.dy()]);
                offset.input()
            }
            Kind::ColorMatrix(matrix) => {
                match matrix.kind() {
                    ColorMatrixKind::Matrix(values) if values[19] <= 0.0 => {
                        preserves_alpha &=
                            values[15..18] == [0.; 3] && values[18] >= 1.0 && values[19] == 0.0;
                    }
                    ColorMatrixKind::LuminanceToAlpha => preserves_alpha = false,
                    ColorMatrixKind::Saturate(_) | ColorMatrixKind::HueRotate(_) => {}
                    _ => return None,
                }
                matrix.input()
            }
            Kind::ComponentTransfer(transfer) => {
                let alpha = transfer::alpha_coverage(transfer.func_a());
                if !alpha.keeps_transparency {
                    return None;
                }
                preserves_alpha &= alpha.retains_ink;
                transfer.input()
            }
            Kind::Morphology(morphology)
                if morphology.operator() == usvg::filter::MorphologyOperator::Erode =>
            {
                preserves_alpha = false;
                morphology.input()
            }
            Kind::DisplacementMap(map) if map.scale() == 0.0 => map.input1(),
            Kind::GaussianBlur(blur)
                if blur.std_dev_x().get() == 0.0 && blur.std_dev_y().get() == 0.0 =>
            {
                blur.input()
            }
            _ => return None,
        };
        match input {
            Input::SourceGraphic | Input::SourceAlpha => break,
            Input::Reference(name) => {
                current = primitives[..current]
                    .iter()
                    .rposition(|candidate| candidate.result() == name)?;
            }
        }
    }
    offsets
        .iter()
        .any(|&step| step != [0.0, 0.0])
        .then_some((offsets, preserves_alpha))
}

fn filter_geometry(group: &usvg::Group, additive_input: bool) -> (FilterGeometry, bool, bool) {
    let [filter] = group.filters() else {
        return if group.filters().is_empty() {
            (FilterGeometry::Retained, additive_input, false)
        } else {
            (FilterGeometry::Unproven, true, false)
        };
    };
    let Some(bounds) = group.stroke_bounding_box() else {
        return (FilterGeometry::Unproven, true, false);
    };
    let output = filter_support(filter, bounds);
    let filter = if output.retains_source {
        FilterGeometry::Retained
    } else if output.within_source && !additive_input && at_most_one_leaf(group, &mut false) {
        // Measure the actual native effect when it can only remove or weaken
        // ink from a single source. Overlapping sources would be ambiguous.
        FilterGeometry::Sample
    } else {
        FilterGeometry::Unproven
    };
    (
        filter,
        additive_input || !output.within_source,
        output.procedural,
    )
}

fn at_most_one_leaf(group: &usvg::Group, seen: &mut bool) -> bool {
    group.children().iter().all(|node| match node {
        usvg::Node::Group(group) => at_most_one_leaf(group, seen),
        _ => !std::mem::replace(seen, true),
    })
}

#[derive(Clone, Copy, Default)]
pub(super) struct GeometryStatus {
    pub(super) filter: FilterGeometry,
    pub(super) unproven_subtree: bool,
    pub(super) sample_subtree: bool,
    pub(super) moved_subtree: bool,
    pub(super) moved_children: bool,
    pub(super) needs_context: bool,
    pub(super) context_subtree: bool,
    pub(super) one_context_source: bool,
    filters_subtree: bool,
    incompatible_stack: bool,
}

#[derive(Default)]
pub(super) struct CaptureGeometry {
    groups: HashMap<*const usvg::Group, GeometryStatus>,
    moves: HashMap<*const usvg::Group, Vec<[f32; 2]>>,
    pub(super) density: [f32; 2],
}

impl CaptureGeometry {
    pub(super) fn new(tree: &usvg::Tree, density: [f32; 2]) -> Self {
        let mut capture = Self {
            density,
            ..Self::default()
        };
        capture.analyze(tree.root());
        capture
    }

    fn analyze(&mut self, group: &usvg::Group) -> (GeometryStatus, bool) {
        let mut status = GeometryStatus::default();
        if group.opacity().get() == 0.0 {
            return (status, false);
        }
        let mut additive_input = false;
        for child in groups(group) {
            let (child, additive) = self.analyze(child);
            status.unproven_subtree |= child.unproven_subtree;
            status.sample_subtree |= child.sample_subtree;
            status.moved_children |= child.moved_subtree;
            status.context_subtree |= child.context_subtree;
            status.filters_subtree |= child.filters_subtree;
            status.incompatible_stack |= child.incompatible_stack;
            additive_input |= additive;
        }
        let (mut filter, additive, procedural) = filter_geometry(group, additive_input);
        let key = std::ptr::from_ref(group);
        if !group.filters().is_empty() {
            let axis_aligned = positive_axes(group.abs_transform());
            let erosion = group
                .filters()
                .iter()
                .flat_map(|filter| filter.primitives())
                .any(|primitive| {
                    matches!(primitive.kind(), usvg::filter::Kind::Morphology(value)
                    if value.operator() == usvg::filter::MorphologyOperator::Erode)
                });
            let can_stack = status.moved_children
                && !status.incompatible_stack
                && axis_aligned
                && at_most_one_leaf(group, &mut false);
            if status.moved_children
                || status.context_subtree
                || (erosion || procedural) && !axis_aligned && filter == FilterGeometry::Sample
            {
                filter = FilterGeometry::Unproven;
            }
            if filter == FilterGeometry::Unproven
                && (!status.filters_subtree || can_stack)
                && (!erosion || axis_aligned)
            {
                if let [native] = group.filters() {
                    if let Some((offsets, preserves_alpha)) = offset_chain(native) {
                        if preserves_alpha || can_stack || at_most_one_leaf(group, &mut false) {
                            self.moves.insert(key, offsets);
                            filter = FilterGeometry::Moved;
                        }
                    }
                }
            }
            // Positive axis-aligned layers rebase their children by whole
            // device pixels. Other layers can remove a rotation/shear from
            // the next layer's raster grid and cannot compose this way.
            status.incompatible_stack |= filter != FilterGeometry::Moved || !axis_aligned;
            status.filters_subtree = true;
            status.needs_context = (erosion || status.moved_children)
                && filter == FilterGeometry::Moved
                || (erosion || procedural) && filter == FilterGeometry::Sample;
        }
        status.filter = filter;
        status.unproven_subtree |= filter == FilterGeometry::Unproven;
        status.sample_subtree |= matches!(filter, FilterGeometry::Sample | FilterGeometry::Moved);
        status.moved_subtree = status.moved_children || filter == FilterGeometry::Moved;
        status.context_subtree |= status.needs_context;
        status.one_context_source =
            status.context_subtree && group.should_isolate() && at_most_one_leaf(group, &mut false);
        if status.unproven_subtree || status.sample_subtree {
            self.groups.insert(key, status);
        }
        (status, additive)
    }

    pub(super) fn shift(&self, group: &usvg::Group, transform: Transform) -> Option<[f32; 2]> {
        let steps = self.moves.get(&std::ptr::from_ref(group))?;
        // Match resvg's isolated raster, including the scale after removing
        // its rotation/shear and truncating each feOffset to a pixel step.
        let sx = transform.sx.hypot(transform.ky);
        let sy = transform.kx.hypot(transform.sy);
        let rest = Transform::from_row(
            transform.sx / sx,
            transform.ky / sx,
            transform.kx / sy,
            transform.sy / sy,
            0.0,
            0.0,
        );
        let layer = rest.invert()?.pre_concat(transform);
        let (sx, sy) = layer.get_scale();
        let mut pixels = [0i32; 2];
        for [x, y] in steps {
            let values = [*x * sx, *y * sy];
            for (sum, value) in pixels.iter_mut().zip(values) {
                if !value.is_finite() || value.abs() > MAX_OFFSET {
                    return None;
                }
                *sum += value as i32;
            }
        }
        let [x, y] = pixels.map(|value| value as f32);
        let shift = [
            (rest.sx * x + rest.kx * y) / self.density[0],
            (rest.ky * x + rest.sy * y) / self.density[1],
        ];
        shift.into_iter().all(f32::is_finite).then_some(shift)
    }

    pub(super) fn status(&self, node: &usvg::Node) -> GeometryStatus {
        match node {
            usvg::Node::Group(group) => self
                .groups
                .get(&std::ptr::from_ref(group.as_ref()))
                .copied()
                .unwrap_or_default(),
            usvg::Node::Text(text) => {
                // Native text exposes its outer filter as a group. A filter
                // inside flattened text is not separately measurable per glyph.
                let mut status = self
                    .groups
                    .get(&std::ptr::from_ref(text.flattened()))
                    .copied()
                    .unwrap_or_default();
                status.unproven_subtree |= status.sample_subtree;
                status
            }
            _ => GeometryStatus::default(),
        }
    }
}

fn positive_axes(transform: Transform) -> bool {
    transform.kx == 0.0
        && transform.ky == 0.0
        && transform.sx.is_finite()
        && transform.sx > 0.0
        && transform.sy.is_finite()
        && transform.sy > 0.0
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
                        let message = "SVG drop shadow offset exceeds 16000 pixels";
                        validate_offset(shadow.dx() * sx, message)?;
                        validate_offset(shadow.dy() * sy, message)?;
                        8 + blur_work(shadow.std_dev_x().get() * sx, shadow.std_dev_y().get() * sy)?
                    }
                    usvg::filter::Kind::ColorMatrix(_) => 8,
                    usvg::filter::Kind::Flood(_) => 5,
                    usvg::filter::Kind::Offset(offset) => {
                        let message = "SVG filter offset exceeds 16000 pixels after scaling";
                        validate_offset(offset.dx() * sx, message)?;
                        validate_offset(offset.dy() * sy, message)?;
                        5
                    }
                    usvg::filter::Kind::Composite(_) | usvg::filter::Kind::Blend(_) => 12,
                    usvg::filter::Kind::ComponentTransfer(transfer) => transfer::work(transfer),
                    usvg::filter::Kind::Merge(merge) => 4 + 5 * merge.inputs().len(),
                    usvg::filter::Kind::Turbulence(turbulence) => {
                        charge_work(pixels, 1, noise::TURBULENCE_SETUP_WORK)?;
                        noise::turbulence_work(turbulence, bounds, [sx, sy])?
                    }
                    usvg::filter::Kind::DisplacementMap(map) => {
                        let message = "SVG displacement scale exceeds 16000 after scaling";
                        validate_offset(map.scale() * sx, message)?;
                        validate_offset(map.scale() * sy, message)?;
                        16
                    }
                    usvg::filter::Kind::Morphology(morphology) => morphology::work(
                        [
                            morphology.radius_x().get() * sx,
                            morphology.radius_y().get() * sy,
                        ],
                        [
                            width.ceil().max(1.0) as usize,
                            height.ceil().max(1.0) as usize,
                        ],
                    )?,
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
