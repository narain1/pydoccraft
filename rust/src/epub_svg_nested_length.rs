//! Fit nested spacing owners without splitting the original shaping chunk.
use std::{collections::HashMap, fmt::Write, ops::Range};

use resvg::usvg;
use roxmltree::Node;

use super::{length_target, measurement_id, solid_fill, Edit, SpacingCheck, MAX_LENGTH};
use crate::{
    epub::{
        css::svg_declarations,
        invalid, limited,
        svg::{self, NS},
        unsupported,
    },
    EngineError,
};

const MAX_POSITIONS: usize = 4096;
const MAX_OWNERS: usize = 8;
const TOLERANCE: f32 = 0.02;

pub(super) struct NestedLength {
    outer: f32,
    children: Vec<Owner>,
}

struct Owner {
    target: f32,
    id: String,
    temporary_id: bool,
    parent: Option<usize>,
}

struct Measurement {
    range: Range<usize>,
    width: f32,
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
    let nested: Vec<_> = text
        .descendants()
        .skip(1)
        .filter(|node| node.has_attribute("textLength"))
        .take(MAX_OWNERS + 1)
        .collect();
    if nested.is_empty() {
        return Ok(None);
    }
    if nested.len() > MAX_OWNERS {
        return Err(limited(
            "EPUB SVG text has more than eight overlapping textLength owners below one parent",
        ));
    }
    if nested
        .iter()
        .any(|owner| !owner.has_tag_name((NS, "tspan")))
        || std::iter::once(text)
            .chain(nested.iter().copied())
            .any(|owner| !matches!(owner.attribute("lengthAdjust"), None | Some("spacing")))
        || text
            .descendants()
            .skip(1)
            .any(|node| node.has_attribute("x") || node.has_attribute("y"))
    {
        return Err(error());
    }
    let mut children = Vec::with_capacity(nested.len());
    for (index, &owner) in nested.iter().enumerate() {
        let enclosing = enclosing_owner(owner, text)?;
        let parent = if enclosing == text {
            None
        } else {
            // The parent owner must already be in the depth-first order.
            // The shared owner limit bounds both branching and depth.
            Some(
                nested[..index]
                    .iter()
                    .position(|&node| node == enclosing)
                    .ok_or_else(error)?,
            )
        };
        let (id, temporary_id) = measurement_id(owner, ids, next_id, source, edits)?;
        children.push(Owner {
            target: length_target(owner, edits)?,
            id,
            temporary_id,
            parent,
        });
    }
    Ok(Some(NestedLength {
        outer: length_target(text, edits)?,
        children,
    }))
}

fn enclosing_owner<'a, 'input>(
    owner: Node<'a, 'input>,
    text: Node<'a, 'input>,
) -> Result<Node<'a, 'input>, EngineError> {
    const NEUTRAL_EFFECTS: [(&str, &str); 7] = [
        ("filter", "none"),
        ("clip-path", "none"),
        ("mask", "none"),
        ("transform", "none"),
        ("opacity", "1"),
        ("mix-blend-mode", "normal"),
        ("isolation", "auto"),
    ];
    for parent in owner.ancestors().skip(1) {
        if parent == text || parent.has_attribute("textLength") {
            return Ok(parent);
        }
        if !parent.has_tag_name((NS, "tspan")) && !parent.has_tag_name((NS, "a")) {
            return Err(error());
        }
        let style = parent
            .attribute("style")
            .map(svg_declarations)
            .transpose()?
            .unwrap_or_default();
        if NEUTRAL_EFFECTS.iter().any(|&(name, neutral)| {
            style
                .get(name)
                .map(String::as_str)
                .or_else(|| parent.attribute(name))
                .is_some_and(|value| value != neutral)
        }) {
            return Err(error());
        }
    }
    Err(error())
}

impl NestedLength {
    pub(super) fn count(&self) -> usize {
        self.children.len()
    }

    pub(super) fn owners<'a, 'input>(
        &self,
        text: Node<'a, 'input>,
    ) -> Result<Vec<Node<'a, 'input>>, EngineError> {
        let mut nodes = Vec::with_capacity(self.children.len());
        for owner in &self.children {
            let parent = owner.parent.map_or(text, |index| nodes[index]);
            let node = parent
                .descendants()
                .skip(1)
                .find(|child| child.attribute("id") == Some(owner.id.as_str()))
                .ok_or_else(error)?;
            nodes.push(node);
        }
        Ok(nodes)
    }

