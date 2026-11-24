//! Bounded OCF package traversal using the shared ZIP guard and a namespace-aware XML parser.
use std::{
    collections::{HashMap, HashSet},
    io::{Cursor, Read},
};

use roxmltree::{Document, Node, ParsingOptions};
use serde_json::{json, Map, Value};

use crate::{model::DocumentPayload, EngineError};

#[path = "epub_content.rs"]
mod content;
#[path = "epub_output.rs"]
mod output;

pub(crate) fn render(
    data: &[u8],
    page: usize,
    dpi: f64,
    password: Option<&str>,
    clip: Option<crate::rendering::Clip>,
) -> Result<Vec<u8>, EngineError> {
    output::render(&extract(data, password)?, page, dpi, clip)
}

pub(crate) fn convert(
    data: &[u8],
    target: &str,
    password: Option<&str>,
) -> Result<Vec<u8>, EngineError> {
    output::convert(&extract(data, password)?, target)
}

const XHTML: &str = "http://www.w3.org/1999/xhtml";
const OPF: &str = "http://www.idpf.org/2007/opf";
const OCF: &str = "urn:oasis:names:tc:opendocument:xmlns:container";
const DC: &str = "http://purl.org/dc/elements/1.1/";
const NCX: &str = "http://www.daisy.org/z3986/2005/ncx/";
const EPUB: &str = "http://www.idpf.org/2007/ops";
const XML: &str = "http://www.w3.org/XML/1998/namespace";
const MIME: &str = "application/epub+zip";
const XML_BYTES: usize = 8 * 1024 * 1024;
const METADATA_BYTES: usize = 1024 * 1024;
const MAX_PATH: usize = 4096;

type Archive<'a> = zip::ZipArchive<Cursor<&'a [u8]>>;

pub fn is_epub(data: &[u8]) -> Result<bool, EngineError> {
    // Detection runs after the common ZIP preflight.
    let mut archive = match Archive::new(Cursor::new(data)) {
        Ok(archive) => archive,
        Err(_) => return Ok(false),
    };
    if !archive.file_names().any(|name| name == "mimetype") {
        return Ok(false);
    }
    let mut entry = archive.by_name("mimetype").map_err(invalid)?;
    if entry.size() > 64 {
        return Ok(false);
    }
    let mut value = Vec::new();
    entry
        .by_ref()
        .take(65)
        .read_to_end(&mut value)
        .map_err(invalid)?;
    Ok(std::str::from_utf8(&value).is_ok_and(|text| text.trim() == MIME))
}

