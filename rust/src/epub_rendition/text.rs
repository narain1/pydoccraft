//! Collect text from the same positioned glyphs and clusters used by the EPUB preview.
use std::{collections::HashMap, sync::Arc};

use kurbo::Affine;
use wordcraft_geom::Rect;
use wordcraft_layout::display::Draw;

use super::{intersection, Capture};
use crate::epub::svg::LinkGeometry;
use crate::EngineError;

const MAX_CLUSTERS: usize = 250_000;
const MAX_TEXT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Default)]
pub(in crate::epub) struct TextLayer {
    pub(super) pages: Vec<Vec<TextCluster>>,
    clusters: usize,
    bytes: usize,
}

pub(super) struct TextCluster {
    pub text: String,
    pub rect: Rect,
}

pub(in crate::epub) struct SvgImages {
    images: HashMap<String, SvgImage>,
    frames: HashMap<String, FrameImage>,
    searchable: bool,
    geometry_bytes: usize,
    clusters: usize,
    bytes: usize,
}

struct SvgImage {
    size: [f32; 2],
    text: Vec<TextCluster>,
    geometry: Option<Arc<LinkGeometry>>,
    tree_size: [f32; 2],
}

struct FrameImage {
    size: [f32; 2],
    text: Vec<TextCluster>,
    links: super::LinkAreas,
}

impl SvgImages {
    pub(in crate::epub) fn new(searchable: bool) -> Self {
        Self {
            images: HashMap::new(),
            frames: HashMap::new(),
            searchable,
            geometry_bytes: 0,
            clusters: 0,
            bytes: 0,
        }
    }

    pub(in crate::epub) fn searchable(&self) -> bool {
        self.searchable
    }

    pub(in crate::epub) fn record_frame(
        &mut self,
        media: &str,
        size: [f32; 2],
        text: Option<TextLayer>,
        links: super::LinkAreas,
    ) -> Result<(), EngineError> {
        let mut text = text.unwrap_or_default();
        super::super::charge(
            &mut self.geometry_bytes,
            links
                .entries
                .iter()
                .map(|(_, _, href)| href.len() + 64)
                .sum(),
            super::super::XML_BYTES,
            "EPUB image annotations exceed 8 MiB",
        )?;
        super::super::charge(
            &mut self.clusters,
            text.clusters,
            MAX_CLUSTERS,
            "EPUB image text has too many clusters",
        )?;
        super::super::charge(
            &mut self.bytes,
            text.bytes,
            MAX_TEXT_BYTES,
            "EPUB image text exceeds 4 MiB",
        )?;
        self.frames.insert(
            media.to_owned(),
            FrameImage {
                size,
                text: text.pages.pop().unwrap_or_default(),
                links,
            },
        );
        Ok(())
    }

    pub(in crate::epub) fn record(
        &mut self,
        media: &str,
        tree: &resvg::usvg::Tree,
        size: [f32; 2],
        pixels: [u32; 2],
        geometry: Option<Arc<LinkGeometry>>,
    ) -> Result<(), EngineError> {
        if self.images.contains_key(media) {
            return Ok(());
        }
        let mut capture = TextLayer::default();
        if self.searchable {
            capture.begin_page();
            crate::epub::svg::collect_image_text(
                tree,
                size,
                [
                    pixels[0] as f32 / tree.size().width(),
                    pixels[1] as f32 / tree.size().height(),
                ],
                &mut capture,
            )?;
        }
        if let Some(geometry) = &geometry {
            super::super::charge(
                &mut self.geometry_bytes,
                geometry.byte_cost(),
                super::super::XML_BYTES,
                "EPUB SVG image annotations exceed 8 MiB",
            )?;
        }
        if capture.clusters > 0 || geometry.is_some() {
            super::super::charge(
                &mut self.clusters,
                capture.clusters,
                MAX_CLUSTERS,
                "EPUB SVG image text has too many clusters",
            )?;
            super::super::charge(
                &mut self.bytes,
                capture.bytes,
                MAX_TEXT_BYTES,
                "EPUB SVG image text exceeds 4 MiB",
            )?;
            self.images.insert(
                media.to_owned(),
                SvgImage {
                    size,
                    text: capture.pages.pop().unwrap_or_default(),
                    geometry,
                    tree_size: [tree.size().width(), tree.size().height()],
                },
            );
        }
        Ok(())
    }
}

impl TextLayer {
    pub(in crate::epub) fn begin_page(&mut self) {
        self.pages.push(Vec::new());
    }

    pub(in crate::epub) fn add(
        &mut self,
        text: &str,
        rect: Rect,
        clip: Rect,
    ) -> Result<(), EngineError> {
        let rect = intersection(rect, clip);
        if text.is_empty()
            || rect.w <= 0.0
            || rect.h <= 0.0
            || text.contains(wordcraft_doc::para::OBJ)
        {
            return Ok(());
        }
        super::super::charge(
            &mut self.clusters,
            1,
            MAX_CLUSTERS,
            "EPUB PDF text has too many clusters",
        )?;
        super::super::charge(
            &mut self.bytes,
            text.len(),
            MAX_TEXT_BYTES,
            "EPUB PDF text exceeds 4 MiB",
        )?;
        let page = self
            .pages
            .last_mut()
            .ok_or_else(|| super::super::invalid("missing EPUB PDF text page"))?;
        page.push(TextCluster {
            text: text.to_owned(),
            rect,
        });
        Ok(())
    }
}

