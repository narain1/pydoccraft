//! Update existing EPUB 3 and EPUB 2 navigation hierarchies after a spine edit.
use std::{
    borrow::Cow,
    collections::{BTreeMap, HashMap, HashSet},
};

use roxmltree::{Document, Node};
use serde_json::{Map, Value};

use super::{
    append, apply_edits, child, editor, escape_text, external, inner_range, invalid, name,
    opening_end, parse_xml, relative_url, resolve, Book, EPUB, NCX, XHTML, XML_BYTES,
};
use crate::EngineError;

pub(super) struct Chapter {
    pub path: String,
    pub href: String,
    pub label: String,
    pub inserted: bool,
}

pub(super) fn xhtml_replacement_xml(data: &[u8]) -> Result<Document<'_>, EngineError> {
    let xml = parse_xml(data)?;
    validate_xhtml_replacement(xml.root_element())?;
    Ok(xml)
}

pub(in crate::epub) fn validate_xhtml_replacement(root: Node<'_, '_>) -> Result<(), EngineError> {
    if !name(root, XHTML, "html") || child(root, XHTML, "body").is_none() {
        return Err(invalid(
            "replacement EPUB navigation requires an XHTML document body",
        ));
    }
    let mut has_toc = false;
    for nav in root.descendants().filter(|node| name(*node, XHTML, "nav")) {
        let Some(kind) = nav.attribute((EPUB, "type")) else {
            continue;
        };
        if kind.split_ascii_whitespace().any(|kind| kind == "toc")
            && std::mem::replace(&mut has_toc, true)
        {
            return Err(invalid(
                "replacement EPUB navigation requires one nonempty table of contents",
            ));
        }
        let mut children = nav.children().filter(Node::is_element);
        let mut list = children.next();
        if list.is_some_and(|node| {
            node.tag_name().namespace() == Some(XHTML)
                && matches!(
                    node.tag_name().name(),
                    "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
                )
        }) {
            list = children.next();
        }
        let list = list
            .filter(|node| name(*node, XHTML, "ol") && children.next().is_none())
            .ok_or_else(|| invalid("replacement EPUB navigation requires one ordered list"))?;
        validate_xhtml_list(list)?;
    }
    if !has_toc {
        return Err(invalid(
            "replacement EPUB navigation requires one nonempty table of contents",
        ));
    }
    Ok(())
}

fn validate_xhtml_list(list: Node<'_, '_>) -> Result<(), EngineError> {
    let mut items = list.children().filter(Node::is_element).peekable();
    if items.peek().is_none() {
        return Err(invalid(
            "replacement EPUB navigation ordered list requires at least one item",
        ));
    }
    for item in items {
        if !name(item, XHTML, "li") {
            return Err(invalid(
                "replacement EPUB navigation ordered list requires list items",
            ));
        }
        let mut children = item.children().filter(Node::is_element);
        let label = children
            .next()
            .filter(|node| name(*node, XHTML, "a") || name(*node, XHTML, "span"))
            .ok_or_else(|| {
                invalid("replacement EPUB navigation item requires an anchor or group label")
            })?;
        let nested = children.next();
        if nested.is_some_and(|node| !name(node, XHTML, "ol")) || children.next().is_some() {
            return Err(invalid(
                "replacement EPUB navigation item requires one label and at most one ordered list",
            ));
        }
        if name(label, XHTML, "a") {
            if label
                .attribute("href")
                .is_none_or(|href| href.trim().is_empty())
            {
                return Err(invalid(
                    "replacement EPUB navigation anchor requires a nonblank href",
                ));
            }
        } else if nested.is_none() {
            return Err(invalid(
                "replacement EPUB navigation group requires a nested ordered list",
            ));
        }
        if super::super::xhtml_navigation_label(label)?.is_empty() {
            return Err(invalid(
                "replacement EPUB navigation requires a nonblank label",
            ));
        }
        if let Some(nested) = nested {
            validate_xhtml_list(nested)?;
        }
    }
    Ok(())
}

