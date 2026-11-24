//! Rebuild a guarded EPUB archive while preserving untouched members.
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io::Write,
};

use roxmltree::{Document, Node};
use serde_json::Value;

use super::{
    external, extract, invalid, limited, name, parse_xml, required, resolve, unsupported, Book, DC,
    METADATA_BYTES, MIME, OPF, XHTML, XML_BYTES,
};
use crate::{
    author_epub::{escape_attribute, escape_text, validate_text, BoundedOutput},
    EngineError,
};

pub(super) fn open_for_edit(data: &[u8]) -> Result<Book<'_>, EngineError> {
    let book = Book::open(data)?;
    if book
        .archive
        .file_names()
        .any(|path| path.eq_ignore_ascii_case("META-INF/signatures.xml"))
    {
        return Err(unsupported("editing signed packages"));
    }
    Ok(book)
}

/// Change selected Dublin Core values while retaining other XML and ZIP members.
pub(crate) fn update_metadata(
    data: &[u8],
    title: Option<&str>,
    description: Option<&str>,
) -> Result<Vec<u8>, EngineError> {
    let mut book = open_for_edit(data)?;
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
    rewrite(
        &mut book,
        HashMap::from([(package_path.as_str(), new_package.as_bytes())]),
        BTreeMap::new(),
        &HashSet::new(),
    )
}

pub(super) fn rewrite(
    book: &mut Book<'_>,
    mut replacements: HashMap<&str, &[u8]>,
    additions: BTreeMap<&str, &[u8]>,
    removed: &HashSet<&str>,
) -> Result<Vec<u8>, EngineError> {
    crate::limits::validate_member_count(
        book.archive
            .len()
            .saturating_sub(removed.len())
            .saturating_add(additions.len()),
    )?;
    for path in additions.keys() {
        super::validate_archive_path(path)?;
        if book.archive.index_for_name(path).is_some() {
            return Err(invalid("new EPUB member already exists"));
        }
    }
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
            if member.name() == "mimetype" || removed.contains(member.name()) {
                continue;
            }
            if let Some(content) = replacements.remove(member.name()) {
                writer.start_file(
                    member.name(),
                    zip::write::SimpleFileOptions::default()
                        .compression_method(zip::CompressionMethod::Stored),
                )?;
                writer.write_all(content)?;
            } else {
                writer.raw_copy_file(member)?;
            }
        }
        for (path, content) in additions {
            writer.start_file(
                path,
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored),
            )?;
            writer.write_all(content)?;
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
    if !replacements.is_empty() {
        return Err(invalid("a replacement member is absent from the archive"));
    }
    let result = sink.into_bytes();
    crate::archive_safety::open_epub(&result)?;
    Ok(result)
}

