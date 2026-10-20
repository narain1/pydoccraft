#[path = "convert.rs"]
mod conversion;
mod docx;
mod limits;
mod model;
mod ooxml;
mod pdf;
mod pptx;

use pyo3::create_exception;
use pyo3::exceptions::PyException;
use pyo3::prelude::*;
use thiserror::Error;

create_exception!(_core, DocExNativeError, PyException);

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("unsupported:{0}")]
    Unsupported(String),
    #[error("malformed:{0}")]
    Malformed(String),
    #[error("unsafe_archive:{0}")]
    UnsafeArchive(String),
    #[error("resource_limit:{0}")]
    ResourceLimit(String),
    #[error("encrypted:document is password-protected or the supplied password is incorrect")]
    Encrypted,
    #[error("conversion_failure:{0}")]
    Conversion(String),
}

impl From<EngineError> for PyErr {
    fn from(error: EngineError) -> Self {
        DocExNativeError::new_err(error.to_string())
    }
}

#[pyfunction]
#[pyo3(signature = (data, filetype, password=None))]
fn extract(data: &[u8], filetype: &str, password: Option<&str>) -> PyResult<String> {
    let payload = match normalize(filetype)? {
        "pdf" => pdf::extract(data, password),
        "docx" => docx::extract(data, password),
        "pptx" => pptx::extract(data),
        _ => unreachable!(),
    }?;
    serde_json::to_string(&payload).map_err(|error| {
        EngineError::Malformed(format!("payload serialization failed: {error}")).into()
    })
}

#[pyfunction]
#[pyo3(signature = (data, source_format, target_format, password=None))]
fn convert(
    data: &[u8],
    source_format: &str,
    target_format: &str,
    password: Option<&str>,
) -> PyResult<Vec<u8>> {
    let source = normalize(source_format)?;
    let target = normalize(target_format)?;
    match (source, target) {
        ("pdf", "docx") => conversion::pdf_to_docx(data, password).map_err(Into::into),
        ("docx", "pdf") => conversion::docx_to_pdf(data, password).map_err(Into::into),
        ("pptx", "pdf") => conversion::pptx_to_pdf(data).map_err(Into::into),
        _ => Err(EngineError::Unsupported(format!("conversion from {source} to {target}")).into()),
    }
}

#[pyfunction]
#[pyo3(signature = (data, filetype, page, dpi=144.0, password=None))]
fn render(
    data: &[u8],
    filetype: &str,
    page: usize,
    dpi: f64,
    password: Option<&str>,
) -> PyResult<Vec<u8>> {
    match normalize(filetype)? {
        "pdf" => pdf::render(data, page, dpi, password).map_err(Into::into),
        "docx" => docx::render(data, page, dpi, password).map_err(Into::into),
        "pptx" => pptx::render(data, page, dpi).map_err(Into::into),
        _ => unreachable!(),
    }
}

#[pyfunction]
fn detect(data: &[u8]) -> PyResult<String> {
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
    }
    Err(EngineError::Unsupported("could not identify document format".into()).into())
}

fn normalize(filetype: &str) -> Result<&'static str, EngineError> {
    match filetype
        .trim_start_matches('.')
        .to_ascii_lowercase()
        .as_str()
    {
        "pdf" => Ok("pdf"),
        "docx" | "word" => Ok("docx"),
        "pptx" | "powerpoint" => Ok("pptx"),
        _ => Err(EngineError::Unsupported(format!("file type {filetype}"))),
    }
}

#[pymodule]
fn _core(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add(
        "DocExNativeError",
        module.py().get_type::<DocExNativeError>(),
    )?;
    module.add_function(wrap_pyfunction!(extract, module)?)?;
    module.add_function(wrap_pyfunction!(convert, module)?)?;
    module.add_function(wrap_pyfunction!(render, module)?)?;
    module.add_function(wrap_pyfunction!(detect, module)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn detects_pdf_signature() {
        assert_eq!(super::detect(b"%PDF-1.4").unwrap(), "pdf");
    }
}
