//! Bounded OCF package traversal using the shared ZIP guard and a namespace-aware XML parser.
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io::{Cursor, Read},
    rc::Rc,
};

use roxmltree::{Document, Node, ParsingOptions};
use serde_json::{json, Map, Value};

use crate::{
    model::{DocumentPayload, PagePayload},
    rendering::{self, Clip},
    EngineError,
};

#[cfg(feature = "python")]
#[path = "epub_audio.rs"]
pub(crate) mod audio;
#[path = "epub_bidi.rs"]
mod bidi;
#[cfg(feature = "python")]
#[path = "epub_caption_paint.rs"]
pub(crate) mod caption_paint;
#[path = "epub_content.rs"]
mod content;
#[path = "epub_css.rs"]
mod css;
#[path = "epub_editor.rs"]
mod editor;
#[path = "epub_fonts.rs"]
mod fonts;
#[path = "epub_frame.rs"]
mod frame;
#[path = "epub_layout.rs"]
mod layout;
#[path = "epub_math.rs"]
mod math;
#[path = "epub_media.rs"]
mod media;
#[cfg(feature = "python")]
#[path = "epub_mp4.rs"]
pub(crate) mod mp4;
#[path = "epub_obfuscation.rs"]
mod obfuscation;
#[cfg(feature = "python")]
#[path = "epub_ogg.rs"]
mod ogg;
#[path = "epub_output.rs"]
mod output;
#[path = "epub_overlay.rs"]
mod overlay;
#[path = "epub_rendition.rs"]
mod rendition;
#[path = "epub_structure.rs"]
mod structure;
#[path = "epub_style.rs"]
mod style;
#[path = "epub_svg.rs"]
mod svg;
#[cfg(feature = "python")]
#[path = "epub_text_tracks.rs"]
pub(crate) mod text_tracks;
#[path = "epub_ttml_clock.rs"]
mod ttml_clock;
#[path = "epub_ttml_condition.rs"]
mod ttml_condition;
#[path = "epub_ttml_regions.rs"]
mod ttml_regions;
#[path = "epub_ttml_styles.rs"]
mod ttml_styles;
#[cfg(feature = "python")]
#[path = "epub_video.rs"]
pub(crate) mod video;
#[cfg(feature = "python")]
#[path = "epub_webm.rs"]
pub(crate) mod webm;

pub(crate) use editor::{update_chapters, update_metadata};
pub(crate) use structure::{
    update_structure, OverlayEdit, ResourceEdit, SpineEdit, SpinePropertyEdit, StructureEdits,
};

pub(crate) fn render(
    data: &[u8],
    page: usize,
    dpi: f64,
    password: Option<&str>,
    clip: Option<Clip>,
) -> Result<Vec<u8>, EngineError> {
    let book = extract(data, password)?;
    let chapter = book
        .pages
        .get(page)
        .ok_or_else(|| EngineError::PageIndex("EPUB chapter is out of range".into()))?;
    render_page(data, &book, chapter, dpi, clip)
}

fn render_page(
    data: &[u8],
    book: &DocumentPayload,
    chapter: &PagePayload,
    dpi: f64,
    clip: Option<Clip>,
) -> Result<Vec<u8>, EngineError> {
    if let Some(pixels) =
        layout::render(data, book, chapter, dpi, rendition::Region::Viewport(clip))?
    {
        return rendering::encode_png(pixels.width, pixels.height, &pixels.pixels, dpi);
    }
    output::render(data, book, chapter.number, dpi, clip)
}

#[cfg(feature = "python")]
pub(crate) fn render_spread(
    data: &[u8],
    page: usize,
    dpi: f64,
    landscape: bool,
    clip: Option<Clip>,
) -> Result<Vec<u8>, EngineError> {
    rendition::render_spread(data, &extract(data, None)?, page, dpi, landscape, clip)
}

#[cfg(feature = "python")]
pub(crate) fn export_fixed_pdf(
    data: &[u8],
    dpi: f64,
    pages: Option<&[usize]>,
    searchable: bool,
) -> Result<Vec<u8>, EngineError> {
    rendition::export_pdf(data, &extract(data, None)?, dpi, pages, searchable)
}

