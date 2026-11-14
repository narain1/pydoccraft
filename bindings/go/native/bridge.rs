//! C ABI shim, compiled against the root crate's locked, Python-free rlib.
use _core::{api, EngineError};
use std::{panic::{catch_unwind, AssertUnwindSafe}, slice, str};

#[repr(C)]
pub struct Buffer { ptr: *mut u8, len: usize }
#[repr(C)]
pub struct ResultBuffer { status: u32, data: Buffer }

fn result(status: u32, bytes: Vec<u8>) -> ResultBuffer {
    let mut bytes = bytes.into_boxed_slice();
    let data = Buffer { ptr: bytes.as_mut_ptr(), len: bytes.len() };
    std::mem::forget(bytes);
    ResultBuffer { status, data }
}
fn status(e: &EngineError) -> u32 {
    match e {
        EngineError::InvalidArgument(_) => 1, EngineError::Unsupported(_) => 2,
        EngineError::Malformed(_) => 3, EngineError::UnsafeArchive(_) => 4,
        EngineError::ResourceLimit(_) => 5, EngineError::Encrypted => 6,
        EngineError::Conversion(_) => 7, EngineError::Closed(_) => 10,
        EngineError::PageIndex(_) => 11, EngineError::Rendering(_) => 12,
    }
}
unsafe fn arguments<'a>(format: *const u8, format_len: usize, password: *const u8,
    password_len: usize, has_password: u8) -> Result<(&'a str, Option<&'a str>), EngineError> {
    if has_password > 1 { return Err(EngineError::InvalidArgument("invalid password flag".into())); }
    if format_len > 128 || password_len > 1_048_576 {
        return Err(EngineError::ResourceLimit("binding argument too large".into()));
    }
    Ok((text(format, format_len)?, if has_password == 1 { Some(text(password, password_len)?) } else { None }))
}
#[no_mangle]
pub unsafe extern "C" fn pydoccraft_extract(ptr: *const u8, len: usize,
    format: *const u8, format_len: usize, password: *const u8, password_len: usize,
    has_password: u8) -> ResultBuffer {
    run(|| {
        let (format, password) = arguments(format, format_len, password, password_len, has_password)?;
        let payload = api::extract(bytes(ptr, len)?, format, password)?;
        serde_json::to_vec(&payload).map_err(|e| EngineError::InvalidArgument(e.to_string()))
    })
}
#[no_mangle]
pub unsafe extern "C" fn pydoccraft_render(ptr: *const u8, len: usize,
    format: *const u8, format_len: usize, password: *const u8, password_len: usize,
    has_password: u8, page: usize, dpi: f64, has_clip: u8,
    x: f64, y: f64, width: f64, height: f64) -> ResultBuffer {
    run(|| {
        if has_clip > 1 { return Err(EngineError::InvalidArgument("invalid clip flag".into())); }
        let (format, password) = arguments(format, format_len, password, password_len, has_password)?;
        let clip = if has_clip == 1 { Some((x, y, width, height)) } else { None };
        api::render(bytes(ptr, len)?, format, page, dpi, password, clip)
    })
}
fn run(f: impl FnOnce() -> Result<Vec<u8>, EngineError>) -> ResultBuffer {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(bytes)) => result(0, bytes),
        Ok(Err(e)) => result(status(&e), e.to_string().into_bytes()),
        Err(_) => result(9, b"native operation panicked".to_vec()),
    }
}
// The caller must supply readable memory for nonempty buffers for the duration of the call.
unsafe fn bytes<'a>(ptr: *const u8, len: usize) -> Result<&'a [u8], EngineError> {
    if len > isize::MAX as usize || (ptr.is_null() && len != 0) {
        return Err(EngineError::InvalidArgument("invalid buffer".into()));
    }
    if len == 0 { return Ok(&[]); }
    Ok(slice::from_raw_parts(ptr, len))
}
unsafe fn text<'a>(ptr: *const u8, len: usize) -> Result<&'a str, EngineError> {
    str::from_utf8(bytes(ptr, len)?).map_err(|_| EngineError::InvalidArgument("invalid UTF-8".into()))
}
#[no_mangle]
pub unsafe extern "C" fn pydoccraft_detect(ptr: *const u8, len: usize) -> ResultBuffer {
    run(|| Ok(api::detect(bytes(ptr, len)?)?.into_bytes()))
}
#[no_mangle]
pub unsafe extern "C" fn pydoccraft_convert(ptr: *const u8, len: usize,
    source: *const u8, source_len: usize, target: *const u8, target_len: usize,
    password: *const u8, password_len: usize, has_password: u8) -> ResultBuffer {
    run(|| {
        if has_password > 1 { return Err(EngineError::InvalidArgument("invalid password flag".into())); }
        if source_len > 128 || target_len > 128 || password_len > 1_048_576 {
            return Err(EngineError::ResourceLimit("binding argument too large".into()));
        }
        let password = if has_password == 1 { Some(text(password, password_len)?) } else { None };
        let source = text(source, source_len)?;
        if matches!(source.trim_start_matches('.').to_ascii_lowercase().as_str(), "pptx" | "powerpoint") {
            return Err(EngineError::Unsupported("PPTX conversion is disabled in this preview".into()));
        }
        api::convert(bytes(ptr, len)?, source, text(target, target_len)?, password)
    })
}
/// Free exactly once, using the unmodified buffer returned by this library.
#[no_mangle]
pub unsafe extern "C" fn pydoccraft_free(buffer: Buffer) {
    if !buffer.ptr.is_null() {
        drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(buffer.ptr, buffer.len)));
    }
}

