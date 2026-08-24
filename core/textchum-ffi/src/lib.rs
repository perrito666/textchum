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

use textchum_core::{App, Buffer, Config, Document, Event};

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

/// Starts an explicit edit group: every edit until
/// [`tc_document_end_edit_group`] undoes as a single step (e.g. a
/// replace-all).
///
/// # Safety
/// `document` must be a live document pointer.
#[no_mangle]
pub unsafe extern "C" fn tc_document_begin_edit_group(document: *mut TcDocument) {
    if let Some(document) = unsafe { document.as_mut() } {
        document.inner.begin_edit_group();
    }
}

/// Commits the open edit group.
///
/// # Safety
/// `document` must be a live document pointer.
#[no_mangle]
pub unsafe extern "C" fn tc_document_end_edit_group(document: *mut TcDocument) {
    if let Some(document) = unsafe { document.as_mut() } {
        document.inner.end_edit_group();
    }
}

/// Undoes the newest step. On success returns true and stores an array of
/// edits — to be replayed on the display cache **in array order** — in
/// `edits_out`/`count_out`; release the array with
/// [`tc_applied_edits_free`]. Returns false (leaving the outputs zeroed)
/// when there is nothing to undo.
///
/// # Safety
/// `document` must be a live document pointer; `edits_out` and `count_out`
/// must point to writable slots.
#[no_mangle]
pub unsafe extern "C" fn tc_document_undo(
    document: *mut TcDocument,
    edits_out: *mut *mut TcAppliedEdit,
    count_out: *mut usize,
) -> bool {
    unsafe { pop_history(document, edits_out, count_out, |d| d.undo()) }
}

/// Redoes the most recently undone step; same contract as
/// [`tc_document_undo`].
///
/// # Safety
/// `document` must be a live document pointer; `edits_out` and `count_out`
/// must point to writable slots.
#[no_mangle]
pub unsafe extern "C" fn tc_document_redo(
    document: *mut TcDocument,
    edits_out: *mut *mut TcAppliedEdit,
    count_out: *mut usize,
) -> bool {
    unsafe { pop_history(document, edits_out, count_out, |d| d.redo()) }
}

/// Releases an edit array returned by [`tc_document_undo`] or
/// [`tc_document_redo`], including the strings it owns.
///
/// # Safety
/// `edits` and `count` must be exactly the pair produced by one undo/redo
/// call, not previously freed.
#[no_mangle]
pub unsafe extern "C" fn tc_applied_edits_free(edits: *mut TcAppliedEdit, count: usize) {
    if edits.is_null() {
        return;
    }
    let slice = unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(edits, count)) };
    for edit in slice.iter() {
        unsafe { tc_string_free(edit.text) };
    }
}

/// Re-reads the document from its file. On success fills `edit_out` with
/// the single replacement to replay on the display cache (an empty range
/// and empty text means the buffer already matched the disk; release its
/// `text` with [`tc_string_free`]). The reload is one undo step, and the
/// document counts as clean afterwards. Returns false on failure and fills
/// the optional `error_out`.
///
/// # Safety
/// `document` must be a live document pointer; `edit_out` must point to a
/// writable [`TcAppliedEdit`]; `error_out`, if non-null, must point to a
/// writable pointer slot.
#[no_mangle]
pub unsafe extern "C" fn tc_document_reload(
    document: *mut TcDocument,
    edit_out: *mut TcAppliedEdit,
    error_out: *mut *mut c_char,
) -> bool {
    if !error_out.is_null() {
        unsafe { *error_out = std::ptr::null_mut() };
    }
    let Some(document) = (unsafe { document.as_mut() }) else {
        return false;
    };
    if edit_out.is_null() {
        return false;
    }
    catch_unwind(AssertUnwindSafe(|| match document.inner.reload() {
        Ok(edit) => {
            unsafe {
                *edit_out = TcAppliedEdit {
                    start: edit.start_utf16,
                    end: edit.end_utf16,
                    text: owned_c_string(edit.text),
                };
            }
            true
        }
        Err(error) => {
            unsafe { write_error(error_out, &error.to_string()) };
            false
        }
    }))
    .unwrap_or(false)
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

/// The application's JSON-backed configuration. Create with
/// [`tc_config_load`], release with [`tc_config_free`].
pub struct TcConfig {
    inner: Config,
}

/// Loads the configuration file at `path` (`len` bytes of UTF-8). Always
/// returns a usable handle (defaults apply for anything missing or
/// unusable); if the file existed but could not be used and `warning_out`
/// is non-null, a human-readable warning is stored there for the shell to
/// surface once (release with [`tc_string_free`]).
///
/// # Safety
/// `path` must point to `len` readable bytes; `warning_out`, if non-null,
/// must point to a writable pointer slot.
#[no_mangle]
pub unsafe extern "C" fn tc_config_load(
    path: *const c_char,
    len: usize,
    warning_out: *mut *mut c_char,
) -> *mut TcConfig {
    if !warning_out.is_null() {
        unsafe { *warning_out = std::ptr::null_mut() };
    }
    let Some(path) = (unsafe { str_from_raw(path, len) }) else {
        return std::ptr::null_mut();
    };
    catch_unwind(AssertUnwindSafe(|| {
        let (config, warning) = Config::load(std::path::Path::new(path));
        if let Some(warning) = warning {
            unsafe { write_error(warning_out, &warning) };
        }
        Box::into_raw(Box::new(TcConfig { inner: config }))
    }))
    .unwrap_or(std::ptr::null_mut())
}

/// Destroys a configuration handle. Does not save.
///
/// # Safety
/// `config` must be a pointer from [`tc_config_load`], not previously
/// freed.
#[no_mangle]
pub unsafe extern "C" fn tc_config_free(config: *mut TcConfig) {
    if !config.is_null() {
        drop(unsafe { Box::from_raw(config) });
    }
}

/// The configured editor font family, or null when the platform default
/// should be used. Release with [`tc_string_free`].
///
/// # Safety
/// `config` must be a live configuration pointer.
#[no_mangle]
pub unsafe extern "C" fn tc_config_font_family(config: *const TcConfig) -> *mut c_char {
    let Some(config) = (unsafe { config.as_ref() }) else {
        return std::ptr::null_mut();
    };
    match config.inner.font_family() {
        Some(family) => owned_c_string(family.to_owned()),
        None => std::ptr::null_mut(),
    }
}

/// The editor font size in points (already clamped to the valid range).
///
/// # Safety
/// `config` must be a live configuration pointer.
#[no_mangle]
pub unsafe extern "C" fn tc_config_font_size(config: *const TcConfig) -> f64 {
    unsafe { config.as_ref() }.map_or(textchum_core::DEFAULT_FONT_SIZE, |c| c.inner.font_size())
}

/// The tab width in columns (already clamped to the valid range).
///
/// # Safety
/// `config` must be a live configuration pointer.
#[no_mangle]
pub unsafe extern "C" fn tc_config_tab_width(config: *const TcConfig) -> u32 {
    unsafe { config.as_ref() }.map_or(textchum_core::DEFAULT_TAB_WIDTH, |c| c.inner.tab_width())
}

/// Sets the editor font family; `len == 0` clears it back to the platform
/// default.
///
/// # Safety
/// `config` must be a live configuration pointer; `family` must point to
/// `len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn tc_config_set_font_family(
    config: *mut TcConfig,
    family: *const c_char,
    len: usize,
) {
    let Some(config) = (unsafe { config.as_mut() }) else {
        return;
    };
    let Some(family) = (unsafe { str_from_raw(family, len) }) else {
        return;
    };
    config
        .inner
        .set_font_family(if family.is_empty() { None } else { Some(family) });
}

