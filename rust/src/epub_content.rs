//! Normalize one bounded XHTML tree while keeping source order and ownership.
use std::collections::HashMap;

use base64::Engine;
use roxmltree::Node;
use serde_json::json;

use super::{
    attribute, charge, child, external, invalid, limited, name, parse_xml, resolve, unsupported,
    Book, XHTML, XML,
};
use crate::{
    model::{BlockPayload, CellPayload, ImagePayload, LinkPayload, PagePayload, TablePayload},
    EngineError,
};

const SVG: &str = "http://www.w3.org/2000/svg";
const MATHML: &str = "http://www.w3.org/1998/Math/MathML";
const MAX_IMAGE: usize = 16 * 1024 * 1024;
const MAX_MODEL: usize = 48 * 1024 * 1024;
const MAX_ITEMS: usize = 50_000;
const MAX_GRID: usize = 100_000;
const MAX_COLUMNS: usize = 1000;

#[derive(Default)]
pub(super) struct Budget {
    bytes: usize,
    objects: usize,
    grid: usize,
}

impl Budget {
    fn bytes(&mut self, bytes: usize) -> Result<(), EngineError> {
        charge(
            &mut self.bytes,
            bytes,
            MAX_MODEL,
            "semantic output exceeds its byte budget",
        )
    }

    fn object(&mut self) -> Result<(), EngineError> {
        charge(
            &mut self.objects,
            1,
            MAX_ITEMS,
            "semantic output has too many items",
        )?;
        self.bytes(128)
    }
}

struct Context<'a, 'book> {
    source: &'a [u8],
    path: &'a str,
    page: usize,
    manifest: &'a HashMap<String, &'a str>,
    book: &'a mut Book<'book>,
    budget: &'a mut Budget,
}

#[derive(Default)]
struct Inline {
    text: String,
    characters: usize,
    links: Vec<LinkPayload>,
    images: Vec<ImagePayload>,
    math: Vec<serde_json::Value>,
}

impl Inline {
    fn push(&mut self, text: &str, budget: &mut Budget) -> Result<(), EngineError> {
        budget.bytes(text.len())?;
        self.text.push_str(text);
        self.characters += text.chars().count();
        Ok(())
    }

    fn flush(
        &mut self,
        out: &mut Vec<BlockPayload>,
        level: Option<u8>,
        kind: Option<&str>,
        cx: &mut Context<'_, '_>,
    ) -> Result<(), EngineError> {
        let inline = std::mem::take(self);
        if inline.text.trim().is_empty() && inline.images.is_empty() && inline.math.is_empty() {
            return Ok(());
        }
        cx.budget.object()?;
        let mut block = BlockPayload {
            kind: if level.is_some() {
                "heading"
            } else {
                "paragraph"
            }
            .into(),
            text: inline.text,
            level,
            links: inline.links,
            images: inline.images,
            ..Default::default()
        };
        if let Some(kind) = kind {
            block.metadata.insert("xhtml_kind".into(), json!(kind));
        }
        if !inline.math.is_empty() {
            block
                .metadata
                .insert("mathml".into(), serde_json::Value::Array(inline.math));
        }
        out.push(block);
        Ok(())
    }
}

pub(super) fn chapter(
    root: Node<'_, '_>,
    source: &[u8],
    page: usize,
    path: &str,
    manifest: &HashMap<String, &str>,
    book: &mut Book<'_>,
    budget: &mut Budget,
) -> Result<PagePayload, EngineError> {
    if !name(root, XHTML, "html") {
        return Err(invalid("spine entry is not XHTML"));
    }
    let body = child(root, XHTML, "body").ok_or_else(|| invalid("XHTML chapter has no body"))?;
    let mut cx = Context {
        source,
        path,
        page,
        manifest,
        book,
        budget,
    };
    let mut blocks = Vec::new();
    flow(body, &mut blocks, 0, &mut cx)?;
    let title = child(root, XHTML, "head")
        .and_then(|head| child(head, XHTML, "title"))
        .map(node_text)
        .filter(|title| !title.trim().is_empty())
        .or_else(|| {
            blocks
                .iter()
                .find(|block| block.level.is_some())
                .map(|block| block.text.clone())
        });
    let title = title.as_deref().map(str::trim).unwrap_or("Untitled");
    cx.budget.bytes(title.len().saturating_mul(3))?;
    let mut result = PagePayload {
        number: page,
        blocks,
        ..Default::default()
    };
    result.metadata.insert("chapter_title".into(), json!(title));
    if let Some(language) = root
        .attribute((XML, "lang"))
        .or_else(|| root.attribute("lang"))
    {
        cx.budget.bytes(language.len().saturating_mul(3))?;
        result.metadata.insert("language".into(), json!(language));
    }
    assets(&result.blocks, &mut result.links, &mut result.images);
    for block in &result.blocks {
        if let Some(table) = &block.table {
            result.tables.push(table.clone());
        }
    }
    Ok(result)
}

