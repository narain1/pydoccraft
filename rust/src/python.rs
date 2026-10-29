use crate::{
    api, author_docx, author_epub, author_pdf, author_pptx_python, docx, limits, pdf, pptx,
    rendering, EngineError,
};
use api::normalize;
use pyo3::create_exception;
use pyo3::exceptions::PyException;
use pyo3::prelude::*;

create_exception!(_core, DocExNativeError, PyException);

#[pyclass(module = "doc_ex._core", name = "EditableEpub")]
struct NativeEditableEpub {
    inner: author_epub::EditableEpub,
}

#[pymethods]
impl NativeEditableEpub {
    #[new]
    fn new() -> Self {
        Self {
            inner: author_epub::EditableEpub::new(),
        }
    }

    fn copy(&self) -> Result<Self, EngineError> {
        Ok(Self {
            inner: self.inner.copy()?,
        })
    }

    fn close(&mut self) {
        self.inner.close();
    }

    fn set_metadata(
        &mut self,
        title: String,
        identifier: String,
        language: String,
        authors: Vec<String>,
    ) -> Result<(), EngineError> {
        self.inner
            .set_metadata(title, identifier, language, authors)
    }

    fn add_chapter(&mut self, title: String) -> Result<usize, EngineError> {
        self.inner.add_chapter(title)
    }

    fn add_heading(&mut self, chapter: usize, text: String, level: u8) -> Result<(), EngineError> {
        self.inner.add_heading(chapter, text, level)
    }

    fn add_paragraph(&mut self, chapter: usize, text: String) -> Result<(), EngineError> {
        self.inner.add_paragraph(chapter, text)
    }

    fn add_link(&mut self, chapter: usize, text: String, href: String) -> Result<(), EngineError> {
        self.inner.add_link(chapter, text, href)
    }

    fn add_image(
        &mut self,
        chapter: usize,
        data: Vec<u8>,
        media_type: String,
        alt: String,
    ) -> Result<(), EngineError> {
        self.inner.add_image(chapter, data, media_type, alt)
    }

    fn chapter_count(&self) -> Result<usize, EngineError> {
        self.inner.chapter_count()
    }

    fn to_bytes(&self) -> Result<Vec<u8>, EngineError> {
        self.inner.to_bytes()
    }
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
    inner: pdf::PdfSession,
}

#[pymethods]
impl NativePdfSession {
    #[pyo3(signature = (page, dpi=144.0, clip=None))]
    fn render(&self, page: usize, dpi: f64, clip: Option<rendering::Clip>) -> PyResult<Vec<u8>> {
        if let Some(clip) = clip {
            rendering::validate_clip(clip)?;
        }
        self.inner.render(page, dpi, clip).map_err(Into::into)
    }

    fn search_for(&self, page: usize, text: &str) -> PyResult<Vec<(f32, f32, f32, f32)>> {
        self.inner
            .search_for(page, text)
            .map(|rects| {
                rects
                    .into_iter()
                    .map(|[x0, y0, x1, y1]| (x0, y0, x1, y1))
                    .collect()
            })
            .map_err(Into::into)
    }

    fn close(&self) -> PyResult<()> {
        self.inner.close().map_err(Into::into)
    }

    fn _stats(&self) -> PyResult<(u64, u64, u64)> {
        let stats = self.inner.stats()?;
        Ok((
            stats.parser_builds,
            stats.render_requests,
            stats.rasterizations,
        ))
    }
}

#[pyfunction]
#[pyo3(signature = (data, password=None))]
fn _open_pdf(data: &[u8], password: Option<&str>) -> PyResult<(NativePdfSession, String)> {
    limits::validate_input_size(data.len())?;
    let (session, payload) = pdf::PdfSession::open(data, password)?;
    let encoded = limits::bounded_json(&payload, limits::MAX_PAYLOAD_SIZE)?;
    Ok((NativePdfSession { inner: session }, encoded))
}

enum OfficeSession {
    Docx(Box<docx::DocxSession>),
    Pptx(Box<pptx::PptxSession>),
}

#[pyclass(module = "doc_ex._core", name = "_OfficeSession")]
struct NativeOfficeSession {
    inner: OfficeSession,
}

#[pymethods]
impl NativeOfficeSession {
    #[pyo3(signature = (page, dpi=144.0, clip=None))]
    fn render(&self, page: usize, dpi: f64, clip: Option<rendering::Clip>) -> PyResult<Vec<u8>> {
        if let Some(clip) = clip {
            rendering::validate_clip(clip)?;
        }
        match &self.inner {
            OfficeSession::Docx(session) => session.render(page, dpi, clip),
            OfficeSession::Pptx(session) => session.render(page, dpi, clip),
        }
        .map_err(Into::into)
    }

