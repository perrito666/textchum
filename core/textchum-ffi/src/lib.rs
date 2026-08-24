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

use textchum_core::{App, Buffer, Document, Event};

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
    catch_unwind(AssertUnwindSafe(|| owned_c_string(buffer.inner.text())))
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

/// A text document: buffer, undo history, path and encoding. Create with
/// [`tc_document_new`] or [`tc_document_open`], release with
/// [`tc_document_free`].
pub struct TcDocument {
    inner: Document,
}

/// An edit the core performed on itself via undo/redo, for the shell to
/// replay on its display cache: replace UTF-16 code units `start..end` with
/// `text`. `text` must be released with [`tc_string_free`].
#[repr(C)]
pub struct TcAppliedEdit {
    pub start: usize,
    pub end: usize,
    pub text: *mut c_char,
}

/// Creates an empty, pathless document.
#[no_mangle]
pub extern "C" fn tc_document_new() -> *mut TcDocument {
    Box::into_raw(Box::new(TcDocument {
        inner: Document::new(),
    }))
}

/// Opens the file at `path` (`len` bytes of UTF-8). Returns null on failure
/// and, if `error_out` is non-null, stores a human-readable message there to
/// be released with [`tc_string_free`].
///
/// # Safety
/// `path` must point to `len` readable bytes; `error_out`, if non-null, must
/// point to a writable pointer slot.
#[no_mangle]
pub unsafe extern "C" fn tc_document_open(
    path: *const c_char,
    len: usize,
    error_out: *mut *mut c_char,
) -> *mut TcDocument {
    if !error_out.is_null() {
        unsafe { *error_out = std::ptr::null_mut() };
    }
    let Some(path) = (unsafe { str_from_raw(path, len) }) else {
        unsafe { write_error(error_out, "invalid path string") };
        return std::ptr::null_mut();
    };
    catch_unwind(AssertUnwindSafe(
        || match Document::open(std::path::Path::new(path)) {
            Ok(document) => Box::into_raw(Box::new(TcDocument { inner: document })),
            Err(error) => {
                unsafe { write_error(error_out, &error.to_string()) };
                std::ptr::null_mut()
            }
        },
    ))
    .unwrap_or(std::ptr::null_mut())
}

/// Destroys a document.
///
/// # Safety
/// `document` must be a pointer from a `tc_document_*` constructor, not
/// previously freed.
#[no_mangle]
pub unsafe extern "C" fn tc_document_free(document: *mut TcDocument) {
    if !document.is_null() {
        drop(unsafe { Box::from_raw(document) });
    }
}

/// Replaces the UTF-16 code unit range `start..end` with `len` bytes of
/// UTF-8, recording the edit in the undo history. Returns false, changing
/// nothing, on invalid input.
///
/// # Safety
/// `document` must be a live document pointer; `text` must point to `len`
/// readable bytes.
#[no_mangle]
pub unsafe extern "C" fn tc_document_replace_utf16(
    document: *mut TcDocument,
    start: usize,
    end: usize,
    text: *const c_char,
    len: usize,
) -> bool {
    let Some(document) = (unsafe { document.as_mut() }) else {
        return false;
    };
    let Some(text) = (unsafe { str_from_raw(text, len) }) else {
        return false;
    };
    catch_unwind(AssertUnwindSafe(|| {
        document.inner.replace_utf16(start, end, text).is_ok()
    }))
    .unwrap_or(false)
}

/// Full document contents as a nul-terminated UTF-8 string; release with
/// [`tc_string_free`].
///
/// # Safety
/// `document` must be a live document pointer.
#[no_mangle]
pub unsafe extern "C" fn tc_document_text(document: *const TcDocument) -> *mut c_char {
    let Some(document) = (unsafe { document.as_ref() }) else {
        return std::ptr::null_mut();
    };
    catch_unwind(AssertUnwindSafe(|| owned_c_string(document.inner.text())))
        .unwrap_or(std::ptr::null_mut())
}

/// Document length in bytes.
///
/// # Safety
/// `document` must be a live document pointer.
#[no_mangle]
pub unsafe extern "C" fn tc_document_len_bytes(document: *const TcDocument) -> usize {
    unsafe { document.as_ref() }.map_or(0, |d| d.inner.len_bytes())
}

/// Document length in UTF-16 code units.
///
/// # Safety
/// `document` must be a live document pointer.
#[no_mangle]
pub unsafe extern "C" fn tc_document_len_utf16(document: *const TcDocument) -> usize {
    unsafe { document.as_ref() }.map_or(0, |d| d.inner.len_utf16())
}

