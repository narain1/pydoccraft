//! Bound the outline work needed to paint one raster region.
use std::collections::{hash_map::Entry, HashMap};

use kurbo::Rect;
use skrifa::{
    instance::NormalizedCoord,
    outline::{DrawSettings, OutlineGlyph, OutlinePen},
};

use super::{invalid, limited};
use crate::EngineError;

const OUTLINE_COMMANDS: usize = 1_000_000;
const MAX_BOUND_BYTES: usize = 8 * 1024 * 1024;

pub(in crate::epub) struct GlyphPaths {
    pub(super) remaining: usize,
    pub(super) clip: Option<OutlineClip>,
}

impl Default for GlyphPaths {
    fn default() -> Self {
        Self {
            remaining: OUTLINE_COMMANDS,
            clip: None,
        }
    }
}

impl GlyphPaths {
    pub(in crate::epub) fn set_clip(&mut self, region: crate::rendering::RasterRegion, scale: f64) {
        let rect = Rect::new(
            f64::from(region.x) / scale,
            f64::from(region.y) / scale,
            f64::from(region.x + region.width) / scale,
            f64::from(region.y + region.height) / scale,
        );
        // Use the snapped raster window with room for antialiasing and the
        // renderer's f32 transform at large chapter coordinates.
        let padding = 2.0 / scale;
        self.clip = Some(OutlineClip {
            rect: rect.inflate(padding, padding),
            fonts: HashMap::new(),
            commands: 0,
            cache_remaining: MAX_BOUND_BYTES,
        });
    }
}

#[derive(Hash, Eq, PartialEq)]
struct FontInstance {
    blob: u64,
    face: u32,
    size: u32,
    coordinates: Vec<NormalizedCoord>,
}

pub(super) struct OutlineClip {
    rect: Rect,
    fonts: HashMap<FontInstance, HashMap<u32, Option<Rect>>>,
    commands: usize,
    cache_remaining: usize,
}

impl OutlineClip {
    pub(super) fn font(
        &mut self,
        blob: u64,
        face: u32,
        size: f32,
        coordinates: &[NormalizedCoord],
        skew: f32,
        embolden: bool,
    ) -> Option<FontClip<'_>> {
        let key = FontInstance {
            blob,
            face,
            size: size.to_bits(),
            coordinates: coordinates.to_vec(),
        };
        let glyphs = match self.fonts.entry(key) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                self.cache_remaining = self
                    .cache_remaining
                    .checked_sub(96 + std::mem::size_of_val(coordinates))?;
                entry.insert(HashMap::new())
            }
        };
        let stroke = if embolden {
            f64::from((size / 30.0).max(0.1))
        } else {
            0.0
        };
        Some(FontClip {
            rect: self.rect.inflate(stroke, stroke),
            skew,
            glyphs,
            commands: &mut self.commands,
            cache_remaining: &mut self.cache_remaining,
        })
    }
}

pub(super) struct FontClip<'a> {
    rect: Rect,
    skew: f32,
    glyphs: &'a mut HashMap<u32, Option<Rect>>,
    commands: &'a mut usize,
    cache_remaining: &'a mut usize,
}

impl FontClip<'_> {
    pub(super) fn visible(
        &mut self,
        id: u32,
        outline: &OutlineGlyph<'_>,
        settings: DrawSettings<'_>,
        [x, y]: [f32; 2],
    ) -> Result<bool, EngineError> {
        let bounds = match self.glyphs.entry(id) {
            Entry::Occupied(entry) => *entry.get(),
            Entry::Vacant(entry) => {
                let Some(remaining) = self.cache_remaining.checked_sub(64) else {
                    // Paint through the original outline budget if the cache fills up.
                    return Ok(true);
                };
                *self.cache_remaining = remaining;
                let mut pen = BoundsPen {
                    rect: None,
                    commands: self.commands,
                };
                outline.draw(settings, &mut pen).map_err(invalid)?;
                if *pen.commands > OUTLINE_COMMANDS {
                    return Err(limited(
                        "EPUB clipped glyph bounds exceed the outline work budget",
                    ));
                }
                *entry.insert(pen.rect)
            }
        };
        let Some(bounds) = bounds else {
            return Ok(false);
        };
        // The control-point box contains the entire unhinted outline. The
        // extrema after x-shear come from its corners, including negative skew.
        let low = self.skew * bounds.y0 as f32;
        let high = self.skew * bounds.y1 as f32;
        let edges = [
            x + bounds.x0 as f32 + low.min(high),
            y - bounds.y1 as f32,
            x + bounds.x1 as f32 + low.max(high),
            y - bounds.y0 as f32,
        ];
        let [left, top, right, bottom] = edges.map(f64::from);
        Ok(!edges.into_iter().all(f32::is_finite)
            || left <= self.rect.x1
                && right >= self.rect.x0
                && top <= self.rect.y1
                && bottom >= self.rect.y0)
    }
}

struct BoundsPen<'a> {
    rect: Option<Rect>,
    commands: &'a mut usize,
}

impl BoundsPen<'_> {
    fn record(&mut self, points: &[(f32, f32)]) {
        *self.commands = self.commands.saturating_add(1);
        if *self.commands > OUTLINE_COMMANDS {
            return;
        }
        for &(x, y) in points {
            let point = (f64::from(x), f64::from(y));
            self.rect = Some(self.rect.map_or_else(
                || Rect::from_points(point, point),
                |rect| rect.union_pt(point),
            ));
        }
    }
}

impl OutlinePen for BoundsPen<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        self.record(&[(x, y)]);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.record(&[(x, y)]);
    }

    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        self.record(&[(cx, cy), (x, y)]);
    }

    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.record(&[(cx0, cy0), (cx1, cy1), (x, y)]);
    }

    fn close(&mut self) {
        self.record(&[]);
    }
}
