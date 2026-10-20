use std::io::{Cursor, Read};

use base64::Engine;
use futures::executor::block_on;
use lexepub::AstNode;
use serde_json::{json, Map, Value};

use crate::model::{BlockPayload, DocumentPayload, ImagePayload, LinkPayload, PagePayload};
use crate::EngineError;

pub fn is_epub(data: &[u8]) -> Result<bool, EngineError> {
    let mut archive = match zip::ZipArchive::new(Cursor::new(data)) {
        Ok(archive) => archive,
        Err(_) => return Ok(false),
    };
    let Ok(mut member) = archive.by_name("mimetype") else {
        return Ok(false);
    };
    if member.size() > 64 {
        return Ok(false);
    }
    let mut value = String::new();
    member
        .read_to_string(&mut value)
        .map_err(|error| EngineError::Malformed(format!("invalid EPUB mimetype: {error}")))?;
    Ok(value.trim() == "application/epub+zip")
}

pub fn extract(data: &[u8], password: Option<&str>) -> Result<DocumentPayload, EngineError> {
    if password.is_some() {
        return Err(EngineError::Unsupported(
            "EPUB passwords and DRM are not supported".into(),
        ));
    }
    crate::ooxml::preflight(data)?;
    validate_container(data)?;

    let mut book = lexepub::LexEpub::from_sync_reader(Cursor::new(data.to_vec()))
        .map_err(|error| EngineError::Malformed(format!("invalid EPUB: {error}")))?;
    let metadata = block_on(book.get_metadata())
        .map_err(|error| EngineError::Malformed(format!("invalid EPUB metadata: {error}")))?;
    let chapters = block_on(book.extract_ast())
        .map_err(|error| EngineError::Malformed(format!("invalid EPUB content: {error}")))?;

    let expected = metadata.spine.len().max(metadata.chapter_count);
    if expected == 0 || chapters.is_empty() {
        return Err(EngineError::Malformed(
            "EPUB contains no readable spine chapters".into(),
        ));
    }
    if chapters.len() != expected {
        return Err(EngineError::Malformed(format!(
            "EPUB chapter extraction was incomplete: expected {expected}, extracted {}",
            chapters.len()
        )));
    }
    if chapters.len() > 10_000 {
        return Err(EngineError::ResourceLimit(
            "EPUB chapter count exceeds 10000".into(),
        ));
    }

    let mut pages = Vec::with_capacity(chapters.len());
    for (number, chapter) in chapters.into_iter().enumerate() {
        validate_xhtml(&chapter.chapter_info.content)?;
        let mut blocks = Vec::new();
        if let Some(ast) = chapter.ast.as_ref() {
            collect_blocks(ast, &mut blocks, number);
        }
        if blocks.is_empty() && !chapter.content.trim().is_empty() {
            blocks.push(BlockPayload::paragraph(chapter.content.trim().to_string()));
        }
        let mut images = Vec::new();
        for block in &mut blocks {
            for image in &mut block.images {
                if has_external_scheme(&image.name) {
                    return Err(EngineError::Unsupported(
                        "remote EPUB images are not fetched".into(),
                    ));
                }
                let resource_name = package_resource_name(&image.name)?;
                let data = block_on(book.read_chapter_resource(number, resource_name)).map_err(
                    |error| {
                        EngineError::Malformed(format!(
                            "EPUB referenced image {} is unavailable: {error}",
                            image.name
                        ))
                    },
                )?;
                image.data = base64::engine::general_purpose::STANDARD.encode(data);
                image.media_type = image_media_type(resource_name)?.into();
                images.push(image.clone());
            }
        }
        let links = blocks
            .iter()
            .flat_map(|block| block.links.iter().cloned())
            .collect();
        pages.push(PagePayload {
            number,
            blocks,
            links,
            images,
            metadata: Map::from_iter([
                ("spine_index".into(), json!(number)),
                ("chapter_href".into(), json!(chapter.chapter_info.href)),
                ("chapter_title".into(), json!(chapter.title)),
            ]),
            ..Default::default()
        });
    }

    let mut values = Map::new();
    insert(&mut values, "title", metadata.title);
    insert(&mut values, "description", metadata.description);
    insert(&mut values, "publisher", metadata.publisher);
    insert(&mut values, "publication_date", metadata.publication_date);
    insert(&mut values, "rights", metadata.rights);
    insert(&mut values, "epub_version", metadata.version);
    values.insert("authors".into(), json!(metadata.authors));
    values.insert("languages".into(), json!(metadata.languages));
    values.insert("subjects".into(), json!(metadata.subjects));
    values.insert("identifiers".into(), json!(metadata.identifiers));
    values.insert("contributors".into(), json!(metadata.contributors));
    values.insert("chapter_count".into(), json!(expected));
    values.insert("chapter_mapping".into(), json!("spine_chapter"));

    Ok(DocumentPayload {
        format: "epub".into(),
        metadata: values,
        pages,
        stories: Vec::new(),
    })
}

