//! Compose EPUB viewports on a common pixel grid and export fixed pages.
use std::{collections::HashMap, sync::Arc};

use serde_json::Value;
use wordcraft_geom::Rect;

#[cfg(feature = "python")]
#[path = "epub_rendition/navigation.rs"]
mod navigation;
#[path = "epub_rendition/text.rs"]
pub(super) mod text;

pub(super) type Anchors<'a> = HashMap<&'a str, Option<[f32; 2]>>;

pub(super) struct Capture<'a, 'key> {
    pub anchors: Option<&'a mut Anchors<'key>>,
    pub links: &'a mut LinkAreas,
    pub text: Option<&'a mut text::TextLayer>,
}

#[derive(Default)]
pub(super) struct LinkAreas {
    page: usize,
    pub(super) entries: Vec<(usize, Rect, Arc<str>)>,
    pub(super) bytes: usize,
}

impl LinkAreas {
    pub(super) fn add(
        &mut self,
        rect: Rect,
        clip: Rect,
        href: Arc<str>,
    ) -> Result<(), EngineError> {
        let rect = intersection(rect, clip);
        if rect.w <= 0.0 || rect.h <= 0.0 {
            return Ok(());
        }
        if let Some((page, existing, previous)) = self.entries.last_mut() {
            if *page == self.page
                && *previous == href
                && (existing.y - rect.y).abs() <= 0.01
                && (existing.h - rect.h).abs() <= 0.01
                && existing.x <= rect.right() + 0.01
                && rect.x <= existing.right() + 0.01
            {
                *existing = existing.union(&rect);
                return Ok(());
            }
        }
        super::charge(
            &mut self.bytes,
            href.len().saturating_add(64),
            4 * 1024 * 1024,
            "EPUB PDF link annotations exceed 4 MiB",
        )?;
        crate::limits::validate_member_count(self.entries.len() + 1)?;
        self.entries.push((self.page, rect, href));
        Ok(())
    }
}

pub(super) fn intersection(a: Rect, b: Rect) -> Rect {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    Rect::new(
        x,
        y,
        a.right().min(b.right()) - x,
        a.bottom().min(b.bottom()) - y,
    )
}

pub(super) fn capture_anchor(
    anchors: &mut Option<&mut Anchors<'_>>,
    id: &str,
    rect: Rect,
    clip: Rect,
) {
    if let Some(position) = anchors
        .as_deref_mut()
        .and_then(|anchors| anchors.get_mut(id))
    {
        if position.is_none() {
            *position = anchor_position(rect, clip);
        }
    }
}

pub(super) fn anchor_position(rect: Rect, clip: Rect) -> Option<[f32; 2]> {
    let left = rect.x.max(clip.x);
    let top = rect.y.max(clip.y);
    let right = rect.right().min(clip.right());
    let bottom = rect.bottom().min(clip.bottom());
    (clip.w > 0.0 && clip.h > 0.0 && left <= right && top <= bottom).then_some([left, top])
}
#[cfg(feature = "python")]
use wordcraft_render::Rendered;

#[cfg(feature = "python")]
use super::{charge, invalid, layout, unsupported};
use crate::{
    model::PagePayload,
    rendering::{self, Clip, RasterRegion},
    EngineError,
};

#[cfg(feature = "python")]
use crate::model::DocumentPayload;

#[derive(Clone, Copy)]
pub(super) enum Region {
    Viewport(Option<Clip>),
    #[cfg(feature = "python")]
    Pixels(RasterRegion),
}

impl Region {
    pub(super) fn for_page(
        self,
        dimensions: [f64; 2],
        dpi: f64,
        fixed: bool,
    ) -> Result<RasterRegion, EngineError> {
        let pixels = if fixed {
            fixed_raster_size(dimensions, dpi)
        } else {
            dimensions.map(|size| (size * (dpi / 72.0)).ceil().max(1.0) as u32)
        };
        let extent = pixels.map(f64::from);
        match self {
            Self::Viewport(Some(clip)) => RasterRegion::new(
                clip,
                dimensions,
                dpi / 72.0,
                extent,
                wordcraft_render::MAX_SIDE,
            ),
            Self::Viewport(None) => {
                rendering::validate_size(extent[0], extent[1], wordcraft_render::MAX_SIDE)?;
                Ok(RasterRegion {
                    x: 0,
                    y: 0,
                    width: pixels[0],
                    height: pixels[1],
                })
            }
            #[cfg(feature = "python")]
            Self::Pixels(region) => {
                rendering::validate_size(
                    f64::from(region.width),
                    f64::from(region.height),
                    wordcraft_render::MAX_SIDE,
                )?;
                if region.x.saturating_add(region.width) > pixels[0]
                    || region.y.saturating_add(region.height) > pixels[1]
                {
                    return Err(invalid("spread tile exceeds its chapter viewport"));
                }
                Ok(region)
            }
        }
    }
}

fn fixed_raster_size(dimensions: [f64; 2], dpi: f64) -> [u32; 2] {
    // Match the PDF renderer's treatment of floating-point noise near a whole pixel.
    dimensions.map(|size| pdfcraft_render::device_pixels(size as f32, (dpi / 72.0) as f32))
}

