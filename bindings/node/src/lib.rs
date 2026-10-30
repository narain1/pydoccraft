//! Minimal Node-API boundary. No Node package or build-time binding generator is needed.
use std::ffi::{c_char, c_void, CString};
use std::ptr;
use std::sync::Mutex;

type Env = *mut c_void;
type Value = *mut c_void;
type Info = *mut c_void;
type Callback = unsafe extern "C" fn(Env, Info) -> Value;
#[repr(C)]
struct TypeTag { lower: u64, upper: u64 }
static SESSION_TAG: TypeTag = TypeTag { lower: 0x93662d6bec3d4f0b, upper: 0xaca15b880f37193b };
static EPUB_TAG: TypeTag = TypeTag { lower: 0x2c789bd485be48c3, upper: 0xbb79402c65ebd965 };
static PDF_TAG: TypeTag = TypeTag { lower: 0xd7282eb140094376, upper: 0x859ba71b5b03281c };
struct SessionHandle {
    session: _core::session_api::DocumentSession,
    snapshot: Mutex<Option<String>>,
}

extern "C" {
    fn napi_get_cb_info(env: Env, info: Info, argc: *mut usize, argv: *mut Value, this: *mut Value, data: *mut *mut c_void) -> i32;
    fn napi_get_buffer_info(env: Env, value: Value, data: *mut *mut c_void, len: *mut usize) -> i32;
    fn napi_is_buffer(env: Env, value: Value, result: *mut bool) -> i32;
    fn napi_get_typedarray_info(env: Env, value: Value, kind: *mut i32, length: *mut usize, data: *mut *mut c_void, arraybuffer: *mut Value, offset: *mut usize) -> i32;
    fn napi_is_arraybuffer(env: Env, value: Value, result: *mut bool) -> i32;
    fn napi_get_value_string_utf8(env: Env, value: Value, buf: *mut c_char, size: usize, len: *mut usize) -> i32;
    fn napi_get_value_string_utf16(env: Env, value: Value, buf: *mut u16, size: usize, len: *mut usize) -> i32;
    fn napi_create_string_utf8(env: Env, text: *const c_char, len: usize, result: *mut Value) -> i32;
    fn napi_create_buffer_copy(env: Env, len: usize, data: *const c_void, result_data: *mut *mut c_void, result: *mut Value) -> i32;
    fn napi_create_function(env: Env, name: *const c_char, len: usize, cb: Option<Callback>, data: *mut c_void, result: *mut Value) -> i32;
    fn napi_set_named_property(env: Env, object: Value, name: *const c_char, value: Value) -> i32;
    fn napi_throw_error(env: Env, code: *const c_char, message: *const c_char) -> i32;
    fn napi_get_value_double(env: Env, value: Value, result: *mut f64) -> i32;
    fn napi_typeof(env: Env, value: Value, result: *mut i32) -> i32;
    fn napi_is_array(env: Env, value: Value, result: *mut bool) -> i32;
    fn napi_get_array_length(env: Env, value: Value, result: *mut u32) -> i32;
    fn napi_get_element(env: Env, value: Value, index: u32, result: *mut Value) -> i32;
    fn napi_create_external(env: Env, data: *mut c_void, finalize: Option<unsafe extern "C" fn(Env, *mut c_void, *mut c_void)>, hint: *mut c_void, result: *mut Value) -> i32;
    fn napi_get_value_external(env: Env, value: Value, result: *mut *mut c_void) -> i32;
    fn napi_type_tag_object(env: Env, value: Value, tag: *const TypeTag) -> i32;
    fn napi_check_object_type_tag(env: Env, value: Value, tag: *const TypeTag, result: *mut bool) -> i32;
    fn napi_get_undefined(env: Env, result: *mut Value) -> i32;
    fn napi_create_uint32(env: Env, value: u32, result: *mut Value) -> i32;
}

unsafe fn fail(env: Env, message: &str) -> Value {
    let clean = message.replace('\0', "\\0");
    let message = CString::new(clean).expect("NUL removed");
    napi_throw_error(env, ptr::null(), message.as_ptr());
    ptr::null_mut()
}

unsafe fn args(env: Env, info: Info, expected: usize) -> Result<Vec<Value>, &'static str> {
    // One extra slot detects surplus arguments.
    let mut values = vec![ptr::null_mut(); expected + 1];
    let mut count = values.len();
    if napi_get_cb_info(env, info, &mut count, values.as_mut_ptr(), ptr::null_mut(), ptr::null_mut()) != 0 || count != expected {
        return Err("invalid_argument:incorrect argument count");
    }
    Ok(values)
}

