//! Verify archive-local access to chapters newly made non-linear by an edit.
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

use serde_json::Value;

use crate::{
    epub::{
        attribute, charge, child, content, element_ids, external, frame, invalid, name, parse_xml,
        percent_decode, required, resolve, Book, METADATA_BYTES, OPF, XML_BYTES,
    },
    model::DocumentPayload,
    EngineError,
};

const LINK_BYTES: usize = 16 * 1024 * 1024;
const XML_VISITS: usize = 1_000_000;

pub(super) fn validate(
    data: &[u8],
    document: &DocumentPayload,
    indices: &[usize],
) -> Result<(), EngineError> {
    let mut book = Book::open(data)?;
    let package_path = book.package_path()?;
    let package = book.read(&package_path, METADATA_BYTES)?;
    let xml = parse_xml(&package)?;
    let root = xml.root_element();
    let spine = required(root, "spine")?;
    let manifest = document
        .metadata
        .get("manifest")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("missing staged EPUB manifest"))?;
    let resources: HashSet<_> = manifest
        .iter()
        .filter_map(|item| Some((item["path"].as_str()?, item["media_type"].as_str()?)))
        .filter(|(_, media)| {
            matches!(
                *media,
                "application/xhtml+xml" | "text/html" | "image/svg+xml"
            )
        })
        .map(|(path, _)| path)
        .collect();
    let by_id: HashMap<_, _> = manifest
        .iter()
        .filter_map(|item| Some((item["id"].as_str()?, item["path"].as_str()?)))
        .collect();
    let mut aliases = HashMap::new();
    for (itemref, page) in spine
        .children()
        .filter(|node| name(*node, OPF, "itemref"))
        .zip(&document.pages)
    {
        let idref = attribute(itemref, "idref")?;
        if let Some(&path) = by_id.get(idref) {
            let chapter = chapter_path(page)?;
            if path != chapter {
                aliases.entry(path).or_insert(chapter);
            }
        }
    }
    let missing = indices
        .iter()
        .map(|&index| {
            document
                .pages
                .get(index)
                .ok_or_else(|| invalid("missing staged EPUB spine entry"))
                .and_then(chapter_path)
        })
        .collect::<Result<_, _>>()?;
    let mut graph = Access {
        book,
        resources,
        aliases,
        missing,
        scanned: HashSet::new(),
        reached: HashSet::new(),
        queue: VecDeque::new(),
        ids: HashMap::new(),
        bytes: 0,
        visits: 0,
    };
    for page in &document.pages {
        if page.metadata.get("linear").and_then(Value::as_bool) == Some(true) {
            graph.enter(chapter_path(page)?, None)?;
            if graph.missing.is_empty() {
                return Ok(());
            }
        }
    }
    let groups = document
        .metadata
        .get("navigation")
        .and_then(Value::as_array)
        .into_iter()
        .flatten();
    let mut items: Vec<_> = groups
        .flat_map(|group| group["items"].as_array().into_iter().flatten())
        .collect();
    while let Some(item) = items.pop() {
        items.extend(item["children"].as_array().into_iter().flatten());
        if let Some(path) = item["path"].as_str() {
            graph.enter(path, item["fragment"].as_str())?;
            if graph.missing.is_empty() {
                return Ok(());
            }
        }
    }
    if let Some(guide) = child(root, OPF, "guide") {
        for link in guide
            .children()
            .filter(|node| name(*node, OPF, "reference"))
        {
            if let Some(href) = link.attribute("href") {
                graph.link(&package_path, href, false)?;
                if graph.missing.is_empty() {
                    return Ok(());
                }
            }
        }
    }
    while let Some(&missing) = graph.missing.first() {
        let Some(path) = graph.queue.pop_front() else {
            return Err(EngineError::InvalidArgument(format!(
                "new non-linear EPUB chapter {missing:?} has no reachable local link"
            )));
        };
        let source = graph.book.read(path, XML_BYTES)?;
        let xml = parse_xml(&source)?;
        for node in xml.descendants().filter(roxmltree::Node::is_element) {
            graph.visit()?;
            if let Some(href) = content::hyperlink_target(node) {
                graph.link(path, href, false)?;
                if graph.missing.is_empty() {
                    return Ok(());
                }
            } else if frame::is_frame(node) {
                if let Some(source) = content::embedded_document_target(node) {
                    graph.link(path, source, true)?;
                }
            }
        }
    }
    Ok(())
}

