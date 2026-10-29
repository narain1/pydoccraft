//! Rust-facing document operations. This API is an initial migration slice;
//! authoring and document sessions are not yet independent of Python.

use crate::{conversion, epub, limits, ooxml, EngineError};

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
