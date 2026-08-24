//! One document page: a GtkSourceView mirroring a core [`Document`].
//!
//! The sync protocol is the macOS one translated: the buffer's
//! `insert-text` / `delete-range` signals are the choke point (they fire
//! before the change lands, when offsets still describe the old text),
//! each change is applied to the core document, and debug builds assert
//! both sides stay byte-identical. Undo lives in the core; the buffer's
//! own undo is disabled and the replays come from the core's edits.
//!
//! Offsets: GtkTextBuffer speaks characters, the core and LSP speak
//! UTF-16 units. Conversions walk the text — O(n), plenty at editor
//! scale, and an honest place to optimize later.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use sourceview5::prelude::*;
use textchum_core::{theme, Document};

use crate::shell::{PageHandles, Shell};

pub struct State {
    pub document: Document,
    /// True while the shell itself mutates the buffer (loads, undo
    /// replays), so the signal handlers do not echo into the core.
    pub syncing: bool,
}

/// One tab's worth of editor.
pub struct Page {
    pub state: Rc<RefCell<State>>,
    pub buffer: sourceview5::Buffer,
    pub view: sourceview5::View,
    pub scrolled: gtk::ScrolledWindow,
    pub search_settings: sourceview5::SearchSettings,
    pub search_context: sourceview5::SearchContext,
    pub path: RefCell<Option<String>>,
}

impl Page {
    pub fn new(path: Option<PathBuf>) -> Rc<Page> {
        let document = path
            .as_deref()
            .map(|path| Document::open(path).unwrap_or_else(|_| Document::new()))
            .unwrap_or_else(Document::new);
        let document_path = document.path().map(|p| p.to_string_lossy().into_owned());
        let state = Rc::new(RefCell::new(State {
            document,
            syncing: false,
        }));

        let buffer = sourceview5::Buffer::new(None);
        // The core's tree-sitter spans are the single highlighting
        // source; GtkSourceView's own engine stays off.
        buffer.set_highlight_syntax(false);
        buffer.set_highlight_matching_brackets(false);
        buffer.set_enable_undo(false);
        install_style_tags(&buffer);
        install_diagnostic_tags(&buffer);

        let view = sourceview5::View::with_buffer(&buffer);
        view.set_monospace(true);
        view.set_show_line_numbers(Shell::instance().config.borrow().line_numbers());
        view.set_tab_width(Shell::instance().config.borrow().tab_width());
        view.set_left_margin(6);
        view.set_top_margin(6);

        let scrolled = gtk::ScrolledWindow::builder()
            .child(&view)
            .hexpand(true)
            .vexpand(true)
            .build();

        let search_settings = sourceview5::SearchSettings::new();
        search_settings.set_wrap_around(true);
        search_settings.set_case_sensitive(false);
        let search_context = sourceview5::SearchContext::new(&buffer, Some(&search_settings));
        search_context.set_highlight(true);

        // --- Load ------------------------------------------------------
        {
            let mut state = state.borrow_mut();
            state.syncing = true;
            buffer.set_text(&state.document.text());
            state.syncing = false;
        }

        // --- The choke point -------------------------------------------
        {
            let state = Rc::clone(&state);
            buffer.connect_insert_text(move |buffer, iter, text| {
                let mut state = state.borrow_mut();
                if state.syncing {
                    return;
                }
                let start = utf16_offset(buffer, iter.offset());
                let _ = state.document.replace_utf16(start, start, text);
            });
        }
        {
            let state = Rc::clone(&state);
            buffer.connect_delete_range(move |buffer, start, end| {
                let mut state = state.borrow_mut();
                if state.syncing {
                    return;
                }
                let from = utf16_offset(buffer, start.offset());
                let to = utf16_offset(buffer, end.offset());
                let _ = state.document.replace_utf16(from, to, "");
            });
        }

        let page = Rc::new(Page {
            state,
            buffer: buffer.clone(),
            view,
            scrolled,
            search_settings,
            search_context,
            path: RefCell::new(document_path),
        });

        // --- After every change: recolor, announce, verify -------------
        {
            let page_weak = Rc::downgrade(&page);
            let recolor_pending = Rc::new(Cell::new(false));
            let lsp_timer: Rc<Cell<Option<glib::SourceId>>> = Rc::new(Cell::new(None));
            buffer.connect_changed(move |buffer| {
                let Some(page) = page_weak.upgrade() else { return };
                if page.state.borrow().syncing {
                    return;
                }
                crate::workbench::refresh_chrome_for(&page);
                debug_assert_eq!(
                    buffer.text(&buffer.start_iter(), &buffer.end_iter(), true),
                    page.state.borrow().document.text(),
                    "shell and core disagree about the document"
                );
                if !recolor_pending.replace(true) {
                    let page = Rc::clone(&page);
                    let pending = Rc::clone(&recolor_pending);
                    glib::idle_add_local_once(move || {
                        pending.set(false);
                        recolor(&page.buffer);
                        apply_highlights(&page.buffer, &page.state.borrow().document);
                    });
                }
                // Announce to the server pool, debounced while typing.
                let document_path = page.path.borrow().clone();
                if let Some(path) = document_path {
                    if let Some(previous) = lsp_timer.take() {
                        previous.remove();
                    }
                    let page = Rc::clone(&page);
                    let timer = Rc::clone(&lsp_timer);
                    let source = glib::timeout_add_local_once(
                        std::time::Duration::from_millis(300),
                        move || {
                            timer.set(None);
                            let text = page.state.borrow().document.text();
                            Shell::instance()
                                .pool
                                .borrow_mut()
                                .did_change(Path::new(&path), &text);
                        },
                    );
                    lsp_timer.set(Some(source));
                }
            });
        }
        apply_highlights(&buffer, &page.state.borrow().document);

        // The pool learns about the document.
        if let (Some(path), Some(language)) = (
            page.path.borrow().clone(),
            page.state.borrow().document.language_name(),
        ) {
            let text = page.state.borrow().document.text();
            Shell::instance()
                .pool
                .borrow_mut()
                .did_open(Path::new(&path), language, &text);
        }
        page
    }

