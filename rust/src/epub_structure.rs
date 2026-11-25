//! Edit reading order and publication resources while retaining untouched archive members.
use std::{
    borrow::Cow,
    collections::{BTreeMap, HashMap, HashSet},
    fmt::Write,
    ops::Range,
};

use roxmltree::Node;
use serde_json::Value;

use super::{
    attribute, charge, child, editor, external, extract, fonts, invalid, limited, name,
    obfuscation, parse_xml, required, resolve, unsupported, validate_archive_path, Book, EPUB,
    METADATA_BYTES, NCX, OPF, XHTML, XML_BYTES,
};
use crate::{
    author_epub::{escape_text, validate_image, validate_text},
    limits::{validate_member_count, MAX_INPUT_SIZE},
    EngineError,
};

#[path = "epub_overlay_edit.rs"]
mod narration;
#[path = "epub_resource_removal.rs"]
mod removal;
#[path = "epub_toc_edit.rs"]
mod toc;

type ChapterEdit = (String, String, Option<String>, bool, bool);
pub(crate) type SpineEdit = (Option<i64>, Option<ChapterEdit>);
pub(crate) type ResourceEdit = (String, String, Vec<u8>);
// Depth-first timing entries keep the native boundary flat and bounded.
type TimingEdit = (
    usize,
    bool,
    Option<String>,
    Option<String>,
    u64,
    Option<u64>,
    Option<String>,
);
pub(crate) type OverlayEdit = (String, Option<Vec<TimingEdit>>);
type Files<'a> = BTreeMap<String, Cow<'a, [u8]>>;

