//! Fit explicit inline lengths after native mixed-face shaping.
use std::collections::{HashMap, HashSet};

use resvg::usvg;
use roxmltree::Node;

use crate::{
    author_epub::escape_attribute,
    epub::{
        charge,
        css::svg_declarations,
        invalid, limited, parse_xml,
        structure::{append, apply_edits},
        svg::{self, NS},
        unsupported, XML_BYTES,
    },
    EngineError,
};

#[path = "epub_svg_nested_length.rs"]
mod nested;

#[path = "epub_svg_relative_length.rs"]
mod relative;

#[path = "epub_svg_font_spacing.rs"]
mod spacing;
use spacing::SpacingCheck;

const MAX_LENGTH: f32 = 25_000.0;
const MAX_OWNERS: usize = 64;
type Edit = (std::ops::Range<usize>, String);

pub(in crate::epub::svg) struct TextLength {
    id: String,
    temporary_id: bool,
    owners: Vec<LengthOwner>,
    nested: Option<nested::NestedLength>,
    relative: Option<relative::RelativePositions>,
}

struct LengthOwner {
    target: f32,
    adjust: usvg::LengthAdjust,
    chunk: Option<PositionedChunk>,
}

#[derive(Clone, Copy)]
struct PositionedChunk {
    index: usize,
    count: usize,
    probe_y: Option<f32>,
}

pub(in crate::epub::svg) struct FittedLengths {
    pub(in crate::epub::svg) source: String,
    spacing: Vec<SpacingCheck>,
}

pub(super) fn prepare_lengths(
    root: Node<'_, '_>,
    owners: Vec<Node<'_, '_>>,
    ids: &HashMap<&str, bool>,
    next_id: &mut usize,
    source: &str,
    edits: &mut Vec<Edit>,
) -> Result<TextLength, EngineError> {
    if owners.len() > MAX_OWNERS {
        return Err(limited(
            "EPUB SVG text has more than 64 mixed-font length owners",
        ));
    }
    if root.descendants().filter(Node::is_element).any(|node| {
        node.has_attribute("rotate")
            || node != root
                && (node.has_attribute("dx")
                    || node.has_attribute("dy")
                    || node.has_tag_name((NS, "text")))
            || node.has_tag_name((NS, "textPath"))
    }) {
        return Err(unsupported(
            "EPUB SVG mixed-font textLength requires linear chunks without rotation or descendant per-glyph positioning",
        ));
    }
    for effect in ["filter", "clip-path", "mask"] {
        if root.attribute(effect).is_some_and(|value| value != "none") {
            return Err(unsupported(
                "EPUB SVG mixed-font textLength with an effect on the text element",
            ));
        }
    }
    let style = root
        .attribute("style")
        .map(svg_declarations)
        .transpose()?
        .unwrap_or_default();
    if ["filter", "clip-path", "mask"]
        .iter()
        .any(|key| style.get(*key).is_some_and(|value| value != "none"))
    {
        return Err(unsupported(
            "EPUB SVG mixed-font textLength with an effect on the text element",
        ));
    }
    let (id, temporary_id) = measurement_id(root, ids, next_id, source, edits)?;
    let nested = nested::prepare(root, ids, next_id, source, edits)?;
    let owners = if nested.is_some() {
        Vec::new()
    } else {
        owners
            .into_iter()
            .map(|owner| prepare_owner(root, owner, edits))
            .collect::<Result<_, _>>()?
    };
    let relative = relative::RelativePositions::read(root)?;
    if relative.is_some()
        && (nested.is_some() || !matches!(owners.as_slice(), [owner] if owner.chunk.is_none()))
    {
        return Err(relative::error());
    }
    Ok(TextLength {
        id,
        temporary_id,
        owners,
        nested,
        relative,
    })
}

