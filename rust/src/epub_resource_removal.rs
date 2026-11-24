//! Exclude only archive members with no retained publication references.
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet, HashSet},
};

use roxmltree::Node;

use super::{apply_edits, argument, Plan};
use crate::{
    epub::{
        attribute, charge, css, element_ids, external, invalid, name, parse_xml, percent_decode,
        required, resolve, svg, unsupported, Book, EPUB, METADATA_BYTES, OPF, XHTML, XML_BYTES,
    },
    EngineError,
};

pub(super) fn apply(
    source: &str,
    plan: &Plan<'_>,
    book: &mut Book<'_>,
    removed: &HashSet<&str>,
) -> Result<String, EngineError> {
    for path in removed {
        if plan.files.contains_key(*path) {
            return Err(argument("EPUB cannot replace and remove the same resource"));
        }
        if book.archive.index_for_name(path).is_none() {
            return Err(argument("removed EPUB resource is absent from the archive"));
        }
    }
    let original = parse_xml(source.as_bytes())?;
    let root = original.root_element();
    let manifest = required(root, "manifest")?;
    let mut covered = HashSet::new();
    let mut ids = HashSet::new();
    let mut retained = BTreeMap::new();
    let mut edits = Vec::new();
    for item in manifest.children().filter(|&node| name(node, OPF, "item")) {
        let href = attribute(item, "href")?;
        let properties = item.attribute("properties").unwrap_or_default();
        if external(href) {
            continue;
        }
        let path = resolve(plan.package, href)?;
        if removed.contains(path.as_str()) {
            if properties
                .split_ascii_whitespace()
                .any(|property| property == "nav")
            {
                return Err(argument("EPUB navigation cannot be removed"));
            }
            covered.insert(path);
            ids.insert(attribute(item, "id")?);
            edits.push((item.range(), String::new()));
        } else {
            if properties
                .split_ascii_whitespace()
                .any(|property| property == "scripted")
            {
                return Err(unsupported(
                    "checking resource dependencies in a scripted EPUB",
                ));
            }
            retained.insert(path, attribute(item, "media-type")?);
        }
    }
    if covered.len() != removed.len() {
        return Err(argument("removal requires a local EPUB manifest resource"));
    }
    let metadata = required(root, "metadata")?;
    for node in metadata.children().filter(|&node| {
        name(node, OPF, "meta") && node.attribute("property") == Some("media:duration")
    }) {
        if node
            .attribute("refines")
            .and_then(|id| id.strip_prefix('#'))
            .is_some_and(|id| ids.contains(id))
        {
            ids.extend(element_ids(node));
            edits.push((node.range(), String::new()));
        }
    }
    let revised = apply_edits(source, edits, METADATA_BYTES)?;
    let final_package = parse_xml(revised.as_bytes())?;
    let mut check = References {
        removed,
        css_bytes: 0,
        stylesheets: BTreeSet::new(),
    };
    check.xml(plan.package, &final_package)?;
    for node in final_package.descendants().filter(Node::is_element) {
        for attr in node.attributes() {
            let value = attr.value();
            let id = match attr.name() {
                "idref" | "fallback" | "fallback-style" | "media-overlay" | "toc" | "handler" => {
                    Some(value)
                }
                "refines" => value.strip_prefix('#'),
                "content" if name(node, OPF, "meta") && node.attribute("name") == Some("cover") => {
                    Some(value)
                }
                _ => None,
            };
            if let Some(id) = id {
                if ids.contains(percent_decode(id)?.as_str()) {
                    return Err(argument(
                        "removed EPUB resource is still referenced by the package",
                    ));
                }
            }
        }
    }

    // XHTML and CSS may exist outside the declared manifest in EPUBs accepted
    // by the reader. Include these members and every other recognized XML
    // resource so a dormant chapter cannot be left with a broken reference.
    for path in book.archive.file_names() {
        if path != plan.package && !removed.contains(path) && kind(path, "").is_some() {
            retained.entry(path.to_owned()).or_default();
        }
    }
    let mut scanned_css = HashSet::new();
    for (path, media) in retained {
        if path == plan.package || path.starts_with("META-INF/") {
            continue;
        }
        let Some(kind) = kind(&path, media) else {
            continue;
        };
        let data = read(&path, &plan.files, book)?;
        match kind {
            Kind::Css => {
                check.css(&path, std::str::from_utf8(&data).map_err(invalid)?, false)?;
                scanned_css.insert(path);
            }
            Kind::Xml => check.xml(&path, &parse_xml(&data)?)?,
        }
    }
    while let Some(path) = check.stylesheets.pop_first() {
        if scanned_css.insert(path.clone()) {
            let data = read(&path, &plan.files, book)?;
            check.css(&path, std::str::from_utf8(&data).map_err(invalid)?, false)?;
        }
    }
    Ok(revised)
}