fn chapter_path(page: &crate::model::PagePayload) -> Result<&str, EngineError> {
    page.metadata
        .get("chapter_path")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("missing staged EPUB chapter path"))
}

struct Access<'a, 'data> {
    book: Book<'data>,
    resources: HashSet<&'a str>,
    aliases: HashMap<&'a str, &'a str>,
    missing: BTreeSet<&'a str>,
    scanned: HashSet<&'a str>,
    reached: HashSet<&'a str>,
    queue: VecDeque<&'a str>,
    ids: HashMap<&'a str, HashSet<String>>,
    bytes: usize,
    visits: usize,
}

impl<'a> Access<'a, '_> {
    fn visit(&mut self) -> Result<(), EngineError> {
        charge(
            &mut self.visits,
            1,
            XML_VISITS,
            "EPUB auxiliary access XML work budget",
        )
    }

    fn local_document(&self, path: &str) -> Option<&'a str> {
        let path = self.aliases.get(path).copied().unwrap_or(path);
        self.resources
            .get(path)
            .copied()
            .filter(|path| self.book.archive.index_for_name(path).is_some())
    }

    fn enqueue(&mut self, path: &'a str) {
        if self.scanned.insert(path) {
            self.queue.push_back(path);
        }
    }

    fn enter(&mut self, path: &str, fragment: Option<&str>) -> Result<(), EngineError> {
        let Some(path) = self.local_document(path) else {
            return Ok(());
        };
        if self.reached.contains(path) {
            return Ok(());
        }
        if let Some(id) = fragment.filter(|id| !id.is_empty()) {
            if !self.contains_id(path, id)? {
                return Ok(());
            }
        }
        self.reached.insert(path);
        self.missing.remove(path);
        self.enqueue(path);
        Ok(())
    }

    fn link(&mut self, base: &str, href: &str, embedded: bool) -> Result<(), EngineError> {
        let href = href.trim();
        if href.is_empty() || external(href) {
            return Ok(());
        }
        charge(
            &mut self.bytes,
            href.len(),
            LINK_BYTES,
            "EPUB auxiliary access link byte budget",
        )?;
        let Ok(path) = resolve(base, href) else {
            // An unrelated malformed legacy link cannot establish access.
            return Ok(());
        };
        if embedded {
            if let Some(path) = self.local_document(&path) {
                self.enqueue(path);
            }
            return Ok(());
        }
        let fragment = href
            .split_once('#')
            .map(|(_, id)| percent_decode(id))
            .transpose();
        if let Ok(fragment) = fragment {
            self.enter(&path, fragment.as_deref())?;
        }
        Ok(())
    }

    fn contains_id(&mut self, path: &'a str, id: &str) -> Result<bool, EngineError> {
        if let Some(ids) = self.ids.get(path) {
            return Ok(ids.contains(id));
        }
        let source = self.book.read(path, XML_BYTES)?;
        let xml = parse_xml(&source)?;
        let mut ids = HashSet::new();
        for node in xml.descendants().filter(roxmltree::Node::is_element) {
            self.visit()?;
            for id in element_ids(node) {
                charge(
                    &mut self.bytes,
                    id.len().saturating_add(48),
                    LINK_BYTES,
                    "EPUB auxiliary access fragment index budget",
                )?;
                ids.insert(id.to_owned());
            }
        }
        let found = ids.contains(id);
        self.ids.insert(path, ids);
        Ok(found)
    }
}