pub fn extract(data: &[u8], password: Option<&str>) -> Result<DocumentPayload, EngineError> {
    if password.is_some() {
        return Err(unsupported("passwords and DRM are not supported"));
    }
    let mut book = Book::open(data)?;
    let mime = book.read("mimetype", 64)?;
    if std::str::from_utf8(&mime).map_err(invalid)?.trim() != MIME {
        return Err(invalid("invalid mimetype entry"));
    }
    let package_path = book.package_path()?;
    let package_path = package_path.as_str();
    let package_bytes = book.read(package_path, METADATA_BYTES)?;
    let package = parse_xml(&package_bytes)?;
    let root = package.root_element();
    if !name(root, OPF, "package") {
        return Err(invalid("missing OPF package element"));
    }
    let version = root
        .attribute("version")
        .ok_or_else(|| invalid("missing OPF version"))?;
    if !matches!(version.split('.').next(), Some("2" | "3")) {
        return Err(unsupported("only EPUB 2 and EPUB 3 packages are supported"));
    }
    let manifest = required(root, "manifest")?;
    let spine = required(root, "spine")?;
    let mut metadata = package_metadata(required(root, "metadata")?)?;
    let mut items = HashMap::new();
    let mut items_by_path = HashMap::new();
    let mut navigation = None;
    let mut resources = Vec::new();
    let mut manifest_bytes = 0;
    for item in manifest.children().filter(|n| name(*n, OPF, "item")) {
        let id = attribute(item, "id")?;
        let href = attribute(item, "href")?;
        let media = attribute(item, "media-type")?;
        let local = !external(href);
        let resolved = if local {
            resolve(package_path, href)?
        } else {
            href.to_owned()
        };
        charge(
            &mut manifest_bytes,
            128 + id.len()
                + href.len()
                + resolved.len() * 3
                + media.len()
                + item.attribute("properties").map_or(0, str::len)
                + item.attribute("media-overlay").map_or(0, str::len),
            4 * METADATA_BYTES,
            "manifest resource budget exceeded",
        )?;
        resources.push(
            json!({"id": id, "href": href, "path": local.then_some(&resolved),
            "media_type": media, "properties": item.attribute("properties"),
            "media_overlay": item.attribute("media-overlay")}),
        );
        if local
            && items_by_path
                .insert(resolved.clone(), media)
                .is_some_and(|previous| previous != media)
        {
            return Err(invalid("conflicting media types for a manifest path"));
        }
        let is_nav = item
            .attribute("properties")
            .is_some_and(|value| value.split_ascii_whitespace().any(|p| p == "nav"));
        let item = Item {
            href,
            resolved,
            media,
            local,
            fallback: item.attribute("fallback"),
        };
        if items.insert(id, item).is_some() {
            return Err(invalid("duplicate manifest id"));
        }
        crate::limits::validate_member_count(items.len())?;
        if is_nav && navigation.replace(id).is_some() {
            return Err(invalid("multiple EPUB navigation documents"));
        }
    }
    metadata.insert("manifest".into(), Value::Array(resources));
    if let Some(id) = navigation.or_else(|| spine.attribute("toc")) {
        let entry = items
            .get(id)
            .ok_or_else(|| invalid("navigation is absent from the manifest"))?;
        if !entry.local {
            return Err(unsupported("remote navigation"));
        }
        let bytes = book.read(&entry.resolved, XML_BYTES)?;
        let doc = parse_xml(&bytes)?;
        let root = doc.root_element();
        let groups = if navigation.is_some() {
            read_navigation(root, &entry.resolved)?
        } else {
            read_ncx(root, &entry.resolved)?
        };
        crate::limits::bounded_json(&groups, 8 * METADATA_BYTES)?;
        metadata.insert("navigation".into(), Value::Array(groups));
    }
    let mut pages = Vec::new();
    let mut checked = HashSet::new();
    let mut fallback_steps = 0;
    let mut page_metadata_bytes = 0;
    let mut budget = content::Budget::default();
    for reference in spine.children().filter(|n| name(*n, OPF, "itemref")) {
        crate::limits::validate_member_count(pages.len() + 1)?;
        let mut id = attribute(reference, "idref")?;
        let entry = loop {
            let entry = items
                .get(id)
                .ok_or_else(|| invalid("spine item is absent from manifest"))?;
            if entry.local
                && matches!(
                    entry.media,
                    "application/xhtml+xml" | "text/html" | "image/svg+xml"
                )
            {
                break entry;
            }
            charge(
                &mut fallback_steps,
                1,
                100_000,
                "spine fallback budget exceeded",
            )?;
            if !checked.insert(id) {
                return Err(invalid("cyclic spine fallback"));
            }
            id = entry.fallback.ok_or_else(|| {
                unsupported("spine item has no supported local content or fallback")
            })?;
        };
        checked.clear();
        charge(
            &mut page_metadata_bytes,
            128 + entry.href.len() + entry.resolved.len(),
            8 * METADATA_BYTES,
            "spine metadata exceeds its budget",
        )?;
        let bytes = book.read(&entry.resolved, XML_BYTES)?;
        let doc = parse_xml(&bytes)?;
        let number = pages.len();
        let root = doc.root_element();
        let mut page = if entry.media == "image/svg+xml" {
            content::svg_page(root, &bytes, number, &entry.resolved, &mut budget)?
        } else {
            content::chapter(
                root,
                &bytes,
                number,
                &entry.resolved,
                &items_by_path,
                &mut book,
                &mut budget,
            )?
        };
        page.metadata.insert("spine_index".into(), json!(number));
        page.metadata
            .insert("chapter_href".into(), json!(entry.href));
        page.metadata
            .insert("chapter_path".into(), json!(entry.resolved));
        page.metadata.insert(
            "linear".into(),
            json!(reference.attribute("linear") != Some("no")),
        );
        if reference.attribute("properties").is_some_and(|properties| {
            properties
                .split_ascii_whitespace()
                .any(|p| p == "rendition:layout-pre-paginated")
        }) {
            page.metadata.insert("fixed_layout".into(), json!(true));
        }
        pages.push(page);
    }
    if pages.is_empty() {
        return Err(invalid("contains no readable spine chapters"));
    }
    // Preserve the existing contract of rejecting malformed XHTML even when
    // a document is not reached through the spine.
    let validated: HashSet<_> = pages
        .iter()
        .filter_map(|p| p.metadata["chapter_path"].as_str())
        .collect();
    let names: Vec<_> = book
        .archive
        .file_names()
        .filter(|path| {
            let lower = path.to_ascii_lowercase();
            (lower.ends_with(".xhtml") || lower.ends_with(".html")) && !validated.contains(*path)
        })
        .map(str::to_owned)
        .collect();
    for path in names {
        parse_xml(&book.read(&path, XML_BYTES)?)?;
    }
    metadata.insert("epub_version".into(), json!(version));
    metadata.insert("chapter_count".into(), json!(pages.len()));
    metadata.insert("chapter_mapping".into(), json!("spine_chapter"));
    Ok(DocumentPayload {
        format: "epub".into(),
        metadata,
        pages,
        ..Default::default()
    })
}