    fn close(&self) -> PyResult<()> {
        match &self.inner {
            OfficeSession::Docx(session) => session.close(),
            OfficeSession::Pptx(session) => session.close(),
        }
        .map_err(Into::into)
    }

    fn _stats(&self) -> PyResult<(u64, u64, u64)> {
        match &self.inner {
            OfficeSession::Docx(session) => session.stats(),
            OfficeSession::Pptx(session) => session.stats(),
        }
        .map_err(Into::into)
    }
}

#[pyfunction]
#[pyo3(signature = (data, filetype, password=None))]
fn _open_office(
    data: &[u8],
    filetype: &str,
    password: Option<&str>,
) -> PyResult<(NativeOfficeSession, String)> {
    limits::validate_input_size(data.len())?;
    let (inner, payload) = match normalize(filetype)? {
        "docx" => {
            let (session, payload) = docx::DocxSession::open(data, password)?;
            (OfficeSession::Docx(Box::new(session)), payload)
        }
        "pptx" => {
            let (session, payload) = pptx::PptxSession::open(data)?;
            (OfficeSession::Pptx(Box::new(session)), payload)
        }
        _ => {
            return Err(
                EngineError::Unsupported("Office session requires DOCX or PPTX".into()).into(),
            )
        }
    };
    let encoded = limits::bounded_json(&payload, limits::MAX_PAYLOAD_SIZE)?;
    Ok((NativeOfficeSession { inner }, encoded))
}

#[pyfunction]
#[pyo3(signature = (data, filetype, password=None))]
fn extract(data: &[u8], filetype: &str, password: Option<&str>) -> PyResult<String> {
    let payload = api::extract(data, filetype, password)?;
    limits::bounded_json(&payload, limits::MAX_PAYLOAD_SIZE).map_err(Into::into)
}

#[pyfunction]
#[pyo3(signature = (data, pages=None, password=None))]
fn extract_text(
    data: &[u8],
    pages: Option<Bound<'_, PyAny>>,
    password: Option<&str>,
) -> PyResult<String> {
    limits::validate_input_size(data.len())?;
    let selected = pages
        .map(|pages| -> PyResult<Vec<isize>> {
            let mut selected = Vec::new();
            for value in pages.try_iter()? {
                if selected.len() == pdf::MAX_SELECTED_PAGES {
                    return Err(EngineError::ResourceLimit(
                        "selected page count exceeds 10000".into(),
                    )
                    .into());
                }
                let value = value?;
                let index = value.extract::<isize>().map_err(|error| {
                    if error.is_instance_of::<pyo3::exceptions::PyOverflowError>(value.py()) {
                        pyo3::exceptions::PyIndexError::new_err("page index is out of range")
                    } else {
                        error
                    }
                })?;
                selected.push(index);
            }
            Ok(selected)
        })
        .transpose()?;
    pdf::extract_text(data, password, selected.as_deref()).map_err(Into::into)
}

#[pyfunction]
#[pyo3(signature = (data, source_format, target_format, password=None))]
fn convert(
    data: &[u8],
    source_format: &str,
    target_format: &str,
    password: Option<&str>,
) -> PyResult<Vec<u8>> {
    api::convert(data, source_format, target_format, password).map_err(Into::into)
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
    api::render(data, filetype, page, dpi, password, clip).map_err(Into::into)
}

#[pyfunction]
fn detect(data: &[u8]) -> PyResult<String> {
    api::detect(data).map_err(Into::into)
}

#[pymodule]
fn _core(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<author_docx::EditableDocx>()?;
    module.add_class::<NativeEditableEpub>()?;
    module.add_class::<author_pdf::EditablePdf>()?;
    module.add_class::<author_pptx_python::NativeEditablePptx>()?;
    module.add_class::<NativePdfSession>()?;
    module.add_class::<NativeOfficeSession>()?;
    module.add(
        "DocExNativeError",
        module.py().get_type::<DocExNativeError>(),
    )?;
    module.add_function(wrap_pyfunction!(_open_pdf, module)?)?;
    module.add_function(wrap_pyfunction!(_open_office, module)?)?;
    module.add_function(wrap_pyfunction!(extract, module)?)?;
    module.add_function(wrap_pyfunction!(extract_text, module)?)?;
    module.add_function(wrap_pyfunction!(convert, module)?)?;
    module.add_function(wrap_pyfunction!(render, module)?)?;
    module.add_function(wrap_pyfunction!(detect, module)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn detects_pdf_signature() {
        assert_eq!(crate::api::detect(b"%PDF-1.4").unwrap(), "pdf");
    }
}