fn measurement_id(
    node: Node<'_, '_>,
    ids: &HashMap<&str, bool>,
    next_id: &mut usize,
    source: &str,
    edits: &mut Vec<Edit>,
) -> Result<(String, bool), EngineError> {
    let id_attribute = node.attribute_node("id");
    let supplied = id_attribute.filter(|value| !value.value().is_empty());
    let temporary_id = supplied.is_none();
    let id = if let Some(supplied) = supplied {
        if ids.get(supplied.value()) == Some(&true) {
            return Err(unsupported(
                "ambiguous EPUB SVG mixed-font textLength source ID",
            ));
        }
        supplied.value().to_owned()
    } else {
        let id = loop {
            let candidate = format!("pydoccraft-svg-measured-text-{}", *next_id);
            *next_id += 1;
            if !ids.contains_key(candidate.as_str()) {
                break candidate;
            }
        };
        let position = id_attribute.map_or_else(
            || svg::style_insertion(source.as_bytes(), node),
            |attr| Ok(attr.range()),
        )?;
        edits.push((position, format!(" id=\"{id}\"")));
        id
    };
    Ok((id, temporary_id))
}

fn prepare_owner(
    root: Node<'_, '_>,
    owner: Node<'_, '_>,
    edits: &mut Vec<Edit>,
) -> Result<LengthOwner, EngineError> {
    let adjust = match owner.attribute("lengthAdjust") {
        None | Some("spacing") => usvg::LengthAdjust::Spacing,
        Some("spacingAndGlyphs") => usvg::LengthAdjust::SpacingAndGlyphs,
        Some(_) => {
            return Err(unsupported(
                "EPUB SVG mixed-font textLength has an invalid lengthAdjust",
            ))
        }
    };
    if owner
        .descendants()
        .skip(1)
        .any(|node| node.has_attribute("textLength"))
    {
        return Err(unsupported(
            "EPUB SVG mixed-font overlapping textLength owners",
        ));
    }
    let chunk = nested_chunk(root, owner, adjust)?;
    let target = length_target(owner, edits)?;
    Ok(LengthOwner {
        target,
        adjust,
        chunk,
    })
}

fn length_target(owner: Node<'_, '_>, edits: &mut Vec<Edit>) -> Result<f32, EngineError> {
    let length = owner
        .attribute_node("textLength")
        .ok_or_else(|| invalid("missing SVG textLength"))?;
    let raw = length.value().trim();
    let target = raw
        .strip_suffix("px")
        .unwrap_or(raw)
        .parse::<f32>()
        .ok()
        .filter(|value| value.is_finite() && *value > 0.0)
        .ok_or_else(|| {
            unsupported("EPUB SVG mixed-font textLength requires positive user units or px")
        })?;
    if target > MAX_LENGTH {
        return Err(limited(
            "EPUB SVG mixed-font textLength exceeds its geometry budget",
        ));
    }
    edits.push((length.range(), String::new()));
    Ok(target)
}

fn nested_chunk(
    root: Node<'_, '_>,
    owner: Node<'_, '_>,
    adjust: usvg::LengthAdjust,
) -> Result<Option<PositionedChunk>, EngineError> {
    if owner == root {
        return Ok(None);
    }
    if !owner.has_tag_name((NS, "tspan")) {
        return Err(unsupported(
            "EPUB SVG mixed-font nested textLength requires a tspan owner",
        ));
    }
    let content = owner.range();
    let preserves_space = root.ancestors().chain(root.descendants()).any(|node| {
        node.attribute(("http://www.w3.org/XML/1998/namespace", "space")) == Some("preserve")
    });
    if root.descendants().filter(Node::is_text).all(|text| {
        let range = text.range();
        range.start >= content.start && range.end <= content.end
            || !preserves_space && text.text().is_none_or(collapsible_space)
    }) {
        // With no surrounding text, measuring the containing text is equivalent.
        return Ok(None);
    }
    if adjust != usvg::LengthAdjust::Spacing
        || root.children().any(|child| {
            child.is_text() && child.text().is_some_and(|value| !collapsible_space(value))
        })
    {
        return Err(nested_error());
    }
    // A wrapped owner must cover the entire independently positioned
    // chunk. This lets the probe retain the same anchors and typography.
    let mut position = owner;
    while let Some(wrapper) = position.parent_element().filter(|&node| node != root) {
        if wrapper.has_attribute("textLength")
            || !transparent_wrapper(wrapper)?
            || wrapper
                .children()
                .any(|child| child != position && (child.is_element() || child.is_text()))
        {
            return Err(nested_error());
        }
        position = wrapper;
    }
    let mut selected = None;
    let mut count = 0;
    let mut baseline = root.attribute("y").map_or(Some(0.0), scalar_position);
    for child in root.children().filter(Node::is_element) {
        if !child.has_tag_name((NS, "tspan"))
            || child.attribute("x").and_then(scalar_position).is_none()
            || child
                .descendants()
                .skip(1)
                .filter(Node::is_element)
                .any(|node| node.has_attribute("x") || node.has_attribute("y"))
        {
            return Err(nested_error());
        }
        let y = child
            .attribute("y")
            .map(|raw| scalar_position(raw).ok_or_else(nested_error))
            .transpose()?;
        baseline = y.or(baseline);
        if baseline.is_none() {
            return Err(nested_error());
        }
        if child == position {
            // An isolated probe has no previous chunk to carry its baseline.
            // Recreate that coordinate in the probe and verify native glyphs.
            selected = Some((count, if y.is_none() { baseline } else { None }));
        }
        count += 1;
    }
    let (index, probe_y) = selected.ok_or_else(nested_error)?;
    Ok(Some(PositionedChunk {
        index,
        count,
        probe_y,
    }))
}