fn validate_xhtml(data: &[u8]) -> Result<(), EngineError> {
    validate_xml(data, "XHTML")
}

#[allow(deprecated)]
fn validate_xml(data: &[u8], label: &str) -> Result<(), EngineError> {
    std::str::from_utf8(data)
        .map_err(|error| EngineError::Malformed(format!("invalid EPUB {label} UTF-8: {error}")))?;
    let mut reader = quick_xml::Reader::from_reader(data);
    let mut buffer = Vec::new();
    let mut depth = 0usize;
    let mut tokens = 0usize;
    loop {
        tokens = tokens
            .checked_add(1)
            .filter(|value| *value <= 1_000_000)
            .ok_or_else(|| {
                EngineError::ResourceLimit(format!("EPUB {label} has too many XML tokens"))
            })?;
        match reader.read_event_into(&mut buffer) {
            Ok(quick_xml::events::Event::Start(event)) => {
                depth = depth
                    .checked_add(1)
                    .filter(|value| *value <= 256)
                    .ok_or_else(|| {
                        EngineError::ResourceLimit(format!("EPUB {label} XML nesting exceeds 256"))
                    })?;
                validate_attributes(&event, reader.decoder(), label)?;
            }
            Ok(quick_xml::events::Event::Empty(event)) => {
                validate_attributes(&event, reader.decoder(), label)?;
            }
            Ok(quick_xml::events::Event::End(_)) => depth = depth.saturating_sub(1),
            Ok(quick_xml::events::Event::Text(text)) => {
                text.decode().map_err(|error| {
                    EngineError::Malformed(format!("invalid EPUB {label} text: {error}"))
                })?;
            }
            Ok(quick_xml::events::Event::CData(text)) => {
                std::str::from_utf8(text.as_ref()).map_err(|error| {
                    EngineError::Malformed(format!("invalid EPUB {label} CDATA: {error}"))
                })?;
            }
            Ok(quick_xml::events::Event::DocType(_)) => {
                return Err(EngineError::Malformed(format!(
                    "EPUB {label} must not contain a DTD"
                )))
            }
            Ok(quick_xml::events::Event::Eof) => return Ok(()),
            Ok(_) => {}
            Err(error) => {
                return Err(EngineError::Malformed(format!(
                    "invalid EPUB {label}: {error}"
                )))
            }
        }
        buffer.clear();
    }
}

#[allow(deprecated)]
fn validate_attributes(
    event: &quick_xml::events::BytesStart<'_>,
    decoder: quick_xml::encoding::Decoder,
    label: &str,
) -> Result<(), EngineError> {
    for attribute in event.attributes() {
        let attribute = attribute.map_err(|error| {
            EngineError::Malformed(format!("invalid EPUB {label} attribute: {error}"))
        })?;
        attribute
            .decode_and_unescape_value(decoder)
            .map_err(|error| {
                EngineError::Malformed(format!("invalid EPUB {label} attribute value: {error}"))
            })?;
    }
    Ok(())
}

fn collect_blocks(node: &AstNode, blocks: &mut Vec<BlockPayload>, page: usize) {
    let AstNode::Element { tag, children, .. } = node else {
        return;
    };
    let tag = tag.to_ascii_lowercase();
    if tag == "head" || tag == "title" {
        return;
    }
    let semantic_block = tag == "p"
        || matches!(
            tag.as_str(),
            "h1" | "h2"
                | "h3"
                | "h4"
                | "h5"
                | "h6"
                | "li"
                | "blockquote"
                | "pre"
                | "figcaption"
                | "dt"
                | "dd"
                | "td"
                | "th"
        );
    if semantic_block {
        let mut text = String::new();
        let mut links = Vec::new();
        let mut images = Vec::new();
        collect_inline(node, &mut text, &mut links, &mut images, page);
        if !text.trim().is_empty() || !images.is_empty() {
            blocks.push(BlockPayload {
                kind: if tag.starts_with('h') && tag.len() == 2 {
                    "heading".into()
                } else {
                    "paragraph".into()
                },
                text,
                level: tag.strip_prefix('h').and_then(|value| value.parse().ok()),
                links,
                images,
                ..Default::default()
            });
        }
        return;
    }
    if tag == "img" {
        let mut text = String::new();
        let mut links = Vec::new();
        let mut images = Vec::new();
        collect_inline(node, &mut text, &mut links, &mut images, page);
        if !images.is_empty() {
            blocks.push(BlockPayload {
                kind: "paragraph".into(),
                images,
                ..Default::default()
            });
        }
        return;
    }
    for child in children {
        collect_blocks(child, blocks, page);
    }
}