pub(crate) fn update_structure<'a>(
    data: &[u8],
    order: Option<&'a [SpineEdit]>,
    resources: &'a [ResourceEdit],
    modified: &str,
    removed: &[String],
    overlays: &[OverlayEdit],
    chapters: &'a [(i64, String)],
) -> Result<Vec<u8>, EngineError> {
    let mut total = 0;
    let mut svg_chapters = Vec::new();
    editor::charge_chapter_replacements(chapters, &mut total)?;
    validate_member_count(removed.len())?;
    narration::charge_edits(overlays, &mut total)?;
    let mut removals = HashSet::new();
    for path in removed {
        check_path(path)?;
        charge(
            &mut total,
            path.len(),
            MAX_INPUT_SIZE,
            "EPUB edits exceed their byte budget",
        )?;
        if !removals.insert(path.as_str()) {
            return Err(argument("duplicate EPUB resource removal"));
        }
    }
    validate_member_count(order.map_or(0, <[_]>::len))?;
    validate_member_count(resources.len())?;
    for (_, resource) in order.into_iter().flatten() {
        if let Some((title, source, path, ..)) = resource {
            charge(
                &mut total,
                title
                    .len()
                    .saturating_add(source.len())
                    .saturating_add(path.as_ref().map_or(0, String::len)),
                MAX_INPUT_SIZE,
                "EPUB edits exceed their byte budget",
            )?;
        }
    }
    for (path, media, bytes) in resources {
        charge(
            &mut total,
            path.len()
                .saturating_add(media.len())
                .saturating_add(bytes.len()),
            MAX_INPUT_SIZE,
            "EPUB edits exceed their byte budget",
        )?;
    }
    let mut book = editor::open_for_edit(data)?;
    if order.is_none()
        && resources.is_empty()
        && removed.is_empty()
        && overlays.is_empty()
        && chapters.is_empty()
    {
        return Ok(data.to_vec());
    }
    let package_path = book.package_path()?;
    let package = book.read(&package_path, METADATA_BYTES)?;
    let package_source = std::str::from_utf8(&package).map_err(invalid)?;
    let xml = parse_xml(&package)?;
    let root = xml.root_element();
    let metadata = required(root, "metadata")?;
    let manifest_node = required(root, "manifest")?;
    let spine = required(root, "spine")?;
    let epub3 = root
        .attribute("version")
        .is_some_and(|version| version.starts_with('3'));
    let mut occupied: HashSet<_> = book.archive.file_names().map(str::to_owned).collect();
    occupied.insert(package_path.clone());
    let mut manifest = HashMap::new();
    let mut paths = HashMap::new();
    let mut existing_toc = None;
    let mut compatibility_ncx = None;
    for item in manifest_node
        .children()
        .filter(|node| name(*node, OPF, "item"))
    {
        let id = attribute(item, "id")?;
        let href = attribute(item, "href")?;
        let media = attribute(item, "media-type")?;
        if external(href) {
            continue;
        }
        let path = resolve(&package_path, href)?;
        occupied.insert(path.clone());
        if manifest.insert(id, path.clone()).is_some()
            || paths
                .insert(path.clone(), media)
                .is_some_and(|old| old != media)
        {
            return Err(invalid("conflicting EPUB manifest resources"));
        }
        if epub3 && spine.attribute("toc") == Some(id) && media == "application/x-dtbncx+xml" {
            compatibility_ncx = Some(path.clone());
        }
        let is_toc = if epub3 {
            item.attribute("properties")
                .is_some_and(|p| p.split_ascii_whitespace().any(|p| p == "nav"))
        } else {
            spine.attribute("toc") == Some(id)
        };
        if is_toc && existing_toc.replace(path).is_some() {
            return Err(invalid("multiple EPUB tables of contents"));
        }
    }
    let spine_paths: HashSet<_> = spine
        .children()
        .filter(|node| name(*node, OPF, "itemref"))
        .filter_map(|node| node.attribute("idref"))
        .filter_map(|id| manifest.get(id))
        .collect();
    let mut nav_edits = resources
        .iter()
        .filter(|(path, _, _)| existing_toc.as_ref() == Some(path));
    let authored_nav = nav_edits.next();
    if nav_edits.next().is_some() {
        return Err(argument("duplicate EPUB navigation replacement"));
    }
    if (!epub3 && authored_nav.is_some())
        || resources
            .iter()
            .any(|(path, _, _)| compatibility_ncx.as_ref() == Some(path))
    {
        return Err(unsupported("explicit EPUB NCX replacement"));
    }
    let mut plan = Plan {
        occupied,
        ids: xml
            .descendants()
            .filter(Node::is_element)
            .flat_map(super::element_ids)
            .map(str::to_owned)
            .collect(),
        package: &package_path,
        counter: 0,
        manifest_additions: String::new(),
        files: Files::new(),
    };
    let original = if order.is_some() || !chapters.is_empty() || authored_nav.is_some() {
        Some(extract(data, None)?)
    } else {
        None
    };
    let mut edits = Vec::new();
    let mut removed_anchors = editor::RemovedAnchors::new();
    let mut anchor_bytes = 0;
    if let Some(original) = original.as_ref().filter(|_| !chapters.is_empty()) {
        let replacement = editor::prepare_chapter_replacements(
            &mut book,
            original,
            chapters,
            &mut removed_anchors,
            &mut anchor_bytes,
        )?;
        editor::chapter_property_edits(
            package_source,
            &package_path,
            manifest_node,
            &replacement.declarations,
            &mut edits,
        )?;
        plan.files.extend(
            replacement
                .files
                .into_iter()
                .map(|(path, data)| (path.to_owned(), Cow::Borrowed(data))),
        );
        svg_chapters.extend(replacement.svg_paths.into_iter().map(str::to_owned));
    }
    if let Some(((path, media, data), original)) = authored_nav.zip(original.as_ref()) {
        check_path(path)?;
        if spine_paths.contains(path)
            || original
                .pages
                .iter()
                .any(|page| page.metadata["chapter_path"].as_str() == Some(path))
        {
            return Err(argument(
                "replace a spine navigation chapter through replacements",
            ));
        }
        if media != "application/xhtml+xml"
            || paths.get(path) != Some(&media.as_str())
            || book.archive.index_for_name(path).is_none()
        {
            return Err(argument(
                "EPUB navigation replacement requires its packaged XHTML manifest entry",
            ));
        }
        if data.len() > XML_BYTES {
            return Err(limited(
                "EPUB navigation replacement exceeds its XML byte budget",
            ));
        }
        plan.files.insert(path.clone(), Cow::Borrowed(data));
    }
    if let Some((order, original)) = order.zip(original.as_ref()) {
        let references: Vec<_> = spine
            .children()
            .filter(|node| name(*node, OPF, "itemref"))
            .collect();
        if references.len() != original.pages.len() {
            return Err(invalid("inconsistent EPUB spine"));
        }
        let mut selected = Vec::new();
        let mut used = HashSet::new();
        let package_fixed = metadata.children().any(|m| {
            name(m, OPF, "meta")
                && m.attribute("property") == Some("rendition:layout")
                && m.attribute("refines").is_none()
                && m.text().is_some_and(|text| text.trim() == "pre-paginated")
        });
        let mut read_order = String::new();
        for entry in order {
            match entry {
                (Some(index), None) => {
                    let count = references.len() as i64;
                    let index = if *index < 0 { count + index } else { *index };
                    let index = usize::try_from(index)
                        .ok()
                        .filter(|index| *index < references.len())
                        .ok_or_else(|| {
                            EngineError::PageIndex("EPUB chapter is out of range".into())
                        })?;
                    if !used.insert(index) {
                        return Err(argument("EPUB spine index was selected more than once"));
                    }
                    let page = &original.pages[index];
                    let path = page
                        .metadata
                        .get("chapter_path")
                        .and_then(Value::as_str)
                        .ok_or_else(|| invalid("missing chapter path"))?;
                    let href = manifest
                        .get(attribute(references[index], "idref")?)
                        .map_or(path, String::as_str);
                    let label = page
                        .metadata
                        .get("chapter_title")
                        .and_then(Value::as_str)
                        .unwrap_or("Chapter");
                    selected.push(toc::Chapter {
                        path: path.into(),
                        href: href.into(),
                        label: label.into(),
                        inserted: false,
                    });
                    append(
                        &mut read_order,
                        &package_source[references[index].range()],
                        METADATA_BYTES,
                    )?;
                }
                (None, Some((label, source, path, fixed, svg))) => {
                    validate_text(label, 1024, "chapter title")?;
                    if label.trim().is_empty() {
                        return Err(argument("EPUB chapter title is empty"));
                    }
                    if source.len() > XML_BYTES {
                        return Err(limited("inserted chapter exceeds its XML byte budget"));
                    }
                    let chapter = parse_xml(source.as_bytes())?;
                    let root = chapter.root_element();
                    let (extension, media) = if *svg {
                        if !epub3 {
                            return Err(argument("inserting an SVG spine chapter requires EPUB 3"));
                        }
                        if !root.has_tag_name((super::svg::NS, "svg")) {
                            return Err(argument("an inserted SVG chapter must have an SVG root"));
                        }
                        ("svg", "image/svg+xml")
                    } else {
                        if !name(root, XHTML, "html") {
                            return Err(argument(
                                "an inserted XHTML chapter must have an XHTML root",
                            ));
                        }
                        ("xhtml", "application/xhtml+xml")
                    };
                    let path = if let Some(path) = path {
                        check_path(path)?;
                        if !path
                            .rsplit_once('.')
                            .is_some_and(|(_, ext)| ext.eq_ignore_ascii_case(extension))
                        {
                            return Err(argument(&format!(
                                "chapter archive path must end in .{extension}"
                            )));
                        }
                        plan.claim_path(path)?;
                        path.clone()
                    } else {
                        plan.new_path("chapter", extension)
                    };
                    let content_properties = if epub3 && !*svg {
                        editor::properties_attribute(&editor::xhtml_properties(root))
                    } else {
                        String::new()
                    };
                    let id = plan.add_manifest(&path, media, &content_properties)?;
                    let properties = if *svg {
                        if super::svg::viewport(root).is_none() {
                            return Err(argument(
                                "a fixed-layout SVG chapter requires a numeric viewBox",
                            ));
                        }
                        svg_chapters.push(path.clone());
                        " properties=\"rendition:layout-pre-paginated\""
                    } else if *fixed {
                        if !epub3 || super::layout::dimensions(root).is_none() {
                            return Err(argument("a fixed-layout chapter requires EPUB 3 and a numeric XHTML viewport"));
                        }
                        " properties=\"rendition:layout-pre-paginated\""
                    } else if epub3 && package_fixed {
                        " properties=\"rendition:layout-reflowable\""
                    } else {
                        ""
                    };
                    append(
                        &mut read_order,
                        &format!("<itemref xmlns=\"{OPF}\" idref=\"{id}\"{properties}/>"),
                        METADATA_BYTES,
                    )?;
                    selected.push(toc::Chapter {
                        path: path.clone(),
                        href: path.clone(),
                        label: label.clone(),
                        inserted: true,
                    });
                    plan.files.insert(path, Cow::Borrowed(source.as_bytes()));
                }
                _ => return Err(argument("invalid EPUB spine entry")),
            }
        }
        if selected.is_empty() {
            return Err(argument("EPUB must retain at least one chapter"));
        }
        if !read_order.is_empty()
            && (used.len() != references.len()
                || order.iter().enumerate().any(|(position, entry)| {
                    let count = references.len() as i64;
                    entry.0.is_none_or(|index| {
                        usize::try_from(if index < 0 { count + index } else { index }).ok()
                            != Some(position)
                    })
                }))
        {
            let range = inner_range(package_source, spine)?;
            let mut spine_contents = String::new();
            let mut position = range.start;
            for reference in &references {
                let item = reference.range();
                append(
                    &mut spine_contents,
                    &package_source[position..item.start],
                    METADATA_BYTES,
                )?;
                position = item.end;
            }
            append(&mut spine_contents, &read_order, METADATA_BYTES)?;
            append(
                &mut spine_contents,
                &package_source[position..range.end],
                METADATA_BYTES,
            )?;
            edits.push((range, spine_contents));
            let old_paths: HashSet<_> = original
                .pages
                .iter()
                .zip(&references)
                .flat_map(|(page, reference)| {
                    [
                        page.metadata.get("chapter_path").and_then(Value::as_str),
                        reference
                            .attribute("idref")
                            .and_then(|id| manifest.get(id))
                            .map(String::as_str),
                    ]
                    .into_iter()
                    .flatten()
                })
                .collect();
            if let Some(path) = &existing_toc {
                let original;
                let source = if let Some(replacement) = plan.files.get(path) {
                    replacement.as_ref()
                } else {
                    original = book.read(path, XML_BYTES)?;
                    &original
                };
                let nav = toc::edit(source, path, epub3, &selected, &old_paths)?;
                plan.files
                    .insert(path.clone(), Cow::Owned(nav.into_bytes()));
            } else {
                let (extension, media, properties) = if epub3 {
                    ("xhtml", "application/xhtml+xml", " properties=\"nav\"")
                } else {
                    ("ncx", "application/x-dtbncx+xml", "")
                };
                let path = plan.new_path("navigation", extension);
                let id = plan.add_manifest(&path, media, properties)?;
                if !epub3 {
                    let end = opening_end(package_source, spine)?;
                    edits.push((end - 1..end - 1, format!(" toc=\"{id}\"")));
                }
                let title = original
                    .metadata
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("Contents");
                let uid = root
                    .attribute("unique-identifier")
                    .and_then(|id| {
                        metadata
                            .children()
                            .find(|node| node.attribute("id") == Some(id))
                            .and_then(|node| node.text())
                    })
                    .unwrap_or_default();
                let nav = toc::create(&path, epub3, title, uid, &selected)?;
                plan.files.insert(path, Cow::Owned(nav.into_bytes()));
            }
            if let Some(path) = compatibility_ncx {
                let bytes = book.read(&path, XML_BYTES)?;
                let ncx = toc::edit(&bytes, &path, false, &selected, &old_paths)?;
                plan.files.insert(path, Cow::Owned(ncx.into_bytes()));
            }
        }
    }
    if let Some((path, _, authored)) = authored_nav {
        let data = plan
            .files
            .get(path)
            .ok_or_else(|| invalid("missing staged EPUB navigation"))?;
        let xml = toc::replacement_xml(data)?;
        let properties = editor::xhtml_properties(xml.root_element());
        editor::chapter_property_edits(
            package_source,
            &package_path,
            manifest_node,
            &HashMap::from([(path.as_str(), properties)]),
            &mut edits,
        )?;
        editor::collect_removed_anchors(
            &mut book,
            path,
            data,
            &xml,
            &mut removed_anchors,
            &mut anchor_bytes,
        )?;
        if authored.as_slice() != data.as_ref() {
            let authored = toc::replacement_xml(authored)?;
            editor::collect_removed_xml_anchors(
                path,
                &authored,
                &xml,
                &mut removed_anchors,
                &mut anchor_bytes,
            )?;
        }
    }
    for (path, media, data) in resources {
        if authored_nav.is_some_and(|(nav_path, _, _)| nav_path == path) {
            continue;
        }
        check_path(path)?;
        if plan.files.contains_key(path)
            || existing_toc.as_ref() == Some(path)
            || spine_paths.contains(path)
            || !editable_resource(media)
        {
            return Err(argument(
                "EPUB resource edit conflicts with chapter or package structure",
            ));
        }
        if data.is_empty() {
            return Err(argument("EPUB resource data cannot be empty"));
        }
        if matches!(media.as_str(), "text/css" | "text/vtt") {
            std::str::from_utf8(data).map_err(invalid)?;
        } else if matches!(
            media.as_str(),
            "image/png" | "image/jpeg" | "image/gif" | "image/webp" | "image/svg+xml"
        ) {
            validate_image(data, media)?;
        } else if matches!(
            media.as_str(),
            "application/smil+xml" | "application/ttml+xml"
        ) {
            if data.len() > XML_BYTES {
                return Err(limited(
                    "EPUB timed media resource exceeds its XML byte budget",
                ));
            }
            parse_xml(data)?;
        }
        if book.archive.index_for_name(path).is_none() {
            if !paths.contains_key(path) {
                plan.claim_path(path)?;
                plan.add_manifest(path, media, "")?;
            } else if paths.get(path) != Some(&media.as_str()) {
                return Err(argument(
                    "EPUB resource media type conflicts with its manifest",
                ));
            }
        } else if paths.get(path) != Some(&media.as_str()) {
            return Err(argument(
                "EPUB resource replacement requires its existing manifest and media type",
            ));
        }
        if media == "image/svg+xml" && book.archive.index_for_name(path).is_some() {
            let xml = parse_xml(data)?;
            editor::collect_removed_anchors(
                &mut book,
                path,
                data,
                &xml,
                &mut removed_anchors,
                &mut anchor_bytes,
            )?;
        }
        let replacement = if let Some(obfuscated) = book
            .obfuscated_fonts
            .as_ref()
            .filter(|fonts| fonts.contains(path))
        {
            if data.len() > fonts::MAX_FONT_BYTES {
                return Err(limited("EPUB replacement font exceeds 8 MiB"));
            }
            let decoded = wordcraft_fonts::decode_font_file(data).map_err(|err| {
                argument(&format!(
                    "EPUB font replacement requires a plain font: {err}"
                ))
            })?;
            if decoded.len() > fonts::MAX_FONT_BYTES {
                return Err(limited("EPUB decoded font exceeds 8 MiB"));
            }
            skrifa::raw::FileRef::new(&decoded)
                .map_err(|err| argument(&format!("invalid EPUB replacement font: {err}")))?;
            let mut stored = data.clone();
            obfuscated.apply(path, &mut stored);
            Cow::Owned(stored)
        } else {
            Cow::Borrowed(data.as_slice())
        };
        plan.files.insert(path.clone(), replacement);
    }
    if !plan.manifest_additions.is_empty() {
        let at = inner_range(package_source, manifest_node)?.end;
        edits.push((at..at, std::mem::take(&mut plan.manifest_additions)));
    }
    if epub3
        && (!edits.is_empty()
            || !plan.files.is_empty()
            || !removed.is_empty()
            || !overlays.is_empty())
    {
        if modified.len() != 20
            || !modified
                .bytes()
                .enumerate()
                .all(|(index, byte)| match index {
                    4 | 7 => byte == b'-',
                    10 => byte == b'T',
                    13 | 16 => byte == b':',
                    19 => byte == b'Z',
                    _ => byte.is_ascii_digit(),
                })
        {
            return Err(argument(
                "EPUB modification time must be an ISO UTC timestamp",
            ));
        }
        let stamp = escape_text(modified);
        if let Some(meta) = metadata.children().find(|node| {
            name(*node, OPF, "meta")
                && node.attribute("property") == Some("dcterms:modified")
                && node.attribute("refines").is_none()
        }) {
            let range = inner_range(package_source, meta)?;
            edits.push((range, stamp));
        } else {
            let at = inner_range(package_source, metadata)?.end;
            edits.push((
                at..at,
                format!("<meta xmlns=\"{OPF}\" property=\"dcterms:modified\">{stamp}</meta>"),
            ));
        }
    }
    let mut revised = if edits.is_empty() {
        Cow::Borrowed(package_source)
    } else {
        Cow::Owned(apply_edits(package_source, edits, METADATA_BYTES)?)
    };
    if !overlays.is_empty() {
        if !epub3 {
            return Err(unsupported("authoring media overlays requires EPUB 3"));
        }
        revised = Cow::Owned(narration::edit(&revised, &mut plan, &mut book, overlays)?);
    }
    if !removals.is_empty() {
        revised = Cow::Owned(removal::apply(&revised, &plan, &mut book, &removals)?);
    }
    let empty_encryption = remove_font_declarations(&mut plan, &mut book, &removals)?;
    removals.extend(empty_encryption.as_deref());
    if revised != package_source {
        plan.files.insert(
            package_path.clone(),
            Cow::Owned(revised.into_owned().into_bytes()),
        );
    }
    if plan.files.is_empty() && removals.is_empty() {
        return Ok(data.to_vec());
    }
    if !removed_anchors.is_empty() {
        let staged = plan
            .files
            .iter()
            .map(|(path, data)| (path.as_str(), data.as_ref()))
            .collect();
        editor::check_staged_links(&mut book, &staged, &removed_anchors, &removals)?;
    }
    let (replacements, additions) = plan
        .files
        .iter()
        .partition::<Vec<_>, _>(|(p, _)| book.archive.index_for_name(p).is_some());
    let updated = editor::rewrite(
        &mut book,
        replacements
            .into_iter()
            .map(|(path, value)| (path.as_str(), value.as_ref()))
            .collect(),
        additions
            .into_iter()
            .map(|(path, value)| (path.as_str(), value.as_ref()))
            .collect(),
        &removals,
    )?;
    let extracted = extract(&updated, None)?;
    if authored_nav.is_some() {
        toc::validate_replacement(&updated, &extracted.metadata)?;
    }
    editor::validate_svg_chapters(&updated, svg_chapters.iter().map(String::as_str))?;
    Ok(updated)
}