struct Item<'a> {
    href: &'a str,
    resolved: String,
    media: &'a str,
    local: bool,
    fallback: Option<&'a str>,
}

struct Book<'a> {
    archive: Archive<'a>,
    read_bytes: usize,
}

impl<'a> Book<'a> {
    fn open(data: &'a [u8]) -> Result<Self, EngineError> {
        let archive = crate::archive_safety::open_epub(data)?;
        for path in archive.file_names() {
            validate_archive_path(path)?;
            if path.eq_ignore_ascii_case("META-INF/encryption.xml") {
                return Err(unsupported("encrypted resources and DRM are not supported"));
            }
        }
        Ok(Self {
            archive,
            read_bytes: 0,
        })
    }

    fn package_path(&mut self) -> Result<String, EngineError> {
        let container = self.read("META-INF/container.xml", METADATA_BYTES)?;
        let container = parse_xml(&container)?;
        let root = container.root_element();
        let files =
            child(root, OCF, "rootfiles").ok_or_else(|| invalid("container has no rootfiles"))?;
        let mut rootfiles = files.children().filter(|node| name(*node, OCF, "rootfile"));
        let rootfile = rootfiles
            .next()
            .ok_or_else(|| invalid("container has no package rootfile"))?;
        if rootfiles.next().is_some() {
            return Err(unsupported("containers with multiple rootfiles"));
        }
        if !name(root, OCF, "container")
            || rootfile.attribute("media-type") != Some("application/oebps-package+xml")
        {
            return Err(invalid("invalid package rootfile"));
        }
        let path = attribute(rootfile, "full-path")?;
        validate_archive_path(path)?;
        Ok(path.to_owned())
    }