fn assets(blocks: &[BlockPayload], links: &mut Vec<LinkPayload>, images: &mut Vec<ImagePayload>) {
    for block in blocks {
        links.extend(block.links.iter().cloned());
        images.extend(block.images.iter().cloned());
        if let Some(table) = &block.table {
            for cell in table.cells.iter().flatten() {
                assets(&cell.blocks, links, images);
            }
        }
    }
}

fn node_text(node: Node<'_, '_>) -> String {
    node.descendants()
        .filter(Node::is_text)
        .filter_map(|n| n.text())
        .collect()
}

fn heading(tag: &str) -> Option<u8> {
    tag.strip_prefix('h')
        .and_then(|n| n.parse().ok())
        .filter(|n| (1..=6).contains(n))
}

fn block(tag: &str) -> bool {
    matches!(
        tag,
        "p" | "section"
            | "article"
            | "main"
            | "aside"
            | "nav"
            | "header"
            | "footer"
            | "address"
            | "div"
            | "pre"
            | "ol"
            | "ul"
            | "li"
            | "dl"
            | "dt"
            | "dd"
            | "blockquote"
            | "figure"
            | "figcaption"
            | "table"
    ) || heading(tag).is_some()
}

fn ignored(tag: &str) -> bool {
    matches!(tag, "style" | "script" | "template" | "head")
}

fn flow(
    node: Node<'_, '_>,
    out: &mut Vec<BlockPayload>,
    table_depth: usize,
    cx: &mut Context<'_, '_>,
) -> Result<(), EngineError> {
    let tag = node.tag_name().name();
    let level = heading(tag);
    let kind = matches!(
        tag,
        "pre" | "li" | "blockquote" | "figcaption" | "dt" | "dd"
    )
    .then_some(tag);
    let mut run = Inline::default();
    for n in node.children() {
        let child_tag = n.tag_name();
        if n.is_element()
            && child_tag.namespace().is_none_or(|ns| ns == XHTML)
            && block(child_tag.name())
        {
            run.flush(out, level, kind, cx)?;
            if child_tag.name() == "table" {
                table(n, out, table_depth, cx)?;
            } else {
                flow(n, out, table_depth, cx)?;
            }
        } else if n.is_element()
            && name(n, XHTML, "a")
            && n.children()
                .any(|n| n.is_element() && block(n.tag_name().name()))
        {
            run.flush(out, level, kind, cx)?;
            let start = out.len();
            flow(n, out, table_depth, cx)?;
            if let Some(href) = n.attribute("href") {
                link_blocks(&mut out[start..], href, cx)?;
            }
        } else {
            inline(n, &mut run, cx)?;
        }
    }
    run.flush(out, level, kind, cx)
}

fn link_blocks(
    blocks: &mut [BlockPayload],
    href: &str,
    cx: &mut Context<'_, '_>,
) -> Result<(), EngineError> {
    for block in blocks {
        if let Some(table) = &mut block.table {
            for cell in table.cells.iter_mut().flatten() {
                link_blocks(&mut cell.blocks, href, cx)?;
            }
        } else {
            if !block.links.is_empty() {
                return Err(invalid("nested links"));
            }
            let link = link(href, &block.text, 0, block.text.chars().count(), cx)?;
            for image in &mut block.images {
                cx.budget.bytes(href.len().saturating_mul(3))?;
                image.metadata.insert("href".into(), json!(href));
            }
            block.links.push(link);
        }
    }
    Ok(())
}