pub(super) fn fixed(page: &PagePayload) -> bool {
    page.metadata.get("fixed_layout").and_then(Value::as_bool) == Some(true)
}

#[cfg(feature = "python")]
fn viewport(page: &PagePayload) -> Result<[f64; 2], EngineError> {
    let dims = page
        .metadata
        .get("viewport")
        .and_then(Value::as_array)
        .and_then(|dims| match dims.as_slice() {
            [width, height] => Some([width.as_f64()?, height.as_f64()?]),
            _ => None,
        });
    dims.filter(|dims| {
        dims.iter()
            .all(|n| n.is_finite() && (0.0..=20_000.0).contains(n) && *n > 0.0)
    })
    .ok_or_else(|| unsupported("fixed-layout chapter requires a numeric viewport"))
}

#[cfg(feature = "python")]
#[derive(Debug, Eq, PartialEq)]
enum Spread {
    Single,
    Facing([Option<usize>; 2]),
}

#[cfg(feature = "python")]
fn spread(book: &DocumentPayload, requested: usize, landscape: bool) -> Spread {
    let leading = usize::from(
        book.metadata
            .get("page_progression_direction")
            .and_then(Value::as_str)
            == Some("rtl"),
    );
    let trailing = 1 - leading;
    let global = book
        .metadata
        .get("rendition_spread")
        .and_then(Value::as_str)
        .unwrap_or("auto");
    let mut automatic = trailing;
    let mut pending = None;
    let facing = |first, second| {
        let mut pages = [None; 2];
        pages[leading] = first;
        pages[trailing] = second;
        Spread::Facing(pages)
    };
    for (index, page) in book.pages.iter().enumerate() {
        if page.metadata.get("linear").and_then(Value::as_bool) == Some(false) {
            if index == requested {
                return Spread::Single;
            }
            continue;
        }
        let placement = page.metadata.get("page_spread").and_then(Value::as_str);
        let policy = page
            .metadata
            .get("rendition_spread")
            .and_then(Value::as_str)
            .unwrap_or(global);
        let pair = match policy {
            "both" | "portrait" => true,
            "none" => false,
            _ => landscape,
        };
        if !fixed(page) || !pair || placement == Some("center") {
            if pending == Some(requested) {
                return facing(pending, None);
            }
            if index == requested {
                return Spread::Single;
            }
            pending = None;
            automatic = leading;
            continue;
        }
        let side = match placement {
            Some("left") => 0,
            Some("right") => 1,
            _ => automatic,
        };
        if side == leading {
            if pending == Some(requested) {
                return facing(pending, None);
            }
            pending = Some(index);
            automatic = trailing;
        } else {
            if pending == Some(requested) || index == requested {
                return facing(pending, Some(index));
            }
            pending = None;
            automatic = leading;
        }
    }
    facing(pending, None)
}

#[cfg(feature = "python")]
pub(super) fn render_spread(
    data: &[u8],
    book: &DocumentPayload,
    page: usize,
    dpi: f64,
    landscape: bool,
    clip: Option<Clip>,
) -> Result<Vec<u8>, EngineError> {
    rendering::validate_dpi(dpi)?;
    let requested = book
        .pages
        .get(page)
        .ok_or_else(|| EngineError::PageIndex("EPUB chapter is out of range".into()))?;
    let Spread::Facing(pages) = spread(book, page, landscape) else {
        return super::render_page(data, book, requested, dpi, clip);
    };
    let default = fixed_raster_size(viewport(requested)?, dpi);
    let mut sizes = [default; 2];
    for (size, index) in sizes.iter_mut().zip(pages) {
        if let Some(index) = index {
            *size = fixed_raster_size(viewport(&book.pages[index])?, dpi);
        }
    }
    let extent = [sizes[0][0] + sizes[1][0], sizes[0][1].max(sizes[1][1])].map(f64::from);
    let dimensions = extent.map(|size| size / (dpi / 72.0));
    let region = if let Some(clip) = clip {
        RasterRegion::new(
            clip,
            dimensions,
            dpi / 72.0,
            extent,
            wordcraft_render::MAX_SIDE,
        )?
    } else {
        rendering::validate_size(extent[0], extent[1], wordcraft_render::MAX_SIDE)?;
        RasterRegion {
            x: 0,
            y: 0,
            width: extent[0] as u32,
            height: extent[1] as u32,
        }
    };
    let row_bytes = region.width as usize * 4;
    let mut rgba = vec![255; row_bytes * region.height as usize];
    for ((index, [width, height]), origin) in pages.into_iter().zip(sizes).zip([0, sizes[0][0]]) {
        let Some(index) = index else { continue };
        let left = origin.max(region.x);
        let right = (origin + width).min(region.x + region.width);
        let bottom = height.min(region.y + region.height);
        if left >= right || region.y >= bottom {
            continue;
        }
        let tile = RasterRegion {
            x: left - origin,
            y: region.y,
            width: right - left,
            height: bottom - region.y,
        };
        let rendered = render_fixed(data, book, &book.pages[index], dpi, Region::Pixels(tile))?;
        tile.validate_result(rendered.width, rendered.height)?;
        let offset = (left - region.x) as usize * 4;
        let tile_row = tile.width as usize * 4;
        for (source, destination) in rendered
            .pixels
            .chunks_exact(tile_row)
            .zip(rgba.chunks_exact_mut(row_bytes))
        {
            destination[offset..offset + tile_row].copy_from_slice(source);
        }
    }
    rendering::encode_png(region.width, region.height, &rgba, dpi)
}