fn transparent_wrapper(node: Node<'_, '_>) -> Result<bool, EngineError> {
    const NEUTRAL_EFFECTS: [(&str, &str); 7] = [
        ("filter", "none"),
        ("clip-path", "none"),
        ("mask", "none"),
        ("transform", "none"),
        ("opacity", "1"),
        ("mix-blend-mode", "normal"),
        ("isolation", "auto"),
    ];
    if !node.has_tag_name((NS, "tspan")) && !node.has_tag_name((NS, "a")) {
        return Ok(false);
    }
    let style = node
        .attribute("style")
        .map(svg_declarations)
        .transpose()?
        .unwrap_or_default();
    Ok(NEUTRAL_EFFECTS.iter().all(|&(name, neutral)| {
        style
            .get(name)
            .map(String::as_str)
            .or_else(|| node.attribute(name))
            .is_none_or(|value| value == neutral)
    }))
}

fn collapsible_space(value: &str) -> bool {
    value.trim_matches([' ', '\t', '\r', '\n']).is_empty()
}

fn scalar_position(raw: &str) -> Option<f32> {
    let value = raw.trim();
    value
        .strip_suffix("px")
        .unwrap_or(value)
        .parse::<f32>()
        .ok()
        .filter(|value| value.is_finite() && value.abs() <= MAX_LENGTH)
}

fn nested_error() -> EngineError {
    unsupported(
        "EPUB SVG mixed-font nested textLength requires an independently positioned spacing chunk",
    )
}

