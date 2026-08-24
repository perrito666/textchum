//! C ABI over `textchum-core`.
//!
//! This crate is the only place where the core meets a foreign language.
//! Its rules:
//!
//! * Every exported type is opaque; callers hold pointers and pass them back.
//! * Strings cross the boundary as UTF-8 `(pointer, length)` pairs, except
//!   for returned strings which are nul-terminated and must be released with
//!   [`tc_string_free`].
//! * Fallible calls return `bool`; failure means the operation validated its
//!   inputs, rejected them, and changed nothing.
//! * Panics never unwind across the boundary: every entry point is wrapped in
//!   `catch_unwind` and reports failure instead.
//! * Calls into this API must come from a single thread. Events flow the
//!   other way on one core-owned dispatch thread (see [`tc_app_new`]).

use std::ffi::{c_char, c_void, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};

use textchum_core::{App, Buffer, Event};

/// Event kind: reply to `tc_app_ping`.
pub const TC_EVENT_PONG: u32 = 1;

/// An event delivered from the core to the shell.
///
/// The struct is only valid for the duration of the callback invocation;
/// copy anything you need out of it.
#[repr(C)]
pub struct TcEvent {
    /// One of the `TC_EVENT_*` constants.
    pub kind: u32,
    /// Event-specific sequence number (currently used by pong events).
    pub seq: u64,
}

/// Shell-provided event sink. Invoked on the core's single dispatch thread;
/// implementations must hop to their UI thread themselves.
pub type TcEventCallback = Option<extern "C" fn(event: *const TcEvent, userdata: *mut c_void)>;

/// Root handle for a core instance. Create with [`tc_app_new`], release with
/// [`tc_app_free`].
pub struct TcApp {
    inner: App,
}

/// An in-memory text document. Create with [`tc_buffer_new`], release with
/// [`tc_buffer_free`].
pub struct TcBuffer {
    inner: Buffer,
}

/// Wrapper making the raw userdata pointer sendable to the dispatch thread.
/// The shell guarantees the pointee outlives the app handle.
struct UserData(*mut c_void);
unsafe impl Send for UserData {}
unsafe impl Sync for UserData {}

impl UserData {
    // Accessed via a method so closures capture the Send wrapper as a whole
    // rather than the raw pointer field (which is not Send).
    fn get(&self) -> *mut c_void {
        self.0
    }
}

/// Returns the core version as a static nul-terminated UTF-8 string.
/// The returned pointer is owned by the core; do not free it.
#[no_mangle]
pub extern "C" fn tc_version() -> *const c_char {
    concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr() as *const c_char
}

/// Creates a core instance whose events are delivered to `callback`.
///
/// `userdata` is passed back verbatim on every invocation and must remain
/// valid until after [`tc_app_free`] returns. Returns null if `callback` is
/// null.
#[no_mangle]
pub extern "C" fn tc_app_new(callback: TcEventCallback, userdata: *mut c_void) -> *mut TcApp {
    let Some(callback) = callback else {
        return std::ptr::null_mut();
    };
    let userdata = UserData(userdata);
    catch_unwind(|| {
        let app = App::new(move |event| {
            let event = match event {
                Event::Pong { seq } => TcEvent {
                    kind: TC_EVENT_PONG,
                    seq,
                },
            };
            callback(&event, userdata.get());
        });
        Box::into_raw(Box::new(TcApp { inner: app }))
    })
    .unwrap_or(std::ptr::null_mut())
}

/// Requests an asynchronous pong event echoing `seq`.
///
/// # Safety
/// `app` must be a live pointer from [`tc_app_new`].
#[no_mangle]
pub unsafe extern "C" fn tc_app_ping(app: *mut TcApp, seq: u64) {
    let Some(app) = (unsafe { app.as_ref() }) else {
        return;
    };
    let _ = catch_unwind(AssertUnwindSafe(|| app.inner.ping(seq)));
}

/// Destroys an app handle. Blocks until queued events have been delivered
/// and guarantees the callback is never invoked afterwards.
///
/// # Safety
/// `app` must be a pointer from [`tc_app_new`], not previously freed.
#[no_mangle]
pub unsafe extern "C" fn tc_app_free(app: *mut TcApp) {
    if !app.is_null() {
        drop(unsafe { Box::from_raw(app) });
    }
}

/// Creates an empty buffer.
#[no_mangle]
pub extern "C" fn tc_buffer_new() -> *mut TcBuffer {
    Box::into_raw(Box::new(TcBuffer {
        inner: Buffer::new(),
    }))
}

/// Destroys a buffer.
///
/// # Safety
/// `buffer` must be a pointer from [`tc_buffer_new`], not previously freed.
#[no_mangle]
pub unsafe extern "C" fn tc_buffer_free(buffer: *mut TcBuffer) {
    if !buffer.is_null() {
        drop(unsafe { Box::from_raw(buffer) });
    }
}

/// Buffer length in bytes.
///
/// # Safety
/// `buffer` must be a live pointer from [`tc_buffer_new`].
#[no_mangle]
pub unsafe extern "C" fn tc_buffer_len_bytes(buffer: *const TcBuffer) -> usize {
    unsafe { buffer.as_ref() }.map_or(0, |b| b.inner.len_bytes())
}

/// Buffer length in UTF-16 code units (the unit AppKit and LSP count in).
///
/// # Safety
/// `buffer` must be a live pointer from [`tc_buffer_new`].
#[no_mangle]
pub unsafe extern "C" fn tc_buffer_len_utf16(buffer: *const TcBuffer) -> usize {
    unsafe { buffer.as_ref() }.map_or(0, |b| b.inner.len_utf16())
}