fn collect_inline(
    node: &AstNode,
    text: &mut String,
    links: &mut Vec<LinkPayload>,
    images: &mut Vec<ImagePayload>,
    page: usize,
) {
    match node {
        AstNode::Text { content } => text.push_str(content),
        AstNode::Comment { .. } => {}
        AstNode::Element {
            tag,
            attrs,
            children,
            ..
        } => {
            let tag = tag.to_ascii_lowercase();
            if tag == "img" {
                if let Some(src) = attrs.get("src") {
                    images.push(ImagePayload {
                        name: src.clone(),
                        alt: attrs.get("alt").cloned().unwrap_or_default(),
                        page: Some(page),
                        ..Default::default()
                    });
                }
                return;
            }
            if tag == "br" {
                text.push('\n');
                return;
            }
            let start = text.chars().count();
            for child in children {
                collect_inline(child, text, links, images, page);
            }
            if tag == "a" {
                if let Some(href) = attrs.get("href") {
                    let end = text.chars().count();
                    let label = text.chars().skip(start).take(end - start).collect();
                    links.push(LinkPayload {
                        uri: href.clone(),
                        text: label,
                        page: Some(page),
                        metadata: Map::from_iter([
                            ("start".into(), json!(start)),
                            ("end".into(), json!(end)),
                            (
                                "target_kind".into(),
                                json!(if has_external_scheme(href) {
                                    "external"
                                } else {
                                    "internal"
                                }),
                            ),
                        ]),
                        ..Default::default()
                    });
                }
            }
        }
    }
}

fn has_external_scheme(value: &str) -> bool {
    value.split(['/', '?', '#']).any(|segment| {
        segment.find(':').is_some_and(|index| {
            index > 0
                && segment[..index]
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.'))
        })
    })
}

fn package_resource_name(value: &str) -> Result<&str, EngineError> {
    let name = value.split(['#', '?']).next().unwrap_or(value);
    if name.is_empty()
        || name.starts_with('/')
        || name.starts_with('\\')
        || name.split(['/', '\\']).any(|part| part == "..")
    {
        return Err(EngineError::UnsafeArchive(
            "EPUB resource path is unsafe".into(),
        ));
    }
    Ok(name)
}

fn image_media_type(name: &str) -> Result<&'static str, EngineError> {
    let path = name
        .split(['#', '?'])
        .next()
        .unwrap_or(name)
        .to_ascii_lowercase();
    if path.ends_with(".png") {
        Ok("image/png")
    } else if path.ends_with(".jpg") || path.ends_with(".jpeg") {
        Ok("image/jpeg")
    } else if path.ends_with(".gif") {
        Ok("image/gif")
    } else if path.ends_with(".svg") {
        Ok("image/svg+xml")
    } else {
        Err(EngineError::Unsupported(format!(
            "EPUB image type for {name} is not supported"
        )))
    }
}

fn validate_container(data: &[u8]) -> Result<(), EngineError> {
    let mut archive = zip::ZipArchive::new(Cursor::new(data))
        .map_err(|error| EngineError::Malformed(format!("invalid EPUB ZIP: {error}")))?;
    if archive.by_name("META-INF/encryption.xml").is_ok() {
        return Err(EngineError::Unsupported(
            "encrypted or DRM-protected EPUB resources are not supported".into(),
        ));
    }
    {
        let mut mimetype = archive
            .by_name("mimetype")
            .map_err(|_| EngineError::Malformed("EPUB is missing its mimetype entry".into()))?;
        let mut value = String::new();
        mimetype
            .read_to_string(&mut value)
            .map_err(|error| EngineError::Malformed(format!("invalid EPUB mimetype: {error}")))?;
        if value.trim() != "application/epub+zip" {
            return Err(EngineError::Malformed("invalid EPUB mimetype".into()));
        }
    }

    let container = {
        let mut member = archive
            .by_name("META-INF/container.xml")
            .map_err(|_| EngineError::Malformed("EPUB is missing META-INF/container.xml".into()))?;
        let size = usize::try_from(member.size())
            .map_err(|_| EngineError::ResourceLimit("EPUB container XML is too large".into()))?;
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(size).map_err(|_| {
            EngineError::ResourceLimit("cannot allocate bounded EPUB container XML".into())
        })?;
        member.read_to_end(&mut bytes).map_err(|error| {
            EngineError::Malformed(format!("invalid EPUB container XML: {error}"))
        })?;
        bytes
    };
    validate_xml(&container, "container XML")?;
    let rootfile = container_rootfile(&container)?;
    package_resource_name(&rootfile)?;

    let package = {
        let mut member = archive
            .by_name(&rootfile)
            .map_err(|_| EngineError::Malformed(format!("EPUB package {rootfile} is missing")))?;
        let size = usize::try_from(member.size())
            .map_err(|_| EngineError::ResourceLimit("EPUB package XML is too large".into()))?;
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(size).map_err(|_| {
            EngineError::ResourceLimit("cannot allocate bounded EPUB package XML".into())
        })?;
        member.read_to_end(&mut bytes).map_err(|error| {
            EngineError::Malformed(format!("invalid EPUB package XML: {error}"))
        })?;
        bytes
    };
    validate_xml(&package, "package XML")?;
    validate_package_structure(&package)?;

    for index in 0..archive.len() {
        let mut member = archive
            .by_index(index)
            .map_err(|error| EngineError::Malformed(format!("invalid EPUB ZIP member: {error}")))?;
        let name = member.name().to_ascii_lowercase();
        if member.is_dir() || !(name.ends_with(".xhtml") || name.ends_with(".html")) {
            continue;
        }
        let size = usize::try_from(member.size())
            .map_err(|_| EngineError::ResourceLimit("EPUB XHTML is too large".into()))?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(size)
            .map_err(|_| EngineError::ResourceLimit("cannot allocate bounded EPUB XHTML".into()))?;
        member.read_to_end(&mut bytes).map_err(|error| {
            EngineError::Malformed(format!("invalid EPUB XHTML member: {error}"))
        })?;
        validate_xhtml(&bytes)?;
    }
    Ok(())
}