fn inline(
    node: Node<'_, '_>,
    run: &mut Inline,
    cx: &mut Context<'_, '_>,
) -> Result<(), EngineError> {
    if node.is_text() {
        return run.push(node.text().unwrap_or_default(), cx.budget);
    }
    if !node.is_element() {
        return Ok(());
    }
    let tag = node.tag_name();
    if tag.namespace() == Some(SVG) && tag.name() == "svg" {
        let bytes = standalone(node, cx.source)?;
        let mut image = svg_image(node, &bytes, cx.page, "inline.svg", cx.budget)?;
        image
            .metadata
            .insert("inline_offset".into(), json!(run.characters));
        run.images.push(image);
        return Ok(());
    }
    if tag.namespace() == Some(MATHML) && tag.name() == "math" {
        cx.budget.object()?;
        let markup = standalone(node, cx.source)?;
        cx.budget.bytes(markup.len())?;
        let text = node_text(node);
        let start = run.characters;
        run.push(&text, cx.budget)?;
        run.math.push(json!({"start": start, "end": run.characters, "source": String::from_utf8(markup).map_err(invalid)?}));
        return Ok(());
    }
    if tag.namespace().is_some_and(|ns| ns != XHTML) {
        return Err(unsupported("unknown XHTML extension element"));
    }
    match tag.name() {
        tag if ignored(tag) => return Ok(()),
        "br" => return run.push("\n", cx.budget),
        "img" => {
            let alt = node.attribute("alt").unwrap_or_default();
            if let Some(src) = node.attribute("src") {
                let mut image = image(src, alt, None, cx)?;
                image
                    .metadata
                    .insert("inline_offset".into(), json!(run.characters));
                run.images.push(image);
            } else {
                run.push(alt, cx.budget)?;
            }
            return Ok(());
        }
        "object" => {
            let mime = node
                .attribute("type")
                .filter(|mime| mime.starts_with("image/"))
                .ok_or_else(|| unsupported("embedded XHTML objects require an image type"))?;
            let data = attribute(node, "data")?;
            let alt = node_text(node);
            let mut image = image(data, &alt, Some(mime), cx)?;
            image
                .metadata
                .insert("inline_offset".into(), json!(run.characters));
            run.images.push(image);
            return Ok(());
        }
        "audio" | "video" | "iframe" | "embed" | "canvas" | "picture" => {
            return Err(unsupported(
                "embedded XHTML media or alternate image sources",
            ));
        }
        _ => {}
    }
    let start = run.characters;
    let start_byte = run.text.len();
    let first_image = run.images.len();
    let first_link = run.links.len();
    for child in node.children() {
        inline(child, run, cx)?;
    }
    if let Some(href) = (tag.name() == "a")
        .then(|| node.attribute("href"))
        .flatten()
    {
        if run.links.len() != first_link {
            return Err(invalid("nested links"));
        }
        for image in &mut run.images[first_image..] {
            cx.budget.bytes(href.len().saturating_mul(3))?;
            image.metadata.insert("href".into(), json!(href));
        }
        if run.characters > start || run.images.len() > first_image {
            run.links.push(link(
                href,
                &run.text[start_byte..],
                start,
                run.characters,
                cx,
            )?);
        }
    }
    Ok(())
}

fn link(
    href: &str,
    label: &str,
    start: usize,
    end: usize,
    cx: &mut Context<'_, '_>,
) -> Result<LinkPayload, EngineError> {
    cx.budget.object()?;
    cx.budget
        .bytes(href.len().saturating_add(label.len()).saturating_mul(3))?;
    let mut link = LinkPayload {
        uri: href.into(),
        text: label.into(),
        page: Some(cx.page),
        ..Default::default()
    };
    link.metadata.insert("start".into(), json!(start));
    link.metadata.insert("end".into(), json!(end));
    let is_external = external(href);
    link.metadata.insert(
        "target_kind".into(),
        json!(if is_external { "external" } else { "internal" }),
    );
    if !is_external {
        let path = resolve(cx.path, href)?;
        cx.budget.bytes(path.len().saturating_mul(3))?;
        link.metadata.insert("target_path".into(), json!(path));
        if let Some((_, fragment)) = href.split_once('#') {
            link.metadata.insert(
                "target_fragment".into(),
                json!(super::percent_decode(fragment)?),
            );
        }
    }
    Ok(link)
}

