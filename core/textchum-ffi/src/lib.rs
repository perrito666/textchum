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
/// Event kind: a language server published diagnostics. `path` is the
/// file; `payload` is a JSON array of `{line, character, endLine,
/// endCharacter, severity, message}` (LSP positions: zero-based line,
/// UTF-16 column).
pub const TC_EVENT_DIAGNOSTICS: u32 = 2;
/// Event kind: a language-server instance changed state. `server` is the
/// server id, `path` its project root, `status` one of
/// starting/running/not-found/failed/exited, `payload` a human-readable
/// message.
pub const TC_EVENT_SERVER_STATUS: u32 = 3;
/// Event kind: a language server answered a request. `seq` is the id the
/// request call returned; `payload` is the response's `result` as JSON.
pub const TC_EVENT_LSP_RESPONSE: u32 = 4;

/// An event delivered from the core to the shell.
///
/// The struct and every string it points to are only valid for the
/// duration of the callback invocation; copy anything you need out of it.
/// Strings not applicable to the event kind are null.
#[repr(C)]
pub struct TcEvent {
    /// One of the `TC_EVENT_*` constants.
    pub kind: u32,
    /// Sequence number (pong events).
    pub seq: u64,
    /// File path (diagnostics) or project root (server status).
    pub path: *const c_char,
    /// Server id (server status).
    pub server: *const c_char,
    /// Status string (server status).
    pub status: *const c_char,
    /// JSON payload (diagnostics) or message text (server status).
    pub payload: *const c_char,
}

impl TcEvent {
    fn new(kind: u32) -> Self {
        Self {
            kind,
            seq: 0,
            path: std::ptr::null(),
            server: std::ptr::null(),
            status: std::ptr::null(),
            payload: std::ptr::null(),
        }
    }
}

/// An owned, sanitized C string for the duration of a callback.
fn callback_cstring(text: String) -> CString {
    CString::new(text.replace('\0', "\u{FFFD}")).expect("nul bytes replaced")
}

/// Shell-provided event sink. Invoked on the core's single dispatch thread;
/// implementations must hop to their UI thread themselves.
pub type TcEventCallback = Option<extern "C" fn(event: *const TcEvent, userdata: *mut c_void)>;

/// Root handle for a core instance. Create with [`tc_app_new`], release with
/// [`tc_app_free`]. Owns the language-server pool.
///
/// Field order is load-bearing: the pool holds an event-sender clone, and
/// `App`'s drop joins its dispatcher thread, which only ends once every
/// sender is gone — so the pool must drop first.
pub struct TcApp {
    pool: textchum_lsp::Pool,
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
            // The CStrings live on this frame for the whole callback.
            match event {
                Event::Pong { seq } => {
                    let mut out = TcEvent::new(TC_EVENT_PONG);
                    out.seq = seq;
                    callback(&out, userdata.get());
                }
                Event::Diagnostics { path, json } => {
                    let path = callback_cstring(path);
                    let json = callback_cstring(json);
                    let mut out = TcEvent::new(TC_EVENT_DIAGNOSTICS);
                    out.path = path.as_ptr();
                    out.payload = json.as_ptr();
                    callback(&out, userdata.get());
                }
                Event::ServerStatus {
                    server,
                    root,
                    status,
                    message,
                } => {
                    let server = callback_cstring(server);
                    let root = callback_cstring(root);
                    let status = callback_cstring(status);
                    let message = callback_cstring(message);
                    let mut out = TcEvent::new(TC_EVENT_SERVER_STATUS);
                    out.server = server.as_ptr();
                    out.path = root.as_ptr();
                    out.status = status.as_ptr();
                    out.payload = message.as_ptr();
                    callback(&out, userdata.get());
                }
                Event::LspResponse { id, json } => {
                    let json = callback_cstring(json);
                    let mut out = TcEvent::new(TC_EVENT_LSP_RESPONSE);
                    out.seq = id;
                    out.payload = json.as_ptr();
                    callback(&out, userdata.get());
                }
            }
        });
        let pool = textchum_lsp::Pool::new(app.sender());
        Box::into_raw(Box::new(TcApp { inner: app, pool }))
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

/// Announces an opened document to the language-server pool: `path`, its
/// syntax `language` name, and the full `text`. Spawns the (server,
/// project-root) instance on first use; does nothing for languages
/// without a registered server.
///
/// # Safety
/// `app` must be a live pointer from [`tc_app_new`]; each pointer/length
/// pair must describe readable UTF-8.
#[no_mangle]
pub unsafe extern "C" fn tc_lsp_did_open(
    app: *mut TcApp,
    path: *const c_char,
    path_len: usize,
    language: *const c_char,
    language_len: usize,
    text: *const c_char,
    text_len: usize,
) {
    let Some(app) = (unsafe { app.as_mut() }) else {
        return;
    };
    let (path, language, text) = unsafe {
        (
            str_from_raw(path, path_len),
            str_from_raw(language, language_len),
            str_from_raw(text, text_len),
        )
    };
    let (Some(path), Some(language), Some(text)) = (path, language, text) else {
        return;
    };
    let _ = catch_unwind(AssertUnwindSafe(|| {
        app.pool.did_open(std::path::Path::new(path), language, text);
    }));
}

/// Announces new contents for an opened document (full-text sync).
///
/// # Safety
/// Same contract as [`tc_lsp_did_open`].
#[no_mangle]
pub unsafe extern "C" fn tc_lsp_did_change(
    app: *mut TcApp,
    path: *const c_char,
    path_len: usize,
    text: *const c_char,
    text_len: usize,
) {
    let Some(app) = (unsafe { app.as_mut() }) else {
        return;
    };
    let (path, text) = unsafe { (str_from_raw(path, path_len), str_from_raw(text, text_len)) };
    let (Some(path), Some(text)) = (path, text) else {
        return;
    };
    let _ = catch_unwind(AssertUnwindSafe(|| {
        app.pool.did_change(std::path::Path::new(path), text);
    }));
}

/// Requests hover information at an LSP position (zero-based line, UTF-16
/// column). Returns the request id whose `TC_EVENT_LSP_RESPONSE` event
/// will carry the answer, or 0 when the document has no server.
///
/// # Safety
/// Same contract as [`tc_lsp_did_open`].
#[no_mangle]
pub unsafe extern "C" fn tc_lsp_hover(
    app: *mut TcApp,
    path: *const c_char,
    path_len: usize,
    line: u32,
    character: u32,
) -> u64 {
    let Some(app) = (unsafe { app.as_mut() }) else {
        return 0;
    };
    let Some(path) = (unsafe { str_from_raw(path, path_len) }) else {
        return 0;
    };
    catch_unwind(AssertUnwindSafe(|| {
        app.pool.hover(std::path::Path::new(path), line, character)
    }))
    .unwrap_or(0)
}

/// Requests the definition location(s) of the symbol at an LSP position;
/// same contract as [`tc_lsp_hover`]. The response's `result` is an LSP
/// `Location`, `Location[]`, or `LocationLink[]`.
///
/// # Safety
/// Same contract as [`tc_lsp_did_open`].
#[no_mangle]
pub unsafe extern "C" fn tc_lsp_definition(
    app: *mut TcApp,
    path: *const c_char,
    path_len: usize,
    line: u32,
    character: u32,
) -> u64 {
    let Some(app) = (unsafe { app.as_mut() }) else {
        return 0;
    };
    let Some(path) = (unsafe { str_from_raw(path, path_len) }) else {
        return 0;
    };
    catch_unwind(AssertUnwindSafe(|| {
        app.pool
            .definition(std::path::Path::new(path), line, character)
    }))
    .unwrap_or(0)
}

/// Requests the document's symbol tree; same contract as
/// [`tc_lsp_hover`]. The response's `result` is an LSP
/// `DocumentSymbol[]` (hierarchical) or `SymbolInformation[]` (flat).
///
/// # Safety
/// Same contract as [`tc_lsp_did_open`].
#[no_mangle]
pub unsafe extern "C" fn tc_lsp_document_symbols(
    app: *mut TcApp,
    path: *const c_char,
    path_len: usize,
) -> u64 {
    let Some(app) = (unsafe { app.as_mut() }) else {
        return 0;
    };
    let Some(path) = (unsafe { str_from_raw(path, path_len) }) else {
        return 0;
    };
    catch_unwind(AssertUnwindSafe(|| {
        app.pool.document_symbols(std::path::Path::new(path))
    }))
    .unwrap_or(0)
}

/// Requests every reference to the symbol at an LSP position, the
/// declaration included; same contract as [`tc_lsp_hover`]. The
/// response's `result` is an LSP `Location[]`.
///
/// # Safety
/// Same contract as [`tc_lsp_did_open`].
#[no_mangle]
pub unsafe extern "C" fn tc_lsp_references(
    app: *mut TcApp,
    path: *const c_char,
    path_len: usize,
    line: u32,
    character: u32,
) -> u64 {
    let Some(app) = (unsafe { app.as_mut() }) else {
        return 0;
    };
    let Some(path) = (unsafe { str_from_raw(path, path_len) }) else {
        return 0;
    };
    catch_unwind(AssertUnwindSafe(|| {
        app.pool
            .references(std::path::Path::new(path), line, character)
    }))
    .unwrap_or(0)
}

/// Requests a workspace-wide rename of the symbol at an LSP position to
/// `new_name` (`new_name_len` bytes of UTF-8); same contract as
/// [`tc_lsp_hover`]. The response's `result` is an LSP `WorkspaceEdit`.
///
/// # Safety
/// Same contract as [`tc_lsp_did_open`]; `new_name` must point to
/// `new_name_len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn tc_lsp_rename(
    app: *mut TcApp,
    path: *const c_char,
    path_len: usize,
    line: u32,
    character: u32,
    new_name: *const c_char,
    new_name_len: usize,
) -> u64 {
    let Some(app) = (unsafe { app.as_mut() }) else {
        return 0;
    };
    let (path, new_name) =
        unsafe { (str_from_raw(path, path_len), str_from_raw(new_name, new_name_len)) };
    let (Some(path), Some(new_name)) = (path, new_name) else {
        return 0;
    };
    catch_unwind(AssertUnwindSafe(|| {
        app.pool
            .rename(std::path::Path::new(path), line, character, new_name)
    }))
    .unwrap_or(0)
}

/// Requests whole-document formatting; same contract as
/// [`tc_lsp_hover`]. The response's `result` is an LSP `TextEdit[]`.
///
/// # Safety
/// Same contract as [`tc_lsp_did_open`].
#[no_mangle]
pub unsafe extern "C" fn tc_lsp_formatting(
    app: *mut TcApp,
    path: *const c_char,
    path_len: usize,
    tab_size: u32,
    insert_spaces: bool,
) -> u64 {
    let Some(app) = (unsafe { app.as_mut() }) else {
        return 0;
    };
    let Some(path) = (unsafe { str_from_raw(path, path_len) }) else {
        return 0;
    };
    catch_unwind(AssertUnwindSafe(|| {
        app.pool
            .formatting(std::path::Path::new(path), tab_size, insert_spaces)
    }))
    .unwrap_or(0)
}

/// Requests completions at an LSP position; same contract as
/// [`tc_lsp_hover`]. The response's `result` is an LSP
/// `CompletionItem[]` or `CompletionList`.
///
/// # Safety
/// Same contract as [`tc_lsp_did_open`].
#[no_mangle]
pub unsafe extern "C" fn tc_lsp_completion(
    app: *mut TcApp,
    path: *const c_char,
    path_len: usize,
    line: u32,
    character: u32,
) -> u64 {
    let Some(app) = (unsafe { app.as_mut() }) else {
        return 0;
    };
    let Some(path) = (unsafe { str_from_raw(path, path_len) }) else {
        return 0;
    };
    catch_unwind(AssertUnwindSafe(|| {
        app.pool
            .completion(std::path::Path::new(path), line, character)
    }))
    .unwrap_or(0)
}

