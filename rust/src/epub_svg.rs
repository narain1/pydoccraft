//! Rasterize a validated EPUB SVG content document on its declared viewport.
use std::{
    collections::{hash_map::Entry, HashMap},
    sync::Arc,
};

use resvg::{tiny_skia, usvg};
use roxmltree::Node;

use super::{
    charge, content, css::Cascade, external, invalid, limited, rendition::Region, resolve,
    unsupported, Book,
};
use crate::{author_epub::escape_attribute, model::ImagePayload, EngineError};

#[path = "epub_svg_effects.rs"]
pub(super) mod effects;
#[path = "epub_svg_links.rs"]
mod links;
#[path = "epub_svg_fonts.rs"]
mod svg_fonts;
pub(super) use links::Geometry as LinkGeometry;
#[path = "epub_svg_use.rs"]
pub(super) mod use_refs;

pub(super) const NS: &str = "http://www.w3.org/2000/svg";
pub(super) const XLINK: &str = "http://www.w3.org/1999/xlink";

pub(super) fn fragment_id(value: &str) -> Option<&str> {
    value.strip_prefix('#').filter(|id| {
        !id.is_empty()
            && id
                .chars()
                .all(|ch| ch.is_alphanumeric() || matches!(ch, '-' | '_' | '.' | ':'))
    })
}

fn unqualified_attribute<'a>(node: Node<'a, '_>, name: &str) -> Option<&'a str> {
    node.attributes()
        .find(|attr| attr.namespace().is_none() && attr.name() == name)
        .map(|attr| attr.value())
}

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
    let reference = stylesheet_reference(pi)?;
    Ok((reference.css && !reference.alternate).then_some((reference.href, reference.media)))
}

pub(super) struct StylesheetReference {
    pub href: String,
    pub css: bool,
    media: String,
    alternate: bool,
}

pub(super) fn stylesheet_reference(
    pi: roxmltree::PI<'_>,
) -> Result<StylesheetReference, EngineError> {
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
    Ok(StylesheetReference {
        href,
        css: values
            .get("type")
            .is_none_or(|value| value.eq_ignore_ascii_case("text/css")),
        media: values.remove("media").unwrap_or_default(),
        alternate: values.get("alternate").is_some_and(|value| value == "yes"),
    })
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
    base: &str,
    book: &mut Book<'_>,
    fixed: bool,
    dpi: f64,
    region: Region,
    mut navigation: Option<&mut super::rendition::Capture<'_, '_>>,
) -> Result<wordcraft_render::Rendered, EngineError> {
    let viewport = viewport(root);
    if fixed && viewport.is_none() {
        return Err(unsupported("fixed-layout SVG requires a numeric viewBox"));
    }
    let links = navigation.is_some().then(|| links::Links::new(root));
    let (tree, geometry) = parse(
        root,
        root.document().input_text().as_bytes(),
        base,
        book,
        viewport,
        links.as_ref(),
    )?;
    let [w, h] = viewport.unwrap_or([tree.size().width() * 0.75, tree.size().height() * 0.75]);
    if let Some((capture, geometry)) = navigation.as_mut().zip(geometry.as_ref()) {
        let clip = wordcraft_geom::Rect::new(0.0, 0.0, w, h);
        geometry.place(
            kurbo::Affine::scale_non_uniform(
                f64::from(w / tree.size().width()),
                f64::from(h / tree.size().height()),
            ),
            clip,
            capture,
        )?;
        if let Some(text) = capture.text.as_deref_mut() {
            collect_image_text(&tree, [w, h], text)?;
        }
    }
    let scale = dpi / 72.0;
    let region = region.for_page([f64::from(w), f64::from(h)], dpi, fixed)?;
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
    effects::validate_raster(&tree, transform)?;
    resvg::render(&tree, transform, &mut image.as_mut());
    Ok(wordcraft_render::Rendered {
        width: region.width,
        height: region.height,
        pixels: image.take(),
    })
}