fn remove_font_declarations(
    plan: &mut Plan<'_>,
    book: &mut Book<'_>,
    removed: &HashSet<&str>,
) -> Result<Option<String>, EngineError> {
    let Some(fonts) = &book.obfuscated_fonts else {
        return Ok(None);
    };
    let edits: Vec<_> = removed
        .iter()
        .filter_map(|path| fonts.declarations.get(*path))
        .map(|range| (range.clone(), String::new()))
        .collect();
    if edits.is_empty() {
        return Ok(None);
    }
    let path = fonts.encryption_path.clone();
    if edits.len() == fonts.declarations.len() {
        return Ok(Some(path));
    }
    let bytes = book.read(&path, METADATA_BYTES)?;
    let source = std::str::from_utf8(&bytes).map_err(invalid)?;
    let encryption = apply_edits(source, edits, METADATA_BYTES)?;
    plan.files.insert(path, Cow::Owned(encryption.into_bytes()));
    Ok(None)
}

struct Plan<'a> {
    occupied: HashSet<String>,
    ids: HashSet<String>,
    package: &'a str,
    counter: usize,
    manifest_additions: String,
    files: Files<'a>,
}

impl Plan<'_> {
    fn new_path(&mut self, role: &str, extension: &str) -> String {
        let stem = self.package.rsplit_once('/').map(|(parent, _)| parent);
        loop {
            self.counter += 1;
            let name = format!("doccraft-{role}-{}.{extension}", self.counter);
            let path = stem.map_or_else(|| name.clone(), |parent| format!("{parent}/{name}"));
            if self.occupied.insert(path.clone()) {
                return path;
            }
        }
    }

    fn claim_path(&mut self, path: &str) -> Result<(), EngineError> {
        if !self.occupied.insert(path.to_owned()) {
            return Err(argument("EPUB archive path already exists"));
        }
        Ok(())
    }

    fn add_manifest(
        &mut self,
        path: &str,
        media: &str,
        attributes: &str,
    ) -> Result<String, EngineError> {
        let id = loop {
            self.counter += 1;
            let candidate = format!("doccraft-resource-{}", self.counter);
            if self.ids.insert(candidate.clone()) {
                break candidate;
            }
        };
        let href = relative_url(self.package, path);
        append(&mut self.manifest_additions, &format!("<item xmlns=\"{OPF}\" id=\"{id}\" href=\"{href}\" media-type=\"{media}\"{attributes}/>"), METADATA_BYTES)?;
        Ok(id)
    }
}