    pub fn display_name(&self) -> String {
        self.state
            .borrow()
            .document
            .path()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled".into())
    }
}

// MARK: Offsets

/// UTF-16 offset of the character offset `chars` in `buffer`.
pub fn utf16_offset(buffer: &sourceview5::Buffer, chars: i32) -> usize {
    let end = buffer.iter_at_offset(chars);
    buffer
        .text(&buffer.start_iter(), &end, true)
        .encode_utf16()
        .count()
}

/// Character offset of the UTF-16 offset `target` in `text`.
pub fn char_offset(text: &str, target: usize) -> i32 {
    let mut utf16 = 0usize;
    let mut chars = 0i32;
    for character in text.chars() {
        if utf16 >= target {
            break;
        }
        utf16 += character.len_utf16();
        chars += 1;
    }
    chars
}

/// The caret as an LSP position (zero-based line, UTF-16 column).
pub fn lsp_caret(buffer: &sourceview5::Buffer) -> (u32, u32) {
    let insert = buffer.iter_at_mark(&buffer.get_insert());
    let line = insert.line();
    let line_start = buffer
        .iter_at_line(line)
        .unwrap_or_else(|| buffer.start_iter());
    let column = buffer
        .text(&line_start, &insert, true)
        .encode_utf16()
        .count();
    (line.max(0) as u32, column as u32)
}

fn iter_at_lsp(
    buffer: &sourceview5::Buffer,
    line: i32,
    character_utf16: usize,
) -> Option<gtk::TextIter> {
    let line_start = buffer.iter_at_line(line)?;
    let mut line_end = line_start;
    if !line_end.ends_line() {
        line_end.forward_to_line_end();
    }
    let line_text = buffer.text(&line_start, &line_end, true);
    let mut target = line_start;
    target.forward_chars(char_offset(&line_text, character_utf16));
    Some(target)
}

/// Puts the caret at an LSP position and scrolls it into view.
pub fn reveal(handles: &PageHandles, line: i32, character_utf16: usize) {
    let buffer = &handles.buffer;
    let clamped = line.min(buffer.line_count() - 1).max(0);
    let Some(target) = iter_at_lsp(buffer, clamped, character_utf16) else {
        return;
    };
    buffer.place_cursor(&target);
    handles
        .view
        .scroll_to_iter(&mut target.clone(), 0.1, false, 0.0, 0.0);
    handles.tab_view.set_selected_page(&handles.tab_page);
    handles.window.present();
    handles.view.grab_focus();
}

// MARK: Highlighting

/// One text tag per style-table entry, named by index. Colors come from
/// the shared theme table; libadwaita's style manager decides which of
/// the pair applies.
fn install_style_tags(buffer: &sourceview5::Buffer) {
    for (index, _) in theme::styles().iter().enumerate() {
        let tag = gtk::TextTag::builder().name(format!("s{index}")).build();
        buffer.tag_table().add(&tag);
    }
    recolor(buffer);
}

/// (Re)binds each style tag's color from the active theme — called at
/// creation, on theme switches, and on light/dark flips.
pub fn refresh_style_tags(buffer: &sourceview5::Buffer) {
    recolor(buffer);
}

pub fn recolor(buffer: &sourceview5::Buffer) {
    let dark = adw::StyleManager::default().is_dark();
    for (index, style) in theme::styles().iter().enumerate() {
        let rgba = if dark { style.dark } else { style.light };
        let color = format!(
            "#{:02X}{:02X}{:02X}",
            (rgba >> 24) & 0xFF,
            (rgba >> 16) & 0xFF,
            (rgba >> 8) & 0xFF
        );
        if let Some(tag) = buffer.tag_table().lookup(&format!("s{index}")) {
            tag.set_foreground(Some(&color));
        }
    }
}