fn image(
    src: &str,
    alt: &str,
    declared: Option<&str>,
    cx: &mut Context<'_, '_>,
) -> Result<ImagePayload, EngineError> {
    if external(src) {
        return Err(unsupported("remote images are not fetched"));
    }
    let path = resolve(cx.path, src)?;
    let mime = declared
        .or_else(|| cx.manifest.get(&path).copied())
        .or_else(
            || match path.rsplit('.').next()?.to_ascii_lowercase().as_str() {
                "png" => Some("image/png"),
                "jpg" | "jpeg" => Some("image/jpeg"),
                "gif" => Some("image/gif"),
                "svg" => Some("image/svg+xml"),
                "webp" => Some("image/webp"),
                _ => None,
            },
        )
        .ok_or_else(|| unsupported("unknown image type"))?;
    if let Some(manifest_mime) = cx.manifest.get(&path) {
        if *manifest_mime != mime {
            return Err(invalid("image type differs from manifest"));
        }
    }
    let bytes = cx.book.read(&path, MAX_IMAGE)?;
    if mime == "image/svg+xml" {
        let d = parse_xml(&bytes)?;
        let mut image = svg_image(d.root_element(), &bytes, cx.page, src, cx.budget)?;
        image.alt = alt.into();
        return Ok(image);
    }
    let valid = match mime {
        "image/png" => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "image/jpeg" => bytes.starts_with(b"\xff\xd8\xff"),
        "image/gif" => bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a"),
        "image/webp" => bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP"),
        _ => return Err(unsupported("image media type")),
    };
    if !valid {
        return Err(invalid("image signature differs from its media type"));
    }
    encoded_image(&bytes, cx.page, src, mime, alt, cx.budget)
}

fn encoded_image(
    bytes: &[u8],
    page: usize,
    path: &str,
    mime: &str,
    alt: &str,
    budget: &mut Budget,
) -> Result<ImagePayload, EngineError> {
    budget.object()?;
    budget.bytes((bytes.len().div_ceil(3) * 4 + path.len() + alt.len()).saturating_mul(3))?;
    Ok(ImagePayload {
        name: path.into(),
        data: base64::engine::general_purpose::STANDARD.encode(bytes),
        media_type: mime.into(),
        page: Some(page),
        alt: alt.into(),
        ..Default::default()
    })
}

pub(super) fn svg_page(
    root: Node<'_, '_>,
    data: &[u8],
    page: usize,
    path: &str,
    budget: &mut Budget,
) -> Result<PagePayload, EngineError> {
    let image = svg_image(root, data, page, path, budget)?;
    Ok(PagePayload {
        number: page,
        images: vec![image.clone()],
        blocks: vec![BlockPayload {
            kind: "paragraph".into(),
            images: vec![image],
            ..Default::default()
        }],
        ..Default::default()
    })
}

fn svg_image(
    root: Node<'_, '_>,
    data: &[u8],
    page: usize,
    path: &str,
    budget: &mut Budget,
) -> Result<ImagePayload, EngineError> {
    validate_svg(root)?;
    let alt = child(root, SVG, "title").map(node_text).unwrap_or_default();
    encoded_image(data, page, path, "image/svg+xml", &alt, budget)
}

