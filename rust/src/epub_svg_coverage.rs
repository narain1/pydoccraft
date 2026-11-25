//! Query the native renderer's alpha coverage for clipped SVG annotations.
use std::collections::HashMap;

use resvg::{tiny_skia, usvg};
use wordcraft_geom::Rect;

use super::{effects, limited, shifted, PaintedNode};
use crate::{
    epub::{charge, rendition::intersection},
    EngineError,
};

const MAX_TILE_PIXELS: usize = 16 * 1024 * 1024;
const MAX_COVERAGE_PIXELS: usize = 32 * 1024 * 1024;
const MAX_COVERAGE_WORK: usize = 128 * 1024 * 1024;
const MAX_DRAW_VISITS: usize = 250_000;
const MAX_DRAW_DEPTH: usize = 256;

pub(super) struct Coverage<'tree> {
    tree: &'tree usvg::Tree,
    density: [f32; 2],
    raster: [u32; 2],
    viewport: Rect,
    tiles: HashMap<*const usvg::Node, Option<Tile>>,
    saved: usize,
    work: usize,
    draw_visits: usize,
    validated_effects: bool,
}

pub(super) enum Target {
    Object,
    PaintedObject,
    Glyph(Option<Rect>),
}

struct Tile {
    bounds: Bounds,
    alpha: tiny_skia::Mask,
}

#[derive(Clone, Copy)]
struct Bounds {
    left: u32,
    top: u32,
    right: u32,
    bottom: u32,
}

impl Bounds {
    fn from_rect(rect: Rect, [sx, sy]: [f32; 2]) -> Option<Self> {
        if rect.w <= 0.0 || rect.h <= 0.0 {
            return None;
        }
        let bounds = Self {
            left: (rect.x * sx).floor() as u32,
            top: (rect.y * sy).floor() as u32,
            right: (rect.right() * sx).ceil() as u32,
            bottom: (rect.bottom() * sy).ceil() as u32,
        };
        (bounds.left < bounds.right && bounds.top < bounds.bottom).then_some(bounds)
    }

    fn intersection(self, other: Self) -> Option<Self> {
        let bounds = Self {
            left: self.left.max(other.left),
            top: self.top.max(other.top),
            right: self.right.min(other.right),
            bottom: self.bottom.min(other.bottom),
        };
        (bounds.left < bounds.right && bounds.top < bounds.bottom).then_some(bounds)
    }

    fn width(self) -> u32 {
        self.right - self.left
    }

    fn height(self) -> u32 {
        self.bottom - self.top
    }

    fn pixels(self) -> usize {
        (self.width() as usize).saturating_mul(self.height() as usize)
    }

    fn empty() -> Self {
        Self {
            left: u32::MAX,
            top: u32::MAX,
            right: 0,
            bottom: 0,
        }
    }

    fn include(&mut self, x: u32, y: u32) {
        self.left = self.left.min(x);
        self.top = self.top.min(y);
        self.right = self.right.max(x + 1);
        self.bottom = self.bottom.max(y + 1);
    }

    fn is_empty(self) -> bool {
        self.left >= self.right || self.top >= self.bottom
    }

    fn as_rect(self, density: [f32; 2]) -> Rect {
        Rect::new(
            self.left as f32 / density[0],
            self.top as f32 / density[1],
            self.width() as f32 / density[0],
            self.height() as f32 / density[1],
        )
    }

    fn trim(self, ink: Self, rect: Rect, [sx, sy]: [f32; 2]) -> Rect {
        // Preserve the normal em box on sides where no painted samples are
        // lost. Keeping it avoids injecting word gaps and moving baselines.
        let left = if self.left > ink.left {
            rect.x.max(self.left as f32 / sx)
        } else {
            rect.x
        };
        let top = if self.top > ink.top {
            rect.y.max(self.top as f32 / sy)
        } else {
            rect.y
        };
        let right = if self.right < ink.right {
            rect.right().min(self.right as f32 / sx)
        } else {
            rect.right()
        };
        let bottom = if self.bottom < ink.bottom {
            rect.bottom().min(self.bottom as f32 / sy)
        } else {
            rect.bottom()
        };
        Rect::new(left, top, right - left, bottom - top)
    }
}