/// Announces a closed document. The server instance stays warm.
///
/// # Safety
/// Same contract as [`tc_lsp_did_open`].
#[no_mangle]
pub unsafe extern "C" fn tc_lsp_did_close(app: *mut TcApp, path: *const c_char, path_len: usize) {
    let Some(app) = (unsafe { app.as_mut() }) else {
        return;
    };
    let Some(path) = (unsafe { str_from_raw(path, path_len) }) else {
        return;
    };
    let _ = catch_unwind(AssertUnwindSafe(|| {
        app.pool.did_close(std::path::Path::new(path));
    }));
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

/// A span of a document in UTF-16 code units, as a selection to make.
#[repr(C)]
pub struct TcRegion {
    pub start: usize,
    pub end: usize,
}

/// Expands an LSP snippet body (`frob(${1:x}, ${2:y})$0`) for an
/// insertion at UTF-16 offset `at`, returning the text to insert as a
/// nul-terminated UTF-8 string; release it with [`tc_string_free`].
/// Nothing is inserted: the shell puts the text in through the same path
/// as anything typed, so its display copy and the core stay in step, and
/// then calls [`tc_document_snippet_begin`] with where it landed.
///
/// # Safety
/// `document` must be a live document pointer; `body` must point to
/// `len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn tc_document_snippet_expand(
    document: *mut TcDocument,
    at: usize,
    body: *const c_char,
    len: usize,
) -> *mut c_char {
    let Some(document) = (unsafe { document.as_mut() }) else {
        return std::ptr::null_mut();
    };
    let Some(body) = (unsafe { str_from_raw(body, len) }) else {
        return std::ptr::null_mut();
    };
    catch_unwind(AssertUnwindSafe(|| {
        owned_c_string(document.inner.expand_snippet(at, body))
    }))
    .unwrap_or(std::ptr::null_mut())
}

/// Starts a tabstop session over the text
/// [`tc_document_snippet_expand`] returned, now sitting at `origin`.
/// Writes the range to select into `region_out`: the first placeholder,
/// or where the caret goes when there is nothing to walk — check
/// [`tc_document_snippet_active`] to tell those apart. Returns false when
/// no expansion is pending.
///
/// # Safety
/// `document` must be a live document pointer; `region_out` must point
/// to a writable slot.
#[no_mangle]
pub unsafe extern "C" fn tc_document_snippet_begin(
    document: *mut TcDocument,
    origin: usize,
    region_out: *mut TcRegion,
) -> bool {
    if region_out.is_null() {
        return false;
    }
    let Some(document) = (unsafe { document.as_mut() }) else {
        return false;
    };
    catch_unwind(AssertUnwindSafe(
        || match document.inner.begin_snippet(origin) {
            Some(region) => {
                unsafe {
                    *region_out = TcRegion {
                        start: region.start,
                        end: region.end,
                    }
                };
                true
            }
            None => false,
        },
    ))
    .unwrap_or(false)
}

/// Whether a snippet is being filled in, and Tab therefore belongs to it
/// rather than to the text view.
///
/// # Safety
/// `document` must be a live document pointer.
#[no_mangle]
pub unsafe extern "C" fn tc_document_snippet_active(document: *const TcDocument) -> bool {
    unsafe { document.as_ref() }.is_some_and(|d| d.inner.snippet_active())
}

/// Moves to the next tabstop, or back to the previous one when
/// `forward` is false, and writes the range to select into
/// `region_out`. Returns false when no session is running.
///
/// Reaching `$0` or running off the end ends the session: the call still
/// returns the caret position, and [`tc_document_snippet_active`] is
/// false afterwards.
///
/// # Safety
/// `document` must be a live document pointer; `region_out` must point
/// to a writable slot.
#[no_mangle]
pub unsafe extern "C" fn tc_document_snippet_advance(
    document: *mut TcDocument,
    forward: bool,
    region_out: *mut TcRegion,
) -> bool {
    if region_out.is_null() {
        return false;
    }
    let Some(document) = (unsafe { document.as_mut() }) else {
        return false;
    };
    catch_unwind(AssertUnwindSafe(
        || match document.inner.snippet_advance(forward) {
            Some(region) => {
                unsafe {
                    *region_out = TcRegion {
                        start: region.start,
                        end: region.end,
                    }
                };
                true
            }
            None => false,
        },
    ))
    .unwrap_or(false)
}

/// Tells the session where the caret went; a caret outside the snippet
/// ends it, so a click elsewhere gives Tab back.
///
/// # Safety
/// `document` must be a live document pointer.
#[no_mangle]
pub unsafe extern "C" fn tc_document_snippet_caret_moved(
    document: *mut TcDocument,
    position: usize,
) {
    if let Some(document) = unsafe { document.as_mut() } {
        document.inner.snippet_caret_moved(position);
    }
}

/// Ends the session, wherever it had got to.
///
/// # Safety
/// `document` must be a live document pointer.
#[no_mangle]
pub unsafe extern "C" fn tc_document_snippet_cancel(document: *mut TcDocument) {
    if let Some(document) = unsafe { document.as_mut() } {
        document.inner.cancel_snippet();
    }
}

/// Copies the tabstop just typed in to the other places carrying the
/// same number. Same contract as [`tc_document_undo`]: on success an
/// array of edits to replay on the display cache **in array order**,
/// released with [`tc_applied_edits_free`]. Returns false when there was
/// nothing to mirror, which is the common case — call it after every
/// edit made while a session is running.
///
/// # Safety
/// `document` must be a live document pointer; `edits_out` and
/// `count_out` must point to writable slots.
#[no_mangle]
pub unsafe extern "C" fn tc_document_snippet_sync(
    document: *mut TcDocument,
    edits_out: *mut *mut TcAppliedEdit,
    count_out: *mut usize,
) -> bool {
    unsafe { pop_history(document, edits_out, count_out, |d| d.snippet_sync()) }
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

/// The document rendered as an HTML fragment for the live preview, or
/// null unless the document's language is markdown. Release with
/// [`tc_string_free`].
///
/// # Safety
/// `document` must be a live document pointer.
#[no_mangle]
pub unsafe extern "C" fn tc_document_markdown_html(document: *const TcDocument) -> *mut c_char {
    let Some(document) = (unsafe { document.as_ref() }) else {
        return std::ptr::null_mut();
    };
    catch_unwind(AssertUnwindSafe(|| {
        match document.inner.markdown_html() {
            Some(html) => owned_c_string(html),
            None => std::ptr::null_mut(),
        }
    }))
    .unwrap_or(std::ptr::null_mut())
}

/// The UTF-16 bounds of the innermost multi-line syntax block containing
/// `position` — the caret's enclosing block, for go-to-block navigation.
/// Returns false (outputs untouched) for plain text or positions outside
/// any block.
///
/// # Safety
/// `document` must be a live document pointer; `start_out` and `end_out`
/// must point to writable slots.
#[no_mangle]
pub unsafe extern "C" fn tc_document_block_bounds(
    document: *const TcDocument,
    position: usize,
    start_out: *mut usize,
    end_out: *mut usize,
) -> bool {
    let Some(document) = (unsafe { document.as_ref() }) else {
        return false;
    };
    if start_out.is_null() || end_out.is_null() {
        return false;
    }
    catch_unwind(AssertUnwindSafe(|| {
        match document.inner.block_bounds(position) {
            Some((start, end)) => {
                unsafe {
                    *start_out = start;
                    *end_out = end;
                }
                true
            }
            None => false,
        }
    }))
    .unwrap_or(false)
}

/// Whether the editor shows a line-number gutter (default true).
///
/// # Safety
/// `config` must be a live configuration pointer.
#[no_mangle]
pub unsafe extern "C" fn tc_config_line_numbers(config: *const TcConfig) -> bool {
    unsafe { config.as_ref() }.map_or(true, |c| c.inner.line_numbers())
}

/// Sets whether the editor shows a line-number gutter.
///
/// # Safety
/// `config` must be a live configuration pointer.
#[no_mangle]
pub unsafe extern "C" fn tc_config_set_line_numbers(config: *mut TcConfig, shown: bool) {
    if let Some(config) = unsafe { config.as_mut() } {
        config.inner.set_line_numbers(shown);
    }
}

/// The keyboard-shortcut overrides (`keys` section), serialized as an
/// object of `{action: "modifiers+key"}` entries; `{}` when unset.
/// Release with [`tc_string_free`].
///
/// # Safety
/// `config` must be a live configuration pointer.
#[no_mangle]
pub unsafe extern "C" fn tc_config_keys_json(config: *const TcConfig) -> *mut c_char {
    let Some(config) = (unsafe { config.as_ref() }) else {
        return std::ptr::null_mut();
    };
    owned_c_string(config.inner.keys_json())
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

/// Appearance choice: follow the system.
pub const TC_APPEARANCE_SYSTEM: u32 = 0;
/// Appearance choice: always light.
pub const TC_APPEARANCE_LIGHT: u32 = 1;
/// Appearance choice: always dark.
pub const TC_APPEARANCE_DARK: u32 = 2;

/// The configured appearance, as a `TC_APPEARANCE_*` value.
///
/// # Safety
/// `config` must be a live configuration pointer.
#[no_mangle]
pub unsafe extern "C" fn tc_config_appearance(config: *const TcConfig) -> u32 {
    use textchum_core::Appearance;
    match unsafe { config.as_ref() }.map(|c| c.inner.appearance()) {
        Some(Appearance::Light) => TC_APPEARANCE_LIGHT,
        Some(Appearance::Dark) => TC_APPEARANCE_DARK,
        _ => TC_APPEARANCE_SYSTEM,
    }
}

/// Sets the appearance choice (`TC_APPEARANCE_*`; unknown values mean
/// system).
///
/// # Safety
/// `config` must be a live configuration pointer.
#[no_mangle]
pub unsafe extern "C" fn tc_config_set_appearance(config: *mut TcConfig, appearance: u32) {
    use textchum_core::Appearance;
    if let Some(config) = unsafe { config.as_mut() } {
        config.inner.set_appearance(match appearance {
            TC_APPEARANCE_LIGHT => Appearance::Light,
            TC_APPEARANCE_DARK => Appearance::Dark,
            _ => Appearance::System,
        });
    }
}

/// Whether selecting a word marks its other occurrences on screen
/// (`editor.mark_occurrences`, default true).
///
/// # Safety
/// `config` must be a live handle.
#[no_mangle]
pub unsafe extern "C" fn tc_config_mark_occurrences(config: *const TcConfig) -> bool {
    let Some(config) = (unsafe { config.as_ref() }) else {
        return true;
    };
    catch_unwind(AssertUnwindSafe(|| config.inner.mark_occurrences())).unwrap_or(true)
}

/// # Safety
/// `config` must be a live handle.
#[no_mangle]
pub unsafe extern "C" fn tc_config_set_mark_occurrences(config: *mut TcConfig, enabled: bool) {
    let Some(config) = (unsafe { config.as_mut() }) else {
        return;
    };
    let _ = catch_unwind(AssertUnwindSafe(|| {
        config.inner.set_mark_occurrences(enabled)
    }));
}

/// Whether occurrence marking tells `Item` from `item`
/// (`editor.occurrences_case_sensitive`, default true).
///
/// # Safety
/// `config` must be a live handle.
#[no_mangle]
pub unsafe extern "C" fn tc_config_occurrences_case_sensitive(config: *const TcConfig) -> bool {
    let Some(config) = (unsafe { config.as_ref() }) else {
        return true;
    };
    catch_unwind(AssertUnwindSafe(|| {
        config.inner.occurrence_options().case_sensitive
    }))
    .unwrap_or(true)
}

/// # Safety
/// `config` must be a live handle.
#[no_mangle]
pub unsafe extern "C" fn tc_config_set_occurrences_case_sensitive(
    config: *mut TcConfig,
    enabled: bool,
) {
    let Some(config) = (unsafe { config.as_mut() }) else {
        return;
    };
    let _ = catch_unwind(AssertUnwindSafe(|| {
        config.inner.set_occurrences_case_sensitive(enabled)
    }));
}

/// Whether `item` inside `items` counts as an occurrence
/// (`editor.occurrences_whole_word`, default true — so it does not).
///
/// # Safety
/// `config` must be a live handle.
#[no_mangle]
pub unsafe extern "C" fn tc_config_occurrences_whole_word(config: *const TcConfig) -> bool {
    let Some(config) = (unsafe { config.as_ref() }) else {
        return true;
    };
    catch_unwind(AssertUnwindSafe(|| {
        config.inner.occurrence_options().whole_word
    }))
    .unwrap_or(true)
}

/// # Safety
/// `config` must be a live handle.
#[no_mangle]
pub unsafe extern "C" fn tc_config_set_occurrences_whole_word(config: *mut TcConfig, enabled: bool) {
    let Some(config) = (unsafe { config.as_mut() }) else {
        return;
    };
    let _ = catch_unwind(AssertUnwindSafe(|| {
        config.inner.set_occurrences_whole_word(enabled)
    }));
}

/// Whether hover documentation pops up on mouse rest
/// (`editor.hover`, default true).
///
/// # Safety
/// `config` must be a live configuration pointer.
#[no_mangle]
pub unsafe extern "C" fn tc_config_hover_docs(config: *const TcConfig) -> bool {
    let Some(config) = (unsafe { config.as_ref() }) else {
        return true;
    };
    catch_unwind(AssertUnwindSafe(|| config.inner.hover_docs())).unwrap_or(true)
}

/// Sets the hover-documentation choice.
///
/// # Safety
/// `config` must be a live configuration pointer.
#[no_mangle]
pub unsafe extern "C" fn tc_config_set_hover_docs(config: *mut TcConfig, enabled: bool) {
    let Some(config) = (unsafe { config.as_mut() }) else {
        return;
    };
    let _ = catch_unwind(AssertUnwindSafe(|| config.inner.set_hover_docs(enabled)));
}

/// Re-reads the configuration file, replacing in-memory state — for
/// following external edits while running. Returns a human-readable
/// warning (release with [`tc_string_free`]) or null when the file was
/// usable.
///
/// # Safety
/// `config` must be a live configuration pointer.
#[no_mangle]
pub unsafe extern "C" fn tc_config_reload(config: *mut TcConfig) -> *mut c_char {
    let Some(config) = (unsafe { config.as_mut() }) else {
        return std::ptr::null_mut();
    };
    match catch_unwind(AssertUnwindSafe(|| config.inner.reload())) {
        Ok(Some(warning)) => owned_c_string(warning),
        _ => std::ptr::null_mut(),
    }
}

/// Per-project editor overrides for a root, serialized (`{}` when
/// none): any of `font_family`, `font_size`, `tab_width`. Release with
/// [`tc_string_free`].
///
/// # Safety
/// `config` must be a live configuration pointer; the pointer/length
/// pair must describe readable bytes.
#[no_mangle]
pub unsafe extern "C" fn tc_config_editor_overrides(
    config: *const TcConfig,
    root: *const c_char,
    root_len: usize,
) -> *mut c_char {
    let Some(config) = (unsafe { config.as_ref() }) else {
        return std::ptr::null_mut();
    };
    let Some(root) = (unsafe { str_from_raw(root, root_len) }) else {
        return std::ptr::null_mut();
    };
    owned_c_string(config.inner.editor_overrides_json(root))
}

/// Sets (or removes, with `value_len == 0`) one per-project editor
/// override. `value` is a JSON value — `13.5`, `"Menlo"`.
///
/// # Safety
/// `config` must be a live configuration pointer; each pointer/length
/// pair must describe readable bytes.
#[no_mangle]
pub unsafe extern "C" fn tc_config_set_editor_override(
    config: *mut TcConfig,
    root: *const c_char,
    root_len: usize,
    key: *const c_char,
    key_len: usize,
    value: *const c_char,
    value_len: usize,
) {
    let Some(config) = (unsafe { config.as_mut() }) else {
        return;
    };
    let (root, key, value) = unsafe {
        (
            str_from_raw(root, root_len),
            str_from_raw(key, key_len),
            str_from_raw(value, value_len),
        )
    };
    let (Some(root), Some(key), Some(value)) = (root, key, value) else {
        return;
    };
    config
        .inner
        .set_editor_override(root, key, (!value.is_empty()).then_some(value));
}

/// Whether hover documentation pops on mouse rest (default true).
///
/// # Safety
/// `config` must be a live configuration pointer.
#[no_mangle]
pub unsafe extern "C" fn tc_config_theme(config: *const TcConfig) -> *mut c_char {
    let Some(config) = (unsafe { config.as_ref() }) else {
        return std::ptr::null_mut();
    };
    catch_unwind(AssertUnwindSafe(|| owned_c_string(config.inner.theme())))
        .unwrap_or(std::ptr::null_mut())
}

/// Sets the theme choice (`len` bytes of UTF-8; the default theme's name
/// removes the key).
///
/// # Safety
/// `config` must be a live configuration pointer; `name` must point to
/// `len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn tc_config_set_theme(
    config: *mut TcConfig,
    name: *const c_char,
    len: usize,
) {
    let Some(config) = (unsafe { config.as_mut() }) else {
        return;
    };
    let Some(name) = (unsafe { str_from_raw(name, len) }) else {
        return;
    };
    let _ = catch_unwind(AssertUnwindSafe(|| config.inner.set_theme(name)));
}

/// The configured file-icon pack, as a path to a VS Code icon theme
/// JSON or the extension folder holding one. Null means the desktop's
/// own icons; release a non-null result with [`tc_string_free`].
///
/// # Safety
/// `config` must be a live configuration pointer.
#[no_mangle]
pub unsafe extern "C" fn tc_config_icon_pack(config: *const TcConfig) -> *mut c_char {
    let Some(config) = (unsafe { config.as_ref() }) else {
        return std::ptr::null_mut();
    };
    catch_unwind(AssertUnwindSafe(|| match config.inner.icon_pack() {
        Some(path) => owned_c_string(path),
        None => std::ptr::null_mut(),
    }))
    .unwrap_or(std::ptr::null_mut())
}

/// Sets the file-icon pack (`len` bytes of UTF-8; an empty path, or a
/// null one, removes the key).
///
/// # Safety
/// `config` must be a live configuration pointer; `path`, if not null,
/// must point to `len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn tc_config_set_icon_pack(
    config: *mut TcConfig,
    path: *const c_char,
    len: usize,
) {
    let Some(config) = (unsafe { config.as_mut() }) else {
        return;
    };
    let path = if path.is_null() {
        None
    } else {
        unsafe { str_from_raw(path, len) }
    };
    let _ = catch_unwind(AssertUnwindSafe(|| config.inner.set_icon_pack(path)));
}