    fn read(&mut self, path: &str, limit: usize) -> Result<Vec<u8>, EngineError> {
        let entry = self
            .archive
            .by_name(path)
            .map_err(|e| invalid(format!("{path}: {e}")))?;
        let size =
            usize::try_from(entry.size()).map_err(|_| limited("resource byte count overflow"))?;
        if entry.is_dir() || size > limit {
            return Err(limited("resource exceeds its byte budget"));
        }
        charge(
            &mut self.read_bytes,
            size,
            128 * 1024 * 1024,
            "aggregate read budget",
        )?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(size + 1)
            .map_err(|_| limited("cannot allocate resource"))?;
        entry
            .take(size as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(invalid)?;
        if bytes.len() != size {
            return Err(invalid("resource size differs from the directory"));
        }
        Ok(bytes)
    }
}

fn parse_xml(data: &[u8]) -> Result<Document<'_>, EngineError> {
    let input = std::str::from_utf8(data).map_err(invalid)?;
    let doc = Document::parse_with_options(
        input,
        ParsingOptions {
            nodes_limit: 150_000,
            ..Default::default()
        },
    )
    .map_err(|error| match error {
        roxmltree::Error::NodesLimitReached => limited("XML node count exceeds 150000"),
        _ => invalid(error),
    })?;
    // Bound recursion before any XPath-like or recursive traversal.
    for node in doc.descendants().filter(Node::is_element) {
        if node.ancestors().nth(96).is_some() {
            return Err(limited("XML nesting exceeds 96"));
        }
        if node.attribute((XML, "base")).is_some() {
            return Err(unsupported("xml:base overrides are not supported"));
        }
    }
    Ok(doc)
}

fn name(node: Node<'_, '_>, namespace: &str, local: &str) -> bool {
    // Earlier pydoccraft versions also admitted unqualified EPUB 2 XML.
    node.is_element()
        && node.tag_name().name() == local
        && node.tag_name().namespace().is_none_or(|ns| ns == namespace)
}

fn child<'a, 'input>(node: Node<'a, 'input>, ns: &str, local: &str) -> Option<Node<'a, 'input>> {
    node.children().find(|node| name(*node, ns, local))
}

fn required<'a, 'input>(
    node: Node<'a, 'input>,
    local: &str,
) -> Result<Node<'a, 'input>, EngineError> {
    let mut matches = node.children().filter(|n| name(*n, OPF, local));
    let first = matches
        .next()
        .ok_or_else(|| invalid(format!("missing OPF {local}")))?;
    if matches.next().is_some() {
        return Err(invalid(format!("duplicate OPF {local}")));
    }
    Ok(first)
}

fn attribute<'a, 'input>(node: Node<'a, 'input>, key: &str) -> Result<&'a str, EngineError> {
    node.attribute(key)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            invalid(format!(
                "missing {key} attribute on {}",
                node.tag_name().name()
            ))
        })
}

fn package_metadata(root: Node<'_, '_>) -> Result<Map<String, Value>, EngineError> {
    let mut fields: HashMap<&str, Vec<String>> = HashMap::new();
    let mut refinements = Vec::new();
    for node in root.children().filter(Node::is_element) {
        let tag = node.tag_name();
        if tag.namespace() == Some(DC) {
            let value = node
                .descendants()
                .filter(Node::is_text)
                .filter_map(|n| n.text())
                .collect::<String>();
            if !value.trim().is_empty() {
                fields
                    .entry(tag.name())
                    .or_default()
                    .push(value.trim().to_owned());
            }
        } else if name(node, OPF, "meta") {
            if let Some(key) = node
                .attribute("property")
                .or_else(|| node.attribute("name"))
            {
                let value = node
                    .attribute("content")
                    .or_else(|| node.text())
                    .unwrap_or_default();
                refinements.push(
                    json!({"property": key, "value": value, "refines": node.attribute("refines")}),
                );
            }
        }
    }
    let mut out = Map::new();
    for (key, dc) in [
        ("title", "title"),
        ("description", "description"),
        ("publisher", "publisher"),
        ("publication_date", "date"),
        ("rights", "rights"),
    ] {
        if let Some(value) = fields.get(dc).and_then(|v| v.last()) {
            out.insert(key.into(), json!(value));
        }
    }
    for (key, dc) in [
        ("authors", "creator"),
        ("languages", "language"),
        ("subjects", "subject"),
        ("identifiers", "identifier"),
        ("contributors", "contributor"),
    ] {
        out.insert(
            key.into(),
            json!(fields.get(dc).map(Vec::as_slice).unwrap_or_default()),
        );
    }
    out.insert("dublin_core".into(), json!(fields));
    if !refinements.is_empty() {
        out.insert("package_metadata".into(), json!(refinements));
    }
    crate::limits::bounded_json(&out, METADATA_BYTES)?;
    Ok(out)
}