// Session pointers are private to the Go wrapper, which serializes use and free.
#[no_mangle]
pub unsafe extern "C" fn pydoccraft_session_open(ptr: *const u8, len: usize,
    format: *const u8, format_len: usize, password: *const u8, password_len: usize,
    has_password: u8, out: *mut *mut _core::session_api::DocumentSession) -> ResultBuffer {
    run(|| {
        if out.is_null() { return Err(EngineError::InvalidArgument("null session output".into())); }
        *out = std::ptr::null_mut();
        let (format, password) = arguments(format, format_len, password, password_len, has_password)?;
        let (session, payload) = _core::session_api::DocumentSession::open(bytes(ptr, len)?, format, password)?;
        let json = serde_json::to_vec(&payload).map_err(|e| EngineError::InvalidArgument(e.to_string()))?;
        *out = Box::into_raw(Box::new(session));
        Ok(json)
    })
}
#[no_mangle]
pub unsafe extern "C" fn pydoccraft_session_render(session: *mut _core::session_api::DocumentSession,
    page: usize, dpi: f64, has_clip: u8, x: f64, y: f64, width: f64, height: f64) -> ResultBuffer {
    run(|| {
        let session = session.as_ref().ok_or_else(|| EngineError::Closed("session closed".into()))?;
        if has_clip > 1 { return Err(EngineError::InvalidArgument("invalid clip flag".into())); }
        session.render(page, dpi, if has_clip == 1 { Some((x, y, width, height)) } else { None })
    })
}
#[no_mangle]
pub unsafe extern "C" fn pydoccraft_session_search(session: *mut _core::session_api::DocumentSession,
    page: usize, query: *const u8, query_len: usize) -> ResultBuffer {
    run(|| {
        let session = session.as_ref().ok_or_else(|| EngineError::Closed("session closed".into()))?;
        if query_len > 1_048_576 { return Err(EngineError::ResourceLimit("search query too large".into())); }
        let matches = session.search_for(page, text(query, query_len)?)?;
        serde_json::to_vec(&matches).map_err(|e| EngineError::InvalidArgument(e.to_string()))
    })
}
#[no_mangle]
pub unsafe extern "C" fn pydoccraft_session_close(session: *mut _core::session_api::DocumentSession) -> ResultBuffer {
    run(|| {
        if !session.is_null() {
            let session = Box::from_raw(session);
            session.close()?;
        }
        Ok(Vec::new())
    })
}

