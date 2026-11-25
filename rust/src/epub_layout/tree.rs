//! Feed the chapter's box tree and calculated lengths to Taffy's layout algorithms.
use taffy::{
    compute_block_layout, compute_cached_layout, compute_flexbox_layout, compute_grid_layout,
    compute_hidden_layout, compute_oof_layout, compute_root_layout,
    geometry::{AbsoluteAxis, Line, Size},
    style::{
        AlignContent, AlignItems, AlignItemsKeyword, AvailableSpace, BoxSizing, CalcHandle,
        Dimension, Display, ExpandedDimension, ExpandedLengthPercentage, FlexDirection, FlexWrap,
        LengthPercentageAuto, Position, Style,
    },
    util::{MaybeResolve, ResolveOrZero},
    BlockContext, Cache, CacheTree, DetailedGridInfo, DetailedLayoutInfo, Layout,
    LayoutBlockContainer, LayoutContainingBlock, LayoutFlexboxContainer, LayoutGridContainer,
    LayoutInput, LayoutOutput, LayoutPartialTree, NodeId, RequestedAxis, RunMode, SizingMode,
    TraversePartialTree,
};

use crate::epub::{
    css::CalculationTable,
    limited,
    style::{IntrinsicBound, IntrinsicBounds, SizeBounds},
    unsupported,
};
use crate::EngineError;

#[path = "grid_budget.rs"]
mod grid_budget;
#[path = "subgrid.rs"]
mod subgrid;

#[derive(Default)]
pub(super) struct Tree {
    nodes: Vec<Node>,
}

struct Node {
    style: Style,
    bounds: IntrinsicBounds,
    parent: Option<NodeId>,
    has_relative_math: bool,
    needs_intrinsic_geometry: bool,
    children: Vec<NodeId>,
    layout: Layout,
    cache: Cache,
    hoisted: Vec<NodeId>,
    details: DetailedLayoutInfo,
}

impl Tree {
    pub(super) fn add(
        &mut self,
        style: Style,
        bounds: IntrinsicBounds,
        children: Vec<NodeId>,
        relative_math: bool,
    ) -> NodeId {
        let id = NodeId::from(self.nodes.len());
        let has_relative_math = relative_math
            || children
                .iter()
                .any(|id| self.nodes[usize::from(*id)].has_relative_math);
        let needs_intrinsic_geometry = bounds.has_inline()
            || bounds.has_block()
            || bounds.contain.width
            || bounds.contain.height
            || bounds.subgrid.width
            || bounds.subgrid.height
            || children
                .iter()
                .any(|id| self.nodes[usize::from(*id)].needs_intrinsic_geometry);
        for child in &children {
            self.nodes[usize::from(*child)].parent = Some(id);
        }
        self.nodes.push(Node {
            style,
            bounds,
            parent: None,
            has_relative_math,
            needs_intrinsic_geometry,
            children,
            layout: Layout::with_order(0),
            cache: Cache::new(),
            hoisted: Vec::new(),
            details: DetailedLayoutInfo::None,
        });
        id
    }

    pub(super) fn style_mut(&mut self, id: NodeId) -> &mut Style {
        &mut self.nodes[usize::from(id)].style
    }

    pub(super) fn layout(&self, id: NodeId) -> &Layout {
        &self.nodes[usize::from(id)].layout
    }

    pub(super) fn hoisted_children(&self, id: NodeId) -> &[NodeId] {
        &self.nodes[usize::from(id)].hoisted
    }

    pub(super) fn place(
        &mut self,
        root: NodeId,
        space: Size<AvailableSpace>,
        calculations: &CalculationTable,
        measure: impl FnMut(LayoutInput, usize, &Style, bool) -> LayoutOutput,
    ) -> Result<(), EngineError> {
        let viewport = space.map(AvailableSpace::into_option);
        let mut view = View {
            viewport,
            geometry: Vec::new(),
            flex_widths: Vec::new(),
            grid_areas: Vec::new(),
            probing_grid: false,
            nodes: &mut self.nodes,
            calculations,
            measure,
            fit_probes: 0,
            grid_limit: false,
        };
        if view.nodes.iter().any(|node| node.needs_intrinsic_geometry) {
            view.geometry
                .resize(view.nodes.len(), BoxGeometry::default());
            view.grid_areas.resize(view.nodes.len(), None);
            view.resolve_geometry()?;
            view.measure_flex_widths()?;
            view.resolve_bounds()?;
        }
        for node in &mut *view.nodes {
            if !matches!(node.details, DetailedLayoutInfo::None) {
                node.details = DetailedLayoutInfo::None;
            }
        }
        compute_root_layout(&mut view, root, space);
        if view.fit_probes > MAX_FIT_PROBES {
            return Err(limited(
                "EPUB CSS intrinsic box layout exceeds its sizing budget",
            ));
        }
        if view.grid_limit {
            return Err(limited(
                "EPUB CSS grid automatic repetition exceeds its track or line name budget",
            ));
        }
        if view
            .flex_widths
            .iter()
            .zip(view.nodes.iter())
            .any(|(width, node)| {
                width.is_some_and(|width| (node.layout.size.width - width).abs() > 0.001)
            })
        {
            return Err(unsupported(
                "EPUB CSS block bounds changed their measured flex width",
            ));
        }
        view.verify_grid_areas()?;
        view.verify_subgrids()?;
        Ok(())
    }
}

pub(super) fn calculated_length(
    calculations: &CalculationTable,
    handle: CalcHandle,
    basis: f32,
) -> f32 {
    calculations.at(handle, basis)
}

#[derive(Clone, Copy, Default)]
struct BoxGeometry {
    content: Size<Option<f32>>,
    padding: Size<Option<f32>>,
    outer: Size<Option<f32>>,
    auto_block: bool,
}

#[derive(Clone, Copy)]
struct GridArea {
    item_slot: usize,
    size: Size<f32>,
    block_probe_width: Option<f32>,
    subgrid_outer: Size<Option<f32>>,
}

struct View<'a, F> {
    viewport: Size<Option<f32>>,
    geometry: Vec<BoxGeometry>,
    flex_widths: Vec<Option<f32>>,
    grid_areas: Vec<Option<GridArea>>,
    probing_grid: bool,
    nodes: &'a mut [Node],
    calculations: &'a CalculationTable,
    measure: F,
    fit_probes: usize,
    grid_limit: bool,
}

const MAX_FIT_PROBES: usize = 4_096;