pub(crate) fn convert(
    data: &[u8],
    target: &str,
    password: Option<&str>,
) -> Result<Vec<u8>, EngineError> {
    output::convert(
        data,
        &extract_impl(data, password, true, &HashSet::new())?,
        target,
    )
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

fn package_fixed_layout(metadata: Node<'_, '_>) -> bool {
    metadata.children().any(|node| {
        name(node, OPF, "meta")
            && node.attribute("property") == Some("rendition:layout")
            && node.attribute("refines").is_none()
            && node
                .text()
                .is_some_and(|text| text.trim() == "pre-paginated")
    })
}

pub fn extract(data: &[u8], password: Option<&str>) -> Result<DocumentPayload, EngineError> {
    extract_impl(data, password, false, &HashSet::new())
}

fn extract_impl(
    data: &[u8],
    password: Option<&str>,
    project_equation_css: bool,
    supplemental: &HashSet<&str>,
) -> Result<DocumentPayload, EngineError> {
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
    let package_info = required(root, "metadata")?;
    let mut metadata = package_metadata(package_info)?;
    let default_fixed = package_fixed_layout(package_info);
    if let Some(direction @ ("ltr" | "rtl" | "default")) =
        spine.attribute("page-progression-direction")
    {
        metadata.insert("page_progression_direction".into(), json!(direction));
    }
    if let Some(policy) = package_info
        .children()
        .find_map(|node| {
            (name(node, OPF, "meta")
                && node.attribute("property") == Some("rendition:spread")
                && node.attribute("refines").is_none())
            .then(|| node.text().map(str::trim))
            .flatten()
        })
        .filter(|policy| matches!(*policy, "auto" | "none" | "landscape" | "portrait" | "both"))
    {
        metadata.insert("rendition_spread".into(), json!(policy));
    }
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
            media_overlay: item.attribute("media-overlay"),
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
    let mut overlays = overlay::read(&mut book, &items, &items_by_path)?;
    if let Some(id) = navigation.or_else(|| spine.attribute("toc")) {
        let entry = items
            .get(id)
            .ok_or_else(|| invalid("navigation is absent from the manifest"))?;
        if !entry.local {
            return Err(unsupported("remote navigation"));
        }
        let bytes = book.read(&entry.resolved, XML_BYTES)?;
        let doc = parse_xml(&bytes)?;
        overlays.check_chapter(&entry.resolved, &doc)?;
        let root = doc.root_element();
        reject_hyperlinked_supplemental(root, &entry.resolved, supplemental)?;
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
        overlays.check_chapter(&entry.resolved, &doc)?;
        let root = doc.root_element();
        reject_hyperlinked_supplemental(root, &entry.resolved, supplemental)?;
        let mut page = if entry.media == "image/svg+xml" {
            content::svg_page(root, &bytes, number, &entry.resolved, &mut budget)?
        } else {
            let viewport = [layout::REFLOW_WIDTH, 792.0];
            let equation_styles = if project_equation_css
                && root
                    .descendants()
                    .any(|node| node.has_tag_name((math::MATHML, "math")))
            {
                let mut cascade = css::Cascade::load(root, &entry.resolved, &mut book, viewport)?;
                if cascade.needed(root) {
                    Some(style::ChapterStyles::compute(root, &mut cascade, viewport)?)
                } else {
                    None
                }
            } else {
                None
            };
            content::chapter(
                root,
                &mut content::Context {
                    source: &bytes,
                    path: &entry.resolved,
                    page: number,
                    manifest: &items_by_path,
                    book: &mut book,
                    budget: &mut budget,
                    styles: None,
                    collect_anchors: false,
                    equation_styles: equation_styles.as_ref().map(|styles| (styles, viewport)),
                },
            )?
        };
        page.metadata.insert("spine_index".into(), json!(number));
        page.metadata
            .insert("chapter_href".into(), json!(entry.href));
        page.metadata
            .insert("chapter_path".into(), json!(entry.resolved));
        page.metadata
            .insert("chapter_media_type".into(), json!(entry.media));
        if let Some(overlay) = entry.media_overlay {
            page.metadata.insert("media_overlay".into(), json!(overlay));
        }
        page.metadata.insert(
            "linear".into(),
            json!(reference.attribute("linear") != Some("no")),
        );
        let mut fixed = default_fixed;
        for property in reference
            .attribute("properties")
            .unwrap_or_default()
            .split_ascii_whitespace()
        {
            match property {
                "rendition:layout-pre-paginated" => fixed = true,
                "rendition:layout-reflowable" => fixed = false,
                "rendition:page-spread-left" | "page-spread-left" => {
                    page.metadata.insert("page_spread".into(), json!("left"));
                }
                "rendition:page-spread-right" | "page-spread-right" => {
                    page.metadata.insert("page_spread".into(), json!("right"));
                }
                "rendition:page-spread-center" => {
                    page.metadata.insert("page_spread".into(), json!("center"));
                }
                "rendition:spread-none"
                | "rendition:spread-landscape"
                | "rendition:spread-portrait"
                | "rendition:spread-both"
                | "rendition:spread-auto" => {
                    page.metadata.insert(
                        "rendition_spread".into(),
                        json!(&property["rendition:spread-".len()..]),
                    );
                }
                _ => {}
            }
        }
        if fixed {
            page.metadata.insert("fixed_layout".into(), json!(true));
            if let Some(viewport) = layout::dimensions(root).or_else(|| svg::viewport(root)) {
                page.metadata.insert("viewport".into(), json!(viewport));
            }
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
            (supplemental.contains(*path) || lower.ends_with(".xhtml") || lower.ends_with(".html"))
                && !validated.contains(*path)
        })
        .map(str::to_owned)
        .collect();
    let mut supplemental_fragments = BTreeMap::<String, HashSet<String>>::new();
    for path in names {
        let bytes = book.read(&path, XML_BYTES)?;
        let xml = parse_xml(&bytes)?;
        overlays.check_chapter(&path, &xml)?;
        if supplemental.contains(path.as_str()) {
            content::chapter(
                xml.root_element(),
                &mut content::Context {
                    source: &bytes,
                    path: &path,
                    page: 0,
                    manifest: &items_by_path,
                    book: &mut book,
                    budget: &mut budget,
                    styles: None,
                    collect_anchors: false,
                    equation_styles: None,
                },
            )?;
            for uri in xml
                .descendants()
                .filter(Node::is_element)
                .filter_map(|node| {
                    content::hyperlink_target(node)
                        .or_else(|| content::embedded_document_target(node))
                })
                .map(str::trim)
                .filter(|uri| !uri.is_empty() && !external(uri))
            {
                let target = resolve(&path, uri)?;
                let mime = items_by_path
                    .get(&target)
                    .filter(|_| book.archive.index_for_name(&target).is_some())
                    .ok_or_else(|| {
                        invalid("supplemental XHTML links to a missing packaged resource")
                    })?;
                if matches!(
                    *mime,
                    "application/xhtml+xml" | "text/html" | "image/svg+xml"
                ) {
                    if let Some((_, fragment)) =
                        uri.split_once('#').filter(|(_, id)| !id.is_empty())
                    {
                        supplemental_fragments
                            .entry(target)
                            .or_default()
                            .insert(percent_decode(fragment)?);
                    }
                }
            }
        }
    }
    for (path, ids) in supplemental_fragments {
        validate_fragment_ids(
            &mut book,
            &path,
            ids.iter().map(String::as_str).collect(),
            "supplemental XHTML links to a missing fragment",
        )?;
    }
    overlays.check_remaining(&mut book)?;
    if !overlays.documents.is_empty() {
        metadata.insert("media_overlays".into(), Value::Object(overlays.documents));
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

fn reject_hyperlinked_supplemental(
    root: Node<'_, '_>,
    path: &str,
    supplemental: &HashSet<&str>,
) -> Result<(), EngineError> {
    if supplemental.is_empty() {
        return Ok(());
    }
    let ncx = name(root, NCX, "ncx");
    for uri in root
        .descendants()
        .filter(Node::is_element)
        .filter_map(|node| {
            if ncx && name(node, NCX, "content") {
                node.attribute("src")
            } else {
                content::hyperlink_target(node)
            }
        })
        .map(str::trim)
        .filter(|uri| !uri.is_empty() && !external(uri))
    {
        if resolve(path, uri).is_ok_and(|target| supplemental.contains(target.as_str())) {
            return Err(EngineError::InvalidArgument(
                "hyperlinked EPUB content must be in the spine. Use linear=False for auxiliary chapters".into(),
            ));
        }
    }
    Ok(())
}

fn validate_fragment_ids(
    book: &mut Book<'_>,
    path: &str,
    mut missing: HashSet<&str>,
    error: &str,
) -> Result<(), EngineError> {
    let source = book.read(path, XML_BYTES)?;
    let xml = parse_xml(&source)?;
    for id in xml
        .descendants()
        .filter(Node::is_element)
        .flat_map(element_ids)
    {
        missing.remove(id);
    }
    if !missing.is_empty() {
        return Err(invalid(error));
    }
    Ok(())
}

struct Item<'a> {
    href: &'a str,
    resolved: String,
    media: &'a str,
    local: bool,
    fallback: Option<&'a str>,
    media_overlay: Option<&'a str>,
}

struct Book<'a> {
    archive: Archive<'a>,
    read_bytes: usize,
    frame_work: Option<Rc<frame::Work>>,
    obfuscated_fonts: Option<obfuscation::FontObfuscation>,
}

impl<'a> Book<'a> {
    fn open(data: &'a [u8]) -> Result<Self, EngineError> {
        let archive = crate::archive_safety::open_epub(data)?;
        let mut encryption = None;
        for path in archive.file_names() {
            validate_archive_path(path)?;
            if path.eq_ignore_ascii_case("META-INF/encryption.xml")
                && encryption.replace(path.to_owned()).is_some()
            {
                return Err(invalid("multiple EPUB encryption documents"));
            }
        }
        let mut book = Self {
            archive,
            read_bytes: 0,
            frame_work: None,
            obfuscated_fonts: None,
        };
        if let Some(path) = encryption {
            book.obfuscated_fonts = Some(obfuscation::load(&mut book, &path)?);
        }
        Ok(book)
    }

    fn read_font(&mut self, path: &str, limit: usize) -> Result<Vec<u8>, EngineError> {
        let mut bytes = self.read(path, limit)?;
        if let Some(fonts) = &self.obfuscated_fonts {
            fonts.apply(path, &mut bytes);
        }
        Ok(bytes)
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
        if let Some(work) = &self.frame_work {
            work.charge_read(size)?;
        }
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
    let options = |allow_dtd| ParsingOptions {
        allow_dtd,
        nodes_limit: 150_000,
        ..Default::default()
    };
    let (doc, doctype) = match Document::parse_with_options(input, options(false)) {
        Err(roxmltree::Error::DtdDetected) => {
            let doctype = inert_doctype(input)?;
            (
                Document::parse_with_options(input, options(true)),
                Some(doctype),
            )
        }
        result => (result, None),
    };
    let doc = doc.map_err(|error| match error {
        roxmltree::Error::NodesLimitReached => limited("XML node count exceeds 150000"),
        _ => invalid(error),
    })?;
    if doctype.is_some_and(|(ns, local)| !name(doc.root_element(), ns, local)) {
        return Err(invalid("XML doctype does not match its root element"));
    }
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

// Only permit known declarations with no internal subset. roxmltree parses the
// identifier as syntax and does not load external DTDs.
fn inert_doctype(input: &str) -> Result<(&'static str, &'static str), EngineError> {
    use quick_xml::events::Event;

    let mut reader = quick_xml::Reader::from_str(input);
    loop {
        match reader.read_event().map_err(invalid)? {
            Event::DocType(declaration) => {
                let declaration = std::str::from_utf8(declaration.as_ref()).map_err(invalid)?;
                let declaration = declaration.trim();
                if declaration == "html" {
                    return Ok((XHTML, "html"));
                }
                for (ns, root, public, system) in [
                    (
                        XHTML,
                        "html",
                        "-//W3C//DTD XHTML 1.1//EN",
                        "http://www.w3.org/TR/xhtml11/DTD/xhtml11.dtd",
                    ),
                    (
                        NCX,
                        "ncx",
                        "-//NISO//DTD ncx 2005-1//EN",
                        "http://www.daisy.org/z3986/2005/ncx-2005-1.dtd",
                    ),
                    (
                        svg::NS,
                        "svg",
                        "-//W3C (World Wide Web Consortium)//DTD SVG 1.1//EN",
                        "http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd",
                    ),
                    (
                        svg::NS,
                        "svg",
                        "-//W3C//DTD SVG 1.1//EN",
                        "http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd",
                    ),
                ] {
                    if matches_public_id(declaration, root, public, system) {
                        return Ok((ns, root));
                    }
                }
                break;
            }
            Event::Decl(_) | Event::Comment(_) | Event::PI(_) => {}
            Event::Text(text) if text.trim_ascii().is_empty() => {}
            _ => break,
        }
    }
    Err(invalid(
        "custom XML document type declarations are not supported",
    ))
}

fn matches_public_id(declaration: &str, root: &str, public: &str, system: &str) -> bool {
    fn consume<'a>(input: &'a str, word: &str) -> Option<&'a str> {
        let rest = input.strip_prefix(word)?;
        rest.starts_with([' ', '\t', '\r', '\n'])
            .then(|| rest.trim_start())
    }

    fn quoted<'a>(input: &'a str, value: &str) -> Option<&'a str> {
        let quote = input.chars().next().filter(|ch| matches!(ch, '\'' | '"'))?;
        input[1..].strip_prefix(value)?.strip_prefix(quote)
    }

    let rest = consume(declaration, root).and_then(|rest| consume(rest, "PUBLIC"));
    rest.and_then(|rest| quoted(rest, public))
        .filter(|rest| rest.starts_with([' ', '\t', '\r', '\n']))
        .and_then(|rest| quoted(rest.trim_start(), system))
        == Some("")
}

