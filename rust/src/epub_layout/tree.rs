//! Feed the chapter's box tree and calculated lengths to Taffy's layout algorithms.
use taffy::{
    compute_block_layout, compute_cached_layout, compute_flexbox_layout, compute_grid_layout,
    compute_hidden_layout, compute_oof_layout, compute_root_layout,
    geometry::Size,
    style::{
        AvailableSpace, BoxSizing, CalcHandle, Dimension, Display, ExpandedDimension,
        ExpandedLengthPercentage, ExpandedLengthPercentageAuto, FlexDirection,
        LengthPercentageAuto, Style,
    },
    BlockContext, Cache, CacheTree, DetailedGridInfo, DetailedLayoutInfo, Layout,
    LayoutBlockContainer, LayoutContainingBlock, LayoutFlexboxContainer, LayoutGridContainer,
    LayoutInput, LayoutOutput, LayoutPartialTree, NodeId, RequestedAxis, RunMode, SizingMode,
    TraversePartialTree,
};

use crate::epub::{
    css::CalculationTable,
    limited,
    style::{InlineBound, IntrinsicBounds},
    unsupported,
};
use crate::EngineError;

#[path = "grid_budget.rs"]
mod grid_budget;

#[derive(Default)]
pub(super) struct Tree {
    nodes: Vec<Node>,
}

struct Node {
    style: Style,
    bounds: IntrinsicBounds,
    parent: Option<NodeId>,
    has_relative_math: bool,
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
        for child in &children {
            self.nodes[usize::from(*child)].parent = Some(id);
        }
        self.nodes.push(Node {
            style,
            bounds,
            parent: None,
            has_relative_math,
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
        let mut view = View {
            nodes: &mut self.nodes,
            calculations,
            measure,
            fit_probes: 0,
            grid_limit: false,
        };
        view.resolve_bounds()?;
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

struct View<'a, F> {
    nodes: &'a mut [Node],
    calculations: &'a CalculationTable,
    measure: F,
    fit_probes: usize,
    grid_limit: bool,
}

const MAX_FIT_PROBES: usize = 4_096;

impl<F: FnMut(LayoutInput, usize, &Style, bool) -> LayoutOutput> View<'_, F> {
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
        input: LayoutInput,
    ) -> Result<(), EngineError> {
        let node = &mut self.nodes[index];
        let bounds = node.bounds.inline;
        let style = &mut node.style;
        if node.has_relative_math {
            return Err(unsupported(
                "EPUB CSS intrinsic width bounds on percentage MathML",
            ));
        }
        let edges = [
            style.padding.left,
            style.padding.right,
            style.border.left,
            style.border.right,
        ];
        let absolute_edges = edges
            .into_iter()
            .map(|edge| match edge.expand() {
                ExpandedLengthPercentage::Length(points) => Some(points),
                _ => None,
            })
            .sum::<Option<f32>>();
        let subtract_padding = if style.box_sizing == BoxSizing::ContentBox {
            // Intrinsic probes have no percentage basis, so only fixed
            // edges are included in their native border-box result.
            edges
                .into_iter()
                .filter_map(|edge| match edge.expand() {
                    ExpandedLengthPercentage::Length(points) => Some(points),
                    _ => None,
                })
                .sum()
        } else {
            if absolute_edges.is_none() {
                return Err(unsupported(
                    "EPUB CSS intrinsic width bounds with relative border-box padding",
                ));
            }
            0.0
        };
        let width = std::mem::replace(&mut style.size.width, Dimension::auto());
        let minimum = std::mem::replace(&mut style.min_size.width, LengthPercentageAuto::auto());
        let maximum = std::mem::replace(&mut style.max_size.width, LengthPercentageAuto::auto());
        let id = NodeId::from(index);
        let choices = [bounds.min, bounds.max];
        let mut probe = |space| (self.probe_width(id, input, space) - subtract_padding).max(0.0);
        let minimum_width = if choices.iter().any(|bound| {
            matches!(
                bound,
                Some(InlineBound::Minimum | InlineBound::FitContent(_))
            )
        }) {
            probe(AvailableSpace::MinContent)
        } else {
            0.0
        };
        let maximum_width = if choices.iter().any(|bound| {
            matches!(
                bound,
                Some(InlineBound::Maximum | InlineBound::FitContent(_))
            )
        }) {
            probe(AvailableSpace::MaxContent).max(minimum_width)
        } else {
            minimum_width
        };
        let used_bound = |bound| {
            let width = match bound {
                InlineBound::Minimum => minimum_width,
                InlineBound::Maximum => maximum_width,
                InlineBound::FitContent(limit) => limit.min(maximum_width).max(minimum_width),
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

    fn resolve_block_bound(&mut self, index: usize, input: LayoutInput) -> Result<(), EngineError> {
        let node = &self.nodes[index];
        let bound = node.bounds.block;
        let style = &node.style;
        let unresolved =
            || unsupported("EPUB CSS intrinsic block bounds require a definite inline size");
        let ExpandedDimension::Length(width) = style.size.width.expand() else {
            return Err(unresolved());
        };
        let width_limit = |length: LengthPercentageAuto, initial| match length.expand() {
            ExpandedLengthPercentageAuto::Length(width) => Ok(width),
            ExpandedLengthPercentageAuto::Auto => Ok(initial),
            _ => Err(unresolved()),
        };
        let width = width
            .min(width_limit(style.max_size.width, f32::INFINITY)?)
            .max(width_limit(style.min_size.width, 0.0)?);
        let horizontal_flex = node.parent.is_some_and(|parent| {
            let parent = &self.nodes[usize::from(parent)].style;
            parent.display == Display::Flex
                && matches!(
                    parent.flex_direction,
                    FlexDirection::Row | FlexDirection::RowReverse
                )
        });
        if !style.position.is_out_of_flow()
            && horizontal_flex
            && (style.flex_grow != 0.0 || style.flex_shrink != 0.0 || !style.flex_basis.is_auto())
        {
            return Err(unresolved());
        }
        if style.aspect_ratio.is_some() && !style.item_is_replaced {
            return Err(unsupported(
                "EPUB CSS intrinsic block bounds on non-replaced aspect ratios",
            ));
        }
        let edge = |length: taffy::LengthPercentage| match length.expand() {
            ExpandedLengthPercentage::Length(points) => Ok(points),
            _ => Err(unsupported(
                "EPUB CSS intrinsic block bounds require fixed padding and borders",
            )),
        };
        let inline_edges = edge(style.padding.left)?
            + edge(style.padding.right)?
            + edge(style.border.left)?
            + edge(style.border.right)?;
        let block_edges = edge(style.padding.top)?
            + edge(style.padding.bottom)?
            + edge(style.border.top)?
            + edge(style.border.bottom)?;
        let (width, height_adjust) = if style.box_sizing == BoxSizing::ContentBox {
            (width + inline_edges, block_edges)
        } else {
            (width.max(inline_edges), 0.0)
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
        node.style.min_size.height = if bound.min { used } else { minimum };
        node.style.max_size.height = if bound.max { used } else { maximum };
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
