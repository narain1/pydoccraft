//! Remap EPUB outlines and visible link areas to the selected PDF pages.
use std::{collections::HashMap, sync::Arc};

use pdfcraft_cos::{Document, ObjRef, Object, SaveOptions};
use serde_json::Value;

use super::{text::TextLayer, viewport, Anchors, Capture, LinkAreas};
use crate::{
    model::{DocumentPayload, PagePayload},
    EngineError,
};

pub(super) struct Navigation<'a> {
    items: &'a [Value],
    destinations: HashMap<&'a str, DestinationPage<'a>>,
    pages: Vec<(&'a str, f64)>,
    links: LinkAreas,
    text: Option<TextLayer>,
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
        searchable: bool,
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
            destinations: HashMap::new(),
            pages: Vec::with_capacity(selected.len()),
            links: LinkAreas::default(),
            text: searchable.then(TextLayer::default),
        };
        for (index, page) in selected.iter().enumerate() {
            let path = chapter_path(page)
                .ok_or_else(|| super::super::invalid("missing EPUB chapter path"))?;
            let height = viewport(page)?[1];
            navigation
                .destinations
                .entry(path)
                .or_insert(DestinationPage {
                    index,
                    height,
                    anchors: HashMap::new(),
                });
            navigation.pages.push((path, height));
        }
        for (index, page) in selected.iter().enumerate() {
            let path = navigation.pages[index].0;
            if navigation.destinations[path].index != index {
                continue;
            }
            for link in &page.links {
                let target = link.metadata.get("target_path").and_then(Value::as_str);
                let fragment = link.metadata.get("target_fragment").and_then(Value::as_str);
                navigation.request_anchor(target, fragment);
            }
            for image in &page.images {
                for target in image
                    .metadata
                    .get("epub_svg_targets")
                    .map(items)
                    .unwrap_or_default()
                {
                    navigation.request_anchor(target[0].as_str(), target[1].as_str());
                }
            }
        }
        let mut pending = toc.iter().collect::<Vec<_>>();
        while let Some(item) = pending.pop() {
            navigation.request_anchor(item["path"].as_str(), item["fragment"].as_str());
            pending.extend(items(&item["children"]));
        }
        Ok(navigation)
    }

    fn request_anchor(&mut self, path: Option<&str>, fragment: Option<&'a str>) {
        if let Some(page) = path.and_then(|path| self.destinations.get_mut(path)) {
            if let Some(fragment) = fragment.filter(|id| !id.is_empty()) {
                page.anchors.insert(fragment, None);
            }
        }
    }

    pub(super) fn capture(&mut self, index: usize, page: &PagePayload) -> Capture<'_, 'a> {
        self.links.page = index;
        let anchors = chapter_path(page)
            .and_then(|path| self.destinations.get_mut(path))
            .filter(|target| target.index == index && !target.anchors.is_empty())
            .map(|target| &mut target.anchors);
        if let Some(text) = &mut self.text {
            text.begin_page();
        }
        Capture {
            anchors,
            links: &mut self.links,
            text: self.text.as_mut(),
        }
    }

    fn bookmarks(&self, nav: &'a [Value]) -> Vec<Bookmark<'a>> {
        nav.iter()
            .filter_map(|item| {
                let label = item["label"].as_str().unwrap_or_default();
                let children = self.bookmarks(items(&item["children"]));
                let destination = item["path"]
                    .as_str()
                    .and_then(|path| self.destinations.get(path))
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
        if bookmarks.is_empty() && self.links.entries.is_empty() && self.text.is_none() {
            return Ok(pdf);
        }
        let mut document = Document::open(Arc::new(pdf)).map_err(conversion)?;
        let pages = pdfcraft_annot::page_refs(&document).map_err(conversion)?;
        if !bookmarks.is_empty() {
            self.attach_bookmarks(&mut document, &pages, &bookmarks)?;
        }
        self.attach_links(&mut document, &pages)?;
        if let Some(text) = &self.text {
            text.attach(
                &mut document,
                &pages,
                self.pages.iter().map(|(_, height)| *height),
            )?;
        }
        pdfcraft_cos::write_full(&document, &SaveOptions::default()).map_err(conversion)
    }

    fn attach_bookmarks(
        &self,
        document: &mut Document,
        pages: &[ObjRef],
        bookmarks: &[Bookmark<'_>],
    ) -> Result<(), EngineError> {
        let mut entries = Vec::new();
        let mut destinations = Vec::new();
        flatten(bookmarks, 1, &mut entries, &mut destinations);

        // This public batch operation constructs and counts an entire subtree in one pass.
        // Promote its children to the PDF outline root so the EPUB hierarchy stays intact.
        pdfcraft_organize::add_bookmark_tree(document, "Contents", &entries).map_err(conversion)?;
        let catalog = document.root().ok_or_else(invalid_outline)?;
        let root = reference(document, catalog, b"Outlines")?;
        let wrapper = reference(document, root, b"First")?;
        let first = reference(document, wrapper, b"First")?;
        let last = reference(document, wrapper, b"Last")?;
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
            let target = destination.object(pages)?;
            document
                .update_dict(item, |dict| {
                    if top_level {
                        dict.set(b"Parent".to_vec(), Object::Ref(root));
                    }
                    dict.set(b"Dest".to_vec(), target);
                })
                .map_err(conversion)?;
        }
        if !pending.is_empty() {
            return Err(invalid_outline());
        }
        document.free(wrapper);
        Ok(())
    }

    fn attach_links(&self, document: &mut Document, pages: &[ObjRef]) -> Result<(), EngineError> {
        use pdfcraft_annot::links::{add_many, LinkStyle};

        let mut entries = Vec::new();
        let mut adjustments = Vec::new();
        let mut page_index = None;
        let mut annotation_index = 0;
        for (index, rect, href) in &self.links.entries {
            let (path, height) = self.pages[*index];
            if page_index != Some(*index) {
                page_index = Some(*index);
                annotation_index = 0;
            }
            let (uri, destination) = if super::super::external(href) {
                if !safe_uri(href) {
                    continue;
                }
                (href.to_string(), None)
            } else {
                let Some(destination) = self.link_destination(path, *index, href)? else {
                    continue;
                };
                // The pinned batch API creates URI actions. Replace the temporary action
                // with the explicit local destination before saving.
                (
                    String::from("about:blank"),
                    Some(destination.object(pages)?),
                )
            };
            let area = [
                f64::from(rect.x),
                height - f64::from(rect.bottom()),
                f64::from(rect.right()),
                height - f64::from(rect.y),
            ];
            // PdfCraft's authoring UI requires a 2 pt box. The PDF rectangle can be
            // smaller, for example for a linked inline icon or a partially clipped line.
            let small = area[2] - area[0] < 2.0 || area[3] - area[1] < 2.0;
            if small || destination.is_some() {
                adjustments.push((*index, annotation_index, destination, small.then_some(area)));
            }
            let authoring_area = if small { [0.0, 0.0, 2.0, 2.0] } else { area };
            entries.push((*index, vec![authoring_area], uri));
            annotation_index += 1;
        }
        if entries.is_empty() {
            return Ok(());
        }
        add_many(document, &entries, &LinkStyle::default()).map_err(conversion)?;
        let mut last_page = None;
        let mut page_object = None;
        for (page, index, destination, area) in adjustments {
            if last_page != Some(page) {
                last_page = Some(page);
                page_object = pages.get(page).map(|&page| document.get(page));
            }
            let annotation = page_object
                .as_deref()
                .and_then(Object::as_dict)
                .and_then(|dict| dict.get(b"Annots"))
                .and_then(Object::as_array)
                .and_then(|items| items.get(index))
                .and_then(Object::as_ref)
                .ok_or_else(|| conversion("missing generated link annotation"))?;
            document
                .update_dict(annotation, |dict| {
                    if let Some(destination) = destination {
                        dict.remove(b"A");
                        dict.set(b"Dest".to_vec(), destination);
                    }
                    if let Some(area) = area {
                        dict.set(
                            b"Rect".to_vec(),
                            Object::Array(area.into_iter().map(Object::Real).collect()),
                        );
                    }
                })
                .map_err(conversion)?;
        }
        Ok(())
    }

    fn link_destination(
        &self,
        source: &str,
        index: usize,
        href: &str,
    ) -> Result<Option<Destination>, EngineError> {
        let path = super::super::resolve(source, href)?;
        let Some(target) = self.destinations.get(path.as_str()) else {
            return Ok(None);
        };
        let point = match href.split_once('#').map(|(_, fragment)| fragment) {
            Some(fragment) if !fragment.is_empty() => {
                let fragment = super::super::percent_decode(fragment)?;
                let Some(Some(point)) = target.anchors.get(fragment.as_str()) else {
                    return Ok(None);
                };
                *point
            }
            _ => [0.0, 0.0],
        };
        Ok(Some(Destination {
            page: if path == source { index } else { target.index },
            left: f64::from(point[0]),
            top: target.height - f64::from(point[1]),
        }))
    }
}

impl Destination {
    fn object(self, pages: &[ObjRef]) -> Result<Object, EngineError> {
        let page = *pages.get(self.page).ok_or_else(invalid_outline)?;
        Ok(Object::Array(vec![
            Object::Ref(page),
            Object::name("XYZ"),
            Object::Real(self.left),
            Object::Real(self.top),
            Object::Null,
        ]))
    }
}

fn safe_uri(href: &str) -> bool {
    let Some((scheme, target)) = href.split_once(':') else {
        return false;
    };
    href.len() <= 8192
        && !href.chars().any(char::is_control)
        && (scheme.eq_ignore_ascii_case("mailto")
            || (scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https"))
                && target.starts_with("//"))
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