unsafe fn bytes<'a>(env: Env, value: Value) -> Result<&'a [u8], &'static str> {
    let mut is_buffer = false;
    let mut backing = ptr::null_mut();
    let mut is_arraybuffer = false;
    if napi_is_buffer(env, value, &mut is_buffer) != 0 || !is_buffer
        || napi_get_typedarray_info(env, value, ptr::null_mut(), ptr::null_mut(), ptr::null_mut(), &mut backing, ptr::null_mut()) != 0
        || napi_is_arraybuffer(env, backing, &mut is_arraybuffer) != 0 || !is_arraybuffer {
        return Err("invalid_argument:expected a Buffer with non-shared backing storage");
    }
    let mut data = ptr::null_mut();
    let mut len = 0;
    if napi_get_buffer_info(env, value, &mut data, &mut len) != 0 {
        return Err("invalid_argument:expected a Buffer");
    }
    if len == 0 { return Ok(&[]); }
    if data.is_null() || len > isize::MAX as usize { return Err("invalid_argument:invalid Buffer storage"); }
    Ok(std::slice::from_raw_parts(data.cast(), len))
}

unsafe fn string(env: Env, value: Value) -> Result<String, &'static str> {
    bounded_string(env, value, 1024)
}

unsafe fn bounded_string(env: Env, value: Value, max: usize) -> Result<String, &'static str> {
    let mut len = 0;
    if napi_get_value_string_utf8(env, value, ptr::null_mut(), 0, &mut len) != 0 || len > max {
        return Err("invalid_argument:expected a short string");
    }
    let mut units = 0;
    if napi_get_value_string_utf16(env, value, ptr::null_mut(), 0, &mut units) != 0 || units > max {
        return Err("invalid_argument:expected a short string");
    }
    let mut utf16 = vec![0u16; units + 1];
    if napi_get_value_string_utf16(env, value, utf16.as_mut_ptr(), utf16.len(), &mut units) != 0
        || char::decode_utf16(utf16[..units].iter().copied()).any(|c| c.is_err()) {
        return Err("invalid_argument:invalid Unicode string");
    }
    let mut buffer = vec![0u8; len + 1];
    if napi_get_value_string_utf8(env, value, buffer.as_mut_ptr().cast(), buffer.len(), &mut len) != 0 {
        return Err("invalid_argument:expected a string");
    }
    buffer.truncate(len);
    String::from_utf8(buffer).map_err(|_| "invalid_argument:invalid UTF-8")
}

unsafe extern "C" fn detect(env: Env, info: Info) -> Value {
    let result = std::panic::catch_unwind(|| -> Result<String, String> {
        let a = args(env, info, 1).map_err(str::to_owned)?;
        _core::api::detect(bytes(env, a[0]).map_err(str::to_owned)?).map_err(|e| e.to_string())
    });
    match result {
        Ok(Ok(text)) => {
            let mut out = ptr::null_mut();
            if napi_create_string_utf8(env, text.as_ptr().cast(), text.len(), &mut out) == 0 { out } else { fail(env, "native_failure:cannot create string") }
        }
        Ok(Err(e)) => fail(env, &e),
        Err(_) => fail(env, "native_failure:unexpected panic"),
    }
}

unsafe fn number(env: Env, value: Value) -> Result<f64, String> {
    let mut n = 0.0;
    if napi_get_value_double(env, value, &mut n) != 0 || !n.is_finite() {
        return Err("invalid_argument:expected a finite number".into());
    }
    Ok(n)
}

unsafe fn optional_string(env: Env, value: Value) -> Result<Option<String>, String> {
    let mut kind = 0;
    if napi_typeof(env, value, &mut kind) != 0 { return Err("invalid_argument:invalid password".into()); }
    if kind == 0 || kind == 1 { return Ok(None); } // undefined or null
    string(env, value).map(Some).map_err(str::to_owned)
}

