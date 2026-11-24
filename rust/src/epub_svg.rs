//! Rasterize a validated EPUB SVG content document on its declared viewport.
use std::{
    collections::{hash_map::Entry, HashMap, HashSet},
    sync::{Arc, OnceLock},
};

use resvg::{tiny_skia, usvg};
use roxmltree::Node;

use super::{
    charge, content, css::Cascade, external, fonts, invalid, limited, resolve, unsupported, Book,
};
use crate::{
    author_epub::escape_attribute,
    model::ImagePayload,
    rendering::{self, Clip, RasterRegion},
    EngineError,
};

pub(super) const NS: &str = "http://www.w3.org/2000/svg";
pub(super) const XLINK: &str = "http://www.w3.org/1999/xlink";

pub(super) fn image_base(image: &ImagePayload) -> Result<&str, EngineError> {
    image
        .metadata
        .get("epub_svg_base")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| invalid("missing EPUB SVG source path"))
}

pub(super) fn image_href<'a>(node: Node<'a, '_>) -> Option<&'a str> {
    node.attribute("href")
        .or_else(|| node.attribute((XLINK, "href")))
}

pub(super) fn resource_path(base: &str, href: &str) -> Result<String, EngineError> {
    if href.is_empty() || href.contains(['?', '#']) || external(href) {
        return Err(unsupported(
            "EPUB SVG resources require relative archive paths",
        ));
    }
    resolve(base, href)
}

pub(super) fn stylesheet(pi: roxmltree::PI<'_>) -> Result<Option<(String, String)>, EngineError> {
    if pi.target != "xml-stylesheet" {
        return Err(unsupported("SVG processing instructions"));
    }
    let mut values = HashMap::new();
    let instruction =
        quick_xml::events::BytesPI::new(format!("xml-stylesheet {}", pi.value.unwrap_or_default()));
    for attr in instruction.attributes() {
        let attr = attr.map_err(invalid)?;
        let key = std::str::from_utf8(attr.key.as_ref()).map_err(invalid)?;
        if !matches!(
            key,
            "href" | "type" | "media" | "alternate" | "title" | "charset"
        ) {
            return Err(invalid("unknown xml-stylesheet pseudo-attribute"));
        }
        values.insert(
            key.to_owned(),
            attr.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                .map_err(invalid)?
                .into_owned(),
        );
    }
    let href = values
        .remove("href")
        .filter(|s| !s.is_empty())
        .ok_or_else(|| invalid("xml-stylesheet requires href"))?;
    if values.get("type").is_some_and(|value| value != "text/css")
        || values.get("alternate").is_some_and(|value| value == "yes")
    {
        return Ok(None);
    }
    Ok(Some((href, values.remove("media").unwrap_or_default())))
}

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
    base: &str,
    book: &mut Book<'_>,
    fixed: bool,
    dpi: f64,
    clip: Option<Clip>,
) -> Result<Vec<u8>, EngineError> {
    let viewport = viewport(root);
    if fixed && viewport.is_none() {
        return Err(unsupported("fixed-layout SVG requires a numeric viewBox"));
    }
    let tree = parse(root, source, base, book, viewport)?;
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

pub(super) fn embedded(
    source: &[u8],
    base: &str,
    book: &mut Book<'_>,
) -> Result<usvg::Tree, EngineError> {
    let xml = super::parse_xml(source)?;
    let root = xml.root_element();
    parse(root, source, base, book, viewport(root))
}

fn parse(
    root: Node<'_, '_>,
    source: &[u8],
    base: &str,
    book: &mut Book<'_>,
    viewport: Option<[f32; 2]>,
) -> Result<usvg::Tree, EngineError> {
    content::validate_svg(root, Some(base))?;
    let images = raster_images(root, base, book)?;
    let mut options = usvg::Options {
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(move |href, _| {
                images.get(href).map(|(_, image)| image.clone())
            }),
        },
        max_depth: 96,
        ..Default::default()
    };
    if let Some([w, h]) = viewport {
        options.default_size = usvg::Size::from_wh(w / 0.75, h / 0.75)
            .ok_or_else(|| invalid("invalid SVG dimensions"))?;
    }
    let viewport = [
        options.default_size.width() * 0.75,
        options.default_size.height() * 0.75,
    ];
    let mut cascade = Cascade::load_svg(root, base, book, viewport)?;
    let (styled, families) = apply_styles(root, source, &mut cascade)?;
    if root
        .descendants()
        .any(|node| node.has_tag_name((NS, "text")))
    {
        options.fontdb = font_database(&cascade.fonts, &families, book)?;
    }
    usvg::Tree::from_data(&styled, &options).map_err(invalid)
}

type FontNames = HashMap<String, HashSet<String>>;