pub(super) fn ncx_replacement_xml(data: &[u8]) -> Result<Document<'_>, EngineError> {
    let xml = parse_xml(data)?;
    let root = xml.root_element();
    if !name(root, NCX, "ncx") {
        return Err(invalid("invalid NCX navigation root"));
    }
    let mut maps = root.children().filter(|node| name(*node, NCX, "navMap"));
    let has_items = maps
        .next()
        .is_some_and(|map| map.children().any(|node| name(node, NCX, "navPoint")));
    if !has_items || maps.next().is_some() {
        return Err(invalid(
            "replacement EPUB NCX requires one nonempty table of contents",
        ));
    }
    for node in root.descendants().filter(|node| {
        [Kind::NcxPoint, Kind::NcxPage, Kind::NcxTarget]
            .into_iter()
            .any(|kind| kind.accepts(*node))
    }) {
        let mut targets = node.children().filter(|node| name(*node, NCX, "content"));
        if targets
            .next()
            .and_then(|target| target.attribute("src"))
            .is_none_or(|src| src.trim().is_empty())
            || targets.next().is_some()
        {
            return Err(invalid("NCX navigation item must have one target"));
        }
    }
    Ok(xml)
}

pub(in crate::epub) fn validate_replacement(
    data: &[u8],
    metadata: &Map<String, Value>,
    xhtml: bool,
    ncx: Option<&str>,
    supplemental: &HashSet<&str>,
) -> Result<(), EngineError> {
    let manifest: HashMap<_, _> = metadata["manifest"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| Some((item["path"].as_str()?, item["media_type"].as_str()?)))
        .collect();
    let mut book = Book::open(data)?;
    if xhtml {
        let groups = metadata["navigation"]
            .as_array()
            .ok_or_else(|| invalid("missing replacement EPUB navigation"))?;
        validate_targets(&mut book, &manifest, groups, false)?;
    }
    if let Some(path) = ncx {
        let source = book.read(path, XML_BYTES)?;
        let xml = ncx_replacement_xml(&source)?;
        super::super::reject_hyperlinked_supplemental(xml.root_element(), path, supplemental)?;
        let groups = super::super::read_ncx(xml.root_element(), path)?;
        validate_targets(&mut book, &manifest, &groups, true)?;
    }
    Ok(())
}

fn validate_targets(
    book: &mut Book<'_>,
    manifest: &HashMap<&str, &str>,
    groups: &[Value],
    ncx: bool,
) -> Result<(), EngineError> {
    let mut has_toc = false;
    let mut fragments = HashMap::<&str, HashSet<&str>>::new();
    for group in groups {
        let kinds = group["kind"].as_str().unwrap_or_default();
        let kind = |name| kinds.split_ascii_whitespace().any(|kind| kind == name);
        let local_content = ncx || kind("toc") || kind("landmarks") || kind("page-list");
        let mut items = group["items"]
            .as_array()
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        if kind("toc") && (std::mem::replace(&mut has_toc, true) || items.is_empty()) {
            return Err(invalid(
                "replacement EPUB navigation requires one nonempty table of contents",
            ));
        }
        while let Some(item) = items.pop() {
            items.extend(item["children"].as_array().into_iter().flatten());
            let Some(path) = item["path"].as_str() else {
                if local_content && item["href"].is_string() {
                    return Err(invalid(
                        "replacement EPUB navigation targets must be local content documents",
                    ));
                }
                continue;
            };
            let media = manifest
                .get(path)
                .filter(|_| book.archive.index_for_name(path).is_some())
                .ok_or_else(|| {
                    invalid("replacement EPUB navigation target is not a packaged resource")
                })?;
            let content = matches!(
                *media,
                "application/xhtml+xml" | "text/html" | "image/svg+xml"
            );
            if local_content && !content {
                return Err(invalid(
                    "replacement EPUB navigation targets must be local content documents",
                ));
            }
            if let Some(fragment) = item["fragment"]
                .as_str()
                .filter(|id| content && !id.is_empty())
            {
                fragments.entry(path).or_default().insert(fragment);
            }
        }
    }
    if !has_toc {
        return Err(invalid(
            "replacement EPUB navigation requires one nonempty table of contents",
        ));
    }
    for (path, missing) in fragments {
        super::super::validate_fragment_ids(
            book,
            path,
            missing,
            "replacement EPUB navigation targets a missing fragment",
        )?;
    }
    Ok(())
}