unsafe fn clip(env: Env, value: Value) -> Result<Option<_core::api::Clip>, String> {
    let mut kind = 0;
    if napi_typeof(env, value, &mut kind) != 0 { return Err("invalid_argument:invalid clip".into()); }
    if kind == 0 || kind == 1 { return Ok(None); }
    let mut array = false;
    let mut len = 0;
    if napi_is_array(env, value, &mut array) != 0 || !array || napi_get_array_length(env, value, &mut len) != 0 || len != 4 {
        return Err("invalid_argument:clip must be [x, y, width, height]".into());
    }
    let mut coords = [0.0; 4];
    for (i, coord) in coords.iter_mut().enumerate() {
        let mut element = ptr::null_mut();
        if napi_get_element(env, value, i as u32, &mut element) != 0 { return Err("invalid_argument:invalid clip".into()); }
        *coord = number(env, element)?;
    }
    Ok(Some((coords[0], coords[1], coords[2], coords[3])))
}

unsafe extern "C" fn extract(env: Env, info: Info) -> Value {
    let result = std::panic::catch_unwind(|| -> Result<String, String> {
        let a = args(env, info, 3).map_err(str::to_owned)?;
        let input = bytes(env, a[0]).map_err(str::to_owned)?;
        let format = string(env, a[1]).map_err(str::to_owned)?;
        let password = optional_string(env, a[2])?;
        let payload = _core::api::extract(input, &format, password.as_deref()).map_err(|e| e.to_string())?;
        serde_json::to_string(&payload).map_err(|e| format!("native_failure:{e}"))
    });
    match result {
        Ok(Ok(text)) => {
            let mut out = ptr::null_mut();
            if napi_create_string_utf8(env, text.as_ptr().cast(), text.len(), &mut out) == 0 { out } else { fail(env, "native_failure:cannot create string") }
        }
        Ok(Err(e)) => fail(env, &e),
        Err(_) => fail(env, "native_failure:unexpected panic"),
    }
}

unsafe extern "C" fn render(env: Env, info: Info) -> Value {
    let result = std::panic::catch_unwind(|| -> Result<Vec<u8>, String> {
        let a = args(env, info, 6).map_err(str::to_owned)?;
        let format = string(env, a[1]).map_err(str::to_owned)?;
        let page = number(env, a[2])?;
        if page < 0.0 || page.fract() != 0.0 || page >= usize::MAX as f64 {
            return Err("invalid_argument:page must be a non-negative integer".into());
        }
        let dpi = number(env, a[3])?;
        let password = optional_string(env, a[4])?;
        let clip = clip(env, a[5])?;
        // Array element access can invoke JavaScript getters. Borrow the input
        // only after those callbacks, which may detach its backing store.
        let input = bytes(env, a[0]).map_err(str::to_owned)?;
        _core::api::render(input, &format, page as usize, dpi, password.as_deref(), clip).map_err(|e| e.to_string())
    });
    match result {
        Ok(Ok(data)) => {
            let mut out = ptr::null_mut();
            if napi_create_buffer_copy(env, data.len(), data.as_ptr().cast(), ptr::null_mut(), &mut out) == 0 { out } else { fail(env, "native_failure:cannot create Buffer") }
        }
        Ok(Err(e)) => fail(env, &e),
        Err(_) => fail(env, "native_failure:unexpected panic"),
    }
}

unsafe extern "C" fn convert(env: Env, info: Info) -> Value {
    let result = std::panic::catch_unwind(|| -> Result<Vec<u8>, String> {
        let a = args(env, info, 4).map_err(str::to_owned)?;
        let input = bytes(env, a[0]).map_err(str::to_owned)?;
        let source = string(env, a[1]).map_err(str::to_owned)?;
        let target = string(env, a[2]).map_err(str::to_owned)?;
        // The core currently has no reliable guard for the upstream line-break
        // corruption defect. Reject the whole pair rather than risk silent loss.
        let normalized_source = source.trim_start_matches('.').to_ascii_lowercase();
        if normalized_source == "pptx" || normalized_source == "powerpoint" {
            return Err("unsupported:PPTX conversion is disabled in this preview".into());
        }
        let password = optional_string(env, a[3])?;
        _core::api::convert(input, &source, &target, password.as_deref()).map_err(|e| e.to_string())
    });
    match result {
        Ok(Ok(data)) => {
            let mut out = ptr::null_mut();
            if napi_create_buffer_copy(env, data.len(), data.as_ptr().cast(), ptr::null_mut(), &mut out) == 0 { out } else { fail(env, "native_failure:cannot create Buffer") }
        }
        Ok(Err(e)) => fail(env, &e),
        Err(_) => fail(env, "native_failure:unexpected panic"),
    }
}