fn outer_size(
    style: &Style,
    axis: AbsoluteAxis,
    edges: f32,
    basis: Option<f32>,
    auto_size: Option<f32>,
    calculate: impl Fn(CalcHandle, f32) -> f32,
) -> Option<f32> {
    let to_outer = |width| {
        if style.box_sizing == BoxSizing::ContentBox {
            width + edges
        } else {
            width
        }
    };
    let size = style.size.get_abs(axis);
    let preferred = if size.is_auto() {
        auto_size?
    } else {
        to_outer(size.maybe_resolve(basis, &calculate)?)
    };
    let limit = |length: LengthPercentageAuto, initial| {
        if length.is_auto() {
            Some(initial)
        } else {
            length.maybe_resolve(basis, &calculate).map(to_outer)
        }
    };
    Some(
        preferred
            .min(limit(style.max_size.get_abs(axis), f32::INFINITY)?)
            .max(limit(style.min_size.get_abs(axis), 0.0)?)
            .max(edges),
    )
}

fn box_axis(
    style: &Style,
    axis: AbsoluteAxis,
    basis: Option<f32>,
    inline_basis: Option<f32>,
    auto_size: Option<f32>,
    calculate: impl Fn(CalcHandle, f32) -> f32,
) -> Option<(f32, f32, f32)> {
    let edges = |sides: taffy::geometry::Rect<taffy::LengthPercentage>| {
        let [start, end] = match axis {
            AbsoluteAxis::Horizontal => [sides.left, sides.right],
            AbsoluteAxis::Vertical => [sides.top, sides.bottom],
        };
        Some(
            start.maybe_resolve(inline_basis, &calculate)?
                + end.maybe_resolve(inline_basis, &calculate)?,
        )
    };
    let padding = edges(style.padding)?;
    let border = edges(style.border)?;
    let outer = outer_size(style, axis, padding + border, basis, auto_size, calculate)?;
    outer.is_finite().then_some((
        (outer - (padding + border)).max(0.0),
        (outer - border).max(0.0),
        outer,
    ))
}

// Both a finite limit and an available extent refer to the same aspect-ratio box.
fn contain_axis(
    available: Option<f32>,
    margin: f32,
    edges: f32,
    property_to_ratio: f32,
    min: LengthPercentageAuto,
    max: LengthPercentageAuto,
    calculate: impl Fn(CalcHandle, f32) -> f32,
) -> Option<(f32, f32)> {
    let limit = |value: LengthPercentageAuto, default| {
        if value.is_auto() {
            Some(default)
        } else {
            value
                .maybe_resolve(available, &calculate)
                .map(|length| (length + property_to_ratio).max(0.0))
        }
    };
    let minimum = limit(min, 0.0)?;
    let maximum = limit(max, f32::INFINITY)?.max(minimum);
    let available = available.map_or(f32::INFINITY, |size| (size - margin - edges).max(0.0));
    Some((minimum, available.min(maximum)))
}

fn unproven_grid() -> EngineError {
    unsupported("EPUB CSS intrinsic bounds changed their independently sized grid area")
}

fn grid_area_size(details: &DetailedGridInfo, slot: usize) -> Option<Size<f32>> {
    let item = details.items.get(slot)?;
    if item.row_start == 0
        || item.column_start == 0
        || item.row_start >= item.row_end
        || item.column_start >= item.column_end
        || usize::from(item.row_end - 1) > details.rows.positions.len()
        || usize::from(item.column_end - 1) > details.columns.positions.len()
    {
        return None;
    }
    let (_, size) = details.item_grid_area(slot)?;
    (size.width.is_finite() && size.height.is_finite()).then_some(size)
}