pub(super) fn validate_svg(root: Node<'_, '_>) -> Result<(), EngineError> {
    if root.tag_name().namespace() != Some(SVG) || root.tag_name().name() != "svg" {
        return Err(invalid("image is not SVG"));
    }
    if root.document().descendants().any(|node| node.is_pi()) {
        return Err(unsupported("SVG processing instructions"));
    }
    for node in root.descendants().filter(Node::is_element) {
        if node.tag_name().namespace() != Some(SVG)
            || !matches!(
                node.tag_name().name(),
                "svg"
                    | "g"
                    | "defs"
                    | "symbol"
                    | "title"
                    | "desc"
                    | "path"
                    | "rect"
                    | "circle"
                    | "ellipse"
                    | "line"
                    | "polyline"
                    | "polygon"
                    | "text"
                    | "tspan"
                    | "linearGradient"
                    | "radialGradient"
                    | "stop"
                    | "clipPath"
                    | "mask"
                    | "pattern"
                    | "marker"
            )
        {
            return Err(unsupported("SVG contains unsupported or active elements"));
        }
        for attribute in node.attributes() {
            let key = attribute.name();
            let value = attribute.value();
            let static_attribute = match attribute.namespace() {
                Some(XML) => matches!(key, "lang" | "space"),
                None => matches!(
                    key,
                    "id" | "role"
                        | "aria-label"
                        | "aria-hidden"
                        | "version"
                        | "viewBox"
                        | "preserveAspectRatio"
                        | "width"
                        | "height"
                        | "x"
                        | "y"
                        | "x1"
                        | "x2"
                        | "y1"
                        | "y2"
                        | "cx"
                        | "cy"
                        | "r"
                        | "rx"
                        | "ry"
                        | "fx"
                        | "fy"
                        | "fr"
                        | "d"
                        | "points"
                        | "pathLength"
                        | "transform"
                        | "fill"
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
                        | "clipPathUnits"
                        | "mask"
                        | "maskUnits"
                        | "maskContentUnits"
                        | "gradientUnits"
                        | "gradientTransform"
                        | "spreadMethod"
                        | "offset"
                        | "stop-color"
                        | "stop-opacity"
                        | "patternUnits"
                        | "patternContentUnits"
                        | "patternTransform"
                        | "marker"
                        | "marker-start"
                        | "marker-mid"
                        | "marker-end"
                        | "markerUnits"
                        | "markerWidth"
                        | "markerHeight"
                        | "orient"
                        | "refX"
                        | "refY"
                        | "dx"
                        | "dy"
                        | "rotate"
                        | "font-family"
                        | "font-size"
                        | "font-weight"
                        | "font-style"
                        | "text-anchor"
                        | "textLength"
                        | "lengthAdjust"
                        | "dominant-baseline"
                        | "letter-spacing"
                        | "word-spacing"
                ),
                Some(_) => false,
            };
            // Paint references may resolve only to a simple same-document fragment.
            let local_paint = matches!(
                key,
                "fill"
                    | "stroke"
                    | "clip-path"
                    | "mask"
                    | "marker"
                    | "marker-start"
                    | "marker-mid"
                    | "marker-end"
            ) && value
                .strip_prefix("url(#")
                .and_then(|v| v.strip_suffix(')'))
                .is_some_and(|v| {
                    !v.is_empty()
                        && v.bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"_-.".contains(&b))
                });
            if !static_attribute
                || value.contains('\\')
                || (value.to_ascii_lowercase().contains("url(") && !local_paint)
            {
                return Err(unsupported("SVG contains external or active attributes"));
            }
        }
    }
    Ok(())
}