/// Whether the document differs from its last saved state.
///
/// # Safety
/// `document` must be a live document pointer.
#[no_mangle]
pub unsafe extern "C" fn tc_document_is_dirty(document: *const TcDocument) -> bool {
    unsafe { document.as_ref() }.map_or(false, |d| d.inner.is_dirty())
}

/// Whether an undo step is available.
///
/// # Safety
/// `document` must be a live document pointer.
#[no_mangle]
pub unsafe extern "C" fn tc_document_can_undo(document: *const TcDocument) -> bool {
    unsafe { document.as_ref() }.map_or(false, |d| d.inner.can_undo())
}

/// Whether a redo step is available.
///
/// # Safety
/// `document` must be a live document pointer.
#[no_mangle]
pub unsafe extern "C" fn tc_document_can_redo(document: *const TcDocument) -> bool {
    unsafe { document.as_ref() }.map_or(false, |d| d.inner.can_redo())
}

/// Ends the current undo coalescing run (call when the caret moves or focus
/// changes, so the next keystroke starts a fresh undo step).
///
/// # Safety
/// `document` must be a live document pointer.
#[no_mangle]
pub unsafe extern "C" fn tc_document_break_undo_group(document: *mut TcDocument) {
    if let Some(document) = unsafe { document.as_mut() } {
        document.inner.break_undo_group();
    }
}

/// Undoes the newest edit. On success returns true and fills `edit_out` with
/// the change the shell must replay on its display cache (release its `text`
/// with [`tc_string_free`]). Returns false when there is nothing to undo.
///
/// # Safety
/// `document` must be a live document pointer; `edit_out` must point to a
/// writable [`TcAppliedEdit`].
#[no_mangle]
pub unsafe extern "C" fn tc_document_undo(
    document: *mut TcDocument,
    edit_out: *mut TcAppliedEdit,
) -> bool {
    unsafe { pop_history(document, edit_out, |d| d.undo()) }
}

/// Redoes the most recently undone edit; same contract as
/// [`tc_document_undo`].
///
/// # Safety
/// `document` must be a live document pointer; `edit_out` must point to a
/// writable [`TcAppliedEdit`].
#[no_mangle]
pub unsafe extern "C" fn tc_document_redo(
    document: *mut TcDocument,
    edit_out: *mut TcAppliedEdit,
) -> bool {
    unsafe { pop_history(document, edit_out, |d| d.redo()) }
}

/// Saves to the document's current path. Returns false on failure and, if
/// `error_out` is non-null, stores a message there (release with
/// [`tc_string_free`]). An untitled document fails; use
/// [`tc_document_save_as`].
///
/// # Safety
/// `document` must be a live document pointer; `error_out`, if non-null,
/// must point to a writable pointer slot.
#[no_mangle]
pub unsafe extern "C" fn tc_document_save(
    document: *mut TcDocument,
    error_out: *mut *mut c_char,
) -> bool {
    unsafe { save_with(document, error_out, |d| d.save()) }
}

/// Saves to `path` (`len` bytes of UTF-8) and adopts it as the document's
/// path. Same error contract as [`tc_document_save`].
///
/// # Safety
/// `document` must be a live document pointer; `path` must point to `len`
/// readable bytes; `error_out`, if non-null, must point to a writable
/// pointer slot.
#[no_mangle]
pub unsafe extern "C" fn tc_document_save_as(
    document: *mut TcDocument,
    path: *const c_char,
    len: usize,
    error_out: *mut *mut c_char,
) -> bool {
    let Some(path) = (unsafe { str_from_raw(path, len) }) else {
        unsafe { write_error(error_out, "invalid path string") };
        return false;
    };
    let path = std::path::Path::new(path).to_owned();
    unsafe { save_with(document, error_out, move |d| d.save_as(&path)) }
}

/// The document's file path as a nul-terminated UTF-8 string, or null for
/// untitled documents. Release with [`tc_string_free`].
///
/// # Safety
/// `document` must be a live document pointer.
#[no_mangle]
pub unsafe extern "C" fn tc_document_path(document: *const TcDocument) -> *mut c_char {
    let Some(document) = (unsafe { document.as_ref() }) else {
        return std::ptr::null_mut();
    };
    match document.inner.path() {
        Some(path) => owned_c_string(path.to_string_lossy().into_owned()),
        None => std::ptr::null_mut(),
    }
}