pub(super) fn edit<'a>(
    data: &[u8],
    path: &'a str,
    epub3: bool,
    chapters: &[Chapter],
    old_paths: &HashSet<&str>,
    (removed_anchors, anchor_bytes): (&mut editor::RemovedAnchors<'a>, &mut usize),
) -> Result<String, EngineError> {
    let source = std::str::from_utf8(data).map_err(invalid)?;
    let xml = parse_xml(data)?;
    let root = xml.root_element();
    let mut ranks = HashMap::new();
    for (position, chapter) in chapters.iter().enumerate() {
        for path in [chapter.path.as_str(), chapter.href.as_str()] {
            ranks.entry(path).or_insert(position);
        }
    }
    let context = Context {
        source,
        path,
        ranks,
        old_paths,
    };
    let mut edits = Vec::new();
    let primary = if epub3 {
        let mut primary = false;
        for nav in root.descendants().filter(|node| name(*node, XHTML, "nav")) {
            let Some(kind) = nav.attribute((EPUB, "type")) else {
                continue;
            };
            let list = child(nav, XHTML, "ol")
                .ok_or_else(|| invalid("EPUB navigation has no ordered list"))?;
            let is_toc = kind.split_ascii_whitespace().any(|kind| kind == "toc");
            if is_toc && std::mem::replace(&mut primary, true) {
                return Err(invalid("multiple EPUB tables of contents"));
            }
            let mut covered = HashSet::new();
            let items = context.items(list, Kind::Html, &mut covered)?;
            let edited = context.render_list(items, Kind::Html, is_toc, chapters, &covered)?;
            if edited.is_empty() {
                edits.push((nav.range(), String::new()));
            } else {
                edits.push(replace_contents(source, list, edited)?);
            }
        }
        primary
    } else {
        let mut primary = false;
        for group in root.children().filter(Node::is_element) {
            let kind = match group.tag_name().name() {
                "navMap" => Kind::NcxPoint,
                "pageList" => Kind::NcxPage,
                "navList" => Kind::NcxTarget,
                _ => continue,
            };
            let is_toc = kind == Kind::NcxPoint;
            if is_toc && std::mem::replace(&mut primary, true) {
                return Err(invalid("multiple NCX tables of contents"));
            }
            let mut content = String::new();
            let mut covered = HashSet::new();
            let items = context.items(group, kind, &mut covered)?;
            let edited = context.render_list(items, kind, is_toc, chapters, &covered)?;
            if edited.is_empty() {
                edits.push((group.range(), String::new()));
            } else {
                let range = inner_range(source, group)?;
                let mut start = range.start;
                for element in group.children().filter(|node| kind.accepts(*node)) {
                    append(
                        &mut content,
                        &source[start..element.range().start],
                        XML_BYTES,
                    )?;
                    start = element.range().end;
                }
                append(&mut content, &edited, XML_BYTES)?;
                append(&mut content, &source[start..range.end], XML_BYTES)?;
                edits.push((range, content));
            }
        }
        primary
    };
    if !primary {
        let container = if epub3 {
            child(root, XHTML, "body").ok_or_else(|| invalid("EPUB navigation has no body"))?
        } else {
            root
        };
        let range = inner_range(source, container)?;
        edits.push((range.end..range.end, flat(path, epub3, chapters)?));
    }
    let revised = apply_edits(source, edits, XML_BYTES)?;
    if revised != source {
        // Pruning and promoting items can delete IDs even when the caller
        // only edited the spine. Check those IDs against the final package.
        let reordered = parse_xml(revised.as_bytes())?;
        editor::collect_removed_xml_anchors(path, &xml, &reordered, removed_anchors, anchor_bytes)?;
    }
    Ok(revised)
}

pub(super) fn create(
    path: &str,
    epub3: bool,
    title: &str,
    uid: &str,
    chapters: &[Chapter],
) -> Result<String, EngineError> {
    let title = escape_text(title);
    let mut output = if epub3 {
        format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><html xmlns=\"{XHTML}\" xmlns:epub=\"{EPUB}\"><head><title>{title}</title></head><body>")
    } else {
        let uid = crate::author_epub::escape_attribute(uid);
        format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><ncx xmlns=\"{NCX}\" version=\"2005-1\"><head><meta name=\"dtb:uid\" content=\"{uid}\"/><meta name=\"dtb:depth\" content=\"1\"/><meta name=\"dtb:totalPageCount\" content=\"0\"/><meta name=\"dtb:maxPageNumber\" content=\"0\"/></head><docTitle><text>{title}</text></docTitle>")
    };
    append(&mut output, &flat(path, epub3, chapters)?, XML_BYTES)?;
    append(
        &mut output,
        if epub3 { "</body></html>" } else { "</ncx>" },
        XML_BYTES,
    )?;
    if epub3 {
        Ok(output)
    } else {
        renumber(&output, path, chapters, |_| {
            Err(invalid("generated NCX has an unexpected fragment"))
        })
    }
}

