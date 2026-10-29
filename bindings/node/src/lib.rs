//! Minimal Node-API boundary. No Node package or build-time binding generator is needed.
use std::ffi::{c_char, c_void, CString};
use std::ptr;

type Env = *mut c_void;
type Value = *mut c_void;
type Info = *mut c_void;
type Callback = unsafe extern "C" fn(Env, Info) -> Value;

extern "C" {
    fn napi_get_cb_info(env: Env, info: Info, argc: *mut usize, argv: *mut Value, this: *mut Value, data: *mut *mut c_void) -> i32;
    fn napi_get_buffer_info(env: Env, value: Value, data: *mut *mut c_void, len: *mut usize) -> i32;
    fn napi_is_buffer(env: Env, value: Value, result: *mut bool) -> i32;
    fn napi_get_typedarray_info(env: Env, value: Value, kind: *mut i32, length: *mut usize, data: *mut *mut c_void, arraybuffer: *mut Value, offset: *mut usize) -> i32;
    fn napi_is_arraybuffer(env: Env, value: Value, result: *mut bool) -> i32;
    fn napi_get_value_string_utf8(env: Env, value: Value, buf: *mut c_char, size: usize, len: *mut usize) -> i32;
    fn napi_create_string_utf8(env: Env, text: *const c_char, len: usize, result: *mut Value) -> i32;
    fn napi_create_buffer_copy(env: Env, len: usize, data: *const c_void, result_data: *mut *mut c_void, result: *mut Value) -> i32;
    fn napi_create_function(env: Env, name: *const c_char, len: usize, cb: Option<Callback>, data: *mut c_void, result: *mut Value) -> i32;
    fn napi_set_named_property(env: Env, object: Value, name: *const c_char, value: Value) -> i32;
    fn napi_throw_error(env: Env, code: *const c_char, message: *const c_char) -> i32;
    fn napi_get_value_double(env: Env, value: Value, result: *mut f64) -> i32;
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
    let mut len = 0;
    if napi_get_value_string_utf8(env, value, ptr::null_mut(), 0, &mut len) != 0 || len > 1024 {
        return Err("invalid_argument:expected a short string");
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

unsafe extern "C" fn extract(env: Env, info: Info) -> Value {
    let result = std::panic::catch_unwind(|| -> Result<String, String> {
        let a = args(env, info, 2).map_err(str::to_owned)?;
        let input = bytes(env, a[0]).map_err(str::to_owned)?;
        let format = string(env, a[1]).map_err(str::to_owned)?;
        let payload = _core::api::extract(input, &format, None).map_err(|e| e.to_string())?;
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
        let a = args(env, info, 4).map_err(str::to_owned)?;
        let input = bytes(env, a[0]).map_err(str::to_owned)?;
        let format = string(env, a[1]).map_err(str::to_owned)?;
        let page = number(env, a[2])?;
        if page < 0.0 || page.fract() != 0.0 || page >= usize::MAX as f64 {
            return Err("invalid_argument:page must be a non-negative integer".into());
        }
        let dpi = number(env, a[3])?;
        _core::api::render(input, &format, page as usize, dpi, None, None).map_err(|e| e.to_string())
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
        let a = args(env, info, 3).map_err(str::to_owned)?;
        let input = bytes(env, a[0]).map_err(str::to_owned)?;
        let source = string(env, a[1]).map_err(str::to_owned)?;
        let target = string(env, a[2]).map_err(str::to_owned)?;
        // The core currently has no reliable guard for the upstream line-break
        // corruption defect. Reject the whole pair rather than risk silent loss.
        let normalized_source = source.trim_start_matches('.').to_ascii_lowercase();
        if normalized_source == "pptx" || normalized_source == "powerpoint" {
            return Err("unsupported:PPTX conversion is disabled in this preview".into());
        }
        _core::api::convert(input, &source, &target, None).map_err(|e| e.to_string())
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

#[no_mangle]
pub unsafe extern "C" fn napi_register_module_v1(env: Env, exports: Value) -> Value {
    for (name, cb) in [(b"detect\0".as_slice(), detect as Callback), (b"convert\0".as_slice(), convert as Callback), (b"extract\0".as_slice(), extract as Callback), (b"render\0".as_slice(), render as Callback)] {
        let mut function = ptr::null_mut();
        if napi_create_function(env, name.as_ptr().cast(), name.len() - 1, Some(cb), ptr::null_mut(), &mut function) != 0 || napi_set_named_property(env, exports, name.as_ptr().cast(), function) != 0 {
            return fail(env, "native_failure:cannot register function");
        }
    }
    exports
}