/// Inserts `len` bytes of UTF-8 at byte offset `offset`.
///
/// Returns false — changing nothing — if the offset is out of bounds or not
/// a character boundary, or if the bytes are not valid UTF-8.
///
/// # Safety
/// `buffer` must be a live pointer from [`tc_buffer_new`]; `text` must point
/// to `len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn tc_buffer_insert(
    buffer: *mut TcBuffer,
    offset: usize,
    text: *const c_char,
    len: usize,
) -> bool {
    let Some(buffer) = (unsafe { buffer.as_mut() }) else {
        return false;
    };
    let Some(text) = (unsafe { str_from_raw(text, len) }) else {
        return false;
    };
    catch_unwind(AssertUnwindSafe(|| {
        buffer.inner.insert_bytes(offset, text).is_ok()
    }))
    .unwrap_or(false)
}

/// Deletes the byte range `start..end`. Returns false, changing nothing, on
/// invalid ranges.
///
/// # Safety
/// `buffer` must be a live pointer from [`tc_buffer_new`].
#[no_mangle]
pub unsafe extern "C" fn tc_buffer_delete(buffer: *mut TcBuffer, start: usize, end: usize) -> bool {
    let Some(buffer) = (unsafe { buffer.as_mut() }) else {
        return false;
    };
    catch_unwind(AssertUnwindSafe(|| {
        buffer.inner.delete_bytes(start, end).is_ok()
    }))
    .unwrap_or(false)
}

/// Replaces the UTF-16 code unit range `start..end` with `len` bytes of
/// UTF-8. This maps one-to-one onto an AppKit `NSRange` edit. Returns false,
/// changing nothing, on invalid input.
///
/// # Safety
/// `buffer` must be a live pointer from [`tc_buffer_new`]; `text` must point
/// to `len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn tc_buffer_replace_utf16(
    buffer: *mut TcBuffer,
    start: usize,
    end: usize,
    text: *const c_char,
    len: usize,
) -> bool {
    let Some(buffer) = (unsafe { buffer.as_mut() }) else {
        return false;
    };
    let Some(text) = (unsafe { str_from_raw(text, len) }) else {
        return false;
    };
    catch_unwind(AssertUnwindSafe(|| {
        buffer.inner.replace_utf16(start, end, text).is_ok()
    }))
    .unwrap_or(false)
}

/// Returns the entire buffer contents as a nul-terminated UTF-8 string.
/// Release with [`tc_string_free`]. Returns null on allocation failure.
///
/// # Safety
/// `buffer` must be a live pointer from [`tc_buffer_new`].
#[no_mangle]
pub unsafe extern "C" fn tc_buffer_text(buffer: *const TcBuffer) -> *mut c_char {
    let Some(buffer) = (unsafe { buffer.as_ref() }) else {
        return std::ptr::null_mut();
    };
    catch_unwind(AssertUnwindSafe(|| {
        // Buffer text never contains interior nuls only if the document has
        // none; documents may legitimately contain them, so replace rather
        // than fail. A (pointer, length) accessor can supersede this if nul
        // bytes ever matter in practice.
        let text = buffer.inner.text().replace('\0', "\u{FFFD}");
        CString::new(text).map_or(std::ptr::null_mut(), CString::into_raw)
    }))
    .unwrap_or(std::ptr::null_mut())
}

/// Releases a string returned by the core.
///
/// # Safety
/// `s` must be a pointer returned by a `tc_*` function documented to be
/// released with this call, not previously freed.
#[no_mangle]
pub unsafe extern "C" fn tc_string_free(s: *mut c_char) {
    if !s.is_null() {
        drop(unsafe { CString::from_raw(s) });
    }
}

/// Borrows a `(pointer, length)` pair as `&str`, rejecting null pointers
/// (unless empty) and invalid UTF-8.
unsafe fn str_from_raw<'a>(text: *const c_char, len: usize) -> Option<&'a str> {
    if len == 0 {
        return Some("");
    }
    if text.is_null() {
        return None;
    }
    let bytes = unsafe { std::slice::from_raw_parts(text as *const u8, len) };
    std::str::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_round_trip_through_ffi() {
        unsafe {
            let buf = tc_buffer_new();
            let hello = "hola mundo";
            assert!(tc_buffer_insert(
                buf,
                0,
                hello.as_ptr() as *const c_char,
                hello.len()
            ));
            assert_eq!(tc_buffer_len_bytes(buf), 10);
            let text = tc_buffer_text(buf);
            let s = std::ffi::CStr::from_ptr(text).to_str().unwrap().to_owned();
            tc_string_free(text);
            tc_buffer_free(buf);
            assert_eq!(s, "hola mundo");
        }
    }

    #[test]
    fn invalid_input_is_rejected_not_fatal() {
        unsafe {
            let buf = tc_buffer_new();
            assert!(!tc_buffer_insert(buf, 5, "x".as_ptr() as *const c_char, 1));
            assert!(!tc_buffer_insert(buf, 0, std::ptr::null(), 3));
            assert!(!tc_buffer_delete(buf, 2, 1));
            tc_buffer_free(buf);
        }
    }

    #[test]
    fn app_pong_round_trip() {
        use std::sync::mpsc;

        extern "C" fn callback(event: *const TcEvent, userdata: *mut c_void) {
            let tx = unsafe { &*(userdata as *const mpsc::Sender<u64>) };
            let event = unsafe { &*event };
            assert_eq!(event.kind, TC_EVENT_PONG);
            tx.send(event.seq).unwrap();
        }

        let (tx, rx) = mpsc::channel::<u64>();
        let app = tc_app_new(Some(callback), &tx as *const _ as *mut c_void);
        assert!(!app.is_null());
        unsafe {
            tc_app_ping(app, 42);
            assert_eq!(
                rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap(),
                42
            );
            tc_app_free(app);
        }
    }
}