unsafe extern "C" fn finalize_session(_: Env, data: *mut c_void, _: *mut c_void) {
    if !data.is_null() {
        // Finalizers must not unwind across the Node-API boundary.
        let _ = std::panic::catch_unwind(|| drop(Box::from_raw(data.cast::<SessionHandle>())));
    }
}

unsafe fn session<'a>(env: Env, value: Value) -> Result<&'a SessionHandle, String> {
    let mut tagged = false;
    let mut data = ptr::null_mut();
    let mut kind = 0;
    if napi_typeof(env, value, &mut kind) != 0 || kind != 8
        || napi_check_object_type_tag(env, value, &SESSION_TAG, &mut tagged) != 0 || !tagged
        || napi_get_value_external(env, value, &mut data) != 0 || data.is_null() {
        return Err("invalid_argument:expected a native document session".into());
    }
    Ok(&*data.cast::<SessionHandle>())
}

unsafe extern "C" fn session_open(env: Env, info: Info) -> Value {
    let result = std::panic::catch_unwind(|| -> Result<Value, String> {
        let a = args(env, info, 3).map_err(str::to_owned)?;
        let format = string(env, a[1]).map_err(str::to_owned)?;
        let password = optional_string(env, a[2])?;
        let input = bytes(env, a[0]).map_err(str::to_owned)?;
        let (session, snapshot) = _core::session_api::DocumentSession::open(input, &format, password.as_deref()).map_err(|e| e.to_string())?;
        let snapshot = serde_json::to_string(&snapshot).map_err(|e| format!("native_failure:{e}"))?;
        let handle = Box::new(SessionHandle { session, snapshot: Mutex::new(Some(snapshot)) });
        let raw = Box::into_raw(handle);
        let mut out = ptr::null_mut();
        if napi_create_external(env, raw.cast(), Some(finalize_session), ptr::null_mut(), &mut out) != 0 {
            drop(Box::from_raw(raw));
            return Err("native_failure:cannot create session".into());
        }
        // Once created, the external owns raw, including if tagging fails.
        if napi_type_tag_object(env, out, &SESSION_TAG) != 0 {
            return Err("native_failure:cannot tag session".into());
        }
        Ok(out)
    });
    match result { Ok(Ok(v)) => v, Ok(Err(e)) => fail(env, &e), Err(_) => fail(env, "native_failure:unexpected panic") }
}

unsafe extern "C" fn session_extract(env: Env, info: Info) -> Value {
    let result = std::panic::catch_unwind(|| -> Result<Value, String> {
        let a = args(env, info, 1).map_err(str::to_owned)?;
        let handle = session(env, a[0])?;
        let snapshot = handle.snapshot.lock().map_err(|_| "native_failure:session lock poisoned")?;
        let text = snapshot.as_ref().ok_or("closed:document session is closed")?;
        let mut out = ptr::null_mut();
        if napi_create_string_utf8(env, text.as_ptr().cast(), text.len(), &mut out) != 0 { return Err("native_failure:cannot create string".into()); }
        Ok(out)
    });
    match result { Ok(Ok(v)) => v, Ok(Err(e)) => fail(env, &e), Err(_) => fail(env, "native_failure:unexpected panic") }
}

unsafe extern "C" fn session_render(env: Env, info: Info) -> Value {
    let result = std::panic::catch_unwind(|| -> Result<Value, String> {
        let a = args(env, info, 4).map_err(str::to_owned)?;
        let page = number(env, a[1])?;
        if page < 0.0 || page.fract() != 0.0 || page >= usize::MAX as f64 { return Err("invalid_argument:page must be a non-negative integer".into()); }
        let dpi = number(env, a[2])?;
        // Clip getters can execute JS, including close(); run them before locking.
        let clip = clip(env, a[3])?;
        let handle = session(env, a[0])?;
        let snapshot = handle.snapshot.lock().map_err(|_| "native_failure:session lock poisoned")?;
        if snapshot.is_none() { return Err("closed:document session is closed".into()); }
        let data = handle.session.render(page as usize, dpi, clip).map_err(|e| e.to_string())?;
        let mut out = ptr::null_mut();
        if napi_create_buffer_copy(env, data.len(), data.as_ptr().cast(), ptr::null_mut(), &mut out) != 0 { return Err("native_failure:cannot create Buffer".into()); }
        Ok(out)
    });
    match result { Ok(Ok(v)) => v, Ok(Err(e)) => fail(env, &e), Err(_) => fail(env, "native_failure:unexpected panic") }
}