fn read<'a>(
    path: &str,
    files: &'a super::Files<'_>,
    book: &mut Book<'_>,
) -> Result<Cow<'a, [u8]>, EngineError> {
    if let Some(bytes) = files.get(path) {
        Ok(Cow::Borrowed(bytes.as_ref()))
    } else {
        Ok(Cow::Owned(book.read(path, XML_BYTES)?))
    }
}

pub(super) fn check_metadata_id(node: Node<'_, '_>) -> Result<(), EngineError> {
    let mut ids = element_ids(node).peekable();
    if ids.peek().is_none() {
        return Ok(());
    }
    let ids: HashSet<_> = ids.collect();
    for fragment in node
        .document()
        .descendants()
        .filter_map(|node| node.attribute("refines")?.strip_prefix('#'))
    {
        if ids.contains(percent_decode(fragment)?.as_str()) {
            return Err(argument("removed EPUB duration metadata is still refined"));
        }
    }
    Ok(())
}

enum Kind {
    Css,
    Xml,
}

fn kind(path: &str, media: &str) -> Option<Kind> {
    if media == "text/css" || path.to_ascii_lowercase().ends_with(".css") {
        return Some(Kind::Css);
    }
    if media.ends_with("+xml") || matches!(media, "text/html" | "text/xml" | "application/xml") {
        return Some(Kind::Xml);
    }
    let path = path.to_ascii_lowercase();
    path.rsplit_once('.')
        .is_some_and(|(_, ext)| {
            matches!(
                ext,
                "xml" | "html" | "xhtml" | "svg" | "smil" | "ncx" | "xpl" | "xsl" | "xslt"
            )
        })
        .then_some(Kind::Xml)
}

struct References<'a> {
    removed: &'a HashSet<&'a str>,
    css_bytes: usize,
    stylesheets: BTreeSet<String>,
}