/// The project root for a file or directory path (`len` bytes of UTF-8),
/// resolved under the workspace settings passed as JSON (`settings_len`
/// bytes; may be empty for defaults — the configuration's `workspace`
/// section). Returns null for loose files outside any project; release
/// non-null results with [`tc_string_free`].
///
/// # Safety
/// `path` and `settings` must point to their stated numbers of readable
/// bytes.
#[no_mangle]
pub unsafe extern "C" fn tc_project_root_for_path(
    path: *const c_char,
    len: usize,
    settings: *const c_char,
    settings_len: usize,
) -> *mut c_char {
    let (path, settings) =
        unsafe { (str_from_raw(path, len), str_from_raw(settings, settings_len)) };
    let (Some(path), Some(settings_json)) = (path, settings) else {
        return std::ptr::null_mut();
    };
    catch_unwind(AssertUnwindSafe(|| {
        let settings = textchum_core::workspace::WorkspaceSettings::from_json(settings_json);
        match textchum_core::workspace::project_root_with(std::path::Path::new(path), &settings)
        {
            Some(root) => owned_c_string(root.to_string_lossy().into_owned()),
            None => std::ptr::null_mut(),
        }
    }))
    .unwrap_or(std::ptr::null_mut())
}

/// A boolean workspace flag for a project root, resolved with the
/// standard rules (the root's own entry, else the top-level default,
/// else false) against the workspace settings JSON. Shell-owned flags
/// (like the ctags fallback) resolve here so the semantics stay in one
/// place.
///
/// # Safety
/// All pointer/length pairs must describe readable bytes.
#[no_mangle]
pub unsafe extern "C" fn tc_workspace_flag(
    settings: *const c_char,
    settings_len: usize,
    root: *const c_char,
    root_len: usize,
    key: *const c_char,
    key_len: usize,
) -> bool {
    let (settings, root, key) = unsafe {
        (
            str_from_raw(settings, settings_len),
            str_from_raw(root, root_len),
            str_from_raw(key, key_len),
        )
    };
    let (Some(settings_json), Some(root), Some(key)) = (settings, root, key) else {
        return false;
    };
    catch_unwind(AssertUnwindSafe(|| {
        textchum_core::workspace::WorkspaceSettings::from_json(settings_json)
            .flag(std::path::Path::new(root), key)
    }))
    .unwrap_or(false)
}

/// The configuration's `workspace` section, serialized (`{}` when
/// unset). Release with [`tc_string_free`]. Feed it to
/// [`tc_project_root_for_path`] and, combined with the `lsp` section, to
/// [`tc_lsp_configure`].
///
/// # Safety
/// `config` must be a live configuration pointer.
#[no_mangle]
pub unsafe extern "C" fn tc_config_workspace_json(config: *const TcConfig) -> *mut c_char {
    let Some(config) = (unsafe { config.as_ref() }) else {
        return std::ptr::null_mut();
    };
    owned_c_string(config.inner.workspace_json())
}

/// Sets (or removes, when `has_value` is false) a workspace flag —
/// `manifest_projects` or `recursive_config` — for a project root
/// (`root_len > 0`) or the defaults.
///
/// # Safety
/// `config` must be a live configuration pointer; each pointer/length
/// pair must describe readable bytes.
#[no_mangle]
pub unsafe extern "C" fn tc_config_set_workspace_flag(
    config: *mut TcConfig,
    root: *const c_char,
    root_len: usize,
    key: *const c_char,
    key_len: usize,
    has_value: bool,
    value: bool,
) {
    let Some(config) = (unsafe { config.as_mut() }) else {
        return;
    };
    let (root, key) = unsafe { (str_from_raw(root, root_len), str_from_raw(key, key_len)) };
    let (Some(root), Some(key)) = (root, key) else {
        return;
    };
    config.inner.set_workspace_flag(
        (!root.is_empty()).then_some(root),
        key,
        has_value.then_some(value),
    );
}

/// Every project root the configuration mentions, in any section, as a
/// nul-terminated JSON array of strings. Release with
/// [`tc_string_free`].
///
/// # Safety
/// `config` must be a live configuration pointer.
#[no_mangle]
pub unsafe extern "C" fn tc_config_configured_projects(config: *const TcConfig) -> *mut c_char {
    let Some(config) = (unsafe { config.as_ref() }) else {
        return std::ptr::null_mut();
    };
    catch_unwind(AssertUnwindSafe(|| {
        let roots = config.inner.configured_projects();
        owned_c_string(serde_json::to_string(&roots).unwrap_or_else(|_| "[]".into()))
    }))
    .unwrap_or(std::ptr::null_mut())
}

/// Removes every trace of a project root: flags, editor overrides,
/// hidden globs, servers and save commands.
///
/// # Safety
/// `config` must be a live configuration pointer; `root` must point to
/// `root_len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn tc_config_remove_project(
    config: *mut TcConfig,
    root: *const c_char,
    root_len: usize,
) {
    let Some(config) = (unsafe { config.as_mut() }) else {
        return;
    };
    let Some(root) = (unsafe { str_from_raw(root, root_len) }) else {
        return;
    };
    let _ = catch_unwind(AssertUnwindSafe(|| config.inner.remove_project(root)));
}

/// Copies one project's settings onto another root, taking the parts
/// asked for: `workspace` is the flags, editor overrides and hidden
/// globs; `servers` is the language servers; `preprocessors` is the
/// save commands. Each part replaces the target's.
///
/// Returns whether anything was copied — a source with no settings of
/// its own copies nothing, and neither does a root onto itself.
///
/// # Safety
/// `config` must be a live configuration pointer; each pointer/length
/// pair must describe readable bytes.
#[no_mangle]
pub unsafe extern "C" fn tc_config_copy_project(
    config: *mut TcConfig,
    from: *const c_char,
    from_len: usize,
    to: *const c_char,
    to_len: usize,
    workspace: bool,
    servers: bool,
    preprocessors: bool,
) -> bool {
    let Some(config) = (unsafe { config.as_mut() }) else {
        return false;
    };
    let (from, to) = unsafe { (str_from_raw(from, from_len), str_from_raw(to, to_len)) };
    let (Some(from), Some(to)) = (from, to) else {
        return false;
    };
    catch_unwind(AssertUnwindSafe(|| {
        config.inner.copy_project(
            from,
            to,
            textchum_core::ProjectParts {
                workspace,
                servers,
                preprocessors,
            },
        )
    }))
    .unwrap_or(false)
}

/// Open-target choice: files open as tabs of the current window group.
pub const TC_OPEN_IN_TAB: u32 = 0;
/// Open-target choice: files open as separate windows.
pub const TC_OPEN_IN_WINDOW: u32 = 1;