#[cfg(feature = "python")]
fn render_fixed(
    data: &[u8],
    book: &DocumentPayload,
    page: &PagePayload,
    dpi: f64,
    region: Region,
) -> Result<Rendered, EngineError> {
    let image = layout::render(data, book, page, dpi, region)?
        .ok_or_else(|| invalid("fixed-layout preview has no raster"))?;
    rendering::validate_rgba(image.width, image.height, &image.pixels)?;
    Ok(image)
}

/// Explicit visual export. Use one image at the original viewport on each PDF page.
#[cfg(feature = "python")]
pub(super) fn export_pdf(
    data: &[u8],
    book: &DocumentPayload,
    dpi: f64,
    pages: Option<&[usize]>,
    searchable: bool,
) -> Result<Vec<u8>, EngineError> {
    use wordcraft_doc::StoryRef;
    use wordcraft_layout::{DocLayout, Page, Placed};

    rendering::validate_dpi(dpi)?;
    let selected = if let Some(indexes) = pages {
        crate::limits::validate_member_count(indexes.len())?;
        indexes
            .iter()
            .map(|&index| {
                book.pages
                    .get(index)
                    .ok_or_else(|| EngineError::PageIndex("EPUB chapter is out of range".into()))
            })
            .collect::<Result<Vec<_>, _>>()?
    } else {
        book.pages.iter().collect()
    };
    if selected.is_empty() {
        return Err(EngineError::InvalidArgument(
            "fixed-layout PDF export requires at least one page".into(),
        ));
    }
    if selected.iter().any(|page| !fixed(page)) {
        return Err(unsupported(
            "visual PDF export requires fixed-layout spine entries",
        ));
    }
    let mut navigation = navigation::Navigation::new(book, &selected, searchable)?;
    let mut document = super::output::Output::new(data, book, false, dpi).document;
    let mut pdf_layout = DocLayout::default();
    let mut rendered_pixels = 0;
    let mut embedded_bytes = 0;
    for (index, page) in selected.into_iter().enumerate() {
        let [w, h] = viewport(page)?;
        // WordCraft's writer clamps sizes outside this range. Reject those
        // sizes here so the exported MediaBox and viewport remain identical.
        if ![w, h]
            .into_iter()
            .all(|size| (3.0..=14_400.0).contains(&size))
        {
            return Err(unsupported(
                "fixed EPUB viewport exceeds PDF page dimensions",
            ));
        }
        let [width, height] = fixed_raster_size([w, h], dpi);
        charge(
            &mut rendered_pixels,
            (width as usize).saturating_mul(height as usize),
            64_000_000,
            "fixed-layout PDF exceeds 64 megapixels",
        )?;
        let image = layout::render_with_navigation(
            data,
            book,
            page,
            dpi,
            Region::Viewport(None),
            Some(navigation.capture(index, page)),
        )?
        .ok_or_else(|| invalid("fixed-layout preview has no raster"))?;
        rendering::validate_rgba(image.width, image.height, &image.pixels)?;
        let png = rendering::encode_png(image.width, image.height, &image.pixels, dpi)?;
        charge(
            &mut embedded_bytes,
            png.len(),
            64 * 1024 * 1024,
            "fixed-layout PDF images exceed 64 MiB",
        )?;
        // Place the original samples at the requested DPI. The PDF page bounds
        // clip the last partial pixel instead of squeezing the whole raster.
        let points_per_pixel = 72.0 / dpi;
        let image_rect = Rect::new(
            0.0,
            0.0,
            (f64::from(image.width) * points_per_pixel) as f32,
            (f64::from(image.height) * points_per_pixel) as f32,
        );
        let media = document.add_media(png, "png");
        let (w, h) = (w as f32, h as f32);
        let full = Rect::new(0.0, 0.0, w, h);
        pdf_layout.pages.push(Page {
            w,
            h,
            number: page.number as u32 + 1,
            body: full,
            items: vec![Placed::Image {
                rect: image_rect,
                media,
                crop: [0.0; 4],
                story: StoryRef::Body,
                path: wordcraft_doc::Path(Vec::new()),
                off: 0,
                spin: Default::default(),
            }],
            ..Default::default()
        });
    }
    let pdf = wordcraft_pdf::export_layout(
        &document,
        &pdf_layout,
        &wordcraft_pdf::PdfOptions::default(),
    )
    .map_err(|error| {
        EngineError::Conversion(format!("EPUB fixed-layout PDF export failed: {error}"))
    })?;
    let pdf = navigation.attach(pdf)?;
    crate::limits::validate_input_size(pdf.len())?;
    Ok(pdf)
}
