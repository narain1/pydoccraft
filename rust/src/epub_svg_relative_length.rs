//! Verify authored relative positions before fitting a mixed-face text length.
use resvg::usvg;
use roxmltree::Node;

use super::{spacing, MAX_LENGTH};
use crate::{
    epub::{limited, svg, unsupported},
    EngineError,
};

const MAX_POSITIONS: usize = 4096;

pub(super) struct RelativePositions {
    dx: Vec<f32>,
    dy: Vec<f32>,
}

impl RelativePositions {
    pub(super) fn read(node: Node<'_, '_>) -> Result<Option<Self>, EngineError> {
        let (dx, dy) = (node.attribute("dx"), node.attribute("dy"));
        if dx.is_none() && dy.is_none() {
            return Ok(None);
        }
        Ok(Some(Self {
            dx: position_list(dx)?,
            dy: position_list(dy)?,
        }))
    }

    pub(super) fn verify(
        &self,
        node: Node<'_, '_>,
        positioned: &usvg::Text,
        reference: &usvg::Text,
    ) -> Result<f32, EngineError> {
        let [chunk] = positioned.chunks() else {
            return Err(error());
        };
        let [unpositioned] = reference.chunks() else {
            return Err(error());
        };
        if chunk.text().chars().take(MAX_POSITIONS + 1).count() > MAX_POSITIONS {
            return Err(position_limit());
        }
        let transform = positioned.abs_transform();
        if !spacing::same_chunk(chunk, unpositioned)
            || !spacing::same_spans(positioned, reference)
            || positioned.writing_mode() != usvg::WritingMode::LeftToRight
            || !matches!(chunk.text_flow(), usvg::TextFlow::Linear)
            || transform != reference.abs_transform()
            || transform.sx <= 0.0
            || transform.sy <= 0.0
            || transform.kx != 0.0
            || transform.ky != 0.0
            || !svg::logical_glyph_clusters(positioned)
            || !svg::logical_glyph_clusters(reference)
        {
            return Err(error());
        }
        verify_source(node, chunk.text())?;
        let mut offset = [0.0; 2];
        let mut character = 0;
        let mut groups = Vec::new();
        for part in positioned
            .layouted()
            .iter()
            .flat_map(|span| svg::glyph_clusters(span, true))
        {
            let label = &part.last().ok_or_else(error)?.text;
            let end = character + label.chars().count();
            for (index, values) in [&self.dx, &self.dy].into_iter().enumerate() {
                if (character + 1..end).any(|index| values.get(index).is_some_and(|v| *v != 0.0)) {
                    return Err(unsupported(
                        "EPUB SVG relative position inside a shaped cluster",
                    ));
                }
                offset[index] += values.get(character).copied().unwrap_or(0.0);
                if !offset[index].is_finite() || offset[index].abs() > MAX_LENGTH {
                    return Err(position_limit());
                }
            }
            character = end;
            groups.push((part, offset));
        }
        let leading = groups.first().ok_or_else(error)?.1[0];
        let width_shift = offset[0] - leading;
        let anchor = match chunk.anchor() {
            usvg::TextAnchor::Start => 0.0,
            usvg::TextAnchor::Middle => width_shift / 2.0,
            usvg::TextAnchor::End => width_shift,
        };
        if !spacing::close(
            positioned.bounding_box().width(),
            reference.bounding_box().width() + width_shift,
        ) {
            return Err(error());
        }
        let mut expected = reference
            .layouted()
            .iter()
            .flat_map(|span| svg::glyph_clusters(span, true));
        let mut prior = f32::NEG_INFINITY;
        for (cluster, [x, y]) in groups {
            let before = expected.next().ok_or_else(error)?;
            let first = cluster.first().ok_or_else(error)?.transform().tx;
            if first <= prior || !same_placement(cluster, before, x - anchor, y) {
                return Err(error());
            }
            prior = first;
        }
        if expected.next().is_some() {
            return Err(error());
        }
        Ok(leading)
    }
}

fn same_placement(
    after: &[usvg::layout::PositionedGlyph],
    before: &[usvg::layout::PositionedGlyph],
    dx: f32,
    dy: f32,
) -> bool {
    after.len() == before.len()
        && after.iter().zip(before).all(|(after, before)| {
            let native = after.transform();
            let mut expected = before.transform();
            let x = expected.tx + dx;
            let y = expected.ty + dy;
            expected.tx = native.tx;
            expected.ty = native.ty;
            after.id.0 != 0
                && after.id == before.id
                && after.font == before.font
                && after.text == before.text
                && native == expected
                && spacing::close(native.tx, x)
                && spacing::close(native.ty, y)
        })
}

fn verify_source(node: Node<'_, '_>, native: &str) -> Result<(), EngineError> {
    let mut remaining = native;
    for text in node
        .descendants()
        .filter(Node::is_text)
        .filter_map(|node| node.text())
    {
        remaining = remaining.strip_prefix(text).ok_or_else(error)?;
    }
    if !remaining.is_empty() {
        return Err(error());
    }
    Ok(())
}

fn position_list(raw: Option<&str>) -> Result<Vec<f32>, EngineError> {
    let Some(raw) = raw else {
        return Ok(Vec::new());
    };
    if !raw.is_ascii() {
        return Err(error());
    }
    let mut values = Vec::new();
    for segment in raw.split(',') {
        if segment.trim_ascii().is_empty() {
            return Err(error());
        }
        for token in segment.split_ascii_whitespace() {
            if values.len() >= MAX_POSITIONS {
                return Err(position_limit());
            }
            let value = token
                .strip_suffix("px")
                .unwrap_or(token)
                .parse::<f32>()
                .ok()
                .filter(|value| value.is_finite())
                .ok_or_else(error)?;
            if value.abs() > MAX_LENGTH {
                return Err(position_limit());
            }
            values.push(value);
        }
    }
    Ok(values)
}

fn position_limit() -> EngineError {
    limited("EPUB SVG relative positioning exceeds its character or coordinate budget")
}

pub(super) fn error() -> EngineError {
    unsupported("EPUB SVG mixed-font textLength requires verifiable relative glyph positioning")
}