impl Tile {
    fn sample(&self, x: u32, y: u32) -> bool {
        if x < self.bounds.left
            || x >= self.bounds.right
            || y < self.bounds.top
            || y >= self.bounds.bottom
        {
            return false;
        }
        let offset = (y - self.bounds.top) as usize * self.bounds.width() as usize
            + (x - self.bounds.left) as usize;
        self.alpha.data()[offset] != 0
    }
}

impl<'tree> Coverage<'tree> {
    pub(super) fn new(tree: &'tree usvg::Tree, density: [f32; 2], raster: [u32; 2]) -> Self {
        Self {
            tree,
            density,
            raster,
            viewport: Rect::new(0.0, 0.0, tree.size().width(), tree.size().height()),
            tiles: HashMap::new(),
            saved: 0,
            work: 0,
            draw_visits: 0,
            validated_effects: false,
        }
    }

    pub(super) fn refine(
        &mut self,
        paint: PaintedNode<'tree>,
        effects: &[PaintedNode<'tree>],
        rect: Rect,
        clip: Rect,
        target: Target,
    ) -> Result<Option<Rect>, EngineError> {
        let rect = shifted(rect, paint.geometry_shift);
        if effects.is_empty() && !paint.sample_geometry && !matches!(target, Target::PaintedObject)
        {
            return Ok(Some(rect));
        }
        let rect = intersection(clip, rect);
        let footprint = if paint.exact_coverage {
            // Use native antialiasing throughout the isolated raster, with
            // the verified support of this branch if it shares the replay.
            paint
                .source_bounds
                .map_or(clip, |bounds| intersection(clip, bounds))
        } else {
            match target {
                Target::Glyph(Some(ink)) => intersection(rect, shifted(ink, paint.geometry_shift)),
                _ => rect,
            }
        };
        let Some(bounds) = Bounds::from_rect(footprint, self.density) else {
            return Ok(None);
        };
        // Replay one source with its original isolated context. Cropping
        // an intermediate layer can change translated or neighboring pixels.
        let owner = if paint.uses_full_viewport || paint.exact_coverage {
            effects
                .iter()
                .find(|effect| effect.context_root)
                .copied()
                .unwrap_or(paint)
        } else {
            paint
        };
        let leaf = self.ensure_tile(owner)?;
        let mut ancestors = Vec::new();
        for effect in effects {
            if std::ptr::eq(effect.node, owner.node) {
                continue;
            }
            let key = self.ensure_tile(*effect)?;
            if self.tiles[&key].is_none() {
                return Ok(None);
            }
            ancestors.push(key);
        }
        let Some(leaf) = &self.tiles[&leaf] else {
            return Ok(None);
        };
        let Some(bounds) = bounds.intersection(leaf.bounds) else {
            return Ok(None);
        };
        charge(
            &mut self.work,
            bounds.pixels().saturating_mul(ancestors.len() + 1),
            MAX_COVERAGE_WORK,
            "EPUB SVG coverage exceeds its sample budget",
        )?;
        let tiles: Vec<_> = ancestors
            .iter()
            .filter_map(|key| self.tiles.get(key).and_then(Option::as_ref))
            .collect();
        let mut ink = Bounds::empty();
        let mut visible = Bounds::empty();
        for y in bounds.top..bounds.bottom {
            for x in bounds.left..bounds.right {
                if leaf.sample(x, y) {
                    ink.include(x, y);
                    if tiles.iter().all(|tile| tile.sample(x, y)) {
                        visible.include(x, y);
                    }
                }
            }
        }
        if paint.exact_coverage {
            return Ok((!visible.is_empty()).then(|| visible.as_rect(self.density)));
        }
        Ok((!visible.is_empty()).then(|| match target {
            Target::Object | Target::PaintedObject => {
                intersection(rect, visible.as_rect(self.density))
            }
            Target::Glyph(_) => visible.trim(ink, rect, self.density),
        }))
    }

    fn ensure_tile(&mut self, paint: PaintedNode<'tree>) -> Result<*const usvg::Node, EngineError> {
        let key = std::ptr::from_ref(paint.node);
        if !self.tiles.contains_key(&key) {
            let tile = self.render(paint)?;
            self.tiles.insert(key, tile);
        }
        Ok(key)
    }

    fn render(&mut self, paint: PaintedNode<'tree>) -> Result<Option<Tile>, EngineError> {
        let Some(bbox) = paint.node.abs_layer_bounding_box() else {
            return Ok(None);
        };
        let [sx, sy] = self.density;
        // The renderer pads isolated layers to keep their antialiased edges.
        let padding = [2.0 / sx, 2.0 / sy];
        let candidate = intersection(
            self.viewport,
            Rect::new(
                bbox.x() + paint.raster_shift[0] - padding[0],
                bbox.y() + paint.raster_shift[1] - padding[1],
                bbox.width() + 2.0 * padding[0],
                bbox.height() + 2.0 * padding[1],
            ),
        );
        let bounds = if paint.uses_full_viewport {
            // Spatial filters can read source pixels outside this tile.
            // Preserve the renderer's viewport when replaying their layers.
            Bounds {
                left: 0,
                top: 0,
                right: self.raster[0],
                bottom: self.raster[1],
            }
        } else {
            let Some(bounds) = Bounds::from_rect(candidate, self.density) else {
                return Ok(None);
            };
            bounds
        };
        if bounds.pixels() > MAX_TILE_PIXELS || bounds.width() > 8192 || bounds.height() > 8192 {
            return Err(limited("EPUB SVG coverage exceeds its tile budget"));
        }
        charge(
            &mut self.saved,
            bounds.pixels(),
            MAX_COVERAGE_PIXELS,
            "EPUB SVG coverage exceeds its raster budget",
        )?;
        charge(
            &mut self.work,
            bounds.pixels(),
            MAX_COVERAGE_WORK,
            "EPUB SVG coverage exceeds its sample budget",
        )?;
        if !self.validated_effects {
            effects::validate_raster(
                self.tree,
                tiny_skia::Transform::from_scale(sx, sy),
                self.raster,
            )?;
            self.validated_effects = true;
        }
        self.measure_draws(paint.node, 0)?;
        let mut pixmap = tiny_skia::Pixmap::new(bounds.width(), bounds.height())
            .ok_or_else(|| limited("cannot allocate EPUB SVG coverage"))?;
        let original = paint.device_parent.post_translate(
            sx * paint.raster_shift[0] - bounds.left as f32,
            sy * paint.raster_shift[1] - bounds.top as f32,
        );
        // The public node renderer subtracts its absolute bbox origin.
        // Undo that translation and check for precision loss before using
        // a replay as the exact owner of the source pixels.
        let transform = original.pre_translate(bbox.x(), bbox.y());
        if paint.exact_coverage && transform.pre_translate(-bbox.x(), -bbox.y()) != original {
            return Err(effects::geometry_error());
        }
        resvg::render_node(paint.node, transform, &mut pixmap.as_mut());
        Ok(Some(Tile {
            bounds,
            alpha: tiny_skia::Mask::from_pixmap(pixmap.as_ref(), tiny_skia::MaskType::Alpha),
        }))
    }

    // Replaying multiple nested groups can repeat work even for small raster
    // regions. Include the resources traversed by the native renderer.
    fn measure_draws(&mut self, node: &usvg::Node, depth: usize) -> Result<(), EngineError> {
        if depth >= MAX_DRAW_DEPTH {
            return Err(limited("EPUB SVG coverage exceeds its drawing depth"));
        }
        charge(
            &mut self.draw_visits,
            1,
            MAX_DRAW_VISITS,
            "EPUB SVG coverage exceeds its drawing budget",
        )?;
        if let usvg::Node::Group(group) = node {
            for primitive in group
                .filters()
                .iter()
                .flat_map(|filter| filter.primitives())
            {
                if let usvg::filter::Kind::Image(image) = primitive.kind() {
                    charge(
                        &mut self.work,
                        effects::image_decode_work(image)?,
                        MAX_COVERAGE_WORK,
                        "EPUB SVG coverage exceeds its raster image budget",
                    )?;
                }
            }
        }
        let mut measure_children = |group: &usvg::Group| {
            group
                .children()
                .iter()
                .try_for_each(|node| self.measure_draws(node, depth + 1))
        };
        if let usvg::Node::Group(group) = node {
            measure_children(group)?;
        }
        let mut measure = Ok(());
        node.subroots(|group| {
            if measure.is_ok() {
                measure = measure_children(group);
            }
        });
        measure
    }
}