/// Squiggle tags for diagnostics: Pango's error underline, tinted per
/// severity — the macOS background tint's GTK cousin.
fn install_diagnostic_tags(buffer: &sourceview5::Buffer) {
    for (name, color) in [("diag-error", "#E4585B"), ("diag-warning", "#E5A54B")] {
        let rgba: gtk::gdk::RGBA = color.parse().unwrap();
        let tag = gtk::TextTag::builder()
            .name(name)
            .underline(gtk::pango::Underline::Error)
            .underline_rgba(&rgba)
            .build();
        buffer.tag_table().add(&tag);
    }
}

/// Paints the core's spans as tags — remove everything, re-apply. The
/// UTF-16 → character mapping is done in one pass over the text with the
/// span boundaries pre-sorted, so recoloring stays linear.
pub fn apply_highlights(buffer: &sourceview5::Buffer, document: &Document) {
    let start = buffer.start_iter();
    let end = buffer.end_iter();
    for index in 0..theme::styles().len() {
        if let Some(tag) = buffer.tag_table().lookup(&format!("s{index}")) {
            buffer.remove_tag(&tag, &start, &end);
        }
    }
    let Ok(spans) = document.highlights(0, document.len_utf16()) else {
        return;
    };
    if spans.is_empty() {
        return;
    }

    let text = document.text();
    let mut boundaries: Vec<usize> = spans
        .iter()
        .flat_map(|span| [span.start_utf16, span.end_utf16])
        .collect();
    boundaries.sort_unstable();
    boundaries.dedup();
    let mapped = map_utf16_to_chars(&text, &boundaries);

    for span in spans {
        let (Some(from), Some(to)) =
            (mapped.get(&span.start_utf16), mapped.get(&span.end_utf16))
        else {
            continue;
        };
        if let Some(tag) = buffer.tag_table().lookup(&format!("s{}", span.style)) {
            buffer.apply_tag(
                &tag,
                &buffer.iter_at_offset(*from),
                &buffer.iter_at_offset(*to),
            );
        }
    }
}

fn map_utf16_to_chars(
    text: &str,
    sorted_boundaries: &[usize],
) -> std::collections::HashMap<usize, i32> {
    let mut mapped = std::collections::HashMap::new();
    let mut utf16 = 0usize;
    let mut chars = 0i32;
    let mut next = sorted_boundaries.iter().peekable();
    for character in text.chars() {
        while next.peek().is_some_and(|boundary| **boundary <= utf16) {
            mapped.insert(*next.next().unwrap(), chars);
        }
        utf16 += character.len_utf16();
        chars += 1;
    }
    for boundary in next {
        mapped.insert(*boundary, chars);
    }
    mapped
}

// MARK: Diagnostics

/// Applies a diagnostics event to its page: squiggles per finding and a
/// problem count for the subtitle. Called by the shell's event pump.
pub fn apply_diagnostics(handles: &PageHandles, json: &str) {
    let buffer = &handles.buffer;
    let start = buffer.start_iter();
    let end = buffer.end_iter();
    for name in ["diag-error", "diag-warning"] {
        if let Some(tag) = buffer.tag_table().lookup(name) {
            buffer.remove_tag(&tag, &start, &end);
        }
    }
    let Ok(serde_json::Value::Array(items)) = serde_json::from_str(json) else {
        return;
    };
    let mut errors = 0usize;
    let mut warnings = 0usize;
    for item in &items {
        let line = item["line"].as_i64().unwrap_or(0) as i32;
        let character = item["character"].as_u64().unwrap_or(0) as usize;
        let end_line = item["endLine"].as_i64().unwrap_or(0) as i32;
        let end_character = item["endCharacter"].as_u64().unwrap_or(0) as usize;
        let severity = item["severity"].as_u64().unwrap_or(1);
        let tag_name = if severity == 1 {
            errors += 1;
            "diag-error"
        } else {
            warnings += 1;
            "diag-warning"
        };
        let Some(tag) = buffer.tag_table().lookup(tag_name) else {
            continue;
        };
        let (Some(from), Some(to)) = (
            iter_at_lsp(buffer, line, character),
            iter_at_lsp(buffer, end_line, end_character),
        ) else {
            continue;
        };
        // A zero-length finding still deserves a visible mark.
        let mut to = to;
        if from.offset() == to.offset() {
            to.forward_char();
        }
        buffer.apply_tag(&tag, &from, &to);
    }
    let mut parts = Vec::new();
    if errors > 0 {
        parts.push(format!("{errors} error{}", if errors == 1 { "" } else { "s" }));
    }
    if warnings > 0 {
        parts.push(format!(
            "{warnings} warning{}",
            if warnings == 1 { "" } else { "s" }
        ));
    }
    *handles.problems.borrow_mut() = parts.join(", ");
    crate::workbench::refresh_subtitle(handles);
}