pub(super) fn collect_image_text(
    tree: &usvg::Tree,
    size: [f32; 2],
    output: &mut super::rendition::text::TextLayer,
) -> Result<(), EngineError> {
    let clip = wordcraft_geom::Rect::new(0.0, 0.0, size[0], size[1]);
    let scale = [
        size[0] / tree.size().width(),
        size[1] / tree.size().height(),
    ];
    let mut metrics = HashMap::new();
    visit_painted_svg(tree, scale, clip, |node, clip| {
        if let usvg::Node::Text(source) = node {
            capture_svg_text(tree.fontdb(), source, scale, clip, &mut metrics, output)?;
        }
        Ok(())
    })
}

fn visit_painted_svg<'tree>(
    tree: &'tree usvg::Tree,
    scale: [f32; 2],
    clip: wordcraft_geom::Rect,
    mut visit: impl FnMut(&'tree usvg::Node, wordcraft_geom::Rect) -> Result<(), EngineError>,
) -> Result<(), EngineError> {
    let geometry = |rect: usvg::Rect| {
        wordcraft_geom::Rect::new(
            rect.x() * scale[0],
            rect.y() * scale[1],
            rect.width() * scale[0],
            rect.height() * scale[1],
        )
    };
    let mut stack = vec![(tree.root().children().iter(), clip)];
    while let Some((children, parent_clip)) = stack.last_mut() {
        let Some(node) = children.next() else {
            stack.pop();
            continue;
        };
        let mut clip = *parent_clip;
        if let usvg::Node::Group(group) = node {
            if group.opacity().get() == 0.0 {
                continue;
            }
            if group.clip_path().is_some() {
                let Some(bounds) = group.abs_layer_bounding_box() else {
                    continue;
                };
                clip = super::rendition::intersection(clip, geometry(bounds.to_rect()));
            }
            stack.push((group.children().iter(), clip));
        }
        visit(node, clip)?;
    }
    Ok(())
}

type SvgGlyphMetrics = HashMap<(usvg::fontdb::ID, u32), Option<[f32; 3]>>;

fn capture_svg_text(
    fonts: &usvg::fontdb::Database,
    node: &usvg::Text,
    scale: [f32; 2],
    clip: wordcraft_geom::Rect,
    metrics: &mut SvgGlyphMetrics,
    output: &mut super::rendition::text::TextLayer,
) -> Result<(), EngineError> {
    visit_glyphs(
        fonts,
        node,
        scale,
        metrics,
        |_| true,
        |_, text, rect| output.add(text, rect, clip),
    )
}

fn visit_glyphs(
    fonts: &usvg::fontdb::Database,
    node: &usvg::Text,
    scale: [f32; 2],
    metrics: &mut SvgGlyphMetrics,
    select: impl Fn(usize) -> bool,
    mut visit: impl FnMut(usize, &str, wordcraft_geom::Rect) -> Result<(), EngineError>,
) -> Result<(), EngineError> {
    use skrifa::{
        instance::{LocationRef, Size},
        FontRef, GlyphId, MetadataProvider,
    };
    for (index, span) in node.layouted().iter().enumerate() {
        if !select(index)
            || !span.visible
            || span
                .fill
                .as_ref()
                .is_none_or(|fill| fill.opacity().get() == 0.0)
                && span
                    .stroke
                    .as_ref()
                    .is_none_or(|stroke| stroke.opacity().get() == 0.0)
        {
            continue;
        }
        for glyph in &span.positioned_glyphs {
            let Some([advance, ascent, descent]) =
                *metrics.entry((glyph.font, glyph.id.0)).or_insert_with(|| {
                    fonts
                        .with_face_data(glyph.font, |data, index| {
                            let font = FontRef::from_index(data, index).ok()?;
                            let size = Size::unscaled();
                            let location = LocationRef::default();
                            let metrics = font.metrics(size, location);
                            let advance = font
                                .glyph_metrics(size, location)
                                .advance_width(GlyphId::new(glyph.id.0))?;
                            Some([advance, metrics.ascent, metrics.descent])
                        })
                        .flatten()
                })
            else {
                continue;
            };
            if advance <= 0.0 {
                // Nonspacing marks share their source cluster with the base glyph.
                continue;
            }
            let mapping = node.abs_transform().pre_concat(glyph.outline_transform());
            let page = kurbo::Affine::scale_non_uniform(scale[0].into(), scale[1].into())
                * kurbo::Affine::new(
                    [
                        mapping.sx, mapping.ky, mapping.kx, mapping.sy, mapping.tx, mapping.ty,
                    ]
                    .map(f64::from),
                );
            let rect = super::rendition::text::transformed(
                wordcraft_geom::Rect::new(0.0, descent, advance, ascent - descent),
                page,
            );
            visit(index, &glyph.text, rect)?;
        }
    }
    Ok(())
}

