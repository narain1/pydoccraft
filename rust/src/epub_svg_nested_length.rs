//! Fit two nested spacing owners without splitting the original shaping chunk.
use std::{collections::HashMap, fmt::Write, ops::Range};

use resvg::usvg;
use roxmltree::Node;

use super::{length_target, measurement_id, solid_fill, Edit, SpacingCheck, MAX_LENGTH};
use crate::{
    epub::{
        invalid, limited,
        svg::{self, NS},
        unsupported,
    },
    EngineError,
};

const MAX_POSITIONS: usize = 4096;
const TOLERANCE: f32 = 0.02;

pub(super) struct NestedLength {
    outer: f32,
    inner: f32,
    inner_id: String,
    temporary_id: bool,
}

pub(super) fn prepare(
    text: Node<'_, '_>,
    ids: &HashMap<&str, bool>,
    next_id: &mut usize,
    source: &str,
    edits: &mut Vec<Edit>,
) -> Result<Option<NestedLength>, EngineError> {
    if !text.has_attribute("textLength") {
        return Ok(None);
    }
    let mut nested = text
        .descendants()
        .skip(1)
        .filter(|node| node.has_attribute("textLength"));
    let Some(inner) = nested.next() else {
        return Ok(None);
    };
    if inner.parent_element() != Some(text)
        || !inner.has_tag_name((NS, "tspan"))
        || nested.next().is_some()
        || [text, inner]
            .iter()
            .any(|owner| !matches!(owner.attribute("lengthAdjust"), None | Some("spacing")))
        || text
            .descendants()
            .skip(1)
            .any(|node| node.has_attribute("x") || node.has_attribute("y"))
    {
        return Err(error());
    }
    let (inner_id, temporary_id) = measurement_id(inner, ids, next_id, source, edits)?;
    Ok(Some(NestedLength {
        outer: length_target(text, edits)?,
        inner: length_target(inner, edits)?,
        inner_id,
        temporary_id,
    }))
}

impl NestedLength {
    pub(super) fn owner<'a, 'input>(
        &self,
        text: Node<'a, 'input>,
    ) -> Result<Node<'a, 'input>, EngineError> {
        text.children()
            .find(|child| child.attribute("id") == Some(self.inner_id.as_str()))
            .ok_or_else(error)
    }

    pub(super) fn fit(
        &self,
        node: Node<'_, '_>,
        index: usize,
        text: &usvg::Text,
        probe: &usvg::Text,
        source: &str,
    ) -> Result<(SpacingCheck, Vec<Edit>), EngineError> {
        let chunk = measurable_chunk(text)?;
        let inner = self.owner(node)?;
        let owner_bytes = source_range(node, inner, chunk.text())?;
        let clusters: Vec<_> = text
            .layouted()
            .iter()
            .flat_map(|span| svg::glyph_clusters(span, true))
            .collect();
        if clusters
            .iter()
            .flat_map(|cluster| cluster.iter())
            .any(|glyph| glyph.id.0 == 0)
        {
            return Err(error());
        }
        let range = verify_inner(&clusters, owner_bytes, probe)?;
        let native_width = text.bounding_box().width();
        let inside = growth(self.inner - probe.bounding_box().width())?;
        let outside = growth(self.outer - native_width - inside)?;
        let groups = clusters.len() - range.len() + 1;
        if groups < 2 || native_width > MAX_LENGTH {
            return Err(error());
        }

        // Spacing resolves the inner length first. The outer fit treats
        // the already resolved child as one unit and translates it intact.
        let mut dx = String::from(" dx=\"");
        let mut offsets = Vec::with_capacity(clusters.len());
        let mut previous = 0.0;
        let anchor_shift = match chunk.anchor() {
            usvg::TextAnchor::Start => 0.0,
            usvg::TextAnchor::Middle => -(inside + outside) / 2.0,
            usvg::TextAnchor::End => -(inside + outside),
        };
        for (index, cluster) in clusters.iter().enumerate() {
            let (unit, inside_shift) = if index < range.start {
                (index, 0.0)
            } else if index >= range.end {
                (index - range.len() + 1, inside)
            } else {
                (
                    range.start,
                    inside / (range.len() - 1) as f32 * (index - range.start) as f32,
                )
            };
            let shift = inside_shift + outside / (groups - 1) as f32 * unit as f32;
            if !offsets.is_empty() {
                dx.push(' ');
            }
            write!(dx, "{}", shift - previous).map_err(invalid)?;
            let label = &cluster.last().ok_or_else(error)?.text;
            for _ in 1..label.chars().count() {
                dx.push_str(" 0");
            }
            offsets.push(shift + anchor_shift);
            previous = shift;
        }
        dx.push('"');
        let mut edits = vec![(svg::style_insertion(source.as_bytes(), node)?, dx)];
        if self.temporary_id {
            let id = inner.attribute_node("id").ok_or_else(error)?;
            edits.push((id.range(), String::new()));
        }
        Ok((
            SpacingCheck::from_offsets(index, self.outer, offsets),
            edits,
        ))
    }
}