unsafe extern "C" fn session_close(env: Env, info: Info) -> Value {
    let result = std::panic::catch_unwind(|| -> Result<Value, String> {
        let a = args(env, info, 1).map_err(str::to_owned)?;
        let handle = session(env, a[0])?;
        let mut snapshot = handle.snapshot.lock().map_err(|_| "native_failure:session lock poisoned")?;
        if snapshot.is_some() {
            handle.session.close().map_err(|e| e.to_string())?;
            *snapshot = None;
        }
        let mut out = ptr::null_mut();
        if napi_get_undefined(env, &mut out) != 0 { return Err("native_failure:cannot return undefined".into()); }
        Ok(out)
    });
    match result { Ok(Ok(v)) => v, Ok(Err(e)) => fail(env, &e), Err(_) => fail(env, "native_failure:unexpected panic") }
}

unsafe extern "C" fn session_convert(env: Env, info: Info) -> Value {
    let result = std::panic::catch_unwind(|| -> Result<Value, String> {
        let a = args(env, info, 2).map_err(str::to_owned)?;
        string(env, a[1]).map_err(str::to_owned)?;
        let handle = session(env, a[0])?;
        let snapshot = handle.snapshot.lock().map_err(|_| "native_failure:session lock poisoned")?;
        if snapshot.is_none() { return Err("closed:document session is closed".into()); }
        Err("unsupported:session conversion is not supported by the core".into())
    });
    match result { Ok(Ok(v)) => v, Ok(Err(e)) => fail(env, &e), Err(_) => fail(env, "native_failure:unexpected panic") }
}

type EpubHandle = Mutex<_core::author_epub::EditableEpub>;

unsafe extern "C" fn finalize_epub(_: Env, data: *mut c_void, _: *mut c_void) {
    if !data.is_null() {
        let _ = std::panic::catch_unwind(|| drop(Box::from_raw(data.cast::<EpubHandle>())));
    }
}

unsafe fn epub_handle<'a>(env: Env, value: Value) -> Result<&'a EpubHandle, String> {
    let mut kind = 0;
    let mut tagged = false;
    let mut data = ptr::null_mut();
    if napi_typeof(env, value, &mut kind) != 0 || kind != 8
        || napi_check_object_type_tag(env, value, &EPUB_TAG, &mut tagged) != 0 || !tagged
        || napi_get_value_external(env, value, &mut data) != 0 || data.is_null() {
        return Err("invalid_argument:expected a native EPUB handle".into());
    }
    Ok(&*data.cast::<EpubHandle>())
}

unsafe fn make_epub(env: Env, book: _core::author_epub::EditableEpub) -> Result<Value, String> {
    let raw = Box::into_raw(Box::new(Mutex::new(book)));
    let mut out = ptr::null_mut();
    if napi_create_external(env, raw.cast(), Some(finalize_epub), ptr::null_mut(), &mut out) != 0 {
        drop(Box::from_raw(raw));
        return Err("native_failure:cannot create EPUB handle".into());
    }
    if napi_type_tag_object(env, out, &EPUB_TAG) != 0 {
        return Err("native_failure:cannot tag EPUB handle".into());
    }
    Ok(out)
}

unsafe extern "C" fn epub_new(env: Env, info: Info) -> Value {
    let result = std::panic::catch_unwind(|| -> Result<Value, String> {
        args(env, info, 0).map_err(str::to_owned)?;
        make_epub(env, _core::author_epub::EditableEpub::new())
    });
    match result { Ok(Ok(v)) => v, Ok(Err(e)) => fail(env, &e), Err(_) => fail(env, "native_failure:unexpected panic") }
}

enum EpubOp {
    Metadata(String, String, String, Vec<String>),
    Chapter(String), Count, Copy, Close, Save,
    Heading(usize, String, u8), Paragraph(usize, String), Link(usize, String, String),
    Image(usize, Vec<u8>, String, String),
}

unsafe fn index(env: Env, value: Value) -> Result<usize, String> {
    let n = number(env, value)?;
    if n < 0.0 || n.fract() != 0.0 || n >= usize::MAX as f64 { return Err("invalid_argument:expected a non-negative integer".into()); }
    Ok(n as usize)
}

