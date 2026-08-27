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
use webkit6::prelude::*;

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
    pub search_settings: sourceview5::SearchSettings,
    pub search_context: sourceview5::SearchContext,
    pub path: RefCell<Option<String>>,
    /// The whole tab child: the scrolled view alone, or a paned with
    /// the Markdown preview beside it.
    pub root: gtk::Widget,
    /// Present for Markdown documents: the live preview.
    pub preview: Option<webkit6::WebView>,
    /// Watches the file on disk; kept alive for the page's lifetime.
    pub monitor: RefCell<Option<gtk::gio::FileMonitor>>,
    /// The hover balloon and its content label.
    hover_popover: gtk::Popover,
    hover_label: gtk::Label,
    /// The completion popup and its current candidates.
    completion: CompletionState,
}

/// The completion popup: a popover under the caret, filtered by the
/// word being typed; ↑/↓ move, ⏎/⇥ accept, ⎋ dismisses — and it never
/// steals the keyboard, the view keeps focus throughout.
struct CompletionState {
    popover: gtk::Popover,
    list: gtk::ListBox,
    /// (label, insert text) per row.
    items: RefCell<Vec<(String, String)>>,
    /// Character offset where the word being completed starts.
    word_start: Cell<i32>,
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
        crate::spell::install_tag(&buffer);

        let view = sourceview5::View::with_buffer(&buffer);
        view.set_monospace(true);
        // Return inherits (and deepens) indentation — GtkSourceView has
        // this built in; macOS hand-rolls the same behavior.
        view.set_auto_indent(true);
        view.set_show_line_numbers(Shell::instance().config.borrow().line_numbers());
        view.set_tab_width(Shell::instance().config.borrow().tab_width());
        view.set_left_margin(6);
        view.set_top_margin(6);

        let scrolled = gtk::ScrolledWindow::builder()
            .child(&view)
            .hexpand(true)
            .vexpand(true)
            .build();

