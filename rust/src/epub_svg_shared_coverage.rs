//! Attribute a shared native affine replay when source pixel supports stay apart.
use std::collections::HashMap;

use resvg::{tiny_skia, tiny_skia::Transform, usvg};
use wordcraft_geom::Rect;

use super::{affine_leaf, contains_filter, CaptureGeometry, RasterAxes};

const MAX_BRANCHES: usize = 8;
const MAX_LAYERS: usize = 4;
const MAX_COORD: f32 = 16_384.0;

pub(in crate::epub::svg) struct Sources(HashMap<*const usvg::Node, Rect>);

impl Sources {
    pub(in crate::epub::svg) fn bounds(&self, node: &usvg::Node) -> Option<Rect> {
        self.0.get(&std::ptr::from_ref(node)).copied()
    }
}

pub(super) fn common_stack(mut group: &usvg::Group) -> Option<&usvg::Group> {
    let mut filters = 0;
    loop {
        if !group.filters().is_empty() {
            filters += 1;
            if filters > MAX_LAYERS || !pointwise_offsets(group) {
                return None;
            }
        }
        match group.children() {
            [usvg::Node::Group(child)] => group = child,
            branches if (2..=MAX_BRANCHES).contains(&branches.len()) && filters != 0 => {
                return branches
                    .iter()
                    .all(|branch| source(branch).is_some())
                    .then_some(group);
            }
            _ => return None,
        }
    }
}

fn pointwise_offsets(group: &usvg::Group) -> bool {
    use usvg::filter::{ColorMatrixKind, Kind, TransferFunction};

    let [filter] = group.filters() else {
        return false;
    };
    // A translated pixel can only carry the source it already belonged to.
    // RGB-only stages may change its color without changing its support.
    filter
        .primitives()
        .iter()
        .all(|primitive| match primitive.kind() {
            Kind::Offset(_) => true,
            Kind::ColorMatrix(matrix) => match matrix.kind() {
                ColorMatrixKind::Saturate(_) | ColorMatrixKind::HueRotate(_) => true,
                ColorMatrixKind::Matrix(values) => values[15..20] == [0.0, 0.0, 0.0, 1.0, 0.0],
                ColorMatrixKind::LuminanceToAlpha => false,
            },
            Kind::ComponentTransfer(transfer) => {
                matches!(transfer.func_a(), TransferFunction::Identity)
            }
            _ => false,
        })
}

fn source(mut node: &usvg::Node) -> Option<&usvg::Node> {
    while let usvg::Node::Group(group) = node {
        let [child] = group.children() else {
            return None;
        };
        if !group.filters().is_empty() || !bounded(group.transform()) {
            return None;
        }
        node = child;
    }
    match node {
        usvg::Node::Text(text) if contains_filter(text.flattened()) => None,
        usvg::Node::Image(image) if !matches!(image.kind(), usvg::ImageKind::PNG(_)) => None,
        _ => affine_leaf(node).then_some(node),
    }
}

fn bounded(transform: Transform) -> bool {
    [transform.sx, transform.kx, transform.ky, transform.sy]
        .into_iter()
        .all(|value| value.abs() <= 64.0)
        && [transform.tx, transform.ty]
            .into_iter()
            .all(|value| value.abs() <= MAX_COORD)
}

fn source_ink(node: &usvg::Node) -> Option<tiny_skia::Rect> {
    if !bounded(node.abs_transform()) {
        return None;
    }
    match node {
        usvg::Node::Path(path) => {
            let hull = path.data().bounds();
            if ![hull.left(), hull.top(), hull.right(), hull.bottom()]
                .into_iter()
                .all(|value| value.abs() <= MAX_COORD)
            {
                return None;
            }
            // Expand in the path's local coordinates before skew or scale.
            // This encloses ellipse/curve control points, displaced dash
            // caps and miter joins even with a strongly anisotropic stroke.
            let pad = path.stroke().map_or(0.0, |stroke| {
                stroke.width().get() * stroke.miterlimit().get().max(2.0)
            });
            tiny_skia::Rect::from_ltrb(
                hull.left() - pad,
                hull.top() - pad,
                hull.right() + pad,
                hull.bottom() + pad,
            )?
            .transform(path.abs_transform())
        }
        usvg::Node::Text(text) => group_ink(text.flattened()),
        usvg::Node::Group(group) => group_ink(group),
        usvg::Node::Image(image) => Some(image.abs_bounding_box()),
    }
}

