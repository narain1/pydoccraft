//! Fit explicit inline lengths after native mixed-face shaping.
use std::collections::HashMap;

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

#[path = "epub_svg_font_spacing.rs"]
mod spacing;
use spacing::SpacingCheck;

const MAX_LENGTH: f32 = 25_000.0;
type Edit = (std::ops::Range<usize>, String);

pub(in crate::epub::svg) struct TextLength {
    id: String,
    target: f32,
    adjust: usvg::LengthAdjust,
    temporary_id: bool,
}

pub(in crate::epub::svg) struct FittedLengths {
    pub(in crate::epub::svg) source: String,
    spacing: Vec<SpacingCheck>,
}

pub(super) fn prepare_length(
    owner: Node<'_, '_>,
    ids: &HashMap<&str, bool>,
    next_id: &mut usize,
    source: &str,
    edits: &mut Vec<Edit>,
) -> Result<TextLength, EngineError> {
    let adjust = match owner.attribute("lengthAdjust") {
        None | Some("spacing") => usvg::LengthAdjust::Spacing,
        Some("spacingAndGlyphs") => usvg::LengthAdjust::SpacingAndGlyphs,
        Some(_) => {
            return Err(unsupported(
                "EPUB SVG mixed-font textLength has an invalid lengthAdjust",
            ))
        }
    };
    if owner.descendants().filter(Node::is_element).any(|node| {
        ["dx", "dy", "rotate"]
            .iter()
            .any(|attribute| node.has_attribute(*attribute))
            || node != owner
                && (node.has_attribute("textLength") || node.has_tag_name((NS, "text")))
    }) {
        return Err(unsupported(
            "EPUB SVG mixed-font textLength requires one linear run without per-glyph positioning or nested lengths",
        ));
    }
    for effect in ["filter", "clip-path", "mask"] {
        if owner.attribute(effect).is_some_and(|value| value != "none") {
            return Err(unsupported(
                "EPUB SVG mixed-font textLength with an effect on the text element",
            ));
        }
    }
    let style = owner
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
    let id_attribute = owner.attribute_node("id");
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
            || svg::style_insertion(source.as_bytes(), owner),
            |attr| Ok(attr.range()),
        )?;
        edits.push((position, format!(" id=\"{id}\"")));
        id
    };
    edits.push((length.range(), String::new()));
    Ok(TextLength {
        id,
        target,
        adjust,
        temporary_id,
    })
}

pub(in crate::epub::svg) fn fit_lengths(
    source: &str,
    tree: &usvg::Tree,
    lengths: &[TextLength],
) -> Result<FittedLengths, EngineError> {
    let xml = parse_xml(source.as_bytes())?;
    let mut remaining: HashMap<_, _> = lengths
        .iter()
        .map(|length| (length.id.as_str(), length))
        .collect();
    // Native use expansion drops IDs, and each instance can have a different
    // resolved origin or font scale. Measure only directly drawn text here.
    let uses = svg::use_refs::used_sources(xml.root_element());
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
        let [chunk] = text.chunks() else {
            return Err(unsupported(
                "EPUB SVG mixed-font textLength requires one positioned native chunk",
            ));
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
        match length.adjust {
            usvg::LengthAdjust::Spacing => {
                let check = SpacingCheck::new(index, length.target, text, chunk)?;
                check.apply(source, node, &mut edits, &mut bytes)?;
                spacing.push(check);
            }
            usvg::LengthAdjust::SpacingAndGlyphs => {
                let edit = scale_glyphs(source, node, text, chunk, length.target)?;
                charge(
                    &mut bytes,
                    edit.1.len(),
                    XML_BYTES,
                    "EPUB SVG textLength edits exceed 8 MiB",
                )?;
                edits.push(edit);
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
    chunk: &usvg::TextChunk,
    target: f32,
) -> Result<Edit, EngineError> {
    // The native text box uses glyph advances rather than ink bounds.
    // Scale the inline positions and outlines around the author's anchor.
    let width = text.bounding_box().width();
    let scale = target / width;
    let x = chunk.x().unwrap_or(0.0);
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
    Ok(edit)
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