fn check_path(path: &str) -> Result<(), EngineError> {
    validate_archive_path(path)?;
    let lower = path.to_ascii_lowercase();
    if lower == "mimetype"
        || lower.starts_with("meta-inf/")
        || lower == "meta-inf"
        || lower.ends_with('/')
        || lower.ends_with(".opf")
    {
        return Err(argument("EPUB archive path is reserved"));
    }
    Ok(())
}

fn editable_resource(media: &str) -> bool {
    matches!(
        media,
        "text/css"
            | "text/vtt"
            | "application/smil+xml"
            | "application/ttml+xml"
            | "application/ogg"
            | "audio/ogg; codecs=opus"
    ) || obfuscation::is_font(media)
        || media.split_once('/').is_some_and(|(kind, subtype)| {
            matches!(kind, "image" | "audio" | "video" | "font")
                && !subtype.is_empty()
                && subtype
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"-+._".contains(&byte))
        })
}

fn relative_url(base: &str, path: &str) -> String {
    let mut folders: Vec<_> = base.split('/').collect();
    folders.pop();
    let target: Vec<_> = path.split('/').collect();
    let shared = folders
        .iter()
        .zip(target.iter().take(target.len().saturating_sub(1)))
        .take_while(|(a, b)| a == b)
        .count();
    let mut url = "../".repeat(folders.len() - shared);
    for (i, part) in target[shared..].iter().enumerate() {
        if i != 0 {
            url.push('/');
        }
        encode_url_part(part, &mut url);
    }
    url
}