unsafe fn authors(env: Env, value: Value) -> Result<Vec<String>, String> {
    let mut array = false;
    let mut len = 0;
    if napi_is_array(env, value, &mut array) != 0 || !array || napi_get_array_length(env, value, &mut len) != 0 {
        return Err("invalid_argument:authors must be an array".into());
    }
    if len > 1000 { return Err("resource_limit:EPUB authors exceed 1000".into()); }
    let mut out = Vec::with_capacity(len as usize);
    let mut total = 0usize;
    for i in 0..len {
        let mut item = ptr::null_mut();
        if napi_get_element(env, value, i, &mut item) != 0 { return Err("invalid_argument:invalid author".into()); }
        let author = bounded_string(env, item, 65_536).map_err(str::to_owned)?;
        total += author.len();
        if total > 1024 * 1024 { return Err("resource_limit:EPUB authors exceed 1 MiB".into()); }
        out.push(author);
    }
    Ok(out)
}

fn safe_epub_label(value: &str) -> Result<(), String> {
    if value.contains(['&', '<', '>']) {
        return Err("unsupported:XML-sensitive EPUB metadata and chapter titles cannot round-trip reliably".into());
    }
    Ok(())
}

unsafe extern "C" fn epub_op(env: Env, info: Info) -> Value {
    let result = std::panic::catch_unwind(|| -> Result<Value, String> {
        let a = args(env, info, 6).map_err(str::to_owned)?;
        let op = string(env, a[1]).map_err(str::to_owned)?;
        let text = |value| bounded_string(env, value, 65_536).map_err(str::to_owned);
        // Parse every JS value before locking: array getters may call back into the addon.
        let op = match op.as_str() {
            "metadata" => EpubOp::Metadata(text(a[2])?, bounded_string(env, a[3], 4096).map_err(str::to_owned)?, bounded_string(env, a[4], 128).map_err(str::to_owned)?, authors(env, a[5])?),
            "chapter" => EpubOp::Chapter(text(a[2])?),
            "count" => EpubOp::Count,
            "copy" => EpubOp::Copy,
            "close" => EpubOp::Close,
            "save" => EpubOp::Save,
            "heading" => {
                let chapter = index(env, a[2])?;
                let text = text(a[3])?;
                let level = index(env, a[4])?;
                if !(1..=6).contains(&level) { return Err("invalid_argument:heading level must be between 1 and 6".into()); }
                EpubOp::Heading(chapter, text, level as u8)
            }
            "paragraph" => EpubOp::Paragraph(index(env, a[2])?, text(a[3])?),
            "link" => EpubOp::Link(index(env, a[2])?, text(a[3])?, bounded_string(env, a[4], 16_384).map_err(str::to_owned)?),
            "image" => {
                let chapter = index(env, a[2])?;
                let media = string(env, a[4]).map_err(str::to_owned)?;
                let alt = text(a[5])?;
                let input = bytes(env, a[3]).map_err(str::to_owned)?;
                if input.is_empty() || input.len() > 16 * 1024 * 1024 { return Err("resource_limit:EPUB image must contain at most 16 MiB".into()); }
                let mut data = Vec::new();
                data.try_reserve_exact(input.len()).map_err(|_| "resource_limit:cannot allocate EPUB image")?;
                data.extend_from_slice(input);
                EpubOp::Image(chapter, data, media, alt)
            }
            _ => return Err("invalid_argument:unknown EPUB operation".into()),
        };
        match &op {
            EpubOp::Metadata(t, i, l, authors) => {
                safe_epub_label(t)?; safe_epub_label(i)?; safe_epub_label(l)?;
                for author in authors { safe_epub_label(author)?; }
            }
            EpubOp::Chapter(title) => safe_epub_label(title)?,
            _ => {}
        }
        let handle = epub_handle(env, a[0])?;
        let mut book = handle.lock().map_err(|_| "native_failure:EPUB lock poisoned")?;
        let mut out = ptr::null_mut();
        match op {
            EpubOp::Metadata(t, i, l, a) => book.set_metadata(t, i, l, a).map_err(|e| e.to_string())?,
            EpubOp::Chapter(t) => {
                let n = book.add_chapter(t).map_err(|e| e.to_string())?;
                if napi_create_uint32(env, n as u32, &mut out) != 0 { return Err("native_failure:cannot create number".into()); }
                return Ok(out);
            }
            EpubOp::Count => {
                let n = book.chapter_count().map_err(|e| e.to_string())?;
                if napi_create_uint32(env, n as u32, &mut out) != 0 { return Err("native_failure:cannot create number".into()); }
                return Ok(out);
            }
            EpubOp::Copy => {
                let copy = book.copy().map_err(|e| e.to_string())?;
                drop(book);
                return make_epub(env, copy);
            }
            EpubOp::Close => book.close(),
            EpubOp::Save => {
                let data = book.to_bytes().map_err(|e| e.to_string())?;
                if napi_create_buffer_copy(env, data.len(), data.as_ptr().cast(), ptr::null_mut(), &mut out) != 0 { return Err("native_failure:cannot create Buffer".into()); }
                return Ok(out);
            }
            EpubOp::Heading(c, t, l) => book.add_heading(c, t, l).map_err(|e| e.to_string())?,
            EpubOp::Paragraph(c, t) => book.add_paragraph(c, t).map_err(|e| e.to_string())?,
            EpubOp::Link(c, t, h) => book.add_link(c, t, h).map_err(|e| e.to_string())?,
            EpubOp::Image(c, d, m, a) => book.add_image(c, d, m, a).map_err(|e| e.to_string())?,
        }
        if napi_get_undefined(env, &mut out) != 0 { return Err("native_failure:cannot return undefined".into()); }
        Ok(out)
    });
    match result { Ok(Ok(v)) => v, Ok(Err(e)) => fail(env, &e), Err(_) => fail(env, "native_failure:unexpected panic") }
}