/// Sets the editor font size in points (clamped to the valid range).
///
/// # Safety
/// `config` must be a live configuration pointer.
#[no_mangle]
pub unsafe extern "C" fn tc_config_set_font_size(config: *mut TcConfig, size: f64) {
    if let Some(config) = unsafe { config.as_mut() } {
        config.inner.set_font_size(size);
    }
}

/// Sets the tab width in columns (clamped to the valid range).
///
/// # Safety
/// `config` must be a live configuration pointer.
#[no_mangle]
pub unsafe extern "C" fn tc_config_set_tab_width(config: *mut TcConfig, width: u32) {
    if let Some(config) = unsafe { config.as_mut() } {
        config.inner.set_tab_width(width);
    }
}

/// Writes the configuration back to its file: pretty-printed JSON, written
/// atomically, preserving keys this version does not recognize. If the
/// on-disk file was unparseable at load time it is first copied to
/// `<name>.bak`. Returns false on failure and fills the optional
/// `error_out` (release with [`tc_string_free`]).
///
/// # Safety
/// `config` must be a live configuration pointer; `error_out`, if
/// non-null, must point to a writable pointer slot.
#[no_mangle]
pub unsafe extern "C" fn tc_config_save(
    config: *mut TcConfig,
    error_out: *mut *mut c_char,
) -> bool {
    if !error_out.is_null() {
        unsafe { *error_out = std::ptr::null_mut() };
    }
    let Some(config) = (unsafe { config.as_mut() }) else {
        return false;
    };
    catch_unwind(AssertUnwindSafe(|| match config.inner.save() {
        Ok(()) => true,
        Err(error) => {
            unsafe { write_error(error_out, &error.to_string()) };
            false
        }
    }))
    .unwrap_or(false)
}

/// Shared implementation of undo/redo entry points.
unsafe fn pop_history(
    document: *mut TcDocument,
    edits_out: *mut *mut TcAppliedEdit,
    count_out: *mut usize,
    operation: impl Fn(&mut Document) -> Vec<textchum_core::AppliedEdit>,
) -> bool {
    if edits_out.is_null() || count_out.is_null() {
        return false;
    }
    unsafe {
        *edits_out = std::ptr::null_mut();
        *count_out = 0;
    }
    let Some(document) = (unsafe { document.as_mut() }) else {
        return false;
    };
    catch_unwind(AssertUnwindSafe(|| {
        let edits = operation(&mut document.inner);
        if edits.is_empty() {
            return false;
        }
        let boxed: Box<[TcAppliedEdit]> = edits
            .into_iter()
            .map(|edit| TcAppliedEdit {
                start: edit.start_utf16,
                end: edit.end_utf16,
                text: owned_c_string(edit.text),
            })
            .collect();
        unsafe {
            *count_out = boxed.len();
            *edits_out = Box::into_raw(boxed) as *mut TcAppliedEdit;
        }
        true
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

            let mut edits: *mut TcAppliedEdit = std::ptr::null_mut();
            let mut count: usize = 0;
            assert!(tc_document_undo(doc, &mut edits, &mut count));
            assert_eq!(count, 1);
            assert_eq!(((*edits).start, (*edits).end), (0, 2));
            tc_applied_edits_free(edits, count);
            assert_eq!(tc_document_len_bytes(doc), 0);
            assert!(tc_document_redo(doc, &mut edits, &mut count));
            tc_applied_edits_free(edits, count);
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
