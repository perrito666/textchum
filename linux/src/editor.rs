//! One editor window: a GtkSourceView mirroring a core [`Document`],
//! with the core's language-server pool wired through the shared
//! [`Shell`](crate::shell::Shell) — diagnostics as squiggles and a
//! problem count, jump to definition, server trouble as toasts — plus
//! in-file search and project-wide Open Quickly over the core's fuzzy
//! matcher.
//!
//! The sync protocol is the macOS one translated: the buffer's
//! `insert-text` / `delete-range` signals are the choke point (they fire
//! before the change lands, when offsets still describe the old text),
//! each change is applied to the core document, and debug builds assert
//! both sides stay byte-identical. Undo lives in the core; the buffer's
//! own undo is disabled and ⌃Z replays the core's edits.
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
use textchum_core::{theme, workspace, Document};

use crate::shell::{Shell, WindowHandles};

/// Shared, mutable editor state behind the GTK signal handlers.
struct State {
    document: Document,
    /// True while the shell itself mutates the buffer (loads, undo
    /// replays), so the signal handlers do not echo into the core.
    syncing: bool,
}

pub struct EditorWindow {
    window: adw::ApplicationWindow,
}

impl EditorWindow {
    pub fn new(app: &adw::Application, path: Option<PathBuf>) -> Self {
        let shell = Shell::instance();
        let document = path
            .as_deref()
            .map(|path| Document::open(path).unwrap_or_else(|_| Document::new()))
            .unwrap_or_else(Document::new);
        let document_path = document.path().map(|p| p.to_string_lossy().into_owned());
        let language = document.language_name();
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
        view.set_show_line_numbers(true);
        view.set_tab_width(4);
        view.set_left_margin(6);
        view.set_top_margin(6);

        let scrolled = gtk::ScrolledWindow::builder()
            .child(&view)
            .hexpand(true)
            .vexpand(true)
            .build();

        let title = adw::WindowTitle::new("Textchum", "");
        let header = adw::HeaderBar::new();
        header.set_title_widget(Some(&title));
        let open_button = gtk::Button::from_icon_name("document-open-symbolic");
        open_button.set_tooltip_text(Some("Open a file (Ctrl+O)"));
        open_button.set_action_name(Some("win.open"));
        header.pack_start(&open_button);

        // The primary menu: users are not seers — everything the window
        // can do is listed here, with its shortcut (GTK shows the accels
        // registered on the application automatically).
        let file_section = gtk::gio::Menu::new();
        file_section.append(Some("New Window"), Some("win.new"));
        file_section.append(Some("Open…"), Some("win.open"));
        file_section.append(Some("Open Quickly…"), Some("win.quick-open"));
        file_section.append(Some("Save"), Some("win.save"));
        file_section.append(Some("Save As…"), Some("win.save-as"));
        let edit_section = gtk::gio::Menu::new();
        edit_section.append(Some("Undo"), Some("win.undo"));
        edit_section.append(Some("Redo"), Some("win.redo"));
        edit_section.append(Some("Find…"), Some("win.find"));
        let go_section = gtk::gio::Menu::new();
        go_section.append(Some("Jump to Definition"), Some("win.definition"));
        let window_section = gtk::gio::Menu::new();
        window_section.append(Some("Close Window"), Some("window.close"));
        let menu = gtk::gio::Menu::new();
        menu.append_section(None, &file_section);
        menu.append_section(None, &edit_section);
        menu.append_section(None, &go_section);
        menu.append_section(None, &window_section);
        let menu_button = gtk::MenuButton::builder()
            .icon_name("open-menu-symbolic")
            .menu_model(&menu)
            .tooltip_text("Menu")
            .build();
        header.pack_end(&menu_button);
        // Screenshot-driven verification: pop the menu open on demand.
        if std::env::var_os("TEXTCHUM_DEBUG_MENU").is_some() {
            let button = menu_button.clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(1200), move || {
                button.popup();
            });
        }