fn apply_styles(
    root: Node<'_, '_>,
    source: &[u8],
    cascade: &mut Cascade,
) -> Result<(Vec<u8>, FontNames), EngineError> {
    let mut output = Vec::with_capacity(source.len());
    let mut cursor = 0;
    let mut families = FontNames::new();
    let mut added_bytes = 0;
    // Process byte ranges in document order. Source XML remains available for extraction.
    for node in root.descendants().filter(Node::is_element) {
        if node.range().start < cursor {
            continue;
        }
        if node.has_tag_name((NS, "style")) {
            let range = node.range();
            output.extend_from_slice(&source[cursor..range.start]);
            cursor = range.end;
            continue;
        }
        let mut props: Vec<_> = cascade.properties(node)?.into_iter().collect();
        props.retain(|(key, _)| svg_property(key));
        props.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        let family = props
            .iter()
            .find(|(key, _)| key == "font-family")
            .map(|(_, value)| value.as_str())
            .or_else(|| node.attribute("font-family"));
        if let Some(family) = family {
            for name in family
                .split(',')
                .map(|name| name.trim().trim_matches(['\'', '"']))
            {
                families
                    .entry(name.to_lowercase())
                    .or_default()
                    .insert(name.to_owned());
            }
        }
        let mut style = String::new();
        for (key, value) in props {
            validate_paint(&value)?;
            style.push_str(&key);
            style.push(':');
            style.push_str(&value);
            style.push(';');
        }
        let range = if let Some(attribute) = node.attribute_node("style") {
            attribute.range()
        } else if !style.is_empty() {
            let start = node.range().start + 1;
            let len = source[start..]
                .iter()
                .position(|byte| byte.is_ascii_whitespace() || matches!(byte, b'/' | b'>'))
                .ok_or_else(|| invalid("invalid SVG opening tag"))?;
            (start + len)..(start + len)
        } else {
            continue;
        };
        let replacement = format!(r#" style="{}""#, escape_attribute(&style));
        charge(
            &mut added_bytes,
            replacement.len(),
            8 * 1024 * 1024,
            "EPUB SVG style expansion exceeds 8 MiB",
        )?;
        output.extend_from_slice(&source[cursor..range.start]);
        output.extend_from_slice(replacement.as_bytes());
        cursor = range.end;
    }
    output.extend_from_slice(&source[cursor..]);
    Ok((output, families))
}

fn svg_property(key: &str) -> bool {
    matches!(
        key,
        "fill"
            | "fill-opacity"
            | "fill-rule"
            | "stroke"
            | "stroke-opacity"
            | "stroke-width"
            | "stroke-linecap"
            | "stroke-linejoin"
            | "stroke-miterlimit"
            | "stroke-dasharray"
            | "stroke-dashoffset"
            | "opacity"
            | "color"
            | "display"
            | "visibility"
            | "clip-path"
            | "clip-rule"
            | "mask"
            | "stop-color"
            | "stop-opacity"
            | "marker-start"
            | "marker-mid"
            | "marker-end"
            | "font-family"
            | "font-size"
            | "font-weight"
            | "font-style"
            | "font-stretch"
            | "text-anchor"
            | "dominant-baseline"
            | "letter-spacing"
            | "word-spacing"
            | "text-decoration"
            | "transform"
            | "paint-order"
            | "font-kerning"
            | "font-feature-settings"
            | "shape-rendering"
            | "text-rendering"
            | "image-rendering"
            | "alignment-baseline"
            | "baseline-shift"
            | "white-space"
    )
}

fn validate_paint(value: &str) -> Result<(), EngineError> {
    if value.contains('\\') {
        return Err(unsupported("escaped SVG CSS values"));
    }
    let lower = value.to_ascii_lowercase();
    if let Some(start) = lower.find("url(") {
        let fragment = value[start + 4..]
            .split_once(')')
            .map(|(url, _)| url.trim().trim_matches(['\'', '"']))
            .and_then(|url| url.strip_prefix('#'));
        if fragment.is_none_or(|id| {
            id.is_empty()
                || !id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"_-.".contains(&byte))
        }) || lower[start + 4..].contains("url(")
        {
            return Err(unsupported(
                "SVG CSS paint requires a same-document fragment",
            ));
        }
    }
    Ok(())
}

type RasterImages<'a> = HashMap<&'a str, (usize, usvg::ImageKind)>;