/// Where opened files go, as a `TC_OPEN_IN_*` value.
///
/// # Safety
/// `config` must be a live configuration pointer.
#[no_mangle]
pub unsafe extern "C" fn tc_config_open_target(config: *const TcConfig) -> u32 {
    use textchum_core::OpenTarget;
    match unsafe { config.as_ref() }.map(|c| c.inner.open_target()) {
        Some(OpenTarget::Window) => TC_OPEN_IN_WINDOW,
        _ => TC_OPEN_IN_TAB,
    }
}

/// Sets the open-target choice (`TC_OPEN_IN_*`; unknown values mean tab).
///
/// # Safety
/// `config` must be a live configuration pointer.
#[no_mangle]
pub unsafe extern "C" fn tc_config_set_open_target(config: *mut TcConfig, target: u32) {
    use textchum_core::OpenTarget;
    if let Some(config) = unsafe { config.as_mut() } {
        config.inner.set_open_target(match target {
            TC_OPEN_IN_WINDOW => OpenTarget::Window,
            _ => OpenTarget::Tab,
        });
    }
}

/// Where File → New places the fresh document (`TC_OPEN_IN_*`; tab is
/// the default).
///
/// # Safety
/// `config` must be a live configuration pointer.
#[no_mangle]
pub unsafe extern "C" fn tc_config_new_file_target(config: *const TcConfig) -> u32 {
    use textchum_core::OpenTarget;
    match unsafe { config.as_ref() }.map(|c| c.inner.new_file_target()) {
        Some(OpenTarget::Window) => TC_OPEN_IN_WINDOW,
        _ => TC_OPEN_IN_TAB,
    }
}

/// Sets the new-file placement (`TC_OPEN_IN_*`; unknown values mean tab).
///
/// # Safety
/// `config` must be a live configuration pointer.
#[no_mangle]
pub unsafe extern "C" fn tc_config_set_new_file_target(config: *mut TcConfig, target: u32) {
    use textchum_core::OpenTarget;
    if let Some(config) = unsafe { config.as_mut() } {
        config.inner.set_new_file_target(match target {
            TC_OPEN_IN_WINDOW => OpenTarget::Window,
            _ => OpenTarget::Tab,
        });
    }
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

/// One styled span, in UTF-16 code units. `style` indexes the table from
/// [`tc_style_table`].
#[repr(C)]
pub struct TcHighlightSpan {
    pub start: usize,
    pub end: usize,
    pub style: u32,
}

/// One style of the theme. Colors are 0xRRGGBBAA for the light and dark
/// appearances; `flags` uses [`TC_STYLE_BOLD`]/[`TC_STYLE_ITALIC`].
#[repr(C)]
pub struct TcStyle {
    pub light: u32,
    pub dark: u32,
    pub flags: u32,
}

/// Style flag: render bold.
pub const TC_STYLE_BOLD: u32 = 1;
/// Style flag: render italic.
pub const TC_STYLE_ITALIC: u32 = 2;

/// The current style table pointer. Tables are leaked on purpose: a
/// handful of bytes per theme switch buys pointers that stay valid for
/// the process lifetime, so shells can hold one across redraws.
static STYLE_TABLE: std::sync::RwLock<Option<&'static [TcStyle]>> =
    std::sync::RwLock::new(None);

fn refresh_style_table() {
    let table: Vec<TcStyle> = textchum_core::theme::styles()
        .into_iter()
        .map(|style| TcStyle {
            light: style.light,
            dark: style.dark,
            flags: style.flags,
        })
        .collect();
    if let Ok(mut current) = STYLE_TABLE.write() {
        *current = Some(Box::leak(table.into_boxed_slice()));
    }
}

/// The style id a capture name resolves to, or -1 when the capture is
/// unstyled. Shells need this to ask questions about what a span *is* —
/// "is this a comment?" for the prose spell checker — without knowing
/// where `comment` sits in the table. It moves whenever a capture is
/// added, so nothing may hardcode it.
///
/// # Safety
/// The pointer/length pair must describe readable bytes.
#[no_mangle]
pub unsafe extern "C" fn tc_theme_style_id(capture: *const c_char, len: usize) -> i32 {
    let Some(capture) = (unsafe { str_from_raw(capture, len) }) else {
        return -1;
    };
    textchum_core::theme::resolve(capture).map_or(-1, |id| id as i32)
}

/// The active theme's style table, indexed by the `style` of a highlight
/// span. Owned by the core and valid for the process lifetime; do not
/// free. Superseded (but not invalidated) by `tc_theme_set_*` — re-fetch
/// after switching themes.
///
/// # Safety
/// `count_out` must point to a writable slot.
#[no_mangle]
pub unsafe extern "C" fn tc_style_table(count_out: *mut usize) -> *const TcStyle {
    if STYLE_TABLE.read().map(|t| t.is_none()).unwrap_or(true) {
        refresh_style_table();
    }
    let table = STYLE_TABLE
        .read()
        .ok()
        .and_then(|current| *current)
        .unwrap_or(&[]);
    if !count_out.is_null() {
        unsafe { *count_out = table.len() };
    }
    table.as_ptr()
}

/// Built-in theme names, newline-joined, in presentation order. Release
/// with [`tc_string_free`].
#[no_mangle]
pub extern "C" fn tc_theme_builtin_names() -> *mut c_char {
    owned_c_string(
        textchum_core::theme::builtin_names()
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

/// Activates a built-in theme by name; false for unknown names. Re-fetch
/// [`tc_style_table`] and redraw afterwards.
///
/// # Safety
/// `name` must point to `len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn tc_theme_set_builtin(name: *const c_char, len: usize) -> bool {
    let Some(name) = (unsafe { str_from_raw(name, len) }) else {
        return false;
    };
    catch_unwind(AssertUnwindSafe(|| {
        match textchum_core::theme::Theme::builtin(name) {
            Some(theme) => {
                textchum_core::theme::set_active(theme);
                refresh_style_table();
                true
            }
            None => false,
        }
    }))
    .unwrap_or(false)
}

/// Activates a user theme from its JSON. On failure returns false, sets
/// `*error_out` (release with [`tc_string_free`]), and leaves the active
/// theme unchanged — same escape-hatch spirit as the configuration.
///
/// # Safety
/// `json` must point to `len` readable bytes; `error_out` may be null.
#[no_mangle]
pub unsafe extern "C" fn tc_theme_set_json(
    json: *const c_char,
    len: usize,
    error_out: *mut *mut c_char,
) -> bool {
    if !error_out.is_null() {
        unsafe { *error_out = std::ptr::null_mut() };
    }
    let Some(json) = (unsafe { str_from_raw(json, len) }) else {
        return false;
    };
    catch_unwind(AssertUnwindSafe(|| {
        match textchum_core::theme::Theme::from_json(json) {
            Ok(theme) => {
                textchum_core::theme::set_active(theme);
                refresh_style_table();
                true
            }
            Err(error) => {
                unsafe { write_error(error_out, &error) };
                false
            }
        }
    }))
    .unwrap_or(false)
}

/// A complete starter theme (every styled capture, default palette) as
/// pretty-printed JSON — what `--emit-theme` writes. Release with
/// [`tc_string_free`].
#[no_mangle]
pub extern "C" fn tc_theme_template_json() -> *mut c_char {
    owned_c_string(textchum_core::theme::Theme::template_json())
}

/// Loads the file-icon pack at `path` — a VS Code icon theme JSON, or
/// the extension folder holding one — and makes it the one
/// [`tc_icons_for_file`] answers from. On success returns a
/// nul-terminated line describing what was loaded; on failure returns
/// null and fills the optional `error_out`. Release either with
/// [`tc_string_free`].
///
/// # Safety
/// `path` must point to `len` readable bytes; `error_out`, if given,
/// must point to a writable slot.
#[no_mangle]
pub unsafe extern "C" fn tc_icons_load(
    path: *const c_char,
    len: usize,
    error_out: *mut *mut c_char,
) -> *mut c_char {
    if !error_out.is_null() {
        unsafe { *error_out = std::ptr::null_mut() };
    }
    let Some(path) = (unsafe { str_from_raw(path, len) }) else {
        unsafe { write_error(error_out, "invalid path string") };
        return std::ptr::null_mut();
    };
    catch_unwind(AssertUnwindSafe(|| {
        match textchum_core::icons::set_active_from(std::path::Path::new(path)) {
            Ok(summary) => owned_c_string(summary),
            Err(error) => {
                unsafe { write_error(error_out, &error) };
                std::ptr::null_mut()
            }
        }
    }))
    .unwrap_or(std::ptr::null_mut())
}

/// Forgets the icon pack, returning the file tree to the desktop's own
/// icons.
#[no_mangle]
pub extern "C" fn tc_icons_clear() {
    textchum_core::icons::clear_active();
}

/// Whether an icon pack is loaded, so a shell can skip asking about
/// every row when none is.
#[no_mangle]
pub extern "C" fn tc_icons_active() -> bool {
    textchum_core::icons::is_active()
}

/// The icon for `filename` from the loaded pack, as a nul-terminated
/// path to an image; null when no pack is loaded or it has nothing for
/// this file. `language` may be null, and is what the editor decided
/// the file is — which catches files a pack lists by language, and the
/// ones a reader named through File Properties. `light` picks the
/// pack's light overrides. Release with [`tc_string_free`].
///
/// # Safety
/// `filename` must point to `filename_len` readable bytes; `language`,
/// if not null, to `language_len`.
#[no_mangle]
pub unsafe extern "C" fn tc_icons_for_file(
    filename: *const c_char,
    filename_len: usize,
    language: *const c_char,
    language_len: usize,
    light: bool,
) -> *mut c_char {
    let Some(filename) = (unsafe { str_from_raw(filename, filename_len) }) else {
        return std::ptr::null_mut();
    };
    let language = if language.is_null() {
        None
    } else {
        unsafe { str_from_raw(language, language_len) }
    };
    catch_unwind(AssertUnwindSafe(|| {
        match textchum_core::icons::icon_for(filename, language, light) {
            Some(path) => owned_c_string(path.to_string_lossy().into_owned()),
            None => std::ptr::null_mut(),
        }
    }))
    .unwrap_or(std::ptr::null_mut())
}

/// Imports every theme at `path` into `themes_dir`, one JSON file per
/// theme, named after the theme itself. `source` is 0 for VS Code and 1
/// for TextMate.
///
/// `path` is a theme file or a folder to look inside — a VS Code
/// extension directory, or a `.tmbundle`. Returns the outcome as a
/// nul-terminated JSON object, released with [`tc_string_free`]:
///
/// ```json
/// {"written": ["Night"], "appearances": ["dark"],
///  "unmapped": ["meta.brace.round"], "errors": []}
/// ```
///
/// `written` names the themes now available to choose, `appearances`
/// says which side of the palette each one filled (the other keeps the
/// default palette's colours), `unmapped` lists scopes no capture
/// answers to, and `errors` carries one line per file that could not be
/// read. A null return means the arguments were unreadable.
///
/// # Safety
/// `path` must point to `path_len` readable bytes and `themes_dir` to
/// `dir_len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn tc_theme_import(
    path: *const c_char,
    path_len: usize,
    source: u32,
    themes_dir: *const c_char,
    dir_len: usize,
) -> *mut c_char {
    let Some(path) = (unsafe { str_from_raw(path, path_len) }) else {
        return std::ptr::null_mut();
    };
    let Some(themes_dir) = (unsafe { str_from_raw(themes_dir, dir_len) }) else {
        return std::ptr::null_mut();
    };
    use textchum_core::theme_import::{import_into, Source};
    let source = if source == 1 { Source::TextMate } else { Source::VsCode };
    catch_unwind(AssertUnwindSafe(|| {
        owned_c_string(
            import_into(
                std::path::Path::new(path),
                source,
                std::path::Path::new(themes_dir),
            )
            .to_json(),
        )
    }))
    .unwrap_or(std::ptr::null_mut())
}

/// Reads a place out of whatever was typed or pasted into a "go to
/// line" prompt: `412`, `412:8`, `src/main.rs:412:8`, `line 412`.
/// Writes the one-based line and column into the outputs and returns
/// true; returns false when the text names no line at all.
///
/// # Safety
/// `text` must point to `len` readable bytes; `line_out` and
/// `column_out` must point to writable slots.
#[no_mangle]
pub unsafe extern "C" fn tc_goto_parse(
    text: *const c_char,
    len: usize,
    line_out: *mut usize,
    column_out: *mut usize,
) -> bool {
    if line_out.is_null() || column_out.is_null() {
        return false;
    }
    let Some(text) = (unsafe { str_from_raw(text, len) }) else {
        return false;
    };
    catch_unwind(AssertUnwindSafe(
        || match textchum_core::goto::parse(text) {
            Some(target) => {
                unsafe {
                    *line_out = target.line;
                    *column_out = target.column;
                }
                true
            }
            None => false,
        },
    ))
    .unwrap_or(false)
}

/// The UTF-16 offset of a one-based line and column, clamped to the
/// document: a line past the end is the last line, and a column past
/// the end of its line is that line's end.
///
/// # Safety
/// `document` must be a live document pointer.
#[no_mangle]
pub unsafe extern "C" fn tc_document_offset_for_line(
    document: *const TcDocument,
    line: usize,
    column: usize,
) -> usize {
    unsafe { document.as_ref() }.map_or(0, |d| d.inner.offset_for_line(line, column))
}

/// How many lines the document has.
///
/// # Safety
/// `document` must be a live document pointer.
#[no_mangle]
pub unsafe extern "C" fn tc_document_len_lines(document: *const TcDocument) -> usize {
    unsafe { document.as_ref() }.map_or(0, |d| d.inner.len_lines())
}


/// Whether a path looks like a test, by the naming conventions of the
/// languages this editor knows: a `tests`/`spec`/`__tests__` directory,
/// or a name like `parser_test.go`, `test_parser.py`, `Button.test.ts`,
/// `ParserTests.swift`. Cautious on purpose — `latest.rs` is not one.
///
/// # Safety
/// `path` must point to `len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn tc_path_is_test(path: *const c_char, len: usize) -> bool {
    let Some(path) = (unsafe { str_from_raw(path, len) }) else {
        return false;
    };
    catch_unwind(AssertUnwindSafe(|| {
        textchum_core::references::is_test_path(path)
    }))
    .unwrap_or(false)
}

