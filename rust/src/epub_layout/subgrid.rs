//! Materialize independently sized inherited grid tracks for the native solver.
use std::ops::Range;

use taffy::{
    geometry::Line,
    style::{
        GridPlacement, GridTemplateArea, GridTemplateAreas, GridTemplateComponent,
        LengthPercentage, TrackSizingFunction,
    },
    style_helpers::line,
    DetailedGridInfo, DetailedGridItemsInfo, DetailedGridTracksInfo,
};

use crate::{
    epub::{
        css::grid::{NameRepeat, MAX_LINE_NAMES, MAX_TRACKS},
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

pub(super) fn inherit_areas(
    source: &GridTemplateAreas<String>,
    details: &DetailedGridInfo,
    item: &DetailedGridItemsInfo,
    columns: &Axis,
    rows: &Axis,
    reverse_columns: bool,
) -> Result<Option<GridTemplateAreas<String>>, EngineError> {
    let intersect = |area: Range<u16>, span: Range<u16>, leading: u16, reverse: bool| {
        let leading = usize::from(leading);
        let span = usize::from(span.start)..usize::from(span.end);
        let first = (usize::from(area.start) + leading).max(span.start);
        let last = (usize::from(area.end) + leading).min(span.end);
        (first < last).then(|| {
            // The measured item uses full grid lines, including implicit
            // tracks. Return one-based lines within the bounded child span.
            if reverse {
                ((span.end - last + 1) as u16, (span.end - first + 1) as u16)
            } else {
                (
                    (first - span.start + 1) as u16,
                    (last - span.start + 1) as u16,
                )
            }
        })
    };
    let line_names = |axis: &Axis| axis.names.iter().map(Vec::len).sum::<usize>();
    let limit = (MAX_LINE_NAMES - line_names(columns).max(line_names(rows))) / 2;
    let mut areas = Vec::new();
    for area in &source.areas {
        let Some((column_start, column_end)) = intersect(
            area.column_start..area.column_end,
            item.column_start..item.column_end,
            details.columns.negative_implicit_tracks,
            reverse_columns,
        ) else {
            continue;
        };
        let Some((row_start, row_end)) = intersect(
            area.row_start..area.row_end,
            item.row_start..item.row_end,
            details.rows.negative_implicit_tracks,
            false,
        ) else {
            continue;
        };
        if areas.len() == limit {
            return Err(limited("EPUB CSS inherited grid line name limit"));
        }
        areas.push(GridTemplateArea {
            name: area.name.clone(),
            row_start,
            row_end,
            column_start,
            column_end,
        });
    }
    // Keep generated names in native area metadata so that they can be
    // clipped again without moving any authored names on nested subgrids.
    Ok((!areas.is_empty()).then_some(GridTemplateAreas {
        areas,
        row_count: rows.tracks.len() as u16,
        column_count: columns.tracks.len() as u16,
    }))
}

pub(super) fn clamp_numeric_placement(placement: &mut Line<GridPlacement<String>>, tracks: usize) {
    let is_named = |line: &GridPlacement<String>| {
        matches!(
            line,
            GridPlacement::NamedLine(..) | GridPlacement::NamedSpan(..)
        )
    };
    if is_named(&placement.start) || is_named(&placement.end) {
        return;
    }
    let lines = placement.into_origin_zero_ignoring_named(tracks as u16);
    if !lines.is_definite() {
        // Auto placement cannot reserve a span larger than the inherited axis.
        for edge in [&mut placement.start, &mut placement.end] {
            if let GridPlacement::Span(span) = edge {
                *span = (*span).min(tracks as u16);
            }
        }
        return;
    }
    let area = lines.resolve_definite_grid_lines();
    let limit = tracks as i16;
    if area.start.0 < 0 || area.end.0 > limit {
        // A completely outside area occupies the nearest edge track.
        *placement = Line {
            start: line(area.start.0.clamp(0, limit - 1) + 1),
            end: line(area.end.0.clamp(1, limit) + 1),
        };
    }
}

pub(super) struct Projection {
    pub(super) span: Range<usize>,
    pub(super) trim: Line<f32>,
    pub(super) local_gap: Option<f32>,
    pub(super) name_repeat: Option<NameRepeat>,
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
        name_repeat,
        parent_rtl,
        child_rtl,
    } = projection;
    let reverse_order = parent_rtl != child_rtl;
    let tracks = source.positions.get(span.clone()).ok_or_else(unresolved)?;
    if tracks.is_empty() || tracks.len() > MAX_TRACKS {
        return Err(limited("EPUB CSS inherited grid track limit"));
    }
    if source
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

    let positions = tracks.len() + 1;
    let alias_index = |position: usize| {
        let Some(repeat) = name_repeat else {
            return position;
        };
        if position < repeat.start {
            return position;
        }
        let fixed = aliases.len() - repeat.length;
        let copies = positions.saturating_sub(fixed) / repeat.length;
        let offset = position - repeat.start;
        if offset < copies * repeat.length {
            repeat.start + offset % repeat.length
        } else {
            position + repeat.length - copies * repeat.length
        }
    };
    let mut total = 0;
    let names = (0..=tracks.len())
        .map(|index| {
            let line_index = if reverse_order {
                span.end - index
            } else {
                span.start + index
            };
            let mut line = source.names_for_line(line_index).to_vec();
            for alias in aliases.get(alias_index(index)).into_iter().flatten() {
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
