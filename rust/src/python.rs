use crate::{
    api, author_docx, author_epub, author_pdf, author_pptx_python, docx, limits, pdf, pptx,
    rendering, EngineError,
};
use api::normalize;
use pyo3::create_exception;
use pyo3::exceptions::PyException;
use pyo3::prelude::*;
use pyo3::types::{PyList, PyString};

create_exception!(_core, DocExNativeError, PyException);

#[pyfunction]
fn _format_join(
    parts: &Bound<'_, PyList>,
    separator: &Bound<'_, PyAny>,
    skip_empty: bool,
) -> PyResult<Option<String>> {
    // Preflight Python objects before PyO3 copies or materializes UTF-8 strings.
    // Returning None selects the original Python implementation, not a new
    // public limit on page text or Markdown.
    const MAX_PARTS: usize = 100_000;
    const MAX_CHARS: usize = 1_000_000;
    if !parts.is_exact_instance_of::<PyList>()
        || parts.len() > MAX_PARTS
        || !separator.is_exact_instance_of::<PyString>()
    {
        return Ok(None);
    }
    let separator_len = separator.len()?;
    if separator_len > MAX_CHARS {
        return Ok(None);
    }
    let Some(mut chars) = separator_len.checked_mul(parts.len().saturating_sub(1)) else {
        return Ok(None);
    };
    if chars > MAX_CHARS {
        return Ok(None);
    }
    for part in parts.iter() {
        if !part.is_exact_instance_of::<PyString>() {
            return Ok(None);
        }
        let Some(total) = chars.checked_add(part.len()?) else {
            return Ok(None);
        };
        chars = total;
        if chars > MAX_CHARS {
            return Ok(None);
        }
    }
    let separator: &str = separator.extract()?;
    let strings: Vec<String> = parts.extract()?;
    Ok(Some(crate::formatter::join_parts(
        strings, separator, skip_empty,
    )))
}

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

    #[pyo3(signature = (date=None))]
    fn set_publication_date(&mut self, date: Option<String>) -> Result<(), EngineError> {
        self.inner.set_publication_date(date)
    }

    fn add_list(
        &mut self,
        chapter: usize,
        items: Vec<String>,
        ordered: bool,
    ) -> Result<(), EngineError> {
        self.inner.add_list(chapter, items, ordered)
    }

    fn add_table(
        &mut self,
        chapter: usize,
        rows: Vec<Vec<String>>,
        header: bool,
    ) -> Result<(), EngineError> {
        self.inner.add_table(chapter, rows, header)
    }

    fn add_preformatted(&mut self, chapter: usize, text: String) -> Result<(), EngineError> {
        self.inner.add_preformatted(chapter, text)
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
#[pyo3(signature = (data, page, dpi, landscape, clip=None))]
fn _render_epub_spread(
    data: &[u8],
    page: usize,
    dpi: f64,
    landscape: bool,
    clip: Option<rendering::Clip>,
) -> Result<Vec<u8>, EngineError> {
    crate::epub::render_spread(data, page, dpi, landscape, clip)
}

#[pyfunction]
#[pyo3(signature = (data, dpi, pages=None, searchable=false))]
fn _export_epub_fixed_pdf(
    data: &[u8],
    dpi: f64,
    pages: Option<Vec<usize>>,
    searchable: bool,
) -> Result<Vec<u8>, EngineError> {
    crate::epub::export_fixed_pdf(data, dpi, pages.as_deref(), searchable)
}

#[pyfunction]
#[pyo3(signature = (data, title=None, description=None))]
fn _edit_epub_metadata(
    data: &[u8],
    title: Option<&str>,
    description: Option<&str>,
) -> Result<Vec<u8>, EngineError> {
    crate::epub::update_metadata(data, title, description)
}

#[pyfunction]
#[pyo3(signature = (data, path, offset=0, length=None))]
fn _epub_resource(
    data: &[u8],
    path: &str,
    offset: u64,
    length: Option<usize>,
) -> Result<Vec<u8>, EngineError> {
    crate::epub::read_resource(data, path, offset, length)
}

#[pyclass(module = "doc_ex._core", name = "_EpubAudio")]
struct NativeEpubAudio(crate::epub::audio::AudioSource);

#[pymethods]
impl NativeEpubAudio {
    #[pyo3(signature = (begin, end))]
    fn clip(
        &mut self,
        py: Python<'_>,
        begin: u64,
        end: Option<u64>,
    ) -> Result<PcmClip, EngineError> {
        py.allow_threads(|| self.0.clip(begin, end)).map(pcm_clip)
    }

    #[pyo3(signature = (begin, end, clamp_to_duration=false))]
    fn start_stream(
        &mut self,
        py: Python<'_>,
        begin: u64,
        end: Option<u64>,
        clamp_to_duration: bool,
    ) -> Result<(), EngineError> {
        py.allow_threads(|| self.0.start_stream(begin, end, clamp_to_duration))
    }

    fn next_chunk(
        &mut self,
        py: Python<'_>,
        frames: usize,
    ) -> Result<Option<PcmClip>, EngineError> {
        py.allow_threads(|| self.0.next_chunk(frames))
            .map(|chunk| chunk.map(pcm_clip))
    }
}

type PcmClip = (Vec<u8>, u32, u16, usize);

fn pcm_clip(clip: crate::epub::audio::AudioClip) -> PcmClip {
    (clip.wave, clip.sample_rate, clip.channels, clip.frames)
}

#[pyfunction]
#[pyo3(signature = (data, path, media, opus_track=None, synchronize_video=false, audio_track=None, fragment_tracks=None))]
#[allow(clippy::too_many_arguments)]
fn _open_epub_audio(
    py: Python<'_>,
    data: &[u8],
    path: &str,
    media: &str,
    opus_track: Option<u32>,
    synchronize_video: bool,
    audio_track: Option<u32>,
    fragment_tracks: Option<Vec<String>>,
) -> Result<NativeEpubAudio, EngineError> {
    py.allow_threads(|| {
        crate::epub::audio::AudioSource::open(
            data,
            path,
            media,
            opus_track.map(|index| index as usize),
            synchronize_video,
            audio_track.map(|index| index as usize),
            fragment_tracks.as_deref().unwrap_or_default(),
        )
    })
    .map(NativeEpubAudio)
}

#[pyclass(module = "doc_ex._core", name = "_EpubVideo")]
struct NativeEpubVideo(crate::epub::video::VideoSource);

type RgbFrame = (Vec<u8>, u32, u32, (u32, u32), u64, u64);

#[pymethods]
impl NativeEpubVideo {
    fn clip_spatial(&mut self, percent: bool, coordinates: [u64; 4]) -> Result<(), EngineError> {
        self.0.clip_spatial(percent, coordinates)
    }

    fn next_frame(&mut self, py: Python<'_>) -> Result<Option<RgbFrame>, EngineError> {
        py.allow_threads(|| self.0.next()).map(|frame| {
            frame.map(|frame| {
                (
                    frame.rgb,
                    frame.width,
                    frame.height,
                    frame.aspect,
                    frame.start_us,
                    frame.duration_us,
                )
            })
        })
    }
}

#[pyfunction]
#[pyo3(signature = (data, path, media, start, end, video_track=None, fragment_tracks=None))]
#[allow(clippy::too_many_arguments)]
fn _open_epub_video(
    py: Python<'_>,
    data: &[u8],
    path: &str,
    media: &str,
    start: u64,
    end: Option<u64>,
    video_track: Option<u32>,
    fragment_tracks: Option<Vec<String>>,
) -> Result<NativeEpubVideo, EngineError> {
    py.allow_threads(|| {
        crate::epub::video::VideoSource::open(
            data,
            path,
            media,
            start,
            end,
            video_track.map(|index| index as usize),
            fragment_tracks.as_deref().unwrap_or_default(),
        )
    })
    .map(NativeEpubVideo)
}

#[pyfunction]
fn _epub_webm_chapter(
    py: Python<'_>,
    data: &[u8],
    path: &str,
    id: &str,
) -> Result<(u64, Option<u64>), EngineError> {
    py.allow_threads(|| crate::epub::webm::chapter_interval(data, path, id))
}

type Rgba = (u8, u8, u8, u8);
type TimedTextStyle = (
    bool,
    &'static str,
    bool,
    bool,
    bool,
    Option<Rgba>,
    Option<Rgba>,
    Vec<String>,
    Option<String>,
    Option<String>,
);
type TimedTextRun = (String, TimedTextStyle, Option<u64>);

fn timed_text_run(run: crate::epub::text_tracks::TextRun) -> TimedTextRun {
    let style = std::sync::Arc::unwrap_or_clone(run.style);
    let rgba = |[r, g, b, a]: [u8; 4]| (r, g, b, a);
    (
        run.text,
        (
            style.bold,
            style.font_style.as_str(),
            style.underline,
            style.line_through,
            style.overline,
            style.color.map(rgba),
            style.background_color.map(rgba),
            style.classes,
            style.voice,
            style.language,
        ),
        run.timestamp,
    )
}

type TimedTextCue = (
    u64,
    Option<u64>,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Vec<TimedTextRun>,
    Option<Rgba>,
);

#[pyfunction]
#[pyo3(signature = (data, path, media, start, end, language=None))]
fn _epub_text_track(
    py: Python<'_>,
    data: &[u8],
    path: &str,
    media: &str,
    start: u64,
    end: Option<u64>,
    language: Option<&str>,
) -> Result<Vec<TimedTextCue>, EngineError> {
    py.allow_threads(|| crate::epub::text_tracks::read(data, path, media, start, end, language))
        .map(|cues| {
            cues.into_iter()
                .map(|cue| {
                    (
                        cue.start,
                        cue.end,
                        cue.text,
                        cue.id,
                        cue.settings,
                        cue.region,
                        cue.markup,
                        cue.runs.into_iter().map(timed_text_run).collect(),
                        cue.background_color
                            .map(|[r, g, b, a]: [u8; 4]| (r, g, b, a)),
                    )
                })
                .collect()
        })
}

#[pyfunction]
#[pyo3(signature = (data, path, fragment))]
fn _epub_narration_text(
    data: &[u8],
    path: &str,
    fragment: Option<&str>,
) -> Result<String, EngineError> {
    crate::epub::audio::narration_text(data, path, fragment)
}

#[pyfunction]
fn _edit_epub_chapters(
    data: &[u8],
    replacements: Vec<(i64, String)>,
) -> Result<Vec<u8>, EngineError> {
    crate::epub::update_chapters(data, &replacements)
}

#[pyfunction]
#[pyo3(signature = (data, order, resources, modified, removed=Vec::new(), overlays=Vec::new(), chapters=Vec::new(), spine_properties=Vec::new()))]
#[allow(
    clippy::too_many_arguments,
    reason = "flat Python binding for EPUB structure edits"
)]
fn _edit_epub_structure(
    data: &[u8],
    order: Option<Vec<crate::epub::SpineEdit>>,
    resources: Vec<crate::epub::ResourceEdit>,
    modified: &str,
    removed: Vec<String>,
    overlays: Vec<crate::epub::OverlayEdit>,
    chapters: Vec<(i64, String)>,
    spine_properties: Vec<crate::epub::SpinePropertyEdit>,
) -> Result<Vec<u8>, EngineError> {
    crate::epub::update_structure(
        data,
        crate::epub::StructureEdits {
            order: order.as_deref(),
            resources: &resources,
            modified,
            removed: &removed,
            overlays: &overlays,
            chapters: &chapters,
            spine_properties: &spine_properties,
        },
    )
}

#[pyfunction]
fn detect(data: &[u8]) -> PyResult<String> {
    api::detect(data).map_err(Into::into)
}

#[pymodule]
fn _core(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(_format_join, module)?)?;
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
    module.add_function(wrap_pyfunction!(_render_epub_spread, module)?)?;
    module.add_function(wrap_pyfunction!(_export_epub_fixed_pdf, module)?)?;
    module.add_function(wrap_pyfunction!(_edit_epub_metadata, module)?)?;
    module.add_function(wrap_pyfunction!(_edit_epub_chapters, module)?)?;
    module.add_function(wrap_pyfunction!(_edit_epub_structure, module)?)?;
    module.add_function(wrap_pyfunction!(_epub_resource, module)?)?;
    module.add_function(wrap_pyfunction!(_open_epub_audio, module)?)?;
    module.add_function(wrap_pyfunction!(_open_epub_video, module)?)?;
    module.add_function(wrap_pyfunction!(_epub_webm_chapter, module)?)?;
    module.add_function(wrap_pyfunction!(_epub_text_track, module)?)?;
    module.add_function(wrap_pyfunction!(_epub_narration_text, module)?)?;
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