fn external(value: &str) -> bool {
    value.starts_with("//")
        || value
            .split(['/', '?', '#'])
            .next()
            .is_some_and(|first| first.contains(':'))
}

fn validate_archive_path(path: &str) -> Result<(), EngineError> {
    if path.is_empty()
        || path.len() > MAX_PATH
        || path.contains(['\\', ':'])
        || path.chars().any(char::is_control)
        || path
            .strip_suffix('/')
            .unwrap_or(path)
            .split('/')
            .any(|p| matches!(p, "" | "." | ".."))
    {
        return Err(EngineError::UnsafeArchive(
            "unsafe EPUB archive path".into(),
        ));
    }
    Ok(())
}

fn resolve(base: &str, target: &str) -> Result<String, EngineError> {
    let path = target.split(['#', '?']).next().unwrap_or_default();
    if path.is_empty() {
        return Ok(base.to_owned());
    }
    if path.starts_with('/') || path.len() > MAX_PATH || external(target) {
        return Err(EngineError::UnsafeArchive(
            "unsafe EPUB resource URI".into(),
        ));
    }
    let mut result: Vec<String> = base
        .rsplit_once('/')
        .map(|(prefix, _)| prefix.split('/').map(str::to_owned).collect())
        .unwrap_or_default();
    for segment in path.split('/') {
        let decoded = percent_decode(segment)?;
        match decoded.as_str() {
            "" | "." => {}
            ".." if segment == ".." => {
                if result.pop().is_none() {
                    return Err(EngineError::UnsafeArchive(
                        "EPUB URI escapes its container".into(),
                    ));
                }
            }
            ".." => {
                return Err(EngineError::UnsafeArchive(
                    "encoded EPUB parent directory".into(),
                ))
            }
            part => {
                if part.contains('/') {
                    return Err(EngineError::UnsafeArchive(
                        "encoded EPUB directory separator".into(),
                    ));
                }
                validate_archive_path(part)?;
                result.push(decoded);
            }
        }
    }
    let result = result.join("/");
    validate_archive_path(&result)?;
    Ok(result)
}

fn percent_decode(input: &str) -> Result<String, EngineError> {
    let mut bytes = Vec::with_capacity(input.len());
    let mut remaining = input.as_bytes().iter().copied();
    while let Some(byte) = remaining.next() {
        if byte == b'%' {
            let mut digit = || {
                remaining
                    .next()
                    .and_then(|n| char::from(n).to_digit(16))
                    .map(|n| n as u8)
                    .ok_or_else(|| invalid("invalid percent escape in EPUB URI"))
            };
            bytes.push(digit()? * 16 + digit()?);
        } else {
            bytes.push(byte);
        }
    }
    String::from_utf8(bytes).map_err(invalid)
}

fn invalid(error: impl std::fmt::Display) -> EngineError {
    EngineError::Malformed(format!("invalid EPUB: {error}"))
}
fn limited(message: &str) -> EngineError {
    EngineError::ResourceLimit(format!("EPUB {message}"))
}
fn unsupported(message: &str) -> EngineError {
    EngineError::Unsupported(format!("EPUB {message}"))
}

fn charge(
    count: &mut usize,
    amount: usize,
    limit: usize,
    message: &str,
) -> Result<(), EngineError> {
    *count = count
        .checked_add(amount)
        .filter(|n| *n <= limit)
        .ok_or_else(|| limited(message))?;
    Ok(())
}

fn read_navigation(root: Node<'_, '_>, base: &str) -> Result<Vec<Value>, EngineError> {
    if !name(root, XHTML, "html") {
        return Err(invalid("navigation document is not XHTML"));
    }
    let mut budget = NavBudget::default();
    let mut groups = Vec::new();
    for nav in root.descendants().filter(|n| name(*n, XHTML, "nav")) {
        let Some(kind) = nav.attribute((EPUB, "type")) else {
            continue;
        };
        let list =
            child(nav, XHTML, "ol").ok_or_else(|| invalid("navigation has no ordered list"))?;
        groups.push(json!({"kind": kind, "items": nav_items(list, base, &mut budget)?}));
    }
    Ok(groups)
}