impl<F: FnMut(LayoutInput, usize, &Style, bool) -> LayoutOutput> View<'_, F> {
    fn resolve_geometry(&mut self) -> Result<(), EngineError> {
        // The tree stores children before parents. Resolve containing boxes
        // once in the other direction so chains of percentages take linear work.
        for index in (0..self.nodes.len()).rev() {
            if self.grid_limit || self.fit_probes > MAX_FIT_PROBES {
                break;
            }
            self.materialize_subgrid(index)?;
            self.resolve_contain(index)?;
            if !self.nodes[index].children.is_empty() {
                self.geometry[index] = self.box_geometry(index);
                if self.nodes[index].style.display == Display::Grid {
                    self.measure_grid_areas(index)?;
                }
            }
        }
        Ok(())
    }

    fn materialize_subgrid(&mut self, index: usize) -> Result<(), EngineError> {
        use AbsoluteAxis::{Horizontal, Vertical};

        let node = &self.nodes[index];
        let axes = node.bounds.subgrid;
        if node.style.display != Display::Grid || !(axes.width || axes.height) {
            return Ok(());
        }
        let parent = node
            .parent
            .map(usize::from)
            .ok_or_else(subgrid::unresolved)?;
        let parent = &self.nodes[parent];
        let area = self.grid_area(index).ok_or_else(subgrid::unresolved)?;
        let DetailedLayoutInfo::Grid(details) = &parent.details else {
            return Err(subgrid::unresolved());
        };
        let item = details
            .items
            .get(area.item_slot)
            .ok_or_else(subgrid::unresolved)?;
        let style = &node.style;
        if (style.aspect_ratio.is_some() && !(axes.width && axes.height))
            || style.scrollbar_width != 0.0
        {
            return Err(subgrid::unresolved());
        }
        let calculate = |handle, basis| self.calculations.at(handle, basis);
        let inline = Some(area.size.width);
        let padding = style.padding.resolve_or_zero(inline, calculate);
        let border = style.border.resolve_or_zero(inline, calculate);
        let margin = style.margin.resolve_or_zero(inline, calculate);
        let trim = padding + border + margin;
        let outer = area.size - margin.sum_axes();
        let inner = area.size - trim.sum_axes();
        let mut layout = [None, None];
        for (axis, output) in [Horizontal, Vertical].into_iter().zip(&mut layout) {
            if !axes.get_abs(axis) {
                continue;
            }
            let (tracks, span, aliases, edges, parent_rtl, child_rtl) = match axis {
                Horizontal => (
                    &details.columns,
                    usize::from(item.column_start - 1)..usize::from(item.column_end - 1),
                    &style.grid_template_column_names,
                    trim.horizontal_components(),
                    parent.style.direction.is_rtl(),
                    style.direction.is_rtl(),
                ),
                Vertical => (
                    &details.rows,
                    usize::from(item.row_start - 1)..usize::from(item.row_end - 1),
                    &style.grid_template_row_names,
                    trim.vertical_components(),
                    false,
                    false,
                ),
            };
            let content = inner.get_abs(axis);
            if !content.is_finite()
                || content < 0.0
                || !outer.get_abs(axis).is_finite()
                || outer.get_abs(axis) < 0.0
            {
                return Err(subgrid::unresolved());
            }
            let local_gap = node.bounds.subgrid_local_gap.get_abs(axis).then(|| {
                style
                    .gap
                    .get_abs(axis)
                    .resolve_or_zero(Some(content), calculate)
            });
            let trim = if child_rtl {
                Line {
                    start: edges.end,
                    end: edges.start,
                }
            } else {
                edges
            };
            *output = Some(subgrid::inherit_axis(
                tracks,
                aliases,
                subgrid::Projection {
                    span,
                    trim,
                    local_gap,
                    parent_rtl,
                    child_rtl,
                },
            )?);
        }

        // CSS Grid stretches subgrids in their inherited axes regardless of
        // the authored preferred size, limits or alignment in those axes.
        let node = &mut self.nodes[index];
        let style = &mut node.style;
        let [horizontal, vertical] = layout;
        let mut expected = Size::NONE;
        if let Some(inherited) = horizontal {
            style.grid_template_columns = inherited.tracks;
            style.grid_template_column_names = inherited.names;
            style.gap.width = inherited.gap;
            style.size.width = Dimension::auto();
            style.min_size.width = LengthPercentageAuto::auto();
            style.max_size.width = LengthPercentageAuto::auto();
            style.justify_self = Some(AlignItems::STRETCH);
            style.justify_content = AlignContent::START;
            style.margin.left = LengthPercentageAuto::length(margin.left);
            style.margin.right = LengthPercentageAuto::length(margin.right);
            node.bounds.inline = SizeBounds {
                min: None,
                max: None,
            };
            expected.width = Some(outer.width);
        }
        if let Some(inherited) = vertical {
            style.grid_template_rows = inherited.tracks;
            style.grid_template_row_names = inherited.names;
            style.gap.height = inherited.gap;
            style.size.height = Dimension::auto();
            style.min_size.height = LengthPercentageAuto::auto();
            style.max_size.height = LengthPercentageAuto::auto();
            style.align_self = Some(AlignItems::STRETCH);
            style.align_content = AlignContent::START;
            style.margin.top = LengthPercentageAuto::length(margin.top);
            style.margin.bottom = LengthPercentageAuto::length(margin.bottom);
            node.bounds.block = SizeBounds {
                min: None,
                max: None,
            };
            expected.height = Some(outer.height);
        }
        if axes.width && axes.height {
            style.aspect_ratio = None;
        }
        self.grid_areas[index]
            .as_mut()
            .ok_or_else(subgrid::unresolved)?
            .subgrid_outer = expected;
        Ok(())
    }

    fn verify_subgrids(&self) -> Result<(), EngineError> {
        for (index, node) in self.nodes.iter().enumerate() {
            let axes = node.bounds.subgrid;
            if node.style.display != Display::Grid || !(axes.width || axes.height) {
                continue;
            }
            let area = self.grid_area(index).ok_or_else(subgrid::unresolved)?;
            let different = node
                .layout
                .size
                .zip_map(area.subgrid_outer, |actual, expected| {
                    expected.is_none_or(|expected| {
                        !actual.is_finite() || (actual - expected).abs() > 0.001
                    })
                });
            if (axes.width && different.width) || (axes.height && different.height) {
                return Err(subgrid::unresolved());
            }
            if let DetailedLayoutInfo::Grid(details) = &node.details {
                if (axes.width && subgrid::implicit(&details.columns))
                    || (axes.height && subgrid::implicit(&details.rows))
                {
                    return Err(unsupported(
                        "EPUB CSS implicit tracks on an inherited subgrid axis",
                    ));
                }
            }
        }
        Ok(())
    }

    fn resolve_contain(&mut self, index: usize) -> Result<(), EngineError> {
        let node = &self.nodes[index];
        let style = &node.style;
        let axes = node
            .bounds
            .contain
            .zip_map(style.size, |contain, size| contain && size.is_stretch());
        let Some(ratio) = style.aspect_ratio else {
            return Ok(());
        };
        if !(axes.width || axes.height) || style.display == Display::None {
            return Ok(());
        }
        let unresolved =
            || unsupported("EPUB CSS contain-fit requires independently sized block geometry");
        if node.bounds.has_inline() || node.bounds.has_block() || style.scrollbar_width != 0.0 {
            return Err(unresolved());
        }

        let available = if style.position == Position::Fixed {
            self.viewport
        } else {
            Size {
                width: Some(
                    self.definite_containing_width(index)
                        .ok_or_else(unresolved)?,
                ),
                height: self.contain_block_height(index).ok_or_else(unresolved)?,
            }
        };
        let width = available.width.ok_or_else(unresolved)?;
        let calculate = |handle, basis| self.calculations.at(handle, basis);
        let margin = style
            .margin
            .resolve_or_zero(Some(width), calculate)
            .sum_axes();
        let padding = style
            .padding
            .resolve_or_zero(Some(width), calculate)
            .sum_axes();
        let border = style
            .border
            .resolve_or_zero(Some(width), calculate)
            .sum_axes();
        let edges = Size {
            width: padding.width + border.width,
            height: padding.height + border.height,
        };
        let property_edges = edges.map(|edge| {
            if style.box_sizing == BoxSizing::ContentBox {
                edge
            } else {
                0.0
            }
        });
        let ratio_edges = if node.bounds.content_box_ratio {
            edges
        } else {
            property_edges
        };
        let (minimum_width, maximum_width) = contain_axis(
            available.width,
            margin.width,
            ratio_edges.width,
            property_edges.width - ratio_edges.width,
            style.min_size.width,
            style.max_size.width,
            calculate,
        )
        .ok_or_else(unresolved)?;
        let (minimum_height, maximum_height) = contain_axis(
            available.height,
            margin.height,
            ratio_edges.height,
            property_edges.height - ratio_edges.height,
            style.min_size.height,
            style.max_size.height,
            calculate,
        )
        .ok_or_else(unresolved)?;
        let width = f64::from(maximum_width).min(f64::from(maximum_height) * f64::from(ratio));
        let height = width / f64::from(ratio);
        let fitted = Size {
            width: width as f32,
            height: height as f32,
        };
        if !fitted.width.is_finite() || !fitted.height.is_finite() {
            return Err(unresolved());
        }
        if fitted.width < minimum_width || fitted.height < minimum_height {
            return Err(unsupported(
                "EPUB CSS contain-fit aspect ratio conflicts with a minimum size",
            ));
        }
        // Keep the other preferred size native unless an independent content-box ratio
        // needs the content-to-border adjustment on both axes.
        let resolve_content_auto =
            node.bounds.content_box_ratio && style.box_sizing == BoxSizing::BorderBox;
        let style = &mut self.nodes[index].style;
        for (axis, size, fitted, ratio_edge, property_edge) in [
            (
                axes.width,
                &mut style.size.width,
                fitted.width,
                ratio_edges.width,
                property_edges.width,
            ),
            (
                axes.height,
                &mut style.size.height,
                fitted.height,
                ratio_edges.height,
                property_edges.height,
            ),
        ] {
            if axis || (resolve_content_auto && size.is_auto()) {
                *size = Dimension::length((fitted + ratio_edge - property_edge).max(0.0));
            }
        }
        Ok(())
    }

    fn containing_geometry(&self, index: usize) -> Option<(BoxGeometry, &Style)> {
        let node = &self.nodes[index];
        let parent = usize::from(node.parent?);
        let parent_style = &self.nodes[parent].style;
        if !matches!(parent_style.display, Display::Block | Display::FlowRoot)
            || (node.style.position == Position::Absolute
                && parent_style.position == Position::Static)
        {
            return None;
        }
        Some((self.geometry[parent], parent_style))
    }

    fn containing_size(&self, index: usize) -> Size<Option<f32>> {
        let node = &self.nodes[index];
        if node.parent.is_none() || node.style.position == Position::Fixed {
            return self.viewport;
        }
        if let Some(area) = self.grid_area(index) {
            return area.size.map(Some);
        }
        let Some((geometry, _)) = self.containing_geometry(index) else {
            return Size::NONE;
        };
        if node.style.position == Position::Absolute {
            geometry.padding
        } else {
            geometry.content
        }
    }

    // The outer option records whether the containing block could be proven.
    fn contain_block_height(&self, index: usize) -> Option<Option<f32>> {
        if let Some(area) = self.grid_area(index) {
            return Some(Some(area.size.height));
        }
        let (geometry, _) = self.containing_geometry(index)?;
        if self.nodes[index].style.position == Position::Absolute {
            geometry.padding.height.map(Some)
        } else if geometry.auto_block {
            Some(None)
        } else {
            geometry.content.height.map(Some)
        }
    }

    fn box_geometry(&self, index: usize) -> BoxGeometry {
        let node = &self.nodes[index];
        let style = &node.style;
        if !matches!(
            style.display,
            Display::Block | Display::FlowRoot | Display::Flex | Display::Grid
        ) || style.aspect_ratio.is_some()
            || style.scrollbar_width != 0.0
        {
            return BoxGeometry::default();
        }

        let basis = self.containing_size(index);
        let calculate = |handle, basis| self.calculations.at(handle, basis);
        let mut geometry = BoxGeometry::default();
        if !node.bounds.has_inline() && !self.inline_flex_changes_width(node) {
            let width = self.auto_outer_inline(index, basis.width);
            if let Some((content, padding, outer)) = box_axis(
                style,
                AbsoluteAxis::Horizontal,
                basis.width,
                basis.width,
                width,
                calculate,
            ) {
                geometry.content.width = Some(content);
                geometry.padding.width = Some(padding);
                geometry.outer.width = Some(outer);
            }
        }
        let grid_area = self.grid_area(index);
        let adjustable_block = grid_area.is_none()
            && !style.position.is_out_of_flow()
            && node.parent.is_some_and(|parent| {
                matches!(
                    self.nodes[usize::from(parent)].style.display,
                    Display::Flex | Display::Grid
                )
            });
        if !node.bounds.has_block() && !adjustable_block {
            if let Some((content, padding, outer)) = box_axis(
                style,
                AbsoluteAxis::Vertical,
                basis.height,
                basis.width,
                grid_area.and_then(|area| area.subgrid_outer.height),
                calculate,
            ) {
                geometry.content.height = Some(content);
                geometry.padding.height = Some(padding);
                geometry.outer.height = Some(outer);
            } else {
                geometry.auto_block = style.size.height.is_auto()
                    && style.min_size.height.is_auto()
                    && style.max_size.height.is_auto()
                    && !(style.position.is_out_of_flow()
                        && !style.inset.top.is_auto()
                        && !style.inset.bottom.is_auto());
            }
        }
        geometry
    }

    fn auto_outer_inline(&self, index: usize, inline: Option<f32>) -> Option<f32> {
        let width = inline?;
        let node = &self.nodes[index];
        let style = &node.style;
        if !style.size.width.is_auto() || style.item_is_replaced {
            return None;
        }
        let grid = self
            .grid_area(index)
            .and_then(|_| node.parent.map(|id| &self.nodes[usize::from(id)].style));
        if grid.is_some() && (style.margin.left.is_auto() || style.margin.right.is_auto()) {
            return None;
        }
        let parent = grid.or_else(|| self.containing_geometry(index).map(|(_, parent)| parent));
        if parent.is_none() && style.position != Position::Fixed {
            return None;
        }
        let alignment = style
            .justify_self
            .or(parent.map(|parent| parent.justify_items))
            .unwrap_or(AlignItems::NORMAL);
        if !matches!(
            alignment.keyword,
            AlignItemsKeyword::Normal | AlignItemsKeyword::Stretch
        ) {
            return None;
        }
        let calculate = |handle, basis| self.calculations.at(handle, basis);
        let margins = style
            .margin
            .resolve_or_zero(inline, calculate)
            .horizontal_axis_sum();
        let available = if style.position.is_out_of_flow() {
            let left = style.inset.left.maybe_resolve(inline, calculate)?;
            let right = style.inset.right.maybe_resolve(inline, calculate)?;
            (width - left - right).max(0.0)
        } else {
            width
        };
        Some((available - margins).max(0.0))
    }

    fn grid_area(&self, index: usize) -> Option<GridArea> {
        self.grid_areas.get(index).copied().flatten()
    }

    fn grid_items(&self, index: usize) -> impl Iterator<Item = usize> + '_ {
        self.nodes[index]
            .children
            .iter()
            .map(|id| usize::from(*id))
            .filter(|&child| {
                let style = &self.nodes[child].style;
                style.display != Display::None && !style.position.is_out_of_flow()
            })
    }

    fn measure_grid_areas(&mut self, index: usize) -> Result<(), EngineError> {
        let node = &self.nodes[index];
        let geometry = self.geometry[index];
        let (Some(width), Some(height)) = (geometry.outer.width, geometry.outer.height) else {
            return Ok(());
        };
        if !self
            .grid_items(index)
            .any(|child| self.nodes[child].needs_intrinsic_geometry)
            || !grid_budget::independently_sized(&node.style, geometry.content)
            || self.fit_probes > MAX_FIT_PROBES
            || self.grid_limit
        {
            return Ok(());
        }
        let input = LayoutInput {
            run_mode: RunMode::PerformLayout,
            sizing_mode: SizingMode::InherentSize,
            axis: RequestedAxis::Both,
            known_dimensions: geometry.outer,
            known_dimensions_are_definite: Size {
                width: true,
                height: true,
            },
            parent_size: self.containing_size(index),
            available_space: Size {
                width: AvailableSpace::Definite(width),
                height: AvailableSpace::Definite(height),
            },
            ..LayoutInput::HIDDEN
        };
        if !grid_budget::permits(&node.style, input, self.calculations) {
            self.grid_limit = true;
            return Ok(());
        }

        // Native placement supplies areas before any target intrinsic bounds
        // are materialized. The track proof makes child contributions irrelevant.
        self.fit_probes += 1;
        self.probing_grid = true;
        compute_grid_layout(self, NodeId::from(index), input);
        self.probing_grid = false;
        let node = &self.nodes[index];
        let DetailedLayoutInfo::Grid(details) = &node.details else {
            return Err(unproven_grid());
        };
        let implicit = |tracks: &taffy::DetailedGridTracksInfo<String>| {
            tracks.negative_implicit_tracks != 0 || tracks.positive_implicit_tracks != 0
        };
        if (node.style.grid_auto_columns.is_empty() && implicit(&details.columns))
            || (node.style.grid_auto_rows.is_empty() && implicit(&details.rows))
        {
            return Ok(());
        }
        let children = self.grid_items(index).collect::<Vec<_>>();
        if children.len() != details.items.len() {
            return Err(unproven_grid());
        }
        for (slot, child) in children.into_iter().enumerate() {
            let size = grid_area_size(details, slot).ok_or_else(unproven_grid)?;
            self.grid_areas[child] = Some(GridArea {
                item_slot: slot,
                size,
                block_probe_width: None,
                subgrid_outer: Size::NONE,
            });
        }
        Ok(())
    }

    fn verify_grid_areas(&self) -> Result<(), EngineError> {
        for (index, node) in self.nodes.iter().enumerate() {
            if node.style.display != Display::Grid
                || !self
                    .grid_items(index)
                    .any(|child| self.grid_area(child).is_some())
            {
                continue;
            }
            let DetailedLayoutInfo::Grid(details) = &node.details else {
                return Err(unproven_grid());
            };
            for (slot, child) in self.grid_items(index).enumerate() {
                let Some(area) = self.grid_area(child) else {
                    continue;
                };
                let actual = grid_area_size(details, slot).ok_or_else(unproven_grid)?;
                let changed =
                    |before: f32, after: f32| !after.is_finite() || (before - after).abs() > 0.001;
                if changed(area.size.width, actual.width)
                    || changed(area.size.height, actual.height)
                    || area
                        .block_probe_width
                        .is_some_and(|width| changed(width, self.nodes[child].layout.size.width))
                {
                    return Err(unproven_grid());
                }
            }
        }
        Ok(())
    }

    fn measure_flex_widths(&mut self) -> Result<(), EngineError> {
        if !self
            .nodes
            .iter()
            .any(|node| node.bounds.has_block() && self.inline_flex_changes_width(node))
        {
            return Ok(());
        }
        let mut parents = vec![false; self.nodes.len()];
        for node in &*self.nodes {
            let Some(parent) = node.parent else { continue };
            if node.bounds.has_block()
                && node.style.display != Display::None
                && self.inline_flex_changes_width(node)
            {
                parents[usize::from(parent)] = true;
            }
        }
        self.flex_widths.resize(self.nodes.len(), None);
        let unsupported_width = || {
            unsupported(
                "EPUB CSS intrinsic block bounds require a definite inline size after flexing",
            )
        };

        for (index, requested) in parents.into_iter().enumerate() {
            if !requested || self.fit_probes > MAX_FIT_PROBES || self.grid_limit {
                continue;
            }
            let node = &self.nodes[index];
            let geometry = self.geometry[index];
            let width = geometry.outer.width.ok_or_else(unsupported_width)?;
            let inline = geometry.content.width;
            let calculate = |handle, basis| self.calculations.at(handle, basis);
            let independently_sized = |id: &NodeId| {
                let child = &self.nodes[usize::from(*id)];
                let style = &child.style;
                if style.display == Display::None || style.position.is_out_of_flow() {
                    return true;
                }
                let basis = if style.flex_basis.is_auto() {
                    style.size.width
                } else {
                    style.flex_basis
                };
                // The native main-axis inputs must never need an intrinsic
                // contribution that could change after applying a block bound.
                !child.bounds.has_inline()
                    && style.aspect_ratio.is_none()
                    && !style.item_is_replaced
                    && basis.maybe_resolve(inline, calculate).is_some()
                    && style
                        .min_size
                        .width
                        .maybe_resolve(inline, calculate)
                        .is_some()
                    && (style.max_size.width.is_auto()
                        || style
                            .max_size
                            .width
                            .maybe_resolve(inline, calculate)
                            .is_some())
            };
            if node.style.flex_wrap != FlexWrap::NoWrap
                || !node.children.iter().all(independently_sized)
            {
                return Err(unsupported_width());
            }

            self.fit_probes += 1;
            self.place_node(
                NodeId::from(index),
                LayoutInput {
                    run_mode: RunMode::PerformLayout,
                    sizing_mode: SizingMode::InherentSize,
                    axis: RequestedAxis::Both,
                    known_dimensions: geometry.outer,
                    known_dimensions_are_definite: geometry.outer.map(|size| size.is_some()),
                    parent_size: self.containing_size(index),
                    available_space: Size {
                        width: AvailableSpace::Definite(width),
                        height: geometry
                            .outer
                            .height
                            .map_or(AvailableSpace::MaxContent, AvailableSpace::Definite),
                    },
                    ..LayoutInput::HIDDEN
                },
                None,
            );
            for child in &self.nodes[index].children {
                let child = usize::from(*child);
                let node = &self.nodes[child];
                if node.bounds.has_block()
                    && node.style.display != Display::None
                    && self.inline_flex_changes_width(node)
                {
                    let width = node.layout.size.width;
                    if !width.is_finite() {
                        return Err(unsupported_width());
                    }
                    self.flex_widths[child] = Some(width);
                }
            }
        }
        for node in &mut *self.nodes {
            node.cache.clear();
        }
        Ok(())
    }

    fn resolve_bounds(&mut self) -> Result<(), EngineError> {
        let input = LayoutInput {
            run_mode: RunMode::ComputeSize,
            sizing_mode: SizingMode::InherentSize,
            axis: RequestedAxis::Horizontal,
            ..LayoutInput::HIDDEN
        };
        let mut measured = false;
        // Each child's computed bounds must be ready before a parent asks
        // for its native style or uses it in an intrinsic contribution.
        for index in 0..self.nodes.len() {
            let node = &self.nodes[index];
            let bounds = node.bounds;
            if node.style.display == Display::None || !bounds.has_inline() && !bounds.has_block() {
                continue;
            }
            if self.fit_probes > MAX_FIT_PROBES || self.grid_limit {
                break;
            }
            if bounds.has_inline() {
                self.resolve_inline_bound(index, input)?;
            }
            if bounds.has_block() {
                self.resolve_block_bound(index, input)?;
            }
            measured = true;
        }
        if measured {
            for node in &mut *self.nodes {
                node.cache.clear();
            }
        }
        Ok(())
    }

    fn resolve_inline_bound(
        &mut self,
        index: usize,
        mut input: LayoutInput,
    ) -> Result<(), EngineError> {
        let node = &self.nodes[index];
        let bounds = node.bounds.inline;
        if node.has_relative_math {
            return Err(unsupported(
                "EPUB CSS intrinsic width bounds on percentage MathML",
            ));
        }
        let style = &node.style;
        let edges = [
            style.padding.left,
            style.padding.right,
            style.border.left,
            style.border.right,
        ];
        let relative_edges = style.box_sizing == BoxSizing::BorderBox
            && edges
                .iter()
                .any(|edge| !matches!(edge.expand(), ExpandedLengthPercentage::Length(_)));
        let relative_limit = [bounds.min, bounds.max].into_iter().flatten().any(|bound| {
            matches!(bound, IntrinsicBound::FitContent(limit)
                if !matches!(limit.expand(), ExpandedLengthPercentage::Length(_)))
        });
        let basis = if relative_edges || relative_limit {
            Some(self.definite_containing_width(index).ok_or_else(|| {
                unsupported(
                    "EPUB CSS intrinsic inline bounds require a definite containing inline size",
                )
            })?)
        } else {
            None
        };
        let calculations = self.calculations;
        let calculate = |handle, basis| calculations.at(handle, basis);
        let subtract_padding = if style.box_sizing == BoxSizing::ContentBox {
            edges
                .into_iter()
                .map(|edge| edge.resolve_or_zero(basis, calculate))
                .sum()
        } else {
            0.0
        };
        input.parent_size.width = basis;
        let style = &mut self.nodes[index].style;
        let width = std::mem::replace(&mut style.size.width, Dimension::auto());
        let minimum = std::mem::replace(&mut style.min_size.width, LengthPercentageAuto::auto());
        let maximum = std::mem::replace(&mut style.max_size.width, LengthPercentageAuto::auto());
        let id = NodeId::from(index);
        let choices = [bounds.min, bounds.max];
        let mut probe = |space| (self.probe_width(id, input, space) - subtract_padding).max(0.0);
        let minimum_width = if choices.iter().any(|bound| {
            matches!(
                bound,
                Some(IntrinsicBound::Minimum | IntrinsicBound::FitContent(_))
            )
        }) {
            probe(AvailableSpace::MinContent)
        } else {
            0.0
        };
        let maximum_width = if choices.iter().any(|bound| {
            matches!(
                bound,
                Some(IntrinsicBound::Maximum | IntrinsicBound::FitContent(_))
            )
        }) {
            probe(AvailableSpace::MaxContent).max(minimum_width)
        } else {
            minimum_width
        };
        let used_bound = |bound| {
            let width = match bound {
                IntrinsicBound::Minimum => minimum_width,
                IntrinsicBound::Maximum => maximum_width,
                IntrinsicBound::FitContent(limit) => limit
                    .resolve_or_zero(basis, calculate)
                    .min(maximum_width)
                    .max(minimum_width),
            };
            LengthPercentageAuto::length(width)
        };
        let measured_min = bounds.min.map(used_bound);
        let measured_max = bounds.max.map(used_bound);
        let node = &mut self.nodes[index];
        node.cache.clear();
        node.style.size.width = width;
        node.style.min_size.width = measured_min.unwrap_or(minimum);
        node.style.max_size.width = measured_max.unwrap_or(maximum);
        Ok(())
    }

    fn definite_containing_width(&self, index: usize) -> Option<f32> {
        self.containing_size(index).width
    }

    fn inline_flex_changes_width(&self, node: &Node) -> bool {
        !node.style.position.is_out_of_flow()
            && node.parent.is_some_and(|parent| {
                let parent = &self.nodes[usize::from(parent)].style;
                parent.display == Display::Flex
                    && matches!(
                        parent.flex_direction,
                        FlexDirection::Row | FlexDirection::RowReverse
                    )
            })
            && (node.style.flex_grow != 0.0
                || node.style.flex_shrink != 0.0
                || !node.style.flex_basis.is_auto())
    }

    fn resolve_block_bound(&mut self, index: usize, input: LayoutInput) -> Result<(), EngineError> {
        let node = &self.nodes[index];
        let bound = node.bounds.block;
        let style = &node.style;
        let unresolved =
            || unsupported("EPUB CSS intrinsic block bounds require a definite inline size");
        let flex_width = self.flex_widths.get(index).copied().flatten();
        if self.inline_flex_changes_width(node) && flex_width.is_none() {
            return Err(unresolved());
        }
        let basis = if let (Some(_), Some(parent)) = (flex_width, node.parent) {
            self.geometry[usize::from(parent)].content
        } else {
            self.containing_size(index)
        };
        if style.aspect_ratio.is_some() && !style.item_is_replaced {
            return Err(unsupported(
                "EPUB CSS intrinsic block bounds on non-replaced aspect ratios",
            ));
        }
        for bound in [bound.min, bound.max].into_iter().flatten() {
            if let IntrinsicBound::FitContent(limit) = bound {
                if !matches!(style.display, Display::Block | Display::FlowRoot) {
                    return Err(unsupported(
                        "EPUB CSS functional block bounds require an ordinary block box",
                    ));
                }
                // Both intrinsic block sizes use the same automatic height in this profile.
                // Still require a definite argument before applying the fit-content formula.
                let basis = basis.height;
                if limit
                    .maybe_resolve(basis, |handle, basis| self.calculations.at(handle, basis))
                    .is_none()
                {
                    return Err(unsupported(
                        "EPUB CSS functional block bounds require a definite containing block size",
                    ));
                }
            }
        }
        let basis = basis.width;
        let calculate = |handle, basis| self.calculations.at(handle, basis);
        let edge = |length: taffy::LengthPercentage| {
            if basis.is_none() && !matches!(length.expand(), ExpandedLengthPercentage::Length(_)) {
                return Err(unsupported(
                    "EPUB CSS intrinsic block bounds require fixed padding and borders or a definite containing width",
                ));
            }
            Ok(length.resolve_or_zero(basis, calculate))
        };
        let inline_edges = edge(style.padding.left)?
            + edge(style.padding.right)?
            + edge(style.border.left)?
            + edge(style.border.right)?;
        let block_edges = edge(style.padding.top)?
            + edge(style.padding.bottom)?
            + edge(style.border.top)?
            + edge(style.border.bottom)?;
        let width = flex_width
            .or_else(|| {
                outer_size(
                    style,
                    AbsoluteAxis::Horizontal,
                    inline_edges,
                    basis,
                    self.auto_outer_inline(index, basis),
                    calculate,
                )
            })
            .ok_or_else(unresolved)?;
        if let Some(area) = self.grid_areas.get_mut(index).and_then(Option::as_mut) {
            area.block_probe_width = Some(width);
        }
        let height_adjust = if style.box_sizing == BoxSizing::ContentBox {
            block_edges
        } else {
            0.0
        };
        let node = &mut self.nodes[index];
        let style = &mut node.style;
        let height = std::mem::replace(&mut style.size.height, Dimension::auto());
        let minimum = std::mem::replace(&mut style.min_size.height, LengthPercentageAuto::auto());
        let maximum = std::mem::replace(&mut style.max_size.height, LengthPercentageAuto::auto());
        node.cache.clear();

        self.fit_probes += 1;
        let natural = self
            .place_node(
                NodeId::from(index),
                LayoutInput {
                    axis: RequestedAxis::Vertical,
                    known_dimensions: Size {
                        width: Some(width),
                        height: None,
                    },
                    known_dimensions_are_definite: Size {
                        width: true,
                        height: false,
                    },
                    parent_size: Size {
                        width: basis,
                        height: None,
                    },
                    available_space: Size {
                        width: AvailableSpace::Definite(width),
                        height: AvailableSpace::MaxContent,
                    },
                    ..input
                },
                None,
            )
            .size
            .height;
        let used = LengthPercentageAuto::length((natural - height_adjust).max(0.0));
        let node = &mut self.nodes[index];
        node.cache.clear();
        node.style.size.height = height;
        node.style.min_size.height = if bound.min.is_some() { used } else { minimum };
        node.style.max_size.height = if bound.max.is_some() { used } else { maximum };
        Ok(())
    }

    fn preferred_width(&mut self, id: NodeId, input: LayoutInput) -> Option<f32> {
        if input.known_dimensions.width.is_some()
            || input.run_mode != RunMode::ComputeSize
            || input.axis == RequestedAxis::Horizontal && !input.available_space.width.is_definite()
        {
            return None;
        }
        let style = &self.nodes[usize::from(id)].style;
        let basis = input.parent_size.width;
        let (limit, functional) = match style.size.width.expand() {
            ExpandedDimension::MinContent if input.axis != RequestedAxis::Horizontal => {
                return Some(self.probe_width(id, input, AvailableSpace::MinContent));
            }
            ExpandedDimension::MaxContent if input.axis != RequestedAxis::Horizontal => {
                return Some(self.probe_width(id, input, AvailableSpace::MaxContent));
            }
            ExpandedDimension::FitContent => (input.available_space.width.into_option()?, false),
            ExpandedDimension::FitContentPx(points) => (points, true),
            ExpandedDimension::FitContentPercent(ratio) => (basis? * ratio, true),
            ExpandedDimension::FitContentCalc(handle) => {
                (self.calculations.at(handle, basis?), true)
            }
            _ => return None,
        };
        // Functional limits use the selected box-sizing edge. Both Taffy's
        // available space and the measured intrinsic extents are outer sizes.
        let limit = limit
            + if functional && style.box_sizing == BoxSizing::ContentBox {
                let resolve = |length: taffy::LengthPercentage| match length.expand() {
                    ExpandedLengthPercentage::Length(points) => points,
                    ExpandedLengthPercentage::Percent(ratio) => basis.unwrap_or(0.0) * ratio,
                    ExpandedLengthPercentage::Calc(handle) => {
                        basis.map_or(0.0, |basis| self.calculations.at(handle, basis))
                    }
                };
                resolve(style.padding.left)
                    + resolve(style.padding.right)
                    + resolve(style.border.left)
                    + resolve(style.border.right)
            } else {
                0.0
            };
        // Wrapping at the limit alone returns the width of the widest wrapped
        // line. The box uses the limit itself when it lies between min and max.
        let minimum = self.probe_width(id, input, AvailableSpace::MinContent);
        let maximum = self.probe_width(id, input, AvailableSpace::MaxContent);
        Some(limit.min(maximum).max(minimum))
    }

    fn probe_width(&mut self, id: NodeId, input: LayoutInput, width: AvailableSpace) -> f32 {
        self.fit_probes += 1;
        if self.fit_probes > MAX_FIT_PROBES {
            return 0.0;
        }
        self.place_node(
            id,
            LayoutInput {
                axis: RequestedAxis::Horizontal,
                available_space: Size {
                    width,
                    ..input.available_space
                },
                ..input
            },
            None,
        )
        .size
        .width
    }

    fn place_node(
        &mut self,
        id: NodeId,
        input: LayoutInput,
        block: Option<&mut BlockContext<'_>>,
    ) -> LayoutOutput {
        if self.probing_grid {
            return LayoutOutput::from_outer_size(
                input.known_dimensions.map(|size| size.unwrap_or(0.0)),
            );
        }
        if input.run_mode == RunMode::PerformHiddenLayout {
            return compute_hidden_layout(self, id);
        }
        if self.grid_limit || self.fit_probes > MAX_FIT_PROBES {
            return LayoutOutput::HIDDEN;
        }
        compute_cached_layout(self, id, input, |tree, id, mut input| {
            if let Some(width) = tree.preferred_width(id, input) {
                if input.axis == RequestedAxis::Horizontal {
                    return LayoutOutput::from_outer_size(Size { width, height: 0.0 });
                }
                input.known_dimensions.width = Some(width);
                input.known_dimensions_are_definite.width = false;
            }
            let index = usize::from(id);
            let node = &tree.nodes[index];
            let mut output = match (node.style.display, node.children.is_empty()) {
                (Display::None, _) => compute_hidden_layout(tree, id),
                (Display::Block, false) => compute_block_layout(tree, id, input, block),
                (Display::FlowRoot, false) => compute_block_layout(tree, id, input, None),
                (Display::Flex, false) => compute_flexbox_layout(tree, id, input),
                (Display::Grid, false) => {
                    if !grid_budget::permits(&node.style, input, tree.calculations) {
                        tree.grid_limit = true;
                        return LayoutOutput::HIDDEN;
                    }
                    compute_grid_layout(tree, id, input)
                }
                (_, true) => (tree.measure)(
                    input,
                    index,
                    &node.style,
                    node.bounds.has_inline() || node.bounds.has_block(),
                ),
            };
            if input.run_mode == RunMode::PerformLayout {
                compute_oof_layout(tree, id, &mut output);
            }
            output
        })
    }
}