#[allow(deprecated)]
fn container_rootfile(data: &[u8]) -> Result<String, EngineError> {
    let mut reader = quick_xml::Reader::from_reader(data);
    let mut buffer = Vec::new();
    let mut rootfile = None;
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(quick_xml::events::Event::Start(event))
            | Ok(quick_xml::events::Event::Empty(event))
                if event.local_name().as_ref() == b"rootfile" =>
            {
                let mut path = None;
                let mut media_type = None;
                for attribute in event.attributes() {
                    let attribute = attribute.map_err(|error| {
                        EngineError::Malformed(format!("invalid EPUB rootfile attribute: {error}"))
                    })?;
                    let value = attribute
                        .decode_and_unescape_value(reader.decoder())
                        .map_err(|error| {
                            EngineError::Malformed(format!(
                                "invalid EPUB rootfile attribute value: {error}"
                            ))
                        })?
                        .into_owned();
                    match attribute.key.local_name().as_ref() {
                        b"full-path" => path = Some(value),
                        b"media-type" => media_type = Some(value),
                        _ => {}
                    }
                }
                if media_type.as_deref() == Some("application/oebps-package+xml") {
                    if rootfile.is_some() {
                        return Err(EngineError::Malformed(
                            "EPUB container has multiple package rootfiles".into(),
                        ));
                    }
                    rootfile = path;
                }
            }
            Ok(quick_xml::events::Event::Eof) => {
                return rootfile.ok_or_else(|| {
                    EngineError::Malformed("EPUB container has no package rootfile".into())
                })
            }
            Ok(_) => {}
            Err(error) => {
                return Err(EngineError::Malformed(format!(
                    "invalid EPUB container XML: {error}"
                )))
            }
        }
        buffer.clear();
    }
}

fn validate_package_structure(data: &[u8]) -> Result<(), EngineError> {
    let mut reader = quick_xml::Reader::from_reader(data);
    let mut buffer = Vec::new();
    let mut package = false;
    let mut manifest = false;
    let mut spine = false;
    let mut itemrefs = 0usize;
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(quick_xml::events::Event::Start(event))
            | Ok(quick_xml::events::Event::Empty(event)) => match event.local_name().as_ref() {
                b"package" => package = true,
                b"manifest" => manifest = true,
                b"spine" => spine = true,
                b"itemref" => {
                    itemrefs = itemrefs.checked_add(1).ok_or_else(|| {
                        EngineError::ResourceLimit("EPUB spine count overflow".into())
                    })?;
                    if itemrefs > 10_000 {
                        return Err(EngineError::ResourceLimit(
                            "EPUB chapter count exceeds 10000".into(),
                        ));
                    }
                }
                _ => {}
            },
            Ok(quick_xml::events::Event::Eof) => break,
            Ok(_) => {}
            Err(error) => {
                return Err(EngineError::Malformed(format!(
                    "invalid EPUB package XML: {error}"
                )))
            }
        }
        buffer.clear();
    }
    if !package || !manifest || !spine || itemrefs == 0 {
        return Err(EngineError::Malformed(
            "EPUB package requires package, manifest and non-empty spine elements".into(),
        ));
    }
    Ok(())
}

fn insert(metadata: &mut Map<String, Value>, key: &str, value: Option<String>) {
    if let Some(value) = value {
        metadata.insert(key.into(), Value::String(value));
    }
}