pub(super) fn embedded(
    source: &[u8],
    base: &str,
    book: &mut Book<'_>,
) -> Result<usvg::Tree, EngineError> {
    embedded_with_links(source, base, book, false).map(|(tree, _)| tree)
}

pub(super) fn embedded_with_links(
    source: &[u8],
    base: &str,
    book: &mut Book<'_>,
    capture_links: bool,
) -> Result<(usvg::Tree, Option<LinkGeometry>), EngineError> {
    let xml = super::parse_xml(source)?;
    let root = xml.root_element();
    let links = capture_links.then(|| links::Links::new(root));
    parse(root, source, base, book, viewport(root), links.as_ref())
}

fn parse(
    root: Node<'_, '_>,
    source: &[u8],
    base: &str,
    book: &mut Book<'_>,
    viewport: Option<[f32; 2]>,
    links: Option<&links::Links<'_>>,
) -> Result<(usvg::Tree, Option<LinkGeometry>), EngineError> {
    let has_uses = content::validate_svg(root, Some(base))?;
    let images = raster_images(root, base, book)?;
    let mut options = usvg::Options {
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(move |href, _| {
                images.get(href).map(|(_, image)| image.clone())
            }),
        },
        max_depth: use_refs::MAX_DEPTH as u32,
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
    let (styled, families) = apply_styles(root, source, &mut cascade, links)?;
    let fonts = if root
        .descendants()
        .any(|node| node.has_tag_name((NS, "text")))
    {
        Some(svg_fonts::Resolver::install(
            &mut options,
            &cascade.fonts,
            families,
            book,
        )?)
    } else {
        None
    };
    let result = native_tree(&styled, &options, has_uses);
    if let Some(fonts) = &fonts {
        fonts.check()?;
    }
    let mut tree = result?;
    let fallback = if let Some(fonts) = &fonts {
        if let Some(fallback) = fonts.split_fallback_runs(&styled, &tree)? {
            options.fontdb = tree.fontdb().clone();
            let result = native_tree(fallback.as_bytes(), &options, has_uses);
            fonts.check()?;
            tree = result?;
            fonts.check_rendered(&tree)?;
            Some(fallback)
        } else {
            None
        }
    } else {
        None
    };
    options.fontdb = tree.fontdb().clone();
    let geometry = links
        .map(|links| {
            links.collect(
                fallback.as_ref().map_or(&styled, |xml| xml.as_bytes()),
                &tree,
                &options,
                has_uses,
            )
        })
        .transpose()?;
    if let Some(fonts) = &fonts {
        fonts.check()?;
    }
    Ok((tree, geometry))
}

fn native_tree(
    source: &[u8],
    options: &usvg::Options<'_>,
    has_uses: bool,
) -> Result<usvg::Tree, EngineError> {
    if has_uses {
        // Count computed style payload too, since usvg copies it into each clone.
        let xml = super::parse_xml(source)?;
        use_refs::validate_use_graph(xml.root_element())?;
        usvg::Tree::from_xmltree(&xml, options).map_err(invalid)
    } else {
        usvg::Tree::from_data(source, options).map_err(invalid)
    }
}