impl<F> TraversePartialTree for View<'_, F> {
    type ChildIter<'a>
        = std::iter::Copied<std::slice::Iter<'a, NodeId>>
    where
        Self: 'a;

    fn child_ids(&self, id: NodeId) -> Self::ChildIter<'_> {
        self.nodes[usize::from(id)].children.iter().copied()
    }

    fn child_count(&self, id: NodeId) -> usize {
        self.nodes[usize::from(id)].children.len()
    }

    fn get_child_id(&self, id: NodeId, index: usize) -> NodeId {
        self.nodes[usize::from(id)].children[index]
    }
}

impl<F: FnMut(LayoutInput, usize, &Style, bool) -> LayoutOutput> LayoutPartialTree for View<'_, F> {
    type CoreContainerStyle<'a>
        = &'a Style
    where
        Self: 'a;
    type CustomIdent = String;

    fn get_core_container_style(&self, id: NodeId) -> Self::CoreContainerStyle<'_> {
        &self.nodes[usize::from(id)].style
    }

    fn resolve_calc_value(&self, handle: CalcHandle, basis: f32) -> f32 {
        calculated_length(self.calculations, handle, basis)
    }

    fn set_unrounded_layout(&mut self, id: NodeId, layout: &Layout) {
        self.nodes[usize::from(id)].layout = *layout;
    }

    fn compute_child_layout(&mut self, id: NodeId, input: LayoutInput) -> LayoutOutput {
        self.place_node(id, input, None)
    }
}