    pub(super) fn fit(
        &self,
        node: Node<'_, '_>,
        index: usize,
        text: &usvg::Text,
        probes: &[&usvg::Text],
        source: &str,
    ) -> Result<(SpacingCheck, Vec<Edit>), EngineError> {
        let chunk = measurable_chunk(text)?;
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
        let owners = self.owners(node)?;
        if owners.len() != probes.len() {
            return Err(error());
        }
        let root = owners.len();
        let whole = 0..clusters.len();
        let mut measured = Vec::<Measurement>::with_capacity(root);
        let mut sibling_ends = vec![0; root + 1];
        for ((owner, inner), &probe) in owners.iter().copied().zip(&self.children).zip(probes) {
            let bytes = source_range(node, owner, chunk.text())?;
            let range = verify_inner(&clusters, bytes, probe)?;
            let parent = inner.parent.unwrap_or(root);
            let enclosing = inner.parent.map_or(&whole, |index| &measured[index].range);
            if range.start < enclosing.start
                || range.end > enclosing.end
                || range.start < sibling_ends[parent]
            {
                return Err(error());
            }
            sibling_ends[parent] = range.end;
            measured.push(Measurement {
                range,
                width: probe.bounding_box().width(),
            });
        }
        let native_width = text.bounding_box().width();
        if native_width > MAX_LENGTH {
            return Err(error());
        }

        let children = |parent| {
            self.children
                .iter()
                .zip(&measured)
                .filter_map(move |(owner, measurement)| {
                    (owner.parent.unwrap_or(root) == parent).then_some(&measurement.range)
                })
        };
        let mut inside = vec![0.0; root + 1];
        let mut offsets = vec![0.0; whole.len()];
        // Each measured native width excludes authored length adjustments.
        // Resolve descendants first, then count each fitted child as one
        // spacing unit and carry only its measured growth into its parent.
        for (index, owner) in self.children.iter().enumerate().rev() {
            let extra = growth(owner.target - measured[index].width - inside[index])?;
            expand(
                measured[index].range.clone(),
                children(index),
                extra,
                &mut offsets,
            )?;
            let growth = inside[index] + extra;
            inside[owner.parent.unwrap_or(root)] += growth;
        }
        let outside = growth(self.outer - native_width - inside[root])?;
        expand(whole, children(root), outside, &mut offsets)?;
        let total = inside[root] + outside;
        let mut dx = String::from(" dx=\"");
        let mut previous = 0.0;
        let anchor_shift = match chunk.anchor() {
            usvg::TextAnchor::Start => 0.0,
            usvg::TextAnchor::Middle => -total / 2.0,
            usvg::TextAnchor::End => -total,
        };
        for (index, (shift, cluster)) in offsets.iter_mut().zip(clusters).enumerate() {
            if index != 0 {
                dx.push(' ');
            }
            write!(dx, "{}", *shift - previous).map_err(invalid)?;
            let label = &cluster.last().ok_or_else(error)?.text;
            for _ in 1..label.chars().count() {
                dx.push_str(" 0");
            }
            previous = *shift;
            *shift += anchor_shift;
        }
        dx.push('"');
        let mut edits = vec![(svg::style_insertion(source.as_bytes(), node)?, dx)];
        for (owner, inner) in owners.into_iter().zip(&self.children) {
            if inner.temporary_id {
                let id = owner.attribute_node("id").ok_or_else(error)?;
                edits.push((id.range(), String::new()));
            }
        }
        Ok((
            SpacingCheck::from_offsets(index, self.outer, offsets),
            edits,
        ))
    }
}

fn expand<'a>(
    range: Range<usize>,
    children: impl Iterator<Item = &'a Range<usize>> + Clone,
    extra: f32,
    offsets: &mut [f32],
) -> Result<(), EngineError> {
    let collapsed = children.clone().map(|child| child.len() - 1).sum::<usize>();
    let units = range
        .len()
        .checked_sub(collapsed)
        .filter(|&units| units >= 2)
        .ok_or_else(error)?;
    let gap = extra / (units - 1) as f32;
    let mut child = children.peekable();
    let mut unit = 0;
    for (index, offset) in range.clone().zip(&mut offsets[range.clone()]) {
        *offset += gap * unit as f32;
        if let Some(inside) = child.peek().filter(|inside| inside.contains(&index)) {
            if index + 1 == inside.end {
                unit += 1;
                child.next();
            }
        } else {
            unit += 1;
        }
    }
    for offset in &mut offsets[range.end..] {
        *offset += extra;
    }
    Ok(())
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
        "EPUB SVG mixed-font overlapping textLength owners require bounded nested spacing expansions in an unkerned horizontal chunk",
    )
}