fn encode_url_part(part: &str, url: &mut String) {
    for byte in part.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            url.push(char::from(byte));
        } else {
            let _ = write!(url, "%{byte:02X}");
        }
    }
}

fn opening_end(source: &str, node: Node<'_, '_>) -> Result<usize, EngineError> {
    let start = node.range().start;
    let mut reader = quick_xml::Reader::from_str(&source[node.range()]);
    if !matches!(
        reader.read_event().map_err(invalid)?,
        quick_xml::events::Event::Start(_)
    ) {
        return Err(unsupported(
            "self-closing EPUB package or navigation sections",
        ));
    }
    Ok(start + reader.buffer_position() as usize)
}

fn inner_range(source: &str, node: Node<'_, '_>) -> Result<Range<usize>, EngineError> {
    let start = opening_end(source, node)?;
    let end = source[node.range()]
        .rfind("</")
        .ok_or_else(|| invalid("missing EPUB closing tag"))?
        + node.range().start;
    Ok(start..end)
}

pub(super) fn append(output: &mut String, text: &str, limit: usize) -> Result<(), EngineError> {
    if output.len().saturating_add(text.len()) > limit {
        return Err(limited("edited EPUB XML exceeds its byte budget"));
    }
    output.push_str(text);
    Ok(())
}

pub(super) fn apply_edits(
    source: &str,
    mut edits: Vec<(Range<usize>, String)>,
    limit: usize,
) -> Result<String, EngineError> {
    edits.sort_by_key(|(range, _)| (range.start, range.end));
    let mut output = String::new();
    let mut start = 0;
    for (range, replacement) in edits {
        if range.start < start {
            return Err(invalid("overlapping EPUB XML changes"));
        }
        append(&mut output, &source[start..range.start], limit)?;
        append(&mut output, &replacement, limit)?;
        start = range.end;
    }
    append(&mut output, &source[start..], limit)?;
    Ok(output)
}

fn argument(reason: &str) -> EngineError {
    EngineError::InvalidArgument(reason.to_owned())
}