/// The other places the selected word appears, for marking them.
///
/// `text` is the stretch to search — the visible one, so a long file
/// costs what a short one does — and `base` is its UTF-16 offset in the
/// document. `selection_start` and `selection_end` are the selection,
/// also in UTF-16 units and also relative to `text`.
///
/// A selection that is not exactly one word answers with an empty
/// array: a partial word and a stretch spanning several were selected
/// for some other reason.
///
/// Returns a nul-terminated JSON array — `[{"start": 12, "end": 16},
/// …]`, in the document's UTF-16 offsets — released with
/// [`tc_string_free`].
///
/// # Safety
/// `text` must point to `text_len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn tc_occurrences_of_selection(
    text: *const c_char,
    text_len: usize,
    selection_start: usize,
    selection_end: usize,
    base: usize,
    case_sensitive: bool,
    whole_word: bool,
) -> *mut c_char {
    let Some(text) = (unsafe { str_from_raw(text, text_len) }) else {
        return std::ptr::null_mut();
    };
    catch_unwind(AssertUnwindSafe(|| {
        use textchum_core::occurrences;
        let Some(word) = occurrences::selected_word(text, selection_start, selection_end) else {
            return owned_c_string("[]".to_string());
        };
        let options = occurrences::Options {
            case_sensitive,
            whole_word,
        };
        let spans = occurrences::occurrences(text, &word, base, options);
        owned_c_string(occurrences::to_json(&spans))
    }))
    .unwrap_or(std::ptr::null_mut())
}

/// What to do with a `textDocument/definition` answer, given where the
/// caret is.
///
/// Jump to Definition has nowhere to go when the caret is already on
/// the definition, so the same key asks who uses the symbol instead.
/// The answer in hand decides it — no second request.
///
/// `result` is the response's `result` member as JSON. `line` and
/// `character` are the caret in LSP terms (zero-based, UTF-16 units).
///
/// Returns a nul-terminated JSON object, released with
/// [`tc_string_free`]:
///
/// ```json
/// {"action": "jump", "targets": [{"path": "/p/lib.rs", "line": 40,
///                                 "character": 3}]}
/// ```
///
/// `action` is `nothing`, `jump`, `references` (the caret is on the
/// definition) or `choose` (several, and the reader picks).
///
/// # Safety
/// `result` must point to `result_len` readable bytes and `path` to
/// `path_len`.
#[no_mangle]
pub unsafe extern "C" fn tc_definition_decide(
    result: *const c_char,
    result_len: usize,
    path: *const c_char,
    path_len: usize,
    line: u32,
    character: u32,
) -> *mut c_char {
    let Some(result) = (unsafe { str_from_raw(result, result_len) }) else {
        return std::ptr::null_mut();
    };
    let Some(path) = (unsafe { str_from_raw(path, path_len) }) else {
        return std::ptr::null_mut();
    };
    catch_unwind(AssertUnwindSafe(|| {
        let decision = textchum_core::definition::decide(result, path, line, character);
        owned_c_string(textchum_core::definition::to_json(&decision))
    }))
    .unwrap_or(std::ptr::null_mut())
}

/// The reference locations that are not the one the caret is in.
///
/// `textDocument/references` includes the declaration, so a definition
/// nobody calls answers with the line the caret is on. What is left
/// after dropping it is the uses.
///
/// Returns a nul-terminated JSON array of `{"path", "line",
/// "character"}`, released with [`tc_string_free`].
///
/// # Safety
/// `result` must point to `result_len` readable bytes and `path` to
/// `path_len`.
#[no_mangle]
pub unsafe extern "C" fn tc_references_elsewhere(
    result: *const c_char,
    result_len: usize,
    path: *const c_char,
    path_len: usize,
    line: u32,
    character: u32,
) -> *mut c_char {
    let Some(result) = (unsafe { str_from_raw(result, result_len) }) else {
        return std::ptr::null_mut();
    };
    let Some(path) = (unsafe { str_from_raw(path, path_len) }) else {
        return std::ptr::null_mut();
    };
    catch_unwind(AssertUnwindSafe(|| {
        let targets = textchum_core::definition::elsewhere(result, path, line, character);
        owned_c_string(textchum_core::definition::targets_to_json(&targets))
    }))
    .unwrap_or(std::ptr::null_mut())
}

/// The gutter marks for a file: which lines differ from the same file
/// at `HEAD`. `text` is the buffer's current contents, so the marks
/// follow what is being edited rather than what is on disk.
///
/// Returns a nul-terminated JSON array — `[{"line": 12, "kind":
/// "modified"}, …]`, lines zero-based, kinds `added`, `modified` and
/// `removed` — released with [`tc_string_free`]. An empty array means
/// nothing to mark, which covers a file with no committed version, one
/// outside a repository, and a machine with no `git`: none of those is
/// an error, and every line of an untracked file being new is true and
/// useless.
///
/// A `removed` mark sits on the line *after* the lines that are gone,
/// since nothing occupies their place any more.
///
/// # Safety
/// `path` must point to `path_len` readable bytes and `text` to
/// `text_len`.
#[no_mangle]
pub unsafe extern "C" fn tc_changes_for_file(
    path: *const c_char,
    path_len: usize,
    text: *const c_char,
    text_len: usize,
) -> *mut c_char {
    let Some(path) = (unsafe { str_from_raw(path, path_len) }) else {
        return std::ptr::null_mut();
    };
    let Some(text) = (unsafe { str_from_raw(text, text_len) }) else {
        return std::ptr::null_mut();
    };
    catch_unwind(AssertUnwindSafe(|| {
        let changes = textchum_core::changes::changes_for(std::path::Path::new(path), text);
        owned_c_string(textchum_core::changes::to_json(&changes))
    }))
    .unwrap_or(std::ptr::null_mut())
}

/// What git knows about one line of a file: who wrote it, when, and
/// which commit it came in on.
///
/// `line` is one-based. `contents` is the buffer's text, not the file
/// on disk — an unsaved edit above the line shifts the answer onto its
/// neighbour otherwise, and an answer about the wrong line arrives
/// looking exactly as right as a correct one.
///
/// Returns a nul-terminated JSON object, released with
/// [`tc_string_free`]:
///
/// ```json
/// {"commit": "0123…", "abbreviated": "0123456", "author": "Ada",
///  "authorMail": "ada@…", "authorDate": "2026-08-28 14:03:22 +0200",
///  "committer": "", "committerDate": "", "summary": "Do the thing",
///  "body": "Because.", "renamedFrom": "", "uncommitted": false}
/// ```
///
/// `committer` and `committerDate` are set only when they differ from
/// the author's, and `renamedFrom` only when the file has been renamed
/// since. `uncommitted` marks a line that is typed and not yet
/// committed, and carries no commit.
///
/// Returns null on failure and fills the optional `error_out` with a
/// sentence to show — a file outside a repository, or a machine with
/// no git, arrives here rather than as an empty answer.
///
/// # Safety
/// `path` must point to `path_len` readable bytes and `contents` to
/// `contents_len`; `error_out`, if given, must point to a writable
/// slot.
#[no_mangle]
pub unsafe extern "C" fn tc_blame_line(
    path: *const c_char,
    path_len: usize,
    line: usize,
    contents: *const c_char,
    contents_len: usize,
    error_out: *mut *mut c_char,
) -> *mut c_char {
    if !error_out.is_null() {
        unsafe { *error_out = std::ptr::null_mut() };
    }
    let Some(path) = (unsafe { str_from_raw(path, path_len) }) else {
        unsafe { write_error(error_out, "invalid path string") };
        return std::ptr::null_mut();
    };
    let Some(contents) = (unsafe { str_from_raw(contents, contents_len) }) else {
        unsafe { write_error(error_out, "invalid contents string") };
        return std::ptr::null_mut();
    };
    catch_unwind(AssertUnwindSafe(|| {
        match textchum_core::blame::blame_line(std::path::Path::new(path), line, contents) {
            Ok(blame) => owned_c_string(textchum_core::blame::to_json(&blame)),
            Err(error) => {
                unsafe { write_error(error_out, &error.to_string()) };
                std::ptr::null_mut()
            }
        }
    }))
    .unwrap_or(std::ptr::null_mut())
}

/// How many characters backspace should remove, given the text of the
/// line before the caret.
///
/// One, unless everything between the start of the line and the caret
/// is spaces — then back to the previous tab stop. Zero at the very
/// start of a line, where backspace joins with the line above and this
/// has nothing to say.
///
/// # Safety
/// `before_caret` must point to `len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn tc_indent_backspace_width(
    before_caret: *const c_char,
    len: usize,
    tab_width: usize,
) -> usize {
    let Some(before) = (unsafe { str_from_raw(before_caret, len) }) else {
        return 1;
    };
    catch_unwind(AssertUnwindSafe(|| {
        textchum_core::indent::backspace_width(before, tab_width)
    }))
    .unwrap_or(1)
}

/// The whitespace a line should be indented with to line up with the
/// block above it, or one level deeper when it is already level.
///
/// `previous` is the nearest non-blank line above (may be null when
/// there is none) and `current_indent` the line's own leading
/// whitespace. Returns a nul-terminated string to put in place of that
/// indentation; release it with [`tc_string_free`].
///
/// # Safety
/// `previous`, if not null, must point to `previous_len` readable
/// bytes; `current_indent` must point to `current_len`.
#[no_mangle]
pub unsafe extern "C" fn tc_indent_aligned(
    previous: *const c_char,
    previous_len: usize,
    current_indent: *const c_char,
    current_len: usize,
    tab_width: usize,
    use_tabs: bool,
) -> *mut c_char {
    let previous = if previous.is_null() {
        None
    } else {
        unsafe { str_from_raw(previous, previous_len) }
    };
    let Some(current) = (unsafe { str_from_raw(current_indent, current_len) }) else {
        return std::ptr::null_mut();
    };
    catch_unwind(AssertUnwindSafe(|| {
        owned_c_string(textchum_core::indent::aligned_indent(
            previous, current, tab_width, use_tabs,
        ))
    }))
    .unwrap_or(std::ptr::null_mut())
}

/// The selectable language names with a representative file extension,
/// one per line as `name \x1f extension` (extension may be empty).
/// Release with [`tc_string_free`]. For "new file with format" pickers.
#[no_mangle]
pub extern "C" fn tc_language_names() -> *mut c_char {
    let joined = textchum_core::syntax::languages::selectable_names()
        .into_iter()
        .filter_map(|name| {
            let spec = textchum_core::syntax::languages::by_name(name)?.spec;
            Some(format!(
                "{}\x1f{}",
                spec.name,
                spec.extensions.first().copied().unwrap_or("")
            ))
        })
        .collect::<Vec<_>>()
        .join("\n");
    owned_c_string(joined)
}

