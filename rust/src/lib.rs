mod author_docx;
mod author_pdf;
mod author_pptx;
#[path = "convert.rs"]
mod conversion;
mod diagnostics;
mod docx;
mod limits;
mod model;
mod ooxml;
mod pdf;
mod pptx;
mod rendering;

use pyo3::create_exception;
use pyo3::exceptions::PyException;
use pyo3::prelude::*;
use thiserror::Error;

create_exception!(_core, DocExNativeError, PyException);

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("invalid_argument:{0}")]
    InvalidArgument(String),
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
    #[error("closed:{0}")]
    Closed(String),
}

impl From<EngineError> for PyErr {
    fn from(error: EngineError) -> Self {
        match error {
            EngineError::InvalidArgument(message) => {
                pyo3::exceptions::PyValueError::new_err(message)
            }
            other => DocExNativeError::new_err(other.to_string()),
        }
    }
}

#[pyclass(module = "doc_ex._core", name = "_PdfSession")]
struct NativePdfSession {
    inner: Option<pdf::PdfSession>,
}

#[pymethods]
impl NativePdfSession {
    #[pyo3(signature = (page, dpi=144.0))]
    fn render(&self, page: usize, dpi: f64) -> PyResult<Vec<u8>> {
        self.inner
            .as_ref()
            .ok_or_else(|| EngineError::Closed("PDF render session is closed".into()))?
            .render(page, dpi)
            .map_err(Into::into)
    }

    fn _stats(&self) -> PyResult<(u64, u64, u64)> {
        let stats = self
            .inner
            .as_ref()
            .ok_or_else(|| EngineError::Closed("PDF render session is closed".into()))?
            .stats()?;
        Ok((
            stats.parser_builds,
            stats.render_requests,
            stats.rasterizations,
        ))
    }

    fn close(&mut self) {
        self.inner = None;
    }
}

#[pyfunction]
#[pyo3(signature = (data, password=None))]
fn _open_pdf(data: &[u8], password: Option<&str>) -> PyResult<(NativePdfSession, String)> {
    limits::validate_input_size(data.len())?;
    let (session, payload) = pdf::PdfSession::open(data, password)?;
    let encoded = limits::bounded_json(&payload, limits::MAX_PAYLOAD_SIZE)?;
    Ok((
        NativePdfSession {
            inner: Some(session),
        },
        encoded,
    ))
}

#[pyfunction]
#[pyo3(signature = (data, filetype, password=None))]
fn extract(data: &[u8], filetype: &str, password: Option<&str>) -> PyResult<String> {
    limits::validate_input_size(data.len())?;
    let payload = match normalize(filetype)? {
        "pdf" => pdf::extract(data, password),
        "docx" => docx::extract(data, password),
        "pptx" => pptx::extract(data),
        _ => unreachable!(),
    }?;
    limits::bounded_json(&payload, limits::MAX_PAYLOAD_SIZE).map_err(Into::into)
}

#[pyfunction]
#[pyo3(signature = (data, source_format, target_format, password=None))]
fn convert(
    data: &[u8],
    source_format: &str,
    target_format: &str,
    password: Option<&str>,
) -> PyResult<Vec<u8>> {
    limits::validate_input_size(data.len())?;
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
#[pyo3(signature = (data, filetype, page, dpi=144.0, password=None, clip=None))]
fn render(
    data: &[u8],
    filetype: &str,
    page: usize,
    dpi: f64,
    password: Option<&str>,
    clip: Option<rendering::Clip>,
) -> PyResult<Vec<u8>> {
    limits::validate_input_size(data.len())?;
    if let Some(clip) = clip {
        rendering::validate_clip(clip)?;
    }
    let output = match (normalize(filetype)?, clip) {
        ("pdf", None) => pdf::render(data, page, dpi, password),
        ("pdf", Some(clip)) => pdf::render_region(data, page, dpi, password, clip),
        ("docx", None) => docx::render(data, page, dpi, password),
        ("docx", Some(clip)) => docx::render_region(data, page, dpi, password, clip),
        ("pptx", None) => pptx::render(data, page, dpi),
        ("pptx", Some(clip)) => pptx::render_region(data, page, dpi, clip),
        _ => unreachable!(),
    };
    output.map_err(Into::into)
}

#[pyfunction]
fn detect(data: &[u8]) -> PyResult<String> {
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
    module.add_class::<author_docx::EditableDocx>()?;
    module.add_class::<author_pdf::EditablePdf>()?;
    module.add_class::<author_pptx::EditablePptx>()?;
    module.add_class::<NativePdfSession>()?;
    module.add(
        "DocExNativeError",
        module.py().get_type::<DocExNativeError>(),
    )?;
    module.add_function(wrap_pyfunction!(_open_pdf, module)?)?;
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