fn apply_styles(
    root: Node<'_, '_>,
    source: &[u8],
    cascade: &mut Cascade,
    links: Option<&links::Links<'_>>,
) -> Result<(Vec<u8>, svg_fonts::Families), EngineError> {
    let mut output = Vec::with_capacity(source.len());
    let mut cursor = 0;
    let mut families = svg_fonts::Families::new(&cascade.fonts);
    let mut typography = HashMap::<roxmltree::NodeId, svg_fonts::CssFont>::new();
    let mut added_bytes = 0;
    let mut effects = None;
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
        let mut style = String::new();
        let mut encoded = family
            .map(|value| families.encode(value))
            .transpose()?
            .flatten();
        if families.tracks_variations() {
            let mut text = node
                .parent_element()
                .and_then(|parent| typography.get(&parent.id()))
                .cloned()
                .unwrap_or_default();
            if let Some(family) = &encoded {
                text.apply("font-family", family);
            }
            for key in [
                "font-weight",
                "font-stretch",
                "font-style",
                "font-variation-settings",
                "font-optical-sizing",
            ] {
                let value = props
                    .iter()
                    .find(|(name, _)| name == key)
                    .map(|(_, value)| value.as_str())
                    .or_else(|| node.attribute(key));
                if let Some(value) = value {
                    text.apply(key, value);
                }
            }
            if matches!(node.tag_name().name(), "text" | "tspan" | "a") {
                if let Some(request) = families.variant(&text)? {
                    encoded = Some(request);
                }
            }
            typography.insert(node.id(), text);
        }
        if let Some(encoded) = encoded {
            style.push_str("font-family:");
            style.push_str(&encoded);
            style.push(';');
        }
        props.retain(|(key, _)| key != "font-family");
        for (key, mut value) in props {
            if key == "font-optical-sizing" {
                value = match super::css::optical_sizing(&value) {
                    Some(true) => "auto",
                    Some(false) => "none",
                    None => continue,
                }
                .into();
            } else if key == "font-variation-settings" {
                match value.to_ascii_lowercase().as_str() {
                    "inherit" | "unset" | "revert" | "revert-layer" => continue,
                    "initial" => value = "normal".into(),
                    _ => {}
                }
            }
            if key == "filter" {
                value = effects
                    .get_or_insert_with(|| effects::Effects::new(root))
                    .validate_filter(node, &value)?
                    .map_or_else(|| "none".into(), |id| format!("url(#{id})"));
            } else {
                validate_paint(&value)?;
            }
            style.push_str(&key);
            style.push(':');
            style.push_str(&value);
            style.push(';');
        }
        let id = links.and_then(|links| links.generated_ids.get(&node.id()));
        let range = if let Some(attribute) = node.attribute_node("style") {
            attribute.range()
        } else if id.is_some() || !style.is_empty() {
            style_insertion(source, node)?
        } else {
            continue;
        };
        let mut replacement = format!(r#" style="{}""#, escape_attribute(&style));
        if let Some(id) = id {
            replacement.push_str(&format!(r#" id="{id}""#));
        }
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

fn style_insertion(
    source: &[u8],
    node: Node<'_, '_>,
) -> Result<std::ops::Range<usize>, EngineError> {
    let start = node.range().start + 1;
    let len = source[start..]
        .iter()
        .position(|byte| byte.is_ascii_whitespace() || matches!(byte, b'/' | b'>'))
        .ok_or_else(|| invalid("invalid SVG opening tag"))?;
    Ok((start + len)..(start + len))
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
            | "filter"
            | "color-interpolation-filters"
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
            | "font-variation-settings"
            | "font-optical-sizing"
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
    effects::validate_raster(tree, scale)?;
    resvg::render(tree, scale, &mut image.as_mut());
    image.encode_png().map_err(invalid)
}