type PdfHandle = Mutex<_core::author_pdf::EditablePdf>;

unsafe extern "C" fn finalize_pdf(_: Env, data: *mut c_void, _: *mut c_void) {
    if !data.is_null() { let _ = std::panic::catch_unwind(|| drop(Box::from_raw(data.cast::<PdfHandle>()))); }
}

unsafe fn pdf_handle<'a>(env: Env, value: Value) -> Result<&'a PdfHandle, String> {
    let (mut kind, mut tagged, mut data) = (0, false, ptr::null_mut());
    if napi_typeof(env, value, &mut kind) != 0 || kind != 8
        || napi_check_object_type_tag(env, value, &PDF_TAG, &mut tagged) != 0 || !tagged
        || napi_get_value_external(env, value, &mut data) != 0 || data.is_null() {
        return Err("invalid_argument:expected a native PDF handle".into());
    }
    Ok(&*data.cast::<PdfHandle>())
}

unsafe fn make_pdf(env: Env, pdf: _core::author_pdf::EditablePdf) -> Result<Value, String> {
    let raw = Box::into_raw(Box::new(Mutex::new(pdf)));
    let mut out = ptr::null_mut();
    if napi_create_external(env, raw.cast(), Some(finalize_pdf), ptr::null_mut(), &mut out) != 0 {
        drop(Box::from_raw(raw));
        return Err("native_failure:cannot create PDF handle".into());
    }
    if napi_type_tag_object(env, out, &PDF_TAG) != 0 { return Err("native_failure:cannot tag PDF handle".into()); }
    Ok(out)
}

unsafe extern "C" fn pdf_new(env: Env, info: Info) -> Value {
    let result = std::panic::catch_unwind(|| -> Result<Value, String> {
        let a = args(env, info, 2).map_err(str::to_owned)?;
        make_pdf(env, _core::author_pdf::EditablePdf::new(number(env, a[0])?, number(env, a[1])?).map_err(|e| e.to_string())?)
    });
    match result { Ok(Ok(v)) => v, Ok(Err(e)) => fail(env, &e), Err(_) => fail(env, "native_failure:unexpected panic") }
}

unsafe extern "C" fn pdf_open(env: Env, info: Info) -> Value {
    let result = std::panic::catch_unwind(|| -> Result<Value, String> {
        let a = args(env, info, 2).map_err(str::to_owned)?;
        let password = optional_string(env, a[1])?;
        let input = bytes(env, a[0]).map_err(str::to_owned)?;
        make_pdf(env, _core::author_pdf::EditablePdf::from_bytes(input, password.as_deref()).map_err(|e| e.to_string())?)
    });
    match result { Ok(Ok(v)) => v, Ok(Err(e)) => fail(env, &e), Err(_) => fail(env, "native_failure:unexpected panic") }
}

enum PdfOp { AddPage(f64, f64), Text(isize, String, f64, f64, f64), Size(isize), Count, Copy, Close, Save }

unsafe fn pdf_index(env: Env, value: Value) -> Result<isize, String> {
    let n = number(env, value)?;
    if n.fract() != 0.0 || n.abs() > 9_007_199_254_740_991.0 || n < isize::MIN as f64 || n >= isize::MAX as f64 {
        return Err("invalid_argument:page must be a safe integer".into());
    }
    Ok(n as isize)
}

