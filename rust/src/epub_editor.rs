//! Rebuild a guarded EPUB archive while preserving untouched members.
use std::{
    collections::{HashMap, HashSet},
    io::Write,
};

use roxmltree::Document;
use serde_json::Value;

use super::{
    extract, invalid, limited, name, parse_xml, required, unsupported, Book, DC, METADATA_BYTES,
    MIME, XHTML, XML_BYTES,
};
use crate::{
    author_epub::{escape_text, validate_text, BoundedOutput},
    EngineError,
};

fn open_for_edit(data: &[u8]) -> Result<Book<'_>, EngineError> {
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
    )
}

fn rewrite(
    book: &mut Book<'_>,
    mut replacements: HashMap<&str, &[u8]>,
) -> Result<Vec<u8>, EngineError> {
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

/// Replace the XHTML files at selected spine positions in one transaction.
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
            return Err(unsupported("chapter replacement requires XHTML content"));
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
    for (&path, &replacement) in &replacements {
        let xml = parse_xml(replacement)?;
        if !name(xml.root_element(), XHTML, "html") {
            return Err(invalid("replacement chapter must be XHTML"));
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
    let result = rewrite(&mut book, replacements)?;
    extract(&result, None)?;
    Ok(result)
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