// Editable EPUB handles are owned and serialized by the Go wrapper.
use _core::author_epub::EditableEpub;
#[no_mangle]
pub unsafe extern "C" fn pydoccraft_epub_new(out: *mut *mut EditableEpub) -> ResultBuffer {
    run(|| {
        if out.is_null() { return Err(EngineError::InvalidArgument("null EPUB output".into())); }
        *out = Box::into_raw(Box::new(EditableEpub::new()));
        Ok(Vec::new())
    })
}
#[no_mangle]
pub unsafe extern "C" fn pydoccraft_epub_copy(book: *mut EditableEpub, out: *mut *mut EditableEpub) -> ResultBuffer {
    run(|| {
        if out.is_null() { return Err(EngineError::InvalidArgument("null EPUB output".into())); }
        *out = std::ptr::null_mut();
        let book = book.as_ref().ok_or_else(|| EngineError::Closed("EPUB closed".into()))?;
        *out = Box::into_raw(Box::new(book.copy()?));
        Ok(Vec::new())
    })
}
#[no_mangle]
pub unsafe extern "C" fn pydoccraft_epub_close(book: *mut EditableEpub) -> ResultBuffer {
    run(|| {
        if !book.is_null() { let mut book = Box::from_raw(book); book.close(); }
        Ok(Vec::new())
    })
}
fn epub_string<'a>(value: &'a serde_json::Value, key: &str, limit: usize) -> Result<&'a str, EngineError> {
    let s = value.get(key).and_then(|v| v.as_str()).ok_or_else(|| EngineError::InvalidArgument(format!("missing EPUB {key}")))?;
    if s.len() > limit { return Err(EngineError::ResourceLimit(format!("EPUB {key} too large"))); }
    Ok(s)
}
fn epub_safe(s: &str) -> Result<&str, EngineError> {
    if s.contains(['&', '<', '>']) { return Err(EngineError::Unsupported("EPUB metadata and chapter titles cannot contain &, < or > in this preview".into())); }
    Ok(s)
}
fn epub_index(value: &serde_json::Value) -> Result<usize, EngineError> {
    value.get("chapter").and_then(|v| v.as_u64()).and_then(|n| usize::try_from(n).ok())
        .ok_or_else(|| EngineError::InvalidArgument("invalid EPUB chapter".into()))
}
#[no_mangle]
pub unsafe extern "C" fn pydoccraft_epub_command(book: *mut EditableEpub, command: *const u8,
    command_len: usize, image: *const u8, image_len: usize) -> ResultBuffer {
    run(|| {
        let book = book.as_mut().ok_or_else(|| EngineError::Closed("EPUB closed".into()))?;
        // Bound JSON before parsing, including escaped metadata.
        if command_len > 8 * 1024 * 1024 || image_len > 16 * 1024 * 1024 {
            return Err(EngineError::ResourceLimit("EPUB binding input too large".into()));
        }
        let v: serde_json::Value = serde_json::from_slice(bytes(command, command_len)?)
            .map_err(|_| EngineError::InvalidArgument("invalid EPUB command".into()))?;
        let op = epub_string(&v, "op", 32)?;
        match op {
            "metadata" => {
                let authors = v.get("authors").and_then(|v| v.as_array()).ok_or_else(|| EngineError::InvalidArgument("invalid EPUB authors".into()))?;
                if authors.len() > 1000 { return Err(EngineError::ResourceLimit("EPUB authors exceed 1000".into())); }
                let mut names = Vec::with_capacity(authors.len());
                for author in authors {
                    let s = author.as_str().ok_or_else(|| EngineError::InvalidArgument("invalid EPUB author".into()))?;
                    if s.len() > 65_536 { return Err(EngineError::ResourceLimit("EPUB author too large".into())); }
                    names.push(epub_safe(s)?.to_owned());
                }
                book.set_metadata(epub_safe(epub_string(&v,"title",65_536)?)?.into(), epub_safe(epub_string(&v,"identifier",4096)?)?.into(), epub_safe(epub_string(&v,"language",128)?)?.into(), names)?;
                Ok(Vec::new())
            },
            "chapter" => Ok(book.add_chapter(epub_safe(epub_string(&v,"text",65_536)?)?.into())?.to_string().into_bytes()),
            "count" => Ok(book.chapter_count()?.to_string().into_bytes()),
            "heading" => {
                let level = v.get("level").and_then(|v| v.as_u64()).and_then(|n| u8::try_from(n).ok()).ok_or_else(|| EngineError::InvalidArgument("invalid heading level".into()))?;
                book.add_heading(epub_index(&v)?, epub_string(&v,"text",65_536)?.into(), level)?; Ok(Vec::new())
            },
            "paragraph" => { book.add_paragraph(epub_index(&v)?, epub_string(&v,"text",65_536)?.into())?; Ok(Vec::new()) },
            "link" => { book.add_link(epub_index(&v)?, epub_string(&v,"text",65_536)?.into(), epub_string(&v,"href",16_384)?.into())?; Ok(Vec::new()) },
            "image" => { book.add_image(epub_index(&v)?, bytes(image,image_len)?.to_vec(), epub_string(&v,"media_type",128)?.into(), epub_string(&v,"alt",65_536)?.into())?; Ok(Vec::new()) },
            "save" => book.to_bytes(),
            _ => Err(EngineError::InvalidArgument("unknown EPUB command".into())),
        }
    })
}