pub(in crate::epub::svg) fn fit_lengths(
    source: &str,
    tree: &usvg::Tree,
    lengths: &[TextLength],
    measure: impl FnOnce(&[u8]) -> Result<usvg::Tree, EngineError>,
) -> Result<FittedLengths, EngineError> {
    let xml = parse_xml(source.as_bytes())?;
    let mut remaining: HashMap<_, _> = lengths
        .iter()
        .map(|length| (length.id.as_str(), length))
        .collect();
    // Native use expansion drops IDs, and each instance can have a different
    // resolved origin or font scale. Measure only directly drawn text here.
    let uses = svg::use_refs::used_sources(xml.root_element());
    let (probe, probe_ids) = probe_chunks(source, xml.root_element(), &remaining, measure)?;
    let probes: HashMap<_, _> = probe
        .as_ref()
        .into_iter()
        .flat_map(direct_texts)
        .map(|text| (text.id(), text))
        .collect();
    let mut measured = HashMap::new();
    for (index, text) in direct_texts(tree).into_iter().enumerate() {
        if remaining.contains_key(text.id()) && measured.insert(text.id(), (index, text)).is_some()
        {
            return Err(unsupported(
                "ambiguous EPUB SVG mixed-font textLength source",
            ));
        }
    }
    let mut edits = Vec::new();
    let mut spacing = Vec::new();
    let mut bytes = 0;
    for node in xml
        .descendants()
        .filter(|node| node.has_tag_name((NS, "text")))
    {
        let Some(id) = node.attribute("id") else {
            continue;
        };
        let Some(length) = remaining.remove(id) else {
            continue;
        };
        if svg::use_refs::scope_contains(&uses, node) {
            return Err(unsupported(
                "EPUB SVG mixed-font textLength inside a use instance",
            ));
        }
        let &(index, text) = measured.get(id).ok_or_else(|| {
            unsupported("EPUB SVG mixed-font textLength source has no native layout")
        })?;
        if let Some(nested) = &length.nested {
            let child_probes = (0..nested.count())
                .map(|index| {
                    probe_ids
                        .get(&(id, index))
                        .and_then(|id| probes.get(id.as_str()).copied())
                        .ok_or_else(nested::error)
                })
                .collect::<Result<Vec<_>, _>>()?;
            let (check, fitted) = nested.fit(node, index, text, &child_probes, source)?;
            for edit in fitted {
                charge(
                    &mut bytes,
                    edit.1.len(),
                    XML_BYTES,
                    "EPUB SVG textLength edits exceed 8 MiB",
                )?;
                edits.push(edit);
            }
            spacing.push(check);
        }
        let leading_dx = length
            .relative
            .as_ref()
            .map(|positions| {
                let reference = probe_ids
                    .get(&(id, 0))
                    .and_then(|id| probes.get(id.as_str()).copied())
                    .ok_or_else(relative::error)?;
                positions.verify(node, text, reference)
            })
            .transpose()?;
        for adjustment in &length.owners {
            let (chunk, start, scope) = if let Some(origin) = adjustment.chunk {
                if text.chunks().len() != origin.count {
                    return Err(nested_error());
                }
                let positioned = positioned_chunk_node(node, origin)?;
                let probe = probe_ids
                    .get(&(id, origin.index))
                    .and_then(|id| probes.get(id.as_str()).copied())
                    .ok_or_else(nested_error)?;
                let [chunk] = probe.chunks() else {
                    return Err(nested_error());
                };
                let start = spacing::verify_probe(text, origin.index, probe, origin.probe_y)?;
                (chunk, Some((start, probe)), positioned)
            } else {
                let [chunk] = text.chunks() else {
                    return Err(unsupported(
                        "EPUB SVG mixed-font textLength requires one positioned native chunk",
                    ));
                };
                (chunk, None, node)
            };
            if text.writing_mode() != usvg::WritingMode::LeftToRight
                || !matches!(chunk.text_flow(), usvg::TextFlow::Linear)
                || chunk.spans().iter().any(|span| {
                    let decoration = span.decoration();
                    span.letter_spacing() < 0.0
                        || span.word_spacing() < 0.0
                        || !solid_fill(span.fill(), span.stroke())
                        || [
                            decoration.underline(),
                            decoration.overline(),
                            decoration.line_through(),
                        ]
                        .into_iter()
                        .flatten()
                        .any(|style| !solid_fill(style.fill(), style.stroke()))
                })
            {
                return Err(unsupported(
                    "EPUB SVG mixed-font textLength requires horizontal solid-fill text with nonnegative spacing",
                ));
            }
            match adjustment.adjust {
                usvg::LengthAdjust::Spacing => {
                    let (offset, measured) = start.unwrap_or((0, text));
                    let mut check = SpacingCheck::new(
                        index,
                        adjustment.target,
                        measured,
                        chunk,
                        offset,
                        start.is_none(),
                    )?;
                    if leading_dx.is_some() {
                        check.require_order();
                    }
                    check.apply(source, scope, &mut edits, &mut bytes)?;
                    spacing.push(check);
                }
                usvg::LengthAdjust::SpacingAndGlyphs => {
                    let anchor = chunk.x().unwrap_or(0.0) + leading_dx.unwrap_or(0.0);
                    let (edit, absolute) =
                        scale_glyphs(source, node, text, anchor, adjustment.target)?;
                    if leading_dx.is_some() {
                        spacing.push(SpacingCheck::scaled(index, text, absolute));
                    }
                    charge(
                        &mut bytes,
                        edit.1.len(),
                        XML_BYTES,
                        "EPUB SVG textLength edits exceed 8 MiB",
                    )?;
                    edits.push(edit);
                }
            }
        }
        if length.temporary_id {
            let id = node
                .attribute_node("id")
                .ok_or_else(|| invalid("missing SVG textLength measurement ID"))?;
            edits.push((id.range(), String::new()));
        }
    }
    if !remaining.is_empty() {
        return Err(invalid("missing EPUB SVG mixed-font textLength source"));
    }
    Ok(FittedLengths {
        source: apply_edits(source, edits, XML_BYTES)?,
        spacing,
    })
}