pub(in crate::epub) fn draw_text(
    drawing: &[Draw],
    origin: [f32; 2],
    clip: Rect,
    images: Option<&SvgImages>,
    capture: &mut Capture<'_, '_>,
) -> Result<(), EngineError> {
    walk_draws(
        drawing,
        Affine::translate((f64::from(origin[0]), f64::from(origin[1]))),
        clip,
        images,
        capture,
    )
}

pub(in crate::epub) fn draw_image(
    images: Option<&SvgImages>,
    media: &str,
    rect: Rect,
    transform: Affine,
    clip: Rect,
    capture: &mut Capture<'_, '_>,
) -> Result<(), EngineError> {
    if let Some(frame) = images.and_then(|images| images.frames.get(media)) {
        let matrix = transform
            * Affine::translate((f64::from(rect.x), f64::from(rect.y)))
            * Affine::scale_non_uniform(
                f64::from(rect.w / frame.size[0]),
                f64::from(rect.h / frame.size[1]),
            );
        for (_, region, href) in &frame.links.entries {
            capture
                .links
                .add(transformed(*region, matrix), clip, href.clone())?;
        }
        if let Some(text) = capture.text.as_deref_mut() {
            for cluster in &frame.text {
                text.add(&cluster.text, transformed(cluster.rect, matrix), clip)?;
            }
        }
    }
    if let Some(image) = images.and_then(|images| images.images.get(media)) {
        let position = transform * Affine::translate((f64::from(rect.x), f64::from(rect.y)));
        if let Some(geometry) = &image.geometry {
            let matrix = position
                * Affine::scale_non_uniform(
                    f64::from(rect.w / image.tree_size[0]),
                    f64::from(rect.h / image.tree_size[1]),
                );
            geometry.place(matrix, clip, capture)?;
        }
        if let Some(text) = capture.text.as_deref_mut() {
            let matrix = position
                * Affine::scale_non_uniform(
                    f64::from(rect.w / image.size[0]),
                    f64::from(rect.h / image.size[1]),
                );
            for cluster in &image.text {
                text.add(&cluster.text, transformed(cluster.rect, matrix), clip)?;
            }
        }
    }
    Ok(())
}

fn walk_draws(
    drawing: &[Draw],
    transform: Affine,
    clip: Rect,
    images: Option<&SvgImages>,
    capture: &mut Capture<'_, '_>,
) -> Result<(), EngineError> {
    use std::collections::BTreeMap;
    for draw in drawing {
        match draw {
            Draw::Glyphs {
                face,
                size,
                glyphs,
                text,
                alpha,
                ranges,
                x_scale,
                ..
            } if *alpha > 0.0 && !text.is_empty() && !glyphs.is_empty() => {
                let Some(layer) = capture.text.as_deref_mut() else {
                    continue;
                };
                let em = f64::from(*size) / face.upem.max(1.0);
                let ascent = (face.ascent * em) as f32;
                let descent = (face.descent * em) as f32;
                let bounds = |&(id, x, baseline): &(u32, f32, f32)| {
                    let width = (face.advance(id) * em) as f32 * x_scale;
                    Rect::new(x, baseline - ascent, width, ascent - descent)
                };
                if ranges.len() == glyphs.len() {
                    let mut clusters = BTreeMap::<_, Rect>::new();
                    for (glyph, range) in glyphs.iter().zip(ranges) {
                        if !range.is_empty() {
                            let rect = bounds(glyph);
                            clusters
                                .entry((range.start, range.end))
                                .and_modify(|r| *r = r.union(&rect))
                                .or_insert(rect);
                        }
                    }
                    for ((start, end), rect) in clusters {
                        if let Some(cluster) = text.get(start..end) {
                            layer.add(cluster, transformed(rect, transform), clip)?;
                        }
                    }
                } else if let Some(rect) = glyphs.iter().map(bounds).reduce(|a, b| a.union(&b)) {
                    // Equation display items provide Unicode per painted atom.
                    layer.add(text, transformed(rect, transform), clip)?;
                }
            }
            Draw::Clip { rect, items } => {
                let clip = intersection(clip, transformed(*rect, transform));
                if clip.w > 0.0 && clip.h > 0.0 {
                    walk_draws(items, transform, clip, images, capture)?;
                }
            }
            Draw::Turned { x, y, turn, items } => {
                let matrix = Draw::turn_matrix(*turn, *x, *y).map(f64::from);
                walk_draws(
                    items,
                    transform * Affine::new(matrix),
                    clip,
                    images,
                    capture,
                )?;
            }
            Draw::Image {
                rect, media, alpha, ..
            } if *alpha > 0.0 => draw_image(images, media, *rect, transform, clip, capture)?,
            Draw::Figure { draws, .. } => walk_draws(draws, transform, clip, images, capture)?,
            _ => {}
        }
    }
    Ok(())
}

pub(in crate::epub) fn transformed(rect: Rect, transform: Affine) -> Rect {
    let bounds = transform.transform_rect_bbox(kurbo::Rect::new(
        f64::from(rect.x),
        f64::from(rect.y),
        f64::from(rect.right()),
        f64::from(rect.bottom()),
    ));
    Rect::new(
        bounds.x0 as f32,
        bounds.y0 as f32,
        bounds.width() as f32,
        bounds.height() as f32,
    )
}

#[cfg(feature = "python")]
#[path = "pdf_text.rs"]
mod pdf;