fn group_ink(group: &usvg::Group) -> Option<tiny_skia::Rect> {
    let (first, remaining) = group.children().split_first()?;
    remaining
        .iter()
        .try_fold(source_ink(first)?, |ink, node| ink.join(&source_ink(node)?))
}

impl CaptureGeometry {
    pub(in crate::epub::svg) fn shared_sources(
        &self,
        mut group: &usvg::Group,
        mut axes: RasterAxes,
    ) -> Option<Sources> {
        let split = common_stack(group)?;
        let [sx, sy] = self.density;
        let mut pad = [2.0, 2.0];
        let mut shift = [0.0, 0.0];
        loop {
            if !bounded(group.transform()) || !bounded(group.abs_transform()) {
                return None;
            }
            axes = axes.enter(group)?;
            if !bounded(axes.local) {
                return None;
            }
            if !group.filters().is_empty() {
                let map = axes.page;
                let x = map.sx.abs() + map.kx.abs();
                let y = map.ky.abs() + map.sy.abs();
                let determinant = map.sx * map.sy - map.kx * map.ky;
                if !(0.125..=8.0).contains(&determinant.abs()) || x > 8.0 || y > 8.0 {
                    return None;
                }
                let [dx, dy] = self.shift(group, axes)?;
                shift[0] += dx * sx;
                shift[1] += dy * sy;
                // Map the native layer's bilinear and antialias support back
                // into page pixels. Three pixels per plane also cover roundoff
                // from the checked affine decomposition and integer origins.
                pad[0] += 3.0 * x;
                pad[1] += 3.0 * y;
            }
            if std::ptr::eq(group, split) {
                break;
            }
            let [usvg::Node::Group(child)] = group.children() else {
                return None;
            };
            group = child;
        }

        let mut sources = HashMap::new();
        let mut footprints = Vec::<[f32; 4]>::new();
        for branch in split.children() {
            let ink = source_ink(source(branch)?)?;
            let [left, top, right, bottom] = [
                ink.left() * sx,
                ink.top() * sy,
                ink.right() * sx,
                ink.bottom() * sy,
            ];
            if ![left, top, right, bottom, shift[0], shift[1]]
                .into_iter()
                .all(|value| value.abs() <= MAX_COORD)
            {
                return None;
            }
            let footprint = [
                (left + shift[0] - pad[0] - 1.0).floor(),
                (top + shift[1] - pad[1] - 1.0).floor(),
                (right + shift[0] + pad[0] + 1.0).ceil(),
                (bottom + shift[1] + pad[1] + 1.0).ceil(),
            ];
            let [left, top, right, bottom] = footprint;
            // Earlier stages have the same relative source positions and
            // fewer resampling passes. A gap after summing all support bounds
            // forbids either source from contributing to any sample of the other.
            if footprints
                .iter()
                .any(|&[l, t, r, b]| left <= r && right >= l && top <= b && bottom >= t)
            {
                return None;
            }
            footprints.push(footprint);
            let bounds = Rect::new(
                left / sx,
                top / sy,
                (right - left) / sx,
                (bottom - top) / sy,
            );
            let mut node = branch;
            loop {
                sources.insert(std::ptr::from_ref(node), bounds);
                let usvg::Node::Group(group) = node else {
                    break;
                };
                node = group.children().first()?;
            }
        }
        Some(Sources(sources))
    }
}