        // Markdown gets a live preview pane beside the text — the
        // core's HTML, reloaded shortly after edits settle.
        let is_markdown = state.borrow().document.language_name() == Some("markdown");
        let (root, preview) = if is_markdown {
            let web = webkit6::WebView::new();
            web.set_hexpand(true);
            web.set_vexpand(true);
            let paned = gtk::Paned::new(gtk::Orientation::Horizontal);
            paned.set_start_child(Some(&scrolled));
            paned.set_end_child(Some(&web));
            paned.set_position(480);
            paned.set_resize_start_child(true);
            paned.set_resize_end_child(true);
            (paned.upcast::<gtk::Widget>(), Some(web))
        } else {
            (scrolled.clone().upcast::<gtk::Widget>(), None)
        };

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
        let last_typed: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));
        {
            let state = Rc::clone(&state);
            let last_typed = Rc::clone(&last_typed);
            buffer.connect_insert_text(move |buffer, iter, text| {
                let mut state = state.borrow_mut();
                if state.syncing {
                    return;
                }
                *last_typed.borrow_mut() = text.to_string();
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

        let completion_list = gtk::ListBox::new();
        completion_list.set_selection_mode(gtk::SelectionMode::Browse);
        let completion_scroll = gtk::ScrolledWindow::builder()
            .child(&completion_list)
            .max_content_height(220)
            .propagate_natural_height(true)
            .min_content_width(280)
            .build();
        let completion_popover = gtk::Popover::new();
        completion_popover.set_parent(&view);
        completion_popover.set_autohide(false);
        completion_popover.set_has_arrow(false);
        completion_popover.set_position(gtk::PositionType::Bottom);
        completion_popover.set_child(Some(&completion_scroll));

        let hover_popover = gtk::Popover::new();
        hover_popover.set_parent(&view);
        hover_popover.set_autohide(false);
        hover_popover.set_position(gtk::PositionType::Top);
        let hover_label = gtk::Label::new(None);
        hover_label.set_wrap(true);
        hover_label.set_max_width_chars(70);
        hover_label.set_margin_top(6);
        hover_label.set_margin_bottom(6);
        hover_label.set_margin_start(8);
        hover_label.set_margin_end(8);
        hover_popover.set_child(Some(&hover_label));

        let page = Rc::new(Page {
            state,
            buffer: buffer.clone(),
            view: view.clone(),
            search_settings,
            search_context,
            path: RefCell::new(document_path),
            root,
            preview,
            monitor: RefCell::new(None),
            hover_popover,
            hover_label,
            completion: CompletionState {
                popover: completion_popover,
                list: completion_list,
                items: RefCell::new(Vec::new()),
                word_start: Cell::new(0),
            },
        });
        install_completion_keys(&page);
        install_hover(&page);
        install_file_monitor(&page);
        install_control_click(&page);
        install_spelling_menu(&page);
        apply_project_editor_overrides(&page);

        // --- After every change: recolor, announce, verify -------------
        {
            let page_weak = Rc::downgrade(&page);
            let recolor_pending = Rc::new(Cell::new(false));
            let lsp_timer: Rc<Cell<Option<glib::SourceId>>> = Rc::new(Cell::new(None));
            let autosave_timer: Rc<Cell<Option<glib::SourceId>>> = Rc::new(Cell::new(None));
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
                // Once the burst settles: announce to the server pool
                // and refresh the Markdown preview.
                let document_path = page.path.borrow().clone();
                if document_path.is_some() || page.preview.is_some() {
                    if let Some(previous) = lsp_timer.take() {
                        previous.remove();
                    }
                    let page = Rc::clone(&page);
                    let timer = Rc::clone(&lsp_timer);
                    let source = glib::timeout_add_local_once(
                        std::time::Duration::from_millis(300),
                        move || {
                            timer.set(None);
                            if let Some(path) = page.path.borrow().clone() {
                                let text = page.state.borrow().document.text();
                                Shell::instance()
                                    .pool
                                    .borrow_mut()
                                    .did_change(Path::new(&path), &text);
                            }
                            update_preview(&page);
                            crate::spell::run(&page);
                        },
                    );
                    lsp_timer.set(Some(source));
                }
                // Autosave, when it is switched on: the clock restarts
                // with every keystroke, so it fires once the typing
                // stops rather than in the middle of a sentence.
                //
                // Unlike Save, this does not run the preprocessor
                // chain. A formatter reflowing the paragraph under the
                // caret while someone is still writing it is not a
                // service; explicit saves remain the place for that.
                let autosave_after = Shell::instance().config.borrow().autosave_seconds();
                if autosave_after > 0 && document_path.is_some() {
                    if let Some(previous) = autosave_timer.take() {
                        previous.remove();
                    }
                    let page = Rc::clone(&page);
                    let timer = Rc::clone(&autosave_timer);
                    let source = glib::timeout_add_local_once(
                        std::time::Duration::from_secs(autosave_after as u64),
                        move || {
                            timer.set(None);
                            autosave(&page);
                        },
                    );
                    autosave_timer.set(Some(source));
                }
                // Completion: identifier characters and '.' ask the
                // server after a short rest; anything else dismisses.
                let typed = last_typed.borrow().clone();
                let triggers = typed.len() == 1
                    && typed.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '.');
                if triggers {
                    let page = Rc::clone(&page);
                    glib::timeout_add_local_once(
                        std::time::Duration::from_millis(250),
                        move || request_completion(&page),
                    );
                } else {
                    page.completion.popover.popdown();
                }
            });
        }
        apply_highlights(&buffer, &page.state.borrow().document);
        update_preview(&page);
        crate::spell::run(&page);

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

    /// Reloads the Markdown preview from the core's HTML.
    pub fn update_preview_now(self: &Rc<Self>) {
        update_preview(self);
    }

    /// Replaces the buffer with the file's current on-disk content —
    /// the core reload replayed into the view, caret preserved where
    /// the text allows.
    pub fn reload_from_disk(self: &Rc<Self>) {
        let caret = {
            let buffer = &self.buffer;
            buffer.iter_at_mark(&buffer.get_insert()).offset()
        };
        let reloaded = {
            let mut state = self.state.borrow_mut();
            state.syncing = true;
            let result = state.document.reload();
            if result.is_err() {
                state.syncing = false;
            }
            result
        };
        if reloaded.is_err() {
            return;
        }
        let buffer = &self.buffer;
        // Bind the text first: set_text re-enters the choke-point
        // handlers, which take their own borrow of the state.
        let text = self.state.borrow().document.text();
        buffer.set_text(&text);
        self.state.borrow_mut().syncing = false;
        let target = buffer.iter_at_offset(caret.min(buffer.char_count()));
        buffer.place_cursor(&target);
        recolor(buffer);
        apply_highlights(buffer, &self.state.borrow().document);
        // The pool sees the fresh text like any other change.
        if let Some(path) = self.path.borrow().clone() {
            let text = self.state.borrow().document.text();
            Shell::instance()
                .pool
                .borrow_mut()
                .did_change(Path::new(&path), &text);
        }
        update_preview(self);
        crate::spell::run(self);
        if let Some(workbench) = crate::workbench::Workbench::active() {
            workbench.refresh_chrome();
        }
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

/// Replaces the buffer with `new` as one minimal edit through the
/// choke point: the common prefix and suffix stay untouched, so the
/// caret and scroll survive a formatter that only changed a few lines.
pub fn apply_whole_document(page: &Rc<Page>, new: &str) {
    let buffer = &page.buffer;
    let old = buffer.text(&buffer.start_iter(), &buffer.end_iter(), true);
    if old == new {
        return;
    }
    let old_chars: Vec<char> = old.chars().collect();
    let new_chars: Vec<char> = new.chars().collect();
    let mut prefix = 0;
    while prefix < old_chars.len().min(new_chars.len())
        && old_chars[prefix] == new_chars[prefix]
    {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < old_chars.len().min(new_chars.len()) - prefix
        && old_chars[old_chars.len() - 1 - suffix] == new_chars[new_chars.len() - 1 - suffix]
    {
        suffix += 1;
    }
    let replacement: String = new_chars[prefix..new_chars.len() - suffix].iter().collect();
    let mut start = buffer.iter_at_offset(prefix as i32);
    let mut end = buffer.iter_at_offset((old_chars.len() - suffix) as i32);
    buffer.delete(&mut start, &mut end);
    let mut at = start;
    buffer.insert(&mut at, &replacement);
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

pub fn iter_at_lsp(
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

// MARK: Completion

/// Parses a completion result — `CompletionItem[]` or a
/// `CompletionList` — into (label, insert text) pairs. Snippet
/// placeholders are flattened, like the macOS popup does.
pub fn parse_completion_items(json: &str) -> Vec<(String, String)> {
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let items = match &parsed {
        serde_json::Value::Array(items) => items.clone(),
        serde_json::Value::Object(object) => object
            .get("items")
            .and_then(|items| items.as_array())
            .cloned()
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    items
        .iter()
        .filter_map(|item| {
            let label = item["label"].as_str()?.to_string();
            let insert = item["insertText"]
                .as_str()
                .or_else(|| item["textEdit"]["newText"].as_str())
                .unwrap_or(&label)
                .to_string();
            Some((label, insert))
        })
        .collect()
}

/// Expands LSP snippet syntax to plain text and remembers where the
/// caret should land, in characters of the result: `${1:placeholder}`
/// keeps its placeholder (the lowest-numbered tabstop comes back
/// selected so typing replaces it), bare `$1`/`$0` vanish (`$0`
/// marking the exit point), and `\$` stays a dollar sign. Later
/// tabstops are plain text — one honest stop, not a tabstop mode.
pub fn expand_snippet(text: &str) -> (String, Option<(i32, i32)>) {
    let mut out = String::new();
    // (tabstop number, char offset, char length) in `out`.
    let mut stops: Vec<(u32, i32, i32)> = Vec::new();
    let mut chars = text.chars().peekable();
    let mut out_chars = 0i32;
    let push = |out: &mut String, out_chars: &mut i32, c: char| {
        out.push(c);
        *out_chars += 1;
    };
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(next) = chars.next() {
                push(&mut out, &mut out_chars, next);
            } else {
                push(&mut out, &mut out_chars, c);
            }
            continue;
        }
        if c != '$' {
            push(&mut out, &mut out_chars, c);
            continue;
        }
        match chars.peek() {
            Some('{') => {
                chars.next();
                let mut body = String::new();
                for inner in chars.by_ref() {
                    if inner == '}' {
                        break;
                    }
                    body.push(inner);
                }
                let (number, placeholder) = match body.split_once(':') {
                    Some((number, placeholder)) => (number, placeholder),
                    None => (body.as_str(), ""),
                };
                let number: u32 = number.parse().unwrap_or(0);
                let length = placeholder.chars().count() as i32;
                stops.push((number, out_chars, length));
                for inner in placeholder.chars() {
                    push(&mut out, &mut out_chars, inner);
                }
            }
            Some(d) if d.is_ascii_digit() => {
                let mut digits = String::new();
                while chars.peek().is_some_and(|d| d.is_ascii_digit()) {
                    digits.push(chars.next().unwrap());
                }
                stops.push((digits.parse().unwrap_or(0), out_chars, 0));
            }
            _ => push(&mut out, &mut out_chars, c),
        }
    }
    let first = stops
        .iter()
        .filter(|(number, _, _)| *number > 0)
        .min_by_key(|(number, _, _)| *number)
        .or_else(|| stops.iter().find(|(number, _, _)| *number == 0));
    (out, first.map(|(_, offset, length)| (*offset, *length)))
}

#[cfg(test)]
mod snippet_tests {
    use super::expand_snippet;

    #[test]
    fn placeholder_selection_and_exit() {
        assert_eq!(
            expand_snippet("frob(${1:x}, ${2:y})$0"),
            ("frob(x, y)".into(), Some((5, 1)))
        );
        assert_eq!(expand_snippet("done()$0 end"), ("done() end".into(), Some((6, 0))));
        assert_eq!(expand_snippet("cost \\$5"), ("cost $5".into(), None));
    }
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The word being typed: (start character offset, prefix text).
fn word_before_caret(buffer: &sourceview5::Buffer) -> (i32, String) {
    let caret = buffer.iter_at_mark(&buffer.get_insert());
    let mut start = caret;
    while start.offset() > 0 {
        let mut previous = start;
        previous.backward_char();
        let c = previous.char();
        if !is_word_char(c) {
            break;
        }
        start = previous;
    }
    (start.offset(), buffer.text(&start, &caret, true).to_string())
}

fn request_completion(page: &Rc<Page>) {
    let Some(path) = page.path.borrow().clone() else { return };
    let (line, character) = lsp_caret(&page.buffer);
    let shell = Shell::instance();
    let id = shell
        .pool
        .borrow_mut()
        .completion(Path::new(&path), line, character);
    let page = Rc::clone(page);
    shell.expect_response(id, move |json| {
        show_completions(&page, parse_completion_items(json));
    });
}

fn show_completions(page: &Rc<Page>, all: Vec<(String, String)>) {
    let (word_start, prefix) = word_before_caret(&page.buffer);
    let prefix_lower = prefix.to_lowercase();
    let matching: Vec<(String, String)> = all
        .into_iter()
        .filter(|(label, _)| {
            prefix_lower.is_empty() || label.to_lowercase().starts_with(&prefix_lower)
        })
        .collect();
    if matching.is_empty() {
        page.completion.popover.popdown();
        return;
    }
    page.completion.word_start.set(word_start);
    while let Some(child) = page.completion.list.first_child() {
        page.completion.list.remove(&child);
    }
    for (label, _) in &matching {
        let text = gtk::Label::new(Some(label));
        text.set_xalign(0.0);
        text.set_margin_start(6);
        text.set_margin_end(6);
        page.completion.list.append(&text);
    }
    *page.completion.items.borrow_mut() = matching;
    if let Some(first) = page.completion.list.row_at_index(0) {
        page.completion.list.select_row(Some(&first));
    }

    // Point the popover at the caret.
    let caret = page.buffer.iter_at_mark(&page.buffer.get_insert());
    let rect = page.view.iter_location(&caret);
    let (x, y) = page.view.buffer_to_window_coords(
        gtk::TextWindowType::Widget,
        rect.x(),
        rect.y(),
    );
    page.completion.popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(
        x,
        y,
        1,
        rect.height(),
    )));
    page.completion.popover.popup();
}

fn accept_completion(page: &Rc<Page>) {
    let index = page
        .completion
        .list
        .selected_row()
        .map(|row| row.index())
        .unwrap_or(0);
    let insert = page
        .completion
        .items
        .borrow()
        .get(index as usize)
        .map(|(_, insert)| insert.clone());
    page.completion.popover.popdown();
    let Some(insert) = insert else { return };
    let (expanded, selection) = expand_snippet(&insert);
    let buffer = &page.buffer;
    let mut start = buffer.iter_at_offset(page.completion.word_start.get());
    let mut caret = buffer.iter_at_mark(&buffer.get_insert());
    // Through the normal buffer path, so the core sees it as typing.
    buffer.delete(&mut start, &mut caret);
    let insert_at = buffer.iter_at_mark(&buffer.get_insert()).offset();
    let mut at = buffer.iter_at_mark(&buffer.get_insert());
    buffer.insert(&mut at, &expanded);
    // A snippet's first placeholder comes back selected, so typing
    // replaces it; a bare tabstop just parks the caret there.
    if let Some((offset, length)) = selection {
        let from = buffer.iter_at_offset(insert_at + offset);
        let to = buffer.iter_at_offset(insert_at + offset + length);
        buffer.select_range(&to, &from);
    }
}

/// Keyboard routing while the popup is visible: arrows navigate it,
/// return/tab accept, escape dismisses — everything else keeps flowing
/// to the editor.
fn install_completion_keys(page: &Rc<Page>) {
    let controller = gtk::EventControllerKey::new();
    controller.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = Rc::downgrade(page);
    controller.connect_key_pressed(move |_, key, _, _| {
        let Some(page) = weak.upgrade() else {
            return glib::Propagation::Proceed;
        };
        if !page.completion.popover.is_visible() {
            return glib::Propagation::Proceed;
        }
        use gtk::gdk::Key;
        let list = &page.completion.list;
        match key {
            Key::Down => {
                let next = list.selected_row().map(|row| row.index() + 1).unwrap_or(0);
                if let Some(row) = list.row_at_index(next) {
                    list.select_row(Some(&row));
                }
                glib::Propagation::Stop
            }
            Key::Up => {
                let previous =
                    list.selected_row().map(|row| row.index() - 1).unwrap_or(0);
                if let Some(row) = list.row_at_index(previous.max(0)) {
                    list.select_row(Some(&row));
                }
                glib::Propagation::Stop
            }
            Key::Return | Key::Tab => {
                accept_completion(&page);
                glib::Propagation::Stop
            }
            Key::Escape => {
                page.completion.popover.popdown();
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        }
    });
    page.view.add_controller(controller);
}

// MARK: Hover

/// Resting the pointer over a symbol asks its server; the answer shows
/// in a popover at the spot. Moving on dismisses it.
/// Watches the document's file: external changes follow the disk
/// silently while the buffer is clean, and offer a toast (with a
/// Reload button) when local edits would be lost. The app's own saves
/// are recognized and ignored.
pub fn install_file_monitor(page: &Rc<Page>) {
    let Some(path) = page.path.borrow().clone() else { return };
    let file = gtk::gio::File::for_path(&path);
    let Ok(monitor) =
        file.monitor_file(gtk::gio::FileMonitorFlags::NONE, gtk::gio::Cancellable::NONE)
    else {
        return;
    };
    let weak = Rc::downgrade(page);
    monitor.connect_changed(move |_, _, _, event| {
        use gtk::gio::FileMonitorEvent;
        if !matches!(
            event,
            FileMonitorEvent::ChangesDoneHint | FileMonitorEvent::Created
        ) {
            return;
        }
        let Some(page) = weak.upgrade() else { return };
        let Some(path) = page.path.borrow().clone() else { return };
        if Shell::instance().is_own_save(&path) {
            return;
        }
        if page.state.borrow().document.is_dirty() {
            // Local edits at stake: never reload silently.
            let toast = adw::Toast::new("The file changed on disk. Reload and lose local edits?");
            toast.set_button_label(Some("Reload"));
            toast.set_timeout(0);
            let weak = Rc::downgrade(&page);
            toast.connect_button_clicked(move |_| {
                if let Some(page) = weak.upgrade() {
                    page.reload_from_disk();
                }
            });
            if let Some(handles) = Shell::instance().pages.borrow().get(&path) {
                handles.toasts.add_toast(toast);
            }
        } else {
            page.reload_from_disk();
        }
    });
    *page.monitor.borrow_mut() = Some(monitor);
}

/// Ctrl+click jumps to the definition under the pointer — the caret
/// moves to the click first, so the jump stack records where the
/// mouse actually was.
fn install_control_click(page: &Rc<Page>) {
    let gesture = gtk::GestureClick::new();
    gesture.set_button(1);
    let weak = Rc::downgrade(page);
    gesture.connect_pressed(move |gesture, _, x, y| {
        let state = gesture.current_event_state();
        if !state.contains(gtk::gdk::ModifierType::CONTROL_MASK) {
            return;
        }
        let Some(page) = weak.upgrade() else { return };
        let (bx, by) = page.view.window_to_buffer_coords(
            gtk::TextWindowType::Widget,
            x as i32,
            y as i32,
        );
        let Some(iter) = page.view.iter_at_location(bx, by) else { return };
        page.buffer.place_cursor(&iter);
        if let Some(workbench) = crate::workbench::Workbench::active() {
            let _ = gtk::prelude::WidgetExt::activate_action(
                &workbench.window,
                "win.definition",
                None,
            );
        }
        gesture.set_state(gtk::EventSequenceState::Claimed);
    });
    page.view.add_controller(gesture);
}

/// Saves a page on the autosave timer, if it still has anything to
/// save. Quiet by design: a toast every thirty seconds would be worse
/// than the problem autosave solves. A failure is not quiet — that is
/// the case where the user's work is not where they think it is.
fn autosave(page: &Rc<Page>) {
    let Some(path) = page.path.borrow().clone() else { return };
    let saved = {
        let mut state = page.state.borrow_mut();
        if !state.document.is_dirty() {
            return;
        }
        state.document.save().is_ok()
    };
    if saved {
        Shell::instance().note_own_save(&path);
        crate::workbench::refresh_chrome_for(page);
    } else if let Some(workbench) = crate::workbench::Workbench::active() {
        workbench.explain(&format!(
            "Autosave could not write {path} — the file is still unsaved."
        ));
    }
}

/// Builds the text view's context menu for each right-click.
///
/// GtkTextView caches its popover but throws the cache away whenever
/// `extra-menu` is set to a different object, and the capture phase
/// runs before the view's own bubble-phase handler opens the menu — so
/// replacing the model here decides what this very click shows.
///
/// Two things depend on that. Spelling actions are about the word under
/// the pointer, which is only known once there is a pointer. And Change
/// Case only makes sense with a selection: GtkSourceView disables its
/// four entries in that case, but the submenu holding them still reads
/// as available, so on an empty document the menu offers an operation
/// that cannot do anything. Owning the menu means simply not offering
/// it until there is something to change the case of.
fn install_spelling_menu(page: &Rc<Page>) {
    let gesture = gtk::GestureClick::new();
    gesture.set_button(3);
    gesture.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = Rc::downgrade(page);
    gesture.connect_pressed(move |_, _, x, y| {
        let Some(page) = weak.upgrade() else { return };
        let (bx, by) = page.view.window_to_buffer_coords(
            gtk::TextWindowType::Widget,
            x as i32,
            y as i32,
        );
        let misspelling = page
            .view
            .iter_at_location(bx, by)
            .and_then(|iter| crate::spell::word_at(&page.buffer, iter.offset()));
        if let Some((word, start, end)) = &misspelling {
            // The action handlers work from the recorded range rather
            // than from the pointer, so a menu still open while the
            // document changes underneath cannot rewrite the wrong
            // span.
            crate::spell::note_menu_target(word, *start, *end);
        }
        let menu = context_menu(
            misspelling.as_ref().map(|(word, _, _)| word.as_str()),
            page.buffer.has_selection(),
        );
        // A fresh GMenu every time: the setter compares pointers, and
        // handing back the same object would leave the cached popover
        // in place. Never None — GtkSourceView puts Change Case here,
        // and clearing it would take that away for good.
        page.view.set_extra_menu(Some(&menu));
        // Deliberately not claimed: the text view still owns this
        // click, and it is the one that opens the menu.
    });
    page.view.add_controller(gesture);
}

/// The application's half of the context menu: spelling actions when
/// the pointer is on a misspelling, and Change Case when there is a
/// selection to apply it to.
fn context_menu(misspelling: Option<&str>, has_selection: bool) -> gtk::gio::Menu {
    let menu = gtk::gio::Menu::new();
    if let Some(word) = misspelling {
        let replacements = gtk::gio::Menu::new();
        let suggestions = crate::spell::suggestions(word);
        if suggestions.is_empty() {
            // An empty section is a gap the reader has to interpret;
            // say why it is empty instead. A menu item with no action
            // renders as a disabled label, which is exactly right.
            replacements.append_item(&gtk::gio::MenuItem::new(Some("No suggestions"), None));
        }
        for suggestion in suggestions.iter().take(8) {
            replacements.append(
                Some(&crate::workbench::menu_label(suggestion)),
                Some(&format!(
                    "win.spell-replace('{}')",
                    suggestion.replace('\'', "\\'")
                )),
            );
        }
        menu.append_section(None, &replacements);
        let dictionary = gtk::gio::Menu::new();
        dictionary.append(Some("Add to Dictionary"), Some("win.spell-add"));
        dictionary.append(Some("Ignore While This Runs"), Some("win.spell-ignore"));
        menu.append_section(None, &dictionary);
    }
    if has_selection {
        // GtkSourceView's own entries, by the action it installs:
        // `source.change-case` takes the case to change to as its
        // parameter.
        let case = gtk::gio::Menu::new();
        case.append(Some("All Upper Case"), Some("source.change-case('upper')"));
        case.append(Some("All Lower Case"), Some("source.change-case('lower')"));
        case.append(Some("Invert Case"), Some("source.change-case('toggle')"));
        case.append(Some("Title Case"), Some("source.change-case('title')"));
        let holder = gtk::gio::Menu::new();
        holder.append_submenu(Some("Change Case"), &case);
        menu.append_section(None, &holder);
    }
    menu
}

/// Applies the project root's `editor` overrides (font family, size,
/// tab width) to this view — the Mac's per-project settings, GTK
/// edition. Font settings ride a per-view CSS provider; the global
/// size provider stays the fallback. (The per-widget style context is
/// deprecated upstream but remains the one per-view hook.)
#[allow(deprecated)]
pub fn apply_project_editor_overrides(page: &Rc<Page>) {
    let Some(path) = page.path.borrow().clone() else { return };
    let Some(root) = textchum_core::workspace::project_root_for(Path::new(&path)) else {
        return;
    };
    let overrides = Shell::instance()
        .config
        .borrow()
        .editor_overrides_json(&root.to_string_lossy());
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&overrides) else {
        return;
    };
    if let Some(width) = parsed["tab_width"].as_u64() {
        page.view.set_tab_width((width as u32).clamp(1, 16));
    }
    let family = parsed["font_family"].as_str().unwrap_or("");
    let size = parsed["font_size"].as_f64();
    if family.is_empty() && size.is_none() {
        return;
    }
    let mut rules = String::from("textview {");
    if !family.is_empty() {
        rules.push_str(&format!(" font-family: \"{family}\";"));
    }
    if let Some(size) = size {
        rules.push_str(&format!(" font-size: {size}pt;"));
    }
    rules.push_str(" }");
    let provider = gtk::CssProvider::new();
    provider.load_from_string(&rules);
    page.view
        .style_context()
        .add_provider(&provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1);
}

fn install_hover(page: &Rc<Page>) {
    let motion = gtk::EventControllerMotion::new();
    let timer: Rc<Cell<Option<glib::SourceId>>> = Rc::new(Cell::new(None));
    let weak = Rc::downgrade(page);
    {
        let timer = Rc::clone(&timer);
        motion.connect_motion(move |_, x, y| {
            let Some(page) = weak.upgrade() else { return };
            page.hover_popover.popdown();
            if let Some(previous) = timer.take() {
                previous.remove();
            }
            // Off unless the configuration says otherwise; the
            // deliberate at-caret command ignores the toggle.
            if !Shell::instance().config.borrow().hover_docs() {
                return;
            }
            let timer_inner = Rc::clone(&timer);
            let weak = Rc::downgrade(&page);
            let source = glib::timeout_add_local_once(
                std::time::Duration::from_millis(500),
                move || {
                    timer_inner.set(None);
                    let Some(page) = weak.upgrade() else { return };
                    let (bx, by) = page.view.window_to_buffer_coords(
                        gtk::TextWindowType::Widget,
                        x as i32,
                        y as i32,
                    );
                    let Some(iter) = page.view.iter_at_location(bx, by) else {
                        return;
                    };
                    request_hover(&page, iter, false);
                },
            );
            timer.set(Some(source));
        });
    }
    {
        let timer = Rc::clone(&timer);
        motion.connect_leave(move |_| {
            if let Some(previous) = timer.take() {
                previous.remove();
            }
        });
    }
    page.view.add_controller(motion);
}

/// Shows hover documentation for the symbol under the caret — works
/// even with mouse hover switched off; this is the deliberate ask.
pub fn hover_at_caret(page: &Rc<Page>) {
    let caret = page.buffer.iter_at_mark(&page.buffer.get_insert());
    request_hover(page, caret, true);
}

/// Asks the server what is at `iter` and shows the balloon there. A
/// passive mouse rest (`deliberate == false`) only asks over
/// identifier characters outside comments — whitespace, punctuation,
/// and comments have no documentation, and an empty answer still
/// costs a round trip and a popover flicker.
fn request_hover(page: &Rc<Page>, iter: gtk::TextIter, deliberate: bool) {
    let Some(path) = page.path.borrow().clone() else { return };
    let line = iter.line();
    let line_start = page
        .buffer
        .iter_at_line(line)
        .unwrap_or_else(|| page.buffer.start_iter());
    let character = page
        .buffer
        .text(&line_start, &iter, true)
        .encode_utf16()
        .count();
    if !deliberate {
        let under = iter.char();
        if !(under.is_alphanumeric() || under == '_') {
            return;
        }
        let offset = utf16_offset(&page.buffer, iter.offset());
        // Style index 1 is the canonical comment capture.
        let in_comment = page
            .state
            .borrow()
            .document
            .highlights(offset, offset + 1)
            .ok()
            .into_iter()
            .flatten()
            .any(|span| span.style == 1 && span.start_utf16 <= offset && offset < span.end_utf16);
        if in_comment {
            return;
        }
    }
    let shell = Shell::instance();
    let id = shell.pool.borrow_mut().hover(
        Path::new(&path),
        line.max(0) as u32,
        character as u32,
    );
    let weak = Rc::downgrade(page);
    shell.expect_response(id, move |json| {
        let Some(page) = weak.upgrade() else { return };
        let Some(text) = hover_text(json) else { return };
        page.hover_label.set_markup(&hover_markup(&text));
        let rect = page.view.iter_location(&iter);
        let (wx, wy) = page.view.buffer_to_window_coords(
            gtk::TextWindowType::Widget,
            rect.x(),
            rect.y(),
        );
        page.hover_popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(
            wx,
            wy,
            1,
            rect.height(),
        )));
        page.hover_popover.popup();
    });
}

