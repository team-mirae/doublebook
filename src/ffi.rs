//! C ABI: one JSON call, one JSON answer. See `include/doublebook.h`.

use std::ffi::{CStr, CString, c_char};
use std::panic::{AssertUnwindSafe, catch_unwind};

fn into_c(s: String) -> *mut c_char {
    // A response never contains NUL: serde_json escapes it.
    CString::new(s).map(CString::into_raw).unwrap_or(std::ptr::null_mut())
}

/// Handles one request (see `api::Request`). The caller owns the returned
/// string and frees it with `doublebook_free`. Never returns NULL for a
/// non-NULL request.
///
/// # Safety
/// `request` must be NULL or a valid NUL-terminated UTF-8 string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn doublebook_call(request: *const c_char) -> *mut c_char {
    if request.is_null() {
        return std::ptr::null_mut();
    }
    let text = unsafe { CStr::from_ptr(request) }.to_string_lossy().into_owned();
    let out = catch_unwind(AssertUnwindSafe(|| crate::api::call(&text)))
        .unwrap_or_else(|_| r#"{"ok":false,"error":{"kind":"badRequest","message":"internal error (panic)"}}"#.to_string());
    into_c(out)
}

/// Frees a string returned by `doublebook_call`.
///
/// # Safety
/// `s` must be NULL or a pointer returned by `doublebook_call`, freed once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn doublebook_free(s: *mut c_char) {
    if !s.is_null() {
        drop(unsafe { CString::from_raw(s) });
    }
}

/// The crate version. Static; do not free.
#[unsafe(no_mangle)]
pub extern "C" fn doublebook_version() -> *const c_char {
    concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr() as *const c_char
}