fn flat(path: &str, epub3: bool, chapters: &[Chapter]) -> Result<String, EngineError> {
    let mut output = if epub3 {
        format!("<nav xmlns=\"{XHTML}\" xmlns:epub=\"{EPUB}\" epub:type=\"toc\"><ol>")
    } else {
        format!("<navMap xmlns=\"{NCX}\">")
    };
    for (position, chapter) in chapters.iter().enumerate() {
        chapter_link(
            &mut output,
            path,
            epub3,
            chapter,
            &format!("doccraft-toc-{}", position + 1),
        )?;
    }
    append(
        &mut output,
        if epub3 { "</ol></nav>" } else { "</navMap>" },
        XML_BYTES,
    )?;
    Ok(output)
}

fn chapter_link(
    output: &mut String,
    path: &str,
    epub3: bool,
    chapter: &Chapter,
    id: &str,
) -> Result<(), EngineError> {
    let href = relative_url(path, &chapter.path);
    let label = escape_text(&chapter.label);
    let markup = if epub3 {
        format!("<li xmlns=\"{XHTML}\"><a href=\"{href}\">{label}</a></li>")
    } else {
        format!("<navPoint xmlns=\"{NCX}\" id=\"{id}\" playOrder=\"0\"><navLabel><text>{label}</text></navLabel><content src=\"{href}\"/></navPoint>")
    };
    append(output, &markup, XML_BYTES)
}

struct Context<'a> {
    source: &'a str,
    path: &'a str,
    ranks: HashMap<&'a str, usize>,
    old_paths: &'a HashSet<&'a str>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Html,
    NcxPoint,
    NcxPage,
    NcxTarget,
}

impl Kind {
    fn accepts(self, node: Node<'_, '_>) -> bool {
        match self {
            Self::Html => name(node, XHTML, "li"),
            Self::NcxPoint => name(node, NCX, "navPoint"),
            Self::NcxPage => name(node, NCX, "pageTarget"),
            Self::NcxTarget => name(node, NCX, "navTarget"),
        }
    }
}

enum Entry<'a> {
    Source {
        node: Node<'a, 'a>,
        before: &'a str,
        after: &'a str,
        children: Vec<Item<'a>>,
        promoted: bool,
    },
    Inserted(&'a Chapter, String),
}

struct Item<'a> {
    rank: usize,
    entry: Entry<'a>,
}

impl<'a> Context<'a> {
    fn items(
        &self,
        container: Node<'a, 'a>,
        kind: Kind,
        covered: &mut HashSet<usize>,
    ) -> Result<Vec<Item<'a>>, EngineError> {
        let mut items = Vec::new();
        for node in container.children().filter(|node| kind.accepts(*node)) {
            let target = if kind == Kind::Html {
                child(node, XHTML, "a").and_then(|anchor| anchor.attribute("href"))
            } else {
                child(node, NCX, "content").and_then(|content| content.attribute("src"))
            };
            let resolved = target
                .filter(|target| !external(target))
                .map(|target| resolve(self.path, target))
                .transpose()?;
            let own_rank = resolved
                .as_deref()
                .and_then(|path| self.ranks.get(path))
                .copied();
            let removed = own_rank.is_none()
                && resolved
                    .as_deref()
                    .is_some_and(|path| self.old_paths.contains(path));
            let range = node.range();
            let (before, after, mut children) = match kind {
                Kind::Html => {
                    if let Some(list) = child(node, XHTML, "ol") {
                        let nested = self.items(list, kind, covered)?;
                        if nested.is_empty() {
                            if target.is_none() {
                                continue;
                            }
                            (
                                &self.source[range.start..list.range().start],
                                &self.source[list.range().end..range.end],
                                nested,
                            )
                        } else {
                            let inner = inner_range(self.source, list)?;
                            (
                                &self.source[range.start..inner.start],
                                &self.source[inner.end..range.end],
                                nested,
                            )
                        }
                    } else {
                        (&self.source[range], "", Vec::new())
                    }
                }
                Kind::NcxPoint => {
                    let nested = self.items(node, kind, covered)?;
                    if let Some(first) = node.children().find(|child| kind.accepts(*child)) {
                        let start = first.range().start;
                        let end = node
                            .children()
                            .rfind(|child| kind.accepts(*child))
                            .map_or(start, |node| node.range().end);
                        (
                            &self.source[range.start..start],
                            &self.source[end..range.end],
                            nested,
                        )
                    } else {
                        (&self.source[range], "", nested)
                    }
                }
                _ => (&self.source[range], "", Vec::new()),
            };
            if removed {
                for child in &mut children {
                    if let Entry::Source { promoted, .. } = &mut child.entry {
                        *promoted = true;
                    }
                }
                items.extend(children);
            } else {
                covered.extend(own_rank);
                let rank = own_rank
                    .into_iter()
                    .chain(children.iter().map(|child| child.rank))
                    .min()
                    .unwrap_or(usize::MAX);
                items.push(Item {
                    rank,
                    entry: Entry::Source {
                        node,
                        before,
                        after,
                        children,
                        promoted: false,
                    },
                });
            }
        }
        Ok(items)
    }

    fn render_list(
        &self,
        mut items: Vec<Item<'a>>,
        kind: Kind,
        primary: bool,
        chapters: &'a [Chapter],
        covered: &HashSet<usize>,
    ) -> Result<String, EngineError> {
        if primary {
            let mut ids: HashSet<String> = parse_xml(self.source.as_bytes())?
                .descendants()
                .filter(Node::is_element)
                .flat_map(super::super::element_ids)
                .map(str::to_owned)
                .collect();
            let fallback = items.is_empty();
            let mut number = 0;
            for (rank, chapter) in chapters.iter().enumerate() {
                if fallback || chapter.inserted && !covered.contains(&rank) {
                    let id = loop {
                        number += 1;
                        let id = format!("doccraft-toc-{number}");
                        if ids.insert(id.clone()) {
                            break id;
                        }
                    };
                    items.push(Item {
                        rank,
                        entry: Entry::Inserted(chapter, id),
                    });
                }
            }
        }
        let mut output = String::new();
        write_items(&mut output, self.path, kind, &mut items, primary)?;
        Ok(output)
    }
}