/// Replace the content files at selected spine positions in one transaction.
pub(crate) fn update_chapters(
    data: &[u8],
    changes: &[(i64, String)],
) -> Result<Vec<u8>, EngineError> {
    crate::limits::validate_member_count(changes.len())?;
    let mut total = 0;
    for (_, replacement) in changes {
        if replacement.len() > XML_BYTES {
            return Err(limited("replacement chapter exceeds its XML byte budget"));
        }
        super::charge(
            &mut total,
            replacement.len(),
            crate::limits::MAX_INPUT_SIZE,
            "chapter replacements exceed their byte budget",
        )?;
    }
    let mut book = open_for_edit(data)?;
    if changes.is_empty() {
        return Ok(data.to_vec());
    }
    let extracted = extract(data, None)?;
    let fixed_paths: HashSet<_> = extracted
        .pages
        .iter()
        .filter(|page| super::rendition::fixed(page))
        .filter_map(|page| page.metadata["chapter_path"].as_str())
        .collect();
    let mut svg_paths = HashSet::new();
    let mut replacements = HashMap::new();
    for (index, replacement) in changes {
        let count = extracted.pages.len() as i64;
        let index = if *index < 0 { index + count } else { *index };
        let page = usize::try_from(index)
            .ok()
            .and_then(|i| extracted.pages.get(i))
            .ok_or_else(|| EngineError::PageIndex("EPUB chapter is out of range".into()))?;
        let path = page.metadata["chapter_path"]
            .as_str()
            .ok_or_else(|| invalid("missing chapter path"))?;
        if page.metadata["chapter_media_type"] == "image/svg+xml" {
            svg_paths.insert(path);
        }
        if let Some(existing) = replacements.insert(path, replacement.as_bytes()) {
            if existing != replacement.as_bytes() {
                return Err(EngineError::InvalidArgument(
                    "conflicting replacements of the same EPUB chapter resource".into(),
                ));
            }
        }
    }
    let mut links: HashMap<&str, HashSet<&str>> = HashMap::new();
    for link in extracted.pages.iter().flat_map(|page| &page.links) {
        if let (Some(path), Some(fragment)) = (
            link.metadata.get("target_path").and_then(Value::as_str),
            link.metadata.get("target_fragment").and_then(Value::as_str),
        ) {
            if replacements.contains_key(path) {
                links.entry(path).or_default().insert(fragment);
            }
        }
    }
    for field in ["navigation", "media_overlays"] {
        if let Some(value) = extracted.metadata.get(field) {
            collect_fragments(value, &replacements, &mut links);
        }
    }
    let epub3 = extracted.metadata["epub_version"]
        .as_str()
        .is_some_and(|version| version.starts_with('3'));
    let mut declarations = HashMap::new();
    for (&path, &replacement) in &replacements {
        let xml = parse_xml(replacement)?;
        let root = xml.root_element();
        if svg_paths.contains(path) {
            if !root.has_tag_name((super::svg::NS, "svg")) {
                return Err(invalid("replacement SVG chapter must have an SVG root"));
            }
            if fixed_paths.contains(path) && super::svg::viewport(root).is_none() {
                return Err(invalid(
                    "replacement fixed-layout SVG chapter requires a numeric viewBox",
                ));
            }
        } else {
            if !name(root, XHTML, "html") {
                return Err(invalid("replacement chapter must be XHTML"));
            }
            if fixed_paths.contains(path) && super::layout::dimensions(root).is_none() {
                return Err(invalid(
                    "replacement fixed-layout XHTML chapter requires a numeric viewport",
                ));
            }
            if epub3 {
                declarations.insert(path, xhtml_properties(root));
            }
        }
        // Preserve target IDs used by existing navigation, synchronization and links.
        if let Some(required) = links.get(path) {
            let ids = anchors(&xml);
            if required.iter().any(|id| !ids.contains(id)) {
                let original = book.read(path, XML_BYTES)?;
                let source = parse_xml(&original)?;
                let old_ids = anchors(&source);
                if required
                    .iter()
                    .any(|id| old_ids.contains(id) && !ids.contains(id))
                {
                    return Err(EngineError::InvalidArgument(
                        "replacement chapter removes a referenced anchor".into(),
                    ));
                }
            }
        }
    }
    let package = sync_chapter_properties(&mut book, &declarations)?;
    if let Some((path, xml)) = &package {
        replacements.insert(path, xml.as_bytes());
    }
    let result = rewrite(&mut book, replacements, BTreeMap::new(), &HashSet::new())?;
    extract(&result, None)?;
    validate_svg_chapters(&result, svg_paths.into_iter())?;
    Ok(result)
}

pub(super) fn validate_svg_chapters<'a>(
    data: &[u8],
    paths: impl Iterator<Item = &'a str>,
) -> Result<(), EngineError> {
    let mut paths = paths.peekable();
    if paths.peek().is_some() {
        let mut book = Book::open(data)?;
        for path in paths {
            let source = book.read(path, XML_BYTES)?;
            super::svg::embedded(&source, path, &mut book)?;
        }
    }
    Ok(())
}

const XHTML_PROPERTIES: [&str; 2] = ["mathml", "svg"];