impl References<'_> {
    fn uri(&self, base: &str, value: &str) -> Result<(), EngineError> {
        self.target(base, value).map(|_| ())
    }

    fn target(&self, base: &str, value: &str) -> Result<Option<String>, EngineError> {
        let value = value.trim();
        if value.is_empty() || external(value) {
            return Ok(None);
        }
        let target = resolve(base, value)?;
        if self.removed.contains(target.as_str()) {
            return Err(EngineError::InvalidArgument(format!(
                "EPUB resource {target} is still referenced by {base}"
            )));
        }
        Ok(Some(target))
    }

    fn stylesheet(&mut self, base: &str, href: &str) -> Result<(), EngineError> {
        if let Some(path) = self.target(base, href)? {
            if self.stylesheets.len() >= crate::limits::MAX_MEMBERS
                && !self.stylesheets.contains(&path)
            {
                return Err(crate::epub::limited("too many stylesheet dependencies"));
            }
            self.stylesheets.insert(path);
        }
        Ok(())
    }

    fn css(&mut self, base: &str, source: &str, inline: bool) -> Result<(), EngineError> {
        charge(
            &mut self.css_bytes,
            source.len(),
            8 * 1024 * 1024,
            "resource dependency CSS budget exceeded",
        )?;
        css::resource_references(source, inline, &mut |value, stylesheet| {
            if stylesheet {
                self.stylesheet(base, value)
            } else {
                self.uri(base, value)
            }
        })
    }

    fn xml(&mut self, base: &str, document: &roxmltree::Document<'_>) -> Result<(), EngineError> {
        for node in document.descendants() {
            if let Some(pi) = node.pi().filter(|pi| pi.target == "xml-stylesheet") {
                let reference = svg::stylesheet_reference(pi)?;
                if reference.css {
                    self.stylesheet(base, &reference.href)?;
                } else {
                    self.uri(base, &reference.href)?;
                }
            }
            if !node.is_element() {
                continue;
            }
            let tag = node.tag_name();
            if matches!(tag.namespace(), Some(XHTML | svg::NS)) {
                if tag.name() == "script" || tag.name() == "base" && node.has_attribute("href") {
                    return Err(unsupported(
                        "checking dependencies with scripting or base URLs",
                    ));
                }
                if tag.name() == "link"
                    && node.attribute("rel").is_some_and(|rel| {
                        rel.split_ascii_whitespace()
                            .any(|part| part.eq_ignore_ascii_case("stylesheet"))
                    })
                {
                    if let Some(href) = node.attribute("href") {
                        self.stylesheet(base, href)?;
                    }
                }
                if tag.name() == "style"
                    && node
                        .attribute("type")
                        .is_none_or(|v| v.eq_ignore_ascii_case("text/css"))
                {
                    let css: String = node
                        .children()
                        .filter(Node::is_text)
                        .filter_map(|node| node.text())
                        .collect();
                    self.css(base, &css, false)?;
                }
            }
            for attr in node.attributes() {
                match attr.name() {
                    "href" | "src" | "poster" | "data" | "longdesc" | "cite" | "background"
                    | "refines" => {
                        self.uri(base, attr.value())?;
                    }
                    "textref" if attr.namespace() == Some(EPUB) => self.uri(base, attr.value())?,
                    "srcset" | "imagesrcset" => self.srcset(base, attr.value())?,
                    "style" if matches!(tag.namespace(), Some(XHTML | svg::NS)) => {
                        self.css(base, attr.value(), true)?
                    }
                    "fill" | "stroke" | "filter" | "clip-path" | "mask" | "marker"
                    | "marker-start" | "marker-mid" | "marker-end"
                        if tag.namespace() == Some(svg::NS)
                            && (attr.value().to_ascii_lowercase().contains("url")
                                || attr.value().contains('\\')) =>
                    {
                        self.css(base, &format!("prop:{}", attr.value()), true)?;
                    }
                    key if matches!(tag.namespace(), Some(XHTML | svg::NS))
                        && key.starts_with("on") =>
                    {
                        return Err(unsupported("checking dependencies with event handlers"));
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }

    fn srcset(&self, base: &str, mut source: &str) -> Result<(), EngineError> {
        // A candidate starts with a whitespace-delimited URL. Commas within
        // the URL are significant, including inside data URLs.
        while !source.is_empty() {
            source = source.trim_start_matches(|ch: char| ch.is_ascii_whitespace() || ch == ',');
            let end = source
                .find(|ch: char| ch.is_ascii_whitespace())
                .unwrap_or(source.len());
            let (url, rest) = source.split_at(end);
            let trimmed = url.trim_end_matches(',');
            self.uri(base, trimmed)?;
            if trimmed.len() < url.len() {
                source = rest;
                continue;
            }
            let mut parentheses = 0usize;
            let mut consumed = rest.len();
            for (i, ch) in rest.char_indices() {
                match ch {
                    '(' => parentheses = parentheses.saturating_add(1),
                    ')' => parentheses = parentheses.saturating_sub(1),
                    ',' if parentheses == 0 => {
                        consumed = i + 1;
                        break;
                    }
                    _ => {}
                }
            }
            source = &rest[consumed..];
        }
        Ok(())
    }
}
