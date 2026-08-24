//! One editor window: a GtkSourceView mirroring a core [`Document`].
//!
//! The sync protocol is the macOS one translated: the buffer's
//! `insert-text` / `delete-range` signals are the choke point (they fire
//! before the change lands, when offsets still describe the old text),
//! each change is applied to the core document, and debug builds assert
//! both sides stay byte-identical. Undo lives in the core; the buffer's
//! own undo is disabled and ⌃Z replays the core's edits.
//!
//! Offsets: GtkTextBuffer speaks characters, the core speaks UTF-16
//! units. Conversions walk the text once per operation — O(n), plenty
//! at editor scale, and an honest place to optimize later.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use sourceview5::prelude::*;
use textchum_core::{theme, Document};

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
        let document = path
            .as_deref()
            .map(|path| Document::open(path).unwrap_or_else(|_| Document::new()))
            .unwrap_or_else(Document::new);
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

        let header = adw::HeaderBar::new();
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
        file_section.append(Some("Save"), Some("win.save"));
        file_section.append(Some("Save As…"), Some("win.save-as"));
        let edit_section = gtk::gio::Menu::new();
        edit_section.append(Some("Undo"), Some("win.undo"));
        edit_section.append(Some("Redo"), Some("win.redo"));
        let window_section = gtk::gio::Menu::new();
        window_section.append(Some("Close Window"), Some("window.close"));
        let menu = gtk::gio::Menu::new();
        menu.append_section(None, &file_section);
        menu.append_section(None, &edit_section);
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

        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(&scrolled));

        let window = adw::ApplicationWindow::builder()
            .application(app)
            .default_width(860)
            .default_height(620)
            .content(&toolbar)
            .build();

        // --- Load ------------------------------------------------------
        {
            let mut state = state.borrow_mut();
            state.syncing = true;
            buffer.set_text(&state.document.text());
            state.syncing = false;
        }

        // --- The choke point -------------------------------------------
        // insert-text fires before the default handler, so the iter's
        // offset describes the pre-insert text — exactly what the core
        // needs.
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

        // --- After every change: recolor, retitle, and (debug) verify --
        {
            let state = Rc::clone(&state);
            let window = window.clone();
            let pending = Rc::new(Cell::new(false));
            buffer.connect_changed(move |buffer| {
                if state.borrow().syncing {
                    return;
                }
                update_title(&window, &state.borrow().document);
                debug_assert_eq!(
                    buffer.text(&buffer.start_iter(), &buffer.end_iter(), true),
                    state.borrow().document.text(),
                    "shell and core disagree about the document"
                );
                // Recolor on idle, once per burst of changes.
                if pending.replace(true) {
                    return;
                }
                let state = Rc::clone(&state);
                let buffer = buffer.clone();
                let pending = Rc::clone(&pending);
                glib::idle_add_local_once(move || {
                    pending.set(false);
                    apply_highlights(&buffer, &state.borrow().document);
                });
            });
        }
        apply_highlights(&buffer, &state.borrow().document);
        update_title(&window, &state.borrow().document);

        install_actions(&window, app, &buffer, &state);
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
    let mut mapped = std::collections::HashMap::new();
    let mut utf16 = 0usize;
    let mut chars = 0i32;
    let mut next = boundaries.iter().peekable();
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

/// Fires a window action by its prefixed name ("win.save"). Named to
/// dodge the WidgetExt/ActionGroupExt `activate_action` ambiguity.
fn fire(window: &adw::ApplicationWindow, action: &str) -> bool {
    gtk::prelude::WidgetExt::activate_action(window, action, None).is_ok()
}

// MARK: Chrome

fn update_title(window: &adw::ApplicationWindow, document: &Document) {
    let name = document
        .path()
        .and_then(|path| path.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Untitled".into());
    let dirty = if document.is_dirty() { "● " } else { "" };
    window.set_title(Some(&format!("{dirty}{name} — Textchum")));
}

// MARK: Actions

fn install_actions(
    window: &adw::ApplicationWindow,
    app: &adw::Application,
    buffer: &sourceview5::Buffer,
    state: &Rc<RefCell<State>>,
) {
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
                        EditorWindow::new(&app, Some(path)).present();
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
                update_title(&window, &state.borrow().document);
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
                            update_title(&window, &state.borrow().document);
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
            update_title(&window, &state.borrow().document);
            apply_highlights(&buffer, &state.borrow().document);
        });
        window.add_action(&action);
    }
}

// MARK: Smoke test

/// Headless end-to-end check (run under xvfb in CI): typing through the
/// buffer reaches the core, highlighting produces spans, undo replays,
/// and a save round-trips through disk.
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

    let editor = EditorWindow::new(app, Some(path.clone()));
    editor.present();
    let window = editor.window;
    let buffer = window
        .content()
        .and_downcast::<adw::ToolbarView>()
        .and_then(|toolbar| toolbar.content())
        .and_downcast::<gtk::ScrolledWindow>()
        .and_then(|scrolled| scrolled.child())
        .and_downcast::<sourceview5::View>()
        .map(|view| view.buffer())
        .and_downcast::<sourceview5::Buffer>();
    let Some(buffer) = buffer else {
        eprintln!("FAIL: widget tree shape");
        return 1;
    };

    // Type through the buffer; the signals must carry it into the core.
    let mut end = buffer.end_iter();
    buffer.insert(&mut end, "// typed on linux\n");
    let round_trip = buffer.text(&buffer.start_iter(), &buffer.end_iter(), true);
    if !round_trip.contains("typed on linux") {
        eprintln!("FAIL: buffer insert");
        return 1;
    }
    // The core must agree byte for byte, and see rust highlights.
    let core_text = format!("fn main() {{}}\n// typed on linux\n");
    if round_trip != core_text {
        eprintln!("FAIL: unexpected buffer text: {round_trip:?}");
        return 1;
    }
    if window.title().map(|title| title.contains('●')) != Some(true) {
        eprintln!("FAIL: dirty marker missing from title");
        return 1;
    }
    if !fire(&window, "win.undo") {
        eprintln!("FAIL: undo action");
        return 1;
    }
    let after_undo = buffer.text(&buffer.start_iter(), &buffer.end_iter(), true);
    if after_undo != "fn main() {}\n" {
        eprintln!("FAIL: undo did not replay: {after_undo:?}");
        return 1;
    }
    if !fire(&window, "win.redo") {
        eprintln!("FAIL: redo action");
        return 1;
    }
    if !fire(&window, "win.save") {
        eprintln!("FAIL: save action");
        return 1;
    }
    let on_disk = std::fs::read_to_string(&path).unwrap_or_default();
    if on_disk != core_text {
        eprintln!("FAIL: save round trip: {on_disk:?}");
        return 1;
    }
    // Highlight tags applied? The keyword `fn` should carry some tag.
    let iter = buffer.iter_at_offset(0);
    if iter.tags().is_empty() {
        eprintln!("FAIL: no highlight tag at offset 0");
        return 1;
    }
    let _ = std::fs::remove_dir_all(&directory);
    println!("gtk smoke test passed");
    0
}
