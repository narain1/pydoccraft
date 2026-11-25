//! Verify native spacing adjustments between identifiable mixed-face glyphs.
use std::ops::Range;

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
    width: Option<f32>,
    cluster_range: Range<usize>,
    gap: f32,
    anchor: f32,
}

impl SpacingCheck {
    pub(super) fn new(
        index: usize,
        target: f32,
        text: &usvg::Text,
        chunk: &usvg::TextChunk,
        offset: usize,
        verify_full_width: bool,
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
        if !svg::logical_glyph_clusters(text) {
            return Err(spacing_error());
        }
        let count = text
            .layouted()
            .iter()
            .flat_map(|span| svg::glyph_clusters(span, true))
            .count();
        if count < 2 {
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
            width: verify_full_width.then_some(target),
            cluster_range: offset..offset + count,
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

pub(super) fn verify_probe(
    original: &usvg::Text,
    index: usize,
    probe: &usvg::Text,
) -> Result<usize, EngineError> {
    let [target] = probe.chunks() else {
        return Err(spacing_error());
    };
    if !svg::logical_glyph_clusters(original)
        || !same_chunk(&original.chunks()[index], target)
        || original.writing_mode() != probe.writing_mode()
        || original.abs_transform() != probe.abs_transform()
    {
        return Err(spacing_error());
    }
    let first_byte = original.chunks()[..index]
        .iter()
        .map(|chunk| chunk.text().len())
        .sum::<usize>();
    let end_byte = first_byte + target.text().len();
    let mut reference = probe
        .layouted()
        .iter()
        .flat_map(|span| svg::glyph_clusters(span, true));
    let mut byte = 0;
    let mut start = None;
    for (index, cluster) in original
        .layouted()
        .iter()
        .flat_map(|span| svg::glyph_clusters(span, true))
        .enumerate()
    {
        if byte == first_byte {
            start = Some(index);
        }
        if (first_byte..end_byte).contains(&byte) {
            let measured = reference.next().ok_or_else(spacing_error)?;
            if measured.len() != cluster.len()
                || measured.iter().zip(cluster).any(|(expected, actual)| {
                    expected.id != actual.id
                        || expected.font != actual.font
                        || expected.text != actual.text
                        || expected.transform() != actual.transform()
                })
            {
                return Err(spacing_error());
            }
        }
        byte += cluster.last().ok_or_else(spacing_error)?.text.len();
    }
    if reference.next().is_some() {
        return Err(spacing_error());
    }
    start.ok_or_else(spacing_error)
}

fn same_chunk(before: &usvg::TextChunk, after: &usvg::TextChunk) -> bool {
    before.x() == after.x()
        && before.y() == after.y()
        && before.anchor() == after.anchor()
        && before.text() == after.text()
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
    for checks in spacing.chunk_by(|left, right| left.index == right.index) {
        let index = checks[0].index;
        let before = original[index];
        let after = fitted[index];
        if before.chunks().len() != after.chunks().len()
            || before
                .chunks()
                .iter()
                .zip(after.chunks())
                .any(|(old, new)| !same_chunk(old, new))
            || before.abs_transform() != after.abs_transform()
            || checks.iter().any(|check| {
                check
                    .width
                    .is_some_and(|width| !close(after.bounding_box().width(), width))
            })
            || checks
                .windows(2)
                .any(|pair| pair[0].cluster_range.end > pair[1].cluster_range.start)
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
            .flat_map(|span| svg::glyph_clusters(span, true));
        let originals = before
            .layouted()
            .iter()
            .flat_map(|span| svg::glyph_clusters(span, true));
        let mut checks = checks.iter().peekable();
        for (index, cluster) in originals.enumerate() {
            let adjusted = revised.next().ok_or_else(spacing_error)?;
            while checks
                .peek()
                .is_some_and(|check| check.cluster_range.end <= index)
            {
                checks.next();
            }
            let shift = checks
                .peek()
                .filter(|check| check.cluster_range.contains(&index))
                .map_or(0.0, |check| {
                    check.gap * ((index - check.cluster_range.start) as f32 - check.anchor)
                });
            if cluster.len() != adjusted.len()
                || cluster.iter().zip(adjusted).any(|(glyph, adjusted)| {
                    let mut old = glyph.transform();
                    let new = adjusted.transform();
                    let expected = old.tx + shift;
                    old.tx = new.tx;
                    glyph.id != adjusted.id
                        || glyph.font != adjusted.font
                        || glyph.text != adjusted.text
                        || old != new
                        || !close(new.tx, expected)
                })
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