fn nav_items(
    list: Node<'_, '_>,
    base: &str,
    budget: &mut NavBudget,
) -> Result<Vec<Value>, EngineError> {
    let mut items = Vec::new();
    for li in list.children().filter(|n| name(*n, XHTML, "li")) {
        let label = child(li, XHTML, "a")
            .or_else(|| child(li, XHTML, "span"))
            .ok_or_else(|| invalid("missing navigation label"))?;
        let mut value = nav_target(label, label.attribute("href"), base, budget)?;
        value["children"] = Value::Array(match child(li, XHTML, "ol") {
            Some(nested) => nav_items(nested, base, budget)?,
            None => Vec::new(),
        });
        items.push(value);
    }
    Ok(items)
}

fn read_ncx(root: Node<'_, '_>, base: &str) -> Result<Vec<Value>, EngineError> {
    if !name(root, NCX, "ncx") {
        return Err(invalid("invalid NCX navigation root"));
    }
    let mut groups = Vec::new();
    let mut budget = NavBudget::default();
    for (tag, item, kind) in [
        ("navMap", "navPoint", "toc"),
        ("pageList", "pageTarget", "page-list"),
        ("navList", "navTarget", "nav-list"),
    ] {
        for group in root.children().filter(|n| name(*n, NCX, tag)) {
            groups.push(json!({"kind": kind, "items": ncx_items(group, item, base, &mut budget)?}));
        }
    }
    Ok(groups)
}

fn ncx_items(
    root: Node<'_, '_>,
    item: &str,
    base: &str,
    budget: &mut NavBudget,
) -> Result<Vec<Value>, EngineError> {
    let mut items = Vec::new();
    for node in root.children().filter(|n| name(*n, NCX, item)) {
        let label = child(node, NCX, "navLabel")
            .and_then(|n| child(n, NCX, "text"))
            .ok_or_else(|| invalid("NCX item has no label"))?;
        let href = child(node, NCX, "content").and_then(|n| n.attribute("src"));
        let mut value = nav_target(label, href, base, budget)?;
        value["children"] = Value::Array(ncx_items(node, item, base, budget)?);
        items.push(value);
    }
    Ok(items)
}

#[derive(Default)]
struct NavBudget {
    count: usize,
    bytes: usize,
}

fn nav_target(
    label: Node<'_, '_>,
    href: Option<&str>,
    base: &str,
    budget: &mut NavBudget,
) -> Result<Value, EngineError> {
    let label = label
        .descendants()
        .filter(Node::is_text)
        .filter_map(|n| n.text())
        .collect::<String>();
    let path = href
        .filter(|href| !external(href))
        .map(|href| resolve(base, href))
        .transpose()?;
    charge(
        &mut budget.count,
        1,
        10_000,
        "navigation item budget exceeded",
    )?;
    charge(
        &mut budget.bytes,
        256 + label.len() + href.map_or(0, str::len) + path.as_ref().map_or(0, String::len),
        4 * METADATA_BYTES,
        "navigation byte budget exceeded",
    )?;
    Ok(json!({"label": label.trim(), "href": href, "path": path}))
}

pub(crate) fn validate_svg_resource(data: &[u8]) -> Result<(), EngineError> {
    let xml = parse_xml(data)?;
    content::validate_svg(xml.root_element())
}