impl<F: FnMut(LayoutInput, usize, &Style, bool) -> LayoutOutput> LayoutContainingBlock
    for View<'_, F>
{
    type OofItemStyle<'a>
        = &'a Style
    where
        Self: 'a;

    fn get_oof_item_style(&self, id: NodeId) -> Self::OofItemStyle<'_> {
        self.get_core_container_style(id)
    }

    fn clear_hoisted_children(&mut self, id: NodeId) {
        self.nodes[usize::from(id)].hoisted.clear();
    }

    fn add_hoisted_children(&mut self, id: NodeId, children: &[NodeId]) {
        self.nodes[usize::from(id)]
            .hoisted
            .extend_from_slice(children);
    }

    fn get_detailed_layout_info(&self, id: NodeId) -> &DetailedLayoutInfo {
        &self.nodes[usize::from(id)].details
    }
}

impl<F> CacheTree for View<'_, F> {
    fn cache_get(&self, id: NodeId, input: &LayoutInput) -> Option<LayoutOutput> {
        self.nodes[usize::from(id)].cache.get(input)
    }

    fn cache_store(&mut self, id: NodeId, input: &LayoutInput, output: LayoutOutput) {
        self.nodes[usize::from(id)].cache.store(input, output);
    }

    fn cache_clear(&mut self, id: NodeId) {
        self.nodes[usize::from(id)].cache.clear();
    }
}