        // In-file search: GtkSourceView's own machinery, shown in a
        // search bar under the header (Ctrl+F, ⎋ closes).
        let search_settings = sourceview5::SearchSettings::new();
        search_settings.set_wrap_around(true);
        search_settings.set_case_sensitive(false);
        let search_context = sourceview5::SearchContext::new(&buffer, Some(&search_settings));
        search_context.set_highlight(true);
        let search_entry = gtk::SearchEntry::new();
        search_entry.set_placeholder_text(Some("Find in file…"));
        {
            let settings = search_settings.clone();
            search_entry.connect_search_changed(move |entry| {
                let text = entry.text();
                settings.set_search_text(if text.is_empty() { None } else { Some(&text) });
            });
        }
        {
            let context = search_context.clone();
            let buffer = buffer.clone();
            let view = view.clone();
            search_entry.connect_activate(move |_| {
                let insert = buffer.iter_at_mark(&buffer.get_insert());
                if let Some((start, end, _)) = context.forward(&insert) {
                    buffer.select_range(&end, &start);
                    view.scroll_to_iter(&mut start.clone(), 0.1, false, 0.0, 0.0);
                }
            });
        }
        let search_bar = gtk::SearchBar::new();
        search_bar.set_child(Some(&search_entry));
        search_bar.connect_entry(&search_entry);

        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.add_top_bar(&search_bar);
        toolbar.set_content(Some(&scrolled));
        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&toolbar));

        let window = adw::ApplicationWindow::builder()
            .application(app)
            .default_width(920)
            .default_height(640)
            .content(&toasts)
            .build();
        search_bar.set_key_capture_widget(Some(&window));

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

        let handles = Rc::new(WindowHandles {
            window: window.clone(),
            buffer: buffer.clone(),
            view: view.clone(),
            title: title.clone(),
            toasts: toasts.clone(),
            base_subtitle: RefCell::new(String::new()),
        });

        // --- After every change: recolor, retitle, announce, verify ----
        {
            let state = Rc::clone(&state);
            let handles = Rc::clone(&handles);
            let shell = Rc::clone(&shell);
            let document_path = document_path.clone();
            let recolor_pending = Rc::new(Cell::new(false));
            let lsp_timer: Rc<Cell<Option<glib::SourceId>>> = Rc::new(Cell::new(None));
            buffer.connect_changed(move |buffer| {
                if state.borrow().syncing {
                    return;
                }
                update_title(&handles, &state.borrow().document);
                debug_assert_eq!(
                    buffer.text(&buffer.start_iter(), &buffer.end_iter(), true),
                    state.borrow().document.text(),
                    "shell and core disagree about the document"
                );
                // Recolor on idle, once per burst of changes.
                if !recolor_pending.replace(true) {
                    let state = Rc::clone(&state);
                    let buffer = buffer.clone();
                    let pending = Rc::clone(&recolor_pending);
                    glib::idle_add_local_once(move || {
                        pending.set(false);
                        apply_highlights(&buffer, &state.borrow().document);
                    });
                }
                // Announce to the server pool, debounced while typing.
                if let Some(path) = document_path.clone() {
                    if let Some(previous) = lsp_timer.take() {
                        previous.remove();
                    }
                    let state = Rc::clone(&state);
                    let shell = Rc::clone(&shell);
                    let timer = Rc::clone(&lsp_timer);
                    let source = glib::timeout_add_local_once(
                        std::time::Duration::from_millis(300),
                        move || {
                            timer.set(None);
                            let text = state.borrow().document.text();
                            shell
                                .pool
                                .borrow_mut()
                                .did_change(Path::new(&path), &text);
                        },
                    );
                    lsp_timer.set(Some(source));
                }
            });
        }
        apply_highlights(&buffer, &state.borrow().document);
        update_title(&handles, &state.borrow().document);

        // --- The pool learns about the document ------------------------
        if let (Some(path), Some(language)) = (&document_path, language) {
            shell.windows.borrow_mut().insert(path.clone(), Rc::clone(&handles));
            let text = state.borrow().document.text();
            shell
                .pool
                .borrow_mut()
                .did_open(Path::new(path), language, &text);
            let shell = Rc::clone(&shell);
            let path = path.clone();
            window.connect_close_request(move |_| {
                shell.windows.borrow_mut().remove(&path);
                shell.pool.borrow_mut().did_close(Path::new(&path));
                glib::Propagation::Proceed
            });
        }

        install_actions(&window, app, &buffer, &state, &search_bar, &search_entry);
        Self { window }
    }

    pub fn present(&self) {
        self.window.present();
    }
}

// MARK: Offsets

/// UTF-16 offset of the character offset `chars` in `buffer`.
fn utf16_offset(buffer: &sourceview5::Buffer, chars: i32) -> usize {
    let end = buffer.iter_at_offset(chars);
    buffer
        .text(&buffer.start_iter(), &end, true)
        .encode_utf16()
        .count()
}