/// Change selected Dublin Core values while retaining other XML and ZIP members.
pub(crate) fn update_metadata(
    data: &[u8],
    title: Option<&str>,
    description: Option<&str>,
) -> Result<Vec<u8>, EngineError> {
    use crate::author_epub::{escape_text, validate_text, BoundedOutput};
    use std::io::Write;
    let mut book = Book::open(data)?;
    if book
        .archive
        .file_names()
        .any(|path| path.eq_ignore_ascii_case("META-INF/signatures.xml"))
    {
        return Err(unsupported("metadata edits on signed packages"));
    }
    let package_path = book.package_path()?;
    let package = book.read(&package_path, METADATA_BYTES)?;
    let xml = parse_xml(&package)?;
    let root = required(xml.root_element(), "metadata")?;
    let source = std::str::from_utf8(&package).map_err(invalid)?;
    let mut edits = Vec::new();
    for (tag, value) in [("title", title), ("description", description)] {
        let Some(value) = value else {
            continue;
        };
        validate_text(value, 65_536, tag)?;
        if tag == "title" && value.trim().is_empty() {
            return Err(EngineError::InvalidArgument(
                "EPUB title cannot be empty".into(),
            ));
        }
        let escaped = escape_text(value);
        let existing = root.children().rfind(|node| node.has_tag_name((DC, tag)));
        if let Some(node) = existing {
            if node.children().any(|node| !node.is_text()) {
                return Err(unsupported(
                    "editing Dublin Core entries with structured content",
                ));
            }
            let range = node.range();
            let raw = &source[range.clone()];
            let mut reader = quick_xml::Reader::from_str(raw);
            let event = reader.read_event().map_err(invalid)?;
            let end_of_open = reader.buffer_position() as usize;
            match event {
                quick_xml::events::Event::Start(_) => {
                    let end = raw
                        .rfind("</")
                        .ok_or_else(|| invalid("invalid source metadata element"))?;
                    edits.push((range.start + end_of_open..range.start + end, escaped));
                }
                quick_xml::events::Event::Empty(node) => {
                    let name = std::str::from_utf8(node.name().into_inner()).map_err(invalid)?;
                    edits.push((range.end - 2..range.end, format!(">{escaped}</{name}>")));
                }
                _ => return Err(invalid("invalid source metadata element")),
            }
        } else {
            let range = root.range();
            let end = source[range.clone()]
                .rfind("</")
                .ok_or_else(|| unsupported("empty metadata elements"))?;
            let start = range.start + end;
            edits.push((
                start..start,
                format!("<dc:{tag} xmlns:dc=\"{DC}\">{escaped}</dc:{tag}>"),
            ));
        }
    }
    if edits.is_empty() {
        return Ok(data.to_vec());
    }
    edits.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
    let mut new_package = source.to_owned();
    for (range, replacement) in edits {
        new_package.replace_range(range, &replacement);
    }
    if new_package.len() > METADATA_BYTES {
        return Err(limited("edited package exceeds its byte budget"));
    }
    parse_xml(new_package.as_bytes())?;
    let mut sink = BoundedOutput::default();
    let result = (|| {
        let mut writer = zip::ZipWriter::new(&mut sink);
        // OCF requires a stored mimetype as the first member.
        writer.start_file(
            "mimetype",
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored),
        )?;
        writer.write_all(MIME.as_bytes())?;
        writer.set_raw_comment(book.archive.comment().into());
        for index in 0..book.archive.len() {
            let member = book.archive.by_index_raw(index)?;
            if member.name() == "mimetype" {
                continue;
            }
            if member.name() == package_path {
                writer.start_file(
                    &package_path,
                    zip::write::SimpleFileOptions::default()
                        .compression_method(zip::CompressionMethod::Deflated),
                )?;
                writer.write_all(new_package.as_bytes())?;
            } else {
                writer.raw_copy_file(member)?;
            }
        }
        writer.finish()?;
        Ok::<(), zip::result::ZipError>(())
    })();
    if let Err(error) = result {
        return Err(if sink.exceeded {
            limited("edited EPUB exceeds its output budget")
        } else {
            invalid(error)
        });
    }
    let result = sink.into_bytes();
    crate::archive_safety::open_epub(&result)?;
    Ok(result)
}

pub(crate) fn read_resource(data: &[u8], path: &str) -> Result<Vec<u8>, EngineError> {
    validate_archive_path(path)?;
    Book::open(data)?.read(path, 16 * 1024 * 1024)
}
