use std::io::{Cursor, Read};

use futures::executor::block_on;
use serde_json::{json, Map, Value};

use crate::model::{BlockPayload, DocumentPayload, PagePayload};
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
    let chapters = block_on(book.extract_text_only())
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

    let pages = chapters
        .into_iter()
        .enumerate()
        .map(|(number, text)| PagePayload {
            number,
            blocks: if text.trim().is_empty() {
                Vec::new()
            } else {
                vec![BlockPayload::paragraph(text.trim().to_string())]
            },
            metadata: Map::from_iter([("spine_index".into(), json!(number))]),
            ..Default::default()
        })
        .collect();

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

fn validate_container(data: &[u8]) -> Result<(), EngineError> {
    let mut archive = zip::ZipArchive::new(Cursor::new(data))
        .map_err(|error| EngineError::Malformed(format!("invalid EPUB ZIP: {error}")))?;
    if archive.by_name("META-INF/encryption.xml").is_ok() {
        return Err(EngineError::Unsupported(
            "encrypted or DRM-protected EPUB resources are not supported".into(),
        ));
    }
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
    Ok(())
}

fn insert(metadata: &mut Map<String, Value>, key: &str, value: Option<String>) {
    if let Some(value) = value {
        metadata.insert(key.into(), Value::String(value));
    }
}