fn write_items(
    output: &mut String,
    path: &str,
    kind: Kind,
    items: &mut [Item<'_>],
    sort: bool,
) -> Result<(), EngineError> {
    if sort {
        items.sort_by_key(|item| item.rank);
    }
    for item in items {
        match &mut item.entry {
            Entry::Source {
                node,
                before,
                after,
                children,
                promoted,
            } => {
                if *promoted {
                    let namespaced =
                        super::super::content::namespace_complete(*node, before.as_bytes())?;
                    append(
                        output,
                        std::str::from_utf8(&namespaced).map_err(invalid)?,
                        XML_BYTES,
                    )?;
                } else {
                    append(output, before, XML_BYTES)?;
                }
                write_items(output, path, kind, children, sort)?;
                append(output, after, XML_BYTES)?;
            }
            Entry::Inserted(chapter, id) => {
                chapter_link(output, path, kind == Kind::Html, chapter, id)?
            }
        }
    }
    Ok(())
}

fn replace_contents(
    source: &str,
    element: Node<'_, '_>,
    content: String,
) -> Result<(std::ops::Range<usize>, String), EngineError> {
    if let Ok(range) = inner_range(source, element) {
        return Ok((range, content));
    }
    let raw = &source[element.range()];
    let mut reader = quick_xml::Reader::from_str(raw);
    let quick_xml::events::Event::Empty(tag) = reader.read_event().map_err(invalid)? else {
        return Err(invalid("invalid EPUB navigation container"));
    };
    let tag = std::str::from_utf8(tag.name().into_inner()).map_err(invalid)?;
    Ok((
        element.range(),
        format!("{}>{content}</{tag}>", &raw[..raw.len() - 2]),
    ))
}

pub(super) fn renumber<'a>(
    source: &str,
    path: &str,
    chapters: &[Chapter],
    read: impl FnMut(&str) -> Result<Cow<'a, [u8]>, EngineError>,
) -> Result<String, EngineError> {
    let xml = parse_xml(source.as_bytes())?;
    let nodes: Vec<_> = xml
        .descendants()
        .filter(|node| {
            [Kind::NcxPoint, Kind::NcxPage, Kind::NcxTarget]
                .into_iter()
                .any(|kind| kind.accepts(*node))
        })
        .collect();
    let uses_order = nodes.iter().any(|node| node.has_attribute("playOrder"));
    let depth = nodes
        .iter()
        .filter(|node| Kind::NcxPoint.accepts(**node))
        .map(|node| {
            node.ancestors()
                .filter(|parent| Kind::NcxPoint.accepts(*parent))
                .count()
        })
        .max()
        .unwrap_or(1);
    let mut edits = Vec::new();
    if uses_order {
        let order = play_orders(&nodes, path, chapters, read)?;
        for (node, order) in nodes.into_iter().zip(order) {
            let order = order.to_string();
            if let Some(attribute) = node.attribute_node("playOrder") {
                edits.push((value_range(source, attribute)?, order));
            } else {
                let end = opening_end(source, node)?;
                edits.push((end - 1..end - 1, format!(" playOrder=\"{order}\"")));
            }
        }
    }
    if let Some(head) = child(xml.root_element(), NCX, "head") {
        if let Some(meta) = head
            .children()
            .find(|node| name(*node, NCX, "meta") && node.attribute("name") == Some("dtb:depth"))
        {
            if let Some(content) = meta.attribute_node("content") {
                edits.push((value_range(source, content)?, depth.to_string()));
            }
        }
    }
    apply_edits(source, edits, XML_BYTES)
}

