//! Map the publication's table of contents to destinations in the selected PDF pages.
use std::{collections::HashMap, sync::Arc};

use pdfcraft_cos::{Document, ObjRef, Object, SaveOptions};
use serde_json::Value;

use super::{viewport, Anchors};
use crate::{
    model::{DocumentPayload, PagePayload},
    EngineError,
};

pub(super) struct Navigation<'a> {
    items: &'a [Value],
    pages: HashMap<&'a str, DestinationPage<'a>>,
}

struct DestinationPage<'a> {
    index: usize,
    height: f64,
    anchors: Anchors<'a>,
}

#[derive(Clone, Copy)]
struct Destination {
    page: usize,
    left: f64,
    top: f64,
}

struct Bookmark<'a> {
    title: &'a str,
    destination: Destination,
    children: Vec<Bookmark<'a>>,
}

fn items(value: &Value) -> &[Value] {
    value.as_array().map_or(&[], Vec::as_slice)
}

fn chapter_path(page: &PagePayload) -> Option<&str> {
    page.metadata.get("chapter_path")?.as_str()
}

impl<'a> Navigation<'a> {
    pub(super) fn new(
        book: &'a DocumentPayload,
        selected: &[&'a PagePayload],
    ) -> Result<Self, EngineError> {
        let toc = book
            .metadata
            .get("navigation")
            .map(items)
            .unwrap_or_default()
            .iter()
            .find(|nav| {
                nav["kind"]
                    .as_str()
                    .is_some_and(|kind| kind.split_ascii_whitespace().any(|token| token == "toc"))
            })
            .map(|nav| items(&nav["items"]))
            .unwrap_or_default();
        let mut navigation = Self {
            items: toc,
            pages: HashMap::new(),
        };
        if toc.is_empty() {
            return Ok(navigation);
        }
        for (index, page) in selected.iter().enumerate() {
            if let Some(path) = chapter_path(page) {
                navigation.pages.entry(path).or_insert(DestinationPage {
                    index,
                    height: viewport(page)?[1],
                    anchors: HashMap::new(),
                });
            }
        }
        let mut pending = toc.iter().collect::<Vec<_>>();
        while let Some(item) = pending.pop() {
            if let Some(page) = item["path"]
                .as_str()
                .and_then(|path| navigation.pages.get_mut(path))
            {
                if let Some(fragment) = item["fragment"].as_str().filter(|id| !id.is_empty()) {
                    page.anchors.insert(fragment, None);
                }
            }
            pending.extend(items(&item["children"]));
        }
        Ok(navigation)
    }

    pub(super) fn anchors(&mut self, index: usize, page: &PagePayload) -> Option<&mut Anchors<'a>> {
        let target = self.pages.get_mut(chapter_path(page)?)?;
        (target.index == index && !target.anchors.is_empty()).then_some(&mut target.anchors)
    }

    fn bookmarks(&self, nav: &'a [Value]) -> Vec<Bookmark<'a>> {
        nav.iter()
            .filter_map(|item| {
                let label = item["label"].as_str().unwrap_or_default();
                let children = self.bookmarks(items(&item["children"]));
                let destination = item["path"]
                    .as_str()
                    .and_then(|path| self.pages.get(path))
                    .map(|page| {
                        let [left, top] = item["fragment"]
                            .as_str()
                            .and_then(|fragment| page.anchors.get(fragment).copied().flatten())
                            .unwrap_or_default();
                        Destination {
                            page: page.index,
                            left: f64::from(left),
                            top: page.height - f64::from(top),
                        }
                    })
                    .or_else(|| children.first().map(|child| child.destination))?;
                Some(Bookmark {
                    title: label,
                    destination,
                    children,
                })
            })
            .collect()
    }

    pub(super) fn attach(&self, pdf: Vec<u8>) -> Result<Vec<u8>, EngineError> {
        let bookmarks = self.bookmarks(self.items);
        if bookmarks.is_empty() {
            return Ok(pdf);
        }
        let mut entries = Vec::new();
        let mut destinations = Vec::new();
        flatten(&bookmarks, 1, &mut entries, &mut destinations);
        let mut document = Document::open(Arc::new(pdf)).map_err(conversion)?;
        let pages = pdfcraft_annot::page_refs(&document).map_err(conversion)?;

        // This public batch operation constructs and counts an entire subtree in one pass.
        // Promote its children to the PDF outline root so the EPUB hierarchy stays intact.
        pdfcraft_organize::add_bookmark_tree(&mut document, "Contents", &entries)
            .map_err(conversion)?;
        let catalog = document.root().ok_or_else(invalid_outline)?;
        let root = reference(&document, catalog, b"Outlines")?;
        let wrapper = reference(&document, root, b"First")?;
        let first = reference(&document, wrapper, b"First")?;
        let last = reference(&document, wrapper, b"Last")?;
        let count = document
            .get(wrapper)
            .as_dict()
            .and_then(|d| d.int(b"Count"))
            .ok_or_else(invalid_outline)?;
        document
            .update_dict(root, |dict| {
                dict.set(b"First".to_vec(), Object::Ref(first));
                dict.set(b"Last".to_vec(), Object::Ref(last));
                dict.set(b"Count".to_vec(), Object::Int(count));
            })
            .map_err(conversion)?;
        let mut pending = vec![(first, true)];
        for destination in destinations {
            let (item, top_level) = pending.pop().ok_or_else(invalid_outline)?;
            let object = document.get(item);
            let dict = object.as_dict().ok_or_else(invalid_outline)?;
            if let Some(next) = dict.reference(b"Next") {
                pending.push((next, top_level));
            }
            if let Some(child) = dict.reference(b"First") {
                pending.push((child, false));
            }
            let page = *pages.get(destination.page).ok_or_else(invalid_outline)?;
            document
                .update_dict(item, |dict| {
                    if top_level {
                        dict.set(b"Parent".to_vec(), Object::Ref(root));
                    }
                    dict.set(
                        b"Dest".to_vec(),
                        Object::Array(vec![
                            Object::Ref(page),
                            Object::name("XYZ"),
                            Object::Real(destination.left),
                            Object::Real(destination.top),
                            Object::Null,
                        ]),
                    );
                })
                .map_err(conversion)?;
        }
        if !pending.is_empty() {
            return Err(invalid_outline());
        }
        document.free(wrapper);
        pdfcraft_cos::write_full(&document, &SaveOptions::default()).map_err(conversion)
    }
}

fn flatten(
    bookmarks: &[Bookmark<'_>],
    level: u8,
    entries: &mut Vec<pdfcraft_organize::OutlineEntry>,
    destinations: &mut Vec<Destination>,
) {
    for bookmark in bookmarks {
        entries.push(pdfcraft_organize::OutlineEntry {
            level,
            title: bookmark.title.to_owned(),
            page: bookmark.destination.page,
            element: None,
        });
        destinations.push(bookmark.destination);
        flatten(&bookmark.children, level + 1, entries, destinations);
    }
}

fn reference(document: &Document, owner: ObjRef, key: &[u8]) -> Result<ObjRef, EngineError> {
    document
        .get(owner)
        .as_dict()
        .and_then(|dict| dict.reference(key))
        .ok_or_else(invalid_outline)
}

fn invalid_outline() -> EngineError {
    conversion("invalid generated outline")
}

fn conversion(error: impl std::fmt::Display) -> EngineError {
    EngineError::Conversion(format!("EPUB fixed-layout PDF navigation: {error}"))
}