fn measurable_chunk(text: &usvg::Text) -> Result<&usvg::TextChunk, EngineError> {
    let [chunk] = text.chunks() else {
        return Err(error());
    };
    if chunk.text().chars().take(MAX_POSITIONS + 1).count() > MAX_POSITIONS {
        return Err(limited(
            "EPUB SVG nested textLength exceeds 4096 characters",
        ));
    }
    if text.writing_mode() != usvg::WritingMode::LeftToRight
        || !matches!(chunk.text_flow(), usvg::TextFlow::Linear)
        || chunk.spans().iter().any(|span| {
            let decoration = span.decoration();
            span.apply_kerning()
                || span.letter_spacing() != 0.0
                || span.word_spacing() != 0.0
                || !solid_fill(span.fill(), span.stroke())
                || decoration.underline().is_some()
                || decoration.overline().is_some()
                || decoration.line_through().is_some()
        })
        || !svg::logical_glyph_clusters(text)
    {
        return Err(error());
    }
    Ok(chunk)
}

fn source_range(
    node: Node<'_, '_>,
    owner: Node<'_, '_>,
    native: &str,
) -> Result<Range<usize>, EngineError> {
    let mut rest = native;
    let mut range = None::<Range<usize>>;
    for node in node.descendants().filter(Node::is_text) {
        let text = node.text().ok_or_else(error)?;
        let start = native.len() - rest.len();
        rest = rest.strip_prefix(text).ok_or_else(error)?;
        if owner.range().contains(&node.range().start) {
            let end = start + text.len();
            range.get_or_insert(start..end).end = end;
        }
    }
    if !rest.is_empty() {
        return Err(error());
    }
    range.filter(|range| !range.is_empty()).ok_or_else(error)
}

type Glyph = usvg::layout::PositionedGlyph;

fn verify_inner(
    clusters: &[&[Glyph]],
    bytes: Range<usize>,
    probe: &usvg::Text,
) -> Result<Range<usize>, EngineError> {
    measurable_chunk(probe)?;
    let mut expected = probe
        .layouted()
        .iter()
        .flat_map(|span| svg::glyph_clusters(span, true));
    let mut offset = 0;
    let mut range = None::<Range<usize>>;
    let mut translation = None;
    for (index, cluster) in clusters.iter().enumerate() {
        let start = offset;
        let label = &cluster.last().ok_or_else(error)?.text;
        offset += label.len();
        if offset <= bytes.start || start >= bytes.end {
            continue;
        }
        if start < bytes.start || offset > bytes.end {
            return Err(error());
        }
        let probe = expected.next().ok_or_else(error)?;
        if cluster.len() != probe.len() {
            return Err(error());
        }
        for (a, b) in cluster.iter().zip(probe) {
            let mut actual = a.transform();
            let probe = b.transform();
            let delta = actual.tx - probe.tx;
            actual.tx = probe.tx;
            if a.id != b.id
                || a.font != b.font
                || a.text != b.text
                || actual != probe
                || (delta - *translation.get_or_insert(delta)).abs() > TOLERANCE
            {
                return Err(error());
            }
        }
        range.get_or_insert(index..index + 1).end = index + 1;
    }
    if expected.next().is_some() {
        return Err(error());
    }
    range.filter(|range| range.len() > 1).ok_or_else(error)
}

fn growth(delta: f32) -> Result<f32, EngineError> {
    if !delta.is_finite() || !(-TOLERANCE..=MAX_LENGTH).contains(&delta) {
        return Err(error());
    }
    Ok(delta.max(0.0))
}

pub(super) fn error() -> EngineError {
    unsupported(
        "EPUB SVG mixed-font overlapping textLength owners require one direct nested spacing expansion in an unkerned horizontal chunk",
    )
}
