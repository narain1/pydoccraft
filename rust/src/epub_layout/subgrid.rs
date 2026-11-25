//! Materialize independently sized inherited grid tracks for the native solver.
use std::ops::Range;

use taffy::{
    geometry::Line,
    style::{GridTemplateComponent, LengthPercentage, TrackSizingFunction},
    DetailedGridTracksInfo,
};

use crate::{
    epub::{
        css::grid::{MAX_LINE_NAMES, MAX_TRACKS},
        limited, unsupported,
    },
    EngineError,
};

pub(super) struct Axis {
    pub(super) tracks: Vec<GridTemplateComponent<String>>,
    pub(super) names: Vec<Vec<String>>,
    pub(super) gap: LengthPercentage,
}

pub(super) fn unresolved() -> EngineError {
    unsupported(
        "EPUB CSS subgrid requires independently sized inherited grid tracks and a stretched box",
    )
}

pub(super) fn implicit(tracks: &DetailedGridTracksInfo<String>) -> bool {
    tracks.negative_implicit_tracks != 0 || tracks.positive_implicit_tracks != 0
}

pub(super) struct Projection {
    pub(super) span: Range<usize>,
    pub(super) trim: Line<f32>,
    pub(super) local_gap: Option<f32>,
    pub(super) parent_rtl: bool,
    pub(super) child_rtl: bool,
}

pub(super) fn inherit_axis(
    source: &DetailedGridTracksInfo<String>,
    aliases: &[Vec<String>],
    projection: Projection,
) -> Result<Axis, EngineError> {
    let Projection {
        span,
        trim,
        local_gap,
        parent_rtl,
        child_rtl,
    } = projection;
    let reverse_order = parent_rtl != child_rtl;
    let tracks = source.positions.get(span.clone()).ok_or_else(unresolved)?;
    if tracks.is_empty() || tracks.len() > MAX_TRACKS {
        return Err(limited("EPUB CSS inherited grid track limit"));
    }
    if aliases.len() > tracks.len() + 1
        || source
            .collapsed_tracks
            .iter()
            .any(|run| run.start < span.end && run.end > span.start)
    {
        return Err(unresolved());
    }

    let separation = |pair: &[Line<f32>]| {
        if parent_rtl {
            pair[0].start - pair[1].end
        } else {
            pair[1].start - pair[0].end
        }
    };
    let mut gaps = tracks.windows(2).map(separation);
    let first = gaps.next().unwrap_or(0.0);
    if !first.is_finite()
        || first < -0.001
        || gaps.any(|gap| !gap.is_finite() || (gap - first).abs() > 0.001)
    {
        return Err(unresolved());
    }
    let inherited_gap = first.max(0.0);
    let gap = local_gap.unwrap_or(inherited_gap);
    if !gap.is_finite() || gap < 0.0 {
        return Err(unresolved());
    }
    // A changed gap moves each inner track edge by half the difference while
    // preserving the first and last outer grid lines.
    let half = (gap - inherited_gap) * 0.5;
    let sizes = (0..tracks.len())
        .map(|index| {
            let position = if reverse_order {
                tracks[tracks.len() - 1 - index]
            } else {
                tracks[index]
            };
            let start = if index == 0 { trim.start } else { half };
            let end = if index + 1 == tracks.len() {
                trim.end
            } else {
                half
            };
            let size = position.end - position.start - start - end;
            if !size.is_finite() || size < 0.0 {
                return Err(unresolved());
            }
            Ok(GridTemplateComponent::Single(TrackSizingFunction::from(
                LengthPercentage::length(size),
            )))
        })
        .collect::<Result<Vec<_>, _>>()?;

    let mut total = 0;
    let names = (0..=tracks.len())
        .map(|index| {
            let line_index = if reverse_order {
                span.end - index
            } else {
                span.start + index
            };
            let mut line = source.names_for_line(line_index).to_vec();
            for alias in aliases.get(index).into_iter().flatten() {
                if !line.contains(alias) {
                    line.push(alias.clone());
                }
            }
            total += line.len();
            if total > MAX_LINE_NAMES {
                return Err(limited("EPUB CSS inherited grid line name limit"));
            }
            Ok(line)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Axis {
        tracks: sizes,
        names,
        gap: LengthPercentage::length(gap),
    })
}