fn name(node: Node<'_, '_>, namespace: &str, local: &str) -> bool {
    // Earlier pydoccraft versions also admitted unqualified EPUB 2 XML.
    node.is_element()
        && node.tag_name().name() == local
        && node.tag_name().namespace().is_none_or(|ns| ns == namespace)
}

fn element_ids<'a>(node: Node<'a, '_>) -> impl Iterator<Item = &'a str> {
    [node.attribute("id"), node.attribute((XML, "id"))]
        .into_iter()
        .flatten()
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

fn package_refinement(package: &str, value: &str) -> Result<Option<String>, EngineError> {
    let value = value.trim();
    let Some((_, fragment)) = value
        .split_once('#')
        .filter(|(_, fragment)| !fragment.is_empty())
    else {
        return Ok(None);
    };
    if external(value) || resolve(package, value)? != package {
        return Ok(None);
    }
    percent_decode(fragment).map(Some)
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

const NAV_BYTES: usize = 4 * METADATA_BYTES;
const NAV_ITEMS: usize = 10_000;

#[derive(Default)]
struct NavBudget {
    count: usize,
    bytes: usize,
}

fn xhtml_navigation_label(label: Node<'_, '_>) -> Result<String, EngineError> {
    let mut output = NavigationLabel::default();
    output.visit(label)?;
    if output.text.is_empty() && output.embedded {
        if let Some(title) = label.attribute("title") {
            output.append(title)?;
        }
    }
    Ok(output.text)
}

#[derive(Default)]
struct NavigationLabel {
    text: String,
    pending_space: bool,
    embedded: bool,
}

impl NavigationLabel {
    fn visit(&mut self, node: Node<'_, '_>) -> Result<(), EngineError> {
        if node.is_text() {
            return self.append(node.text().unwrap_or_default());
        }
        if !node.is_element() {
            return Ok(());
        }
        let tag = node.tag_name();
        if tag.namespace() == Some(XHTML) {
            match tag.name() {
                "script" | "style" | "template" => return Ok(()),
                "br" => return self.append(" "),
                _ => {}
            }
        }
        let embedded = node.has_tag_name((svg::NS, "svg"))
            || node.has_tag_name((math::MATHML, "math"))
            || (tag.namespace() == Some(XHTML)
                && matches!(
                    tag.name(),
                    "audio"
                        | "canvas"
                        | "embed"
                        | "iframe"
                        | "img"
                        | "object"
                        | "picture"
                        | "video"
                ));
        self.embedded |= embedded;
        let start = self.text.len();
        let pending = self.pending_space;
        for child in node.children() {
            self.visit(child)?;
        }
        if embedded && self.text.len() == start {
            self.pending_space = pending;
            if let Some(alternative) = ["alt", "title"]
                .into_iter()
                .filter_map(|attribute| node.attribute(attribute))
                .find(|value| {
                    value
                        .chars()
                        .any(|character| !character.is_ascii_whitespace())
                })
            {
                self.append(alternative)?;
            }
        }
        Ok(())
    }

    fn append(&mut self, value: &str) -> Result<(), EngineError> {
        for character in value.chars() {
            if character.is_ascii_whitespace() {
                self.pending_space = !self.text.is_empty();
                continue;
            }
            if self.text.len() + usize::from(self.pending_space) + character.len_utf8() > NAV_BYTES
            {
                return Err(limited("navigation byte budget exceeded"));
            }
            if std::mem::take(&mut self.pending_space) {
                self.text.push(' ');
            }
            self.text.push(character);
        }
        Ok(())
    }
}

fn navigation_fragment(href: &str) -> Result<Option<String>, EngineError> {
    href.split_once('#')
        .map(|(_, fragment)| percent_decode(fragment))
        .transpose()
}

fn nav_target(
    label: Node<'_, '_>,
    href: Option<&str>,
    base: &str,
    budget: &mut NavBudget,
) -> Result<Value, EngineError> {
    let xhtml = label.tag_name().namespace() == Some(XHTML);
    let label = if xhtml {
        xhtml_navigation_label(label)?
    } else {
        label
            .descendants()
            .filter(Node::is_text)
            .filter_map(|n| n.text())
            .collect::<String>()
    };
    let path = href
        .filter(|href| !external(href))
        .map(|href| resolve(base, href))
        .transpose()?;
    charge(
        &mut budget.count,
        1,
        NAV_ITEMS,
        "navigation item budget exceeded",
    )?;
    charge(
        &mut budget.bytes,
        256 + label.len() + href.map_or(0, str::len) + path.as_ref().map_or(0, String::len),
        NAV_BYTES,
        "navigation byte budget exceeded",
    )?;
    let fragment = href
        .filter(|_| path.is_some())
        .map(navigation_fragment)
        .transpose()?
        .flatten();
    let label = if xhtml { label.as_str() } else { label.trim() };
    Ok(json!({"label": label, "href": href, "path": path, "fragment": fragment}))
}

pub(crate) fn validate_svg_resource(data: &[u8]) -> Result<(), EngineError> {
    let xml = parse_xml(data)?;
    content::validate_svg(xml.root_element(), None).map(|_| ())
}

pub(crate) fn read_resource(
    data: &[u8],
    path: &str,
    offset: u64,
    length: Option<usize>,
) -> Result<Vec<u8>, EngineError> {
    use std::io;

    const LIMIT: usize = 16 * 1024 * 1024;
    validate_archive_path(path)?;
    let mut book = Book::open(data)?;
    if offset == 0 && length.is_none() {
        return book.read(path, LIMIT);
    }
    let mut member = book.archive.by_name(path).map_err(invalid)?;
    if member.is_dir() || offset > member.size() {
        return Err(EngineError::InvalidArgument(
            "resource offset is out of range".into(),
        ));
    }
    let remaining = member.size() - offset;
    let count = length.map_or(remaining, |length| (length as u64).min(remaining));
    if length.is_some_and(|length| length > LIMIT) || count > LIMIT as u64 {
        return Err(limited("resource slice exceeds 16 MiB"));
    }
    if count == 0 {
        return Ok(Vec::new());
    }
    let consumed = io::copy(&mut member.by_ref().take(offset), &mut io::sink()).map_err(invalid)?;
    if consumed != offset {
        return Err(invalid("resource shorter than its declared size"));
    }
    let read_limit = count + u64::from(count == remaining);
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(read_limit as usize)
        .map_err(|_| limited("cannot allocate resource slice"))?;
    member
        .take(read_limit)
        .read_to_end(&mut bytes)
        .map_err(invalid)?;
    if bytes.len() as u64 != count {
        return Err(invalid("resource slice size disagrees with the directory"));
    }
    Ok(bytes)
}
