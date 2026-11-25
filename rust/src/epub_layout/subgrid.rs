//! Materialize independently sized inherited grid tracks for the native solver.
use std::{collections::BTreeMap, ops::Range};

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

pub(super) struct AreaOrigins {
    pub(super) areas: Vec<GridTemplateArea<String>>,
    pub(super) authored_names: [Option<Vec<Vec<String>>>; 2],
}

pub(super) struct AreaTemplates {
    pub(super) native: GridTemplateAreas<String>,
    pub(super) origins: Option<Box<AreaOrigins>>,
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
    source: &[GridTemplateArea<String>],
    local: Option<&GridTemplateAreas<String>>,
    details: &DetailedGridInfo,
    item: &DetailedGridItemsInfo,
    axes: &mut [Option<Axis>; 2],
    reverse_columns: bool,
) -> Result<Option<AreaTemplates>, EngineError> {
    let [columns, rows] = &*axes;
    let count =
        |axis: &Option<Axis>, local| axis.as_ref().map_or(local, |axis| axis.tracks.len() as u16);
    let column_count = count(columns, local.map_or(0, |template| template.column_count));
    let row_count = count(rows, local.map_or(0, |template| template.row_count));
    let project =
        |inherited: bool, area: Range<u16>, span: Range<u16>, leading: u16, reverse: bool| {
            if !inherited || area.start == 0 {
                // Taffy's native resolver uses zero for a missing area edge. An
                // absent axis stays absent even when a descendant inherits it.
                return Some((0, 0));
            }
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
    let mut areas = Vec::new();
    for area in source {
        let Some((column_start, column_end)) = project(
            columns.is_some(),
            area.column_start..area.column_end,
            item.column_start..item.column_end,
            details.columns.negative_implicit_tracks,
            reverse_columns,
        ) else {
            continue;
        };
        let Some((row_start, row_end)) = project(
            rows.is_some(),
            area.row_start..area.row_end,
            item.row_start..item.row_end,
            details.rows.negative_implicit_tracks,
            false,
        ) else {
            continue;
        };
        if column_start == 0 && row_start == 0 {
            continue;
        }
        areas.push(GridTemplateArea {
            name: area.name.clone(),
            row_start,
            row_end,
            column_start,
            column_end,
        });
    }
    let has_inherited = !areas.is_empty();
    if let Some(local) = local {
        for area in &local.areas {
            let column_end = area.column_end.min(column_count + 1);
            let row_end = area.row_end.min(row_count + 1);
            if area.column_start < column_end && area.row_start < row_end {
                areas.push(GridTemplateArea {
                    column_end,
                    row_end,
                    ..area.clone()
                });
            }
        }
    } else if !has_inherited {
        return Ok(None);
    }
    if has_inherited {
        for (axis, count) in axes.iter().zip([
            areas.iter().filter(|area| area.column_start != 0).count(),
            areas.iter().filter(|area| area.row_start != 0).count(),
        ]) {
            if let Some(axis) = axis {
                let authored = axis.names.iter().map(Vec::len).sum::<usize>();
                if authored + count * 2 > MAX_LINE_NAMES {
                    return Err(limited("EPUB CSS inherited grid line name limit"));
                }
            }
        }
    }
    let (areas, origins) = merge_area_names(areas, axes);
    Ok(Some(AreaTemplates {
        native: GridTemplateAreas {
            areas,
            row_count,
            column_count,
        },
        origins,
    }))
}

fn merge_area_names(
    areas: Vec<GridTemplateArea<String>>,
    axes: &mut [Option<Axis>; 2],
) -> (Vec<GridTemplateArea<String>>, Option<Box<AreaOrigins>>) {
    let earliest = |old, next| match (old, next) {
        (0, _) => next,
        (_, 0) => old,
        (old, next) => u16::min(old, next),
    };
    let mut resolved = BTreeMap::<&str, GridTemplateArea<String>>::new();
    for area in &areas {
        resolved
            .entry(&area.name)
            .and_modify(|first| {
                first.column_start = earliest(first.column_start, area.column_start);
                first.column_end = earliest(first.column_end, area.column_end);
                first.row_start = earliest(first.row_start, area.row_start);
                first.row_end = earliest(first.row_end, area.row_end);
            })
            .or_insert_with(|| area.clone());
    }
    if resolved.len() == areas.len() {
        return (areas, None);
    }

    // Taffy stores a single rectangle per area name. Its earliest edges
    // resolve bare names. Later edges remain available as numbered lines.
    let authored_names = axes
        .each_ref()
        .map(|axis| axis.as_ref().map(|axis| axis.names.clone()));
    for area in &areas {
        let first = &resolved[area.name.as_str()];
        let edges = [
            [
                (area.column_start, first.column_start),
                (area.column_end, first.column_end),
            ],
            [
                (area.row_start, first.row_start),
                (area.row_end, first.row_end),
            ],
        ];
        for (axis, edges) in axes.iter_mut().zip(edges) {
            let Some(axis) = axis else { continue };
            for (suffix, (position, first)) in ["start", "end"].into_iter().zip(edges) {
                if position > first {
                    let names = &mut axis.names[usize::from(position - 1)];
                    let name = format!("{}-{suffix}", area.name);
                    if !names.contains(&name) {
                        names.push(name);
                    }
                }
            }
        }
    }
    let native = resolved.into_values().collect();
    (
        native,
        Some(Box::new(AreaOrigins {
            areas,
            authored_names,
        })),
    )
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
    authored_names: Option<&[Vec<String>]>,
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
            // Additional generated aliases used by native placement must
            // be clipped again from the original rectangles, not inherited
            // as authored names on their previous physical lines.
            let mut line = if let Some(names) = authored_names {
                line_index
                    .checked_sub(usize::from(source.negative_implicit_tracks))
                    .and_then(|index| names.get(index))
                    .cloned()
                    .unwrap_or_default()
            } else {
                source.names_for_line(line_index).to_vec()
            };
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
