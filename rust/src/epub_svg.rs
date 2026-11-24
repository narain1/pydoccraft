//! Rasterize a validated EPUB SVG content document on its declared viewport.
use std::sync::{Arc, OnceLock};

use resvg::{tiny_skia, usvg};
use roxmltree::Node;

use super::{content, invalid, limited, unsupported};
use crate::{
    rendering::{self, Clip, RasterRegion},
    EngineError,
};

pub(super) const NS: &str = "http://www.w3.org/2000/svg";

pub(super) fn viewport(root: Node<'_, '_>) -> Option<[f32; 2]> {
    let parts: Vec<_> = root
        .attribute("viewBox")?
        .split(|c: char| c == ',' || c.is_ascii_whitespace())
        .filter(|value| !value.is_empty())
        .take(5)
        .map(str::parse::<f32>)
        .collect::<Result<_, _>>()
        .ok()?;
    let [x, y, width, height] = parts.as_slice() else {
        return None;
    };
    if ![x, y, width, height]
        .into_iter()
        .all(|n| n.is_finite() && n.abs() <= 25_000.0)
        || *width <= 0.0
        || *height <= 0.0
    {
        return None;
    }
    Some([width * 0.75, height * 0.75])
}

pub(super) fn render(
    root: Node<'_, '_>,
    source: &[u8],
    fixed: bool,
    dpi: f64,
    clip: Option<Clip>,
) -> Result<Vec<u8>, EngineError> {
    content::validate_svg(root)?;
    let viewport = viewport(root);
    if fixed && viewport.is_none() {
        return Err(unsupported("fixed-layout SVG requires a numeric viewBox"));
    }
    let mut options = usvg::Options {
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        },
        max_depth: 96,
        ..Default::default()
    };
    if let Some([w, h]) = viewport {
        options.default_size = usvg::Size::from_wh(w / 0.75, h / 0.75)
            .ok_or_else(|| invalid("invalid SVG dimensions"))?;
    }
    if root
        .descendants()
        .any(|node| node.has_tag_name((NS, "text")))
    {
        static FONTS: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
        options.fontdb = FONTS
            .get_or_init(|| {
                let mut database = usvg::fontdb::Database::new();
                database.load_system_fonts();
                Arc::new(database)
            })
            .clone();
        if options.fontdb.faces().next().is_none() {
            return Err(unsupported("no installed fonts available for SVG text"));
        }
    }
    let tree = usvg::Tree::from_data(source, &options).map_err(invalid)?;
    let [w, h] = viewport.unwrap_or([tree.size().width() * 0.75, tree.size().height() * 0.75]);
    let scale = dpi / 72.0;
    let pixel_size = [w, h].map(|n| (f64::from(n) * scale).ceil().max(1.0));
    let region = if let Some(clip) = clip {
        RasterRegion::new(
            clip,
            [f64::from(w), f64::from(h)],
            scale,
            pixel_size,
            wordcraft_render::MAX_SIDE,
        )?
    } else {
        rendering::validate_size(pixel_size[0], pixel_size[1], wordcraft_render::MAX_SIDE)?;
        RasterRegion {
            x: 0,
            y: 0,
            width: pixel_size[0] as u32,
            height: pixel_size[1] as u32,
        }
    };
    let mut image = tiny_skia::Pixmap::new(region.width, region.height)
        .ok_or_else(|| limited("cannot allocate EPUB SVG viewport"))?;
    image.fill(tiny_skia::Color::WHITE);
    let transform = tiny_skia::Transform::from_row(
        w * scale as f32 / tree.size().width(),
        0.0,
        0.0,
        h * scale as f32 / tree.size().height(),
        -(region.x as f32),
        -(region.y as f32),
    );
    resvg::render(&tree, transform, &mut image.as_mut());
    rendering::encode_png(region.width, region.height, image.data(), dpi)
}