fn positioned_chunk_node<'a, 'input>(
    text: Node<'a, 'input>,
    origin: PositionedChunk,
) -> Result<Node<'a, 'input>, EngineError> {
    let mut children = text.children().filter(Node::is_element);
    if children.clone().count() != origin.count {
        return Err(nested_error());
    }
    children
        .nth(origin.index)
        .filter(|child| child.has_tag_name((NS, "tspan")))
        .ok_or_else(nested_error)
}

type ProbeIds<'a> = HashMap<(&'a str, usize), String>;

fn probe_chunks<'a>(
    source: &str,
    root: Node<'_, '_>,
    lengths: &HashMap<&'a str, &TextLength>,
    measure: impl FnOnce(&[u8]) -> Result<usvg::Tree, EngineError>,
) -> Result<(Option<usvg::Tree>, ProbeIds<'a>), EngineError> {
    if !lengths.values().any(|length| {
        length.nested.is_some()
            || length.relative.is_some()
            || length.owners.iter().any(|owner| owner.chunk.is_some())
    }) {
        return Ok((None, HashMap::new()));
    }
    let ids: HashSet<_> = root
        .descendants()
        .filter_map(|node| node.attribute("id"))
        .collect();
    let mut next_id = 0;
    let mut probe_ids = HashMap::new();
    let mut edits = Vec::new();
    let mut bytes = 0;
    for text in root
        .descendants()
        .filter(|node| node.has_tag_name((NS, "text")))
    {
        let Some(attr) = text.attribute_node("id") else {
            continue;
        };
        let Some((&id, length)) = lengths.get_key_value(attr.value()) else {
            continue;
        };
        let range = text.range();
        let local = |range: std::ops::Range<usize>| {
            range.start - text.range().start..range.end - text.range().start
        };
        let mut replacement = String::new();
        let mut selected = Vec::new();
        if let Some(nested) = &length.nested {
            selected.extend(
                nested
                    .owners(text)?
                    .into_iter()
                    .enumerate()
                    .map(|(index, owner)| (index, owner, None)),
            );
        } else if length.relative.is_some() {
            selected.push((0, text, None));
        } else {
            for origin in length.owners.iter().filter_map(|owner| owner.chunk) {
                selected.push((
                    origin.index,
                    positioned_chunk_node(text, origin)?,
                    origin.probe_y,
                ));
            }
        }
        for (index, owner, probe_y) in selected {
            let probe_id = loop {
                let id = format!("pydoccraft-svg-spacing-probe-{next_id}");
                next_id += 1;
                if !ids.contains(id.as_str()) {
                    break id;
                }
            };
            let mut copy_edits: Vec<Edit> = if owner == text {
                ["dx", "dy"]
                    .into_iter()
                    .filter_map(|name| text.attribute_node(name))
                    .map(|attr| (local(attr.range()), String::new()))
                    .collect()
            } else {
                let mut outside = Vec::new();
                for retained in owner.ancestors().take_while(|node| *node != text) {
                    let parent = retained.parent_element().ok_or_else(nested_error)?;
                    outside.extend(
                        parent
                            .children()
                            .filter(|sibling| *sibling != retained)
                            .map(|sibling| (local(sibling.range()), String::new())),
                    );
                }
                outside
            };
            if let Some(y) = probe_y {
                copy_edits.push((
                    local(svg::style_insertion(source.as_bytes(), owner)?),
                    format!(" y=\"{y}\""),
                ));
            }
            copy_edits.push((local(attr.range_value()), probe_id.clone()));
            let copy = apply_edits(&source[range.clone()], copy_edits, XML_BYTES)?;
            charge(
                &mut bytes,
                copy.len(),
                XML_BYTES,
                "EPUB SVG textLength probes exceed 8 MiB",
            )?;
            append(&mut replacement, &copy, XML_BYTES)?;
            probe_ids.insert((id, index), probe_id);
        }
        if !replacement.is_empty() {
            edits.push((range, replacement));
        }
    }
    let source = apply_edits(source, edits, XML_BYTES)?;
    Ok((Some(measure(source.as_bytes())?), probe_ids))
}

