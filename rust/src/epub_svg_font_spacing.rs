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
    shift: Shift,
    absolute: Option<usvg::Transform>,
    preserve_order: bool,
}

enum Shift {
    Even { gap: f32, anchor: f32 },
    Explicit(Vec<f32>),
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
            shift: Shift::Even { gap, anchor },
            absolute: None,
            preserve_order: false,
        })
    }

    pub(super) fn from_offsets(index: usize, target: f32, offsets: Vec<f32>) -> Self {
        Self {
            index,
            width: Some(target),
            cluster_range: 0..offsets.len(),
            shift: Shift::Explicit(offsets),
            absolute: None,
            preserve_order: false,
        }
    }

    pub(super) fn scaled(index: usize, text: &usvg::Text, absolute: usvg::Transform) -> Self {
        let count = text
            .layouted()
            .iter()
            .flat_map(|span| svg::glyph_clusters(span, true))
            .count();
        Self {
            index,
            width: Some(text.bounding_box().width()),
            cluster_range: 0..count,
            shift: Shift::Even {
                gap: 0.0,
                anchor: 0.0,
            },
            absolute: Some(absolute),
            preserve_order: true,
        }
    }

    pub(super) fn require_order(&mut self) {
        self.preserve_order = true;
    }

    fn offset(&self, cluster: usize) -> f32 {
        let index = cluster - self.cluster_range.start;
        match &self.shift {
            Shift::Even { gap, anchor } => *gap * (index as f32 - *anchor),
            Shift::Explicit(offsets) => offsets[index],
        }
    }

    pub(super) fn apply(
        &self,
        source: &str,
        node: Node<'_, '_>,
        edits: &mut Vec<Edit>,
        bytes: &mut usize,
    ) -> Result<(), EngineError> {
        let Shift::Even { gap, .. } = self.shift else {
            return Err(spacing_error());
        };
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
    probe_y: Option<f32>,
) -> Result<usize, EngineError> {
    let [target] = probe.chunks() else {
        return Err(spacing_error());
    };
    let chunk = &original.chunks()[index];
    if !svg::logical_glyph_clusters(original)
        || !same_chunk_at(chunk, target, chunk.y().or(probe_y))
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

pub(super) fn same_chunk(before: &usvg::TextChunk, after: &usvg::TextChunk) -> bool {
    same_chunk_at(before, after, before.y())
}

fn same_chunk_at(before: &usvg::TextChunk, after: &usvg::TextChunk, y: Option<f32>) -> bool {
    before.x() == after.x()
        && y == after.y()
        && before.anchor() == after.anchor()
        && before.text() == after.text()
}

pub(super) fn same_spans(before: &usvg::Text, after: &usvg::Text) -> bool {
    let before = before.layouted();
    let after = after.layouted();
    before.len() == after.len()
        && before.iter().zip(after).all(|(old, new)| {
            old.variations == new.variations
                && old.font_optical_sizing == new.font_optical_sizing
                && old.visible == new.visible
                && old.positioned_glyphs.len() == new.positioned_glyphs.len()
        })
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
            || checks[0].absolute.unwrap_or_else(|| before.abs_transform()) != after.abs_transform()
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
        if !same_spans(before, after) {
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
        let preserve_order = checks.iter().any(|check| check.preserve_order);
        let mut previous_x = f32::NEG_INFINITY;
        let mut checks = checks.iter().peekable();
        for (index, cluster) in originals.enumerate() {
            let adjusted = revised.next().ok_or_else(spacing_error)?;
            let x = adjusted.first().ok_or_else(spacing_error)?.transform().tx;
            if preserve_order && x <= previous_x {
                return Err(spacing_error());
            }
            previous_x = x;

            while checks
                .peek()
                .is_some_and(|check| check.cluster_range.end <= index)
            {
                checks.next();
            }
            let shift = checks
                .peek()
                .filter(|check| check.cluster_range.contains(&index))
                .map_or(0.0, |check| check.offset(index));
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

pub(super) fn close(actual: f32, expected: f32) -> bool {
    (actual - expected).abs() <= 0.02
}

fn spacing_error() -> EngineError {
    unsupported(
        "EPUB SVG mixed-font textLength could not preserve native spacing and glyph geometry",
    )
}