/// LSP hover Markdown as Pango markup: fenced code blocks and `code`
/// spans in monospace, **bold** and *italic* styled, everything else
/// escaped and left as its literal text.
pub fn hover_markup(text: &str) -> String {
    let mut out = String::new();
    let mut in_fence = false;
    let mut first = true;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if !first {
            out.push('\n');
        }
        first = false;
        let escaped = glib::markup_escape_text(line);
        if in_fence {
            out.push_str(&format!("<tt>{escaped}</tt>"));
        } else {
            out.push_str(&inline_markup(&escaped));
        }
    }
    out
}

/// Inline Markdown over already-escaped text: **bold**, *italic*, and
/// `code`, non-greedy and unnested — hover text, not a renderer.
fn inline_markup(escaped: &str) -> String {
    fn wrap(text: &str, delimiter: &str, open: &str, close: &str) -> String {
        let mut out = String::new();
        let mut rest = text;
        loop {
            let Some(start) = rest.find(delimiter) else {
                out.push_str(rest);
                return out;
            };
            let after = &rest[start + delimiter.len()..];
            let Some(length) = after.find(delimiter) else {
                out.push_str(rest);
                return out;
            };
            if length == 0 {
                // "**" with nothing inside: literal.
                out.push_str(&rest[..start + delimiter.len() * 2]);
                rest = &after[delimiter.len()..];
                continue;
            }
            out.push_str(&rest[..start]);
            out.push_str(open);
            out.push_str(&after[..length]);
            out.push_str(close);
            rest = &after[length + delimiter.len()..];
        }
    }
    let bolded = wrap(escaped, "**", "<b>", "</b>");
    let coded = wrap(&bolded, "`", "<tt>", "</tt>");
    wrap(&coded, "*", "<i>", "</i>")
}