// Reattach inherited namespace bindings to a detached fragment.
fn standalone(node: Node<'_, '_>, data: &[u8]) -> Result<Vec<u8>, EngineError> {
    let raw = data
        .get(node.range())
        .ok_or_else(|| invalid("invalid XML fragment range"))?;
    let mut reader = quick_xml::Reader::from_reader(raw);
    let start = match reader.read_event().map_err(invalid)? {
        quick_xml::events::Event::Start(start) | quick_xml::events::Event::Empty(start) => start,
        _ => return Err(invalid("invalid XML fragment opening tag")),
    };
    let declared = start
        .attributes()
        .map(|a| a.map(|a| a.key.as_ref().to_vec()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(invalid)?;
    let mut declarations = String::new();
    for ns in node.namespaces().filter(|ns| ns.name() != Some("xml")) {
        let key = ns
            .name()
            .map_or("xmlns".to_owned(), |n| format!("xmlns:{n}"));
        if !declared.iter().any(|existing| existing == key.as_bytes()) {
            let uri = ns
                .uri()
                .replace('&', "&amp;")
                .replace('"', "&quot;")
                .replace('<', "&lt;");
            declarations.push_str(&format!(" {key}=\"{uri}\""));
        }
    }
    let pos = start.name().as_ref().len() + 1;
    let mut output = Vec::with_capacity(raw.len() + declarations.len());
    output.extend_from_slice(&raw[..pos]);
    output.extend_from_slice(declarations.as_bytes());
    output.extend_from_slice(&raw[pos..]);
    Ok(output)
}

fn table(
    node: Node<'_, '_>,
    out: &mut Vec<BlockPayload>,
    depth: usize,
    cx: &mut Context<'_, '_>,
) -> Result<(), EngineError> {
    if depth >= 8 {
        return Err(limited("nested tables exceed depth 8"));
    }
    let mut source = Vec::new();
    let mut groups = Vec::new();
    let mut direct_start = None;
    let mut columns = Vec::new();
    let mut caption = false;
    for n in node.children() {
        if n.is_text() && !n.text().unwrap_or_default().trim().is_empty() {
            return Err(invalid("text outside EPUB table cells"));
        }
        if !n.is_element() {
            continue;
        }
        if n.tag_name().namespace().is_some_and(|ns| ns != XHTML) {
            return Err(unsupported("non-XHTML table children"));
        }
        let tag = n.tag_name().name();
        if tag != "tr" {
            finish_direct(&mut groups, &mut direct_start, source.len());
        }
        match tag {
            "caption" if !caption && source.is_empty() => {
                flow(n, out, depth + 1, cx)?;
                caption = true;
            }
            "colgroup" if source.is_empty() => {
                cx.budget.object()?;
                let mut cols = Vec::new();
                for col in n.children().filter(Node::is_element) {
                    if !name(col, XHTML, "col") {
                        return Err(invalid("invalid colgroup child"));
                    }
                    cx.budget.object()?;
                    cols.push(column(col, cx.budget)?);
                }
                let mut group = column(n, cx.budget)?;
                group["columns"] = serde_json::Value::Array(cols);
                columns.push(group);
            }
            "tr" => {
                direct_start.get_or_insert(source.len());
                source.push(n);
            }
            "thead" | "tbody" | "tfoot" => {
                let start = source.len();
                for row in n.children().filter(Node::is_element) {
                    if !name(row, XHTML, "tr") {
                        return Err(invalid("invalid table row group child"));
                    }
                    source.push(row);
                }
                groups.push((tag, start, source.len(), n.attribute("id")));
            }
            _ => return Err(unsupported("table structure cannot be represented")),
        }
        if source.len() > 10_000 {
            return Err(limited("table row count exceeds 10000"));
        }
    }
    finish_direct(&mut groups, &mut direct_start, source.len());
    let mut occupied = vec![0usize; MAX_COLUMNS];
    let mut result = TablePayload {
        page: Some(cx.page),
        ..Default::default()
    };
    let mut width = 0;
    let mut html_groups = Vec::new();
    for (kind, start, end, id) in groups {
        if kind != "implicit" {
            cx.budget.object()?;
            cx.budget.bytes(id.map_or(0, str::len).saturating_mul(3))?;
            html_groups
                .push(json!({"kind": kind, "start_row": start, "row_count": end-start, "id": id}));
        }
        for (row_index, row) in source.iter().copied().enumerate().take(end).skip(start) {
            cx.budget.object()?;
            let mut cells = Vec::new();
            let mut row_text = Vec::new();
            let mut col = 0;
            for cell in row.children() {
                if cell.is_text() && !cell.text().unwrap_or_default().trim().is_empty() {
                    return Err(invalid("text outside EPUB table cells"));
                }
                if !cell.is_element() {
                    continue;
                }
                let header = name(cell, XHTML, "th");
                if !header && !name(cell, XHTML, "td") {
                    return Err(invalid("invalid table cell element"));
                }
                cx.budget.object()?;
                while col < MAX_COLUMNS && occupied[col] > row_index {
                    col += 1;
                }
                let col_span = span(cell, "colspan")?;
                let rows = span(cell, "rowspan")?;
                let row_span = if rows == 0 { end - row_index } else { rows };
                if col_span == 0
                    || col_span > MAX_COLUMNS
                    || col > MAX_COLUMNS - col_span
                    || row_span > end - row_index
                {
                    return Err(invalid("table span exceeds its grid or row group"));
                }
                let right = col + col_span;
                if occupied[col..right].iter().any(|end| *end > row_index) {
                    return Err(invalid("overlapping EPUB table spans"));
                }
                charge(
                    &mut cx.budget.grid,
                    col_span * row_span,
                    MAX_GRID,
                    "table coverage exceeds its budget",
                )?;
                occupied[col..right].fill(row_index + row_span);
                let mut mapped = CellPayload {
                    col_span,
                    row_span,
                    ..Default::default()
                };
                mapped.metadata.insert("grid_column".into(), json!(col));
                mapped.metadata.insert("header".into(), json!(header));
                for key in ["id", "scope", "headers", "abbr"] {
                    if let Some(value) = cell.attribute(key) {
                        cx.budget.bytes(value.len().saturating_mul(3))?;
                        mapped.metadata.insert(format!("html_{key}"), json!(value));
                    }
                }
                flow(cell, &mut mapped.blocks, depth + 1, cx)?;
                let text = flat_text(&mapped.blocks, cx.budget)?;
                row_text.push(text);
                cells.push(mapped);
                col = right;
                width = width.max(right);
            }
            // Keep row strings compact until the final rectangular-grid check.
            result.rows.push(row_text);
            result.cells.push(cells);
        }
    }
    charge(
        &mut cx.budget.grid,
        width.saturating_mul(result.rows.len()),
        MAX_GRID,
        "table grid exceeds its budget",
    )?;
    for (row, cells) in result.rows.iter_mut().zip(&result.cells) {
        let mut grid = vec![String::new(); width];
        for (field, cell) in row.drain(..).zip(cells) {
            let col = cell.metadata["grid_column"].as_u64().unwrap_or_default() as usize;
            grid[col] = field;
        }
        *row = grid;
    }
    if !html_groups.is_empty() {
        result.metadata.insert(
            "html_row_groups".into(),
            serde_json::Value::Array(html_groups),
        );
    }
    if !columns.is_empty() {
        result.metadata.insert(
            "html_column_groups".into(),
            serde_json::Value::Array(columns),
        );
    }
    cx.budget.object()?;
    out.push(BlockPayload {
        kind: "table".into(),
        table: Some(result),
        ..Default::default()
    });
    Ok(())
}

fn finish_direct<'a>(
    groups: &mut Vec<(&'a str, usize, usize, Option<&'a str>)>,
    direct_start: &mut Option<usize>,
    end: usize,
) {
    if let Some(start) = direct_start.take() {
        groups.push(("implicit", start, end, None));
    }
}

fn span(node: Node<'_, '_>, name: &str) -> Result<usize, EngineError> {
    node.attribute(name)
        .map_or(Ok(1), |value| value.trim().parse().map_err(invalid))
}

fn column(node: Node<'_, '_>, budget: &mut Budget) -> Result<serde_json::Value, EngineError> {
    let count = span(node, "span")?;
    if !(1..=MAX_COLUMNS).contains(&count) {
        return Err(limited("invalid table column span"));
    }
    let width = node.attribute("width");
    let style = node.attribute("style");
    if width.into_iter().chain(style).any(|text| text.len() > 4096) {
        return Err(limited("table column attributes exceed their budget"));
    }
    for value in width.into_iter().chain(style) {
        budget.bytes(value.len().saturating_mul(3))?;
    }
    Ok(json!({"span": count, "width": width, "style": style}))
}

fn flat_text(blocks: &[BlockPayload], budget: &mut Budget) -> Result<String, EngineError> {
    let mut out = String::new();
    for block in blocks {
        if !out.is_empty() {
            out.push('\n');
        }
        if let Some(table) = &block.table {
            for (index, row) in table.rows.iter().enumerate() {
                if index > 0 {
                    out.push('\n');
                }
                for (index, cell) in row.iter().enumerate() {
                    if index > 0 {
                        out.push('\t');
                    }
                    budget.bytes(cell.len().saturating_mul(3))?;
                    out.push_str(cell);
                }
            }
        } else {
            budget.bytes(block.text.len().saturating_mul(3))?;
            out.push_str(&block.text);
        }
    }
    Ok(out)
}
