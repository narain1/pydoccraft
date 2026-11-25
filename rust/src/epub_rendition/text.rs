//! Collect text from the same positioned glyphs and clusters used by the EPUB preview.
use std::collections::HashMap;

use kurbo::Affine;
use wordcraft_geom::Rect;
use wordcraft_layout::display::Draw;

use super::intersection;
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

#[derive(Default)]
pub(in crate::epub) struct SvgImages {
    images: HashMap<String, SvgImage>,
    clusters: usize,
    bytes: usize,
}

struct SvgImage {
    size: [f32; 2],
    text: Vec<TextCluster>,
}

impl SvgImages {
    pub(in crate::epub) fn record(
        &mut self,
        media: &str,
        tree: &resvg::usvg::Tree,
        size: [f32; 2],
    ) -> Result<(), EngineError> {
        if self.images.contains_key(media) {
            return Ok(());
        }
        let mut capture = TextLayer::default();
        capture.begin_page();
        crate::epub::svg::collect_image_text(tree, size, &mut capture)?;
        if capture.clusters > 0 {
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
                },
            );
        }
        Ok(())
    }
}

impl TextLayer {
    pub(super) fn begin_page(&mut self) {
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

    pub(in crate::epub) fn draw_text(
        &mut self,
        drawing: &[Draw],
        origin: [f32; 2],
        clip: Rect,
        images: Option<&SvgImages>,
    ) -> Result<(), EngineError> {
        self.walk_draws(
            drawing,
            Affine::translate((f64::from(origin[0]), f64::from(origin[1]))),
            clip,
            images,
        )
    }

    pub(in crate::epub) fn draw_image(
        &mut self,
        images: Option<&SvgImages>,
        media: &str,
        rect: Rect,
        clip: Rect,
    ) -> Result<(), EngineError> {
        self.image_text(images, media, rect, Affine::IDENTITY, clip)
    }

    fn image_text(
        &mut self,
        images: Option<&SvgImages>,
        media: &str,
        rect: Rect,
        transform: Affine,
        clip: Rect,
    ) -> Result<(), EngineError> {
        if let Some(image) = images.and_then(|images| images.images.get(media)) {
            let matrix = transform
                * Affine::translate((f64::from(rect.x), f64::from(rect.y)))
                * Affine::scale_non_uniform(
                    f64::from(rect.w / image.size[0]),
                    f64::from(rect.h / image.size[1]),
                );
            for cluster in &image.text {
                self.add(&cluster.text, transformed(cluster.rect, matrix), clip)?;
            }
        }
        Ok(())
    }

    fn walk_draws(
        &mut self,
        drawing: &[Draw],
        transform: Affine,
        clip: Rect,
        images: Option<&SvgImages>,
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
                                self.add(cluster, transformed(rect, transform), clip)?;
                            }
                        }
                    } else if let Some(rect) = glyphs.iter().map(bounds).reduce(|a, b| a.union(&b))
                    {
                        // Equation display items provide Unicode per painted atom.
                        self.add(text, transformed(rect, transform), clip)?;
                    }
                }
                Draw::Clip { rect, items } => {
                    let clip = intersection(clip, transformed(*rect, transform));
                    if clip.w > 0.0 && clip.h > 0.0 {
                        self.walk_draws(items, transform, clip, images)?;
                    }
                }
                Draw::Turned { x, y, turn, items } => {
                    let matrix = Draw::turn_matrix(*turn, *x, *y).map(f64::from);
                    self.walk_draws(items, transform * Affine::new(matrix), clip, images)?;
                }
                Draw::Image {
                    rect, media, alpha, ..
                } if *alpha > 0.0 => self.image_text(images, media, *rect, transform, clip)?,
                Draw::Figure { draws, .. } => self.walk_draws(draws, transform, clip, images)?,
                _ => {}
            }
        }
        Ok(())
    }
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
