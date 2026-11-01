//! Rust-facing document operations. This API is an initial migration slice;
//! authoring and document sessions are not yet independent of Python.

use crate::{conversion, docx, epub, limits, ooxml, pdf, pptx, rendering, EngineError};

pub use crate::model::{
    BlockPayload, CellPayload, DocumentPayload, ImagePayload, LinkPayload, PagePayload,
    StoryPayload, TablePayload,
};

/// A clipping rectangle in displayed page points: x, y, width, height.
pub type Clip = (f64, f64, f64, f64);

/// Extract an owned, normalized document, subject to the same limits as Python.
pub fn extract(
    data: &[u8],
    filetype: &str,
    password: Option<&str>,
) -> Result<DocumentPayload, EngineError> {
    limits::validate_input_size(data.len())?;
    let payload = match normalize(filetype)? {
        "pdf" => pdf::extract(data, password),
        "docx" => docx::extract(data, password),
        "pptx" => pptx::extract(data),
        "epub" => epub::extract(data, password),
        _ => unreachable!(),
    }?;
    // Keep the normalized payload budget identical to the Python boundary.
    limits::bounded_json(&payload, limits::MAX_PAYLOAD_SIZE)?;
    Ok(payload)
}

/// Render a zero-based page as PNG. Clipping coordinates are in page points.
pub fn render(
    data: &[u8],
    filetype: &str,
    page: usize,
    dpi: f64,
    password: Option<&str>,
    clip: Option<Clip>,
) -> Result<Vec<u8>, EngineError> {
    limits::validate_input_size(data.len())?;
    if let Some(clip) = clip {
        rendering::validate_clip(clip)?;
    }
    match (normalize(filetype)?, clip) {
        ("pdf", None) => pdf::render(data, page, dpi, password),
        ("pdf", Some(clip)) => pdf::render_region(data, page, dpi, password, clip),
        ("docx", None) => docx::render(data, page, dpi, password),
        ("docx", Some(clip)) => docx::render_region(data, page, dpi, password, clip),
        ("pptx", None) => pptx::render(data, page, dpi),
        ("pptx", Some(clip)) => pptx::render_region(data, page, dpi, clip),
        ("epub", _) => Err(EngineError::Unsupported(
            "EPUB rendering is not supported".into(),
        )),
        _ => unreachable!(),
    }
}

/// Identify a document from its bytes, enforcing the input size limit.
pub fn detect(data: &[u8]) -> Result<String, EngineError> {
    limits::validate_input_size(data.len())?;
    if data.starts_with(b"%PDF-") {
        return Ok("pdf".into());
    }
    if wordcraft_docx::is_encrypted(data) {
        return Ok("docx".into());
    }
    if data.starts_with(b"PK\x03\x04") {
        if let Some(filetype) = ooxml::document_type(data)? {
            return Ok(filetype.into());
        }
        if epub::is_epub(data)? {
            return Ok("epub".into());
        }
    }
    Err(EngineError::Unsupported(
        "could not identify document format".into(),
    ))
}

/// Convert between supported formats using the upstream document engines.
pub fn convert(
    data: &[u8],
    source_format: &str,
    target_format: &str,
    password: Option<&str>,
) -> Result<Vec<u8>, EngineError> {
    limits::validate_input_size(data.len())?;
    let source = normalize(source_format)?;
    let target = normalize(target_format)?;
    match (source, target) {
        ("pdf", "docx") => conversion::pdf_to_docx(data, password),
        ("docx", "pdf") => conversion::docx_to_pdf(data, password),
        ("pptx", "pdf") => conversion::pptx_to_pdf(data),
        _ => Err(EngineError::Unsupported(format!(
            "conversion from {source} to {target}"
        ))),
    }
}

pub(crate) fn normalize(filetype: &str) -> Result<&'static str, EngineError> {
    match filetype
        .trim_start_matches('.')
        .to_ascii_lowercase()
        .as_str()
    {
        "pdf" => Ok("pdf"),
        "docx" | "word" => Ok("docx"),
        "pptx" | "powerpoint" => Ok("pptx"),
        "epub" | "ebook" => Ok("epub"),
        _ => Err(EngineError::Unsupported(format!("file type {filetype}"))),
    }
}