/// Human text from a hover result: MarkupContent, a bare string, or an
/// array of either.
pub fn hover_text(json: &str) -> Option<String> {
    let parsed = serde_json::from_str::<serde_json::Value>(json).ok()?;
    let contents = parsed.get("contents")?;
    fn text_of(value: &serde_json::Value) -> Option<String> {
        if let Some(text) = value.as_str() {
            return Some(text.to_string());
        }
        value["value"].as_str().map(str::to_string)
    }
    let text = match contents {
        serde_json::Value::Array(parts) => parts
            .iter()
            .filter_map(text_of)
            .collect::<Vec<_>>()
            .join("\n"),
        other => text_of(other)?,
    };
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}


/// The preview's page furniture: the shell owns the chrome around the
/// core's HTML, including how Hugo's front matter and shortcode
/// placeholders look.
const PREVIEW_STYLE: &str = r#"<style>
:root { color-scheme: light dark; }
body { font: 15px/1.6 system-ui, sans-serif; margin: 0; padding: 1.5em 2em; }
h1, h2 { border-bottom: 1px solid rgba(128,128,128,.3); padding-bottom: .3em; }
code { font-family: monospace; font-size: .9em; background: rgba(128,128,128,.15);
       border-radius: 4px; padding: .1em .35em; }
