//! Session persistence: which files were open and where the caret was,
//! so relaunching continues where things left off.
//!
//! Same contract as the macOS shell: a plain, hand-readable JSON file,
//! written atomically and eagerly (openings, closings, and quit — a
//! crash loses at most a moment of position, never the file list).
//! Deleting the file is a complete reset; `--fresh` ignores it once.

use std::path::PathBuf;

use adw::prelude::*;
use serde_json::{json, Value};

use std::cell::RefCell;

use crate::shell::Shell;
use crate::workbench::Workbench;

thread_local! {
    /// Where the last untitled document was saved to — the next
    /// untitled one's dialog starts there. Session data, like the open
    /// tabs; read at restore, written with the session.
    static LAST_UNTITLED_FOLDER: RefCell<Option<String>> = const { RefCell::new(None) };
}

pub fn last_untitled_folder() -> Option<String> {
    LAST_UNTITLED_FOLDER.with(|slot| slot.borrow().clone())
}

pub fn note_untitled_save(path: &std::path::Path) {
    if let Some(parent) = path.parent() {
        LAST_UNTITLED_FOLDER
            .with(|slot| *slot.borrow_mut() = Some(parent.to_string_lossy().into_owned()));
        save();
    }
}

/// `$XDG_STATE_HOME/textchum/session.json` (`~/.local/state` by
/// default), or the profile `--data-dir` named.
pub fn session_path() -> PathBuf {
    crate::paths::session_path()
}

/// Writes the current session: every pathed page with its caret, and
/// which one was selected.
pub fn save() {
    let mut windows = Vec::new();
    let mut frontmost: Option<String> = None;
    Workbench::for_each(|workbench| {
        let selected = workbench.selected().and_then(|page| page.path().borrow().clone());
        for page in workbench.all_pages() {
            let Some(path) = page.path().borrow().clone() else { continue };
            let buffer = &page.buffer;
            let insert = buffer.iter_at_mark(&buffer.get_insert());
            let caret = crate::page::utf16_offset(buffer, insert.offset());
            if selected.as_deref() == Some(path.as_str()) && workbench.window.is_active() {
                frontmost = Some(path.clone());
            }
            windows.push(json!({"path": path, "caret": caret, "scroll": 0.0}));
        }
    });
    // The shape of each window as well as its files: the columns, what
    // each was showing, and how many views of it were stacked.
    let mut layout = Vec::new();
    Workbench::for_each(|workbench| {
        layout.push(json!({"columns": workbench.column_state()}));
    });
    let mut state = json!({
        "version": 1,
        "windows": windows,
        "layout": layout,
        "frontmost": frontmost,
    });
    if let Some(folder) = last_untitled_folder() {
        state["last_untitled_folder"] = Value::String(folder);
    }
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
    if let Some(folder) = state["last_untitled_folder"].as_str() {
        LAST_UNTITLED_FOLDER.with(|slot| *slot.borrow_mut() = Some(folder.to_owned()));
    }
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
    // The columns come back: what each was showing, and the views it
    // had stacked. Everything opened above is in the first column, so
    // the rest are made here and pointed at their files.
    if let Some(columns) = state["layout"][0]["columns"].as_array() {
        for (index, column) in columns.iter().enumerate() {
            let file = column["file"].as_str().unwrap_or_default();
            if index > 0 {
                if !std::path::Path::new(file).is_file() {
                    continue;
                }
                workbench.new_column();
                workbench.open(Some(PathBuf::from(file)), None);
            }
            let views = column["views"].as_u64().unwrap_or(1);
            for _ in 1..views {
                workbench.add_view();
            }
        }
        workbench.focus_pane(0, 0);
    }
    if let Some(front) = frontmost_path {
        if let Some(handles) = Shell::instance().pages.borrow().get(&front) {
            handles.tab_view.set_selected_page(&handles.tab_page);
        }
    }
    opened
}