// PDF handles are opaque and must be serialized and consumed once by the caller.
use _core::author_pdf::EditablePdf;
fn pdf_error(e: _core::author_docx::error::AuthoringError) -> EngineError {
    use _core::author_docx::error::AuthoringError::*;
    match e { Value(s) | Key(s) => EngineError::InvalidArgument(s), Index(s) => EngineError::PageIndex(s), Runtime(s) => EngineError::Closed(s), Engine(e) => e }
}
#[no_mangle]
pub unsafe extern "C" fn pydoccraft_pdf_new(width: f64, height: f64, out: *mut *mut EditablePdf) -> ResultBuffer {
    run(|| { if out.is_null() { return Err(EngineError::InvalidArgument("null PDF output".into())); } *out = std::ptr::null_mut(); *out = Box::into_raw(Box::new(EditablePdf::new(width,height).map_err(pdf_error)?)); Ok(Vec::new()) })
}
#[no_mangle]
pub unsafe extern "C" fn pydoccraft_pdf_open(data: *const u8, len: usize, password: *const u8, password_len: usize, has: u8, out: *mut *mut EditablePdf) -> ResultBuffer {
    run(|| { if out.is_null() { return Err(EngineError::InvalidArgument("null PDF output".into())); } *out = std::ptr::null_mut(); if has > 1 { return Err(EngineError::InvalidArgument("invalid password flag".into())); } if password_len > 1_048_576 { return Err(EngineError::ResourceLimit("password too large".into())); } let p = if has == 1 { Some(text(password,password_len)?) } else { None }; *out = Box::into_raw(Box::new(EditablePdf::from_bytes(bytes(data,len)?,p).map_err(pdf_error)?)); Ok(Vec::new()) })
}
#[no_mangle]
pub unsafe extern "C" fn pydoccraft_pdf_copy(pdf: *mut EditablePdf, out: *mut *mut EditablePdf) -> ResultBuffer {
    run(|| { if out.is_null() { return Err(EngineError::InvalidArgument("null PDF output".into())); } *out = std::ptr::null_mut(); let pdf = pdf.as_ref().ok_or_else(|| EngineError::Closed("PDF closed".into()))?; *out = Box::into_raw(Box::new(pdf.copy().map_err(pdf_error)?)); Ok(Vec::new()) })
}
#[no_mangle]
pub unsafe extern "C" fn pydoccraft_pdf_close(pdf: *mut EditablePdf) -> ResultBuffer {
    run(|| { if !pdf.is_null() { let mut pdf = Box::from_raw(pdf); pdf.close(); } Ok(Vec::new()) })
}
#[no_mangle]
pub unsafe extern "C" fn pydoccraft_pdf_command(pdf: *mut EditablePdf, command: *const u8, len: usize) -> ResultBuffer {
    run(|| {
        let pdf = pdf.as_mut().ok_or_else(|| EngineError::Closed("PDF closed".into()))?;
        if len > 2_000_000 { return Err(EngineError::ResourceLimit("PDF command too large".into())); }
        let v: serde_json::Value = serde_json::from_slice(bytes(command,len)?).map_err(|_| EngineError::InvalidArgument("invalid PDF command".into()))?;
        let number = |key: &str| v.get(key).and_then(|x| x.as_f64()).ok_or_else(|| EngineError::InvalidArgument(format!("invalid PDF {key}")));
        let page = || v.get("page").and_then(|x| x.as_i64()).and_then(|x| isize::try_from(x).ok()).ok_or_else(|| EngineError::InvalidArgument("invalid PDF page".into()));
        match v.get("op").and_then(|x| x.as_str()) {
            Some("count") => Ok(pdf.page_count().map_err(pdf_error)?.to_string().into_bytes()),
            Some("size") => serde_json::to_vec(&pdf.page_size(page()?).map_err(pdf_error)?).map_err(|e| EngineError::InvalidArgument(e.to_string())),
            Some("add_page") => Ok(pdf.add_page(number("width")?,number("height")?).map_err(pdf_error)?.to_string().into_bytes()),
            Some("delete_page") => { pdf.delete_page(page()?).map_err(pdf_error)?; Ok(Vec::new()) },
            Some("rotate") => { let degrees = v.get("degrees").and_then(|x| x.as_i64()).ok_or_else(|| EngineError::InvalidArgument("invalid rotation".into()))?; pdf.rotate_page(page()?, degrees).map_err(pdf_error)?; Ok(Vec::new()) },
            Some("text") => { let s = v.get("text").and_then(|x| x.as_str()).ok_or_else(|| EngineError::InvalidArgument("invalid PDF text".into()))?; if s.len() > 1_000_000 { return Err(EngineError::ResourceLimit("PDF text too large".into())); } pdf.add_text(page()?,s,number("x")?,number("y")?,number("size")?,(0,0,0),false).map_err(pdf_error)?; Ok(Vec::new()) },
            Some("save") => pdf.to_bytes().map_err(pdf_error),
            _ => Err(EngineError::Unsupported("PDF operation unsupported by Go binding".into())),
        }
    })
}