unsafe extern "C" fn pdf_op(env: Env, info: Info) -> Value {
    let result = std::panic::catch_unwind(|| -> Result<Value, String> {
        let a = args(env, info, 7).map_err(str::to_owned)?;
        let op = match string(env, a[1]).map_err(str::to_owned)?.as_str() {
            "addPage" => PdfOp::AddPage(number(env, a[2])?, number(env, a[3])?),
            "text" => PdfOp::Text(pdf_index(env, a[2])?, bounded_string(env, a[3], 1_000_000).map_err(str::to_owned)?, number(env, a[4])?, number(env, a[5])?, number(env, a[6])?),
            "size" => PdfOp::Size(pdf_index(env, a[2])?),
            "count" => PdfOp::Count, "copy" => PdfOp::Copy, "close" => PdfOp::Close, "save" => PdfOp::Save,
            _ => return Err("unsupported:unknown PDF authoring operation".into()),
        };
        let mut pdf = pdf_handle(env, a[0])?.lock().map_err(|_| "native_failure:PDF lock poisoned")?;
        let mut out = ptr::null_mut();
        match op {
            PdfOp::AddPage(w, h) => { let n = pdf.add_page(w, h).map_err(|e| e.to_string())?; if napi_create_uint32(env, n as u32, &mut out) != 0 { return Err("native_failure:cannot create number".into()); } return Ok(out); }
            PdfOp::Text(p, t, x, y, s) => pdf.add_text(p, &t, x, y, s, (0, 0, 0), false).map_err(|e| e.to_string())?,
            PdfOp::Size(p) => { let (w, h) = pdf.page_size(p).map_err(|e| e.to_string())?; let text = format!("[{w},{h}]"); if napi_create_string_utf8(env, text.as_ptr().cast(), text.len(), &mut out) != 0 { return Err("native_failure:cannot create string".into()); } return Ok(out); }
            PdfOp::Count => { let n = pdf.page_count().map_err(|e| e.to_string())?; if napi_create_uint32(env, n as u32, &mut out) != 0 { return Err("native_failure:cannot create number".into()); } return Ok(out); }
            PdfOp::Copy => { let copy = pdf.copy().map_err(|e| e.to_string())?; drop(pdf); return make_pdf(env, copy); }
            PdfOp::Close => pdf.close(),
            PdfOp::Save => { let data = pdf.to_bytes().map_err(|e| e.to_string())?; if napi_create_buffer_copy(env, data.len(), data.as_ptr().cast(), ptr::null_mut(), &mut out) != 0 { return Err("native_failure:cannot create Buffer".into()); } return Ok(out); }
        }
        if napi_get_undefined(env, &mut out) != 0 { return Err("native_failure:cannot return undefined".into()); }
        Ok(out)
    });
    match result { Ok(Ok(v)) => v, Ok(Err(e)) => fail(env, &e), Err(_) => fail(env, "native_failure:unexpected panic") }
}

#[no_mangle]
pub unsafe extern "C" fn napi_register_module_v1(env: Env, exports: Value) -> Value {
    for (name, cb) in [(b"detect\0".as_slice(), detect as Callback), (b"convert\0".as_slice(), convert as Callback), (b"extract\0".as_slice(), extract as Callback), (b"render\0".as_slice(), render as Callback), (b"sessionOpen\0".as_slice(), session_open as Callback), (b"sessionExtract\0".as_slice(), session_extract as Callback), (b"sessionRender\0".as_slice(), session_render as Callback), (b"sessionClose\0".as_slice(), session_close as Callback), (b"sessionConvert\0".as_slice(), session_convert as Callback), (b"epubNew\0".as_slice(), epub_new as Callback), (b"epubOp\0".as_slice(), epub_op as Callback), (b"pdfNew\0".as_slice(), pdf_new as Callback), (b"pdfOpen\0".as_slice(), pdf_open as Callback), (b"pdfOp\0".as_slice(), pdf_op as Callback)] {
        let mut function = ptr::null_mut();
        if napi_create_function(env, name.as_ptr().cast(), name.len() - 1, Some(cb), ptr::null_mut(), &mut function) != 0 || napi_set_named_property(env, exports, name.as_ptr().cast(), function) != 0 {
            return fail(env, "native_failure:cannot register function");
        }
    }
    exports
}