/// The document's encoding as a static human-readable name (e.g. "UTF-8").
/// Owned by the core; do not free.
///
/// # Safety
/// `document` must be a live document pointer.
#[no_mangle]
pub unsafe extern "C" fn tc_document_encoding_name(document: *const TcDocument) -> *const c_char {
    let name: &'static str = unsafe { document.as_ref() }
        .map_or("UTF-8", |d| d.inner.encoding().name());
    // Encoding names are compile-time constants; return a static
    // nul-terminated variant of the same.
    match name {
        "UTF-8 with BOM" => "UTF-8 with BOM\0".as_ptr() as *const c_char,
        "ISO-8859-1" => "ISO-8859-1\0".as_ptr() as *const c_char,
        _ => "UTF-8\0".as_ptr() as *const c_char,
    }
}

/// Shared implementation of undo/redo entry points.
unsafe fn pop_history(
    document: *mut TcDocument,
    edit_out: *mut TcAppliedEdit,
    operation: impl Fn(&mut Document) -> Option<textchum_core::AppliedEdit>,
) -> bool {
    let Some(document) = (unsafe { document.as_mut() }) else {
        return false;
    };
    if edit_out.is_null() {
        return false;
    }
    catch_unwind(AssertUnwindSafe(|| {
        match operation(&mut document.inner) {
            Some(edit) => {
                unsafe {
                    *edit_out = TcAppliedEdit {
                        start: edit.start_utf16,
                        end: edit.end_utf16,
                        text: owned_c_string(edit.text),
                    };
                }
                true
            }
            None => false,
        }
    }))
    .unwrap_or(false)
}

/// Shared implementation of the save entry points.
unsafe fn save_with(
    document: *mut TcDocument,
    error_out: *mut *mut c_char,
    operation: impl FnOnce(&mut Document) -> Result<(), textchum_core::DocumentError>,
) -> bool {
    if !error_out.is_null() {
        unsafe { *error_out = std::ptr::null_mut() };
    }
    let Some(document) = (unsafe { document.as_mut() }) else {
        return false;
    };
    catch_unwind(AssertUnwindSafe(|| match operation(&mut document.inner) {
        Ok(()) => true,
        Err(error) => {
            unsafe { write_error(error_out, &error.to_string()) };
            false
        }
    }))
    .unwrap_or(false)
}

/// Stores an error message in an optional out-parameter.
unsafe fn write_error(error_out: *mut *mut c_char, message: &str) {
    if !error_out.is_null() {
        unsafe { *error_out = owned_c_string(message.to_owned()) };
    }
}

/// Converts an owned string to a caller-freed C string, replacing interior
/// nuls (legal in documents, fatal to C strings) with U+FFFD.
fn owned_c_string(text: String) -> *mut c_char {
    let sanitized = if text.contains('\0') {
        text.replace('\0', "\u{FFFD}")
    } else {
        text
    };
    CString::new(sanitized).map_or(std::ptr::null_mut(), CString::into_raw)
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
    fn document_edit_undo_save_through_ffi() {
        unsafe {
            let dir = std::env::temp_dir().join(format!("textchum-ffi-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("ffi.txt").to_string_lossy().into_owned();

            let doc = tc_document_new();
            let hi = "hi";
            assert!(tc_document_replace_utf16(
                doc,
                0,
                0,
                hi.as_ptr() as *const c_char,
                hi.len()
            ));
            assert!(tc_document_is_dirty(doc));

            let mut error: *mut c_char = std::ptr::null_mut();
            assert!(!tc_document_save(doc, &mut error), "untitled save must fail");
            assert!(!error.is_null());
            tc_string_free(error);

            let mut error: *mut c_char = std::ptr::null_mut();
            assert!(tc_document_save_as(
                doc,
                path.as_ptr() as *const c_char,
                path.len(),
                &mut error
            ));
            assert!(error.is_null());
            assert!(!tc_document_is_dirty(doc));

            let mut edit = TcAppliedEdit {
                start: 0,
                end: 0,
                text: std::ptr::null_mut(),
            };
            assert!(tc_document_undo(doc, &mut edit));
            assert_eq!((edit.start, edit.end), (0, 2));
            tc_string_free(edit.text);
            assert_eq!(tc_document_len_bytes(doc), 0);
            assert!(tc_document_redo(doc, &mut edit));
            tc_string_free(edit.text);
            tc_document_free(doc);

            let mut error: *mut c_char = std::ptr::null_mut();
            let reopened = tc_document_open(
                path.as_ptr() as *const c_char,
                path.len(),
                &mut error
            );
            assert!(!reopened.is_null());
            let text = tc_document_text(reopened);
            assert_eq!(std::ffi::CStr::from_ptr(text).to_str().unwrap(), "hi");
            tc_string_free(text);
            tc_document_free(reopened);
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