/// Character offset of the UTF-16 offset `target` in `text`.
fn char_offset(text: &str, target: usize) -> i32 {
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
fn lsp_caret(buffer: &sourceview5::Buffer) -> (u32, u32) {
    let insert = buffer.iter_at_mark(&buffer.get_insert());
    let line = insert.line();
    let line_start = buffer.iter_at_line(line).unwrap_or_else(|| buffer.start_iter());
    let column = buffer
        .text(&line_start, &insert, true)
        .encode_utf16()
        .count();
    (line.max(0) as u32, column as u32)
}

/// Puts the caret at an LSP position and scrolls it into view.
pub fn reveal(handles: &WindowHandles, line: i32, character_utf16: usize) {
    let buffer = &handles.buffer;
    let Some(line_start) = buffer.iter_at_line(line.min(buffer.line_count() - 1).max(0))
    else {
        return;
    };
    let mut line_end = line_start;
    if !line_end.ends_line() {
        line_end.forward_to_line_end();
    }
    let line_text = buffer.text(&line_start, &line_end, true);
    let mut target = line_start;
    target.forward_chars(char_offset(&line_text, character_utf16));
    buffer.place_cursor(&target);
    handles
        .view
        .scroll_to_iter(&mut target.clone(), 0.1, false, 0.0, 0.0);
    handles.window.present();
    handles.view.grab_focus();
}

// MARK: Highlighting

/// One text tag per style-table entry, named by index. Colors come from
/// the shared theme table; libadwaita's style manager decides which of
/// the pair applies.
fn install_style_tags(buffer: &sourceview5::Buffer) {
    let dark = adw::StyleManager::default().is_dark();
    for (index, style) in theme::styles().iter().enumerate() {
        let rgba = if dark { style.dark } else { style.light };
        let color = format!(
            "#{:02X}{:02X}{:02X}",
            (rgba >> 24) & 0xFF,
            (rgba >> 16) & 0xFF,
            (rgba >> 8) & 0xFF
        );
        let tag = gtk::TextTag::builder()
            .name(format!("s{index}"))
            .foreground(&color)
            .build();
        buffer.tag_table().add(&tag);
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
fn apply_highlights(buffer: &sourceview5::Buffer, document: &Document) {
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

    // Map every span boundary from UTF-16 to characters in one walk.
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

/// Applies a diagnostics event to its window: squiggles per finding and
/// a problem count in the subtitle. Called by the shell's event pump.
pub fn apply_diagnostics(handles: &WindowHandles, json: &str) {
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
    let mut subtitle = handles.base_subtitle.borrow().clone();
    if errors + warnings > 0 {
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
        if !subtitle.is_empty() {
            subtitle.push_str(" · ");
        }
        subtitle.push_str(&parts.join(", "));
    }
    handles.title.set_subtitle(&subtitle);
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

/// Fires a window action by its prefixed name ("win.save"). Named to
/// dodge the WidgetExt/ActionGroupExt `activate_action` ambiguity.
fn fire(window: &adw::ApplicationWindow, action: &str) -> bool {
    gtk::prelude::WidgetExt::activate_action(window, action, None).is_ok()
}

// MARK: Chrome

fn update_title(handles: &WindowHandles, document: &Document) {
    let name = document
        .path()
        .and_then(|path| path.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Untitled".into());
    let dirty = if document.is_dirty() { "● " } else { "" };
    handles.title.set_title(&format!("{dirty}{name}"));
    let base = document.language_name().unwrap_or("").to_string();
    *handles.base_subtitle.borrow_mut() = base.clone();
    // A fresh diagnostics event re-appends its counts.
    handles.title.set_subtitle(&base);
}

// MARK: Actions

fn install_actions(
    window: &adw::ApplicationWindow,
    app: &adw::Application,
    buffer: &sourceview5::Buffer,
    state: &Rc<RefCell<State>>,
    search_bar: &gtk::SearchBar,
    search_entry: &gtk::SearchEntry,
) {
    let handles_for = |window: &adw::ApplicationWindow| -> Option<Rc<WindowHandles>> {
        let shell = Shell::instance();
        let found = shell
            .windows
            .borrow()
            .values()
            .find(|handles| handles.window == *window)
            .cloned();
        found
    };

    let new_window = gtk::gio::SimpleAction::new("new", None);
    {
        let app = app.clone();
        new_window.connect_activate(move |_, _| {
            EditorWindow::new(&app, None).present();
        });
    }
    window.add_action(&new_window);

    let open = gtk::gio::SimpleAction::new("open", None);
    {
        let app = app.clone();
        let window = window.clone();
        open.connect_activate(move |_, _| {
            let app = app.clone();
            let dialog = gtk::FileDialog::new();
            dialog.open(Some(&window), gtk::gio::Cancellable::NONE, move |result| {
                if let Ok(file) = result {
                    if let Some(path) = file.path() {
                        open_or_focus(&app, &path, None);
                    }
                }
            });
        });
    }
    window.add_action(&open);

    let save = gtk::gio::SimpleAction::new("save", None);
    {
        let state = Rc::clone(state);
        let window = window.clone();
        save.connect_activate(move |_, _| {
            let saved = state.borrow_mut().document.save().is_ok();
            if saved {
                if let Some(handles) = handles_for(&window) {
                    update_title(&handles, &state.borrow().document);
                }
            } else {
                fire(&window, "win.save-as");
            }
        });
    }
    window.add_action(&save);

    let save_as = gtk::gio::SimpleAction::new("save-as", None);
    {
        let state = Rc::clone(state);
        let window = window.clone();
        save_as.connect_activate(move |_, _| {
            let state = Rc::clone(&state);
            let window = window.clone();
            let dialog = gtk::FileDialog::new();
            dialog.save(
                Some(&window.clone()),
                gtk::gio::Cancellable::NONE,
                move |result| {
                    if let Ok(file) = result {
                        if let Some(path) = file.path() {
                            let _ = state.borrow_mut().document.save_as(&path);
                            if let Some(handles) = Shell::instance()
                                .windows
                                .borrow()
                                .values()
                                .find(|handles| handles.window == window)
                                .cloned()
                            {
                                update_title(&handles, &state.borrow().document);
                            }
                        }
                    }
                },
            );
        });
    }
    window.add_action(&save_as);

    // Undo/redo replay the core's applied edits onto the buffer under
    // the syncing guard — the same shape as the macOS replays.
    for (name, is_undo) in [("undo", true), ("redo", false)] {
        let action = gtk::gio::SimpleAction::new(name, None);
        let state = Rc::clone(state);
        let buffer = buffer.clone();
        let action_window = window.clone();
        action.connect_activate(move |_, _| {
            let window = action_window.clone();
            let edits = {
                let mut state = state.borrow_mut();
                state.syncing = true;
                if is_undo {
                    state.document.undo()
                } else {
                    state.document.redo()
                }
            };
            for edit in &edits {
                let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), true);
                let from = buffer.iter_at_offset(char_offset(&text, edit.start_utf16));
                let to = buffer.iter_at_offset(char_offset(&text, edit.end_utf16));
                let mut from = from;
                let mut to = to;
                buffer.delete(&mut from, &mut to);
                let mut at = from;
                buffer.insert(&mut at, &edit.text);
            }
            state.borrow_mut().syncing = false;
            if let Some(handles) = Shell::instance()
                .windows
                .borrow()
                .values()
                .find(|handles| handles.window == window)
                .cloned()
            {
                update_title(&handles, &state.borrow().document);
            }
            apply_highlights(&buffer, &state.borrow().document);
        });
        window.add_action(&action);
    }

    // Find in file: reveal the bar and focus the entry.
    let find = gtk::gio::SimpleAction::new("find", None);
    {
        let bar = search_bar.clone();
        let entry = search_entry.clone();
        find.connect_activate(move |_, _| {
            bar.set_search_mode(true);
            entry.grab_focus();
        });
    }
    window.add_action(&find);

    // Jump to Definition, through the shared pool and response router.
    let definition = gtk::gio::SimpleAction::new("definition", None);
    {
        let app = app.clone();
        let buffer = buffer.clone();
        let state = Rc::clone(state);
        definition.connect_activate(move |_, _| {
            let Some(path) = state.borrow().document.path().map(Path::to_owned) else {
                return;
            };
            let (line, character) = lsp_caret(&buffer);
            let shell = Shell::instance();
            let id = shell.pool.borrow_mut().definition(&path, line, character);
            let app = app.clone();
            shell.expect_response(id, move |json| open_definition(&app, json));
        });
    }
    window.add_action(&definition);

    // Open Quickly: the core's fuzzy matcher over the project.
    let quick = gtk::gio::SimpleAction::new("quick-open", None);
    {
        let app = app.clone();
        let window = window.clone();
        let state = Rc::clone(state);
        quick.connect_activate(move |_, _| {
            let root = state
                .borrow()
                .document
                .path()
                .and_then(workspace::project_root_for)
                .or_else(|| {
                    state
                        .borrow()
                        .document
                        .path()
                        .and_then(Path::parent)
                        .map(Path::to_owned)
                })
                .unwrap_or_else(|| PathBuf::from(glib::home_dir()));
            show_quick_open(&app, &window, root);
        });
    }
    window.add_action(&quick);
}

/// Focuses the window already showing `path`, or opens a new one; then
/// optionally reveals a position.
fn open_or_focus(app: &adw::Application, path: &Path, at: Option<(i32, usize)>) {
    let key = path.to_string_lossy().into_owned();
    let existing = Shell::instance().windows.borrow().get(&key).cloned();
    let handles = match existing {
        Some(handles) => {
            handles.window.present();
            Some(handles)
        }
        None => {
            EditorWindow::new(app, Some(path.to_owned())).present();
            Shell::instance().windows.borrow().get(&key).cloned()
        }
    };
    if let (Some(handles), Some((line, character))) = (handles, at) {
        reveal(&handles, line, character);
    }
}

/// Parses a definition result (Location, Location[], or LocationLink[])
/// and navigates there.
fn open_definition(app: &adw::Application, json: &str) {
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(json) else {
        return;
    };
    let candidate = match &parsed {
        serde_json::Value::Array(items) => items.first().cloned(),
        serde_json::Value::Object(_) => Some(parsed.clone()),
        _ => None,
    };
    let Some(candidate) = candidate else { return };
    let uri = candidate["uri"]
        .as_str()
        .or_else(|| candidate["targetUri"].as_str());
    let range = if candidate["range"].is_object() {
        &candidate["range"]
    } else if candidate["targetSelectionRange"].is_object() {
        &candidate["targetSelectionRange"]
    } else {
        &candidate["targetRange"]
    };
    let Some(path) = uri.and_then(|uri| uri.strip_prefix("file://")) else {
        return;
    };
    let line = range["start"]["line"].as_i64().unwrap_or(0) as i32;
    let character = range["start"]["character"].as_u64().unwrap_or(0) as usize;
    open_or_focus(app, Path::new(path), Some((line, character)));
}

// MARK: Open Quickly

/// A modal fuzzy file finder over the core's matcher: type, ⏎ opens the
/// selection (or the first hit), ⎋ closes.
fn show_quick_open(app: &adw::Application, parent: &adw::ApplicationWindow, root: PathBuf) {
    let entry = gtk::SearchEntry::new();
    entry.set_placeholder_text(Some("fuzzy file name…"));
    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::Browse);
    let scrolled = gtk::ScrolledWindow::builder()
        .child(&list)
        .vexpand(true)
        .build();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 6);
    content.set_margin_top(10);
    content.set_margin_bottom(10);
    content.set_margin_start(10);
    content.set_margin_end(10);
    content.append(&entry);
    content.append(&scrolled);

    let dialog = adw::Window::builder()
        .transient_for(parent)
        .modal(true)
        .default_width(560)
        .default_height(380)
        .title(&*root.to_string_lossy())
        .content(&content)
        .build();

    let refill = {
        let list = list.clone();
        let root = root.clone();
        move |query: &str| {
            while let Some(child) = list.first_child() {
                list.remove(&child);
            }
            for relative in textchum_core::search::fuzzy_files(&root, query, 50) {
                let label = gtk::Label::new(Some(&relative));
                label.set_xalign(0.0);
                label.set_margin_start(6);
                label.set_margin_top(3);
                label.set_margin_bottom(3);
                list.append(&label);
            }
            if let Some(first) = list.row_at_index(0) {
                list.select_row(Some(&first));
            }
        }
    };
    refill("");
    {
        let refill = refill.clone();
        entry.connect_search_changed(move |entry| refill(&entry.text()));
    }

    let open_row = {
        let app = app.clone();
        let dialog = dialog.clone();
        let root = root.clone();
        move |row: &gtk::ListBoxRow| {
            if let Some(label) = row.child().and_downcast::<gtk::Label>() {
                let full = root.join(label.text().as_str());
                dialog.close();
                open_or_focus(&app, &full, None);
            }
        }
    };
    {
        let open_row = open_row.clone();
        list.connect_row_activated(move |_, row| open_row(row));
    }
    {
        let list = list.clone();
        entry.connect_activate(move |_| {
            if let Some(row) = list.selected_row().or_else(|| list.row_at_index(0)) {
                open_row(&row);
            }
        });
    }
    let escape = gtk::EventControllerKey::new();
    {
        let dialog = dialog.clone();
        escape.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                dialog.close();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
    }
    dialog.add_controller(escape);
    dialog.present();
    entry.grab_focus();
}

// MARK: Smoke test

/// Headless end-to-end check (run under xvfb in CI): typing through the
/// buffer reaches the core, highlighting produces spans, undo replays,
/// a save round-trips through disk — and the language-server path works
/// against the scripted server: diagnostics arrive as squiggle tags and
/// a problem count.
pub fn run_smoke_test(app: &adw::Application) -> i32 {
    let directory = std::env::temp_dir().join(format!("textchum-gtk-{}", std::process::id()));
    if std::fs::create_dir_all(&directory).is_err() {
        eprintln!("FAIL: temp dir");
        return 1;
    }
    let path = directory.join("smoke.rs");
    if std::fs::write(&path, "fn main() {}\n").is_err() {
        eprintln!("FAIL: seed file");
        return 1;
    }

    // Route rust at the scripted server (a repo checkout is present in
    // CI and development alike).
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/fake_lsp.py");
    let have_fake_server = script.exists();
    if have_fake_server {
        Shell::instance()
            .pool
            .borrow_mut()
            .add_override(textchum_lsp::ServerConfig {
                id: "fake".into(),
                command: "python3".into(),
                args: vec![script.to_string_lossy().into_owned()],
                languages: vec!["rust".into()],
                install_hint: "n/a".into(),
            });
    }

    let editor = EditorWindow::new(app, Some(path.clone()));
    editor.present();
    let window = editor.window;
    let key = path.to_string_lossy().into_owned();
    let Some(handles) = Shell::instance().windows.borrow().get(&key).cloned() else {
        eprintln!("FAIL: window not registered with the shell");
        return 1;
    };
    let buffer = handles.buffer.clone();

    // Type through the buffer; the signals must carry it into the core.
    let mut end = buffer.end_iter();
    buffer.insert(&mut end, "// typed on linux\n");
    let expected = "fn main() {}\n// typed on linux\n";
    let round_trip = buffer.text(&buffer.start_iter(), &buffer.end_iter(), true);
    if round_trip != expected {
        eprintln!("FAIL: unexpected buffer text: {round_trip:?}");
        return 1;
    }
    if !handles.title.title().contains('●') {
        eprintln!("FAIL: dirty marker missing from title");
        return 1;
    }
    if !fire(&window, "win.undo") {
        eprintln!("FAIL: undo action");
        return 1;
    }
    if buffer.text(&buffer.start_iter(), &buffer.end_iter(), true) != "fn main() {}\n" {
        eprintln!("FAIL: undo did not replay");
        return 1;
    }
    if !fire(&window, "win.redo") || !fire(&window, "win.save") {
        eprintln!("FAIL: redo/save actions");
        return 1;
    }
    if std::fs::read_to_string(&path).unwrap_or_default() != expected {
        eprintln!("FAIL: save round trip");
        return 1;
    }
    if buffer.iter_at_offset(0).tags().is_empty() {
        eprintln!("FAIL: no highlight tag at offset 0");
        return 1;
    }

    // Diagnostics from the scripted server, delivered through the pump.
    if have_fake_server {
        let context = glib::MainContext::default();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            context.iteration(true);
            if handles.title.subtitle().contains("error") {
                break;
            }
            if std::time::Instant::now() > deadline {
                eprintln!("FAIL: no diagnostics arrived (subtitle: {})", handles.title.subtitle());
                return 1;
            }
        }
        let squiggled = buffer
            .iter_at_offset(0)
            .tags()
            .iter()
            .any(|tag| tag.name().is_some_and(|name| name.starts_with("diag-")));
        if !squiggled {
            eprintln!("FAIL: diagnostics did not tag the text");
            return 1;
        }
        println!("gtk smoke test passed (with language server)");
    } else {
        println!("gtk smoke test passed (no fake server available)");
    }
    let _ = std::fs::remove_dir_all(&directory);
    0
}