fn direct_texts(tree: &usvg::Tree) -> Vec<&usvg::Text> {
    let mut texts = Vec::new();
    let mut stack = vec![tree.root().children().iter()];
    while let Some(children) = stack.last_mut() {
        match children.next() {
            Some(usvg::Node::Group(group)) => stack.push(group.children().iter()),
            Some(usvg::Node::Text(text)) => texts.push(text.as_ref()),
            Some(_) => {}
            None => {
                stack.pop();
            }
        }
    }
    texts
}

fn scale_glyphs(
    source: &str,
    node: Node<'_, '_>,
    text: &usvg::Text,
    x: f32,
    target: f32,
) -> Result<(Edit, usvg::Transform), EngineError> {
    // The native text box uses glyph advances rather than ink bounds.
    // Scale the inline positions and outlines around the author's anchor.
    let width = text.bounding_box().width();
    let scale = target / width;
    if !scale.is_finite()
        || !(0.001..=1000.0).contains(&scale)
        || !x.is_finite()
        || x.abs() > MAX_LENGTH
    {
        return Err(limited(
            "EPUB SVG mixed-font textLength exceeds its scale budget",
        ));
    }
    let dx = x - x * scale;
    if !dx.is_finite() {
        return Err(limited(
            "EPUB SVG mixed-font textLength exceeds its scale budget",
        ));
    }
    let adjustment = format!("matrix({scale} 0 0 1 {dx} 0)");
    let style = node.attribute_node("style");
    let mut declarations = style
        .map(|style| svg_declarations(style.value()))
        .transpose()?
        .unwrap_or_default();
    let edit = if let Some(transform) = declarations.get_mut("transform") {
        if transform.eq_ignore_ascii_case("none") {
            *transform = adjustment;
        } else {
            transform.push(' ');
            transform.push_str(&adjustment);
        }
        let mut declarations = declarations.into_iter().collect::<Vec<_>>();
        declarations.sort_unstable_by(|(key, _), (other, _)| key.cmp(other));
        let mut merged = String::new();
        for (key, value) in declarations {
            append(&mut merged, &format!("{key}:{value};"), XML_BYTES)?;
        }
        let style = style.ok_or_else(|| invalid("missing SVG text style"))?;
        (
            style.range(),
            format!(" style=\"{}\"", escape_attribute(&merged)),
        )
    } else if let Some(transform) = node.attribute_node("transform") {
        let prior = transform.value().trim();
        let combined = if prior.eq_ignore_ascii_case("none") || prior.is_empty() {
            adjustment
        } else {
            format!("{prior} {adjustment}")
        };
        (transform.range_value(), escape_attribute(&combined))
    } else {
        (
            svg::style_insertion(source.as_bytes(), node)?,
            format!(" transform=\"{adjustment}\""),
        )
    };
    let absolute = text
        .abs_transform()
        .pre_concat(usvg::Transform::from_row(scale, 0.0, 0.0, 1.0, dx, 0.0));
    Ok((edit, absolute))
}

impl FittedLengths {
    pub(in crate::epub::svg) fn verify_spacing(
        &self,
        original: &usvg::Tree,
        fitted: &usvg::Tree,
    ) -> Result<(), EngineError> {
        spacing::verify(&self.spacing, original, fitted)
    }
}

fn solid_fill(fill: Option<&usvg::Fill>, stroke: Option<&usvg::Stroke>) -> bool {
    stroke.is_none() && fill.is_none_or(|fill| matches!(fill.paint(), usvg::Paint::Color(_)))
}