pre { background: rgba(128,128,128,.12); border-radius: 6px; padding: .8em 1em;
      overflow-x: auto; }
pre code { background: none; padding: 0; }
blockquote { border-left: 4px solid rgba(128,128,128,.4); margin-left: 0;
             padding-left: 1em; opacity: .85; }
.front-matter { display: grid; grid-template-columns: auto 1fr; gap: .15em 1em;
                margin: 0 0 1.4em; padding: .8em 1em; border-radius: 6px;
                background: rgba(128,128,128,.10);
                border-left: 3px solid rgba(128,128,128,.45); font-size: .9em; }
.front-matter dt { grid-column: 1; margin: 0; font-weight: 600; opacity: .75; }
.front-matter dd { grid-column: 2; margin: 0; font-family: monospace; }
.shortcode { display: inline-block; padding: .05em .5em; border-radius: 999px;
             font-size: .85em; font-family: monospace;
             background: rgba(128,128,128,.18);
             border: 1px solid rgba(128,128,128,.35); }
</style>"#;

fn update_preview(page: &Rc<Page>) {
    let Some(web) = &page.preview else { return };
    if !web.is_visible() {
        return;
    }
    if let Some(html) = page.state.borrow().document.markdown_html() {
        web.load_html(&format!("{PREVIEW_STYLE}{html}"), None);
    }
}
