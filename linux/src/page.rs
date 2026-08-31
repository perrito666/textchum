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
use textchum_core::{changes::ChangeKind, indent, theme, Document};
use webkit6::prelude::*;

use crate::shell::{PageHandles, Shell};
use textchum_core::t;
use textchum_core::i18n::{fill, tr};

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
    /// The document this is a view of. The text, the path, the folds
    /// and the file monitor live there, so two views of one file agree
    /// about all of them.
    pub document: Rc<crate::shell::OpenDocument>,
    /// The whole tab child: the scrolled view alone, or a paned with
    /// the Markdown preview beside it.
    pub root: gtk::Widget,
    /// Present for Markdown documents: the live preview.
    pub preview: Option<webkit6::WebView>,

    /// The hover balloon and its content label.
    hover_popover: gtk::Popover,
    hover_label: gtk::Label,
    /// The completion popup and its current candidates.
    completion: CompletionState,
    /// The git change bar, and the marks it draws: line number to kind.
    change_bar: gtk::DrawingArea,
    change_marks: RefCell<Vec<(i32, ChangeKind)>>,
    /// The character a context-menu command is about, while one runs.
    /// A right-click does not move the caret, and the menu is about
    /// what was clicked.
    pub context_offset: Cell<Option<i32>>,
    /// The pinned-context rows over the top of the view, and the lines
    /// they currently show.
    context_strip: gtk::Box,
    pub context_pins: RefCell<Vec<usize>>,
    /// What the pins said when last drawn — an edit above moves text
    /// under unchanged line numbers, and only a real change is worth
    /// rebuilding five rows for.
    context_texts: RefCell<Vec<String>>,

}

/// The completion popup: a popover under the caret, filtered by the
/// word being typed; ↑/↓ move, ⏎/⇥ accept, ⎋ dismisses — and it never
/// steals the keyboard, the view keeps focus throughout.
struct CompletionState {
    popover: gtk::Popover,
    list: gtk::ListBox,
    /// (label, insert text, whether the insert text is a snippet body)
    /// per row.
    items: RefCell<Vec<(String, String, bool)>>,
    /// Character offset where the word being completed starts.
    word_start: Cell<i32>,
}

/// The preview shows this document and never anything else.
///
/// A link clicked in it used to navigate the pane, which has no back
/// button, no history and no address bar: the document was gone until
/// it was edited or the pane was closed and reopened. A link goes to
/// the browser, which is where a link a reader wants to follow belongs.
pub fn install_preview_link_policy(web: &webkit6::WebView) {
    web.connect_decide_policy(|web, decision, kind| {
        if kind != webkit6::PolicyDecisionType::NavigationAction {
            return false;
        }
        let Some(decision) = decision.downcast_ref::<webkit6::NavigationPolicyDecision>() else {
            return false;
        };
        let Some(mut action) = decision.navigation_action() else { return false };
        // The template and the rendered document arrive as content, not
        // as a click.
        if action.navigation_type() != webkit6::NavigationType::LinkClicked {
            return false;
        }
        let Some(request) = action.request() else { return false };
        let Some(uri) = request.uri() else { return false };
        // A link into the document itself is a place in this page; the
        // page scrolls to it and stays.
        let here = web.uri().unwrap_or_default();
        if textchum_core::preview::is_place_in_page(&here, &uri) {
            return false;
        }
        decision.ignore();
        let launcher = gtk::UriLauncher::new(&uri);
        launcher.launch(None::<&gtk::Window>, None::<&gtk::gio::Cancellable>, |result| {
            if let Err(error) = result {
                eprintln!("textchum: could not open {error}");
            }
        });
        true
    });
}

impl Page {
    /// Opens a file — or nothing, for an untitled document — and
    /// returns the first view of it.
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
        apply_source_scheme(&buffer);
        buffer.set_enable_undo(false);
        install_style_tags(&buffer);
        install_diagnostic_tags(&buffer);
        crate::spell::install_tag(&buffer);
        install_occurrence_tag(&buffer);
        install_fold_tag(&buffer);

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

        // The document is registered before any view of it exists, and
        // holds everything the file knows about itself.
        let document = Shell::instance().open_document(
            &buffer,
            &state,
            document_path.as_deref(),
        );
        install_change_handler(&document, last_typed);

        let page = Page::view_of(&document);

        apply_highlights(&buffer, &page.state.borrow().document);
        update_preview(&page);
        crate::spell::run(&page);

