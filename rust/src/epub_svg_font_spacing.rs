//! Verify native spacing adjustments between identifiable mixed-face glyphs.
use resvg::usvg;
use roxmltree::Node;

use super::{direct_texts, Edit, MAX_LENGTH};
use crate::{
    author_epub::escape_attribute,
    epub::{charge, limited, svg, unsupported, XML_BYTES},
    EngineError,
};

pub(super) struct SpacingCheck {
    index: usize,
    target: f32,
    gap: f32,
    anchor: f32,
}

impl SpacingCheck {
    pub(super) fn new(
        index: usize,
        target: f32,
        text: &usvg::Text,
        chunk: &usvg::TextChunk,
    ) -> Result<Self, EngineError> {
        if chunk
            .spans()
            .iter()
            .any(|span| span.letter_spacing() != 0.0 || span.word_spacing() != 0.0)
        {
            return Err(unsupported(
                "EPUB SVG mixed-font spacing textLength with authored letter or word spacing",
            ));
        }
        // A nonempty mapping for every glyph and a complete logical sequence
        // make each native cluster boundary unambiguous through the public API.
        let mut remaining = chunk.text();
        let mut count = 0;
        for glyph in text
            .layouted()
            .iter()
            .flat_map(|span| &span.positioned_glyphs)
        {
            if glyph.text.is_empty() {
                return Err(spacing_error());
            }
            remaining = remaining
                .strip_prefix(&glyph.text)
                .ok_or_else(spacing_error)?;
            count += 1;
        }
        if !remaining.is_empty() || count < 2 {
            return Err(spacing_error());
        }
        let gaps = (count - 1) as f32;
        let gap = (target - text.bounding_box().width()) / gaps;
        if !gap.is_finite() || gap.abs() > MAX_LENGTH {
            return Err(limited(
                "EPUB SVG mixed-font textLength exceeds its spacing budget",
            ));
        }
        let anchor = match chunk.anchor() {
            usvg::TextAnchor::Start => 0.0,
            usvg::TextAnchor::Middle => gaps / 2.0,
            usvg::TextAnchor::End => gaps,
        };
        Ok(Self {
            index,
            target,
            gap,
            anchor,
        })
    }

    pub(super) fn apply(
        &self,
        source: &str,
        node: Node<'_, '_>,
        edits: &mut Vec<Edit>,
        bytes: &mut usize,
    ) -> Result<(), EngineError> {
        let gap = self.gap;
        for parent in node
            .descendants()
            .filter(|node| node.is_element() && node.children().any(|child| child.is_text()))
        {
            let (range, style) = if let Some(style) = parent.attribute_node("style") {
                (style.range(), style.value().trim_end_matches(';'))
            } else {
                (svg::style_insertion(source.as_bytes(), parent)?, "")
            };
            let separator = if style.is_empty() { "" } else { ";" };
            let value = format!(
                r#" style="{}{separator}letter-spacing:{gap}px;""#,
                escape_attribute(style),
            );
            charge(
                bytes,
                value.len(),
                XML_BYTES,
                "EPUB SVG textLength edits exceed 8 MiB",
            )?;
            edits.push((range, value));
        }
        Ok(())
    }
}

pub(super) fn verify(
    spacing: &[SpacingCheck],
    original: &usvg::Tree,
    fitted: &usvg::Tree,
) -> Result<(), EngineError> {
    if spacing.is_empty() {
        return Ok(());
    }
    let original = direct_texts(original);
    let fitted = direct_texts(fitted);
    // Edits retain the native text order, even for elements whose adapter
    // measurement IDs have been removed from the final source.
    if original.len() != fitted.len() {
        return Err(spacing_error());
    }
    for check in spacing {
        let before = original[check.index];
        let after = fitted[check.index];
        let [chunk] = after.chunks() else {
            return Err(spacing_error());
        };
        if chunk.text() != before.chunks()[0].text()
            || before.abs_transform() != after.abs_transform()
            || !close(after.bounding_box().width(), check.target)
        {
            return Err(spacing_error());
        }
        let before_spans = before.layouted();
        let after_spans = after.layouted();
        if before_spans.len() != after_spans.len()
            || before_spans.iter().zip(after_spans).any(|(old, new)| {
                old.variations != new.variations
                    || old.font_optical_sizing != new.font_optical_sizing
                    || old.visible != new.visible
                    || old.positioned_glyphs.len() != new.positioned_glyphs.len()
            })
        {
            return Err(spacing_error());
        }
        let mut revised = after
            .layouted()
            .iter()
            .flat_map(|span| &span.positioned_glyphs);
        let originals = before
            .layouted()
            .iter()
            .flat_map(|span| &span.positioned_glyphs);
        for (index, glyph) in originals.enumerate() {
            let adjusted = revised.next().ok_or_else(spacing_error)?;
            let mut old = glyph.transform();
            let new = adjusted.transform();
            let expected = old.tx + check.gap * (index as f32 - check.anchor);
            old.tx = new.tx;
            if glyph.id != adjusted.id
                || glyph.font != adjusted.font
                || glyph.text != adjusted.text
                || old != new
                || !close(new.tx, expected)
            {
                return Err(spacing_error());
            }
        }
        if revised.next().is_some() {
            return Err(spacing_error());
        }
    }
    Ok(())
}

fn close(actual: f32, expected: f32) -> bool {
    (actual - expected).abs() <= 0.02
}

fn spacing_error() -> EngineError {
    unsupported(
        "EPUB SVG mixed-font textLength could not preserve native spacing and glyph geometry",
    )
}