fn raster_images<'a>(
    root: Node<'a, '_>,
    base: &str,
    book: &mut Book<'_>,
) -> Result<RasterImages<'a>, EngineError> {
    let mut images = HashMap::new();
    let mut pixels = 0;
    let mut encoded = 0;
    let links = root
        .descendants()
        .filter(|node| node.has_tag_name((NS, "image")))
        .filter_map(image_href);
    for (index, href) in links.enumerate() {
        if index >= 256 {
            return Err(limited("EPUB SVG raster image placements exceed 256"));
        }
        if images.len() >= 32 && !images.contains_key(href) {
            return Err(limited("EPUB SVG raster image count exceeds 32"));
        }
        let entry = match images.entry(href) {
            Entry::Occupied(entry) => {
                let &(area, _) = entry.get();
                charge(
                    &mut pixels,
                    area,
                    16_777_216,
                    "EPUB SVG raster images exceed 16 megapixels",
                )?;
                continue;
            }
            Entry::Vacant(entry) => entry,
        };
        let path = resource_path(base, href)?;
        let bytes = book.read(&path, 8 * 1024 * 1024)?;
        if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") && !bytes.starts_with(b"\xff\xd8\xff") {
            return Err(unsupported(
                "EPUB SVG image requires a PNG or JPEG archive resource",
            ));
        }
        let (width, height) = wordcraft_render::image_size(&bytes)
            .ok_or_else(|| invalid("invalid EPUB SVG raster image dimensions"))?;
        if width == 0 || height == 0 || width > 8192 || height > 8192 {
            return Err(limited("EPUB SVG raster image dimensions are out of range"));
        }
        let area = width as usize * height as usize;
        charge(
            &mut pixels,
            area,
            16_777_216,
            "EPUB SVG raster images exceed 16 megapixels",
        )?;
        let png = wordcraft_render::decode_pixmap(&bytes)
            .ok_or_else(|| invalid("invalid EPUB SVG raster image"))?
            .into_png()
            .map_err(invalid)?;
        charge(
            &mut encoded,
            png.len(),
            64 * 1024 * 1024,
            "EPUB SVG raster image data exceeds 64 MiB",
        )?;
        entry.insert((area, usvg::ImageKind::PNG(Arc::new(png))));
    }
    Ok(images)
}

fn font_database(
    faces: &[fonts::Face],
    families: &FontNames,
    book: &mut Book<'_>,
) -> Result<Arc<usvg::fontdb::Database>, EngineError> {
    use usvg::fontdb::{Database, Source, Style, Weight};

    static FONTS: OnceLock<Arc<Database>> = OnceLock::new();
    let mut database = FONTS
        .get_or_init(|| {
            let mut database = Database::new();
            database.load_system_fonts();
            for &font in wordcraft_fonts::bundled() {
                database.load_font_source(Source::Binary(Arc::new(font)));
            }
            database.set_serif_family("Source Serif 4");
            database.set_sans_serif_family("Source Sans 3");
            database.set_monospace_family("JetBrains Mono");
            Arc::new(database)
        })
        .clone();
    let mut bytes = 0;
    for face in faces {
        let Some(names) = families.get(&face.family.to_lowercase()) else {
            continue;
        };
        let (Some(weight), Some(style)) = (face.weight, face.style) else {
            return Err(unsupported("EPUB SVG font-face weight and style ranges"));
        };
        if face.unicode_range {
            return Err(unsupported("EPUB SVG font-face unicode-range"));
        }
        let data = fonts::read_face(face, book)?;
        charge(
            &mut bytes,
            data.len(),
            16 * 1024 * 1024,
            "EPUB SVG decoded fonts exceed 16 MiB",
        )?;
        let database = Arc::make_mut(&mut database);
        let ids = database.load_font_source(Source::Binary(Arc::new(data)));
        if ids.is_empty() {
            return Err(invalid("EPUB SVG font-face has no usable font"));
        }
        for id in ids {
            let mut info = database
                .face(id)
                .cloned()
                .ok_or_else(|| invalid("missing EPUB SVG font face"))?;
            database.remove_face(id);
            info.families = names
                .iter()
                .map(|name| (name.clone(), usvg::fontdb::Language::English_UnitedStates))
                .collect();
            info.style = match style {
                parley::FontStyle::Normal => Style::Normal,
                parley::FontStyle::Italic => Style::Italic,
                parley::FontStyle::Oblique(_) => Style::Oblique,
            };
            info.weight = Weight(weight as u16);
            database.push_face_info(info);
        }
    }
    Ok(database)
}

pub(super) fn transparent_png(
    tree: &usvg::Tree,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, EngineError> {
    let mut image = tiny_skia::Pixmap::new(width, height)
        .ok_or_else(|| limited("cannot allocate embedded SVG image"))?;
    let scale = tiny_skia::Transform::from_scale(
        width as f32 / tree.size().width(),
        height as f32 / tree.size().height(),
    );
    resvg::render(tree, scale, &mut image.as_mut());
    image.encode_png().map_err(invalid)
}