impl<F: FnMut(LayoutInput, usize, &Style, bool) -> LayoutOutput> LayoutBlockContainer
    for View<'_, F>
{
    type BlockContainerStyle<'a>
        = &'a Style
    where
        Self: 'a;
    type BlockItemStyle<'a>
        = &'a Style
    where
        Self: 'a;

    fn get_block_container_style(&self, id: NodeId) -> Self::BlockContainerStyle<'_> {
        self.get_core_container_style(id)
    }

    fn get_block_child_style(&self, id: NodeId) -> Self::BlockItemStyle<'_> {
        self.get_core_container_style(id)
    }

    fn compute_block_child_layout(
        &mut self,
        id: NodeId,
        input: LayoutInput,
        block: Option<&mut BlockContext<'_>>,
    ) -> LayoutOutput {
        self.place_node(id, input, block)
    }
}

impl<F: FnMut(LayoutInput, usize, &Style, bool) -> LayoutOutput> LayoutFlexboxContainer
    for View<'_, F>
{
    type FlexboxContainerStyle<'a>
        = &'a Style
    where
        Self: 'a;
    type FlexboxItemStyle<'a>
        = &'a Style
    where
        Self: 'a;

    fn get_flexbox_container_style(&self, id: NodeId) -> Self::FlexboxContainerStyle<'_> {
        self.get_core_container_style(id)
    }

    fn get_flexbox_child_style(&self, id: NodeId) -> Self::FlexboxItemStyle<'_> {
        self.get_core_container_style(id)
    }
}

impl<F: FnMut(LayoutInput, usize, &Style, bool) -> LayoutOutput> LayoutGridContainer
    for View<'_, F>
{
    type GridContainerStyle<'a>
        = &'a Style
    where
        Self: 'a;
    type GridItemStyle<'a>
        = &'a Style
    where
        Self: 'a;

    fn get_grid_container_style(&self, id: NodeId) -> Self::GridContainerStyle<'_> {
        self.get_core_container_style(id)
    }

    fn get_grid_child_style(&self, id: NodeId) -> Self::GridItemStyle<'_> {
        self.get_core_container_style(id)
    }

    fn set_detailed_grid_info(&mut self, id: NodeId, details: DetailedGridInfo) {
        self.nodes[usize::from(id)].details = DetailedLayoutInfo::Grid(Box::new(details));
    }
}