pub(super) fn xhtml_properties(root: Node<'_, '_>) -> Vec<&'static str> {
    let mut math = false;
    let mut svg = false;
    for node in root.descendants().filter(Node::is_element) {
        match node.tag_name().namespace() {
            Some(super::math::MATHML) => math = true,
            Some(super::svg::NS) => svg = true,
            _ => {}
        }
        if math && svg {
            break;
        }
    }
    XHTML_PROPERTIES
        .into_iter()
        .zip([math, svg])
        .filter_map(|(property, present)| present.then_some(property))
        .collect()
}

pub(super) fn properties_attribute(properties: &[&str]) -> String {
    if properties.is_empty() {
        String::new()
    } else {
        format!(
            " properties=\"{}\"",
            escape_attribute(&properties.join(" "))
        )
    }
}

pub(super) fn before_attributes(source: &str, element: Node<'_, '_>) -> Result<usize, EngineError> {
    let name_start = element.range().start + 1;
    let name_length = source[name_start..]
        .bytes()
        .position(|byte| byte.is_ascii_whitespace() || matches!(byte, b'/' | b'>'))
        .ok_or_else(|| invalid("invalid EPUB manifest opening tag"))?;
    Ok(name_start + name_length)
}

fn sync_chapter_properties(
    book: &mut Book<'_>,
    declarations: &HashMap<&str, Vec<&str>>,
) -> Result<Option<(String, String)>, EngineError> {
    if declarations.is_empty() {
        return Ok(None);
    }
    let path = book.package_path()?;
    let bytes = book.read(&path, METADATA_BYTES)?;
    let source = std::str::from_utf8(&bytes).map_err(invalid)?;
    let xml = parse_xml(&bytes)?;
    let manifest = required(xml.root_element(), "manifest")?;
    let mut edits = Vec::new();
    for item in manifest.children().filter(|&node| name(node, OPF, "item")) {
        let href = super::attribute(item, "href")?;
        if external(href) {
            continue;
        }
        let target = resolve(&path, href)?;
        let Some(inferred) = declarations.get(target.as_str()) else {
            continue;
        };
        let current: Vec<_> = item
            .attribute("properties")
            .unwrap_or_default()
            .split_ascii_whitespace()
            .collect();
        let mut properties = Vec::new();
        for &property in &current {
            if !XHTML_PROPERTIES.contains(&property)
                || (inferred.contains(&property) && !properties.contains(&property))
            {
                properties.push(property);
            }
        }
        for &property in inferred {
            if !properties.contains(&property) {
                properties.push(property);
            }
        }
        if properties == current {
            continue;
        }
        let range = if let Some(attribute) = item.attribute_node("properties") {
            attribute.range()
        } else {
            let at = before_attributes(source, item)?;
            at..at
        };
        edits.push((range, properties_attribute(&properties)));
    }
    if edits.is_empty() {
        return Ok(None);
    }
    let updated = super::structure::apply_edits(source, edits, METADATA_BYTES)?;
    Ok(Some((path, updated)))
}

fn anchors<'a>(xml: &'a Document<'_>) -> HashSet<&'a str> {
    xml.descendants()
        .filter(roxmltree::Node::is_element)
        .flat_map(super::element_ids)
        .collect()
}

fn collect_fragments<'a>(
    value: &'a Value,
    replacements: &HashMap<&str, &[u8]>,
    links: &mut HashMap<&'a str, HashSet<&'a str>>,
) {
    match value {
        Value::Object(fields) => {
            if let (Some(path), Some(fragment)) = (
                fields.get("path").and_then(Value::as_str),
                fields.get("fragment").and_then(Value::as_str),
            ) {
                if replacements.contains_key(path) {
                    links.entry(path).or_default().insert(fragment);
                }
            }
            for child in fields.values() {
                collect_fragments(child, replacements, links);
            }
        }
        Value::Array(items) => {
            for child in items {
                collect_fragments(child, replacements, links);
            }
        }
        _ => {}
    }
}
