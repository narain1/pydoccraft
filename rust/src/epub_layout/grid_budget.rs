//! Bound automatic grid expansion before Taffy allocates the occupancy grid.
use taffy::{
    geometry::Size,
    style::{
        BoxSizing, GridTemplateComponent, LengthPercentage, Overflow, Style, TrackSizingFunction,
    },
    util::{MaybeMath, MaybeResolve, ResolveOrZero},
    LayoutInput, RequestedAxis, RunMode, SizingMode,
};

use crate::epub::css::{
    grid::{MAX_LINE_NAMES, MAX_TRACKS},
    CalculationTable,
};

pub(super) fn permits(style: &Style, input: LayoutInput, calculations: &CalculationTable) -> bool {
    let has_repeat =
        |part: &GridTemplateComponent<String>| matches!(part, GridTemplateComponent::Repeat(_));
    if !style.grid_template_columns.iter().any(has_repeat)
        && !style.grid_template_rows.iter().any(has_repeat)
    {
        return true;
    }

    let parent = input.parent_size;
    let calc = |handle, basis| calculations.at(handle, basis);
    let padding = style.padding.resolve_or_zero(parent.width, calc);
    let border = style.border.resolve_or_zero(parent.width, calc);
    let edges = (padding + border).sum_axes();
    let gutter = style.overflow.transpose().map(|value| match value {
        Overflow::Scroll => style.scrollbar_width,
        _ => 0.0,
    });
    let box_sizing = if style.box_sizing == BoxSizing::ContentBox {
        edges
    } else {
        Size::ZERO
    };
    let resolve = |size: Size<taffy::Dimension>| {
        size.maybe_resolve(parent, calc)
            .maybe_apply_aspect_ratio(style.aspect_ratio)
            .maybe_add(box_sizing)
    };
    let minimum = resolve(style.min_size.map(Into::into));
    let maximum = resolve(style.max_size.map(Into::into));
    let preferred = if input.sizing_mode == SizingMode::InherentSize {
        resolve(style.size)
    } else {
        Size::NONE
    };
    let outer = input
        .known_dimensions
        .or(preferred)
        .maybe_clamp(minimum, maximum)
        .maybe_max(edges);
    if input.run_mode == RunMode::ComputeSize
        && outer.width.is_some()
        && (outer.height.is_some() || input.axis == RequestedAxis::Horizontal)
    {
        return true;
    }

    let largest = outer.or(maximum);
    let capacity = largest
        .or(minimum)
        .maybe_clamp(minimum, maximum)
        .maybe_max(edges)
        .maybe_sub(edges + Size::from(gutter));
    let check =
        |template: &[GridTemplateComponent<String>],
         names: &[Vec<String>],
         capacity,
         round_up,
         gap| { permits_axis(template, names, capacity, round_up, gap, calc) };
    check(
        &style.grid_template_columns,
        &style.grid_template_column_names,
        capacity.width,
        largest.width.is_none(),
        style.gap.width,
    ) && check(
        &style.grid_template_rows,
        &style.grid_template_row_names,
        capacity.height,
        largest.height.is_none(),
        style.gap.height,
    )
}

fn permits_axis(
    template: &[GridTemplateComponent<String>],
    names: &[Vec<String>],
    capacity: Option<f32>,
    round_up: bool,
    gap: LengthPercentage,
    calc: impl Fn(taffy::style::CalcHandle, f32) -> f32,
) -> bool {
    let Some(capacity) = capacity else {
        return true;
    };
    let Some(repeat) = template.iter().find_map(|part| match part {
        GridTemplateComponent::Single(_) => None,
        GridTemplateComponent::Repeat(repeat) => Some(repeat),
    }) else {
        return true;
    };

    // EPUB flattens integer repeats and rejects multiple automatic repeats.
    // The native count uses the widest definite side of each track, and
    // includes one fewer gap in its first repetition.
    let fixed = template.len() - 1;
    let pattern = repeat.tracks.len();
    let track_cap = (MAX_TRACKS - fixed) / pattern;
    let outer_names: usize = names.iter().map(Vec::len).sum();
    let repeated_names: usize = repeat.line_names.iter().map(Vec::len).sum();
    let name_cap = (MAX_LINE_NAMES - outer_names)
        .checked_div(repeated_names)
        .unwrap_or(track_cap);
    let cap = name_cap.min(track_cap);
    let breadth = |track: &TrackSizingFunction| {
        let min = track.min.definite_value(Some(capacity), &calc);
        let max = track.max.definite_value(Some(capacity), &calc);
        max.map(|max| max.maybe_max(min)).or(min).unwrap_or(0.0)
    };
    let fixed_space: f32 = template
        .iter()
        .map(|part| match part {
            GridTemplateComponent::Single(track) => breadth(track),
            GridTemplateComponent::Repeat(_) => 0.0,
        })
        .sum();
    let pattern_space: f32 = repeat.tracks.iter().map(breadth).sum();
    let gap = gap.resolve_or_zero(Some(capacity), &calc);
    let first = fixed_space + pattern_space + (fixed + pattern - 1) as f32 * gap;
    if first > capacity {
        return cap > 0;
    }
    let extra = (capacity - first) / (pattern_space + pattern as f32 * gap);
    let extra = if round_up {
        extra.ceil()
    } else {
        extra.floor()
    };
    (extra as usize) < cap
}
