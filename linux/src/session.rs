//! Session persistence: which files were open and where the caret was,
//! so relaunching continues where things left off.
//!
//! Same contract as the macOS shell: a plain, hand-readable JSON file,
//! written atomically and eagerly (openings, closings, and quit — a
//! crash loses at most a moment of position, never the file list).
//! Deleting the file is a complete reset; `--fresh` ignores it once.

use std::path::PathBuf;

use adw::prelude::*;
use gtk::glib;
use serde_json::{json, Value};

use crate::shell::Shell;
use crate::workbench::Workbench;

/// `$XDG_STATE_HOME/textchum/session.json` (`~/.local/state` by
/// default).
///
/// A build run from the checkout keeps its own file beside it. It has
/// no business in the one an installed Textchum owns: it is a test
/// run, a `cargo run`, a screenshot session or a measurement, and
/// opening one scratch file with it is enough to replace a day's worth
/// of open documents.
pub fn session_path() -> PathBuf {
    let name = if is_development_build() {
        "textchum/session-development.json"
    } else {
        "textchum/session.json"
    };
    state_dir().join(name)
}

/// Whether this binary came out of a checkout rather than a package.
/// Cargo puts its builds under `target/`; every way of installing the
/// shell puts it somewhere else.
fn is_development_build() -> bool {
    std::env::current_exe()
        .map(|path| {
            path.components()
                .any(|component| component.as_os_str() == "target")
        })
        .unwrap_or(false)
}

/// The XDG state directory: not config (this is not configuration) and
/// not cache (losing it loses real state).
pub fn state_dir() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| glib::home_dir().join(".local/state"))
}

/// Writes the current session: every pathed page with its caret, and
/// which one was selected.
pub fn save() {
    let mut windows = Vec::new();
    let mut frontmost: Option<String> = None;
    Workbench::for_each(|workbench| {
        let selected = workbench.selected().and_then(|page| page.path.borrow().clone());
        for page in workbench.all_pages() {
            let Some(path) = page.path.borrow().clone() else { continue };
            let buffer = &page.buffer;
            let insert = buffer.iter_at_mark(&buffer.get_insert());
            let caret = crate::page::utf16_offset(buffer, insert.offset());
            if selected.as_deref() == Some(path.as_str()) && workbench.window.is_active() {
                frontmost = Some(path.clone());
            }
            windows.push(json!({"path": path, "caret": caret, "scroll": 0.0}));
        }
    });
    let state = json!({
        "version": 1,
        "windows": windows,
        "frontmost": frontmost,
    });
    let path = session_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let pretty = serde_json::to_string_pretty(&state).unwrap_or_default();
    // Atomic like every write: temp file plus rename.
    let temp = path.with_extension("json.tmp");
    if std::fs::write(&temp, pretty).is_ok() {
        let _ = std::fs::rename(&temp, &path);
    }
}

/// Reopens the saved session into `workbench`. Returns how many files
/// came back.
pub fn restore(workbench: &std::rc::Rc<Workbench>) -> usize {
    let Ok(data) = std::fs::read_to_string(session_path()) else { return 0 };
    // A file that fails to parse is ignored, never overwritten with
    // garbage: the next save replaces it wholesale anyway.
    let Ok(state) = serde_json::from_str::<Value>(&data) else { return 0 };
    let mut opened = 0;
    let mut frontmost_path: Option<String> = None;
    if let Some(front) = state["frontmost"].as_str() {
        frontmost_path = Some(front.to_owned());
    }
    for window in state["windows"].as_array().into_iter().flatten() {
        let Some(path) = window["path"].as_str() else { continue };
        if !std::path::Path::new(path).is_file() {
            continue;
        }
        workbench.open(Some(PathBuf::from(path)), None);
        opened += 1;
        // The caret is stored as a UTF-16 offset into the document.
        if let Some(caret) = window["caret"].as_u64() {
            if let Some(page) = workbench.page_for(path) {
                let buffer = &page.buffer;
                let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), true);
                let target = buffer.iter_at_offset(crate::page::char_offset(&text, caret as usize));
                buffer.place_cursor(&target);
                page.view
                    .scroll_to_iter(&mut target.clone(), 0.1, false, 0.0, 0.0);
            }
        }
    }
    if let Some(front) = frontmost_path {
        if let Some(handles) = Shell::instance().pages.borrow().get(&front) {
            handles.tab_view.set_selected_page(&handles.tab_page);
        }
    }
    opened
}