fn play_orders<'a>(
    nodes: &[Node<'_, '_>],
    path: &str,
    chapters: &[Chapter],
    mut read: impl FnMut(&str) -> Result<Cow<'a, [u8]>, EngineError>,
) -> Result<Vec<usize>, EngineError> {
    let mut ranks = HashMap::new();
    for (position, chapter) in chapters.iter().enumerate() {
        for name in [chapter.path.as_str(), chapter.href.as_str()] {
            ranks.entry(name).or_insert(position);
        }
    }
    let mut destinations = HashMap::new();
    let mut entries = Vec::with_capacity(nodes.len());
    for node in nodes {
        let target = child(*node, NCX, "content")
            .and_then(|node| node.attribute("src"))
            .ok_or_else(|| invalid("NCX navigation item has no target"))?;
        let key = if external(target) {
            (target.to_owned(), None)
        } else {
            let fragment = target
                .split_once('#')
                .map(|(_, fragment)| super::super::percent_decode(fragment))
                .transpose()?
                .filter(|fragment| !fragment.is_empty());
            (resolve(path, target)?, fragment)
        };
        let next = destinations.len();
        entries.push(*destinations.entry(key).or_insert(next));
    }
    // A whole document precedes its fragments. Fall back to encounter order
    // for legacy destinations without a readable section in the final spine.
    let mut positions = vec![(usize::MAX, usize::MAX); destinations.len()];
    let mut by_chapter = BTreeMap::<usize, HashMap<&str, Vec<usize>>>::new();
    for ((target, fragment), &index) in &destinations {
        let Some(&chapter) = ranks.get(target.as_str()) else {
            continue;
        };
        if let Some(fragment) = fragment {
            positions[index].0 = chapter;
            by_chapter
                .entry(chapter)
                .or_default()
                .entry(fragment)
                .or_default()
                .push(index);
        } else {
            positions[index] = (chapter, 0);
        }
    }
    for (chapter, mut fragments) in by_chapter {
        if fragments.len() < 2 {
            continue;
        }
        let bytes = read(&chapters[chapter].path)?;
        let xml = parse_xml(&bytes)?;
        for node in xml.descendants().filter(Node::is_element) {
            for id in super::super::element_ids(node) {
                if let Some(indices) = fragments.remove(id) {
                    for index in indices {
                        positions[index].1 = node.range().start + 1;
                    }
                }
            }
            if fragments.is_empty() {
                break;
            }
        }
    }
    let mut sorted: Vec<_> = (0..positions.len()).collect();
    sorted.sort_unstable_by_key(|&index| (positions[index], index));
    let mut order = vec![0; positions.len()];
    for (position, index) in sorted.into_iter().enumerate() {
        order[index] = position + 1;
    }
    Ok(entries.into_iter().map(|index| order[index]).collect())
}

// Use the complete attribute range so unusually long whitespace around '='
// doesn't hit roxmltree's compact storage limit for range_value().
fn value_range(
    source: &str,
    attribute: roxmltree::Attribute<'_, '_>,
) -> Result<std::ops::Range<usize>, EngineError> {
    let range = attribute.range();
    let raw = &source[range.clone()];
    let (name, after) = raw
        .split_once('=')
        .ok_or_else(|| invalid("missing NCX attribute value"))?;
    let trimmed = after.trim_start_matches([' ', '\n', '\r', '\t']);
    let start = name.len() + 1 + after.len() - trimmed.len();
    if trimmed.len() < 2 || !trimmed.starts_with(['\'', '"']) {
        return Err(invalid("invalid NCX attribute range"));
    }
    Ok(range.start + start + 1..range.end - 1)
}
