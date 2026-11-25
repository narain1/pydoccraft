//! Feed the chapter's box tree and calculated lengths to Taffy's layout algorithms.
use taffy::{
    compute_block_layout, compute_cached_layout, compute_flexbox_layout, compute_grid_layout,
    compute_hidden_layout, compute_oof_layout, compute_root_layout,
    geometry::Size,
    style::{
        AvailableSpace, BoxSizing, CalcHandle, Display, ExpandedDimension,
        ExpandedLengthPercentage, Style,
    },
    BlockContext, Cache, CacheTree, DetailedGridInfo, DetailedLayoutInfo, Layout,
    LayoutBlockContainer, LayoutContainingBlock, LayoutFlexboxContainer, LayoutGridContainer,
    LayoutInput, LayoutOutput, LayoutPartialTree, NodeId, RequestedAxis, RunMode,
    TraversePartialTree,
};

use crate::epub::{css::CalculationTable, limited};
use crate::EngineError;

#[derive(Default)]
pub(super) struct Tree {
    nodes: Vec<Node>,
}

struct Node {
    style: Style,
    children: Vec<NodeId>,
    layout: Layout,
    cache: Cache,
    hoisted: Vec<NodeId>,
    details: DetailedLayoutInfo,
}

impl Tree {
    pub(super) fn add(&mut self, style: Style, children: Vec<NodeId>) -> NodeId {
        let id = NodeId::from(self.nodes.len());
        self.nodes.push(Node {
            style,
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
        measure: impl FnMut(LayoutInput, usize, &Style) -> LayoutOutput,
    ) -> Result<(), EngineError> {
        let mut view = View {
            nodes: &mut self.nodes,
            calculations,
            measure,
            fit_probes: 0,
        };
        compute_root_layout(&mut view, root, space);
        if view.fit_probes > MAX_FIT_PROBES {
            return Err(limited(
                "EPUB CSS intrinsic box layout exceeds its sizing budget",
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
}

const MAX_FIT_PROBES: usize = 4_096;

impl<F: FnMut(LayoutInput, usize, &Style) -> LayoutOutput> View<'_, F> {
    fn fit_width(&mut self, id: NodeId, input: LayoutInput) -> Option<f32> {
        let AvailableSpace::Definite(limit) = input.available_space.width else {
            return None;
        };
        if input.known_dimensions.width.is_some() || input.run_mode != RunMode::ComputeSize {
            return None;
        }
        let style = &self.nodes[usize::from(id)].style;
        let limit = match style.size.width.expand() {
            ExpandedDimension::FitContent => limit,
            ExpandedDimension::FitContentPx(_)
            | ExpandedDimension::FitContentPercent(_)
            | ExpandedDimension::FitContentCalc(_) => {
                // A functional limit uses the chosen box-sizing edge. Taffy's
                // available space and the measured intrinsic extents are outer sizes.
                let resolve = |length: taffy::LengthPercentage| {
                    let basis = input.parent_size.width.unwrap_or(0.0);
                    match length.expand() {
                        ExpandedLengthPercentage::Length(points) => points,
                        ExpandedLengthPercentage::Percent(ratio) => basis * ratio,
                        ExpandedLengthPercentage::Calc(handle) => {
                            self.calculations.at(handle, basis)
                        }
                    }
                };
                limit
                    + if style.box_sizing == BoxSizing::ContentBox {
                        resolve(style.padding.left)
                            + resolve(style.padding.right)
                            + resolve(style.border.left)
                            + resolve(style.border.right)
                    } else {
                        0.0
                    }
            }
            _ => return None,
        };
        self.fit_probes += 1;
        if self.fit_probes > MAX_FIT_PROBES {
            return Some(0.0);
        }
        // Wrapping at the limit alone returns the width of the widest wrapped
        // line. The box uses the limit itself when it lies between min and max.
        let mut probe = |width| {
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
        };
        let minimum = probe(AvailableSpace::MinContent);
        let maximum = probe(AvailableSpace::MaxContent);
        Some(limit.min(maximum).max(minimum))
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
        if self.fit_probes > MAX_FIT_PROBES {
            return LayoutOutput::HIDDEN;
        }
        compute_cached_layout(self, id, input, |tree, id, mut input| {
            if let Some(width) = tree.fit_width(id, input) {
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
                (Display::Grid, false) => compute_grid_layout(tree, id, input),
                (_, true) => (tree.measure)(input, index, &node.style),
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

impl<F: FnMut(LayoutInput, usize, &Style) -> LayoutOutput> LayoutPartialTree for View<'_, F> {
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

impl<F: FnMut(LayoutInput, usize, &Style) -> LayoutOutput> LayoutContainingBlock for View<'_, F> {
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

impl<F: FnMut(LayoutInput, usize, &Style) -> LayoutOutput> LayoutBlockContainer for View<'_, F> {
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

impl<F: FnMut(LayoutInput, usize, &Style) -> LayoutOutput> LayoutFlexboxContainer for View<'_, F> {
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

impl<F: FnMut(LayoutInput, usize, &Style) -> LayoutOutput> LayoutGridContainer for View<'_, F> {
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
