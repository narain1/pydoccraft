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