        // The pool learns about the document.
        if let (Some(path), Some(language)) = (
            page.path().borrow().clone(),
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

    /// Another view of a document that is already open — the other half
    /// of a split, or the same file in a second tab. The buffer, the
    /// path and the folds belong to the document, so the two views show
    /// one file and not two copies of it.
    pub fn view_of(document: &Rc<crate::shell::OpenDocument>) -> Rc<Page> {
        let state = Rc::clone(&document.state);
        let buffer = document.buffer.clone();

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

        // The git change bar: a stripe per line that differs from the
        // committed file. A sibling of the view rather than a gutter
        // renderer, which would mean subclassing GtkSourceGutterRenderer
        // for three coloured rectangles — and this is what the macOS
        // shell does too, so the two look alike.
        let change_bar = gtk::DrawingArea::new();
        change_bar.set_content_width(5);
        change_bar.set_vexpand(true);
        // The pinned context lies over the top of the view: the first
        // line of each enclosing construct, stacked, while a long
        // method scrolls. An overlay, so switching it off costs no
        // layout and the text keeps its geometry.
        let context_strip = gtk::Box::new(gtk::Orientation::Vertical, 0);
        context_strip.set_valign(gtk::Align::Start);
        context_strip.add_css_class("view");
        context_strip.set_visible(false);
        let editor_overlay = gtk::Overlay::new();
        editor_overlay.set_child(Some(&scrolled));
        editor_overlay.add_overlay(&context_strip);
        editor_overlay.set_hexpand(true);
        let editor_row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        editor_row.append(&change_bar);
        editor_row.append(&editor_overlay);

        // Markdown gets a live preview pane beside the text — the
        // core's HTML, reloaded shortly after edits settle.
        let is_markdown = state.borrow().document.language_name() == Some("markdown");
        let (root, preview) = if is_markdown {
            let web = webkit6::WebView::new();
            web.set_hexpand(true);
            web.set_vexpand(true);
            install_preview_link_policy(&web);
            let paned = gtk::Paned::new(gtk::Orientation::Horizontal);
            paned.set_start_child(Some(&editor_row));
            paned.set_end_child(Some(&web));
            paned.set_position(480);
            paned.set_resize_start_child(true);
            paned.set_resize_end_child(true);
            (paned.upcast::<gtk::Widget>(), Some(web))
        } else {
            (editor_row.clone().upcast::<gtk::Widget>(), None)
        };

        let search_settings = sourceview5::SearchSettings::new();
        search_settings.set_wrap_around(true);
        search_settings.set_case_sensitive(false);
        let search_context = sourceview5::SearchContext::new(&buffer, Some(&search_settings));
        search_context.set_highlight(true);

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
            document: Rc::clone(document),
            root,
            preview,
            hover_popover,
            hover_label,
            completion: CompletionState {
                popover: completion_popover,
                list: completion_list,
                items: RefCell::new(Vec::new()),
                word_start: Cell::new(0),
            },
            change_bar: change_bar.clone(),
            change_marks: RefCell::new(Vec::new()),
            context_offset: Cell::new(None),
            context_strip,
            context_pins: RefCell::new(Vec::new()),
            context_texts: RefCell::new(Vec::new()),
        });
        document.views.borrow_mut().push(Rc::downgrade(&page));
        {
            // The status bar follows the caret.
            let weak = Rc::downgrade(&page);
            buffer.connect_cursor_position_notify(move |_| {
                if let Some(page) = weak.upgrade() {
                    crate::workbench::refresh_status_for(&page);
                }
            });
        }
        install_completion_keys(&page);
        install_snippet_keys(&page);
        install_change_bar(&page);
        install_indent_keys(&page);
        install_wrap_keys(&page);
        install_context_strip(&page);
        // A file opens already differing from its committed self as
        // often as not, so the marks are wanted on the first paint.
        {
            let page = Rc::clone(&page);
            glib::idle_add_local_once(move || refresh_change_marks(&page));
        }
        install_hover(&page);
        if document.views().len() == 1 {
        install_file_monitor(&page);
    }
        install_control_click(&page);
        install_spelling_menu(&page);
        apply_project_editor_overrides(&page);
        page
    }

}
/// Everything an edit sets off, installed once per document: the views
/// are told, the colours are redone, the server pool and the autosave
/// clock hear about it.
fn install_change_handler(
    document: &Rc<crate::shell::OpenDocument>,
    last_typed: Rc<RefCell<String>>,
) {
    let buffer = document.buffer.clone();
    // --- After every change: recolor, announce, verify -----------------
        // --- After every change: recolor, announce, verify -------------
        {
            let document_weak = Rc::downgrade(document);
            let recolor_pending = Rc::new(Cell::new(false));
            let lsp_timer: Rc<Cell<Option<glib::SourceId>>> = Rc::new(Cell::new(None));
            let autosave_timer: Rc<Cell<Option<glib::SourceId>>> = Rc::new(Cell::new(None));
            buffer.connect_changed(move |buffer| {
                let Some(document) = document_weak.upgrade() else { return };
                if document.state.borrow().syncing {
                    return;
                }
                // Every view of the file learns about the edit.
                let views = document.views();
                for view in &views {
                    crate::workbench::refresh_chrome_for(view);
                    refresh_context_strip(view);
                }
                debug_assert_eq!(
                    buffer.text(&buffer.start_iter(), &buffer.end_iter(), true),
                    document.state.borrow().document.text(),
                    "shell and core disagree about the document"
                );
                if !recolor_pending.replace(true) {
                    let document = Rc::clone(&document);
                    let pending = Rc::clone(&recolor_pending);
                    glib::idle_add_local_once(move || {
                        pending.set(false);
                        recolor(&document.buffer);
                        apply_highlights(&document.buffer, &document.state.borrow().document);
                    });
                }
                // Once the burst settles: announce to the server pool
                // and refresh the Markdown preview.
                let document_path = document.path.borrow().clone();
                if document_path.is_some() || views.iter().any(|view| view.preview.is_some()) {
                    if let Some(previous) = lsp_timer.take() {
                        previous.remove();
                    }
                    let document = Rc::clone(&document);
                    let timer = Rc::clone(&lsp_timer);
                    let source = glib::timeout_add_local_once(
                        std::time::Duration::from_millis(300),
                        move || {
                            timer.set(None);
                            if let Some(path) = document.path.borrow().clone() {
                                let text = document.state.borrow().document.text();
                                Shell::instance()
                                    .pool
                                    .borrow_mut()
                                    .did_change(Path::new(&path), &text);
                            }
                            for view in document.views() {
                                update_preview(&view);
                                crate::spell::run(&view);
                                refresh_change_marks(&view);
                            }
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
                    let document = Rc::clone(&document);
                    let timer = Rc::clone(&autosave_timer);
                    let source = glib::timeout_add_local_once(
                        std::time::Duration::from_secs(autosave_after as u64),
                        move || {
                            timer.set(None);
                            if let Some(view) = document.views().first() {
                                autosave(view);
                            }
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
                    // The suggestion belongs under the caret being typed
                    // at, so it goes to the view with the keyboard.
                    if let Some(page) = views.iter().find(|view| view.view.has_focus()).cloned() {
                        glib::timeout_add_local_once(
                            std::time::Duration::from_millis(250),
                            move || request_completion(&page),
                        );
                    }
                } else {
                    for view in &views {
                        view.completion.popover.popdown();
                    }
                }
            });
        }
}

impl Page {
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
        // A commit or a branch switch moves what git compares against,
        // so the marks are recomputed whether or not the text moved.
        refresh_change_marks(self);
        // The pool sees the fresh text like any other change.
        if let Some(path) = self.path().borrow().clone() {
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

/// What a view reads through to its document for.
impl Page {
    /// Where the file lives, once it lives anywhere.
    pub fn path(&self) -> &RefCell<Option<String>> {
        &self.document.path
    }

    /// The watch on that file.
    pub fn monitor(&self) -> &RefCell<Option<gtk::gio::FileMonitor>> {
        &self.document.monitor
    }

    /// The line ranges folded away. Folding a function folds it in
    /// every view of the file.
    fn folded(&self) -> &RefCell<Vec<(i32, i32)>> {
        &self.document.folded
    }
}

// MARK: Folding

/// The tag folded lines wear.
const FOLD_TAG: &str = "folded";

/// Installs that tag on a fresh buffer.
fn install_fold_tag(buffer: &sourceview5::Buffer) {
    let tag = gtk::TextTag::new(Some(FOLD_TAG));
    tag.set_invisible(true);
    buffer.tag_table().add(&tag);
}

/// Folds the block the caret's line opens, or unfolds it when it is
/// already folded.
///
/// What is hidden is everything after the opening line, so the line
/// that says what the block is stays where it was.
pub fn toggle_fold(page: &Rc<Page>) -> bool {
    let buffer = &page.buffer;
    let line = buffer.iter_at_mark(&buffer.get_insert()).line();
    if unfold_line(page, line) {
        return true;
    }
    let folds = page.state.borrow().document.fold_ranges();
    let Some((start, end)) = folds.into_iter().find(|(start, _)| *start as i32 == line) else {
        return false;
    };
    fold(page, start as i32, end as i32)
}

/// Folds one range, by the lines it covers.
fn fold(page: &Rc<Page>, start: i32, end: i32) -> bool {
    let buffer = &page.buffer;
    let Some(mut from) = buffer.iter_at_line(start) else {
        return false;
    };
    if !from.ends_line() {
        from.forward_to_line_end();
    }
    let Some(mut to) = buffer.iter_at_line(end) else {
        return false;
    };
    if !to.ends_line() {
        to.forward_to_line_end();
    }
    if from >= to {
        return false;
    }
    buffer.apply_tag_by_name(FOLD_TAG, &from, &to);
    page.folded().borrow_mut().push((start, end));
    page.document.record_project_state();
    true
}

/// Unfolds the range opening on `line`, if one is folded there.
fn unfold_line(page: &Rc<Page>, line: i32) -> bool {
    let found = page
        .folded()
        .borrow()
        .iter()
        .position(|(start, _)| *start == line);
    let Some(at) = found else { return false };
    let (start, end) = page.folded().borrow_mut().remove(at);
    let buffer = &page.buffer;
    let (Some(from), Some(mut to)) = (buffer.iter_at_line(start), buffer.iter_at_line(end))
    else {
        return false;
    };
    if !to.ends_line() {
        to.forward_to_line_end();
    }
    buffer.remove_tag_by_name(FOLD_TAG, &from, &to);
    true
}

/// Folds every block in the document.
pub fn fold_all(page: &Rc<Page>) -> bool {
    let folds = page.state.borrow().document.fold_ranges();
    let mut any = false;
    for (start, end) in folds {
        let (start, end) = (start as i32, end as i32);
        // A block inside one already folded is hidden either way.
        let inside = page
            .folded()
            .borrow()
            .iter()
            .any(|(from, to)| start > *from && start <= *to);
        if inside {
            continue;
        }
        any |= fold(page, start, end);
    }
    any
}

/// Unfolds everything.
pub fn unfold_all(page: &Rc<Page>) -> bool {
    if page.folded().borrow().is_empty() {
        return false;
    }
    let buffer = &page.buffer;
    buffer.remove_tag_by_name(FOLD_TAG, &buffer.start_iter(), &buffer.end_iter());
    page.folded().borrow_mut().clear();
    page.document.record_project_state();
    true
}

/// Whether anything is folded.
pub fn has_folds(page: &Rc<Page>) -> bool {
    !page.folded().borrow().is_empty()
}

// MARK: Transformations

/// The transformations the menu offers, in the order it offers them.
/// An empty id is a separator.
pub const TRANSFORMS: &[(&str, &str)] = &[
    ("Upper Case", "upper"),
    ("Lower Case", "lower"),
    ("Title Case", "title"),
    ("Invert Case", "invert"),
    ("", ""),
    ("Sort Lines", "sort"),
    ("Sort Lines Reversed", "sort-reversed"),
    ("Remove Duplicate Lines", "dedupe"),
    ("Join Lines", "join"),
    ("Trim Trailing Whitespace", "trim"),
    ("", ""),
    ("Convert to Unix Line Endings (LF)", "lf"),
    ("Convert to Windows Line Endings (CRLF)", "crlf"),
];

/// Transforms the selection, or the whole document when nothing is
/// selected.
///
/// A line-wise transformation is given whole lines: the selection grows
/// to the boundaries around it first, because sorting half a line is
/// not something anyone asked for.
pub fn transform_selection(page: &Rc<Page>, kind: &str) {
    use textchum_core::transform::Transform;
    let Some(transform) = Transform::from_id(kind) else {
        return;
    };
    let buffer = &page.buffer;
    let (mut start, mut end) = match buffer.selection_bounds() {
        Some(bounds) => bounds,
        None => (buffer.start_iter(), buffer.end_iter()),
    };
    if transform.is_line_wise() && buffer.selection_bounds().is_some() {
        start.set_line_offset(0);
        if !end.ends_line() {
            end.forward_to_line_end();
        }
    }
    if start == end {
        return;
    }
    let text = buffer.text(&start, &end, true).to_string();
    let replacement = textchum_core::transform::apply(transform, &text);
    if replacement == text {
        return;
    }

    // One undo step: the whole stretch goes at once.
    buffer.begin_user_action();
    buffer.delete(&mut start, &mut end);
    buffer.insert(&mut start, &replacement);
    buffer.end_user_action();

    // The transformed stretch stays selected, so a second one can
    // follow without selecting it again.
    let from = buffer.iter_at_offset(start.offset() - replacement.chars().count() as i32);
    buffer.select_range(&from, &start);
}

// MARK: Occurrences

/// The tag the other places the selected word appears wear.
pub const OCCURRENCE_TAG: &str = "occurrence";

/// Installs that tag on a fresh buffer: a neutral grey, so it reads as
/// neither the selection, nor a misspelling, nor a finding.
fn install_occurrence_tag(buffer: &sourceview5::Buffer) {
    let tag = gtk::TextTag::new(Some(OCCURRENCE_TAG));
    tag.set_background_rgba(Some(&gtk::gdk::RGBA::new(0.50, 0.50, 0.50, 0.30)));
    buffer.tag_table().add(&tag);
}

/// Marks the other places the selected word appears, over the visible
/// stretch — so a long document costs what a short one does.
///
/// Only a selection that is exactly one word marks anything; the core
/// decides that and answers with nothing otherwise.
pub fn refresh_occurrences(page: &Rc<Page>) {
    let buffer = &page.buffer;
    buffer.remove_tag_by_name(OCCURRENCE_TAG, &buffer.start_iter(), &buffer.end_iter());

    let shell = Shell::instance();
    let (mark, options) = {
        let config = shell.config.borrow();
        (config.mark_occurrences(), config.occurrence_options())
    };
    if !mark {
        return;
    }
    let Some((selection_start, selection_end)) = buffer.selection_bounds() else {
        return;
    };

    // The visible stretch, by the lines the view is showing.
    let visible = page.view.visible_rect();
    let (top, _) = page.view.line_at_y(visible.y());
    let (bottom, _) = page
        .view
        .line_at_y(visible.y() + visible.height());
    let from = top;
    let mut to = bottom;
    to.forward_to_line_end();
    if selection_start < from || selection_end > to {
        return;
    }

    let text = buffer.text(&from, &to, true).to_string();
    let utf16_within = |target: &gtk::TextIter| -> usize {
        buffer.text(&from, target, true).encode_utf16().count()
    };
    let spans = textchum_core::occurrences::selected_word(
        &text,
        utf16_within(&selection_start),
        utf16_within(&selection_end),
    )
    .map(|word| textchum_core::occurrences::occurrences(&text, &word, 0, options))
    .unwrap_or_default();

    let base = from.offset();
    for span in spans {
        let start = base + char_offset(&text, span.start);
        let end = base + char_offset(&text, span.end);
        let start = buffer.iter_at_offset(start);
        let end = buffer.iter_at_offset(end);
        // The selection is already marked, by being selected.
        if start == selection_start && end == selection_end {
            continue;
        }
        buffer.apply_tag_by_name(OCCURRENCE_TAG, &start, &end);
    }
}

/// Whether anything is marked, so Escape only claims the key when it
/// has something to do.
pub fn has_occurrences(page: &Rc<Page>) -> bool {
    let buffer = &page.buffer;
    let Some(tag) = buffer.tag_table().lookup(OCCURRENCE_TAG) else {
        return false;
    };
    let mut at = buffer.start_iter();
    at.forward_to_tag_toggle(Some(&tag))
}

/// Puts the occurrence marks away. Escape says "I am done looking",
/// and leaves the selection where it is.
pub fn clear_occurrences(page: &Rc<Page>) {
    let buffer = &page.buffer;
    buffer.remove_tag_by_name(OCCURRENCE_TAG, &buffer.start_iter(), &buffer.end_iter());
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
/// Where a command that acts "under the caret" should look: the
/// character a context-menu command is about, or the caret.
pub fn lsp_anchor(page: &Page) -> (u32, u32) {
    let Some(offset) = page.context_offset.get() else {
        return lsp_caret(&page.buffer);
    };
    let at = page.buffer.iter_at_offset(offset);
    let line = at.line();
    let line_start = page
        .buffer
        .iter_at_line(line)
        .unwrap_or_else(|| page.buffer.start_iter());
    let column = page
        .buffer
        .text(&line_start, &at, true)
        .encode_utf16()
        .count();
    (line.max(0) as u32, column as u32)
}

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
    let buffer = &handles.document.buffer;
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

/// Gives a buffer the GtkSourceView scheme matching the current colour
/// scheme.
///
/// Without one, the text area's background comes from whatever CSS
/// happens to apply, which is not reliably the dark one: an untitled
/// document came up white on a dark desktop while a document with a
/// language came up dark. GtkSourceView ships Adwaita and Adwaita-dark
/// for this, and naming the scheme makes the background a decision
/// rather than a side effect.
///
/// The scheme's own syntax colours do not interfere: highlighting is
/// off and every token is painted with a tag. What it supplies is the
/// background, the cursor, the selection and the current-line tint.
pub fn apply_source_scheme(buffer: &sourceview5::Buffer) {
    let dark = adw::StyleManager::default().is_dark();
    let manager = sourceview5::StyleSchemeManager::default();
    let scheme = manager
        .scheme(if dark { "Adwaita-dark" } else { "Adwaita" })
        // Older GtkSourceView installs ship classic/classic-dark only.
        .or_else(|| manager.scheme(if dark { "classic-dark" } else { "classic" }));
    buffer.set_style_scheme(scheme.as_ref());
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
    let buffer = &handles.document.buffer;
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
    let mut kept: Vec<crate::shell::Diagnostic> = Vec::new();
    for item in &items {
        let line = item["line"].as_i64().unwrap_or(0) as i32;
        let character = item["character"].as_u64().unwrap_or(0) as usize;
        let end_line = item["endLine"].as_i64().unwrap_or(0) as i32;
        let end_character = item["endCharacter"].as_u64().unwrap_or(0) as usize;
        let severity = item["severity"].as_u64().unwrap_or(1);
        kept.push(crate::shell::Diagnostic {
            line,
            character,
            end_line,
            end_character,
            severity,
            message: item["message"].as_str().unwrap_or_default().to_owned(),
        });
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
    *handles.document.diagnostics.borrow_mut() = kept;
    crate::workbench::refresh_subtitle(handles);
}

/// The diagnostic covering a position, or — failing that — the first
/// one anywhere on its line.
///
/// The line matters because the caret is rarely inside the marked
/// stretch: it is usually at the end of the line being fixed, and
/// answering "nothing here" then would be true and useless.
pub fn diagnostic_at(
    handles: &PageHandles,
    line: i32,
    character: usize,
) -> Option<crate::shell::Diagnostic> {
    let found = handles.document.diagnostics.borrow();
    let covering = found.iter().find(|d| {
        let after_start = (d.line, d.character) <= (line, character);
        // A zero-length finding still marks a spot; give it one
        // character's reach so it can be pointed at.
        let end = if (d.line, d.character) == (d.end_line, d.end_character) {
            (d.end_line, d.end_character + 1)
        } else {
            (d.end_line, d.end_character)
        };
        after_start && (line, character) < end
    });
    covering
        .or_else(|| found.iter().find(|d| d.line == line))
        .cloned()
}

// MARK: Completion

/// Parses a completion result — `CompletionItem[]` or a
/// `CompletionList` — into (label, insert text, is-snippet) triples.
/// Snippet bodies are carried through unexpanded; the core expands one
/// when it is accepted, so both shells expand them the same way.
pub fn parse_completion_items(json: &str) -> Vec<(String, String, bool)> {
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
            let is_snippet = match item["insertTextFormat"].as_u64() {
                Some(format) => format == 2,
                None => looks_like_a_snippet(&insert),
            };
            Some((label, insert, is_snippet))
        })
        .collect()
}

/// Whether a body written without an `insertTextFormat` is a snippet
/// anyway. Servers that leave the field out and write placeholders are
/// common enough to take at their word; a lone `$`, as in a shell
/// variable, is not one.
fn looks_like_a_snippet(insert: &str) -> bool {
    let characters: Vec<char> = insert.chars().collect();
    let mut index = 0;
    while index + 1 < characters.len() {
        if characters[index] == '\\' {
            index += 2;
            continue;
        }
        if characters[index] == '$'
            && (characters[index + 1] == '{' || characters[index + 1].is_ascii_digit())
        {
            return true;
        }
        index += 1;
    }
    false
}

#[cfg(test)]
mod snippet_tests {
    use super::{looks_like_a_snippet, parse_completion_items};

    #[test]
    fn a_declared_format_decides_and_placeholders_speak_for_themselves() {
        let json = r#"[
            {"label": "frob", "insertText": "frob(${1:x})", "insertTextFormat": 2},
            {"label": "plain", "insertText": "cost $5", "insertTextFormat": 1},
            {"label": "undeclared", "insertText": "wrap(${1:x})"}
        ]"#;
        let items = parse_completion_items(json);
        assert_eq!(items[0], ("frob".into(), "frob(${1:x})".into(), true));
        assert_eq!(items[1], ("plain".into(), "cost $5".into(), false));
        assert!(items[2].2);
    }

    #[test]
    fn a_bare_dollar_is_not_a_snippet() {
        assert!(!looks_like_a_snippet("echo $HOME"));
        assert!(!looks_like_a_snippet("cost \\$5"));
        assert!(looks_like_a_snippet("$1 and $2"));
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

/// Asks the server for completions now, for the command that does it
/// deliberately — typing asks on its own after a rest.
pub fn complete_now(page: &Rc<Page>) {
    request_completion(page);
}

fn request_completion(page: &Rc<Page>) {
    let Some(path) = page.path().borrow().clone() else { return };
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

fn show_completions(page: &Rc<Page>, all: Vec<(String, String, bool)>) {
    let (word_start, prefix) = word_before_caret(&page.buffer);
    let prefix_lower = prefix.to_lowercase();
    let matching: Vec<(String, String, bool)> = all
        .into_iter()
        .filter(|(label, _, _)| {
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
    for (label, _, _) in &matching {
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
    let chosen = page
        .completion
        .items
        .borrow()
        .get(index as usize)
        .map(|(_, insert, is_snippet)| (insert.clone(), *is_snippet));
    page.completion.popover.popdown();
    let Some((insert, is_snippet)) = chosen else { return };

    let buffer = &page.buffer;
    let mut start = buffer.iter_at_offset(page.completion.word_start.get());
    let mut caret = buffer.iter_at_mark(&buffer.get_insert());
    // Through the normal buffer path, so the core sees it as typing.
    buffer.delete(&mut start, &mut caret);
    let insert_at = buffer.iter_at_mark(&buffer.get_insert()).offset();
    let origin = utf16_offset(buffer, insert_at);

    // The core expands the body and the buffer inserts what comes back,
    // so the insertion is ordinary typing as far as the sync protocol is
    // concerned. The core is then told where it landed, which is what
    // starts the tabstop session.
    let expanded = {
        let mut state = page.state.borrow_mut();
        state.document.cancel_snippet();
        if is_snippet {
            state.document.expand_snippet(origin, &insert)
        } else {
            insert
        }
    };
    let mut at = buffer.iter_at_mark(&buffer.get_insert());
    buffer.insert(&mut at, &expanded);
    if !is_snippet {
        return;
    }
    let selection = page.state.borrow_mut().document.begin_snippet(origin);
    let Some(region) = selection else { return };
    select_utf16(page, region.start, region.end);
}

/// Selects the UTF-16 range `start..end`, leaving the caret at the
/// start so typing replaces the selection.
fn select_utf16(page: &Rc<Page>, start: usize, end: usize) {
    let buffer = &page.buffer;
    let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), true);
    let from = buffer.iter_at_offset(char_offset(&text, start));
    let to = buffer.iter_at_offset(char_offset(&text, end));
    buffer.select_range(&to, &from);
    page.view
        .scroll_to_iter(&mut buffer.iter_at_offset(char_offset(&text, start)), 0.0, false, 0.0, 0.0);
}

/// Moves to the next tabstop, or back to the previous one, selecting
/// its placeholder. Returns false when no snippet is being filled in,
/// so Tab stays Tab.
pub fn move_through_snippet(page: &Rc<Page>, forward: bool) -> bool {
    let region = {
        let mut state = page.state.borrow_mut();
        if !state.document.snippet_active() {
            return false;
        }
        state.document.snippet_advance(forward)
    };
    let Some(region) = region else { return false };
    select_utf16(page, region.start, region.end);
    true
}

/// Copies the tabstop just typed in to the other places carrying the
/// same number. The buffer's marks carry the caret over the copies, so
/// typing a linked placeholder feels like typing anything else.
fn mirror_snippet_stops(page: &Rc<Page>) {
    let edits = {
        let Ok(mut state) = page.state.try_borrow_mut() else { return };
        if !state.document.snippet_active() {
            return;
        }
        let edits = state.document.snippet_sync();
        if edits.is_empty() {
            return;
        }
        state.syncing = true;
        edits
    };
    let buffer = &page.buffer;
    for edit in &edits {
        let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), true);
        let mut from = buffer.iter_at_offset(char_offset(&text, edit.start_utf16));
        let mut to = buffer.iter_at_offset(char_offset(&text, edit.end_utf16));
        buffer.delete(&mut from, &mut to);
        let mut at = from;
        buffer.insert(&mut at, &edit.text);
    }
    if let Ok(mut state) = page.state.try_borrow_mut() {
        state.syncing = false;
    }
    recolor(buffer);
    apply_highlights(buffer, &page.state.borrow().document);
}

/// Tab, Shift-Tab and Escape while a snippet is being filled in: walk
/// the stops, and give the keys back on the way out. Installed after
/// the completion controller so the popup, when it is up, still gets
/// Tab first.
///
/// The mirroring of linked stops hangs off the same page: every change
/// to the buffer is checked for one, which costs a boolean while no
/// snippet is running.
fn install_snippet_keys(page: &Rc<Page>) {
    let controller = gtk::EventControllerKey::new();
    controller.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = Rc::downgrade(page);
    controller.connect_key_pressed(move |_, key, _, modifiers| {
        let Some(page) = weak.upgrade() else {
            return glib::Propagation::Proceed;
        };
        if page.completion.popover.is_visible() {
            return glib::Propagation::Proceed;
        }
        use gtk::gdk::Key;
        if !page.state.borrow().document.snippet_active() {
            // Escape puts the occurrence marks away, and only those:
            // the selection stays, so the word is still there to act
            // on.
            if key == Key::Escape && has_occurrences(&page) {
                clear_occurrences(&page);
                return glib::Propagation::Stop;
            }
            return glib::Propagation::Proceed;
        }
        match key {
            Key::Tab if !modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK) => {
                if move_through_snippet(&page, true) {
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            }
            Key::ISO_Left_Tab | Key::Tab => {
                if move_through_snippet(&page, false) {
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            }
            Key::Escape => {
                // Escape gives the keys back where the caret is, rather
                // than jumping it to the end of the snippet.
                page.state.borrow_mut().document.cancel_snippet();
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        }
    });
    page.view.add_controller(controller);

    {
        // Deferred to an idle turn rather than done in the handler: a
        // caller that is midway through its own buffer work still holds
        // iterators, and mutating the buffer under it invalidates them.
        // The caret rides on marks, so waiting costs nothing.
        let weak = Rc::downgrade(page);
        page.buffer.connect_changed(move |_| {
            let Some(page) = weak.upgrade() else { return };
            let Ok(state) = page.state.try_borrow() else { return };
            if !state.document.snippet_active() {
                return;
            }
            drop(state);
            let weak = Rc::downgrade(&page);
            glib::idle_add_local_once(move || {
                if let Some(page) = weak.upgrade() {
                    mirror_snippet_stops(&page);
                }
            });
        });
    }
    {
        // A new selection asks a new question; an edit answers the old
        // one differently. Both move the marks.
        let weak = Rc::downgrade(page);
        page.buffer
            .connect_mark_set(move |_, _, mark| {
                let Some(page) = weak.upgrade() else { return };
                let name = mark.name();
                let name = name.as_deref().unwrap_or_default();
                if name != "insert" && name != "selection_bound" {
                    return;
                }
                // Deferred: the caller may still hold iterators, and
                // tagging under it invalidates them.
                let weak = Rc::downgrade(&page);
                glib::idle_add_local_once(move || {
                    if let Some(page) = weak.upgrade() {
                        refresh_occurrences(&page);
                    }
                });
            });
    }
    {
        // Scrolling brings fresh text into view, where the selected
        // word may also appear.
        let weak = Rc::downgrade(page);
        page.view.vadjustment().inspect(|adjustment| {
            adjustment.connect_value_changed(move |_| {
                let Some(page) = weak.upgrade() else { return };
                refresh_occurrences(&page);
            });
        });
    }
    {
        // Clicking away from a snippet is done with it; leaving Tab
        // captured after that would be a mode with no way out.
        let weak = Rc::downgrade(page);
        page.buffer.connect_cursor_position_notify(move |buffer| {
            let Some(page) = weak.upgrade() else { return };
            let Ok(mut state) = page.state.try_borrow_mut() else { return };
            if !state.document.snippet_active() {
                return;
            }
            let caret = buffer.iter_at_mark(&buffer.get_insert()).offset();
            let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), true);
            let utf16 = text
                .chars()
                .take(caret.max(0) as usize)
                .map(char::len_utf16)
                .sum();
            state.document.snippet_caret_moved(utf16);
        });
    }
}

/// Backspace and Tab in a line's leading whitespace.
///
/// Backspace there deletes back to the previous tab stop rather than
/// one space at a time; Tab there lines the line up with the block
/// above, and goes one level deeper once it is already level. Anywhere
/// else in the line both keys are themselves, which is what keeps the
/// behaviour from surprising anyone: it is the position that decides.
/// Typing an opening delimiter over a selection wraps it in the pair
/// instead of replacing it: `hello` and `[` give `[hello]`, with the
/// selection kept on `hello`, so `[({` in a row gives `[({hello})]`.
///
/// The table of pairs is the core's, so both shells wrap the same
/// things. Nothing selected types the delimiter as it always did.
fn install_wrap_keys(page: &Rc<Page>) {
    let controller = gtk::EventControllerKey::new();
    controller.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = Rc::downgrade(page);
    controller.connect_key_pressed(move |_, key, _, modifiers| {
        let Some(page) = weak.upgrade() else {
            return glib::Propagation::Proceed;
        };
        // A shortcut is not typing, and the popups own their keys.
        if page.completion.popover.is_visible()
            || page.state.borrow().document.snippet_active()
            || modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK)
            || modifiers.contains(gtk::gdk::ModifierType::ALT_MASK)
            || modifiers.contains(gtk::gdk::ModifierType::META_MASK)
        {
            return glib::Propagation::Proceed;
        }
        if wrap_selection(&page, key) {
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    page.view.add_controller(controller);
}

/// Follows scrolling with the pinned context.
fn install_context_strip(page: &Rc<Page>) {
    let weak = Rc::downgrade(page);
    scrolled_window_of(page).vadjustment().connect_value_changed(move |_| {
        if let Some(page) = weak.upgrade() {
            refresh_context_strip(&page);
        }
    });
    let weak = Rc::downgrade(page);
    glib::idle_add_local_once(move || {
        if let Some(page) = weak.upgrade() {
            refresh_context_strip(&page);
        }
    });
}

fn scrolled_window_of(page: &Rc<Page>) -> gtk::ScrolledWindow {
    page.view
        .parent()
        .and_downcast::<gtk::ScrolledWindow>()
        .expect("the view lives in its scrolled window")
}

/// Recomputes the pinned context for a page from its scroll position:
/// at most five rows, rebuilt only when the lines change.
pub fn refresh_context_strip(page: &Rc<Page>) {
    let strip = &page.context_strip;
    let enabled = crate::shell::Shell::instance().config.borrow().context_lines();
    let state = page.state.borrow();
    if !enabled || state.document.language_name().is_none() {
        drop(state);
        strip.set_visible(false);
        page.context_pins.borrow_mut().clear();
        return;
    }
    let top = scrolled_window_of(page).vadjustment().value() as i32;
    let (top_iter, _) = page.view.line_at_y(top);
    let lines = state.document.context_lines(top_iter.line() as usize, 5);
    drop(state);
    let buffer = &page.buffer;
    let texts: Vec<String> = lines
        .iter()
        .map(|line| {
            let start = buffer
                .iter_at_line(*line as i32)
                .unwrap_or_else(|| buffer.start_iter());
            let mut end = start.clone();
            if !end.ends_line() {
                end.forward_to_line_end();
            }
            buffer.text(&start, &end, true).to_string()
        })
        .collect();
    if *page.context_pins.borrow() == lines && *page.context_texts.borrow() == texts {
        return;
    }
    *page.context_pins.borrow_mut() = lines.clone();
    *page.context_texts.borrow_mut() = texts.clone();
    while let Some(child) = strip.first_child() {
        strip.remove(&child);
    }
    strip.set_visible(!lines.is_empty());
    for (line, text) in lines.into_iter().zip(texts) {
        let row = gtk::Label::new(Some(&text));
        row.add_css_class("monospace");
        row.set_xalign(0.0);
        row.set_ellipsize(gtk::pango::EllipsizeMode::End);
        row.set_margin_start(6);
        let click = gtk::GestureClick::new();
        let weak = Rc::downgrade(page);
        click.connect_pressed(move |_, _, _, _| {
            let Some(page) = weak.upgrade() else { return };
            let Some(mut at) = page.buffer.iter_at_line(line as i32) else { return };
            page.buffer.place_cursor(&at);
            page.view.scroll_to_iter(&mut at, 0.0, true, 0.0, 0.0);
            page.view.grab_focus();
        });
        row.add_controller(click);
        strip.append(&row);
    }
    // A hairline under the pins, so they read as a shelf and not as
    // the first lines of the view.
    strip.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
}

/// Wraps the selection when `key` is an opening delimiter, and answers
/// whether it did.
fn wrap_selection(page: &Rc<Page>, key: gtk::gdk::Key) -> bool {
    let Some(typed) = key.to_unicode() else { return false };
    let mut encoded = [0u8; 4];
    let typed = typed.encode_utf8(&mut encoded);
    let Some((open, close)) = textchum_core::pairs::wraps(typed) else {
        return false;
    };
    let buffer = &page.buffer;
    let Some((start, end)) = buffer.selection_bounds() else { return false };
    let selected = buffer.text(&start, &end, true).to_string();
    if selected.is_empty() {
        return false;
    }
    // One undo step for the pair, through the same door as any other
    // edit: the buffer, which the choke point mirrors into the core.
    buffer.begin_user_action();
    let start_offset = start.offset();
    buffer.delete(&mut start.clone(), &mut end.clone());
    let mut at = buffer.iter_at_offset(start_offset);
    buffer.insert(&mut at, &format!("{open}{selected}{close}"));
    buffer.end_user_action();
    // The selection stays on what was wrapped, so the next delimiter
    // nests inside this one.
    let inner_start = buffer.iter_at_offset(start_offset + 1);
    let inner_end = buffer.iter_at_offset(start_offset + 1 + selected.chars().count() as i32);
    buffer.select_range(&inner_end, &inner_start);
    true
}

/// The wrap, for the smoke test to drive without a key event.
pub fn wrap_selection_for_test(page: &Rc<Page>, key: gtk::gdk::Key) -> bool {
    wrap_selection(page, key)
}

fn install_indent_keys(page: &Rc<Page>) {
    let controller = gtk::EventControllerKey::new();
    controller.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = Rc::downgrade(page);
    controller.connect_key_pressed(move |_, key, _, modifiers| {
        let Some(page) = weak.upgrade() else {
            return glib::Propagation::Proceed;
        };
        // The completion popup and a running snippet own these keys
        // first.
        if page.completion.popover.is_visible()
            || page.state.borrow().document.snippet_active()
            || !modifiers.is_empty()
        {
            return glib::Propagation::Proceed;
        }
        use gtk::gdk::Key;
        match key {
            Key::BackSpace => {
                if delete_back_one_indent(&page) {
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            }
            Key::Tab => {
                if indent_to_block_above(&page) {
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            }
            _ => glib::Propagation::Proceed,
        }
    });
    page.view.add_controller(controller);
}

/// The caret's line, and the text of it before the caret.
fn line_before_caret(page: &Rc<Page>) -> (gtk::TextIter, String, String) {
    let buffer = &page.buffer;
    let caret = buffer.iter_at_mark(&buffer.get_insert());
    let mut start = caret;
    start.set_line_offset(0);
    let before = buffer.text(&start, &caret, true).to_string();
    let mut end = caret;
    if !end.ends_line() {
        end.forward_to_line_end();
    }
    let whole = buffer.text(&start, &end, true).to_string();
    (start, before, whole)
}

fn tab_width(page: &Rc<Page>) -> usize {
    let _ = page;
    Shell::instance().config.borrow().tab_width() as usize
}

fn delete_back_one_indent(page: &Rc<Page>) -> bool {
    let buffer = &page.buffer;
    if buffer.selection_bounds().is_some() {
        return false;
    }
    let (start, before, _) = line_before_caret(page);
    let width = indent::backspace_width(&before, tab_width(page));
    if width <= 1 {
        return false;
    }
    let caret = buffer.iter_at_mark(&buffer.get_insert());
    let mut from = buffer.iter_at_offset(caret.offset() - width as i32);
    let mut to = caret;
    let _ = start;
    buffer.delete(&mut from, &mut to);
    true
}

fn indent_to_block_above(page: &Rc<Page>) -> bool {
    let buffer = &page.buffer;
    if buffer.selection_bounds().is_some() {
        return false;
    }
    let (start, before, whole) = line_before_caret(page);
    let current_indent = indent::leading_whitespace(&whole).to_string();
    // Only in the indentation: past it, Tab inserts as it always has.
    if before.chars().count() > current_indent.chars().count() {
        return false;
    }
    let mut previous = None;
    let mut line = start.line() - 1;
    while line >= 0 {
        let Some(above) = buffer.iter_at_line(line) else { break };
        let mut end = above;
        if !end.ends_line() {
            end.forward_to_line_end();
        }
        let text = buffer.text(&above, &end, true).to_string();
        if !text.trim().is_empty() {
            previous = Some(text);
            break;
        }
        line -= 1;
    }
    // What the document already does, the way auto-indent decides it.
    let uses_tabs = current_indent.contains('\t')
        || previous.as_deref().is_some_and(|line| line.starts_with('\t'));
    let wanted = indent::aligned_indent(
        previous.as_deref(),
        &current_indent,
        tab_width(page),
        uses_tabs,
    );
    if wanted == current_indent {
        return false;
    }
    let mut from = start;
    let mut to = buffer.iter_at_offset(start.offset() + current_indent.chars().count() as i32);
    buffer.delete(&mut from, &mut to);
    let mut at = from;
    buffer.insert(&mut at, &wanted);
    true
}

/// The git change bar: a stripe beside every line that differs from
/// the committed file, and a wedge where lines were deleted.
///
/// Drawn from the marks the core works out, redrawn on every scroll and
/// whenever they change. The colours are deliberately not the theme's:
/// they say what git thinks, not what the language means, and a reader
/// should not have to wonder whether a green bar is a string.
fn install_change_bar(page: &Rc<Page>) {
    let weak = Rc::downgrade(page);
    page.change_bar.set_draw_func(move |area, context, width, _height| {
        let Some(page) = weak.upgrade() else { return };
        let marks = page.change_marks.borrow();
        if marks.is_empty() {
            return;
        }
        let view = &page.view;
        let visible = view.visible_rect();
        let width = width as f64;
        for (line, kind) in marks.iter() {
            let Some(iter) = view.buffer().iter_at_line(*line) else { continue };
            let (top, height) = view.line_yrange(&iter);
            if top + height < visible.y() || top > visible.y() + visible.height() {
                continue;
            }
            let (_, y) = view.buffer_to_window_coords(gtk::TextWindowType::Widget, 0, top);
            let y = y as f64;
            match kind {
                ChangeKind::Added => {
                    context.set_source_rgba(0.30, 0.72, 0.36, 0.85);
                    context.rectangle(0.0, y, width, height as f64);
                }
                ChangeKind::Modified => {
                    context.set_source_rgba(0.25, 0.55, 0.92, 0.85);
                    context.rectangle(0.0, y, width, height as f64);
                }
                // Deleted lines occupy no height, so a stripe would
                // have nothing to cover: a wedge on the boundary.
                ChangeKind::Removed => {
                    context.set_source_rgba(0.90, 0.31, 0.28, 0.85);
                    context.move_to(0.0, y - 3.0);
                    context.line_to(width, y);
                    context.line_to(0.0, y + 3.0);
                    context.close_path();
                }
            }
            let _ = context.fill();
        }
        let _ = area;
    });

    // The bar is not inside the scroller, so it has to follow it.
    {
        let bar = page.change_bar.clone();
        page.view.vadjustment().inspect(|adjustment| {
            adjustment.connect_value_changed(move |_| bar.queue_draw());
        });
    }
}

/// Recomputes the change bar's marks for a page.
///
/// The git call and the diff both happen here, on the main thread: the
/// diff is about a tenth of a millisecond for a file with a handful of
/// edits, and the git call is a process spawn. Callers debounce.
pub fn refresh_change_marks(page: &Rc<Page>) {
    let Some(path) = page.path().borrow().clone() else {
        page.change_marks.borrow_mut().clear();
        page.change_bar.queue_draw();
        return;
    };
    let text = page.state.borrow().document.text();
    // The baseline and the branch priorities come from the project's
    // override when it has one, the configuration otherwise.
    let root = page
        .document
        .project_root
        .borrow()
        .clone()
        .unwrap_or_default();
    let baseline = {
        let shell = crate::shell::Shell::instance();
        let config = shell.config.borrow();
        let overrides = config.editor_overrides_json(&root);
        serde_json::from_str::<serde_json::Value>(&overrides)
            .ok()
            .and_then(|parsed| parsed["git_marks"].as_str().map(str::to_string))
            .unwrap_or_else(|| config.git_marks())
    };
    let branches = crate::workbench::merge_base_branches_for(std::path::Path::new(&root));
    let marks = textchum_core::changes::changes_against(
        std::path::Path::new(&path),
        &text,
        textchum_core::changes::Baseline::parse(&baseline),
        &branches,
    );
    *page.change_marks.borrow_mut() = marks
        .into_iter()
        .map(|mark| (mark.line as i32, mark.kind))
        .collect();
    page.change_bar.queue_draw();
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
    let Some(path) = page.path().borrow().clone() else { return };
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
        let Some(path) = page.path().borrow().clone() else { return };
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
    *page.monitor().borrow_mut() = Some(monitor);
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
    let Some(path) = page.path().borrow().clone() else { return };
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
        let offset = page
            .view
            .iter_at_location(bx, by)
            .map(|iter| iter.offset())
            .unwrap_or_else(|| page.buffer.iter_at_mark(&page.buffer.get_insert()).offset());
        let path = page.path().borrow().clone();
        let menu = context_menu(
            misspelling.as_ref().map(|(word, _, _)| word.as_str()),
            page.buffer.has_selection(),
            path.as_deref(),
            offset,
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
/// the pointer is on a misspelling, Change Case when there is a
/// selection to apply it to, and the editor's own commands.
///
/// Those commands act on `offset` — the character that was clicked —
/// rather than on the caret, which a right-click leaves where it was.
/// The ones that need a language server are left out when none is
/// running for the document: a greyed row explains nothing.
pub fn context_menu(
    misspelling: Option<&str>,
    has_selection: bool,
    path: Option<&str>,
    offset: i32,
) -> gtk::gio::Menu {
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
        dictionary.append(Some(&tr("Add to Dictionary")), Some("win.spell-add"));
        dictionary.append(Some(&tr("Ignore While This Runs")), Some("win.spell-ignore"));
        menu.append_section(None, &dictionary);
    }
    if has_selection {
        // GtkSourceView's own entries, by the action it installs:
        // `source.change-case` takes the case to change to as its
        // parameter.
        let case = gtk::gio::Menu::new();
        case.append(Some("All Upper Case"), Some("source.change-case('upper')"));
        case.append(Some("All Lower Case"), Some("source.change-case('lower')"));
        case.append(Some(&tr("Invert Case")), Some("source.change-case('toggle')"));
        case.append(Some(&tr("Title Case")), Some("source.change-case('title')"));
        let holder = gtk::gio::Menu::new();
        holder.append_submenu(Some("Change Case"), &case);
        menu.append_section(None, &holder);
    }

    let mut commands: Vec<(&str, &str)> = Vec::new();
    if path.is_some() {
        // Without a server the ctags index may still answer.
        commands.push(("Jump to Definition", "definition"));
    }
    if path.is_some_and(server_running_for) {
        commands.push(("Find References", "references"));
        commands.push(("Code Actions…", "code-actions"));
        commands.push(("Rename Symbol…", "rename"));
    }
    if path.is_some_and(has_diagnostics) {
        commands.push(("Show Diagnostic for Line", "diagnostic"));
        commands.push(("Diagnostics…", "diagnostic-list"));
    }
    if path.is_some() {
        commands.push(("Blame Line…", "blame"));
    }
    // Formatting falls back to the save-preprocessor chain, so it is
    // offered with or without a server.
    commands.push(("Format Document", "format"));
    commands.push(("File Properties…", "file-properties"));

    let editor = gtk::gio::Menu::new();
    for (label, action) in commands {
        editor.append(
            Some(label),
            Some(&format!("win.context-command(('{action}', {offset}))")),
        );
    }
    menu.append_section(None, &editor);
    menu
}

/// Whether the server has said anything about this document. With
/// nothing reported the two diagnostic commands have nothing to show,
/// so they are left out.
fn has_diagnostics(path: &str) -> bool {
    Shell::instance()
        .pages
        .borrow()
        .get(path)
        .is_some_and(|handles| !handles.document.diagnostics.borrow().is_empty())
}

/// Whether a language server is up for this document.
fn server_running_for(path: &str) -> bool {
    Shell::instance()
        .pool
        .borrow()
        .running()
        .iter()
        .any(|(_, root)| Path::new(path).starts_with(root))
}

/// Applies the project root's `editor` overrides (font family, size,
/// tab width) to this view — the Mac's per-project settings, GTK
/// edition. Font settings ride a per-view CSS provider; the global
/// size provider stays the fallback. (The per-widget style context is
/// deprecated upstream but remains the one per-view hook.)
#[allow(deprecated)]
pub fn apply_project_editor_overrides(page: &Rc<Page>) {
    let Some(path) = page.path().borrow().clone() else { return };
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

/// The diagnostic under a point in the view, if there is one.
fn diagnostic_under(page: &Rc<Page>, x: f64, y: f64) -> Option<crate::shell::Diagnostic> {
    let path = page.path().borrow().clone()?;
    let shell = Shell::instance();
    let pages = shell.pages.borrow();
    let handles = pages.get(&path)?;
    let (bx, by) = page.view.window_to_buffer_coords(
        gtk::TextWindowType::Widget,
        x as i32,
        y as i32,
    );
    let (iter, _) = page.view.iter_at_position(bx, by)?;
    let line = iter.line();
    let line_start = page.buffer.iter_at_line(line)?;
    let character = page
        .buffer
        .text(&line_start, &iter, true)
        .encode_utf16()
        .count();
    diagnostic_at(handles, line, character)
}

/// The balloon hover documentation and diagnostics both appear in.
fn show_balloon(page: &Rc<Page>, text: &str, x: f64, y: f64) {
    page.hover_label.set_text(text);
    page.hover_popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(
        x as i32,
        y as i32,
        1,
        1,
    )));
    page.hover_popover.popup();
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
            // A diagnostic is already in hand and needs no server, so
            // it answers whether or not hover documentation is on: the
            // mark is on screen either way.
            if let Some(found) = diagnostic_under(&page, x, y) {
                show_balloon(&page, &format!("{}\n{}", found.kind(), found.message), x, y);
                return;
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
    let Some(path) = page.path().borrow().clone() else { return };
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
        // Asked by name: style ids are positions in an alphabetical
        // table and move whenever a capture is added.
        let in_comment = page
            .state
            .borrow()
            .document
            .highlights(offset, offset + 1)
            .ok()
            .into_iter()
            .flatten()
            .any(|span| {
                Some(span.style) == textchum_core::theme::resolve("comment")
                    && span.start_utf16 <= offset
                    && offset < span.end_utf16
            });
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