/// Sets the document's syntax language by name (`len == 0` clears it back
/// to plain text). Returns false for unknown names or documents beyond the
/// syntax size cap.
///
/// # Safety
/// `document` must be a live document pointer; `name` must point to `len`
/// readable bytes.
#[no_mangle]
pub unsafe extern "C" fn tc_document_set_language(
    document: *mut TcDocument,
    name: *const c_char,
    len: usize,
) -> bool {
    let Some(document) = (unsafe { document.as_mut() }) else {
        return false;
    };
    let Some(name) = (unsafe { str_from_raw(name, len) }) else {
        return false;
    };
    catch_unwind(AssertUnwindSafe(|| {
        document
            .inner
            .set_language(if name.is_empty() { None } else { Some(name) })
    }))
    .unwrap_or(false)
}

/// The document's syntax language name, or null for plain text. Release
/// with [`tc_string_free`].
///
/// # Safety
/// `document` must be a live document pointer.
#[no_mangle]
pub unsafe extern "C" fn tc_document_language_name(document: *const TcDocument) -> *mut c_char {
    let Some(document) = (unsafe { document.as_ref() }) else {
        return std::ptr::null_mut();
    };
    match document.inner.language_name() {
        Some(name) => owned_c_string(name.to_owned()),
        None => std::ptr::null_mut(),
    }
}

/// Styled spans over the UTF-16 code unit range `start..end`, in
/// application order — where spans overlap, the later one wins. On success
/// stores the array in `spans_out`/`count_out` (empty is a success:
/// null/0); release with [`tc_highlight_spans_free`]. Returns false on
/// invalid ranges.
///
/// # Safety
/// `document` must be a live document pointer; `spans_out` and `count_out`
/// must point to writable slots.
#[no_mangle]
pub unsafe extern "C" fn tc_document_highlights(
    document: *const TcDocument,
    start: usize,
    end: usize,
    spans_out: *mut *mut TcHighlightSpan,
    count_out: *mut usize,
) -> bool {
    if spans_out.is_null() || count_out.is_null() {
        return false;
    }
    unsafe {
        *spans_out = std::ptr::null_mut();
        *count_out = 0;
    }
    let Some(document) = (unsafe { document.as_ref() }) else {
        return false;
    };
    catch_unwind(AssertUnwindSafe(|| {
        match document.inner.highlights(start, end) {
            Ok(spans) if spans.is_empty() => true,
            Ok(spans) => {
                let boxed: Box<[TcHighlightSpan]> = spans
                    .into_iter()
                    .map(|span| TcHighlightSpan {
                        start: span.start_utf16,
                        end: span.end_utf16,
                        style: span.style,
                    })
                    .collect();
                unsafe {
                    *count_out = boxed.len();
                    *spans_out = Box::into_raw(boxed) as *mut TcHighlightSpan;
                }
                true
            }
            Err(_) => false,
        }
    }))
    .unwrap_or(false)
}

/// Releases a span array from [`tc_document_highlights`].
///
/// # Safety
/// `spans` and `count` must be exactly the pair produced by one highlights
/// call, not previously freed.
#[no_mangle]
pub unsafe extern "C" fn tc_highlight_spans_free(spans: *mut TcHighlightSpan, count: usize) {
    if !spans.is_null() {
        drop(unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(spans, count)) });
    }
}

/// Fuzzy-matches file paths under `root` against `query`, best match
/// first (an empty query lists files alphabetically). Returns the
/// relative paths joined by `\n` as one string (release with
/// [`tc_string_free`]); empty string for no matches, null on invalid
/// input. Pure function over the file system — callable from any thread.
///
/// # Safety
/// `root` and `query` must point to their stated numbers of readable
/// bytes.
#[no_mangle]
pub unsafe extern "C" fn tc_fuzzy_files(
    root: *const c_char,
    root_len: usize,
    query: *const c_char,
    query_len: usize,
    limit: usize,
) -> *mut c_char {
    let (root, query) =
        unsafe { (str_from_raw(root, root_len), str_from_raw(query, query_len)) };
    let (Some(root), Some(query)) = (root, query) else {
        return std::ptr::null_mut();
    };
    catch_unwind(AssertUnwindSafe(|| {
        let paths =
            textchum_core::search::fuzzy_files(std::path::Path::new(root), query, limit);
        owned_c_string(paths.join("\n"))
    }))
    .unwrap_or(std::ptr::null_mut())
}

/// A Markdown document's headings, one per line as
/// `level \x1f line \x1f character \x1f text` — the outline a post
/// deserves when no language server is answering. Front matter and
/// fenced code are skipped. Release with [`tc_string_free`].
///
/// # Safety
/// `text` must point to its stated number of readable bytes.
#[no_mangle]
pub unsafe extern "C" fn tc_markdown_headings(
    text: *const c_char,
    text_len: usize,
) -> *mut c_char {
    let Some(text) = (unsafe { str_from_raw(text, text_len) }) else {
        return std::ptr::null_mut();
    };
    let joined = catch_unwind(AssertUnwindSafe(|| {
        textchum_core::hugo::headings(text)
            .into_iter()
            .map(|heading| {
                format!(
                    "{}\x1f{}\x1f{}\x1f{}",
                    heading.level, heading.line, heading.character, heading.text
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }))
    .unwrap_or_default();
    owned_c_string(joined)
}

/// The UTF-16 ranges a spell checker must skip in a Hugo document —
/// front matter and shortcode calls — one per line as `start \x1f end`.
/// Empty when the document has neither. Release with
/// [`tc_string_free`].
///
/// # Safety
/// `text` must point to its stated number of readable bytes.
#[no_mangle]
pub unsafe extern "C" fn tc_hugo_non_prose_ranges(
    text: *const c_char,
    text_len: usize,
) -> *mut c_char {
    let Some(text) = (unsafe { str_from_raw(text, text_len) }) else {
        return std::ptr::null_mut();
    };
    let joined = catch_unwind(AssertUnwindSafe(|| {
        textchum_core::hugo::non_prose_ranges(text)
            .into_iter()
            .map(|range| {
                // Byte ranges become UTF-16 ones, which is what the
                // shells address text with.
                let start = text[..range.start].encode_utf16().count();
                let end = start + text[range.clone()].encode_utf16().count();
                format!("{start}\x1f{end}")
            })
            .collect::<Vec<_>>()
            .join("\n")
    }))
    .unwrap_or_default();
    owned_c_string(joined)
}

/// The project's file list under `root` (ignore-aware), `\n`-joined
/// (release with [`tc_string_free`]). Walk once, then match many times
/// with [`tc_match_files`] — re-walking per keystroke is what makes a
/// fuzzy finder feel broken on a real repository. Pure function —
/// callable from any thread.
///
/// # Safety
/// `root` must point to its stated number of readable bytes.
#[no_mangle]
pub unsafe extern "C" fn tc_list_files(root: *const c_char, root_len: usize) -> *mut c_char {
    let Some(root) = (unsafe { str_from_raw(root, root_len) }) else {
        return std::ptr::null_mut();
    };
    catch_unwind(AssertUnwindSafe(|| {
        let paths = textchum_core::search::list_files(std::path::Path::new(root));
        owned_c_string(paths.join("\n"))
    }))
    .unwrap_or(std::ptr::null_mut())
}

/// Fuzzy-matches an already-walked `\n`-joined file list (from
/// [`tc_list_files`]) against `query`, best first; an empty query lists
/// alphabetically. Returns the `\n`-joined matches (release with
/// [`tc_string_free`]). Pure function — callable from any thread.
///
/// # Safety
/// `paths` and `query` must point to their stated numbers of readable
/// bytes.
#[no_mangle]
pub unsafe extern "C" fn tc_match_files(
    paths: *const c_char,
    paths_len: usize,
    query: *const c_char,
    query_len: usize,
    limit: usize,
) -> *mut c_char {
    let (paths, query) =
        unsafe { (str_from_raw(paths, paths_len), str_from_raw(query, query_len)) };
    let (Some(paths), Some(query)) = (paths, query) else {
        return std::ptr::null_mut();
    };
    catch_unwind(AssertUnwindSafe(|| {
        let list: Vec<String> = if paths.is_empty() {
            Vec::new()
        } else {
            paths.split('\n').map(str::to_owned).collect()
        };
        let matched = textchum_core::search::match_files(&list, query, limit);
        owned_c_string(matched.join("\n"))
    }))
    .unwrap_or(std::ptr::null_mut())
}

/// Searches file contents under `root` for the regex `pattern`. Returns
/// one string (release with [`tc_string_free`]) of `\n`-joined records:
/// the **first line is always statistics** —
/// `files_seen \x1f files_searched \x1f unreadable` — and each line after
/// it is a hit, `path \x1f line \x1f text`. A search with no hits still
/// returns its statistics line, so callers can tell "nothing matched"
/// from "nothing was readable". On a bad pattern returns null and fills
/// the optional `error_out` (release with [`tc_string_free`]). Pure
/// function — callable from any thread.
///
/// `filters` (`filters_len` bytes; may be empty) is a JSON array of
/// stacked refinements applied case-insensitively as substrings:
/// `[{"kind": "line"|"file", "include": bool, "pattern": "…"}]`.
///
/// # Safety
/// Each pointer/length pair must describe readable bytes; `error_out`,
/// if non-null, must point to a writable slot.
#[no_mangle]
pub unsafe extern "C" fn tc_grep(
    root: *const c_char,
    root_len: usize,
    pattern: *const c_char,
    pattern_len: usize,
    case_insensitive: bool,
    limit: usize,
    filters: *const c_char,
    filters_len: usize,
    error_out: *mut *mut c_char,
) -> *mut c_char {
    if !error_out.is_null() {
        unsafe { *error_out = std::ptr::null_mut() };
    }
    let (root, pattern, filters) = unsafe {
        (
            str_from_raw(root, root_len),
            str_from_raw(pattern, pattern_len),
            str_from_raw(filters, filters_len),
        )
    };
    let (Some(root), Some(pattern), Some(filters_json)) = (root, pattern, filters) else {
        return std::ptr::null_mut();
    };
    let filters = parse_filters(filters_json);
    catch_unwind(AssertUnwindSafe(|| {
        match textchum_core::search::grep_with_stats(
            std::path::Path::new(root),
            pattern,
            case_insensitive,
            limit,
            &filters,
        ) {
            Ok((hits, stats)) => {
                // The first line is always the search's own account of
                // itself: files seen, files searched, unreadable entries.
                let mut joined = vec![format!(
                    "{}\x1f{}\x1f{}",
                    stats.files_seen, stats.files_searched, stats.errors
                )];
                joined.extend(
                    hits.into_iter()
                        .map(|hit| format!("{}\x1f{}\x1f{}", hit.path, hit.line, hit.text)),
                );
                owned_c_string(joined.join("\n"))
            }
            Err(error) => {
                unsafe { write_error(error_out, &error) };
                std::ptr::null_mut()
            }
        }
    }))
    .unwrap_or(std::ptr::null_mut())
}

/// Parses the JSON filter array for [`tc_grep`]; malformed entries are
/// skipped rather than failing the search.
fn parse_filters(json: &str) -> Vec<textchum_core::search::Filter> {
    use textchum_core::search::{Filter, FilterKind};
    let Ok(values) = serde_json::from_str::<Vec<serde_json::Value>>(json) else {
        return Vec::new();
    };
    values
        .into_iter()
        .filter_map(|value| {
            let kind = match value["kind"].as_str()? {
                "line" => FilterKind::Line,
                "file" => FilterKind::File,
                _ => return None,
            };
            let pattern = value["pattern"].as_str()?.to_owned();
            if pattern.is_empty() {
                return None;
            }
            Some(Filter {
                kind,
                include: value["include"].as_bool().unwrap_or(true),
                pattern,
            })
        })
        .collect()
}

/// The configuration's `lsp` section, serialized (`{}` when unset):
/// `{"defaults": {lang: cmdline}, "projects": {root: {lang: cmdline}}}`.
/// Release with [`tc_string_free`]. Feed to [`tc_lsp_configure`].
///
/// # Safety
/// `config` must be a live configuration pointer.
#[no_mangle]
pub unsafe extern "C" fn tc_config_lsp_json(config: *const TcConfig) -> *mut c_char {
    let Some(config) = (unsafe { config.as_ref() }) else {
        return std::ptr::null_mut();
    };
    owned_c_string(config.inner.lsp_json())
}

/// Sets (or removes, with `command_len == 0`) the server command line for
/// a language — scoped to a project root when `root_len > 0`, the
/// defaults otherwise.
///
/// # Safety
/// `config` must be a live configuration pointer; each pointer/length
/// pair must describe readable bytes.
#[no_mangle]
pub unsafe extern "C" fn tc_config_set_lsp_entry(
    config: *mut TcConfig,
    root: *const c_char,
    root_len: usize,
    language: *const c_char,
    language_len: usize,
    command: *const c_char,
    command_len: usize,
) {
    let Some(config) = (unsafe { config.as_mut() }) else {
        return;
    };
    let (root, language, command) = unsafe {
        (
            str_from_raw(root, root_len),
            str_from_raw(language, language_len),
            str_from_raw(command, command_len),
        )
    };
    let (Some(root), Some(language), Some(command)) = (root, language, command) else {
        return;
    };
    config.inner.set_lsp_entry(
        (!root.is_empty()).then_some(root),
        language,
        (!command.is_empty()).then_some(command),
    );
}

/// The configuration's `preprocessors` section, serialized (`{}` when
/// unset): `{"defaults": {lang: [cmd, ...]}, "projects": {root: {...}}}`.
/// Release with [`tc_string_free`].
///
/// # Safety
/// `config` must be a live configuration pointer.
#[no_mangle]
pub unsafe extern "C" fn tc_config_preprocessors_json(config: *const TcConfig) -> *mut c_char {
    let Some(config) = (unsafe { config.as_ref() }) else {
        return std::ptr::null_mut();
    };
    owned_c_string(config.inner.preprocessors_json())
}

/// Sets (or removes, with `commands_len == 0`) the save-preprocessor
/// chain for a language — newline-separated command lines — scoped to a
/// project root when `root_len > 0`, the defaults otherwise.
///
/// # Safety
/// `config` must be a live configuration pointer; each pointer/length
/// pair must describe readable bytes.
#[no_mangle]
pub unsafe extern "C" fn tc_config_set_preprocessor_entry(
    config: *mut TcConfig,
    root: *const c_char,
    root_len: usize,
    language: *const c_char,
    language_len: usize,
    commands: *const c_char,
    commands_len: usize,
) {
    let Some(config) = (unsafe { config.as_mut() }) else {
        return;
    };
    let (root, language, commands) = unsafe {
        (
            str_from_raw(root, root_len),
            str_from_raw(language, language_len),
            str_from_raw(commands, commands_len),
        )
    };
    let (Some(root), Some(language), Some(commands)) = (root, language, commands) else {
        return;
    };
    config.inner.set_preprocessor_entry(
        (!root.is_empty()).then_some(root),
        language,
        (!commands.is_empty()).then_some(commands),
    );
}

/// The resolved preprocessor chain for a language under a project root
/// (the defaults when `root_len == 0` or the root has no entry), as
/// newline-separated command lines — empty when none configured.
/// Release with [`tc_string_free`].
///
/// # Safety
/// `config` must be a live configuration pointer; each pointer/length
/// pair must describe readable bytes.
#[no_mangle]
pub unsafe extern "C" fn tc_config_preprocessor_commands(
    config: *const TcConfig,
    root: *const c_char,
    root_len: usize,
    language: *const c_char,
    language_len: usize,
) -> *mut c_char {
    let Some(config) = (unsafe { config.as_ref() }) else {
        return std::ptr::null_mut();
    };
    let (root, language) = unsafe {
        (str_from_raw(root, root_len), str_from_raw(language, language_len))
    };
    let (Some(root), Some(language)) = (root, language) else {
        return std::ptr::null_mut();
    };
    let commands = config
        .inner
        .preprocessor_commands((!root.is_empty()).then_some(root), language);
    owned_c_string(commands.join("\n"))
}

/// The navigator's hidden-name globs for a root (the defaults when
/// `root_len == 0`), newline-joined. Release with [`tc_string_free`].
///
/// # Safety
/// `config` must be a live configuration pointer; the pointer/length
/// pair must describe readable bytes.
#[no_mangle]
pub unsafe extern "C" fn tc_config_hide_globs(
    config: *const TcConfig,
    root: *const c_char,
    root_len: usize,
) -> *mut c_char {
    let Some(config) = (unsafe { config.as_ref() }) else {
        return std::ptr::null_mut();
    };
    let Some(root) = (unsafe { str_from_raw(root, root_len) }) else {
        return std::ptr::null_mut();
    };
    let globs = config.inner.hide_globs((!root.is_empty()).then_some(root));
    owned_c_string(globs.join("\n"))
}

/// Sets (or removes, with `globs_len == 0`) the hidden-name globs —
/// whitespace-separated — for a root, or the defaults when
/// `root_len == 0`.
///
/// # Safety
/// `config` must be a live configuration pointer; each pointer/length
/// pair must describe readable bytes.
#[no_mangle]
pub unsafe extern "C" fn tc_config_set_hide_globs(
    config: *mut TcConfig,
    root: *const c_char,
    root_len: usize,
    globs: *const c_char,
    globs_len: usize,
) {
    let Some(config) = (unsafe { config.as_mut() }) else {
        return;
    };
    let (root, globs) = unsafe {
        (str_from_raw(root, root_len), str_from_raw(globs, globs_len))
    };
    let (Some(root), Some(globs)) = (root, globs) else { return };
    config.inner.set_hide_globs(
        (!root.is_empty()).then_some(root),
        (!globs.is_empty()).then_some(globs),
    );
}

/// Whether a name is hidden by any of the newline-joined globs.
///
/// # Safety
/// Each pointer/length pair must describe readable bytes.
#[no_mangle]
pub unsafe extern "C" fn tc_workspace_is_hidden(
    name: *const c_char,
    name_len: usize,
    globs: *const c_char,
    globs_len: usize,
) -> bool {
    let (name, globs) = unsafe {
        (str_from_raw(name, name_len), str_from_raw(globs, globs_len))
    };
    let (Some(name), Some(globs)) = (name, globs) else { return false };
    let globs: Vec<String> = globs.lines().map(str::to_owned).collect();
    textchum_core::workspace::is_hidden(name, &globs)
}

/// The hidden-glob presets, one per line as `name\x1fglob glob …`,
/// sorted by name. Release with [`tc_string_free`].
///
/// # Safety
/// `config` must be a live configuration pointer.
#[no_mangle]
pub unsafe extern "C" fn tc_config_hide_presets(config: *const TcConfig) -> *mut c_char {
    let Some(config) = (unsafe { config.as_ref() }) else {
        return std::ptr::null_mut();
    };
    let joined = config
        .inner
        .hide_presets()
        .into_iter()
        .map(|(name, globs)| format!("{name}\x1f{}", globs.join(" ")))
        .collect::<Vec<_>>()
        .join("\n");
    owned_c_string(joined)
}

/// Sets (or removes, with `globs_len == 0`) one preset by name.
///
/// # Safety
/// `config` must be a live configuration pointer; each pointer/length
/// pair must describe readable bytes.
#[no_mangle]
pub unsafe extern "C" fn tc_config_set_hide_preset(
    config: *mut TcConfig,
    name: *const c_char,
    name_len: usize,
    globs: *const c_char,
    globs_len: usize,
) {
    let Some(config) = (unsafe { config.as_mut() }) else {
        return;
    };
    let (name, globs) = unsafe {
        (str_from_raw(name, name_len), str_from_raw(globs, globs_len))
    };
    let (Some(name), Some(globs)) = (name, globs) else { return };
    if name.is_empty() {
        return;
    }
    config
        .inner
        .set_hide_preset(name, (!globs.is_empty()).then_some(globs));
}

/// Forgets the user's presets, restoring the built-ins.
///
/// # Safety
/// `config` must be a live configuration pointer.
#[no_mangle]
pub unsafe extern "C" fn tc_config_reset_hide_presets(config: *mut TcConfig) {
    if let Some(config) = unsafe { config.as_mut() } {
        config.inner.reset_hide_presets();
    }
}

/// Whether the navigator follows the current file (default true).
///
/// # Safety
/// `config` must be a live configuration pointer.
#[no_mangle]
pub unsafe extern "C" fn tc_config_follow_file(config: *const TcConfig) -> bool {
    let Some(config) = (unsafe { config.as_ref() }) else {
        return true;
    };
    catch_unwind(AssertUnwindSafe(|| config.inner.follow_file())).unwrap_or(true)
}

/// Sets the follow-the-file choice.
///
/// # Safety
/// `config` must be a live configuration pointer.
#[no_mangle]
pub unsafe extern "C" fn tc_config_set_follow_file(config: *mut TcConfig, enabled: bool) {
    let Some(config) = (unsafe { config.as_mut() }) else {
        return;
    };
    let _ = catch_unwind(AssertUnwindSafe(|| config.inner.set_follow_file(enabled)));
}

/// The prose spell-check language (`editor.spell`): a spelling
/// identifier, `"auto"`, or empty when spell checking is off.
/// Release with [`tc_string_free`].
///
/// # Safety
/// `config` must be a live configuration pointer.
#[no_mangle]
pub unsafe extern "C" fn tc_config_spell_language(config: *const TcConfig) -> *mut c_char {
    let Some(config) = (unsafe { config.as_ref() }) else {
        return std::ptr::null_mut();
    };
    owned_c_string(config.inner.spell_language().unwrap_or_default())
}

/// Sets (or removes, with `language_len == 0`) the spell-check language.
///
/// # Safety
/// `config` must be a live configuration pointer; the pointer/length
/// pair must describe readable bytes.
#[no_mangle]
pub unsafe extern "C" fn tc_config_set_spell_language(
    config: *mut TcConfig,
    language: *const c_char,
    language_len: usize,
) {
    let Some(config) = (unsafe { config.as_mut() }) else {
        return;
    };
    let Some(language) = (unsafe { str_from_raw(language, language_len) }) else {
        return;
    };
    config
        .inner
        .set_spell_language((!language.is_empty()).then_some(language));
}

/// The spell-check setting split into the dictionaries it names, as a
/// JSON array of strings. `"en_US, es_ES"` is two; `"auto"` is one.
/// Release with [`tc_string_free`].
///
/// # Safety
/// `config` must be a live configuration pointer.
#[no_mangle]
pub unsafe extern "C" fn tc_config_spell_languages_json(
    config: *const TcConfig,
) -> *mut c_char {
    let Some(config) = (unsafe { config.as_ref() }) else {
        return std::ptr::null_mut();
    };
    let languages = config.inner.spell_languages();
    owned_c_string(serde_json::to_string(&languages).unwrap_or_else(|_| "[]".into()))
}

/// The personal word list (`editor.spell_words`) as a JSON array of
/// strings — words the spell checker accepts whatever the dictionary
/// says. Release with [`tc_string_free`].
///
/// # Safety
/// `config` must be a live configuration pointer.
#[no_mangle]
pub unsafe extern "C" fn tc_config_spell_words_json(config: *const TcConfig) -> *mut c_char {
    let Some(config) = (unsafe { config.as_ref() }) else {
        return std::ptr::null_mut();
    };
    let words = config.inner.spell_words();
    owned_c_string(serde_json::to_string(&words).unwrap_or_else(|_| "[]".into()))
}

/// Replaces the personal word list from a JSON array of strings. A
/// malformed document is ignored rather than emptying the list.
///
/// # Safety
/// `config` must be a live configuration pointer; the pointer/length
/// pair must describe readable bytes.
#[no_mangle]
pub unsafe extern "C" fn tc_config_set_spell_words_json(
    config: *mut TcConfig,
    json: *const c_char,
    len: usize,
) {
    let Some(config) = (unsafe { config.as_mut() }) else {
        return;
    };
    let Some(json) = (unsafe { str_from_raw(json, len) }) else {
        return;
    };
    let Ok(words) = serde_json::from_str::<Vec<String>>(json) else {
        return;
    };
    config.inner.set_spell_words(&words);
}

/// Adds one word to the personal list. Returns true when it was new, so
/// the caller can skip a re-check that would change nothing.
///
/// # Safety
/// `config` must be a live configuration pointer; the pointer/length
/// pair must describe readable bytes.
#[no_mangle]
pub unsafe extern "C" fn tc_config_add_spell_word(
    config: *mut TcConfig,
    word: *const c_char,
    len: usize,
) -> bool {
    let Some(config) = (unsafe { config.as_mut() }) else {
        return false;
    };
    let Some(word) = (unsafe { str_from_raw(word, len) }) else {
        return false;
    };
    config.inner.add_spell_word(word)
}

/// Seconds of quiet before the editor saves by itself (`editor.autosave`);
/// zero means autosave is off, which is the default.
///
/// # Safety
/// `config` must be a live configuration pointer.
#[no_mangle]
pub unsafe extern "C" fn tc_config_autosave_seconds(config: *const TcConfig) -> u32 {
    let Some(config) = (unsafe { config.as_ref() }) else {
        return 0;
    };
    config.inner.autosave_seconds()
}

/// Sets the autosave delay; zero removes the key and turns it off.
///
/// # Safety
/// `config` must be a live configuration pointer.
#[no_mangle]
pub unsafe extern "C" fn tc_config_set_autosave_seconds(
    config: *mut TcConfig,
    seconds: u32,
) {
    let Some(config) = (unsafe { config.as_mut() }) else {
        return;
    };
    let _ = catch_unwind(AssertUnwindSafe(|| {
        config.inner.set_autosave_seconds(seconds)
    }));
}

/// Whether a file is one the editor can meaningfully open: text rather
/// than a PNG the desktop's recent-files list happens to remember. The
/// content decides, not the extension.
///
/// # Safety
/// The pointer/length pair must describe readable bytes.
#[no_mangle]
pub unsafe extern "C" fn tc_path_looks_editable(path: *const c_char, len: usize) -> bool {
    let Some(path) = (unsafe { str_from_raw(path, len) }) else {
        return false;
    };
    textchum_core::workspace::looks_editable(std::path::Path::new(path))
}

/// Whether a language server command would start: an absolute path that
/// exists, or a bare name found on `PATH`. Only the command's first word
/// is looked at, because that is what the pool runs.
///
/// # Safety
/// The pointer/length pair must describe readable bytes.
#[no_mangle]
pub unsafe extern "C" fn tc_lsp_executable_exists(
    command: *const c_char,
    len: usize,
) -> bool {
    let Some(command) = (unsafe { str_from_raw(command, len) }) else {
        return false;
    };
    textchum_lsp::registry::executable_exists(command)
}

/// The server registry as JSON: an array of
/// `{"id", "command", "languages": [...], "installHint"}`, so a settings
/// screen can list what there is to configure rather than only what has
/// already been overridden. Release with [`tc_string_free`].
#[no_mangle]
pub extern "C" fn tc_lsp_registry_json() -> *mut c_char {
    let entries: Vec<serde_json::Value> = textchum_lsp::registry::all()
        .iter()
        .map(|spec| {
            let command = if spec.args.is_empty() {
                spec.command.to_string()
            } else {
                format!("{} {}", spec.command, spec.args.join(" "))
            };
            serde_json::json!({
                "id": spec.id,
                "command": command,
                "languages": spec.languages,
                "installHint": spec.install_hint,
            })
        })
        .collect();
    owned_c_string(serde_json::to_string(&entries).unwrap_or_else(|_| "[]".into()))
}

/// The language a path implies, by extension or by name — empty when
/// nothing matches. What "Automatic" means in the file properties
/// panel, and the answer a document falls back to when its override is
/// removed. Release with [`tc_string_free`].
///
/// # Safety
/// The pointer/length pair must describe readable bytes.
#[no_mangle]
pub unsafe extern "C" fn tc_language_for_path(
    path: *const c_char,
    len: usize,
) -> *mut c_char {
    let Some(path) = (unsafe { str_from_raw(path, len) }) else {
        return std::ptr::null_mut();
    };
    let name = textchum_core::syntax::languages::by_path(std::path::Path::new(path))
        .map(|entry| entry.spec.name)
        .unwrap_or("");
    owned_c_string(name.to_owned())
}

/// What a document has been told about itself, as JSON:
/// `{"language": "sql", "tab_width": 2, "spaces": true}`, with absent
/// keys meaning the usual answer applies. `{}` when nothing was said.
/// Release with [`tc_string_free`].
///
/// # Safety
/// `config` must be a live configuration pointer; the pointer/length
/// pair must describe readable bytes.
#[no_mangle]
pub unsafe extern "C" fn tc_config_file_override_json(
    config: *const TcConfig,
    path: *const c_char,
    path_len: usize,
) -> *mut c_char {
    let Some(config) = (unsafe { config.as_ref() }) else {
        return std::ptr::null_mut();
    };
    let Some(path) = (unsafe { str_from_raw(path, path_len) }) else {
        return std::ptr::null_mut();
    };
    let entry = config.inner.file_override(path);
    let mut object = serde_json::Map::new();
    if let Some(language) = entry.language {
        object.insert("language".into(), serde_json::Value::String(language));
    }
    if let Some(width) = entry.tab_width {
        object.insert("tab_width".into(), serde_json::Value::Number(width.into()));
    }
    if let Some(spaces) = entry.spaces {
        object.insert("spaces".into(), serde_json::Value::Bool(spaces));
    }
    owned_c_string(serde_json::Value::Object(object).to_string())
}

/// Records what a document is, from the same JSON shape. An empty
/// object forgets the file. Malformed JSON is ignored.
///
/// # Safety
/// `config` must be a live configuration pointer; both pointer/length
/// pairs must describe readable bytes.
#[no_mangle]
pub unsafe extern "C" fn tc_config_set_file_override_json(
    config: *mut TcConfig,
    path: *const c_char,
    path_len: usize,
    json: *const c_char,
    json_len: usize,
) {
    let Some(config) = (unsafe { config.as_mut() }) else {
        return;
    };
    let Some(path) = (unsafe { str_from_raw(path, path_len) }) else {
        return;
    };
    let Some(json) = (unsafe { str_from_raw(json, json_len) }) else {
        return;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return;
    };
    let entry = textchum_core::FileOverride {
        language: value
            .get("language")
            .and_then(|v| v.as_str())
            .map(str::to_owned),
        tab_width: value
            .get("tab_width")
            .and_then(|v| v.as_u64())
            .map(|w| w as u32),
        spaces: value.get("spaces").and_then(|v| v.as_bool()),
    };
    config.inner.set_file_override(path, &entry);
}

/// Applies a server configuration (the JSON from [`tc_config_lsp_json`])
/// to the pool. Takes effect for instances spawned afterwards and clears
/// the missing-server memory.
///
/// # Safety
/// `app` must be a live pointer from [`tc_app_new`]; `json` must point to
/// `len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn tc_lsp_configure(app: *mut TcApp, json: *const c_char, len: usize) {
    let Some(app) = (unsafe { app.as_mut() }) else {
        return;
    };
    let Some(json) = (unsafe { str_from_raw(json, len) }) else {
        return;
    };
    let _ = catch_unwind(AssertUnwindSafe(|| app.pool.configure(json)));
}

/// Points the LSP debug log at a file (`len` bytes of UTF-8 path),
/// created (with parent directories) and appended to. Every pool
/// decision and server status transition is recorded there. Global, not
/// per-app; an unopenable path silently disables logging.
///
/// # Safety
/// `path` must point to `len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn tc_lsp_set_log_path(path: *const c_char, len: usize) {
    let Some(path) = (unsafe { str_from_raw(path, len) }) else {
        return;
    };
    let _ = catch_unwind(AssertUnwindSafe(|| {
        textchum_lsp::log::set_path(std::path::Path::new(path));
    }));
}

/// Forgets one (server, root) instance after a crash; the shell
/// re-announces the affected documents to spawn a replacement.
///
/// # Safety
/// `app` must be a live pointer from [`tc_app_new`]; the pointer/length
/// pairs must describe readable bytes.
#[no_mangle]
pub unsafe extern "C" fn tc_lsp_retire(
    app: *mut TcApp,
    server: *const c_char,
    server_len: usize,
    root: *const c_char,
    root_len: usize,
) {
    let Some(app) = (unsafe { app.as_mut() }) else {
        return;
    };
    let (server, root) =
        unsafe { (str_from_raw(server, server_len), str_from_raw(root, root_len)) };
    let (Some(server), Some(root)) = (server, root) else {
        return;
    };
    let _ = catch_unwind(AssertUnwindSafe(|| app.pool.retire(server, root)));
}

/// The pool's live instances, one per line as `server\x1froot` —
/// empty when nothing runs. Release with [`tc_string_free`].
///
/// # Safety
/// `app` must be a live pointer from [`tc_app_new`].
#[no_mangle]
pub unsafe extern "C" fn tc_lsp_running(app: *const TcApp) -> *mut c_char {
    let Some(app) = (unsafe { app.as_ref() }) else {
        return std::ptr::null_mut();
    };
    let joined = catch_unwind(AssertUnwindSafe(|| {
        app.pool
            .running()
            .into_iter()
            .map(|(server, root)| format!("{server}\x1f{root}"))
            .collect::<Vec<_>>()
            .join("\n")
    }))
    .unwrap_or_default();
    owned_c_string(joined)
}

/// Shuts down every running server instance. The shell re-announces its
/// open documents afterwards to respawn them under the current
/// configuration.
///
/// # Safety
/// `app` must be a live pointer from [`tc_app_new`].
#[no_mangle]
pub unsafe extern "C" fn tc_lsp_restart_servers(app: *mut TcApp) {
    let Some(app) = (unsafe { app.as_mut() }) else {
        return;
    };
    let _ = catch_unwind(AssertUnwindSafe(|| app.pool.shutdown_all()));
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
    fn a_snippet_walks_its_stops_and_mirrors_through_ffi() {
        unsafe {
            let doc = tc_document_new();
            let body = "let ${1:name} = ${1:name}.frob(${2:arg});$0";
            let expanded = tc_document_snippet_expand(
                doc,
                0,
                body.as_ptr() as *const c_char,
                body.len(),
            );
            let text = std::ffi::CStr::from_ptr(expanded).to_str().unwrap().to_owned();
            tc_string_free(expanded);
            assert_eq!(text, "let name = name.frob(arg);");
            // The shell's own insertion, as a text view would make it.
            assert!(tc_document_replace_utf16(
                doc,
                0,
                0,
                text.as_ptr() as *const c_char,
                text.len()
            ));
            let mut region = TcRegion { start: 0, end: 0 };
            assert!(tc_document_snippet_begin(doc, 0, &mut region));
            assert_eq!((region.start, region.end), (4, 8));
            assert!(tc_document_snippet_active(doc));

            // Type over the first stop; its twin follows.
            let typed = "value";
            assert!(tc_document_replace_utf16(
                doc,
                4,
                8,
                typed.as_ptr() as *const c_char,
                typed.len()
            ));
            let mut edits: *mut TcAppliedEdit = std::ptr::null_mut();
            let mut count: usize = 0;
            assert!(tc_document_snippet_sync(doc, &mut edits, &mut count));
            assert_eq!(count, 1);
            tc_applied_edits_free(edits, count);
            let text = tc_document_text(doc);
            assert_eq!(
                std::ffi::CStr::from_ptr(text).to_str().unwrap(),
                "let value = value.frob(arg);"
            );
            tc_string_free(text);

            // Tab to the last stop, then off the end.
            assert!(tc_document_snippet_advance(doc, true, &mut region));
            assert_eq!((region.start, region.end), (23, 26));
            assert!(tc_document_snippet_active(doc));
            assert!(tc_document_snippet_advance(doc, true, &mut region));
            assert_eq!((region.start, region.end), (28, 28));
            assert!(!tc_document_snippet_active(doc));
            assert!(!tc_document_snippet_advance(doc, true, &mut region));

            tc_document_free(doc);
        }
    }

    #[test]
    fn a_caret_leaving_a_snippet_ends_it_through_ffi() {
        unsafe {
            let doc = tc_document_new();
            let body = "${1:a}${2:b}";
            let expanded =
                tc_document_snippet_expand(doc, 0, body.as_ptr() as *const c_char, body.len());
            let text = std::ffi::CStr::from_ptr(expanded).to_str().unwrap().to_owned();
            tc_string_free(expanded);
            assert!(tc_document_replace_utf16(
                doc,
                0,
                0,
                text.as_ptr() as *const c_char,
                text.len()
            ));
            let mut region = TcRegion { start: 0, end: 0 };
            assert!(tc_document_snippet_begin(doc, 0, &mut region));
            tc_document_snippet_caret_moved(doc, 1);
            assert!(tc_document_snippet_active(doc));
            tc_document_snippet_caret_moved(doc, 9);
            assert!(!tc_document_snippet_active(doc));

            // Cancelling an ended session is not an error.
            tc_document_snippet_cancel(doc);
            tc_document_free(doc);
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
