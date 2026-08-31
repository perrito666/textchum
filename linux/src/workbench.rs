//! The workbench window: tabs (AdwTabView), a project file tree in a
//! sidebar, the search bar, the primary menu, preferences, and every
//! action — over pages that each mirror one core document.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use sourceview5::prelude::*;
use textchum_core::{blame, goto, icons, references, t, theme_import, workspace, Appearance};

use crate::page::{self, Page};
use crate::shell::{PageHandles, Shell};
use textchum_core::i18n::{fill, tr, tr_n};

/// One column: a tab group, and the views of that group's file
/// stacked under it.
///
/// The column owns the file; the views are places to look at it. View 0
/// is the tab group itself, which is why `views` holds only the ones
/// under it.
struct Column {
    tab_view: adw::TabView,
    /// Bar and stack together, which is what sits in the row.
    root: gtk::Box,
    /// The views under the tab group, top to bottom.
    views: RefCell<Vec<Rc<Page>>>,
    /// The file this column is showing. Kept because the tab has
    /// already changed by the time the change is announced, and the
    /// shape being written down belongs to the file that left.
    showing: RefCell<Option<Rc<crate::shell::OpenDocument>>>,
}

impl Column {
    fn new(tab_view: &adw::TabView) -> Column {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let bar = adw::TabBar::new();
        bar.set_view(Some(tab_view));
        root.append(&bar);
        Column {
            tab_view: tab_view.clone(),
            root,
            views: RefCell::new(Vec::new()),
            showing: RefCell::new(None),
        }
    }

    /// Puts the column's stack back together: the tab group on top, the
    /// extra views under it.
    fn rebuild(&self) {
        // Everything below the bar comes out first.
        while let Some(child) = self.root.last_child() {
            if child.downcast_ref::<adw::TabBar>().is_some() {
                break;
            }
            self.root.remove(&child);
        }
        let mut widgets: Vec<gtk::Widget> = vec![self.tab_view.clone().upcast()];
        widgets.extend(
            self.views
                .borrow()
                .iter()
                .map(|page| page.root.clone().upcast::<gtk::Widget>()),
        );
        if let Some(stack) = nest(gtk::Orientation::Vertical, &widgets) {
            stack.set_vexpand(true);
            self.root.append(&stack);
        }
    }

    /// Gives up every view this column holds — the column is going.
    fn release_views(&self, workbench: &Rc<Workbench>) {
        for page in self.views.borrow().iter() {
            workbench.forget_view(page);
        }
        self.views.borrow_mut().clear();
    }
}

/// Takes a widget out of whatever is holding it.
fn detach(widget: &gtk::Widget) {
    let Some(parent) = widget.parent() else { return };
    if let Some(paned) = parent.downcast_ref::<gtk::Paned>() {
        if paned.start_child().as_ref() == Some(widget) {
            paned.set_start_child(None::<&gtk::Widget>);
        }
        if paned.end_child().as_ref() == Some(widget) {
            paned.set_end_child(None::<&gtk::Widget>);
        }
    } else if let Some(holder) = parent.downcast_ref::<gtk::Box>() {
        holder.remove(widget);
    } else {
        widget.unparent();
    }
}

/// Nests paneds so that `widgets` share the space, each resizable.
/// GTK's paned holds exactly two children, so N widgets are N−1 of
/// them. None when there is nothing to nest.
fn nest(orientation: gtk::Orientation, widgets: &[gtk::Widget]) -> Option<gtk::Widget> {
    let (first, rest) = widgets.split_first()?;
    // The chain is built afresh every time it changes, so a widget
    // arrives here still held by the paned it was in. A widget with a
    // parent cannot be given another, and the attempt shows as an empty
    // editor area.
    detach(first);
    let Some(tail) = nest(orientation, rest) else {
        return Some(first.clone());
    };
    let paned = gtk::Paned::new(orientation);
    paned.set_shrink_start_child(false);
    paned.set_shrink_end_child(false);
    paned.set_resize_start_child(true);
    paned.set_resize_end_child(true);
    paned.set_start_child(Some(first));
    paned.set_end_child(Some(&tail));
    Some(paned.upcast())
}

pub struct Workbench {
    pub window: adw::ApplicationWindow,
    /// The first column's tab group. Every window has one, and the
    /// commands that speak of "the tabs" without saying which column
    /// mean the focused one — see [`Workbench::active_view`].
    pub tab_view: adw::TabView,
    /// The columns, left to right. A column shows one file at a time
    /// and holds one or more views of it, stacked.
    columns: RefCell<Vec<Column>>,
    /// Which column has the keyboard, and which view inside it: view 0
    /// is the tab group itself, the rest are stacked under it.
    focused_column: Cell<usize>,
    focused_view: Cell<usize>,
    /// Where the row of columns is put.
    columns_host: gtk::Box,
    /// Set while a window closes that has already asked about its
    /// unsaved files, so the close it starts again goes straight
    /// through.
    closing_settled: Cell<bool>,
    title: adw::WindowTitle,
    toasts: adw::ToastOverlay,
    split: adw::OverlaySplitView,
    buffers_box: gtk::Box,
    tree_box: gtk::Box,
    /// Aligned with the buffer list's rows: None for group headers.
    buffer_rows: RefCell<Vec<Option<Rc<Page>>>>,
    /// (title, dirty, selected) per page — rebuild only on real change.
    buffer_signature: RefCell<Vec<(String, bool, bool)>>,
    status_position: gtk::Label,
    status_indent: gtk::Button,
    status_language: gtk::Button,
    status_encoding: gtk::Label,
    search_bar: gtk::SearchBar,
    search_entry: gtk::SearchEntry,
    replace_entry: gtk::Entry,
    search_case: gtk::ToggleButton,
    search_regex: gtk::ToggleButton,
    search_word: gtk::ToggleButton,
    pages: RefCell<Vec<Rc<Page>>>,
    /// The root the sidebar currently shows.
    sidebar_root: RefCell<Option<PathBuf>>,
    /// Where jumps came from, vim-jumplist style: Go Back retraces,
    /// Go Forward returns, and a new jump clears the forward trail.
    jumps: RefCell<JumpStack>,
    /// True while Go Back/Forward navigates, so retracing a jump does
    /// not record itself as a new one.
    retracing: std::cell::Cell<bool>,
    /// Momentary path display: buffer rows show project-relative paths
    /// instead of bare names while this is on.
    show_full_paths: std::cell::Cell<bool>,
    /// The explanation currently on screen, so a failure that repeats
    /// does not queue a second identical notice behind the first.
    explaining: Rc<RefCell<Option<String>>>,
    /// Tabs that were closed, newest last, with where the caret was.
    /// Only saved documents go here: an untitled buffer has nothing to
    /// reopen from, and pretending otherwise would reopen it empty.
    closed_tabs: RefCell<Vec<(PathBuf, i32, usize)>>,
}

/// How many closed tabs are worth remembering. Deep enough to undo a
/// run of mistaken closes, shallow enough that the list stays a list of
/// recent mistakes rather than a second history.
const CLOSED_TAB_MEMORY: usize = 20;

/// (path, line, UTF-16 column) positions with a cursor into them.
#[derive(Default)]
pub struct JumpStack {
    entries: Vec<(String, i32, usize)>,
    cursor: usize,
}

impl JumpStack {
    /// Records where a jump started; anything forward of the cursor is
    /// no longer reachable (same contract as the macOS stack).
    fn note(&mut self, origin: (String, i32, usize)) {
        self.entries.truncate(self.cursor);
        self.entries.push(origin);
        self.cursor = self.entries.len();
    }

    fn back(&mut self, current: Option<(String, i32, usize)>) -> Option<(String, i32, usize)> {
        if self.cursor == 0 {
            return None;
        }
        // Standing past the end: remember where we are, so Forward can
        // return here.
        if self.cursor == self.entries.len() {
            if let Some(current) = current {
                self.entries.push(current);
            }
        }
        self.cursor -= 1;
        self.entries.get(self.cursor).cloned()
    }

    fn forward(&mut self) -> Option<(String, i32, usize)> {
        if self.cursor + 1 >= self.entries.len() {
            return None;
        }
        self.cursor += 1;
        self.entries.get(self.cursor).cloned()
    }
}

thread_local! {
    static WORKBENCHES: RefCell<Vec<Rc<Workbench>>> = const { RefCell::new(Vec::new()) };
}

impl Workbench {
    pub fn new(app: &adw::Application) -> Rc<Workbench> {
        let tab_view = adw::TabView::new();

        let title = adw::WindowTitle::new("Textchum", "");
        let header = adw::HeaderBar::new();
        header.set_title_widget(Some(&title));

        let sidebar_toggle = gtk::ToggleButton::new();
        sidebar_toggle.set_icon_name("sidebar-show-symbolic");
        sidebar_toggle.set_tooltip_text(Some("Toggle the file tree (F9)"));
        header.pack_start(&sidebar_toggle);
        let open_button = gtk::Button::from_icon_name("document-open-symbolic");
        open_button.set_tooltip_text(Some("Open a file (Ctrl+O)"));
        open_button.set_action_name(Some("win.open"));
        header.pack_start(&open_button);

        let file_section = gtk::gio::Menu::new();
        file_section.append(Some("New Tab"), Some("win.new-tab"));
        // One entry per selectable language: a new tab that speaks it
        // before its first save.
        let formats_menu = gtk::gio::Menu::new();
        for name in textchum_core::syntax::languages::selectable_names() {
            formats_menu.append(
                Some(&menu_label(name)),
                Some(&format!("win.new-with-format('{name}')")),
            );
        }
        file_section.append(Some(&tr("New with Format…")), Some("win.new-format-picker"));
        file_section.append_submenu(Some(&tr("New with Format")), &formats_menu);
        file_section.append(Some(&tr("New Window")), Some("win.new"));
        file_section.append(Some(&tr("Open…")), Some("win.open"));
        // The desktop's shared recent-files list, newest first — and
        // only the part of it this editor can open. The list belongs to
        // the whole session, so a photo viewer's PNGs are in it too, and
        // offering them here promises an edit that cannot happen.
        let recent_menu = gtk::gio::Menu::new();
        let mut recent: Vec<gtk::RecentInfo> = gtk::RecentManager::default().items();
        recent.sort_by_key(|info| std::cmp::Reverse(info.modified()));
        let mut listed = 0;
        for info in recent.iter() {
            if listed == 10 {
                break;
            }
            let uri = info.uri();
            let Some(path) = uri.strip_prefix("file://") else { continue };
            let path = percent_decode(path);
            if !workspace::looks_editable(Path::new(&path)) {
                continue;
            }
            let name = info.display_name();
            recent_menu.append(
                Some(&menu_label(&name)),
                Some(&format!("win.open-recent('{}')", path.replace('\'', "\\'"))),
            );
            listed += 1;
        }
        if listed == 0 {
            // An empty submenu renders as an unexplained dead end.
            let empty = gtk::gio::MenuItem::new(Some("No recent text files"), None);
            recent_menu.append_item(&empty);
        }
        file_section.append_submenu(Some(&tr("Open Recent")), &recent_menu);
        file_section.append(Some(&tr("Open Quickly…")), Some("win.quick-open"));
        file_section.append(Some(&tr("Changed in Branch…")), Some("win.changed-files"));
        file_section.append(Some(&tr("Save")), Some("win.save"));
        file_section.append(Some(&tr("Save As…")), Some("win.save-as"));
        file_section.append(Some(&tr("Revert to Saved")), Some("win.revert"));
        let edit_section = gtk::gio::Menu::new();
        edit_section.append(Some(&tr("Undo")), Some("win.undo"));
        edit_section.append(Some(&tr("Redo")), Some("win.redo"));
        edit_section.append(Some("Find…"), Some("win.find"));
        edit_section.append(Some(&tr("Find in Project…")), Some("win.find-in-project"));
        edit_section.append(Some(&tr("Run Save Preprocessors")), Some("win.preprocess"));
        edit_section.append(Some(&tr("Redraw")), Some("win.redraw"));
        edit_section.append(Some(&tr("Fold")), Some("win.fold"));
        edit_section.append(Some(&tr("Fold All")), Some("win.fold-all"));
        edit_section.append(Some(&tr("Unfold All")), Some("win.unfold-all"));
        edit_section.append(Some(&tr("New Column")), Some("win.new-column"));
        edit_section.append(Some(&tr("Close Column")), Some("win.close-column"));
        edit_section.append(Some(&tr("Second View")), Some("win.add-view"));
        edit_section.append(Some(&tr("Close View")), Some("win.close-view"));
        edit_section.append(Some(&tr("Next Pane")), Some("win.focus-other-group"));
        {
            // What to do to the selection, or to the whole document
            // when nothing is selected. GtkSourceView has Change Case
            // of its own in the context menu; these go through the
            // core, so both shells agree on what sorting and joining
            // mean.
            let transforms = gtk::gio::Menu::new();
            let mut section = gtk::gio::Menu::new();
            for (label, kind) in crate::page::TRANSFORMS {
                if kind.is_empty() {
                    transforms.append_section(None, &section);
                    section = gtk::gio::Menu::new();
                    continue;
                }
                section.append(Some(label), Some(&format!("win.transform('{kind}')")));
            }
            transforms.append_section(None, &section);
            edit_section.append_submenu(Some(&tr("Transform")), &transforms);
        }
        let go_section = gtk::gio::Menu::new();
        go_section.append(Some(&tr("Jump to Definition")), Some("win.definition"));
        go_section.append(Some(&tr("Go Back")), Some("win.back"));
        go_section.append(Some(&tr("Go Forward")), Some("win.forward"));
        go_section.append(Some(&tr("Find References")), Some("win.references"));
        go_section.append(Some(&tr("Code Actions…")), Some("win.code-actions"));
        go_section.append(Some(&tr("Rename Symbol…")), Some("win.rename"));
        go_section.append(Some(&tr("Format Document")), Some("win.format"));
        go_section.append(Some(&tr("Document Outline…")), Some("win.outline"));
        go_section.append(Some(&tr("Show Documentation for Symbol")), Some("win.hover"));
        go_section.append(Some(&tr("Go to Line…")), Some("win.goto-line"));
        go_section.append(Some(&tr("Blame Line…")), Some("win.blame"));
        go_section.append(Some(&tr("Show Diagnostic for Line")), Some("win.diagnostic"));
        go_section.append(Some(&tr("Diagnostics…")), Some("win.diagnostic-list"));
        go_section.append(Some(&tr("Go to Block Start")), Some("win.block-start"));
        go_section.append(Some(&tr("Go to Block End")), Some("win.block-end"));
        go_section.append(Some(&tr("Command Palette…")), Some("win.palette"));
        go_section.append(Some(&tr("Language Server Status")), Some("win.server-status"));
        go_section.append(Some("File Properties…"), Some("win.file-properties"));
        go_section.append(Some(&tr("Toggle Path Display")), Some("win.paths"));
        go_section.append(Some("Toggle File Tree"), Some("win.sidebar"));
        go_section.append(Some(&tr("Toggle Markdown Preview")), Some("win.preview"));
        let app_section = gtk::gio::Menu::new();
        app_section.append(Some("Preferences…"), Some("win.preferences"));
        let import_theme = gtk::gio::Menu::new();
        import_theme.append(Some(&tr("From VS Code…")), Some("win.import-theme-vscode"));
        import_theme.append(Some(&tr("From TextMate…")), Some("win.import-theme-textmate"));
        app_section.append_submenu(Some(&tr("Import Theme")), &import_theme);
        app_section.append(Some(&tr("About Textchum")), Some("win.about"));
        app_section.append(Some(&tr("Close Tab")), Some("win.close-tab"));
        app_section.append(Some(&tr("Reopen Closed Tab")), Some("win.reopen-tab"));
        app_section.append(Some(&tr("Close Window")), Some("window.close"));
        app_section.append(Some(&tr("Quit")), Some("app.quit"));
        let menu = gtk::gio::Menu::new();
        menu.append_section(None, &file_section);
        menu.append_section(None, &edit_section);
        menu.append_section(None, &go_section);
        menu.append_section(None, &app_section);
        let menu_button = gtk::MenuButton::builder()
            .icon_name("open-menu-symbolic")
            .menu_model(&menu)
            .tooltip_text("Menu")
            .build();
        header.pack_end(&menu_button);
        if std::env::var_os("TEXTCHUM_DEBUG_MENU").is_some() {
            let button = menu_button.clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(1200), move || {
                button.popup();
            });
        }

        let search_entry = gtk::SearchEntry::new();
        search_entry.set_placeholder_text(Some(&tr("Find in file…")));
        search_entry.set_hexpand(true);
        let search_case = gtk::ToggleButton::with_label("Aa");
        search_case.set_tooltip_text(Some(&tr("Match case")));
        let search_regex = gtk::ToggleButton::with_label(".*");
        search_regex.set_tooltip_text(Some("Regular expression"));
        let search_word = gtk::ToggleButton::with_label("⌊w⌋");
        search_word.set_tooltip_text(Some("Whole words"));
        let find_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        find_row.append(&search_entry);
        find_row.append(&search_case);
        find_row.append(&search_regex);
        find_row.append(&search_word);
        let replace_entry = gtk::Entry::new();
        replace_entry.set_placeholder_text(Some(&tr("Replace with…")));
        replace_entry.set_hexpand(true);
        let replace_button = gtk::Button::with_label("Replace");
        let replace_all_button = gtk::Button::with_label("All");
        let replace_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        replace_row.append(&replace_entry);
        replace_row.append(&replace_button);
        replace_row.append(&replace_all_button);
        let search_rows = gtk::Box::new(gtk::Orientation::Vertical, 6);
        search_rows.append(&find_row);
        search_rows.append(&replace_row);
        let search_bar = gtk::SearchBar::new();
        search_bar.set_child(Some(&search_rows));
        search_bar.connect_entry(&search_entry);

        let content_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        // Every column carries its own tab bar, so the row of columns
        // is the whole editor area.
        let columns_host = gtk::Box::new(gtk::Orientation::Vertical, 0);
        columns_host.set_vexpand(true);
        content_box.append(&columns_host);

        // The bar under the editor: where the caret is, how the file is
        // indented, what it is treated as. The parts that name a
        // per-file choice open File Properties, where it is made.
        let status_position = gtk::Label::new(None);
        let status_indent = gtk::Button::with_label("");
        let status_language = gtk::Button::with_label("");
        let status_encoding = gtk::Label::new(None);
        status_indent.set_tooltip_text(Some(&tr(
            "How this file is indented — click to change it",
        )));
        status_language.set_tooltip_text(Some(&tr(
            "What this file is treated as — click to change it",
        )));
        for button in [&status_indent, &status_language] {
            button.add_css_class("flat");
            button.set_action_name(Some("win.file-properties"));
        }
        let status_bar = gtk::Box::new(gtk::Orientation::Horizontal, 14);
        status_bar.add_css_class("dim-label");
        status_bar.set_margin_start(12);
        status_bar.set_margin_end(12);
        status_bar.set_margin_top(2);
        status_bar.set_margin_bottom(2);
        status_bar.append(&status_position);
        status_bar.append(&status_indent);
        status_bar.append(&status_language);
        status_bar.append(&status_encoding);
        content_box.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        content_box.append(&status_bar);
        tab_view.set_vexpand(true);

        // Sidebar: open buffers grouped by project, over the selected
        // page's project file tree — the drawer, GTK edition.
        let sidebar_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        sidebar_box.set_width_request(220);
        let buffers_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let tree_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        tree_box.set_vexpand(true);
        sidebar_box.append(&buffers_box);
        sidebar_box.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        sidebar_box.append(&tree_box);
        let split = adw::OverlaySplitView::new();
        split.set_sidebar(Some(&sidebar_box));
        split.set_content(Some(&content_box));
        split.set_show_sidebar(false);
        split.set_max_sidebar_width(360.0);
        {
            let split = split.clone();
            sidebar_toggle.connect_toggled(move |toggle| {
                split.set_show_sidebar(toggle.is_active());
            });
        }

        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.add_top_bar(&search_bar);
        toolbar.set_content(Some(&split));
        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&toolbar));

        let window = adw::ApplicationWindow::builder()
            .application(app)
            .default_width(1080)
            .default_height(720)
            .content(&toasts)
            .build();
        search_bar.set_key_capture_widget(Some(&window));
        apply_font_size(Shell::instance().config.borrow().font_size());
        // Screenshot-driven verification hooks.
        if std::env::var_os("TEXTCHUM_DEBUG_SIDEBAR").is_some() {
            sidebar_toggle.set_active(true);
        }
        if std::env::var_os("TEXTCHUM_DEBUG_PREFS").is_some() {
            let for_prefs = window.clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(1200), move || {
                show_preferences(&for_prefs);
            });
        }

        let workbench = Rc::new(Workbench {
            window: window.clone(),
            tab_view: tab_view.clone(),
            columns: RefCell::new(vec![Column::new(&tab_view)]),
            focused_column: Cell::new(0),
            focused_view: Cell::new(0),
            columns_host: columns_host.clone(),
            closing_settled: Cell::new(false),
            title,
            toasts,
            split,
            buffers_box,
            tree_box,
            buffer_rows: RefCell::new(Vec::new()),
            buffer_signature: RefCell::new(Vec::new()),
            status_position: status_position.clone(),
            status_indent: status_indent.clone(),
            status_language: status_language.clone(),
            status_encoding: status_encoding.clone(),
            search_bar,
            search_entry: search_entry.clone(),
            replace_entry: replace_entry.clone(),
            search_case: search_case.clone(),
            search_regex: search_regex.clone(),
            search_word: search_word.clone(),
            pages: RefCell::new(Vec::new()),
            sidebar_root: RefCell::new(None),
            jumps: RefCell::new(JumpStack::default()),
            retracing: std::cell::Cell::new(false),
            show_full_paths: std::cell::Cell::new(false),
            explaining: Rc::new(RefCell::new(None)),
            closed_tabs: RefCell::new(Vec::new()),
        });
        WORKBENCHES.with(|list| list.borrow_mut().push(Rc::clone(&workbench)));

        workbench.install_group(&tab_view);
        workbench.rebuild_columns();
        {
            // Closing a window is closing what it shows, unless the
            // files are set to outlive their windows.
            let weak = Rc::downgrade(&workbench);
            window.connect_close_request(move |_| {
                let Some(workbench) = weak.upgrade() else {
                    return glib::Propagation::Proceed;
                };
                if workbench.closing_settled.replace(false) {
                    return glib::Propagation::Proceed;
                }
                if Shell::instance().config.borrow().keep_buffers() {
                    workbench.stash_documents();
                    return glib::Propagation::Proceed;
                }
                if workbench.asks_about_closing() {
                    return glib::Propagation::Stop;
                }
                workbench.stash_documents();
                glib::Propagation::Proceed
            });
        }
        // Search acts on the selected page's context.
        {
            let workbench = Rc::downgrade(&workbench);
            search_entry.connect_search_changed(move |entry| {
                let Some(workbench) = workbench.upgrade() else { return };
                if let Some(page) = workbench.selected() {
                    let text = entry.text();
                    page.search_settings
                        .set_search_text(if text.is_empty() { None } else { Some(&text) });
                }
            });
        }
        {
            let workbench = Rc::downgrade(&workbench);
            search_entry.connect_activate(move |_| {
                let Some(workbench) = workbench.upgrade() else { return };
                let Some(page) = workbench.selected() else { return };
                let buffer = &page.buffer;
                let insert = buffer.iter_at_mark(&buffer.get_insert());
                if let Some((start, end, _)) = page.search_context.forward(&insert) {
                    buffer.select_range(&end, &start);
                    page.view
                        .scroll_to_iter(&mut start.clone(), 0.1, false, 0.0, 0.0);
                }
            });
        }
        // The find toggles push into the selected page's settings the
        // moment they change (and page switches re-apply them).
        for toggle in [&search_case, &search_regex, &search_word] {
            let workbench = Rc::downgrade(&workbench);
            toggle.connect_toggled(move |_| {
                if let Some(workbench) = workbench.upgrade() {
                    workbench.apply_search_options();
                }
            });
        }
        {
            let workbench = Rc::downgrade(&workbench);
            replace_button.connect_clicked(move |_| {
                let Some(workbench) = workbench.upgrade() else { return };
                let Some(page) = workbench.selected() else { return };
                let replacement = workbench.replace_entry.text();
                let buffer = &page.buffer;
                let insert = buffer.iter_at_mark(&buffer.get_insert());
                if let Some((mut start, mut end, _)) = page.search_context.forward(&insert) {
                    let _ = page
                        .search_context
                        .replace(&mut start, &mut end, &replacement);
                    buffer.place_cursor(&end);
                    page.view
                        .scroll_to_iter(&mut end.clone(), 0.1, false, 0.0, 0.0);
                }
            });
        }
        {
            let workbench = Rc::downgrade(&workbench);
            replace_all_button.connect_clicked(move |_| {
                let Some(workbench) = workbench.upgrade() else { return };
                let Some(page) = workbench.selected() else { return };
                let replacement = workbench.replace_entry.text();
                match page.search_context.replace_all(&replacement) {
                    Ok(()) => workbench.toast(&tr("Replaced every occurrence.")),
                    Err(error) => workbench.toast(&fill(&tr("Replace failed: {}"), &[&error.to_string()])),
                }
            });
        }
        install_actions(app, &workbench);
        workbench
    }

    /// Runs `f` over every live workbench.
    pub fn for_each(mut f: impl FnMut(&Rc<Workbench>)) {
        WORKBENCHES.with(|list| {
            for workbench in list.borrow().iter() {
                f(workbench);
            }
        });
    }

    /// Every page this workbench hosts, in tab order.
    pub fn all_pages(&self) -> Vec<Rc<Page>> {
        self.pages.borrow().clone()
    }

    /// The page showing `path`, if this workbench has it.
    pub fn page_for(&self, path: &str) -> Option<Rc<Page>> {
        self.pages
            .borrow()
            .iter()
            .find(|page| page.path().borrow().as_deref() == Some(path))
            .cloned()
    }

    /// Pushes the find toggles into the selected page's settings.
    fn apply_search_options(&self) {
        let Some(page) = self.selected() else { return };
        page.search_settings
            .set_case_sensitive(self.search_case.is_active());
        page.search_settings
            .set_regex_enabled(self.search_regex.is_active());
        page.search_settings
            .set_at_word_boundaries(self.search_word.is_active());
    }

    /// The selected page's position, for the jump stack.
    fn current_position(&self) -> Option<(String, i32, usize)> {
        let page = self.selected()?;
        let path = page.path().borrow().clone()?;
        let (line, character) = page::lsp_caret(&page.buffer);
        Some((path, line as i32, character as usize))
    }

    /// Records the current position as a jump origin (called before any
    /// navigation that deserves a Go Back).
    pub fn note_jump(&self) {
        if let Some(position) = self.current_position() {
            self.jumps.borrow_mut().note(position);
        }
    }

    /// The workbench of the currently focused window (or the last one).
    pub fn active() -> Option<Rc<Workbench>> {
        WORKBENCHES.with(|list| {
            let list = list.borrow();
            list.iter()
                .find(|workbench| workbench.window.is_active())
                .or_else(|| list.last())
                .cloned()
        })
    }

    /// Something the user should hear about, softly.
    pub fn toast(&self, text: &str) {
        self.toasts.add_toast(adw::Toast::new(text));
    }

    /// An explanation rather than a note: it wraps, and it waits to be
    /// dismissed. A default toast is one ellipsized line that fades
    /// after a few seconds, which is fine for "saved" and useless for
    /// "here is the package you need to install".
    ///
    /// Repeats of the explanation already on screen are dropped. Waiting
    /// to be dismissed is what makes these readable, and it is also what
    /// would make a language server that fails on every keystroke pile
    /// up a queue of identical notices to click through.
    pub fn explain(&self, text: &str) {
        if self.explaining.borrow().as_deref() == Some(text) {
            return;
        }
        *self.explaining.borrow_mut() = Some(text.to_owned());
        let toast = long_toast(text);
        let showing = Rc::clone(&self.explaining);
        toast.connect_dismissed(move |_| {
            showing.borrow_mut().take();
        });
        self.toasts.add_toast(toast);
    }

    /// Wires a group up: the chrome follows its selection, closing a
    /// tab retires its document, and a click in it makes it the group
    /// commands act on.
    ///
    /// Both groups get this, which is what keeps the second one a real
    /// group rather than a display of the first.
    fn install_group(self: &Rc<Self>, view: &adw::TabView) {
        {
            let workbench = Rc::downgrade(self);
            view.connect_selected_page_notify(move |view| {
                if let Some(workbench) = workbench.upgrade() {
                    // A column shows one file at a time, so the views
                    // stacked in it follow the tab — and the new file
                    // is shown the way that file is shown.
                    workbench.follow_tab(view);
                    workbench.refresh_chrome();
                    workbench.refresh_sidebar();
                    workbench.apply_search_options();
                }
            });
        }
        {
            let workbench = Rc::downgrade(self);
            view.connect_close_page(move |view, tab_page| {
                if let Some(workbench) = workbench.upgrade() {
                    // The last view of a document with unsaved changes
                    // asks before it takes them with it. The close
                    // waits for the answer: adw::TabView expects the
                    // handler to finish it whenever it can.
                    if workbench.asks_about_unsaved(view, tab_page) {
                        return glib::Propagation::Stop;
                    }
                    workbench.forget(tab_page);
                    // A column emptied by its last tab closing is a
                    // divider with nothing on one side of it.
                    if view.n_pages() <= 1 && workbench.is_split() {
                        let weak = Rc::downgrade(&workbench);
                        glib::idle_add_local_once(move || {
                            if let Some(workbench) = weak.upgrade() {
                                workbench.close_empty_columns();
                            }
                        });
                    }
                }
                view.close_page_finish(tab_page, true);
                glib::Propagation::Stop
            });
        }
        {
            // Clicking in a group is how you say which one you mean.
            let workbench = Rc::downgrade(self);
            let controller = gtk::EventControllerFocus::new();
            controller.connect_enter(move |controller| {
                let Some(workbench) = workbench.upgrade() else { return };
                let Some(view) = controller.widget().and_downcast::<adw::TabView>() else {
                    return;
                };
                let at = workbench
                    .all_views()
                    .iter()
                    .position(|other| *other == view);
                if let Some(at) = at {
                    if workbench.focused_column.get() != at
                        || workbench.focused_view.get() != 0
                    {
                        workbench.focused_column.set(at);
                        workbench.focused_view.set(0);
                        workbench.refresh_chrome();
                        workbench.refresh_sidebar();
                    }
                }
            });
            view.add_controller(controller);
        }
    }

    /// The tab group commands act on: the focused column's.
    pub fn active_view(&self) -> adw::TabView {
        let columns = self.columns.borrow();
        columns
            .get(self.focused_column.get())
            .map(|column| column.tab_view.clone())
            .unwrap_or_else(|| self.tab_view.clone())
    }

    /// Every group, for the commands that are about all the open
    /// documents rather than the current one.
    pub fn all_views(&self) -> Vec<adw::TabView> {
        self.columns
            .borrow()
            .iter()
            .map(|column| column.tab_view.clone())
            .collect()
    }

    /// The group a page is in.
    pub fn view_holding(&self, page: &Rc<Page>) -> Option<adw::TabView> {
        self.all_views()
            .into_iter()
            .find(|view| tab_page_of(view, page).is_some())
    }

    /// A column beside this one, showing the same file to start with.
    /// It takes any tab afterwards, and tabs drag between columns.
    pub fn new_column(self: &Rc<Self>) {
        let view = adw::TabView::new();
        view.set_vexpand(true);
        let column = Column::new(&view);
        self.install_group(&view);
        self.columns.borrow_mut().push(column);
        self.rebuild_columns();
        // The new column starts on the file the old one was showing.
        if let Some(page) = self.selected() {
            let copy = Page::view_of(&page.document);
            let tab_page = view.append(&copy.root);
            tab_page.set_title(&copy.display_name());
            self.pages.borrow_mut().push(Rc::clone(&copy));
            view.set_selected_page(&tab_page);
            if let Some(column) = self.columns.borrow().last() {
                *column.showing.borrow_mut() = Some(Rc::clone(&page.document));
            }
        }
        let last = self.columns.borrow().len() - 1;
        self.focus_pane(last, 0);
    }

    /// Takes the focused column away; its files move to the column
    /// beside it, and views that were only a second look at a file
    /// already open there close.
    pub fn close_column(self: &Rc<Self>) {
        if self.columns.borrow().len() < 2 {
            return;
        }
        let at = self.focused_column.get().min(self.columns.borrow().len() - 1);
        let column = self.columns.borrow_mut().remove(at);
        let keep = self
            .columns
            .borrow()
            .get(at.saturating_sub(1))
            .map(|other| other.tab_view.clone());
        if let Some(keep) = keep {
            while column.tab_view.n_pages() > 0 {
                let tab_page = column.tab_view.nth_page(0);
                // A second view of a file that is open where it is
                // going would arrive as a duplicate tab.
                let duplicate = self
                    .pages
                    .borrow()
                    .iter()
                    .find(|page| page.root == tab_page.child())
                    .is_some_and(|page| page.document.views().len() > 1);
                if duplicate {
                    column.tab_view.close_page(&tab_page);
                } else {
                    column
                        .tab_view
                        .transfer_page(&tab_page, &keep, keep.n_pages());
                }
            }
        }
        column.release_views(self);
        self.rebuild_columns();
        self.focus_pane(at.saturating_sub(1), 0);
    }

    /// Another view of this column's file, under the one with the
    /// keyboard.
    pub fn add_view(self: &Rc<Self>) {
        let Some(page) = self.selected() else { return };
        let at = self.focused_column.get();
        let view = Page::view_of(&page.document);
        self.pages.borrow_mut().push(Rc::clone(&view));
        {
            let columns = self.columns.borrow();
            let Some(column) = columns.get(at) else { return };
            column.views.borrow_mut().push(Rc::clone(&view));
        }
        self.rebuild_columns();
        self.record_layout(at);
        let count = self.columns.borrow()[at].views.borrow().len();
        self.focus_pane(at, count);
    }

    /// Takes the focused view out of its column, leaving the rest.
    pub fn close_view(self: &Rc<Self>) {
        let at = self.focused_column.get();
        let which = self.focused_view.get();
        // View 0 is the tab group; it goes with the column.
        if which == 0 {
            return;
        }
        let gone = {
            let columns = self.columns.borrow();
            let Some(column) = columns.get(at) else { return };
            let mut views = column.views.borrow_mut();
            if which > views.len() {
                return;
            }
            views.remove(which - 1)
        };
        self.forget_view(&gone);
        self.rebuild_columns();
        self.record_layout(at);
        self.focus_pane(at, which.saturating_sub(1));
    }

    /// Drops a view that is going away: it stops being one of the
    /// window's pages, and the document stops counting it.
    fn forget_view(&self, page: &Rc<Page>) {
        page.document.drop_view(page);
        self.pages.borrow_mut().retain(|other| !Rc::ptr_eq(other, page));
    }

    /// Whether this window has more than one column.
    pub fn is_split(&self) -> bool {
        self.columns.borrow().len() > 1
    }

    /// Whether there is more than one place for the keyboard to be.
    pub fn has_several_panes(&self) -> bool {
        self.columns.borrow().len() > 1
            || self
                .columns
                .borrow()
                .iter()
                .any(|column| !column.views.borrow().is_empty())
    }

    /// What each column is showing and how many views it has, for the
    /// session to write down.
    pub fn column_state(&self) -> Vec<serde_json::Value> {
        self.columns
            .borrow()
            .iter()
            .map(|column| {
                let file = column
                    .tab_view
                    .selected_page()
                    .and_then(|tab_page| self.page_of(&tab_page))
                    .and_then(|page| page.path().borrow().clone())
                    .unwrap_or_default();
                serde_json::json!({
                    "file": file,
                    "views": column.views.borrow().len() + 1,
                })
            })
            .collect()
    }

    /// Points a column's stacked views at the file its tab bar now
    /// shows, in as many views as that file is shown with.
    fn follow_tab(self: &Rc<Self>, view: &adw::TabView) {
        let Some(at) = self.all_views().iter().position(|other| *other == *view) else {
            return;
        };
        let Some(document) = view
            .selected_page()
            .and_then(|tab_page| self.page_of(&tab_page))
            .map(|page| Rc::clone(&page.document))
        else {
            return;
        };
        let already = {
            let columns = self.columns.borrow();
            let Some(column) = columns.get(at) else { return };
            let showing = column.showing.borrow();
            showing
                .as_ref()
                .is_some_and(|current| Rc::ptr_eq(current, &document))
        };
        if already {
            return;
        }
        // What the outgoing file looked like here is what it looks like
        // when it comes back.
        self.record_layout(at);
        let wanted = document.layout.borrow().views.max(1);
        {
            let columns = self.columns.borrow();
            let Some(column) = columns.get(at) else { return };
            let gone: Vec<Rc<Page>> = column.views.borrow_mut().drain(..).collect();
            for page in gone {
                self.forget_view(&page);
            }
            let mut views = column.views.borrow_mut();
            for _ in 1..wanted {
                let page = Page::view_of(&document);
                self.pages.borrow_mut().push(Rc::clone(&page));
                views.push(page);
            }
            *column.showing.borrow_mut() = Some(Rc::clone(&document));
        }
        self.rebuild_columns();
    }

    /// Writes down how a column is showing its file: the file is what
    /// remembers, so any column it lands in afterwards looks the same.
    fn record_layout(&self, at: usize) {
        let columns = self.columns.borrow();
        let Some(column) = columns.get(at) else { return };
        let document = column.showing.borrow().clone().or_else(|| {
            column
                .tab_view
                .selected_page()
                .and_then(|tab_page| self.page_of(&tab_page))
                .map(|page| Rc::clone(&page.document))
        });
        let Some(document) = document else { return };
        let views = column.views.borrow().len() + 1;
        *document.layout.borrow_mut() = crate::shell::DocumentLayout {
            views,
            dividers: Vec::new(),
        };
        // The file is what remembers, and the project is where that is
        // written down.
        document.record_project_state();
    }

    /// Takes away any column whose last tab has closed: a divider with
    /// nothing on one side of it is not a column.
    pub fn close_empty_columns(self: &Rc<Self>) {
        loop {
            let empty = self
                .columns
                .borrow()
                .iter()
                .position(|column| column.tab_view.n_pages() == 0);
            let Some(at) = empty else { break };
            if self.columns.borrow().len() < 2 {
                break;
            }
            let column = self.columns.borrow_mut().remove(at);
            column.release_views(self);
            self.rebuild_columns();
            let focused = self.focused_column.get();
            self.focus_pane(focused.min(self.columns.borrow().len() - 1), 0);
        }
    }

    /// The keyboard moves to the next place there is one: the next view
    /// down this column, then the next column along.
    pub fn focus_other_group(self: &Rc<Self>) {
        let places: Vec<(usize, usize)> = self
            .columns
            .borrow()
            .iter()
            .enumerate()
            .flat_map(|(index, column)| {
                let extra = column.views.borrow().len();
                (0..=extra).map(move |view| (index, view))
            })
            .collect();
        if places.len() < 2 {
            return;
        }
        let here = (self.focused_column.get(), self.focused_view.get());
        let at = places.iter().position(|place| *place == here).unwrap_or(0);
        let (column, view) = places[(at + 1) % places.len()];
        self.focus_pane(column, view);
    }

    /// Gives a pane the keyboard: view 0 is the column's tab group,
    /// the rest are the views stacked under it.
    pub fn focus_pane(self: &Rc<Self>, column: usize, view: usize) {
        {
            let columns = self.columns.borrow();
            let Some(holder) = columns.get(column) else { return };
            let extra = holder.views.borrow().len();
            let view = view.min(extra);
            self.focused_column.set(column);
            self.focused_view.set(view);
            if view == 0 {
                if let Some(page) = holder
                    .tab_view
                    .selected_page()
                    .and_then(|tab_page| self.page_of(&tab_page))
                {
                    page.view.grab_focus();
                }
            } else if let Some(page) = holder.views.borrow().get(view - 1) {
                page.view.grab_focus();
            }
        }
        self.refresh_chrome();
        self.refresh_sidebar();
    }

    /// Puts the row of columns back together after one is added or
    /// taken away, and each column's stack of views with it.
    ///
    /// GTK's paned holds exactly two children, so a row of N columns is
    /// N−1 of them nested. Rebuilding the chain is what keeps that
    /// bookkeeping in one place.
    fn rebuild_columns(&self) {
        while let Some(child) = self.columns_host.first_child() {
            self.columns_host.remove(&child);
        }
        let columns = self.columns.borrow();
        let sides: Vec<gtk::Widget> = columns
            .iter()
            .map(|column| {
                column.rebuild();
                column.root.clone().upcast::<gtk::Widget>()
            })
            .collect();
        if let Some(row) = nest(gtk::Orientation::Horizontal, &sides) {
            row.set_vexpand(true);
            self.columns_host.append(&row);
        }
    }

    /// The documents this window is the last place to see, with
    /// changes that were never saved.
    pub fn unsaved_documents(&self) -> Vec<Rc<crate::shell::OpenDocument>> {
        let mut found: Vec<Rc<crate::shell::OpenDocument>> = Vec::new();
        for page in self.pages.borrow().iter() {
            let document = &page.document;
            if !document.state.borrow().document.is_dirty() {
                continue;
            }
            if found.iter().any(|other| other.id == document.id) {
                continue;
            }
            // A view in another window keeps the file open there.
            let elsewhere = document.views().iter().any(|view| {
                !self.pages.borrow().iter().any(|mine| Rc::ptr_eq(mine, view))
            });
            if elsewhere {
                continue;
            }
            found.push(Rc::clone(document));
        }
        found
    }

    /// Hands this window's documents to the shell's cache on the way
    /// out, so that reopening one of them within the next twenty
    /// closings gets the buffer back rather than the file.
    fn stash_documents(&self) {
        let pages: Vec<Rc<Page>> = self.pages.borrow().clone();
        for page in pages {
            let document = Rc::clone(&page.document);
            document.drop_view(&page);
            if !document.views().is_empty() {
                continue;
            }
            if let Some(path) = document.path.borrow().clone() {
                Shell::instance().pages.borrow_mut().remove(&path);
                Shell::instance()
                    .pool
                    .borrow_mut()
                    .did_close(Path::new(&path));
            }
            Shell::instance().close_document(document.id);
        }
        self.pages.borrow_mut().clear();
    }

    /// Asks about the files this window would take unsaved changes
    /// from, and answers whether it asked.
    fn asks_about_closing(self: &Rc<Self>) -> bool {
        let unsaved = self.unsaved_documents();
        if unsaved.is_empty() {
            return false;
        }
        let names: Vec<String> = unsaved
            .iter()
            .map(|document| {
                document
                    .path
                    .borrow()
                    .as_deref()
                    .and_then(|path| {
                        Path::new(path)
                            .file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                    })
                    .unwrap_or_else(|| "Untitled".to_string())
            })
            .collect();
        let title = if names.len() == 1 {
            fill(&tr("Save changes to {}?"), &[&names[0]])
        } else {
            fill(
            &tr_n("Save changes to {} file?", "Save changes to {} files?", names.len()),
            &[&names.len().to_string()],
        )
        };
        let dialog = adw::AlertDialog::new(Some(&title), Some(&names.join(", ")));
        dialog.add_response("cancel", &tr("Cancel"));
        dialog.add_response("discard", &tr("Close Without Saving"));
        dialog.add_response("save", &tr("Save All"));
        dialog.set_response_appearance("discard", adw::ResponseAppearance::Destructive);
        dialog.set_response_appearance("save", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("save"));
        dialog.set_close_response("cancel");
        let workbench = Rc::clone(self);
        dialog.connect_response(None, move |_, response| {
            match response {
                "save" => {
                    for document in &unsaved {
                        // An untitled file has nowhere to be written,
                        // so it is kept rather than lost: it goes to
                        // the cache with the rest.
                        if document.path.borrow().is_none() {
                            continue;
                        }
                        let saved = document.state.borrow_mut().document.save().is_ok();
                        if let (true, Some(path)) = (saved, document.path.borrow().as_deref())
                        {
                            Shell::instance().note_own_save(path);
                        }
                    }
                }
                "discard" => {}
                _ => return,
            }
            workbench.closing_settled.set(true);
            workbench.stash_documents();
            workbench.window.close();
        });
        dialog.present(Some(&self.window));
        true
    }

    /// Moves the selection to the next match of what the find bar
    /// holds, or the previous one. Find Next with nothing searched for
    /// opens the bar instead of doing nothing quietly.
    pub fn step_match(self: &Rc<Self>, forward: bool) {
        let Some(page) = self.selected() else { return };
        if self.search_entry.text().is_empty() {
            self.search_bar.set_search_mode(true);
            self.search_entry.grab_focus();
            return;
        }
        let buffer = &page.buffer;
        let (start, end) = (
            buffer.iter_at_mark(&buffer.selection_bound()),
            buffer.iter_at_mark(&buffer.get_insert()),
        );
        let found = if forward {
            page.search_context.forward(&start.max(end))
        } else {
            page.search_context.backward(&start.min(end))
        };
        let Some((mut from, to, _)) = found else {
            self.toast(&tr("No more matches."));
            return;
        };
        buffer.select_range(&to, &from);
        page.view.scroll_to_iter(&mut from, 0.1, false, 0.0, 0.0);
    }

    /// The page a tab holds, if this window holds it.
    fn page_of(&self, tab_page: &adw::TabPage) -> Option<Rc<Page>> {
        self.pages
            .borrow()
            .iter()
            .find(|page| page.root == tab_page.child())
            .map(Rc::clone)
    }

    /// Asks about a document about to lose its last view with changes
    /// that were never saved, and answers whether it asked.
    ///
    /// Save writes the file and closes; Discard closes and gives the
    /// text up; Cancel leaves the tab where it is. Any of the three
    /// finishes the close the tab view is waiting on.
    fn asks_about_unsaved(
        self: &Rc<Self>,
        view: &adw::TabView,
        tab_page: &adw::TabPage,
    ) -> bool {
        let Some(page) = self.page_of(tab_page) else { return false };
        // Another view keeps the text; this one is free to go.
        if page.document.views().len() > 1 {
            return false;
        }
        if !page.state.borrow().document.is_dirty() {
            return false;
        }
        let name = self.disambiguated_name(&page);
        let dialog = adw::AlertDialog::new(
            Some(&format!("Save changes to {name}?")),
            Some(&tr("Changes that are not saved are lost when the file closes.")),
        );
        dialog.add_response("cancel", &tr("Cancel"));
        dialog.add_response("discard", &tr("Discard"));
        dialog.add_response("save", &tr("Save"));
        dialog.set_response_appearance("discard", adw::ResponseAppearance::Destructive);
        dialog.set_response_appearance("save", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("save"));
        dialog.set_close_response("cancel");
        let workbench = Rc::clone(self);
        let view = view.clone();
        let tab_page = tab_page.clone();
        dialog.connect_response(None, move |_, response| {
            match response {
                "save" => {
                    // An untitled document has nowhere to be written
                    // yet, so Save asks where before it closes.
                    let has_path = page.path().borrow().is_some();
                    if has_path {
                        let saved = page.state.borrow_mut().document.save().is_ok();
                        if let (true, Some(path)) = (saved, page.path().borrow().as_deref()) {
                            Shell::instance().note_own_save(path);
                        }
                        if !saved {
                            workbench.toast(&tr("The file could not be saved."));
                            view.close_page_finish(&tab_page, false);
                            return;
                        }
                    } else {
                        // Nowhere to write it yet: Save As asks where,
                        // and the tab stays open behind the question.
                        view.set_selected_page(&tab_page);
                        let _ = gtk::prelude::WidgetExt::activate_action(
                            &workbench.window,
                            "win.save-as",
                            None,
                        );
                        view.close_page_finish(&tab_page, false);
                        return;
                    }
                }
                "discard" => {}
                _ => {
                    view.close_page_finish(&tab_page, false);
                    return;
                }
            }
            workbench.forget(&tab_page);
            view.close_page_finish(&tab_page, true);
        });
        dialog.present(Some(&self.window));
        true
    }

    pub fn forget(&self, tab_page: &adw::TabPage) {
        let mut pages = self.pages.borrow_mut();
        let Some(index) = pages
            .iter()
            .position(|page| page.root == tab_page.child())
        else {
            return;
        };
        let page = pages.remove(index);
        drop(pages);

        // The view closed. The document closes with it only when it was
        // the last view of it: half a split going away leaves the file
        // open in the other half, with its server session and its
        // unsaved text where they were.
        let document = Rc::clone(&page.document);
        document.drop_view(&page);
        if !document.views().is_empty() {
            crate::session::save();
            return;
        }

        if let Some(path) = document.path.borrow().clone() {
            // Remember it before the handles go: reopening should
            // land the caret where it was, not at the top.
            let (line, character) = page::lsp_caret(&document.buffer);
            let mut closed = self.closed_tabs.borrow_mut();
            let path_buf = PathBuf::from(&path);
            closed.retain(|(other, _, _)| *other != path_buf);
            closed.push((path_buf, line as i32, character as usize));
            let overflow = closed.len().saturating_sub(CLOSED_TAB_MEMORY);
            closed.drain(..overflow);
            drop(closed);
            Shell::instance().pages.borrow_mut().remove(&path);
            Shell::instance()
                .pool
                .borrow_mut()
                .did_close(Path::new(&path));
        }
        Shell::instance().close_document(document.id);
        crate::session::save();
    }

    pub fn selected(&self) -> Option<Rc<Page>> {
        let view = self.active_view();
        let selected = view.selected_page()?;
        let child = selected.child();
        self.pages
            .borrow()
            .iter()
            .find(|page| page.root == child)
            .cloned()
    }

    /// Opens `path` as a tab (or focuses the tab already showing it,
    /// anywhere), then optionally reveals a position.
    pub fn open(self: &Rc<Self>, path: Option<PathBuf>, at: Option<(i32, usize)>) {
        // Navigations to a position (definitions, search results,
        // outline picks) leave a trail; plain opens do not, and
        // neither does retracing the trail itself.
        if at.is_some() && !self.retracing.get() {
            self.note_jump();
        }
        if let Some(path) = &path {
            let key = path.to_string_lossy().into_owned();
            let existing = Shell::instance().pages.borrow().get(&key).cloned();
            if let Some(handles) = existing {
                handles.tab_view.set_selected_page(&handles.tab_page);
                handles.window.present();
                if let Some((line, character)) = at {
                    page::reveal(&handles, line, character);
                }
                return;
            }
        }
        let page = Page::new(path);
        let view = self.active_view();
        let tab_page = view.append(&page.root);
        tab_page.set_title(&page.display_name());
        self.pages.borrow_mut().push(Rc::clone(&page));
        if let Some(path) = page.path().borrow().clone() {
            let handles = Rc::new(PageHandles {
                window: self.window.clone(),
                tab_view: view.clone(),
                tab_page: tab_page.clone(),
                document: Rc::clone(&page.document),
                view: page.view.clone(),
                toasts: self.toasts.clone(),
                title: self.title.clone(),
                language: RefCell::new(
                    page.state
                        .borrow()
                        .document
                        .language_name()
                        .unwrap_or("")
                        .to_string(),
                ),
                problems: RefCell::new(String::new()),
                detail: RefCell::new(String::new()),
            });
            Shell::instance().pages.borrow_mut().insert(path, handles);
        }
        view.set_selected_page(&tab_page);
        if let Some((line, character)) = at {
            if let Some(path) = page.path().borrow().clone() {
                if let Some(handles) = Shell::instance().pages.borrow().get(&path).cloned() {
                    page::reveal(&handles, line, character);
                }
            }
        }
        // A document told what it is stays told: reopening a .txt that
        // holds SQL should not find it plain text again. What it was
        // told is data about the file, so it comes from the project
        // record; the configuration is where it used to live, and is
        // still read for files recorded before there was a record.
        if let Some(path) = page.path().borrow().clone() {
            *page.document.project_root.borrow_mut() =
                textchum_core::workspace::project_root_for(Path::new(&path))
                    .map(|root| root.to_string_lossy().into_owned());
            page.document.adopt_project_state();
            let recorded = page.document.language_override.borrow().clone();
            let stored = Shell::instance().config.borrow().file_override(&path);
            if recorded.is_some() {
                apply_file_properties(self, &page, recorded.as_deref(), stored.tab_width);
            } else if !stored.is_empty() {
                if let Some(language) = &stored.language {
                    *page.document.language_override.borrow_mut() = Some(language.clone());
                }
                apply_file_properties(
                    self,
                    &page,
                    stored.language.as_deref(),
                    stored.tab_width,
                );
            }
        }
        self.refresh_chrome();
        self.refresh_sidebar();
        // The desktop's shared recent-files list learns about it too.
        if let Some(path) = self.selected().and_then(|page| page.path().borrow().clone()) {
            let uri = gtk::gio::File::for_path(&path).uri();
            let _ = gtk::RecentManager::default().add_item(&uri);
        }
        crate::session::save();
    }

    /// Puts a view of a document that is already open in the group
    /// with the focus, and selects it.
    pub fn show(self: &Rc<Self>, document: &Rc<crate::shell::OpenDocument>, at: Option<(i32, usize)>) {
        let page = Page::view_of(document);
        let view = self.active_view();
        let tab_page = view.append(&page.root);
        tab_page.set_title(&page.display_name());
        self.pages.borrow_mut().push(Rc::clone(&page));
        if let Some(path) = document.path.borrow().clone() {
            let handles = Rc::new(PageHandles {
                window: self.window.clone(),
                tab_view: view.clone(),
                tab_page: tab_page.clone(),
                document: Rc::clone(document),
                view: page.view.clone(),
                toasts: self.toasts.clone(),
                title: self.title.clone(),
                language: RefCell::new(
                    page.state
                        .borrow()
                        .document
                        .language_name()
                        .unwrap_or("")
                        .to_string(),
                ),
                problems: RefCell::new(String::new()),
                detail: RefCell::new(String::new()),
            });
            Shell::instance().pages.borrow_mut().insert(path.clone(), Rc::clone(&handles));
            if let Some((line, character)) = at {
                page::reveal(&handles, line, character);
            }
            // The server hears about it again: it was told the file
            // closed when the last view went.
            if let Some(language) = page.state.borrow().document.language_name() {
                let text = page.state.borrow().document.text();
                Shell::instance()
                    .pool
                    .borrow_mut()
                    .did_open(Path::new(&path), language, &text);
            }
        }
        view.set_selected_page(&tab_page);
        self.refresh_chrome();
        self.refresh_sidebar();
    }

    // MARK: Chrome

    /// The page's display name, with enough parent path to tell it
    /// apart when another open file shares its bare name.
    pub fn disambiguated_name(&self, page: &Rc<Page>) -> String {
        if self.show_full_paths.get() {
            if let Some(path) = page.path().borrow().clone() {
                let root = workspace::project_root_for(Path::new(&path))
                    .map(|root| root.to_string_lossy().into_owned());
                return crate::path_actions::relative_path(&path, root.as_deref());
            }
        }
        let name = page.display_name();
        let collides = self.pages.borrow().iter().any(|other| {
            !Rc::ptr_eq(other, page) && other.display_name() == name
        });
        if !collides {
            return name;
        }
        let Some(path) = page.path().borrow().clone() else { return name };
        Path::new(&path)
            .parent()
            .and_then(Path::file_name)
            .map(|parent| format!("{}/{name}", parent.to_string_lossy()))
            .unwrap_or(name)
    }

    /// Redraws the status bar from the selected page.
    pub fn refresh_status(&self) {
        let Some(page) = self.selected() else {
            self.status_position.set_text("");
            self.status_indent.set_label("");
            self.status_language.set_label("");
            self.status_encoding.set_text("");
            return;
        };
        let buffer = &page.buffer;
        let at = buffer.iter_at_mark(&buffer.get_insert());
        self.status_position.set_text(&fill(
            &tr("Ln {}, Col {}"),
            &[&(at.line() + 1).to_string(), &(at.line_offset() + 1).to_string()],
        ));
        let width = page.view.tab_width();
        // The override answers when it says; the file itself otherwise,
        // the way formatting already reads it.
        let stored = page
            .path()
            .borrow()
            .as_ref()
            .map(|path| Shell::instance().config.borrow().file_override(path));
        // Searched through iterators, not a copied string: the bar
        // redraws on every caret move, and copying the whole buffer per
        // arrow key is what stuttered on large files.
        let uses_tabs = stored.and_then(|s| s.spaces.map(|spaces| !spaces)).unwrap_or_else(|| {
            let start = buffer.start_iter();
            start.char() == '\t'
                || start
                    .forward_search("\n\t", gtk::TextSearchFlags::TEXT_ONLY, None)
                    .is_some()
        });
        self.status_indent.set_label(&if uses_tabs {
            fill(&tr("Tabs: {}"), &[&width.to_string()])
        } else {
            fill(&tr("Spaces: {}"), &[&width.to_string()])
        });
        let state = page.state.borrow();
        let language = state
            .document
            .language_name()
            .map(str::to_string)
            .unwrap_or_else(|| tr("Plain Text"));
        self.status_language.set_label(&language);
        self.status_encoding.set_text(state.document.encoding().name());
    }

    /// The bar's four parts as one line, for the smoke test to read.
    pub fn status_summary(&self) -> String {
        format!(
            "{} | {} | {} | {}",
            self.status_position.text(),
            self.status_indent.label().unwrap_or_default(),
            self.status_language.label().unwrap_or_default(),
            self.status_encoding.text(),
        )
    }

    pub fn refresh_chrome(&self) {
        self.refresh_status();
        let Some(page) = self.selected() else {
            self.title.set_title("Textchum");
            self.title.set_subtitle("");
            return;
        };
        let state = page.state.borrow();
        let dirty = if state.document.is_dirty() { "● " } else { "" };
        drop(state);
        let shown = self.disambiguated_name(&page);
        let state = page.state.borrow();
        self.title.set_title(&format!("{dirty}{shown}"));
        if let Some(tab_page) = self.active_view().selected_page() {
            tab_page.set_title(&format!("{dirty}{shown}"));
        }
        let detail = format!(
            "{} · {} bytes",
            state.document.encoding().name(),
            state.document.len_bytes()
        );
        drop(state);
        if let Some(path) = page.path().borrow().clone() {
            if let Some(handles) = Shell::instance().pages.borrow().get(&path) {
                *handles.detail.borrow_mut() = detail;
                refresh_subtitle(handles);
                self.rebuild_buffer_list();
                return;
            }
        }
        self.title.set_subtitle("");
        self.rebuild_buffer_list();
    }

    // MARK: Sidebar (project file tree)

    pub fn refresh_sidebar(&self) {
        self.rebuild_buffer_list();
        self.rebuild_tree();
    }

    /// The open-buffers half: every tab, grouped by project, the
    /// selected one emphasized; clicking a row selects its tab.
    /// Rebuilt only when the (title, dirty, selected) signature moves.
    pub fn rebuild_buffer_list(&self) {
        let selected = self.selected();
        let signature: Vec<(String, bool, bool)> = self
            .pages
            .borrow()
            .iter()
            .map(|page| {
                (
                    self.disambiguated_name(page),
                    page.state.borrow().document.is_dirty(),
                    selected.as_ref().is_some_and(|s| Rc::ptr_eq(s, page)),
                )
            })
            .collect();
        if *self.buffer_signature.borrow() == signature {
            return;
        }
        *self.buffer_signature.borrow_mut() = signature;

        while let Some(child) = self.buffers_box.first_child() {
            self.buffers_box.remove(&child);
        }
        let heading = gtk::Label::new(Some(&tr("Open Files")));
        heading.set_xalign(0.0);
        heading.add_css_class("heading");
        heading.set_margin_start(12);
        heading.set_margin_top(10);
        heading.set_margin_bottom(4);
        self.buffers_box.append(&heading);

        // Group pages by project root, loose files last.
        let mut groups: Vec<(String, Vec<Rc<Page>>)> = Vec::new();
        for page in self.pages.borrow().iter() {
            let root = page
                .path()
                .borrow()
                .as_deref()
                .map(Path::new)
                .and_then(workspace::project_root_for);
            let label = root
                .as_deref()
                .and_then(Path::file_name)
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Other".into());
            match groups.iter_mut().find(|(name, _)| *name == label) {
                Some((_, pages)) => pages.push(Rc::clone(page)),
                None => groups.push((label, vec![Rc::clone(page)])),
            }
        }
        groups.sort_by(|a, b| {
            (a.0 == "Other").cmp(&(b.0 == "Other")).then(a.0.cmp(&b.0))
        });

        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::None);
        list.add_css_class("navigation-sidebar");
        let mut rows: Vec<Option<Rc<Page>>> = Vec::new();
        for (group, pages) in groups {
            let header = gtk::Label::new(Some(&group));
            header.set_xalign(0.0);
            header.add_css_class("dim-label");
            header.set_margin_start(6);
            header.set_margin_top(6);
            let header_row = gtk::ListBoxRow::new();
            header_row.set_child(Some(&header));
            header_row.set_activatable(false);
            list.append(&header_row);
            rows.push(None);
            for page in pages {
                let dirty = page.state.borrow().document.is_dirty();
                let is_selected =
                    selected.as_ref().is_some_and(|s| Rc::ptr_eq(s, &page));
                let label = gtk::Label::new(Some(&format!(
                    "{}{}",
                    if dirty { "● " } else { "" },
                    self.disambiguated_name(&page)
                )));
                label.set_xalign(0.0);
                label.set_margin_start(14);
                if is_selected {
                    label.add_css_class("heading");
                }
                let row = gtk::ListBoxRow::new();
                row.set_child(Some(&label));
                list.append(&row);
                rows.push(Some(Rc::clone(&page)));
            }
        }
        *self.buffer_rows.borrow_mut() = rows;
        {
            let workbench = Rc::downgrade(&WORKBENCHES.with(|list| {
                list.borrow()
                    .iter()
                    .find(|w| w.window == self.window)
                    .cloned()
                    .expect("workbench registered")
            }));
            list.connect_row_activated(move |_, row| {
                let Some(workbench) = workbench.upgrade() else { return };
                let index = row.index();
                let page = workbench
                    .buffer_rows
                    .borrow()
                    .get(index as usize)
                    .cloned()
                    .flatten();
                if let Some(page) = page {
                    if let Some(view) = workbench.view_holding(&page) {
                        let Some(tab_page) = tab_page_of(&view, &page) else { return };
                        if let Some(at) =
                            workbench.all_views().iter().position(|other| *other == view)
                        {
                            workbench.focused_column.set(at);
                            workbench.focused_view.set(0);
                        }
                        view.set_selected_page(&tab_page);
                    }
                }
            });
        }
        // Right-click: the copy-path menu for that row's document.
        {
            let workbench = Rc::downgrade(&WORKBENCHES.with(|all| {
                all.borrow()
                    .iter()
                    .find(|w| w.window == self.window)
                    .cloned()
                    .expect("workbench registered")
            }));
            let gesture = gtk::GestureClick::new();
            gesture.set_button(3);
            let list_in_handler = list.clone();
            let list = list.clone();
            gesture.connect_pressed(move |gesture, _, x, y| {
                let list = &list_in_handler;
                let Some(workbench) = workbench.upgrade() else { return };
                let Some(row) = list.row_at_y(y as i32) else { return };
                let page = workbench
                    .buffer_rows
                    .borrow()
                    .get(row.index() as usize)
                    .cloned()
                    .flatten();
                let Some(path) = page.and_then(|page| page.path().borrow().clone()) else {
                    return;
                };
                let menu = copy_menu(&workbench, &path);
                let popover = gtk::PopoverMenu::from_model(Some(&menu));
                popover.set_parent(list);
                popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(
                    x as i32, y as i32, 1, 1,
                )));
                popover.set_has_arrow(false);
                popover.popup();
                gesture.set_state(gtk::EventSequenceState::Claimed);
            });
            list.add_controller(gesture);
        }
        self.buffers_box.append(&list);
    }

    /// The file-tree half, rebuilt when the project root changes.
    fn rebuild_tree(&self) {
        let root = self.selected().and_then(|page| {
            page.path()
                .borrow()
                .as_deref()
                .map(Path::new)
                .and_then(|path| {
                    workspace::project_root_for(path)
                        .or_else(|| path.parent().map(Path::to_owned))
                })
        });
        if *self.sidebar_root.borrow() == root {
            return;
        }
        *self.sidebar_root.borrow_mut() = root.clone();
        while let Some(child) = self.tree_box.first_child() {
            self.tree_box.remove(&child);
        }
        let Some(root) = root else { return };

        let header = gtk::Label::new(Some(
            &root
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| root.to_string_lossy().into_owned()),
        ));
        header.set_xalign(0.0);
        header.add_css_class("heading");
        header.set_margin_start(12);
        header.set_margin_top(10);
        header.set_margin_bottom(4);
        self.tree_box.append(&header);

        let tree = build_file_tree(&root);
        let scrolled = gtk::ScrolledWindow::builder().child(&tree).vexpand(true).build();
        self.tree_box.append(&scrolled);
    }
}

/// Selected-page chrome refresh, callable from page signal handlers.
/// The caret moved in `page`: the bar of whichever window shows it.
/// The pinned-context switch moved: every view redraws its strip.
/// The gutter's baseline moved: every page re-reads its marks.
pub fn refresh_all_change_marks() {
    WORKBENCHES.with(|list| {
        for workbench in list.borrow().iter() {
            for page in workbench.pages.borrow().iter() {
                page::refresh_change_marks(page);
            }
        }
    });
}

pub fn refresh_all_context_strips() {
    WORKBENCHES.with(|list| {
        for workbench in list.borrow().iter() {
            for page in workbench.pages.borrow().iter() {
                page::refresh_context_strip(page);
            }
        }
    });
}

pub fn refresh_status_for(page: &Rc<Page>) {
    WORKBENCHES.with(|list| {
        for workbench in list.borrow().iter() {
            if let Some(selected) = workbench.selected() {
                if Rc::ptr_eq(&selected, page) {
                    workbench.refresh_status();
                }
            }
        }
    });
}

pub fn refresh_chrome_for(page: &Rc<Page>) {
    WORKBENCHES.with(|list| {
        for workbench in list.borrow().iter() {
            if let Some(selected) = workbench.selected() {
                if Rc::ptr_eq(&selected, page) {
                    workbench.refresh_chrome();
                }
            }
        }
    });
}

/// Rebuilds a page's window subtitle from its language and problems.
pub fn refresh_subtitle(handles: &PageHandles) {
    if handles
        .tab_view
        .selected_page()
        .is_some_and(|selected| selected.as_ptr() == handles.tab_page.as_ptr())
    {
        let detail = handles.detail.borrow();
        let language = handles.language.borrow();
        let problems = handles.problems.borrow();
        let subtitle = [detail.as_str(), language.as_str(), problems.as_str()]
            .iter()
            .filter(|half| !half.is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join(" · ");
        handles.title.set_subtitle(&subtitle);
    }
}

// MARK: File tree

/// A lazily-populated tree over the project directory: directories
/// expand on demand, activating a file opens it as a tab.
/// Redraws every window's file tree, so a pack put on or taken off
/// shows without reopening anything. The rows read the pack as they
/// draw, so forgetting the remembered root is enough to make them.
fn refresh_file_trees() {
    WORKBENCHES.with(|list| {
        for workbench in list.borrow().iter() {
            *workbench.sidebar_root.borrow_mut() = None;
            workbench.rebuild_tree();
        }
    });
}

/// The icon pack's image for a file, if a pack is loaded and has one.
///
/// The language is the one the file's name implies rather than whatever
/// an open tab decided: a tree row is a file on disk, and most of them
/// are not open.
fn packed_icon(path: &Path) -> Option<std::path::PathBuf> {
    if !icons::is_active() {
        return None;
    }
    let name = path.file_name()?.to_string_lossy();
    let language = textchum_core::syntax::languages::by_path(path).map(|found| found.spec.name);
    // The appearance in force, not the one configured: "system"
    // resolves through libadwaita, which is what the widgets follow.
    let light = !adw::StyleManager::default().is_dark();
    icons::icon_for(&name, language, light)
}

fn build_file_tree(root: &Path) -> gtk::ListView {
    // The configuration decides what the tree hides: dotfiles by
    // default, plus whatever globs the workspace section adds.
    let hide_globs: Rc<Vec<String>> = Rc::new(
        Shell::instance()
            .config
            .borrow()
            .hide_globs(Some(&root.to_string_lossy())),
    );
    let children_of = {
        let hide_globs = Rc::clone(&hide_globs);
        move |path: &Path| -> gtk::gio::ListStore {
        let store = gtk::gio::ListStore::new::<gtk::StringObject>();
        let mut entries: Vec<(bool, String)> = std::fs::read_dir(path)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                if workspace::is_hidden(&name, &hide_globs) {
                    return None;
                }
                let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
                Some((is_dir, entry.path().to_string_lossy().into_owned()))
            })
            .collect();
        // Directories first, then case-insensitive by name.
        entries.sort_by_key(|(is_dir, path)| {
            (
                !*is_dir,
                Path::new(path)
                    .file_name()
                    .map(|name| name.to_string_lossy().to_lowercase())
                    .unwrap_or_default(),
            )
        });
        for (_, path) in entries {
            store.append(&gtk::StringObject::new(&path));
        }
        store
        }
    };

    let root_model = children_of(root);
    let tree_model = gtk::TreeListModel::new(root_model, false, false, move |item| {
        let path = item.downcast_ref::<gtk::StringObject>()?.string();
        let path = Path::new(path.as_str());
        if path.is_dir() {
            Some(children_of(path).upcast())
        } else {
            None
        }
    });
    let selection = gtk::SingleSelection::new(Some(tree_model));

    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        let expander = gtk::TreeExpander::new();
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let icon = gtk::Image::new();
        let label = gtk::Label::new(None);
        label.set_xalign(0.0);
        row.append(&icon);
        row.append(&label);
        expander.set_child(Some(&row));
        item.set_child(Some(&expander));
    });
    factory.connect_bind(|_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        let Some(tree_row) = item.item().and_downcast::<gtk::TreeListRow>() else {
            return;
        };
        let Some(expander) = item.child().and_downcast::<gtk::TreeExpander>() else {
            return;
        };
        expander.set_list_row(Some(&tree_row));
        let Some(path) = tree_row.item().and_downcast::<gtk::StringObject>() else {
            return;
        };
        let path = PathBuf::from(path.string().as_str());
        let Some(row) = expander.child().and_downcast::<gtk::Box>() else {
            return;
        };
        let icon = row.first_child().and_downcast::<gtk::Image>();
        let label = row.last_child().and_downcast::<gtk::Label>();
        if let Some(icon) = icon {
            if path.is_dir() {
                icon.set_icon_name(Some("folder-symbolic"));
            } else {
                // An icon pack first, when one is loaded: it knows
                // hundreds of types and knows `Dockerfile` by name,
                // where the desktop's table runs out early. Failing
                // that, the desktop's own icon for the content type,
                // guessed from the name — the system icons the Mac
                // shell gets from NSWorkspace.
                let packed = packed_icon(&path);
                if let Some(file) = packed {
                    icon.set_from_file(Some(&file));
                    icon.set_pixel_size(16);
                } else {
                    let (content_type, _) = gtk::gio::content_type_guess(
                        Some(std::path::Path::new(&path)),
                        &[],
                    );
                    let themed = gtk::gio::content_type_get_symbolic_icon(&content_type);
                    icon.set_from_gicon(&themed);
                }
            }
        }
        if let Some(label) = label {
            label.set_text(
                &path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            );
        }
    });

    let view = gtk::ListView::new(Some(selection.clone()), Some(factory));
    // One click opens, the way the Mac shell's tree taps do — and
    // GtkListView's double-click activation was not reaching the
    // handler at all through the TreeExpander rows.
    view.set_single_click_activate(true);
    view.connect_activate(move |_, position| {
        let Some(tree_row) = selection
            .item(position)
            .and_downcast::<gtk::TreeListRow>()
        else {
            return;
        };
        let Some(path) = tree_row.item().and_downcast::<gtk::StringObject>() else {
            return;
        };
        let path = PathBuf::from(path.string().as_str());
        if path.is_dir() {
            tree_row.set_expanded(!tree_row.is_expanded());
        } else if let Some(workbench) = Workbench::active() {
            workbench.open(Some(path), None);
        }
    });
    view
}

// MARK: Actions

fn install_actions(app: &adw::Application, workbench: &Rc<Workbench>) {
    let window = &workbench.window;
    let add = |name: &str, workbench: &Rc<Workbench>, f: fn(&Rc<Workbench>, &adw::Application)| {
        let action = gtk::gio::SimpleAction::new(name, None);
        let weak = Rc::downgrade(workbench);
        let app = app.clone();
        action.connect_activate(move |_, _| {
            if let Some(workbench) = weak.upgrade() {
                f(&workbench, &app);
            }
        });
        workbench.window.add_action(&action);
    };

    add("new-tab", workbench, |workbench, _| {
        workbench.open(None, None);
    });
    add("new", workbench, |_, app| {
        let fresh = Workbench::new(app);
        fresh.open(None, None);
        fresh.window.present();
    });
    add("open", workbench, |workbench, _| {
        let dialog = gtk::FileDialog::new();
        let workbench = Rc::clone(workbench);
        // Opening several at once, because the window has tabs: picking
        // files one dialog at a time is the only thing a single-select
        // chooser buys, and it buys nothing.
        dialog.open_multiple(
            Some(&workbench.window.clone()),
            gtk::gio::Cancellable::NONE,
            move |result| {
                let Ok(files) = result else { return };
                for file in files.iter::<gtk::gio::File>().flatten() {
                    if let Some(path) = file.path() {
                        workbench.open(Some(path), None);
                    }
                }
            },
        );
    });
    add("reopen-tab", workbench, |workbench, _| {
        let Some((path, line, character)) = workbench.closed_tabs.borrow_mut().pop() else {
            workbench.toast(&tr("No recently closed tab to reopen."));
            return;
        };
        // A file closed a moment ago is still in the shell's cache,
        // whole: reopening it is taking the closing back, so the text
        // that was never saved and the blocks that were folded come
        // back with it.
        let key = path.to_string_lossy().into_owned();
        if let Some(document) = Shell::instance().reclaim_document(&key) {
            workbench.show(&document, Some((line, character)));
            return;
        }
        if !path.is_file() {
            workbench.toast(&fill(&tr("{} is no longer there."), &[&path.display().to_string()]));
            return;
        }
        // Reopening is a jump to where the caret was, not a plain open,
        // so `at` carries the remembered position.
        workbench.open(Some(path), Some((line, character)));
    });
    add("save", workbench, |workbench, _| {
        let Some(page) = workbench.selected() else { return };
        let workbench = Rc::clone(workbench);
        preprocess_gate(&workbench.clone(), &page, move |workbench, page| {
            let saved = page.state.borrow_mut().document.save().is_ok();
            if saved {
                if let Some(path) = page.path().borrow().as_deref() {
                    Shell::instance().note_own_save(path);
                }
                workbench.refresh_chrome();
                // Saving moves what git compares against.
                page::refresh_change_marks(&page);
                page.view.grab_focus();
            } else {
                let _ = gtk::prelude::WidgetExt::activate_action(
                    &workbench.window,
                    "win.save-as",
                    None,
                );
            }
        });
    });
    add("save-as", workbench, |workbench, _| {
        let Some(page) = workbench.selected() else { return };
        let dialog = gtk::FileDialog::new();
        // Seed the folder from the document itself or, for untitled
        // ones, from any open file — the user is probably adding a
        // file to that project.
        let seed = page
            .path()
            .borrow()
            .clone()
            .or_else(|| {
                workbench
                    .all_pages()
                    .iter()
                    .find_map(|other| other.path().borrow().clone())
            })
            .and_then(|path| Path::new(&path).parent().map(Path::to_owned));
        if let Some(folder) = seed {
            dialog.set_initial_folder(Some(&gtk::gio::File::for_path(&folder)));
        }
        let workbench = Rc::clone(workbench);
        dialog.save(
            Some(&workbench.window.clone()),
            gtk::gio::Cancellable::NONE,
            move |result| {
                if let Ok(file) = result {
                    if let Some(path) = file.path() {
                        save_page_as(&workbench, &page, &path);
                    }
                }
            },
        );
    });
    add("undo", workbench, |workbench, _| replay(workbench, true));
    add("redo", workbench, |workbench, _| replay(workbench, false));
    add("find", workbench, |workbench, _| {
        workbench.search_bar.set_search_mode(true);
        workbench.search_entry.grab_focus();
    });
    add("preview", workbench, |workbench, _| {
        let Some(page) = workbench.selected() else { return };
        match &page.preview {
            Some(web) => {
                web.set_visible(!web.is_visible());
                page.update_preview_now();
            }
            None => workbench.toast(&tr("Not a Markdown document.")),
        }
    });
    add("sidebar", workbench, |workbench, _| {
        workbench
            .split
            .set_show_sidebar(!workbench.split.shows_sidebar());
    });
    add("fold", workbench, |workbench, _| {
        let Some(page) = workbench.selected() else { return };
        if !crate::page::toggle_fold(&page) {
            workbench.toast(&tr("No block opens on this line."));
        }
    });
    add("fold-all", workbench, |workbench, _| {
        let Some(page) = workbench.selected() else { return };
        if !crate::page::fold_all(&page) {
            workbench.toast(&tr("Nothing here folds."));
        }
    });
    add("unfold-all", workbench, |workbench, _| {
        let Some(page) = workbench.selected() else { return };
        if !crate::page::unfold_all(&page) {
            workbench.toast(&tr("Nothing is folded."));
        }
    });
    add("new-column", workbench, |workbench, _| {
        workbench.new_column();
    });
    add("close-column", workbench, |workbench, _| {
        if !workbench.is_split() {
            workbench.toast(&tr("This window has one column."));
            return;
        }
        workbench.close_column();
    });
    add("add-view", workbench, |workbench, _| {
        workbench.add_view();
    });
    add("close-view", workbench, |workbench, _| {
        workbench.close_view();
    });
    add("focus-other-group", workbench, |workbench, _| {
        if !workbench.has_several_panes() {
            workbench.toast(&tr("There is one pane in this window."));
            return;
        }
        workbench.focus_other_group();
    });
    add("close-tab", workbench, |workbench, _| {
        let view = workbench.active_view();
        if let Some(selected) = view.selected_page() {
            view.close_page(&selected);
        }
    });
    add("definition", workbench, |workbench, _| {
        let Some(page) = workbench.selected() else { return };
        let Some(path) = page.path().borrow().clone() else {
            workbench.toast(&tr("Save the file first — untitled documents have no server."));
            return;
        };
        let (line, character) = page::lsp_anchor(&page);
        let shell = Shell::instance();
        let id = shell
            .pool
            .borrow_mut()
            .definition(Path::new(&path), line, character);
        if id == 0 {
            // No server: the ctags fallback gets its chance before the
            // explanation does.
            if !ctags_jump(workbench, &page, &path) {
                workbench.toast(&tr("No language server is running for this document."));
            }
            return;
        }
        let weak = Rc::downgrade(workbench);
        let fallback_page = Rc::clone(&page);
        shell.expect_response(id, move |json| {
            let Some(workbench) = weak.upgrade() else { return };
            use textchum_core::definition::Decision;
            match textchum_core::definition::decide(json, &path, line, character) {
                Decision::Jump(target) => {
                    workbench.open(
                        Some(PathBuf::from(target.path)),
                        Some((target.line as i32, target.character as usize)),
                    );
                }
                Decision::AlreadyThere => {
                    uses_of_definition(&workbench, &path, line, character);
                }
                Decision::Choose(targets) => {
                    show_places(&workbench, "Definitions", places(&targets));
                }
                Decision::Nothing => {
                    // The server answered but had nothing; consult the
                    // index for projects that opted in.
                    if !ctags_jump(&workbench, &fallback_page, &path) {
                        workbench.toast(&tr("No definition found."));
                    }
                }
            }
        });
    });
    add("find-next", workbench, |workbench, _| {
        workbench.step_match(true);
    });
    add("find-previous", workbench, |workbench, _| {
        workbench.step_match(false);
    });
    add("find-replace", workbench, |workbench, _| {
        // The find bar carries the replace row; this is the same bar
        // with the caret in the second field.
        workbench.search_bar.set_search_mode(true);
        workbench.replace_entry.grab_focus();
    });
    add("reveal-in-tree", workbench, |workbench, _| {
        let Some(page) = workbench.selected() else { return };
        let Some(path) = page.path().borrow().clone() else {
            workbench.toast(&tr("Save the file first — an untitled document has no path."));
            return;
        };
        let _ = path;
        // The navigator shows the tree of the selected page's project,
        // so revealing is opening it and letting it rebuild.
        workbench.split.set_show_sidebar(true);
        workbench.refresh_sidebar();
    });
    add("complete", workbench, |workbench, _| {
        let Some(page) = workbench.selected() else { return };
        crate::page::complete_now(&page);
    });
    add("find-in-project", workbench, |workbench, _| {
        let root = workbench
            .selected()
            .and_then(|page| {
                page.path().borrow().as_deref().map(Path::new).and_then(|path| {
                    workspace::project_root_for(path)
                        .or_else(|| path.parent().map(Path::to_owned))
                })
            })
            .unwrap_or_else(glib::home_dir);
        show_grep(workbench, root);
    });
    add("changed-files", workbench, |workbench, _| {
        let root = workbench
            .selected()
            .and_then(|page| {
                page.path().borrow().as_deref().map(Path::new).and_then(|path| {
                    workspace::project_root_for(path)
                        .or_else(|| path.parent().map(Path::to_owned))
                })
            })
            .unwrap_or_else(glib::home_dir);
        show_changed_files(workbench, root);
    });
    add("quick-open", workbench, |workbench, _| {
        let root = workbench
            .selected()
            .and_then(|page| {
                page.path().borrow().as_deref().map(Path::new).and_then(|path| {
                    workspace::project_root_for(path)
                        .or_else(|| path.parent().map(Path::to_owned))
                })
            })
            .unwrap_or_else(glib::home_dir);
        show_quick_open(workbench, root);
    });
    add("goto-line", workbench, |workbench, _| {
        let Some(page) = workbench.selected() else { return };
        let lines = page.state.borrow().document.len_lines();
        let dialog = adw::AlertDialog::new(
            Some(&tr("Go to Line")),
            Some(&format!("Line number, or line:column — of {lines}.")),
        );
        let entry = gtk::Entry::new();
        entry.set_placeholder_text(Some("412 or 412:8"));
        entry.set_activates_default(true);
        dialog.set_extra_child(Some(&entry));
        let entry_for_focus = entry.clone();
        dialog.add_response("cancel", &tr("Cancel"));
        dialog.add_response("go", &tr("Go"));
        dialog.set_response_appearance("go", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("go"));
        let weak = Rc::downgrade(workbench);
        dialog.connect_response(None, move |_, response| {
            if response != "go" {
                return;
            }
            let Some(workbench) = weak.upgrade() else { return };
            // Reading it is the core's job, so both shells accept the
            // same shapes: a bare number, `412:8` from a compiler, a
            // whole pasted `src/main.rs:412:8`.
            let Some(target) = goto::parse(&entry.text()) else {
                workbench.toast(&tr("Nothing in that names a line."));
                return;
            };
            let Some(page) = workbench.selected() else { return };
            let offset = page
                .state
                .borrow()
                .document
                .offset_for_line(target.line, target.column);
            // The jump is one Go Back should return from: reading was
            // interrupted here.
            workbench.note_jump();
            let text = page
                .buffer
                .text(&page.buffer.start_iter(), &page.buffer.end_iter(), true);
            let mut at = page.buffer.iter_at_offset(page::char_offset(&text, offset));
            page.buffer.place_cursor(&at);
            // Centred rather than merely visible: a line scrolled to
            // the last row of the window is one you have to scroll to
            // read.
            page.view.scroll_to_iter(&mut at, 0.0, true, 0.0, 0.5);
            page.view.grab_focus();
        });
        dialog.present(Some(&workbench.window.clone()));
        // The dialog opens with its default button focused, so without
        // this the number goes nowhere and Return dismisses an empty
        // field.
        entry_for_focus.grab_focus();
    });
    add("blame", workbench, |workbench, _| {
        let Some(page) = workbench.selected() else { return };
        let Some(path) = page.path().borrow().clone() else {
            workbench.toast(&tr("Save the file first — git has nothing to blame yet."));
            return;
        };
        // The buffer's text goes with the line number, so an unsaved
        // edit above the caret cannot shift the answer onto the
        // neighbouring line.
        let text = page.state.borrow().document.text();
        let line = page::lsp_anchor(&page).0 as usize + 1;
        let blame = match blame::blame_line(Path::new(&path), line, &text) {
            Ok(blame) => blame,
            Err(error) => {
                workbench.toast(&fill(&tr("git could not blame this line: {}"), &[&error.to_string()]));
                return;
            }
        };
        show_blame(workbench, &blame, &path);
    });
    add("diagnostic", workbench, |workbench, _| {
        let Some(page) = workbench.selected() else { return };
        let Some(path) = page.path().borrow().clone() else {
            workbench.toast(&tr("No diagnostics for an unsaved document."));
            return;
        };
        let (line, character) = page::lsp_anchor(&page);
        let shell = Shell::instance();
        let found = {
            let pages = shell.pages.borrow();
            pages
                .get(&path)
                .and_then(|handles| {
                    page::diagnostic_at(handles, line as i32, character as usize)
                })
        };
        let Some(found) = found else {
            workbench.toast(&tr("Nothing reported on this line."));
            return;
        };
        // The same words the balloon uses, in a dialog: the caret is
        // not a place a balloon can point at reliably.
        let dialog = adw::AlertDialog::new(Some(found.kind()), Some(&found.message));
        dialog.add_response("close", &tr("Close"));
        dialog.set_default_response(Some("close"));
        dialog.set_close_response("close");
        dialog.present(Some(&workbench.window.clone()));
    });
    add("diagnostic-list", workbench, |workbench, _| {
        let Some(page) = workbench.selected() else { return };
        let Some(path) = page.path().borrow().clone() else {
            workbench.toast(&tr("No diagnostics for an unsaved document."));
            return;
        };
        let shell = Shell::instance();
        let mut found = {
            let pages = shell.pages.borrow();
            match pages.get(&path) {
                Some(handles) => handles.document.diagnostics.borrow().clone(),
                None => Vec::new(),
            }
        };
        if found.is_empty() {
            workbench.toast(&tr("Nothing reported in this document."));
            return;
        }
        // In the order they appear, which is the order they are fixed
        // in and the order the gutter shows them.
        found.sort_by_key(|d| (d.line, d.character));
        let labels: Vec<String> = found
            .iter()
            .map(|d| {
                let first = d.message.lines().next().unwrap_or_default();
                format!("{}  {}  {first}", d.line + 1, d.kind())
            })
            .collect();
        let jumps: Vec<(i32, usize)> =
            found.iter().map(|d| (d.line, d.character)).collect();
        let path = path.clone();
        present_picker(
            workbench,
            &format!("Diagnostics ({})", jumps.len()),
            picker_items(labels),
            move |workbench, index| {
                let (line, character) = jumps[index];
                workbench.note_jump();
                workbench.open(Some(PathBuf::from(&path)), Some((line, character)));
            },
        );
    });
    add("preferences", workbench, |workbench, _| {
        show_preferences(&workbench.window);
    });
    add("import-theme-vscode", workbench, |workbench, _| {
        import_theme(workbench, theme_import::Source::VsCode);
    });
    add("import-theme-textmate", workbench, |workbench, _| {
        import_theme(workbench, theme_import::Source::TextMate);
    });
    add("file-properties", workbench, |workbench, _| show_file_properties(workbench));
    add("about", workbench, |workbench, _| {
        // The real build version comes from git at compile time (the
        // tag in CI); the rest is the who/where/under-what.
        let about = adw::AboutWindow::builder()
            .transient_for(&workbench.window)
            .application_name("Textchum")
            .application_icon("to.perri.textchum")
            .version(env!("TEXTCHUM_BUILD_VERSION"))
            .comments(
                "A text editor in the spirit of TextMate: native, fast, and \
                 focused on editing.",
            )
            .developer_name("Horacio Duran")
            .website("https://perri.to")
            .issue_url("https://github.com/perrito666/textchum/issues")
            .license_type(gtk::License::MitX11)
            .build();
        about.add_link("Repository", "https://github.com/perrito666/textchum");
        about.add_credit_section(
            Some("Contributors"),
            &["Juan Diaz https://github.com/nueces"],
        );
        about.present();
    });
    add("spell-add", workbench, |_workbench, _| {
        let Some((word, _, _)) = crate::spell::menu_target() else { return };
        let shell = Shell::instance();
        if shell.config.borrow_mut().add_spell_word(&word) {
            shell.save_config();
        }
        recheck_every_page();
    });
    add("spell-ignore", workbench, |_workbench, _| {
        let Some((word, _, _)) = crate::spell::menu_target() else { return };
        crate::spell::ignore(&word);
        recheck_every_page();
    });
    add("block-start", workbench, |workbench, _| move_to_block_edge(workbench, true));
    add("block-end", workbench, |workbench, _| move_to_block_edge(workbench, false));
    add("palette", workbench, |workbench, _| show_palette(workbench));
    add("new-format-picker", workbench, |workbench, _| show_format_picker(workbench));
    add("server-status", workbench, |workbench, _| show_server_status(workbench));
    add("paths", workbench, |workbench, _| {
        workbench
            .show_full_paths
            .set(!workbench.show_full_paths.get());
        workbench.refresh_chrome();
        workbench.rebuild_buffer_list();
    });
    // Parameterized actions: the language for a fresh tab, and a
    // recent file's path.
    {
        let action = gtk::gio::SimpleAction::new(
            "new-with-format",
            Some(glib::VariantTy::STRING),
        );
        let weak = Rc::downgrade(workbench);
        action.connect_activate(move |_, parameter| {
            let Some(workbench) = weak.upgrade() else { return };
            let Some(name) = parameter.and_then(|p| p.str().map(str::to_owned)) else {
                return;
            };
            workbench.open(None, None);
            if let Some(page) = workbench.selected() {
                page.state.borrow_mut().document.set_language(Some(&name));
                page::refresh_style_tags(&page.buffer);
                page::recolor(&page.buffer);
                page::apply_highlights(&page.buffer, &page.state.borrow().document);
                workbench.refresh_chrome();
            }
        });
        workbench.window.add_action(&action);
    }
    // Replacing a misspelling: the parameter is the chosen suggestion,
    // and the span it goes into is the one recorded when the menu was
    // Sorts, cases, trims or converts the selection — or the whole
    // document when nothing is selected, since that is what the
    // operation is about when no part of it was singled out.
    {
        let action =
            gtk::gio::SimpleAction::new("transform", Some(glib::VariantTy::STRING));
        let weak = Rc::downgrade(workbench);
        action.connect_activate(move |_, parameter| {
            let Some(workbench) = weak.upgrade() else { return };
            let Some(kind) = parameter.and_then(|p| p.str().map(str::to_owned)) else {
                return;
            };
            let Some(page) = workbench.selected() else { return };
            crate::page::transform_selection(&page, &kind);
        });
        workbench.window.add_action(&action);
    }

    // A context-menu command runs the ordinary action, about the
    // character that was clicked rather than about the caret: a
    // right-click does not move the caret, and a menu that answered
    // about somewhere else would answer the wrong question.
    {
        let action = gtk::gio::SimpleAction::new(
            "context-command",
            Some(glib::VariantTy::new("(si)").unwrap()),
        );
        let weak = Rc::downgrade(workbench);
        action.connect_activate(move |_, parameter| {
            let Some(workbench) = weak.upgrade() else { return };
            let Some((name, offset)) = parameter.and_then(|p| p.get::<(String, i32)>()) else {
                return;
            };
            let Some(page) = workbench.selected() else { return };
            page.context_offset.set(Some(offset));
            // Activation is synchronous, so the offset is in place for
            // exactly as long as the command reads it.
            gtk::prelude::ActionGroupExt::activate_action(&workbench.window, &name, None);
            page.context_offset.set(None);
        });
        workbench.window.add_action(&action);
    }

    // built.
    {
        let action =
            gtk::gio::SimpleAction::new("spell-replace", Some(glib::VariantTy::STRING));
        let weak = Rc::downgrade(workbench);
        action.connect_activate(move |_, parameter| {
            let Some(workbench) = weak.upgrade() else { return };
            let Some(replacement) = parameter.and_then(|p| p.str().map(str::to_owned)) else {
                return;
            };
            let Some((word, start, end)) = crate::spell::menu_target() else { return };
            let Some(page) = workbench.selected() else { return };
            let buffer = &page.buffer;
            let mut from = buffer.iter_at_offset(start);
            let mut to = buffer.iter_at_offset(end);
            // The document may have moved on between the right-click
            // and the choice; replacing whatever now sits at those
            // offsets would corrupt it. Only proceed if the span still
            // holds the word the menu was built for.
            if buffer.text(&from, &to, false) != word {
                workbench.toast(&tr("The text moved — nothing was replaced."));
                return;
            }
            buffer.delete(&mut from, &mut to);
            let mut at = from;
            buffer.insert(&mut at, &replacement);
            crate::spell::run(&page);
        });
        workbench.window.add_action(&action);
    }
    // The copy-path family: each takes the document path as its
    // parameter and puts one shape of it on the clipboard.
    {
        let copies: [(&str, fn(&str) -> Option<String>); 4] = [
            ("copy-name", |path| {
                Path::new(path)
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            }),
            ("copy-relative", |path| {
                let root = workspace::project_root_for(Path::new(path))
                    .map(|root| root.to_string_lossy().into_owned());
                Some(crate::path_actions::relative_path(path, root.as_deref()))
            }),
            ("copy-absolute", |path| Some(path.to_owned())),
            ("copy-forge", |path| crate::path_actions::forge_url(path)),
        ];
        for (name, shape) in copies {
            let action = gtk::gio::SimpleAction::new(name, Some(glib::VariantTy::STRING));
            let weak = Rc::downgrade(workbench);
            action.connect_activate(move |_, parameter| {
                let Some(workbench) = weak.upgrade() else { return };
                let Some(path) = parameter.and_then(|p| p.str().map(str::to_owned)) else {
                    return;
                };
                match shape(&path) {
                    Some(text) => workbench.window.clipboard().set_text(&text),
                    None => workbench.toast(&tr("Not in a git repository with a remote.")),
                }
            });
            workbench.window.add_action(&action);
        }
    }
    {
        let action =
            gtk::gio::SimpleAction::new("move-to-window", Some(glib::VariantTy::STRING));
        let weak = Rc::downgrade(workbench);
        let app = app.clone();
        action.connect_activate(move |_, parameter| {
            let Some(workbench) = weak.upgrade() else { return };
            let Some(spec) = parameter.and_then(|p| p.str().map(str::to_owned)) else {
                return;
            };
            let Some((target, path)) = spec.split_once('|') else { return };
            let target = if target == "new" {
                None
            } else {
                let index: usize = match target.parse() {
                    Ok(index) => index,
                    Err(_) => return,
                };
                let found =
                    WORKBENCHES.with(|all| all.borrow().get(index).cloned());
                match found {
                    Some(found) => Some(found),
                    None => return,
                }
            };
            move_page_to(&workbench, &app, path, target);
        });
        workbench.window.add_action(&action);
    }
    {
        let action =
            gtk::gio::SimpleAction::new("open-recent", Some(glib::VariantTy::STRING));
        let weak = Rc::downgrade(workbench);
        action.connect_activate(move |_, parameter| {
            let Some(workbench) = weak.upgrade() else { return };
            if let Some(path) = parameter.and_then(|p| p.str().map(str::to_owned)) {
                workbench.open(Some(PathBuf::from(path)), None);
            }
        });
        workbench.window.add_action(&action);
    }
    add("redraw", workbench, |workbench, _| {
        let Some(page) = workbench.selected() else { return };
        page::recolor(&page.buffer);
        page::apply_highlights(&page.buffer, &page.state.borrow().document);
    });
    add("hover", workbench, |workbench, _| {
        let Some(page) = workbench.selected() else { return };
        page::hover_at_caret(&page);
    });
    add("preprocess", workbench, |workbench, _| {
        let Some(page) = workbench.selected() else { return };
        if preprocessor_chain(&page).is_none() {
            workbench.toast(&tr("No save preprocessors configured for this language."));
            return;
        }
        match run_preprocessor_chain(&page) {
            Ok(()) => {
                page.view.grab_focus();
            }
            Err(failure) => workbench.toast(&format!(
                "Preprocessor failed: {} — {}",
                failure.command, failure.details
            )),
        }
    });
    add("revert", workbench, |workbench, _| {
        let Some(page) = workbench.selected() else { return };
        if page.path().borrow().is_none() {
            workbench.toast(&tr("Untitled documents have no file to revert to."));
            return;
        }
        if !page.state.borrow().document.is_dirty() {
            page.reload_from_disk();
            return;
        }
        let dialog = adw::AlertDialog::new(
            Some(&tr("Revert to Saved?")),
            Some(&tr("Local changes will be replaced with the file on disk.")),
        );
        dialog.add_response("cancel", &tr("Cancel"));
        dialog.add_response("revert", &tr("Revert"));
        dialog.set_response_appearance("revert", adw::ResponseAppearance::Destructive);
        let page = Rc::clone(&page);
        dialog.connect_response(None, move |_, response| {
            if response == "revert" {
                page.reload_from_disk();
            }
        });
        dialog.present(Some(&workbench.window));
    });
    add("back", workbench, |workbench, _| {
        let target = {
            let current = workbench.current_position();
            workbench.jumps.borrow_mut().back(current)
        };
        match target {
            Some((path, line, character)) => {
                jump_without_trail(workbench, &path, line, character);
            }
            None => workbench.toast(&tr("Nowhere to go back to.")),
        }
    });
    add("forward", workbench, |workbench, _| {
        let target = workbench.jumps.borrow_mut().forward();
        match target {
            Some((path, line, character)) => {
                jump_without_trail(workbench, &path, line, character);
            }
            None => workbench.toast(&tr("Nowhere to go forward to.")),
        }
    });
    add("references", workbench, |workbench, _| {
        let Some((page, path)) = workbench
            .selected()
            .and_then(|page| {
                let path = page.path().borrow().clone();
                path.map(|path| (page, path))
            })
        else {
            workbench.toast(&tr("Save the file first — untitled documents have no server."));
            return;
        };
        let (line, character) = page::lsp_anchor(&page);
        let shell = Shell::instance();
        let id = shell
            .pool
            .borrow_mut()
            .references(Path::new(&path), line, character);
        if id == 0 {
            workbench.toast(&tr("No language server is running for this document."));
            return;
        }
        let weak = Rc::downgrade(workbench);
        shell.expect_response(id, move |json| {
            if let Some(workbench) = weak.upgrade() {
                show_locations(&workbench, "References", json);
            }
        });
    });
    add("code-actions", workbench, |workbench, _| {
        let Some((page, path)) = workbench
            .selected()
            .and_then(|page| {
                let path = page.path().borrow().clone();
                path.map(|path| (page, path))
            })
        else {
            workbench.toast(&tr("Save the file first — untitled documents have no server."));
            return;
        };
        let (line, character) = page::lsp_anchor(&page);
        let shell = Shell::instance();
        let id = shell
            .pool
            .borrow_mut()
            .code_action(Path::new(&path), line, character);
        if id == 0 {
            workbench.toast(&tr("No language server is running for this document."));
            return;
        }
        let weak = Rc::downgrade(workbench);
        shell.expect_response(id, move |json| {
            let Some(workbench) = weak.upgrade() else { return };
            show_code_actions(&workbench, &path, json);
        });
    });
    add("rename", workbench, |workbench, _| {
        let Some((page, path)) = workbench
            .selected()
            .and_then(|page| {
                let path = page.path().borrow().clone();
                path.map(|path| (page, path))
            })
        else {
            workbench.toast(&tr("Save the file first — untitled documents have no server."));
            return;
        };
        let (line, character) = page::lsp_anchor(&page);
        let dialog = adw::AlertDialog::new(Some(&tr("Rename Symbol")), None);
        let entry = gtk::Entry::new();
        entry.set_placeholder_text(Some(&tr("New name")));
        entry.set_activates_default(true);
        dialog.set_extra_child(Some(&entry));
        let entry_for_focus = entry.clone();
        dialog.add_response("cancel", &tr("Cancel"));
        dialog.add_response("rename", &tr("Rename"));
        dialog.set_response_appearance("rename", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("rename"));
        let weak = Rc::downgrade(workbench);
        dialog.connect_response(None, move |_, response| {
            if response != "rename" {
                return;
            }
            let new_name = entry.text().to_string();
            if new_name.trim().is_empty() {
                return;
            }
            let shell = Shell::instance();
            let id = shell
                .pool
                .borrow_mut()
                .rename(Path::new(&path), line, character, new_name.trim());
            let weak = weak.clone();
            if id == 0 {
                if let Some(workbench) = weak.upgrade() {
                    workbench.toast(&tr("No language server is running for this document."));
                }
                return;
            }
            shell.expect_response(id, move |json| {
                if let Some(workbench) = weak.upgrade() {
                    apply_workspace_edit(&workbench, json);
                }
            });
        });
        dialog.present(Some(&workbench.window));
        // Same as Go to Line: the default button takes focus, so the
        // new name has to be clicked into without this.
        entry_for_focus.grab_focus();
        let _ = page;
    });
    add("format", workbench, |workbench, _| {
        let Some(page) = workbench.selected() else { return };
        let path = page.path().borrow().clone();
        // Untitled documents have no server to ask (servers speak in
        // files) — but a configured preprocessor chain formats fine.
        let Some(path) = path else {
            format_via_preprocessors(workbench, &page, FormatFallback::Untitled);
            return;
        };
        // A tab-indented file keeps tabs; everything else gets spaces.
        let text = page.state.borrow().document.text();
        let uses_tabs = text.contains("\n\t") || text.starts_with('\t');
        let tab_width = Shell::instance().config.borrow().tab_width();
        let shell = Shell::instance();
        let id = shell
            .pool
            .borrow_mut()
            .formatting(Path::new(&path), tab_width, !uses_tabs);
        if id == 0 {
            format_via_preprocessors(workbench, &page, FormatFallback::NoServer);
            return;
        }
        let weak = Rc::downgrade(workbench);
        let fallback_page = Rc::clone(&page);
        shell.expect_response(id, move |json| {
            let Some(workbench) = weak.upgrade() else { return };
            let edits = crate::lsp_edits::text_edits(json);
            if edits.is_empty() {
                // The server answered with nothing; the chain gets its
                // chance before an excuse does.
                format_via_preprocessors(
                    &workbench,
                    &fallback_page,
                    FormatFallback::ServerHadNothing,
                );
                return;
            }
            let Some(page) = workbench.page_for(&path) else { return };
            apply_edits_to_page(&page, edits);
            workbench.refresh_chrome();
            page.view.grab_focus();
        });
    });
    add("outline", workbench, |workbench, _| {
        let Some(page) = workbench.selected() else { return };
        let Some(path) = page.path().borrow().clone() else {
            // Markdown outlines itself; anything else needs a server,
            // and a server needs a file.
            if !show_markdown_outline(workbench, &page) {
                workbench.toast(&tr("Save the file first — untitled documents have no server."));
            }
            return;
        };
        let shell = Shell::instance();
        let id = shell.pool.borrow_mut().document_symbols(Path::new(&path));
        if id == 0 {
            if !show_markdown_outline(workbench, &page) {
                workbench.toast(&tr("No language server is running for this document."));
            }
            return;
        }
        let weak = Rc::downgrade(workbench);
        let fallback = Rc::clone(&page);
        shell.expect_response(id, move |json| {
            if let Some(workbench) = weak.upgrade() {
                if !show_outline(&workbench, &path, json)
                    && !show_markdown_outline(&workbench, &fallback)
                {
                    workbench.toast(&tr("The server offered no outline."));
                }
            }
        });
    });
    let _ = window;
}

fn replay(workbench: &Rc<Workbench>, is_undo: bool) {
    let Some(page) = workbench.selected() else { return };
    let edits = {
        let mut state = page.state.borrow_mut();
        state.syncing = true;
        if is_undo {
            state.document.undo()
        } else {
            state.document.redo()
        }
    };
    let buffer = &page.buffer;
    for edit in &edits {
        let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), true);
        let from = buffer.iter_at_offset(page::char_offset(&text, edit.start_utf16));
        let to = buffer.iter_at_offset(page::char_offset(&text, edit.end_utf16));
        let mut from = from;
        let mut to = to;
        buffer.delete(&mut from, &mut to);
        let mut at = from;
        buffer.insert(&mut at, &edit.text);
    }
    page.state.borrow_mut().syncing = false;
    workbench.refresh_chrome();
    page::recolor(buffer);
    page::apply_highlights(buffer, &page.state.borrow().document);
    // The menu stole focus to run this; give it back so the selection
    // and caret show again.
    page.view.grab_focus();
}

/// The caret is on the definition, so the jump key asks the question
/// that is left: who uses this.
///
/// One use is a jump — a panel offering a single row asks for a
/// keystroke that decides nothing. Several open the panel. None says
/// so, since the answer is that nothing refers to the symbol.
fn uses_of_definition(workbench: &Rc<Workbench>, path: &str, line: u32, character: u32) {
    let shell = Shell::instance();
    let id = shell
        .pool
        .borrow_mut()
        .references(Path::new(path), line, character);
    if id == 0 {
        return;
    }
    let weak = Rc::downgrade(workbench);
    let path = path.to_owned();
    shell.expect_response(id, move |json| {
        let Some(workbench) = weak.upgrade() else { return };
        let uses = textchum_core::definition::elsewhere(json, &path, line, character);
        match uses.len() {
            0 => workbench.toast(&tr("Nothing else refers to this symbol.")),
            1 => workbench.open(
                Some(PathBuf::from(uses[0].path.clone())),
                Some((uses[0].line as i32, uses[0].character as usize)),
            ),
            _ => show_places(&workbench, "Uses", places(&uses)),
        }
    });
}

/// Core targets in the shape the picker takes.
fn places(targets: &[textchum_core::definition::Target]) -> Vec<(String, i32, usize)> {
    targets
        .iter()
        .map(|target| {
            (
                target.path.clone(),
                target.line as i32,
                target.character as usize,
            )
        })
        .collect()
}

/// Jumps via the Universal Ctags index, for projects that opted into
/// the fallback. Returns whether a jump happened.
fn ctags_jump(workbench: &Rc<Workbench>, page: &Rc<Page>, path: &str) -> bool {
    let settings = workspace::WorkspaceSettings::from_json(
        &Shell::instance().config.borrow().workspace_json(),
    );
    let Some(root) = workspace::project_root_for(Path::new(path)) else { return false };
    if !settings.flag(&root, "ctags_fallback") {
        return false;
    }
    let Some(symbol) = symbol_under_caret(page) else { return false };
    let Some((target, line)) = crate::ctags::definition(&symbol, &root) else {
        if !crate::ctags::available() {
            workbench.toast(
                "The ctags fallback needs Universal Ctags — install the \
                 universal-ctags package.",
            );
            return true;
        }
        return false;
    };
    workbench.open(Some(target), Some((line, 0)));
    true
}

/// The identifier around the caret (letters, digits, underscore).
fn symbol_under_caret(page: &Rc<Page>) -> Option<String> {
    let buffer = &page.buffer;
    let text = buffer
        .text(&buffer.start_iter(), &buffer.end_iter(), true)
        .to_string();
    let characters: Vec<char> = text.chars().collect();
    let is_word = |index: usize| {
        characters
            .get(index)
            .is_some_and(|c| c.is_alphanumeric() || *c == '_')
    };
    let caret = buffer.iter_at_mark(&buffer.get_insert()).offset().max(0) as usize;
    let mut start = caret.min(characters.len());
    // A caret just past the last character of a word still means it.
    if !is_word(start) && start > 0 && is_word(start - 1) {
        start -= 1;
    }
    if !is_word(start) {
        return None;
    }
    let mut end = start + 1;
    while start > 0 && is_word(start - 1) {
        start -= 1;
    }
    while is_word(end) {
        end += 1;
    }
    Some(characters[start..end].iter().collect())
}

/// The context menu for one document row: the copy-path family, and
/// where the tab could move — a fresh window, or any of the others.
fn copy_menu(current: &Workbench, path: &str) -> gtk::gio::Menu {
    let escaped = path.replace('\\', "\\\\").replace('\'', "\\'");
    let menu = gtk::gio::Menu::new();
    let copies = gtk::gio::Menu::new();
    copies.append(Some("Copy Name"), Some(&format!("win.copy-name('{escaped}')")));
    copies.append(
        Some("Copy Relative Path"),
        Some(&format!("win.copy-relative('{escaped}')")),
    );
    copies.append(
        Some("Copy Absolute Path"),
        Some(&format!("win.copy-absolute('{escaped}')")),
    );
    if crate::path_actions::forge_url(path).is_some() {
        copies.append(
            Some("Copy Forge URL"),
            Some(&format!("win.copy-forge('{escaped}')")),
        );
    }
    menu.append_section(None, &copies);
    let moves = gtk::gio::Menu::new();
    moves.append(
        Some("Move to New Window"),
        Some(&format!("win.move-to-window('new|{escaped}')")),
    );
    WORKBENCHES.with(|all| {
        for (index, other) in all.borrow().iter().enumerate() {
            if std::ptr::eq(other.as_ref() as *const Workbench, current as *const Workbench)
            {
                continue;
            }
            let title = other
                .selected()
                .map(|page| other.disambiguated_name(&page))
                .unwrap_or_else(|| "Empty window".into());
            moves.append(
                Some(&menu_label(&format!("Move to “{title}”"))),
                Some(&format!("win.move-to-window('{index}|{escaped}')")),
            );
        }
    });
    menu.append_section(Some("Move Tab"), &moves);
    menu
}

/// Moves the tab showing `path` into `target` (or a new window),
/// re-homing its shell handles so the pump keeps finding it.
fn move_page_to(
    source: &Rc<Workbench>,
    app: &adw::Application,
    path: &str,
    target: Option<Rc<Workbench>>,
) {
    let Some(page) = source.page_for(path) else { return };
    let target = target.unwrap_or_else(|| {
        let fresh = Workbench::new(app);
        fresh.window.present();
        fresh
    });
    if std::ptr::eq(source.as_ref() as *const Workbench, target.as_ref() as *const Workbench)
    {
        return;
    }
    let Some(source_view) = source.view_holding(&page) else { return };
    let Some(tab_page) = tab_page_of(&source_view, &page) else { return };
    let position = target.tab_view.n_pages();
    source
        .tab_view
        .transfer_page(&tab_page, &target.tab_view, position);
    source.pages.borrow_mut().retain(|other| !Rc::ptr_eq(other, &page));
    target.pages.borrow_mut().push(Rc::clone(&page));
    // The shell's handles carried the old window's chrome; rebuild
    // them around the new one, keeping the subtitle halves.
    let shell = Shell::instance();
    let old = shell.pages.borrow().get(path).cloned();
    if let Some(old) = old {
        let handles = Rc::new(crate::shell::PageHandles {
            window: target.window.clone(),
            tab_view: target.tab_view.clone(),
            tab_page: tab_page.clone(),
            // The same document, shown in another window: its text and
            // its diagnostics travel with it because they were never
            // the view's to begin with.
            document: Rc::clone(&old.document),
            view: page.view.clone(),
            toasts: target.toasts.clone(),
            title: target.title.clone(),
            language: RefCell::new(old.language.borrow().clone()),
            problems: RefCell::new(old.problems.borrow().clone()),
            detail: RefCell::new(old.detail.borrow().clone()),
        });
        shell.pages.borrow_mut().insert(path.to_owned(), handles);
    }
    target.tab_view.set_selected_page(&tab_page);
    target.window.present();
    source.refresh_chrome();
    source.refresh_sidebar();
    target.refresh_chrome();
    target.refresh_sidebar();
    crate::session::save();
}

/// Moves the caret to the innermost multi-line block's start or end,
/// courtesy of the same tree that powers highlighting.
fn move_to_block_edge(workbench: &Rc<Workbench>, to_start: bool) {
    let Some(page) = workbench.selected() else { return };
    let buffer = &page.buffer;
    let insert = buffer.iter_at_mark(&buffer.get_insert());
    let position = page::utf16_offset(buffer, insert.offset());
    let Some((start, end)) = page.state.borrow().document.block_bounds(position) else {
        workbench.toast(&tr("No enclosing block here."));
        return;
    };
    let target_utf16 = if to_start { start } else { end };
    let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), true);
    let target = buffer.iter_at_offset(page::char_offset(&text, target_utf16));
    buffer.place_cursor(&target);
    page.view
        .scroll_to_iter(&mut target.clone(), 0.1, false, 0.0, 0.0);
}

/// Every palette entry: what the menus can do, searchable by name.
const PALETTE: &[(&str, &str)] = &[
    ("New Tab", "win.new-tab"),
    ("New with Format…", "win.new-format-picker"),
    ("New Window", "win.new"),
    ("Open…", "win.open"),
    ("Open Quickly…", "win.quick-open"),
    ("Changed in Branch…", "win.changed-files"),
    ("Save", "win.save"),
    ("Save As…", "win.save-as"),
    ("Revert to Saved", "win.revert"),
    ("Undo", "win.undo"),
    ("Redo", "win.redo"),
    ("Find…", "win.find"),
    ("Find in Project…", "win.find-in-project"),
    ("Run Save Preprocessors", "win.preprocess"),
    ("Redraw", "win.redraw"),
    ("Jump to Definition", "win.definition"),
    ("Go Back", "win.back"),
    ("Go Forward", "win.forward"),
    ("Find References", "win.references"),
    ("Rename Symbol…", "win.rename"),
    ("Format Document", "win.format"),
    ("Document Outline…", "win.outline"),
    ("Show Documentation for Symbol", "win.hover"),
    ("Go to Line…", "win.goto-line"),
    ("Blame Line…", "win.blame"),
    ("Show Diagnostic for Line", "win.diagnostic"),
    ("Diagnostics…", "win.diagnostic-list"),
    ("Go to Block Start", "win.block-start"),
    ("Go to Block End", "win.block-end"),
    ("Language Server Status", "win.server-status"),
    ("Toggle File Tree", "win.sidebar"),
    ("Toggle Markdown Preview", "win.preview"),
    ("Preferences…", "win.preferences"),
    ("Import Theme from VS Code…", "win.import-theme-vscode"),
    ("Import Theme from TextMate…", "win.import-theme-textmate"),
    ("About Textchum", "win.about"),
    ("Close Tab", "win.close-tab"),
];

/// New with Format for keyboards (Ctrl+Shift+N): the language list
/// behind an editable filter; ⏎ opens an untitled tab already
/// speaking the selection.
/// File properties (Ctrl+I): what this document is, when its name does
/// not say. A .txt holding SQL, a dotfile with no extension, an .inc
/// that is really C — the filename decides the language everywhere
/// else, and this is where it can be told otherwise. Indentation too:
/// tab width and tabs-or-spaces are set per project or globally, and
/// one file that disagrees had nowhere to say so.
///
/// Each choice applies and is saved as it is made, so there is no OK
/// button to forget.
fn show_file_properties(workbench: &Rc<Workbench>) {
    let Some(page) = workbench.selected() else { return };
    let Some(path) = page.path().borrow().clone() else {
        // Nothing to remember a choice against; New with Format is
        // where an untitled document's language is set.
        workbench.toast(&tr("Save the file first — an untitled document has no path."));
        return;
    };
    let shell = Shell::instance();
    let stored = shell.config.borrow().file_override(&path);
    let detected = textchum_core::syntax::languages::by_path(Path::new(&path))
        .map(|entry| entry.spec.name);

    let window = adw::Window::builder()
        .transient_for(&workbench.window)
        .modal(false)
        .title(
            Path::new(&path)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.clone()),
        )
        .default_width(420)
        .build();
    let page_box = adw::PreferencesPage::new();
    let group = adw::PreferencesGroup::new();

    let language_row = adw::ComboRow::new();
    language_row.set_title(&tr("Language"));
    let names: Vec<&'static str> =
        textchum_core::syntax::languages::selectable_names().to_vec();
    let automatic = match detected {
        Some(name) => format!("Automatic ({name})"),
        None => "Automatic (plain text)".to_string(),
    };
    let mut entries: Vec<&str> = vec![automatic.as_str()];
    entries.extend(names.iter().copied());
    language_row.set_model(Some(&gtk::StringList::new(&entries)));
    language_row.set_selected(
        stored
            .language
            .as_deref()
            .and_then(|chosen| names.iter().position(|name| *name == chosen))
            .map(|index| index as u32 + 1)
            .unwrap_or(0),
    );
    group.add(&language_row);

    let tab_row = adw::SpinRow::with_range(0.0, 16.0, 1.0);
    tab_row.set_title(&tr("Tab width"));
    tab_row.set_subtitle(&tr("0 follows the project"));
    tab_row.set_value(stored.tab_width.unwrap_or(0) as f64);
    group.add(&tab_row);

    let indent_row = adw::ComboRow::new();
    indent_row.set_title(&tr("Indent with"));
    indent_row.set_model(Some(&gtk::StringList::new(&["Automatic", "Spaces", "Tabs"])));
    indent_row.set_selected(match stored.spaces {
        Some(true) => 1,
        Some(false) => 2,
        None => 0,
    });
    group.add(&indent_row);

    let facts = adw::ActionRow::new();
    let state = page.state.borrow();
    facts.set_title(&format!(
        "{} · {} bytes",
        match state.document.encoding() {
            textchum_core::Encoding::Utf8 => "UTF-8",
            textchum_core::Encoding::Utf8WithBom => "UTF-8 with BOM",
            textchum_core::Encoding::Latin1 => "Latin-1",
        },
        state.document.len_bytes()
    ));
    drop(state);
    facts.set_subtitle(&path);
    facts.add_css_class("dim-label");
    group.add(&facts);
    page_box.add(&group);

    let header = adw::HeaderBar::new();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&header);
    content.append(&page_box);
    window.set_content(Some(&content));

    // One closure behind all three rows: read them together, apply, and
    // save. Reading all three means a change to any one cannot drop
    // what the others say.
    let apply: Rc<dyn Fn()> = {
        let workbench = Rc::downgrade(workbench);
        let page = Rc::downgrade(&page);
        let language_row = language_row.clone();
        let tab_row = tab_row.clone();
        let indent_row = indent_row.clone();
        let names = names.clone();
        let path = path.clone();
        Rc::new(move || {
            let (Some(workbench), Some(page)) = (workbench.upgrade(), page.upgrade())
            else {
                return;
            };
            let selected = language_row.selected() as usize;
            let language = (selected > 0)
                .then(|| names.get(selected - 1).map(|name| (*name).to_owned()))
                .flatten();
            let tab_width = match tab_row.value() as u32 {
                0 => None,
                width => Some(width),
            };
            let spaces = match indent_row.selected() {
                1 => Some(true),
                2 => Some(false),
                _ => None,
            };
            let shell = Shell::instance();
            // What a file is, when its name does not say, is data about
            // the file: it goes with the project record. The rest of
            // the override stays in the configuration.
            *page.document.language_override.borrow_mut() = language.clone();
            page.document.adopted_state.set(true);
            page.document.record_project_state();
            shell.config.borrow_mut().set_file_override(
                &path,
                &textchum_core::FileOverride {
                    language: None,
                    tab_width,
                    spaces,
                },
            );
            shell.save_config();
            apply_file_properties(&workbench, &page, language.as_deref(), tab_width);
        })
    };
    {
        let apply = Rc::clone(&apply);
        language_row.connect_selected_notify(move |_| apply());
    }
    {
        let apply = Rc::clone(&apply);
        tab_row.connect_value_notify(move |_| apply());
    }
    {
        let apply = Rc::clone(&apply);
        indent_row.connect_selected_notify(move |_| apply());
    }
    window.present();
}

/// Puts a document's own settings into effect: the language it is read
/// as, and how wide a tab is drawn. `None` for the language means fall
/// back to what the filename implies.
pub fn apply_file_properties(
    workbench: &Rc<Workbench>,
    page: &Rc<Page>,
    language: Option<&str>,
    tab_width: Option<u32>,
) {
    let resolved = language.map(str::to_owned).or_else(|| {
        page.path().borrow().as_deref().and_then(|path| {
            textchum_core::syntax::languages::by_path(Path::new(path))
                .map(|entry| entry.spec.name.to_owned())
        })
    });
    page.state
        .borrow_mut()
        .document
        .set_language(resolved.as_deref());
    page::refresh_style_tags(&page.buffer);
    page::recolor(&page.buffer);
    page::apply_highlights(&page.buffer, &page.state.borrow().document);
    if let Some(width) = tab_width {
        page.view.set_tab_width(width);
    }
    // The title bar reads a cached language, set when the page was
    // built. Changing what a document is has to update it, or the
    // subtitle keeps naming what the filename implied.
    if let Some(path) = page.path().borrow().clone() {
        if let Some(handles) = Shell::instance().pages.borrow().get(&path) {
            *handles.language.borrow_mut() =
                page.state.borrow().document.language_name().unwrap_or("").to_owned();
            refresh_subtitle(handles);
        }
    }
    workbench.refresh_chrome();
}

fn show_format_picker(workbench: &Rc<Workbench>) {
    let names: Vec<&'static str> =
        textchum_core::syntax::languages::selectable_names().to_vec();
    filterable_picker(workbench, "New with Format", names.clone(), move |workbench, index| {
        let name = names[index];
        let _ = gtk::prelude::WidgetExt::activate_action(
            &workbench.window,
            &format!("win.new-with-format('{name}')"),
            None,
        );
    });
}

/// A modal list with an editable subsequence filter — the palette's
/// skeleton, reusable for any pick-one-by-name flow. ↑/↓ move from the
/// entry, ⏎ picks, ⎋ closes.
fn filterable_picker(
    workbench: &Rc<Workbench>,
    title: &str,
    labels: Vec<&'static str>,
    choose: impl Fn(&Rc<Workbench>, usize) + 'static,
) {
    let entry = gtk::SearchEntry::new();
    entry.set_placeholder_text(Some(&tr("Type to filter…")));
    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::Browse);
    let scrolled = gtk::ScrolledWindow::builder()
        .child(&list)
        .min_content_height(300)
        .build();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 6);
    content.set_margin_top(10);
    content.set_margin_bottom(10);
    content.set_margin_start(10);
    content.set_margin_end(10);
    content.append(&entry);
    content.append(&scrolled);
    let dialog = adw::Window::builder()
        .transient_for(&workbench.window)
        .modal(true)
        .title(title)
        .default_width(380)
        .default_height(360)
        .build();
    dialog.set_content(Some(&content));

    let visible: Rc<RefCell<Vec<usize>>> = Rc::new(RefCell::new(Vec::new()));
    let rebuild = {
        let list = list.clone();
        let visible = Rc::clone(&visible);
        let labels = labels.clone();
        move |query: &str| {
            while let Some(row) = list.row_at_index(0) {
                list.remove(&row);
            }
            let query = query.to_lowercase();
            let mut shown = Vec::new();
            for (index, label) in labels.iter().enumerate() {
                let mut characters = query.chars().peekable();
                for candidate in label.to_lowercase().chars() {
                    if characters.peek() == Some(&candidate) {
                        characters.next();
                    }
                }
                if characters.peek().is_none() {
                    let row_label = gtk::Label::new(Some(label));
                    row_label.set_xalign(0.0);
                    row_label.set_margin_start(10);
                    row_label.set_margin_end(10);
                    row_label.set_margin_top(4);
                    row_label.set_margin_bottom(4);
                    list.append(&row_label);
                    shown.push(index);
                }
            }
            *visible.borrow_mut() = shown;
            if let Some(first) = list.row_at_index(0) {
                list.select_row(Some(&first));
            }
        }
    };
    rebuild("");
    {
        let rebuild = rebuild.clone();
        entry.connect_search_changed(move |entry| rebuild(&entry.text()));
    }
    let run = {
        let workbench = Rc::downgrade(workbench);
        let visible = Rc::clone(&visible);
        let dialog = dialog.clone();
        let choose = Rc::new(choose);
        Rc::new(move |row_index: i32| {
            let Some(workbench) = workbench.upgrade() else { return };
            let Some(&index) = visible.borrow().get(row_index.max(0) as usize) else {
                return;
            };
            dialog.close();
            choose(&workbench, index);
        })
    };
    {
        let list = list.clone();
        let run = Rc::clone(&run);
        entry.connect_activate(move |_| {
            if let Some(row) = list.selected_row() {
                run(row.index());
            }
        });
    }
    {
        let run = Rc::clone(&run);
        list.connect_row_activated(move |_, row| run(row.index()));
    }
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    {
        let dialog = dialog.clone();
        let list = list.clone();
        keys.connect_key_pressed(move |_, key, _, _| match key {
            gtk::gdk::Key::Escape => {
                dialog.close();
                glib::Propagation::Stop
            }
            gtk::gdk::Key::Down | gtk::gdk::Key::Up => {
                let delta = if key == gtk::gdk::Key::Down { 1 } else { -1 };
                let next = list.selected_row().map(|row| row.index() + delta).unwrap_or(0);
                if let Some(row) = list.row_at_index(next.max(0)) {
                    list.select_row(Some(&row));
                }
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        });
    }
    dialog.add_controller(keys);
    dialog.present();
    entry.grab_focus();
}

/// A fuzzy-filterable list over every menu action; ⏎ runs the
/// selection. The shortcut for when the shortcut escapes memory.
fn show_palette(workbench: &Rc<Workbench>) {
    let entry = gtk::SearchEntry::new();
    entry.set_placeholder_text(Some(&tr("Type a command…")));
    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::Browse);
    let scrolled = gtk::ScrolledWindow::builder()
        .child(&list)
        .min_content_height(300)
        .build();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 6);
    content.set_margin_top(10);
    content.set_margin_bottom(10);
    content.set_margin_start(10);
    content.set_margin_end(10);
    content.append(&entry);
    content.append(&scrolled);
    let dialog = adw::Window::builder()
        .transient_for(&workbench.window)
        .modal(true)
        .title("Command Palette")
        .default_width(440)
        .default_height(380)
        .build();
    dialog.set_content(Some(&content));

    // Rows carry their action index; filtering rebuilds the list.
    let visible: Rc<RefCell<Vec<usize>>> = Rc::new(RefCell::new(Vec::new()));
    let rebuild = {
        let list = list.clone();
        let visible = Rc::clone(&visible);
        move |query: &str| {
            while let Some(row) = list.row_at_index(0) {
                list.remove(&row);
            }
            let query = query.to_lowercase();
            let mut shown = Vec::new();
            for (index, (label, _)) in PALETTE.iter().enumerate() {
                // Subsequence match: every query character in order.
                let mut characters = query.chars().peekable();
                for candidate in label.to_lowercase().chars() {
                    if characters.peek() == Some(&candidate) {
                        characters.next();
                    }
                }
                if characters.peek().is_none() {
                    let row_label = gtk::Label::new(Some(label));
                    row_label.set_xalign(0.0);
                    row_label.set_margin_start(10);
                    row_label.set_margin_end(10);
                    row_label.set_margin_top(4);
                    row_label.set_margin_bottom(4);
                    list.append(&row_label);
                    shown.push(index);
                }
            }
            *visible.borrow_mut() = shown;
            if let Some(first) = list.row_at_index(0) {
                list.select_row(Some(&first));
            }
        }
    };
    rebuild("");
    {
        let rebuild = rebuild.clone();
        entry.connect_search_changed(move |entry| rebuild(&entry.text()));
    }
    let run = {
        let workbench = Rc::downgrade(workbench);
        let visible = Rc::clone(&visible);
        let dialog = dialog.clone();
        Rc::new(move |row_index: i32| {
            let Some(workbench) = workbench.upgrade() else { return };
            let Some(&index) = visible.borrow().get(row_index.max(0) as usize) else {
                return;
            };
            dialog.close();
            let _ = gtk::prelude::WidgetExt::activate_action(
                &workbench.window,
                PALETTE[index].1,
                None,
            );
        })
    };
    {
        let list = list.clone();
        let run = Rc::clone(&run);
        entry.connect_activate(move |_| {
            if let Some(row) = list.selected_row() {
                run(row.index());
            }
        });
    }
    {
        let run = Rc::clone(&run);
        list.connect_row_activated(move |_, row| run(row.index()));
    }
    // ↑/↓ move the selection from the entry; Escape closes.
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    {
        let dialog = dialog.clone();
        let list = list.clone();
        keys.connect_key_pressed(move |_, key, _, _| match key {
            gtk::gdk::Key::Escape => {
                dialog.close();
                glib::Propagation::Stop
            }
            gtk::gdk::Key::Down | gtk::gdk::Key::Up => {
                let delta = if key == gtk::gdk::Key::Down { 1 } else { -1 };
                let next = list.selected_row().map(|row| row.index() + delta).unwrap_or(0);
                if let Some(row) = list.row_at_index(next.max(0)) {
                    list.select_row(Some(&row));
                }
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        });
    }
    dialog.add_controller(keys);
    dialog.present();
    entry.grab_focus();
}

/// A plain list of words as a preferences row, one per line. Same
/// shape as the glob editor and for the same reason: a list someone
/// adds to a word at a time over months is unreadable on one line.
/// Commits when the text view loses focus.
fn word_list_row(
    title: &str,
    subtitle: &str,
    words: &[String],
    commit: impl Fn(&[String]) + 'static,
) -> adw::ExpanderRow {
    let row = adw::ExpanderRow::new();
    row.set_title(title);
    row.set_subtitle(&format!(
        "{subtitle} — {} word{}",
        words.len(),
        if words.len() == 1 { "" } else { "s" }
    ));

    let buffer = gtk::TextBuffer::new(None);
    buffer.set_text(&words.join("\n"));
    let view = gtk::TextView::with_buffer(&buffer);
    view.set_monospace(true);
    view.set_top_margin(6);
    view.set_bottom_margin(6);
    view.set_left_margin(8);
    view.set_right_margin(8);
    let scrolled = gtk::ScrolledWindow::builder()
        .child(&view)
        .min_content_height(96)
        .build();
    scrolled.add_css_class("card");

    let focus = gtk::EventControllerFocus::new();
    focus.connect_leave(move |_| {
        let text = buffer
            .text(&buffer.start_iter(), &buffer.end_iter(), false)
            .to_string();
        let words: Vec<String> = text
            .lines()
            .map(str::trim)
            .filter(|word| !word.is_empty())
            .map(str::to_owned)
            .collect();
        commit(&words);
    });
    view.add_controller(focus);

    let holder = gtk::Box::new(gtk::Orientation::Vertical, 6);
    holder.set_margin_top(6);
    holder.set_margin_bottom(6);
    holder.set_margin_start(12);
    holder.set_margin_end(12);
    holder.append(&scrolled);
    row.add_row(&holder);
    row
}

/// A multi-line glob editor as a preferences row: one pattern per
/// line (a space-separated one-liner is unreadable past two entries),
/// with a menu that appends a preset's patterns. Commits when the
/// text view loses focus.
fn glob_editor_row(
    title: &str,
    subtitle: Option<&str>,
    globs: &[String],
    presets: Vec<(String, Vec<String>)>,
    commit: impl Fn(&str) + 'static,
) -> adw::ExpanderRow {
    let row = adw::ExpanderRow::new();
    row.set_title(title);
    let summary = if globs.is_empty() {
        "inherit".to_string()
    } else {
        format!("{} pattern{}", globs.len(), if globs.len() == 1 { "" } else { "s" })
    };
    row.set_subtitle(&match subtitle {
        Some(subtitle) => format!("{subtitle} — {summary}"),
        None => summary,
    });

    let buffer = gtk::TextBuffer::new(None);
    buffer.set_text(&globs.join("\n"));
    let view = gtk::TextView::with_buffer(&buffer);
    view.set_monospace(true);
    view.set_top_margin(6);
    view.set_bottom_margin(6);
    view.set_left_margin(8);
    view.set_right_margin(8);
    let scrolled = gtk::ScrolledWindow::builder()
        .child(&view)
        .min_content_height(96)
        .build();
    scrolled.add_css_class("card");

    let commit = Rc::new(commit);
    {
        // Focus leaving the view is the commit point, like the Mac's.
        let buffer = buffer.clone();
        let commit = Rc::clone(&commit);
        let focus = gtk::EventControllerFocus::new();
        focus.connect_leave(move |_| {
            let text = buffer
                .text(&buffer.start_iter(), &buffer.end_iter(), false)
                .to_string();
            commit(&text.split_whitespace().collect::<Vec<_>>().join(" "));
        });
        view.add_controller(focus);
    }

    let menu = gtk::gio::Menu::new();
    for (name, _) in &presets {
        menu.append(Some(&menu_label(name)), Some(&format!("globs.preset::{name}")));
    }
    let preset_button = gtk::MenuButton::builder()
        .label("Add preset")
        .menu_model(&menu)
        .halign(gtk::Align::Start)
        .build();
    // The action group lives on the button, so several rows can each
    // carry their own without colliding.
    let actions = gtk::gio::SimpleActionGroup::new();
    let action = gtk::gio::SimpleAction::new("preset", Some(gtk::glib::VariantTy::STRING));
    {
        let buffer = buffer.clone();
        let commit = Rc::clone(&commit);
        let presets = presets.clone();
        action.connect_activate(move |_, parameter| {
            let Some(name) = parameter.and_then(|p| p.str().map(str::to_owned)) else {
                return;
            };
            let Some((_, globs)) = presets.iter().find(|(preset, _)| *preset == name) else {
                return;
            };
            let current = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
            let mut lines: Vec<String> =
                current.split_whitespace().map(str::to_owned).collect();
            for glob in globs {
                if !lines.contains(glob) {
                    lines.push(glob.clone());
                }
            }
            buffer.set_text(&lines.join("\n"));
            commit(&lines.join(" "));
        });
    }
    actions.add_action(&action);
    preset_button.insert_action_group("globs", Some(&actions));

    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 6);
    box_.set_margin_top(6);
    box_.set_margin_bottom(6);
    box_.set_margin_start(12);
    box_.set_margin_end(12);
    box_.append(&scrolled);
    box_.append(&preset_button);
    let hint = gtk::Label::new(Some(
        "One pattern per line. Glob patterns: * and ? match; names only, not paths.",
    ));
    hint.set_xalign(0.0);
    hint.add_css_class("dim-label");
    box_.append(&hint);
    row.add_row(&box_);
    row
}

/// A live view of the pool: what runs where, and the recent status
/// transitions — the at-a-glance answer to "is my server alive?".
fn show_server_status(workbench: &Rc<Workbench>) {
    let label = gtk::Label::new(None);
    label.set_xalign(0.0);
    label.set_yalign(0.0);
    label.set_selectable(true);
    label.add_css_class("monospace");
    label.set_margin_top(10);
    label.set_margin_bottom(10);
    label.set_margin_start(12);
    label.set_margin_end(12);
    let scrolled = gtk::ScrolledWindow::builder().child(&label).vexpand(true).build();
    let header = adw::HeaderBar::new();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&header);
    content.append(&scrolled);
    let dialog = adw::Window::builder()
        .transient_for(&workbench.window)
        .title("Language Server Status")
        .default_width(560)
        .default_height(380)
        .content(&content)
        .build();

    let render = {
        let label = label.clone();
        move || {
            let shell = Shell::instance();
            let mut lines: Vec<String> = Vec::new();
            let running = shell.pool.borrow().running();
            lines.push(format!("Running instances ({}):", running.len()));
            if running.is_empty() {
                lines.push("  none — servers start when a matching document opens".into());
            }
            for (server, root) in &running {
                lines.push(format!("  {server}  {root}"));
            }
            lines.push(String::new());
            lines.push("Recent transitions:".into());
            let log = shell.status_log.borrow();
            if log.is_empty() {
                lines.push("  none this session".into());
            }
            for (at, server, root, line) in log.iter().rev().take(30) {
                let clock = gtk::glib::DateTime::from_unix_local(
                    at.duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs() as i64)
                        .unwrap_or(0),
                )
                .map(|t| t.format("%H:%M:%S").map(|s| s.to_string()).unwrap_or_default())
                .unwrap_or_default();
                lines.push(format!("  {clock}  {server} [{root}]: {line}"));
            }
            lines.push(String::new());
            lines.push(format!("Full trail: {}", crate::paths::lsp_log_for_display()));
            label.set_text(&lines.join("\n"));
        }
    };
    render();
    // Refresh while open; the source dies with the dialog.
    let source = glib::timeout_add_local(std::time::Duration::from_secs(2), {
        let dialog = dialog.clone();
        move || {
            if !dialog.is_visible() {
                return glib::ControlFlow::Break;
            }
            render();
            glib::ControlFlow::Continue
        }
    });
    let _ = source;
    dialog.present();
}

/// Saves a page under a new path and finishes what that implies: an
/// untitled document that just gained an extension gets its language
/// (the core detects it), fresh colors, shell handles so diagnostics
/// and the subtitle work, a file monitor, its project's editor
/// overrides, a pool announcement, and a spell pass — everything a
/// pathed open would have had. Returns whether the save landed.
pub fn save_page_as(workbench: &Rc<Workbench>, page: &Rc<Page>, path: &Path) -> bool {
    let previous = page.path().borrow().clone();
    if page.state.borrow_mut().document.save_as(path).is_err() {
        workbench.toast(&tr("Could not save the document."));
        return false;
    }
    let key = path.to_string_lossy().into_owned();
    let shell = Shell::instance();
    shell.note_own_save(&key);
    *page.path().borrow_mut() = Some(key.clone());
    if let Some(gone) = previous.clone().filter(|previous| *previous != key) {
        shell.pages.borrow_mut().remove(&gone);
        shell.pool.borrow_mut().did_close(Path::new(&gone));
    }
    page::refresh_style_tags(&page.buffer);
    page::recolor(&page.buffer);
    page::apply_highlights(&page.buffer, &page.state.borrow().document);
    // The same document under a new name: the index follows the path,
    // and everything the document knows stays with it.
    let document = Rc::clone(&page.document);
    shell.rename_document(document.id, previous.as_deref(), &key);
    let view = workbench
        .view_holding(page)
        .unwrap_or_else(|| workbench.tab_view.clone());
    let Some(tab_page) = tab_page_of(&view, page) else { return false };
    let handles = Rc::new(crate::shell::PageHandles {
        window: workbench.window.clone(),
        tab_view: view.clone(),
        tab_page,
        document,
        view: page.view.clone(),
        toasts: workbench.toasts.clone(),
        title: workbench.title.clone(),
        language: RefCell::new(
            page.state
                .borrow()
                .document
                .language_name()
                .unwrap_or("")
                .to_string(),
        ),
        problems: RefCell::new(String::new()),
        detail: RefCell::new(String::new()),
    });
    shell.pages.borrow_mut().insert(key.clone(), handles);
    // Always re-arm: a page saved under a new path must watch the new
    // file, not the old one (or nothing).
    page::install_file_monitor(page);
    page::apply_project_editor_overrides(page);
    if let Some(language) = page.state.borrow().document.language_name() {
        let text = page.state.borrow().document.text();
        shell.pool.borrow_mut().did_open(path, language, &text);
    }
    crate::spell::run(page);
    workbench.refresh_chrome();
    workbench.refresh_sidebar();
    crate::session::save();
    true
}

/// Formats through the save-preprocessor chain — Format Document's
/// fallback when no server can help — or says exactly what would make
/// formatting possible.
/// Why Format Document reached the preprocessor fallback. It decides
/// what the failure message may claim: telling someone to save a file
/// they just saved is the fastest way to make an error message useless.
#[derive(Clone, Copy)]
enum FormatFallback {
    /// The document has never been saved, so no server can attach to it.
    Untitled,
    /// The document is on disk but no server is running for it.
    NoServer,
    /// A server is running and returned no edits.
    ServerHadNothing,
}

fn format_via_preprocessors(
    workbench: &Rc<Workbench>,
    page: &Rc<Page>,
    why: FormatFallback,
) {
    if preprocessor_chain(page).is_none() {
        let language = page.state.borrow().document.language_name();
        let Some(language) = language else {
            workbench.explain(
                "Textchum does not know what kind of document this is, so it has \
                 nothing to format it with. Save it with a file extension, or start \
                 it from New with Format.",
            );
            return;
        };
        let preprocessor_advice = format!(
            "add a save preprocessor for {language} in Preferences → Preprocessors"
        );
        workbench.explain(&match why {
            FormatFallback::Untitled => format!(
                "Save the file first: a {language} language server formats files on \
                 disk, and this one has never been saved. Or {preprocessor_advice}."
            ),
            FormatFallback::NoServer => {
                match textchum_lsp::registry::server_for_language(language) {
                    Some(spec) => format!(
                        "Nothing here can format {language}: no language server is \
                         running and no save preprocessor is configured. Install \
                         {} ({}), or {preprocessor_advice}.",
                        spec.command, spec.install_hint
                    ),
                    None => format!(
                        "Nothing here can format {language}: Textchum knows no \
                         language server for it. To format it anyway, \
                         {preprocessor_advice}."
                    ),
                }
            }
            FormatFallback::ServerHadNothing => format!(
                "The {language} language server had no changes to make. To run a \
                 formatter of your own as well, {preprocessor_advice}."
            ),
        });
        return;
    }
    match run_preprocessor_chain(page) {
        Ok(()) => {
            workbench.refresh_chrome();
            page.view.grab_focus();
        }
        Err(failure) => workbench.explain(&format!(
            "Preprocessor failed: {} — {}",
            failure.command, failure.details
        )),
    }
}

/// The configured preprocessor chain for a page, if any. Untitled
/// documents qualify too — the chain runs over the buffer, and
/// {filename} gets `Untitled` plus the language's extension, same as
/// the Mac.
fn preprocessor_chain(page: &Rc<Page>) -> Option<(Vec<String>, Option<PathBuf>, String)> {
    let language = page.state.borrow().document.language_name()?;
    let path = page.path().borrow().clone();
    let root = path
        .as_deref()
        .and_then(|path| workspace::project_root_for(Path::new(path)));
    let root_string = root.as_deref().map(|r| r.to_string_lossy().into_owned());
    let commands = Shell::instance()
        .config
        .borrow()
        .preprocessor_commands(root_string.as_deref(), language);
    if commands.is_empty() {
        return None;
    }
    let document_path = path.unwrap_or_else(|| {
        let extension = textchum_core::syntax::languages::by_name(language)
            .and_then(|entry| entry.spec.extensions.first().copied())
            .unwrap_or("txt");
        format!("Untitled.{extension}")
    });
    Some((commands, root, document_path))
}

/// Runs the chain over the buffer and applies the result as one
/// minimal edit through the choke point (so the core follows and undo
/// works). `Ok(())` also covers "no chain configured".
fn run_preprocessor_chain(page: &Rc<Page>) -> Result<(), crate::preprocessors::Failure> {
    let Some((commands, root, path)) = preprocessor_chain(page) else {
        return Ok(());
    };
    let text = page.state.borrow().document.text();
    let output =
        crate::preprocessors::run(&commands, &text, root.as_deref(), Some(&path))?;
    page::apply_whole_document(page, &output);
    Ok(())
}

/// Runs the chain, then `proceed` — immediately when clean, after the
/// user chooses "Save Without Preprocessing" when a link failed.
fn preprocess_gate(
    workbench: &Rc<Workbench>,
    page: &Rc<Page>,
    proceed: impl FnOnce(&Rc<Workbench>, &Rc<Page>) + 'static,
) {
    match run_preprocessor_chain(page) {
        Ok(()) => proceed(workbench, page),
        Err(failure) => {
            let dialog = adw::AlertDialog::new(
                Some(&format!("Save preprocessor failed: {}", failure.command)),
                Some(&failure.details),
            );
            dialog.add_response("cancel", &tr("Cancel"));
            dialog.add_response("save", &tr("Save Without Preprocessing"));
            let parent = workbench.window.clone();
            let workbench = Rc::clone(workbench);
            let page = Rc::clone(page);
            // connect_response takes Fn; the once-only continuation
            // rides in a RefCell.
            let proceed = RefCell::new(Some(proceed));
            dialog.connect_response(None, move |_, response| {
                if response == "save" {
                    if let Some(proceed) = proceed.borrow_mut().take() {
                        proceed(&workbench, &page);
                    }
                }
            });
            dialog.present(Some(&parent));
        }
    }
}

/// Navigates for Go Back/Forward: same as a jump, minus the trail.
fn jump_without_trail(workbench: &Rc<Workbench>, path: &str, line: i32, character: usize) {
    workbench.retracing.set(true);
    workbench.open(Some(PathBuf::from(path)), Some((line, character)));
    workbench.retracing.set(false);
}

/// A floating list of locations (references); activating a row jumps.
fn show_locations(workbench: &Rc<Workbench>, title: &str, json: &str) {
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(json) else {
        workbench.toast(&tr("No references found."));
        return;
    };
    let mut rows: Vec<(String, i32, usize)> = Vec::new();
    for item in parsed.as_array().into_iter().flatten() {
        let Some(uri) = item["uri"].as_str() else { continue };
        let Some(path) = uri.strip_prefix("file://") else { continue };
        let line = item["range"]["start"]["line"].as_i64().unwrap_or(0) as i32;
        let character = item["range"]["start"]["character"].as_u64().unwrap_or(0) as usize;
        rows.push((percent_decode(path), line, character));
    }
    if rows.is_empty() {
        workbench.toast(&tr("No references found."));
        return;
    }
    show_places(workbench, title, rows);
}

/// What the server offers for the place the caret is: the quick fix
/// for the diagnostic there, or the refactorings it has for the range.
fn show_code_actions(workbench: &Rc<Workbench>, path: &str, json: &str) {
    use textchum_core::code_action;
    let actions = code_action::actions(json);
    if actions.is_empty() {
        workbench.toast(&tr("The language server has no action for this place."));
        return;
    }
    let labels: Vec<String> = actions
        .iter()
        .map(|action| {
            if action.preferred {
                format!("{}  ·  suggested", action.title)
            } else {
                action.title.clone()
            }
        })
        .collect();
    let path = path.to_owned();
    let json = json.to_owned();
    present_picker(
        workbench,
        "Code Actions",
        picker_items(labels),
        move |workbench, index| {
            let Some(action) = code_action::actions(&json).into_iter().nth(index) else {
                return;
            };
            run_code_action(workbench, &path, action.outcome());
        },
    );
}

/// Carries out a chosen action: apply the edit, run the command, or ask
/// the server to fill the edit in first — servers are allowed to answer
/// cheaply and compute only the one that was chosen.
fn run_code_action(
    workbench: &Rc<Workbench>,
    path: &str,
    outcome: textchum_core::code_action::Outcome,
) {
    use textchum_core::code_action::Outcome;
    let shell = Shell::instance();
    match outcome {
        Outcome::Edit(edit) => apply_workspace_edit(workbench, &edit.to_string()),
        Outcome::Command { name, arguments } => {
            // The command's own answer is the server saying it is done;
            // the edits it makes arrive as a workspace/applyEdit
            // request of its own.
            let id = shell
                .pool
                .borrow_mut()
                .execute_command(Path::new(path), &name, arguments);
            if id == 0 {
                workbench.toast(&tr("The language server would not run that action."));
            }
        }
        Outcome::Resolve(action) => {
            let id = shell
                .pool
                .borrow_mut()
                .resolve_code_action(Path::new(path), action);
            if id == 0 {
                workbench.toast(&tr("The language server would not finish that action."));
                return;
            }
            let weak = Rc::downgrade(workbench);
            let path = path.to_owned();
            shell.expect_response(id, move |json| {
                let Some(workbench) = weak.upgrade() else { return };
                // The resolved action carries the edit; anything else
                // means the server had nothing after all.
                let resolved = textchum_core::code_action::actions(&format!("[{json}]"));
                match resolved.first().map(|action| action.outcome()) {
                    Some(Outcome::Edit(edit)) => {
                        apply_workspace_edit(&workbench, &edit.to_string())
                    }
                    Some(Outcome::Command { name, arguments }) => {
                        Shell::instance().pool.borrow_mut().execute_command(
                            Path::new(&path),
                            &name,
                            arguments,
                        );
                    }
                    _ => workbench.toast(&tr("The language server had no edit for that action.")),
                }
            });
        }
    }
}

/// The same panel over places already parsed.
fn show_places(workbench: &Rc<Workbench>, title: &str, rows: Vec<(String, i32, usize)>) {
    // Code first, tests after. Ask where a function is used and the
    // answer is usually dominated by its test file; what calls this is
    // the question, and what checks it is the follow-up. All of one or
    // the other gets no headings — a heading over every row there is
    // tells the reader nothing.
    let describe = |(path, line, _): &(String, i32, usize)| {
        let name = Path::new(path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.clone());
        format!("{name}:{}", line + 1)
    };
    let (tests, code): (Vec<_>, Vec<_>) = rows
        .iter()
        .cloned()
        .partition(|(path, _, _)| references::is_test_path(path));
    let (picker_rows, rows) = if code.is_empty() || tests.is_empty() {
        (picker_items(rows.iter().map(describe).collect()), rows)
    } else {
        let mut labels: Vec<PickerRow> = Vec::with_capacity(rows.len());
        for (at, reference) in code.iter().enumerate() {
            labels.push(PickerRow {
                label: describe(reference),
                heading: (at == 0).then(|| format!("Code ({})", code.len())),
            });
        }
        for (at, reference) in tests.iter().enumerate() {
            labels.push(PickerRow {
                label: describe(reference),
                heading: (at == 0).then(|| format!("Tests ({})", tests.len())),
            });
        }
        // The order the labels are in is the order the rows must be in.
        (labels, code.into_iter().chain(tests).collect())
    };
    present_picker(workbench, title, picker_rows, move |workbench, index| {
        let (path, line, character) = rows[index].clone();
        workbench.open(Some(PathBuf::from(path)), Some((line, character)));
    });
}

/// The document's symbols, flattened; activating a row jumps.
fn show_outline(workbench: &Rc<Workbench>, path: &str, json: &str) -> bool {
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(json) else {
        return false;
    };
    let mut rows: Vec<(String, i32, usize)> = Vec::new();
    fn walk(rows: &mut Vec<(String, i32, usize)>, items: &serde_json::Value, depth: usize) {
        for item in items.as_array().into_iter().flatten() {
            let Some(name) = item["name"].as_str() else { continue };
            // DocumentSymbol has selectionRange; SymbolInformation has
            // location.range.
            let range = if item["selectionRange"].is_object() {
                &item["selectionRange"]
            } else {
                &item["location"]["range"]
            };
            let line = range["start"]["line"].as_i64().unwrap_or(0) as i32;
            let character = range["start"]["character"].as_u64().unwrap_or(0) as usize;
            rows.push((format!("{}{name}", "  ".repeat(depth)), line, character));
            walk(rows, &item["children"], depth + 1);
        }
    }
    walk(&mut rows, &parsed, 0);
    if rows.is_empty() {
        return false;
    }
    let path = path.to_owned();
    let labels: Vec<String> = rows.iter().map(|(label, _, _)| label.clone()).collect();
    present_picker(workbench, "Document Outline", picker_items(labels), move |workbench, index| {
        let (_, line, character) = rows[index];
        workbench.open(Some(PathBuf::from(&path)), Some((line, character)));
    });
    true
}

/// A Markdown document outlines itself: its headings, nested by depth.
/// No language server required, which is what a blog post needs.
fn show_markdown_outline(workbench: &Rc<Workbench>, page: &Rc<Page>) -> bool {
    if page.state.borrow().document.language_name() != Some("markdown") {
        return false;
    }
    let text = page.state.borrow().document.text();
    let headings = textchum_core::hugo::headings(&text);
    if headings.is_empty() {
        return false;
    }
    let rows: Vec<(i32, usize)> = headings
        .iter()
        .map(|heading| (heading.line as i32, heading.character))
        .collect();
    let labels: Vec<String> = headings
        .iter()
        .map(|heading| format!("{}{}", "  ".repeat(heading.level - 1), heading.text))
        .collect();
    let path = page.path().borrow().clone();
    present_picker(workbench, "Document Outline", picker_items(labels), move |workbench, index| {
        let (line, character) = rows[index];
        match &path {
            Some(path) => {
                workbench.open(Some(PathBuf::from(path)), Some((line, character)))
            }
            // An unsaved post still navigates, in place.
            None => {
                if let Some(page) = workbench.selected() {
                    if let Some(iter) =
                        crate::page::iter_at_lsp(&page.buffer, line, character)
                    {
                        page.buffer.place_cursor(&iter);
                        page.view
                            .scroll_to_iter(&mut iter.clone(), 0.1, false, 0.0, 0.0);
                        page.view.grab_focus();
                    }
                }
            }
        }
    });
    true
}

/// A small modal list; activating a row runs `choose` and closes.
/// One row of a picker: what to show, and the heading that starts a
/// section above it, if it starts one.
struct PickerRow {
    label: String,
    heading: Option<String>,
}

/// Every label as a row, for pickers with nothing to divide.
fn picker_items(labels: Vec<String>) -> Vec<PickerRow> {
    labels
        .into_iter()
        .map(|label| PickerRow {
            label,
            heading: None,
        })
        .collect()
}

fn present_picker(
    workbench: &Rc<Workbench>,
    title: &str,
    rows: Vec<PickerRow>,
    choose: impl Fn(&Rc<Workbench>, usize) + 'static,
) {
    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::Browse);
    for row in &rows {
        let row_label = gtk::Label::new(Some(&row.label));
        row_label.set_xalign(0.0);
        row_label.set_margin_start(10);
        row_label.set_margin_end(10);
        row_label.set_margin_top(4);
        row_label.set_margin_bottom(4);
        list.append(&row_label);
    }
    // Headings go on as GtkListBox headers rather than as rows: a
    // header is not a row, so ↑/↓ never land on one and the row index
    // stays the item index.
    let headings: Vec<Option<String>> = rows.iter().map(|row| row.heading.clone()).collect();
    list.set_header_func(move |row, _| {
        let Some(heading) = headings.get(row.index().max(0) as usize).cloned().flatten() else {
            row.set_header(gtk::Widget::NONE);
            return;
        };
        let label = gtk::Label::new(Some(&heading));
        label.set_xalign(0.0);
        label.set_margin_start(10);
        label.set_margin_top(8);
        label.set_margin_bottom(2);
        label.add_css_class("dim-label");
        label.add_css_class("heading");
        row.set_header(Some(&label));
    });
    let scrolled = gtk::ScrolledWindow::builder()
        .child(&list)
        .min_content_height(280)
        .min_content_width(420)
        .build();
    let dialog = adw::Window::builder()
        .transient_for(&workbench.window)
        .modal(true)
        .title(title)
        .default_width(460)
        .default_height(340)
        .build();
    let header = adw::HeaderBar::new();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&header);
    content.append(&scrolled);
    dialog.set_content(Some(&content));
    {
        let workbench = Rc::clone(workbench);
        let dialog = dialog.clone();
        list.connect_row_activated(move |_, row| {
            let index = row.index();
            if index >= 0 {
                choose(&workbench, index as usize);
            }
            dialog.close();
        });
    }
    // Escape closes, like every dialog should.
    let keys = gtk::EventControllerKey::new();
    {
        let dialog = dialog.clone();
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                dialog.close();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
    }
    dialog.add_controller(keys);
    dialog.present();
    if let Some(first) = list.row_at_index(0) {
        list.select_row(Some(&first));
    }
}

/// Applies LSP text edits to an open page through the buffer (the
/// choke point carries them into the core, so undo works), bottom-up.
fn apply_edits_to_page(page: &Rc<Page>, edits: Vec<crate::lsp_edits::TextEdit>) {
    let buffer = &page.buffer;
    for edit in crate::lsp_edits::bottom_up(edits) {
        let Some(start) = page::iter_at_lsp(buffer, edit.start_line, edit.start_character)
        else {
            continue;
        };
        let Some(end) = page::iter_at_lsp(buffer, edit.end_line, edit.end_character) else {
            continue;
        };
        let mut start = start;
        let mut end = end;
        buffer.delete(&mut start, &mut end);
        let mut at = start;
        buffer.insert(&mut at, &edit.new_text);
    }
}

/// Applies a rename's WorkspaceEdit: open pages edit in place, files
/// nobody has open are rewritten on disk.
fn apply_workspace_edit(workbench: &Rc<Workbench>, json: &str) {
    let by_file = crate::lsp_edits::workspace_edits(json);
    if by_file.is_empty() {
        workbench.toast(&tr("The server had no rename to offer."));
        return;
    }
    let mut touched = 0usize;
    for (path, edits) in by_file {
        let mut open_page: Option<Rc<Page>> = None;
        Workbench::for_each(|candidate| {
            if open_page.is_none() {
                open_page = candidate.page_for(&path);
            }
        });
        if let Some(page) = open_page {
            apply_edits_to_page(&page, edits);
            touched += 1;
        } else if let Ok(text) = std::fs::read_to_string(&path) {
            let rewritten = crate::lsp_edits::apply_to_string(&text, edits);
            if std::fs::write(&path, rewritten).is_ok() {
                touched += 1;
            }
        }
    }
    workbench.toast(&format!(
        "Renamed across {touched} file{}.",
        if touched == 1 { "" } else { "s" }
    ));
    workbench.refresh_chrome();
}

pub fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let Ok(byte) =
                u8::from_str_radix(std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or(""), 16)
            {
                decoded.push(byte);
                index += 3;
                continue;
            }
        }
        decoded.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

// MARK: Open Quickly

/// A modal fuzzy file finder over the core's matcher: type, ⏎ opens the
/// selection (or the first hit) as a tab, ⎋ closes.
fn show_quick_open(workbench: &Rc<Workbench>, root: PathBuf) {
    let index = textchum_core::search::list_files(&root);
    show_file_picker(workbench, root, index);
}

/// The files the branch touches — the pull request's files, read from
/// git alone — behind the same fuzzy filter as Open Quickly.
fn show_changed_files(workbench: &Rc<Workbench>, root: PathBuf) {
    let branches = merge_base_branches_for(&root);
    let Some((top, files)) = textchum_core::changes::branch_files(&root, &branches) else {
        workbench.toast(&tr("Not inside a git repository."));
        return;
    };
    let index: Vec<String> = files.into_iter().map(|(_, path)| path).collect();
    if index.is_empty() {
        workbench.toast(&tr("The branch touches no files."));
        return;
    }
    show_file_picker(workbench, PathBuf::from(top), index);
}

/// What the gutter and the changed-files list resolve the fork point
/// with: the project's override when it has one, the configuration
/// otherwise.
pub fn merge_base_branches_for(root: &Path) -> Vec<String> {
    let shell = Shell::instance();
    let config = shell.config.borrow();
    let overrides = config.editor_overrides_json(&root.to_string_lossy());
    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&overrides) {
        if let Some(names) = parsed["merge_base_branches"].as_array() {
            let names: Vec<String> = names
                .iter()
                .filter_map(|name| name.as_str().map(str::to_string))
                .collect();
            if !names.is_empty() {
                return names;
            }
        }
    }
    config.merge_base_branches()
}

fn show_file_picker(workbench: &Rc<Workbench>, root: PathBuf, index: Vec<String>) {
    let entry = gtk::SearchEntry::new();
    entry.set_placeholder_text(Some(&tr("fuzzy file name…")));
    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::Browse);
    let scrolled = gtk::ScrolledWindow::builder()
        .child(&list)
        .vexpand(true)
        .build();
    let status = gtk::Label::new(None);
    status.set_xalign(0.0);
    status.add_css_class("dim-label");
    status.set_margin_start(4);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 6);
    content.set_margin_top(10);
    content.set_margin_bottom(10);
    content.set_margin_start(10);
    content.set_margin_end(10);
    content.append(&entry);
    content.append(&scrolled);
    content.append(&status);

    let dialog = adw::Window::builder()
        .transient_for(&workbench.window)
        .modal(true)
        .default_width(560)
        .default_height(380)
        .title(&*root.to_string_lossy())
        .content(&content)
        .build();

    // Walk once, match many: re-walking a real repository on every
    // keystroke is what makes a fuzzy finder feel broken.
    let index: Rc<Vec<String>> = Rc::new(index);
    let refill = {
        let list = list.clone();
        let index = Rc::clone(&index);
        let status = status.clone();
        move |query: &str| {
            while let Some(child) = list.first_child() {
                list.remove(&child);
            }
            let matches = textchum_core::search::match_files(&index, query, 50);
            for relative in &matches {
                let label = gtk::Label::new(Some(relative));
                label.set_xalign(0.0);
                label.set_margin_start(6);
                label.set_margin_top(3);
                label.set_margin_bottom(3);
                list.append(&label);
            }
            status.set_text(&format!(
                "{} of {} files   ·   ↑↓ select · ⏎ search · Ctrl+⏎ open · Esc close",
                matches.len(),
                index.len()
            ));
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
        let workbench = Rc::clone(workbench);
        let dialog = dialog.clone();
        let root = root.clone();
        move |row: &gtk::ListBoxRow| {
            if let Some(label) = row.child().and_downcast::<gtk::Label>() {
                let full = root.join(label.text().as_str());
                dialog.close();
                workbench.open(Some(full), None);
            }
        }
    };
    {
        let open_row = open_row.clone();
        list.connect_row_activated(move |_, row| open_row(row));
    }
    {
        // ⏎ re-runs the search; Ctrl+⏎ is what opens, so a keystroke
        // meant to refine the query never opens the wrong file.
        let refill = refill.clone();
        let entry_for_activate = entry.clone();
        entry.connect_activate(move |_| refill(&entry_for_activate.text()));
    }
    {
        let list = list.clone();
        let open_row = open_row.clone();
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed(move |_, key, _, state| {
            let is_return = key == gtk::gdk::Key::Return || key == gtk::gdk::Key::KP_Enter;
            if is_return && state.contains(gtk::gdk::ModifierType::CONTROL_MASK) {
                if let Some(row) = list.selected_row().or_else(|| list.row_at_index(0)) {
                    open_row(&row);
                }
                return glib::Propagation::Stop;
            }
            if key == gtk::gdk::Key::Down || key == gtk::gdk::Key::Up {
                let delta = if key == gtk::gdk::Key::Down { 1 } else { -1 };
                let next = list.selected_row().map(|row| row.index() + delta).unwrap_or(0);
                if let Some(row) = list.row_at_index(next.max(0)) {
                    list.select_row(Some(&row));
                }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        dialog.add_controller(keys);
    }
    wire_escape(&dialog, &entry);
    dialog.present();
    entry.grab_focus();
}

/// ⎋ must close a search dialog from anywhere — including the entry,
/// which consumes Escape as its own stop-search signal.
fn wire_escape(dialog: &adw::Window, entry: &gtk::SearchEntry) {
    {
        let dialog = dialog.clone();
        entry.connect_stop_search(move |_| dialog.close());
    }
    let escape = gtk::EventControllerKey::new();
    escape.set_propagation_phase(gtk::PropagationPhase::Capture);
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
}

// MARK: Find in Project

/// Project-wide content search over the core's grep: the query is a
/// regex with ripgrep's smart-case rule, results are `path:line: text`,
/// ⏎ jumps, and a status line says what the search did — matches and
/// files searched, or the reason nothing was (bad pattern quoted).
fn show_grep(workbench: &Rc<Workbench>, root: PathBuf) {
    let entry = gtk::SearchEntry::new();
    entry.set_placeholder_text(Some(&tr("regular expression…")));
    let filters_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let add_button = gtk::Button::with_label("＋ Add Filter");
    add_button.set_halign(gtk::Align::Start);
    add_button.add_css_class("flat");
    let add_row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    add_row.append(&add_button);
    let status = gtk::Label::new(None);
    status.set_xalign(0.0);
    status.add_css_class("dim-label");
    status.set_margin_start(4);
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
    content.append(&filters_box);
    content.append(&add_row);
    content.append(&scrolled);
    content.append(&status);

    let dialog = adw::Window::builder()
        .transient_for(&workbench.window)
        .modal(true)
        .default_width(680)
        .default_height(440)
        .title(&*root.to_string_lossy())
        .content(&content)
        .build();

    // (relative path, one-based line) per row, aligned with the list.
    let hits: Rc<RefCell<Vec<(String, usize)>>> = Rc::new(RefCell::new(Vec::new()));
    let run = {
        let list = list.clone();
        let status = status.clone();
        let root = root.clone();
        let hits = Rc::clone(&hits);
        let filters_box = filters_box.clone();
        move |query: &str| {
            while let Some(child) = list.first_child() {
                list.remove(&child);
            }
            hits.borrow_mut().clear();
            if query.is_empty() {
                status.set_text("Type to search.");
                return;
            }
            // Smart case: lowercase queries match any case.
            let smart_case = query == query.to_lowercase();
            match textchum_core::search::grep_with_stats(
                &root, query, smart_case, 200, &grep_filters(&filters_box),
            ) {
                Ok((found, stats)) => {
                    for hit in &found {
                        let label = gtk::Label::new(Some(&format!(
                            "{}:{}: {}",
                            hit.path, hit.line, hit.text
                        )));
                        label.set_xalign(0.0);
                        label.set_margin_start(6);
                        label.set_margin_top(2);
                        label.set_margin_bottom(2);
                        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
                        list.append(&label);
                        hits.borrow_mut().push((hit.path.clone(), hit.line as usize));
                    }
                    if let Some(first) = list.row_at_index(0) {
                        list.select_row(Some(&first));
                    }
                    status.set_text(&if found.is_empty() {
                        if stats.files_searched == 0 {
                            "No files to search here — everything is ignored or \
                             the scope is empty."
                                .to_string()
                        } else {
                            format!("No matches in {} files searched.", stats.files_searched)
                        }
                    } else {
                        format!(
                            "{} matches · {} files searched",
                            found.len(),
                            stats.files_searched
                        )
                    });
                }
                Err(error) => status.set_text(&error),
            }
        }
    };
    run("");
    // One debounced rerun, shared by the query and every filter row.
    let pending: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));
    let rerun: Rc<dyn Fn()> = {
        let run = run.clone();
        let entry = entry.clone();
        let pending = Rc::clone(&pending);
        Rc::new(move || {
            if let Some(previous) = pending.borrow_mut().take() {
                previous.remove();
            }
            let run = run.clone();
            let text = entry.text().to_string();
            let pending_inner = Rc::clone(&pending);
            let source = glib::timeout_add_local_once(
                std::time::Duration::from_millis(200),
                move || {
                    *pending_inner.borrow_mut() = None;
                    run(&text);
                },
            );
            *pending.borrow_mut() = Some(source);
        })
    };
    {
        let rerun = Rc::clone(&rerun);
        entry.connect_search_changed(move |_| rerun());
    }
    {
        let filters_box = filters_box.clone();
        let rerun = Rc::clone(&rerun);
        add_button.connect_clicked(move |_| add_filter_row(&filters_box, &rerun));
    }

    let open_row = {
        let workbench = Rc::clone(workbench);
        let dialog = dialog.clone();
        let root = root.clone();
        let hits = Rc::clone(&hits);
        move |row: &gtk::ListBoxRow| {
            let hit = hits.borrow().get(row.index() as usize).cloned();
            if let Some((relative, line)) = hit {
                dialog.close();
                workbench.open(
                    Some(root.join(&relative)),
                    Some((line.saturating_sub(1) as i32, 0)),
                );
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
    wire_escape(&dialog, &entry);
    dialog.present();
    entry.grab_focus();
}

/// One stacked refinement: kind dropdown + pattern + remove — the
/// macOS panel's filters, GTK edition. Case-insensitive substrings,
/// combined with *and*.
fn add_filter_row(filters_box: &gtk::Box, rerun: &Rc<dyn Fn()>) {
    let kinds = gtk::StringList::new(&[
        "line contains",
        "line excludes",
        "file contains",
        "file excludes",
    ]);
    let kind = gtk::DropDown::new(Some(kinds), gtk::Expression::NONE);
    let pattern = gtk::Entry::new();
    pattern.set_placeholder_text(Some(&tr("filter text…")));
    pattern.set_hexpand(true);
    let remove = gtk::Button::from_icon_name("list-remove-symbolic");
    remove.add_css_class("flat");
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.append(&kind);
    row.append(&pattern);
    row.append(&remove);
    {
        let rerun = Rc::clone(rerun);
        kind.connect_selected_notify(move |_| rerun());
    }
    {
        let rerun = Rc::clone(rerun);
        pattern.connect_changed(move |_| rerun());
    }
    {
        let filters_box = filters_box.clone();
        let rerun = Rc::clone(rerun);
        let this_row = row.clone();
        remove.connect_clicked(move |_| {
            filters_box.remove(&this_row);
            rerun();
        });
    }
    filters_box.append(&row);
    pattern.grab_focus();
}

/// The current filter rows as core filters (empty patterns skipped).
fn grep_filters(filters_box: &gtk::Box) -> Vec<textchum_core::search::Filter> {
    use textchum_core::search::{Filter, FilterKind};
    let mut filters = Vec::new();
    let mut child = filters_box.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        let Ok(row) = widget.downcast::<gtk::Box>() else { continue };
        let Some(kind) = row.first_child().and_downcast::<gtk::DropDown>() else {
            continue;
        };
        let Some(pattern) = kind.next_sibling().and_downcast::<gtk::Entry>() else {
            continue;
        };
        let text = pattern.text().to_string();
        if text.is_empty() {
            continue;
        }
        let (kind, include) = match kind.selected() {
            0 => (FilterKind::Line, true),
            1 => (FilterKind::Line, false),
            2 => (FilterKind::File, true),
            _ => (FilterKind::File, false),
        };
        filters.push(Filter {
            kind,
            include,
            pattern: text,
        });
    }
    filters
}

/// What git said about a line, in a dialog with the commit ready to
/// copy — pasting the hash into `git show` or a message is most of what
/// the answer is for.
fn show_blame(workbench: &Rc<Workbench>, blame: &blame::Blame, path: &str) {
    let name = Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_owned());

    let detail = if blame.uncommitted {
        "This line is not committed yet. It was typed since the last commit, so git \
         has nobody to name."
            .to_owned()
    } else {
        let mut rows = Vec::new();
        let mut row = |label: &str, value: &str| {
            if !value.is_empty() {
                rows.push(format!("{label}\n{value}"));
            }
        };
        row("Commit", &blame.abbreviated);
        row(
            "Author",
            &if blame.author_mail.is_empty() {
                blame.author.clone()
            } else {
                format!("{} <{}>", blame.author, blame.author_mail)
            },
        );
        row("Written", &blame.author_date);
        row("Committed by", &blame.committer);
        row("Committed", &blame.committer_date);
        row("Named at the time", &blame.renamed_from);
        row("Subject", &blame.summary);
        row("Message", &blame.body);
        rows.join("\n\n")
    };

    let dialog = adw::AlertDialog::new(Some(&format!("{name}:{}", blame.line)), Some(&detail));
    dialog.add_response("close", &tr("Close"));
    if !blame.commit.is_empty() {
        dialog.add_response("copy", &tr("Copy Commit"));
        dialog.set_response_appearance("copy", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("copy"));
        let commit = blame.commit.clone();
        let window = workbench.window.clone();
        dialog.connect_response(None, move |_, response| {
            if response == "copy" {
                window.clipboard().set_text(&commit);
            }
        });
    } else {
        dialog.set_default_response(Some("close"));
    }
    dialog.set_close_response("close");
    dialog.present(Some(&workbench.window.clone()));
}

// MARK: Import a theme

/// Textchum ▸ Import Theme: bringing colours over from the editor
/// someone used before this one.
///
/// The chooser takes a folder as readily as a file, because a VS Code
/// theme lives inside an extension directory that may contribute
/// several, and a TextMate bundle keeps its own in `Themes/`. Someone
/// who installed a pack of six wanted the pack, so all of them come in
/// and the first is put on.
fn import_theme(workbench: &Rc<Workbench>, source: theme_import::Source) {
    // Two buttons rather than one chooser that accepts both: GTK's file
    // dialog picks files or folders, never either, and guessing which
    // one someone meant from a filter is worse than asking.
    let choose_folder = gtk::AlertDialog::builder()
        .message(format!("Import a {} theme", source.label()))
        .detail(
            "A single theme file, or a folder holding several — an extension \
             directory or a bundle.",
        )
        .buttons(["Cancel", "Choose Folder…", "Choose File…"])
        .default_button(2)
        .cancel_button(0)
        .build();
    let workbench = Rc::clone(workbench);
    choose_folder.choose(
        Some(&workbench.window.clone()),
        gtk::gio::Cancellable::NONE,
        move |answer| {
            let Ok(answer) = answer else { return };
            if answer == 0 {
                return;
            }
            let dialog = gtk::FileDialog::new();
            dialog.set_title(&format!("Import a {} theme", source.label()));
            let window = workbench.window.clone();
            let done = {
                let workbench = Rc::clone(&workbench);
                move |result: Result<gtk::gio::File, glib::Error>| {
                    let Ok(file) = result else { return };
                    let Some(path) = file.path() else { return };
                    finish_theme_import(&workbench, &path, source);
                }
            };
            if answer == 1 {
                dialog.select_folder(Some(&window), gtk::gio::Cancellable::NONE, done);
            } else {
                dialog.open(Some(&window), gtk::gio::Cancellable::NONE, done);
            }
        },
    );
}

/// Reads what was chosen, writes the themes, puts the first on, and
/// says what happened — including the parts easy to miss: which side of
/// the palette was filled, and any scope that had nowhere to go.
fn finish_theme_import(
    workbench: &Rc<Workbench>,
    path: &Path,
    source: theme_import::Source,
) {
    let outcome = theme_import::import_into(path, source, &crate::shell::themes_dir());
    if outcome.written.is_empty() {
        let reason = outcome
            .errors
            .first()
            .cloned()
            .unwrap_or_else(|| format!("No {} theme was found there.", source.label()));
        workbench.toast(&fill(&tr("Nothing was imported. {}"), &[&reason]));
        return;
    }

    let mut lines = Vec::new();
    for (name, appearance) in outcome.written.iter().zip(&outcome.appearances) {
        lines.push(format!(
            "{name} — written for a {appearance} background, so its {appearance} colours \
             are set and the other side keeps Textchum's."
        ));
    }
    if !outcome.unmapped.is_empty() {
        let shown = outcome.unmapped.iter().take(6).cloned().collect::<Vec<_>>().join(", ");
        let rest = outcome.unmapped.len().saturating_sub(6);
        lines.push(format!(
            "Nothing here answers to {shown}{} — those colours are unused.",
            if rest > 0 { format!(", and {rest} more") } else { String::new() }
        ));
    }
    lines.extend(outcome.errors.iter().cloned());

    let heading = if outcome.written.len() == 1 {
        format!("Imported “{}”", outcome.written[0])
    } else {
        format!("Imported {} themes", outcome.written.len())
    };
    gtk::AlertDialog::builder()
        .message(heading)
        .detail(lines.join("\n\n"))
        .build()
        .show(Some(&workbench.window.clone()));

    // Wearing the theme is the point.
    if let Some(first) = outcome.written.first() {
        let shell = crate::shell::Shell::instance();
        shell.config.borrow_mut().set_theme(first);
        shell.apply_theme();
        shell.save_config();
    }
}

// MARK: Preferences

/// The preferences window, over the same config.json contract as
/// everywhere else: every change applies immediately and saves.
thread_local! {
    /// The one Preferences window. Settings are application-wide, so a
    /// second copy of the screen is never a second thing to look at —
    /// it is two views of one file that disagree the moment either is
    /// edited.
    static PREFERENCES: RefCell<Option<adw::PreferencesWindow>> = const { RefCell::new(None) };
}

fn show_preferences(parent: &adw::ApplicationWindow) {
    if let Some(existing) = PREFERENCES.with(|slot| slot.borrow().clone()) {
        existing.present();
        return;
    }
    let shell = Shell::instance();
    let window = adw::PreferencesWindow::new();
    // Deliberately not transient for the editor window, and never
    // modal: a transient utility window is kept above its parent and,
    // on some window managers, above the desktop's other applications
    // too — which reads as "Textchum will not let go of the focus".
    // Belonging to the application instead makes it an ordinary window
    // that can be left behind.
    if let Some(app) = parent.application() {
        window.set_application(Some(&app));
    }
    window.set_modal(false);
    window.set_title(Some(&tr("Preferences")));
    PREFERENCES.with(|slot| *slot.borrow_mut() = Some(window.clone()));
    window.connect_close_request(|_| {
        PREFERENCES.with(|slot| *slot.borrow_mut() = None);
        glib::Propagation::Proceed
    });

    let general = adw::PreferencesPage::new();
    general.set_title(&tr("General"));
    general.set_icon_name(Some("preferences-system-symbolic"));

    let appearance_group = adw::PreferencesGroup::new();
    appearance_group.set_title(&tr("Appearance"));
    let appearance_row = adw::ComboRow::new();
    appearance_row.set_title(&tr("Appearance"));
    let appearance_model = gtk::StringList::new(&["System", "Light", "Dark"]);
    appearance_row.set_model(Some(&appearance_model));
    appearance_row.set_selected(match shell.config.borrow().appearance() {
        Appearance::System => 0,
        Appearance::Light => 1,
        Appearance::Dark => 2,
    });
    {
        let shell = Rc::clone(&shell);
        appearance_row.connect_selected_notify(move |row| {
            let choice = match row.selected() {
                1 => Appearance::Light,
                2 => Appearance::Dark,
                _ => Appearance::System,
            };
            shell.config.borrow_mut().set_appearance(choice);
            shell.apply_appearance();
            shell.apply_theme();
            shell.save_config();
        });
    }
    appearance_group.add(&appearance_row);

    let theme_row = adw::ComboRow::new();
    theme_row.set_title(&tr("Theme"));
    let names: Vec<String> = crate::shell::theme_names();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let theme_model = gtk::StringList::new(&name_refs);
    theme_row.set_model(Some(&theme_model));
    let current = shell.config.borrow().theme();
    if let Some(index) = names.iter().position(|name| *name == current) {
        theme_row.set_selected(index as u32);
    }
    {
        let shell = Rc::clone(&shell);
        let names: Vec<String> = names.clone();
        theme_row.connect_selected_notify(move |row| {
            if let Some(name) = names.get(row.selected() as usize) {
                shell.config.borrow_mut().set_theme(name);
                shell.apply_theme();
                shell.save_config();
            }
        });
    }
    appearance_group.add(&theme_row);

    // Packs already seen are a list; a new one is a folder somewhere on
    // disk, so both a picker and a chooser. Importing copies the pack
    // into Textchum's own folder, where moving or deleting the original
    // cannot take it away.
    let icons_row = adw::ComboRow::new();
    icons_row.set_title(&tr("File icons"));
    let import = gtk::Button::with_label("Import…");
    import.set_valign(gtk::Align::Center);
    let open = gtk::Button::with_label(&tr("Open…"));
    open.set_valign(gtk::Align::Center);
    let delete = gtk::Button::with_label("Delete");
    delete.set_valign(gtk::Align::Center);

    // Set while the list is being rebuilt, so the selection handler
    // does not answer its own writes.
    let refilling = Rc::new(std::cell::Cell::new(false));
    // The paths the rows stand for, in the same order.
    let listed: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));

    let rebuild = {
        let shell = Rc::clone(&shell);
        let row = icons_row.clone();
        let delete = delete.clone();
        let refilling = Rc::clone(&refilling);
        let listed = Rc::clone(&listed);
        Rc::new(move || {
            let packs = {
                let config = shell.config.borrow();
                textchum_core::icons::available(
                    &icon_packs_dir(),
                    &config.known_icon_packs(),
                    config.icon_pack().as_deref(),
                )
            };
            let current = shell.config.borrow().icon_pack();
            let mut labels: Vec<String> = vec!["System icons".to_string()];
            let mut paths: Vec<String> = vec![String::new()];
            for pack in &packs {
                // A combo row has no sections, so where a pack lives is
                // said in its label rather than by a heading.
                labels.push(if pack.imported {
                    pack.name.clone()
                } else {
                    format!("{} (elsewhere)", pack.name)
                });
                paths.push(pack.path.clone());
            }
            let selected = current
                .as_deref()
                .and_then(|path| paths.iter().position(|had| had == path))
                .unwrap_or(0);
            let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();
            refilling.set(true);
            row.set_model(Some(&gtk::StringList::new(&label_refs)));
            row.set_selected(selected as u32);
            refilling.set(false);
            delete.set_visible(packs.get(selected.wrapping_sub(1)).is_some_and(|pack| {
                pack.imported && Some(pack.path.as_str()) == current.as_deref()
            }));
            *listed.borrow_mut() = paths;
        })
    };
    rebuild();

    {
        let shell = Rc::clone(&shell);
        let refilling = Rc::clone(&refilling);
        let listed = Rc::clone(&listed);
        let rebuild = Rc::clone(&rebuild);
        icons_row.connect_selected_notify(move |row| {
            if refilling.get() {
                return;
            }
            let paths = listed.borrow();
            let Some(path) = paths.get(row.selected() as usize).cloned() else {
                return;
            };
            drop(paths);
            shell
                .config
                .borrow_mut()
                .set_icon_pack((!path.is_empty()).then_some(path.as_str()));
            shell.apply_icon_pack();
            if !path.is_empty() && !textchum_core::icons::is_active() {
                // apply_icon_pack said why on the terminal; the row must
                // not claim a pack that was refused.
                shell.config.borrow_mut().set_icon_pack(None);
            }
            shell.save_config();
            refresh_file_trees();
            rebuild();
        });
    }

    {
        let shell = Rc::clone(&shell);
        let parent = window.clone();
        let rebuild = Rc::clone(&rebuild);
        import.connect_clicked(move |_| {
            let shell = Rc::clone(&shell);
            let rebuild = Rc::clone(&rebuild);
            choose_icon_pack(&parent, move |path| {
                let outcome = {
                    let mut config = shell.config.borrow_mut();
                    match textchum_core::icons::import(Path::new(&path), &icon_packs_dir()) {
                        Ok(copied) => {
                            config.forget_icon_pack(&path);
                            config.set_icon_pack(Some(&copied));
                            Ok(())
                        }
                        Err(message) => Err(message),
                    }
                };
                match outcome {
                    Ok(()) => {
                        shell.apply_icon_pack();
                        shell.save_config();
                        refresh_file_trees();
                        rebuild();
                    }
                    Err(message) => eprintln!("textchum: {message}"),
                }
            });
        });
    }

    {
        let shell = Rc::clone(&shell);
        let parent = window.clone();
        let rebuild = Rc::clone(&rebuild);
        open.connect_clicked(move |_| {
            let shell = Rc::clone(&shell);
            let rebuild = Rc::clone(&rebuild);
            choose_icon_pack(&parent, move |path| {
                {
                    let mut config = shell.config.borrow_mut();
                    config.remember_icon_pack(&path);
                    config.set_icon_pack(Some(&path));
                }
                shell.apply_icon_pack();
                if !textchum_core::icons::is_active() {
                    let mut config = shell.config.borrow_mut();
                    config.set_icon_pack(None);
                    config.forget_icon_pack(&path);
                }
                shell.save_config();
                refresh_file_trees();
                rebuild();
            });
        });
    }

    {
        let shell = Rc::clone(&shell);
        let rebuild = Rc::clone(&rebuild);
        delete.connect_clicked(move |_| {
            let Some(path) = shell.config.borrow().icon_pack() else {
                return;
            };
            if textchum_core::icons::remove_imported(Path::new(&path), &icon_packs_dir()).is_err()
            {
                return;
            }
            {
                let mut config = shell.config.borrow_mut();
                config.forget_icon_pack(&path);
                config.set_icon_pack(None);
            }
            shell.apply_icon_pack();
            shell.save_config();
            refresh_file_trees();
            rebuild();
        });
    }

    icons_row.add_suffix(&import);
    icons_row.add_suffix(&open);
    icons_row.add_suffix(&delete);
    appearance_group.add(&icons_row);
    general.add(&appearance_group);

    let editor_group = adw::PreferencesGroup::new();
    editor_group.set_title(&tr("Editor"));
    let font_row = adw::SpinRow::with_range(6.0, 72.0, 1.0);
    font_row.set_title(&tr("Font size"));
    font_row.set_value(shell.config.borrow().font_size());
    {
        let shell = Rc::clone(&shell);
        font_row.connect_value_notify(move |row| {
            shell.config.borrow_mut().set_font_size(row.value());
            shell.save_config();
            apply_font_size(row.value());
        });
    }
    editor_group.add(&font_row);
    let tab_row = adw::SpinRow::with_range(1.0, 16.0, 1.0);
    tab_row.set_title(&tr("Tab width"));
    tab_row.set_value(shell.config.borrow().tab_width() as f64);
    {
        let shell = Rc::clone(&shell);
        tab_row.connect_value_notify(move |row| {
            shell.config.borrow_mut().set_tab_width(row.value() as u32);
            shell.save_config();
            for_all_views(|view| view.set_tab_width(row.value() as u32));
        });
    }
    editor_group.add(&tab_row);
    let lines_row = adw::SwitchRow::new();
    lines_row.set_title(&tr("Show line numbers"));
    lines_row.set_active(shell.config.borrow().line_numbers());
    {
        let shell = Rc::clone(&shell);
        lines_row.connect_active_notify(move |row| {
            shell.config.borrow_mut().set_line_numbers(row.is_active());
            shell.save_config();
            for_all_views(|view| view.set_show_line_numbers(row.is_active()));
        });
    }
    editor_group.add(&lines_row);
    let context_row = adw::SwitchRow::new();
    context_row.set_title(&tr("Pin enclosing context lines"));
    context_row.set_active(shell.config.borrow().context_lines());
    {
        let shell = Rc::clone(&shell);
        context_row.connect_active_notify(move |row| {
            shell.config.borrow_mut().set_context_lines(row.is_active());
            shell.save_config();
            refresh_all_context_strips();
        });
    }
    editor_group.add(&context_row);
    let marks_row = adw::ComboRow::new();
    marks_row.set_title(&tr("Gutter marks compare against"));
    let marks_choices =
        gtk::StringList::new(&[&tr("The last commit"), &tr("Where the branch forked")]);
    marks_row.set_model(Some(&marks_choices));
    marks_row.set_selected(if shell.config.borrow().git_marks() == "branch" { 1 } else { 0 });
    {
        let shell = Rc::clone(&shell);
        marks_row.connect_selected_notify(move |row| {
            shell
                .config
                .borrow_mut()
                .set_git_marks(if row.selected() == 1 { "branch" } else { "head" });
            shell.save_config();
            refresh_all_change_marks();
        });
    }
    editor_group.add(&marks_row);
    let branches_view = gtk::TextView::new();
    branches_view.set_monospace(true);
    branches_view.set_top_margin(6);
    branches_view.set_bottom_margin(6);
    branches_view.set_left_margin(8);
    branches_view
        .buffer()
        .set_text(&shell.config.borrow().merge_base_branches().join("\n"));
    {
        let shell = Rc::clone(&shell);
        branches_view.buffer().connect_changed(move |buffer| {
            let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), true);
            let names: Vec<String> = text
                .lines()
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_string)
                .collect();
            shell.config.borrow_mut().set_merge_base_branches(&names);
            shell.save_config();
        });
    }
    let branches_scroll = gtk::ScrolledWindow::builder()
        .child(&branches_view)
        .min_content_height(72)
        .build();
    branches_scroll.add_css_class("card");
    let branches_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let branches_title = gtk::Label::new(Some(&tr("Merge-base branches, most likely first")));
    branches_title.set_xalign(0.0);
    let branches_hint =
        gtk::Label::new(Some(&tr("Tried in order when git does not name a default branch.")));
    branches_hint.set_xalign(0.0);
    branches_hint.add_css_class("dim-label");
    branches_box.append(&branches_title);
    branches_box.append(&branches_hint);
    branches_box.append(&branches_scroll);
    branches_box.set_margin_top(8);
    editor_group.add(&branches_box);
    let keep_row = adw::SwitchRow::new();
    keep_row.set_title(&tr("Keep files open when their window closes"));
    keep_row.set_subtitle(
        "Closing a window puts its files aside with anything unsaved, to \
         reopen or settle when the editor closes",
    );
    keep_row.set_active(shell.config.borrow().keep_buffers());
    {
        let shell = Rc::clone(&shell);
        keep_row.connect_active_notify(move |row| {
            shell.config.borrow_mut().set_keep_buffers(row.is_active());
            shell.save_config();
        });
    }
    editor_group.add(&keep_row);
    let in_project_row = adw::SwitchRow::new();
    in_project_row.set_title(&tr("Keep each project's state with the checkout"));
    in_project_row.set_subtitle(
        "A file remembers how it is split, what is folded and what it was told it is. \
         With this on that goes to .tchum in the project; otherwise it is kept here, \
         one record per project",
    );
    in_project_row.set_active(shell.config.borrow().project_state_in_project());
    {
        let shell = Rc::clone(&shell);
        in_project_row.connect_active_notify(move |row| {
            shell
                .config
                .borrow_mut()
                .set_project_state_in_project(row.is_active());
            shell.save_config();
        });
    }
    editor_group.add(&in_project_row);

    let language_row = adw::ComboRow::new();
    language_row.set_title(&tr("Interface language"));
    language_row.set_subtitle(&tr("Restart to apply"));
    let language_names = gtk::StringList::new(&[]);
    language_names.append(&tr("System"));
    for (_, name) in textchum_core::i18n::LANGUAGES {
        language_names.append(name);
    }
    language_row.set_model(Some(&language_names));
    let current = shell.config.borrow().interface_language();
    let selected = if current == "system" {
        0
    } else {
        textchum_core::i18n::LANGUAGES
            .iter()
            .position(|(tag, _)| *tag == current)
            .map(|at| at + 1)
            .unwrap_or(0)
    };
    language_row.set_selected(selected as u32);
    {
        let shell = Rc::clone(&shell);
        language_row.connect_selected_notify(move |row| {
            let tag = match row.selected() {
                0 => "system".to_string(),
                index => textchum_core::i18n::LANGUAGES
                    .get(index as usize - 1)
                    .map(|(tag, _)| (*tag).to_string())
                    .unwrap_or_else(|| "system".to_string()),
            };
            shell.config.borrow_mut().set_interface_language(&tag);
            shell.save_config();
        });
    }
    editor_group.add(&language_row);

    let records_row = adw::EntryRow::new();
    records_row.set_title(&tr("Records folder"));
    records_row.set_text(
        &shell
            .config
            .borrow()
            .project_state_dir()
            .unwrap_or_else(|| crate::paths::projects_dir().to_string_lossy().into_owned()),
    );
    {
        let shell = Rc::clone(&shell);
        records_row.connect_apply(move |row| {
            let text = row.text().to_string();
            let default = crate::paths::projects_dir().to_string_lossy().into_owned();
            let value = if text == default { None } else { Some(text.as_str()) };
            shell.config.borrow_mut().set_project_state_dir(value);
            shell.save_config();
        });
    }
    let manage = gtk::Button::with_label(&tr("Manage…"));
    manage.set_valign(gtk::Align::Center);
    {
        let window = window.clone();
        manage.connect_clicked(move |_| show_project_records(&window));
    }
    records_row.add_suffix(&manage);
    editor_group.add(&records_row);

    let sweep_row = adw::SwitchRow::new();
    sweep_row.set_title(&tr("Forget records at launch"));
    sweep_row.set_subtitle(
        "On a thread of its own, and the records of projects that are no longer there \
         go whatever the window says",
    );
    sweep_row.set_active(shell.config.borrow().project_state_sweep());
    {
        let shell = Rc::clone(&shell);
        sweep_row.connect_active_notify(move |row| {
            shell
                .config
                .borrow_mut()
                .set_project_state_sweep(row.is_active());
            shell.save_config();
        });
    }
    editor_group.add(&sweep_row);

    let keep_days_row = adw::SpinRow::with_range(0.0, 3650.0, 30.0);
    keep_days_row.set_title(&tr("Keep records for"));
    keep_days_row.set_subtitle(&tr("Days since a record was last written; zero keeps them"));
    keep_days_row.set_value(shell.config.borrow().project_state_keep_days() as f64);
    {
        let shell = Rc::clone(&shell);
        keep_days_row.connect_value_notify(move |row| {
            shell
                .config
                .borrow_mut()
                .set_project_state_keep_days(row.value() as u32);
            shell.save_config();
        });
    }
    editor_group.add(&keep_days_row);
    let hover_row = adw::SwitchRow::new();
    hover_row.set_title(&tr("Hover documentation"));
    hover_row.set_subtitle(&tr("Show server documentation when the mouse rests on a symbol"));
    hover_row.set_active(shell.config.borrow().hover_docs());
    {
        let shell = Rc::clone(&shell);
        hover_row.connect_active_notify(move |row| {
            shell.config.borrow_mut().set_hover_docs(row.is_active());
            shell.save_config();
        });
    }
    editor_group.add(&hover_row);

    let occurrences_row = adw::SwitchRow::new();
    occurrences_row.set_title(&tr("Mark the selected word elsewhere"));
    occurrences_row.set_subtitle(&tr("Selecting a whole word marks its other occurrences on screen"));
    occurrences_row.set_active(shell.config.borrow().mark_occurrences());
    let occurrences_case = adw::SwitchRow::new();
    occurrences_case.set_title(&tr("Match case"));
    occurrences_case.set_active(shell.config.borrow().occurrence_options().case_sensitive);
    occurrences_case.set_sensitive(occurrences_row.is_active());
    let occurrences_word = adw::SwitchRow::new();
    occurrences_word.set_title(&tr("Whole words only"));
    occurrences_word.set_subtitle(&tr("Off marks `item` inside `items` too"));
    occurrences_word.set_active(shell.config.borrow().occurrence_options().whole_word);
    occurrences_word.set_sensitive(occurrences_row.is_active());
    {
        let shell = Rc::clone(&shell);
        let case = occurrences_case.clone();
        let word = occurrences_word.clone();
        occurrences_row.connect_active_notify(move |row| {
            shell.config.borrow_mut().set_mark_occurrences(row.is_active());
            shell.save_config();
            case.set_sensitive(row.is_active());
            word.set_sensitive(row.is_active());
            refresh_every_page_occurrences();
        });
    }
    {
        let shell = Rc::clone(&shell);
        occurrences_case.connect_active_notify(move |row| {
            shell
                .config
                .borrow_mut()
                .set_occurrences_case_sensitive(row.is_active());
            shell.save_config();
            refresh_every_page_occurrences();
        });
    }
    {
        let shell = Rc::clone(&shell);
        occurrences_word.connect_active_notify(move |row| {
            shell
                .config
                .borrow_mut()
                .set_occurrences_whole_word(row.is_active());
            shell.save_config();
            refresh_every_page_occurrences();
        });
    }
    editor_group.add(&occurrences_row);
    editor_group.add(&occurrences_case);
    editor_group.add(&occurrences_word);

    let spell_row = adw::EntryRow::new();
    spell_row.set_title(&tr("Spell check prose (off, auto, or dictionaries like en_US, es_ES)"));
    spell_row.set_text(
        shell
            .config
            .borrow()
            .spell_language()
            .as_deref()
            .unwrap_or(""),
    );
    spell_row.set_show_apply_button(true);
    spell_row.set_tooltip_text(Some(
        "Checks comments in code, and everything in Markdown, git commit \
         messages, and plain text. Several dictionaries can apply at once, \
         separated by commas. Needs hunspell and its dictionaries.",
    ));
    // The field stays free text — a dictionary can be named that
    // hunspell finds and this list does not — but what is installed is
    // one click away, because "es_ES" is only guessable if you already
    // know the convention.
    let installed = crate::spell::available_dictionaries();
    if !installed.is_empty() {
        let dictionaries = gtk::gio::Menu::new();
        for name in &installed {
            dictionaries.append(
                Some(&menu_label(name)),
                Some(&format!("spell.pick('{}')", name.replace('\'', "\\'"))),
            );
        }
        let button = gtk::MenuButton::builder()
            .icon_name("view-more-symbolic")
            .menu_model(&dictionaries)
            .valign(gtk::Align::Center)
            .tooltip_text("Installed dictionaries")
            .build();
        button.add_css_class("flat");
        spell_row.add_suffix(&button);
        // The menu writes the setting; the row follows it so the field
        // never disagrees with the file.
        let row = spell_row.clone();
        let shell_for_pick = Rc::clone(&shell);
        let action = gtk::gio::SimpleAction::new("pick", Some(glib::VariantTy::STRING));
        action.connect_activate(move |_, parameter| {
            let Some(name) = parameter.and_then(|p| p.str().map(str::to_owned)) else {
                return;
            };
            // Adding to what is there rather than replacing it: picking
            // a second dictionary is how a bilingual document gets set
            // up, and that is the harder thing to type by hand.
            let mut chosen = shell_for_pick.config.borrow().spell_languages();
            if !chosen.iter().any(|existing| *existing == name) {
                chosen.push(name);
            }
            let value = chosen.join(", ");
            row.set_text(&value);
            shell_for_pick
                .config
                .borrow_mut()
                .set_spell_language(Some(&value));
            shell_for_pick.save_config();
            recheck_every_page();
        });
        // The action group hangs off the button rather than the window:
        // AdwPreferencesWindow is a GtkWindow, and a plain GtkWindow is
        // not a GActionMap — only GtkApplicationWindow is.
        let actions = gtk::gio::SimpleActionGroup::new();
        actions.add_action(&action);
        button.insert_action_group("spell", Some(&actions));
    }
    {
        let shell = Rc::clone(&shell);
        spell_row.connect_apply(move |row| {
            let text = row.text();
            let trimmed = text.trim();
            let value = match trimmed {
                "" | "off" => None,
                other => Some(other),
            };
            shell.config.borrow_mut().set_spell_language(value);
            shell.save_config();
            recheck_every_page();
        });
    }
    editor_group.add(&spell_row);

    // The personal word list, edited the way the hide globs are: one
    // word per line, because that is what the context menu's "Add to
    // Dictionary" builds up and someone eventually wants to prune.
    let words = shell.config.borrow().spell_words();
    let words_row = word_list_row(
        "Dictionary",
        "Words to accept everywhere — project names, acronyms",
        &words,
        {
            let shell = Rc::clone(&shell);
            move |words: &[String]| {
                shell.config.borrow_mut().set_spell_words(words);
                shell.save_config();
                recheck_every_page();
            }
        },
    );
    editor_group.add(&words_row);

    let autosave_row = adw::SpinRow::with_range(0.0, 600.0, 5.0);
    autosave_row.set_title(&tr("Autosave after (seconds)"));
    autosave_row.set_subtitle(
        "0 keeps saving manual. Files that have a name only, and without \
         running save preprocessors — a formatter reflowing the line you are \
         typing is not a favour.",
    );
    autosave_row.set_value(shell.config.borrow().autosave_seconds() as f64);
    {
        let shell = Rc::clone(&shell);
        autosave_row.connect_value_notify(move |row| {
            shell
                .config
                .borrow_mut()
                .set_autosave_seconds(row.value() as u32);
            shell.save_config();
        });
    }
    editor_group.add(&autosave_row);
    general.add(&editor_group);
    window.add(&general);

    // Language servers: the defaults section of the same lsp config the
    // macOS Settings edit — language → command line, applied to the
    // pool for servers started afterwards.
    let servers = adw::PreferencesPage::new();
    servers.set_title(&tr("Language Servers"));
    servers.set_icon_name(Some("network-workgroup-symbolic"));
    let servers_group = adw::PreferencesGroup::new();
    servers_group.set_title(&tr("Default server commands"));
    servers_group.set_description(Some(
        "Override which command serves a language (for every project). \
         Unlisted languages use the built-in registry.",
    ));

    let configured: std::collections::BTreeMap<String, String> =
        serde_json::from_str::<serde_json::Value>(&shell.config.borrow().lsp_json())
            .ok()
            .and_then(|parsed| {
                parsed["defaults"].as_object().map(|defaults| {
                    defaults
                        .iter()
                        .filter_map(|(language, command)| {
                            command
                                .as_str()
                                .map(|command| (language.clone(), command.to_string()))
                        })
                        .collect()
                })
            })
            .unwrap_or_default();

    // Every language the registry knows gets a row, whether or not it
    // has been overridden: a screen that lists only what you already
    // configured cannot tell you what there is to configure. Languages
    // configured for something the registry does not know follow.
    let mut listed: Vec<(String, String, bool)> = Vec::new();
    for spec in textchum_lsp::registry::all() {
        for language in spec.languages {
            let default = if spec.args.is_empty() {
                spec.command.to_string()
            } else {
                format!("{} {}", spec.command, spec.args.join(" "))
            };
            let command = configured
                .get(*language)
                .cloned()
                .unwrap_or_else(|| default.clone());
            let overridden = configured.contains_key(*language);
            listed.push(((*language).to_string(), command, overridden));
        }
    }
    for (language, command) in &configured {
        if !listed.iter().any(|(name, _, _)| name == language) {
            listed.push((language.clone(), command.clone(), true));
        }
    }
    listed.sort_by(|a, b| a.0.cmp(&b.0));

    for (language, command, overridden) in listed {
        let row = adw::EntryRow::new();
        row.set_title(&language);
        row.set_text(&command);
        row.set_show_apply_button(true);
        // Whether the command would actually start, which is the
        // question a settings screen is otherwise silent about — and
        // the one behind "I installed a server and nothing happened".
        let initial = describe_server(&language, &command, overridden);
        row.set_tooltip_text(Some(&initial));
        let status = gtk::Label::new(Some(&initial));
        status.set_wrap(true);
        status.set_max_width_chars(48);
        status.set_xalign(1.0);
        status.set_valign(gtk::Align::Center);
        status.add_css_class("dim-label");
        status.add_css_class("caption");
        row.add_suffix(&status);
        let shell = Rc::clone(&shell);
        let status_for_apply = status.clone();
        row.connect_apply(move |row| {
            let text = row.text();
            let trimmed = text.trim();
            shell.config.borrow_mut().set_lsp_entry(
                None,
                &language,
                if trimmed.is_empty() { None } else { Some(trimmed) },
            );
            shell.save_config();
            shell.reconfigure_pool();
            status_for_apply.set_text(&describe_server(
                &language,
                trimmed,
                !trimmed.is_empty(),
            ));
        });
        servers_group.add(&row);
    }

    let projects_group = adw::PreferencesGroup::new();
    projects_group.set_title(&tr("Per-project overrides"));
    projects_group.set_description(Some(
        "A project root's own command wins over the defaults above.",
    ));
    let project_entries: Vec<(String, String, String)> =
        serde_json::from_str::<serde_json::Value>(&shell.config.borrow().lsp_json())
            .ok()
            .and_then(|parsed| {
                parsed["projects"].as_object().map(|projects| {
                    projects
                        .iter()
                        .flat_map(|(root, languages)| {
                            let root = root.clone();
                            languages
                                .as_object()
                                .into_iter()
                                .flatten()
                                .filter_map(|(language, command)| {
                                    command.as_str().map(|command| {
                                        (
                                            root.clone(),
                                            language.clone(),
                                            command.to_string(),
                                        )
                                    })
                                })
                                .collect::<Vec<_>>()
                        })
                        .collect()
                })
            })
            .unwrap_or_default();
    for (root, language, command) in project_entries {
        let row = adw::EntryRow::new();
        let basename = std::path::Path::new(&root)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| root.clone());
        row.set_title(&format!("{language} — {basename}"));
        row.set_tooltip_text(Some(&root));
        row.set_text(&command);
        row.set_show_apply_button(true);
        let shell = Rc::clone(&shell);
        row.connect_apply(move |row| {
            let text = row.text();
            let trimmed = text.trim();
            shell.config.borrow_mut().set_lsp_entry(
                Some(&root),
                &language,
                if trimmed.is_empty() { None } else { Some(trimmed) },
            );
            shell.save_config();
            shell.reconfigure_pool();
        });
        projects_group.add(&row);
    }
    let add_root = adw::EntryRow::new();
    add_root.set_title(&tr("project root path"));
    let add_project_language = adw::EntryRow::new();
    add_project_language.set_title(&tr("language"));
    attach_language_choices(&add_project_language);
    let add_project_command = adw::EntryRow::new();
    add_project_command.set_title(&tr("command"));
    add_project_command.set_show_apply_button(true);
    {
        let shell = Rc::clone(&shell);
        let root_row = add_root.clone();
        let language_row = add_project_language.clone();
        add_project_command.connect_apply(move |row| {
            let root = root_row.text().trim().to_string();
            let language = language_row.text().trim().to_lowercase();
            let command = row.text().trim().to_string();
            if root.is_empty() || language.is_empty() || command.is_empty() {
                return;
            }
            shell
                .config
                .borrow_mut()
                .set_lsp_entry(Some(&root), &language, Some(&command));
            shell.save_config();
            shell.reconfigure_pool();
            root_row.set_text("");
            language_row.set_text("");
            row.set_text("");
        });
    }
    projects_group.add(&add_root);
    projects_group.add(&add_project_language);
    projects_group.add(&add_project_command);

    let add_language = adw::EntryRow::new();
    add_language.set_title(&tr("language (e.g. python)"));
    attach_language_choices(&add_language);
    let add_command = adw::EntryRow::new();
    add_command.set_title(&tr("command (e.g. pylsp)"));
    add_command.set_show_apply_button(true);
    {
        let shell = Rc::clone(&shell);
        let language_row = add_language.clone();
        add_command.connect_apply(move |row| {
            let language = language_row.text().trim().to_lowercase();
            let command = row.text().trim().to_string();
            if language.is_empty() || command.is_empty() {
                return;
            }
            shell
                .config
                .borrow_mut()
                .set_lsp_entry(None, &language, Some(&command));
            shell.save_config();
            shell.reconfigure_pool();
            language_row.set_text("");
            row.set_text("");
        });
    }
    servers_group.add(&add_language);
    servers_group.add(&add_command);
    servers.add(&servers_group);
    servers.add(&projects_group);

    // Save preprocessors: the same chains the macOS Settings edit —
    // one command per link, ` ;; ` separating links in these rows
    // (the file stores them as an array, one per line).
    let preprocessors_group = adw::PreferencesGroup::new();
    preprocessors_group.set_title(&tr("Save preprocessors"));
    preprocessors_group.set_description(Some(
        "Commands run before every save, in order, separated by ' ;; ' — \
         each reads the document on stdin and writes it back on stdout \
         (ruff check --fix - ;; black -). {path} and {filename} expand \
         to the document's. A project entry replaces the defaults.",
    ));
    let preprocessor_entries: Vec<(Option<String>, String, String)> = {
        let json = shell.config.borrow().preprocessors_json();
        let parsed = serde_json::from_str::<serde_json::Value>(&json).unwrap_or_default();
        let chain_of = |value: &serde_json::Value| -> Option<String> {
            if let Some(one) = value.as_str() {
                return Some(one.to_owned());
            }
            value.as_array().map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str())
                    .collect::<Vec<_>>()
                    .join(" ;; ")
            })
        };
        let mut entries = Vec::new();
        for (language, value) in parsed["defaults"].as_object().into_iter().flatten() {
            if let Some(chain) = chain_of(value) {
                entries.push((None, language.clone(), chain));
            }
        }
        for (root, languages) in parsed["projects"].as_object().into_iter().flatten() {
            for (language, value) in languages.as_object().into_iter().flatten() {
                if let Some(chain) = chain_of(value) {
                    entries.push((Some(root.clone()), language.clone(), chain));
                }
            }
        }
        entries
    };
    for (root, language, chain) in preprocessor_entries {
        let row = adw::EntryRow::new();
        match &root {
            None => row.set_title(&language),
            Some(root) => {
                let basename = std::path::Path::new(root)
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| root.clone());
                row.set_title(&format!("{language} — {basename}"));
                row.set_tooltip_text(Some(root));
            }
        }
        row.set_text(&chain);
        row.set_show_apply_button(true);
        let shell = Rc::clone(&shell);
        row.connect_apply(move |row| {
            let text = row.text();
            let commands = text.trim().replace(" ;; ", "\n");
            shell.config.borrow_mut().set_preprocessor_entry(
                root.as_deref(),
                &language,
                if commands.is_empty() { None } else { Some(&commands) },
            );
            shell.save_config();
        });
        preprocessors_group.add(&row);
    }
    let add_preprocessor_root = adw::EntryRow::new();
    add_preprocessor_root.set_title(&tr("project root (empty = default for all projects)"));
    let add_preprocessor_language = adw::EntryRow::new();
    add_preprocessor_language.set_title(&tr("language (e.g. python)"));
    let add_preprocessor_chain = adw::EntryRow::new();
    add_preprocessor_chain.set_title(&tr("commands (e.g. ruff check --fix - ;; black -)"));
    add_preprocessor_chain.set_show_apply_button(true);
    {
        let shell = Rc::clone(&shell);
        let root_row = add_preprocessor_root.clone();
        let language_row = add_preprocessor_language.clone();
        add_preprocessor_chain.connect_apply(move |row| {
            let root = root_row.text().trim().to_string();
            let language = language_row.text().trim().to_lowercase();
            let commands = row.text().trim().replace(" ;; ", "\n");
            if language.is_empty() || commands.is_empty() {
                return;
            }
            shell.config.borrow_mut().set_preprocessor_entry(
                (!root.is_empty()).then_some(root.as_str()),
                &language,
                Some(&commands),
            );
            shell.save_config();
            root_row.set_text("");
            language_row.set_text("");
            row.set_text("");
        });
    }
    preprocessors_group.add(&add_preprocessor_root);
    preprocessors_group.add(&add_preprocessor_language);
    preprocessors_group.add(&add_preprocessor_chain);
    servers.add(&preprocessors_group);
    window.add(&servers);

    // Projects: how roots are detected, defaults plus per-root
    // overrides — the same workspace section the macOS Settings edit.
    let projects_page = adw::PreferencesPage::new();
    projects_page.set_title(&tr("Projects"));
    projects_page.set_icon_name(Some("folder-symbolic"));
    let workspace_defaults = adw::PreferencesGroup::new();
    workspace_defaults.set_title(&tr("Defaults (all projects)"));
    workspace_defaults.set_description(Some(
        "Manifest projects splits a repository at language manifests; \
         recursive config cascades a root's settings into nested projects.",
    ));
    let workspace: serde_json::Value =
        serde_json::from_str(&shell.config.borrow().workspace_json())
            .unwrap_or(serde_json::Value::Null);
    for (key, title) in [
        ("manifest_projects", "Manifest projects"),
        ("recursive_config", "Recursive config"),
        ("ctags_fallback", "Ctags fallback"),
    ] {
        let row = adw::SwitchRow::new();
        row.set_title(title);
        row.set_active(workspace[key].as_bool().unwrap_or(false));
        let shell = Rc::clone(&shell);
        row.connect_active_notify(move |row| {
            shell
                .config
                .borrow_mut()
                .set_workspace_flag(None, key, Some(row.is_active()));
            shell.save_config();
            shell.reconfigure_pool();
        });
        workspace_defaults.add(&row);
    }
    {
        let presets = shell.config.borrow().hide_presets();
        let globs = shell.config.borrow().hide_globs(None);
        let shell = Rc::clone(&shell);
        workspace_defaults.add(&glob_editor_row(
            "Hide in tree",
            Some("every project"),
            &globs,
            presets,
            move |globs| {
                shell
                    .config
                    .borrow_mut()
                    .set_hide_globs(None, (!globs.is_empty()).then_some(globs));
                shell.save_config();
            },
        ));
    }
    projects_page.add(&workspace_defaults);

    let workspace_overrides = adw::PreferencesGroup::new();
    workspace_overrides.set_title(&tr("Per-project overrides"));
    let configured_roots = shell.config.borrow().configured_projects();
    if let Some(projects) = workspace["projects"].as_object() {
        for (root, flags) in projects {
            let expander = adw::ExpanderRow::new();
            let basename = std::path::Path::new(root)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| root.clone());
            expander.set_title(&basename);
            if Path::new(root).exists() {
                expander.set_subtitle(root);
            } else {
                expander.set_subtitle(&format!("{root} — missing"));
            }
            {
                // Copying onto a project that already exists: the same
                // question, asked from the other end.
                let sources: Vec<String> = configured_roots
                    .iter()
                    .filter(|other| *other != root)
                    .cloned()
                    .collect();
                let row = adw::EntryRow::new();
                row.set_title(&tr("copy settings from this project"));
                row.set_show_apply_button(true);
                attach_path_choices(&row, "Configured projects", sources);
                let shell = Rc::clone(&shell);
                let root = root.clone();
                row.connect_apply(move |row| {
                    let source = row.text().trim().to_string();
                    if source.is_empty() {
                        return;
                    }
                    shell.config.borrow_mut().copy_project(
                        &source,
                        &root,
                        textchum_core::ProjectParts::default(),
                    );
                    shell.save_config();
                    shell.reconfigure_pool();
                    row.set_text("");
                });
                expander.add_row(&row);
            }
            for (key, title) in [
                ("manifest_projects", "Manifest projects"),
                ("recursive_config", "Recursive config"),
                ("ctags_fallback", "Ctags fallback"),
            ] {
                let row = adw::SwitchRow::new();
                row.set_title(title);
                row.set_active(flags[key].as_bool().unwrap_or(false));
                let shell = Rc::clone(&shell);
                let root = root.clone();
                row.connect_active_notify(move |row| {
                    shell.config.borrow_mut().set_workspace_flag(
                        Some(&root),
                        key,
                        Some(row.is_active()),
                    );
                    shell.save_config();
                    shell.reconfigure_pool();
                });
                expander.add_row(&row);
            }
            // Editor overrides for windows inside this root; empty
            // fields inherit the General page's values. New windows
            // pick them up (existing views keep their look until
            // reopened).
            let overrides: serde_json::Value = serde_json::from_str(
                &shell.config.borrow().editor_overrides_json(root),
            )
            .unwrap_or_default();
            for (key, title, current) in [
                (
                    "font_family",
                    "Editor font family (empty = inherit)",
                    overrides["font_family"].as_str().unwrap_or("").to_owned(),
                ),
                (
                    "font_size",
                    "Editor font size (empty = inherit)",
                    overrides["font_size"]
                        .as_f64()
                        .map(|size| {
                            if size == size.trunc() {
                                format!("{}", size as i64)
                            } else {
                                format!("{size}")
                            }
                        })
                        .unwrap_or_default(),
                ),
                (
                    "tab_width",
                    "Editor tab width (empty = inherit)",
                    overrides["tab_width"]
                        .as_u64()
                        .map(|width| width.to_string())
                        .unwrap_or_default(),
                ),
            ] {
                let row = adw::EntryRow::new();
                row.set_title(title);
                row.set_text(&current);
                row.set_show_apply_button(true);
                let shell = Rc::clone(&shell);
                let root = root.clone();
                row.connect_apply(move |row| {
                    let text = row.text();
                    let trimmed = text.trim();
                    let value = if trimmed.is_empty() {
                        None
                    } else if key == "font_family" {
                        Some(format!("\"{}\"", trimmed.replace('"', "")))
                    } else if key == "tab_width" {
                        trimmed.parse::<u32>().ok().map(|v| v.to_string())
                    } else {
                        trimmed.parse::<f64>().ok().map(|v| v.to_string())
                    };
                    shell
                        .config
                        .borrow_mut()
                        .set_editor_override(&root, key, value.as_deref());
                    shell.save_config();
                });
                expander.add_row(&row);
            }
            workspace_overrides.add(&expander);
            {
                let presets = shell.config.borrow().hide_presets();
                let globs = shell.config.borrow().hide_globs(Some(root));
                let shell = Rc::clone(&shell);
                let root = root.clone();
                workspace_overrides.add(&glob_editor_row(
                    "Hide in tree",
                    Some(&basename),
                    &globs,
                    presets,
                    move |globs| {
                        shell.config.borrow_mut().set_hide_globs(
                            Some(&root),
                            (!globs.is_empty()).then_some(globs),
                        );
                        shell.save_config();
                    },
                ));
            }
        }
    }
    // A second service in the same layout wants the settings the first
    // one has; entering them again is a transcription exercise with a
    // typo in it.
    let copy_from = adw::EntryRow::new();
    copy_from.set_title(&tr("start from this project's settings (optional)"));
    attach_path_choices(
        &copy_from,
        "Configured projects",
        shell.config.borrow().configured_projects(),
    );

    let add_workspace_root = adw::EntryRow::new();
    add_workspace_root.set_title(&tr("add project root path"));
    add_workspace_root.set_show_apply_button(true);
    attach_path_choices(
        &add_workspace_root,
        "Open projects",
        open_project_roots()
            .into_iter()
            .filter(|root| !configured_roots.contains(root))
            .collect(),
    );
    {
        let shell = Rc::clone(&shell);
        let copy_from = copy_from.clone();
        add_workspace_root.connect_apply(move |row| {
            let root = row.text().trim().to_string();
            if root.is_empty() {
                return;
            }
            let source = copy_from.text().trim().to_string();
            if !source.is_empty() {
                shell.config.borrow_mut().copy_project(
                    &source,
                    &root,
                    textchum_core::ProjectParts::default(),
                );
            }
            shell
                .config
                .borrow_mut()
                .set_workspace_flag(Some(&root), "manifest_projects", Some(false));
            shell
                .config
                .borrow_mut()
                .set_workspace_flag(Some(&root), "recursive_config", Some(false));
            shell.save_config();
            shell.reconfigure_pool();
            row.set_text("");
            copy_from.set_text("");
        });
    }
    workspace_overrides.add(&copy_from);
    workspace_overrides.add(&add_workspace_root);

    // A root whose directory is gone is an entry nothing will ever
    // match again.
    let stale: Vec<String> = configured_roots
        .iter()
        .filter(|root| !Path::new(root).exists())
        .cloned()
        .collect();
    if !stale.is_empty() {
        let row = adw::ActionRow::new();
        row.set_title(&if stale.len() == 1 {
            "1 configured project no longer exists on disk".to_string()
        } else {
            format!("{} configured projects no longer exist on disk", stale.len())
        });
        row.set_subtitle(&stale.join(", "));
        let button = gtk::Button::with_label(&tr("Remove missing"));
        button.set_valign(gtk::Align::Center);
        {
            let shell = Rc::clone(&shell);
            let stale = stale.clone();
            button.connect_clicked(move |button| {
                for root in &stale {
                    shell.config.borrow_mut().remove_project(root);
                }
                shell.save_config();
                shell.reconfigure_pool();
                button.set_sensitive(false);
                button.set_label("Removed");
            });
        }
        row.add_suffix(&button);
        workspace_overrides.add(&row);
    }
    projects_page.add(&workspace_overrides);

    // The presets themselves, edited the same way: one pattern per
    // line. Editing any preset takes ownership of the whole set.
    let presets_group = adw::PreferencesGroup::new();
    presets_group.set_title(&tr("Hide presets"));
    presets_group.set_description(Some(
        "Named glob sets the hide editors add in one click. Edit any preset \
         and this list replaces the built-in one, so removals stick.",
    ));
    for (name, globs) in shell.config.borrow().hide_presets() {
        let shell = Rc::clone(&shell);
        let preset_name = name.clone();
        presets_group.add(&glob_editor_row(
            &name,
            None,
            &globs,
            Vec::new(),
            move |globs| {
                shell
                    .config
                    .borrow_mut()
                    .set_hide_preset(&preset_name, (!globs.is_empty()).then_some(globs));
                shell.save_config();
            },
        ));
    }
    let add_preset = adw::EntryRow::new();
    add_preset.set_title(&tr("new preset name"));
    add_preset.set_show_apply_button(true);
    {
        let shell = Rc::clone(&shell);
        add_preset.connect_apply(move |row| {
            let name = row.text().trim().to_string();
            if name.is_empty() {
                return;
            }
            // A fresh preset starts with a placeholder pattern: an
            // empty one would remove itself immediately.
            shell
                .config
                .borrow_mut()
                .set_hide_preset(&name, Some(&name.to_lowercase()));
            shell.save_config();
            row.set_text("");
        });
    }
    presets_group.add(&add_preset);
    let restore = adw::ActionRow::new();
    restore.set_title(&tr("Restore built-in presets"));
    let restore_button = gtk::Button::with_label("Restore");
    restore_button.set_valign(gtk::Align::Center);
    {
        let shell = Rc::clone(&shell);
        restore_button.connect_clicked(move |_| {
            shell.config.borrow_mut().reset_hide_presets();
            shell.save_config();
        });
    }
    restore.add_suffix(&restore_button);
    presets_group.add(&restore);
    projects_page.add(&presets_group);
    window.add(&projects_page);

    window.add(&keyboard_page(&shell));

    window.present();
}

/// Where imported icon packs live: `~/.local/share/textchum/icons`,
/// or the profile `--data-dir` named.
fn icon_packs_dir() -> PathBuf {
    crate::paths::icons_dir()
}

/// Asks for an icon pack — the theme's JSON file, or the extension
/// folder holding it — and hands the path over.
///
/// Both are offered because packs are distributed both ways, and asking
/// someone which one they have is asking them to read a manifest.
fn choose_icon_pack<F>(parent: &adw::PreferencesWindow, done: F)
where
    F: Fn(String) + Clone + 'static,
{
    let ask = gtk::AlertDialog::builder()
        .message("Choose an icon pack")
        .detail("The icon theme's JSON file, or the extension folder holding it.")
        .buttons(["Cancel", "Choose Folder…", "Choose File…"])
        .default_button(2)
        .cancel_button(0)
        .build();
    let parent = parent.clone();
    ask.choose(
        Some(&parent.clone()),
        gtk::gio::Cancellable::NONE,
        move |answer| {
            let Ok(answer) = answer else { return };
            if answer == 0 {
                return;
            }
            let dialog = gtk::FileDialog::new();
            dialog.set_title(&tr("Choose an icon pack"));
            let done = done.clone();
            let chosen = move |result: Result<gtk::gio::File, glib::Error>| {
                let Ok(file) = result else { return };
                let Some(path) = file.path() else { return };
                done(path.to_string_lossy().into_owned());
            };
            if answer == 1 {
                dialog.select_folder(Some(&parent), gtk::gio::Cancellable::NONE, chosen);
            } else {
                dialog.open(Some(&parent), gtk::gio::Cancellable::NONE, chosen);
            }
        },
    );
}

/// The tab holding a page in a group, or `None` when that group is not
/// the one it is in.
///
/// `adw_tab_view_get_page` asserts that the child belongs to the view
/// rather than answering, so it cannot be asked speculatively — and
/// with two groups, every lookup is speculative.
pub fn tab_page_of(view: &adw::TabView, page: &Rc<Page>) -> Option<adw::TabPage> {
    (0..view.n_pages())
        .map(|at| view.nth_page(at))
        .find(|tab_page| tab_page.child() == page.root)
}

/// Re-installs the shortcuts after a change to the profile or an
/// override.
fn reapply_keys() {
    let Some(app) = gtk::gio::Application::default() else {
        return;
    };
    let Ok(app) = app.downcast::<adw::Application>() else {
        return;
    };
    crate::apply_key_overrides(&app);
}

/// The keyboard shortcuts, and the profiles that set them.
///
/// People arrive from another editor with its shortcuts in their
/// fingers, so the ones those editors are known for are offered whole.
/// A profile names the commands it moves and nothing else; a single
/// shortcut can still be changed on top of one, and saving the result
/// makes it a profile of its own.
fn keyboard_page(shell: &Rc<Shell>) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::new();
    page.set_title(&tr("Keyboard"));
    page.set_icon_name(Some("preferences-desktop-keyboard-symbolic"));

    let profile_group = adw::PreferencesGroup::new();
    profile_group.set_title(&tr("Profile"));
    profile_group.set_description(Some(
        "A profile sets the shortcuts its editor is known for and leaves the rest \
         alone. Changing one on top of a profile keeps the profile; saving turns \
         what is in force into a profile of your own. Shortcuts are written as \
         \"cmd+shift+f\" — cmd is Ctrl here and Command on macOS.",
    ));

    let choices = textchum_core::keys::choices(&shell.config.borrow().key_profiles_json());
    let mut labels: Vec<String> = vec!["Textchum".to_string()];
    labels.extend(choices.iter().map(|(_, name)| name.clone()));
    let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    let chosen = shell.config.borrow().keys_profile();
    let profile_row = adw::ComboRow::new();
    profile_row.set_title(&tr("Profile"));
    profile_row.set_model(Some(&gtk::StringList::new(&label_refs)));
    profile_row.set_selected(
        choices
            .iter()
            .position(|(id, _)| *id == chosen)
            .map(|index| index as u32 + 1)
            .unwrap_or(0),
    );
    {
        let shell = Rc::clone(shell);
        let ids: Vec<String> = choices.iter().map(|(id, _)| id.clone()).collect();
        profile_row.connect_selected_notify(move |row| {
            let selected = row.selected() as usize;
            let name = if selected == 0 {
                None
            } else {
                ids.get(selected - 1).map(String::as_str)
            };
            shell.config.borrow_mut().set_keys_profile(name);
            shell.save_config();
            reapply_keys();
            refresh_shortcut_fields();
        });
    }
    profile_group.add(&profile_row);

    let save_row = adw::EntryRow::new();
    save_row.set_title(&tr("save what is in force as a profile named"));
    save_row.set_show_apply_button(true);
    {
        let shell = Rc::clone(shell);
        save_row.connect_apply(move |row| {
            let name = row.text().trim().to_string();
            if name.is_empty() {
                return;
            }
            let bindings = {
                let config = shell.config.borrow();
                textchum_core::keys::effective(
                    &config.keys_profile(),
                    &config.key_profiles_json(),
                    &config.keys_json(),
                )
            };
            {
                let mut config = shell.config.borrow_mut();
                config.set_key_profile(&name, Some(&textchum_core::keys::to_json(&bindings)));
                config.set_keys_profile(Some(&name));
                // The changes are in the profile now; leaving them
                // behind would apply them twice and hide what the
                // profile says.
                config.clear_key_bindings();
            }
            shell.save_config();
            reapply_keys();
            row.set_text("");
        });
    }
    profile_group.add(&save_row);

    let reset = adw::ActionRow::new();
    reset.set_title(&tr("Reset changes"));
    reset.set_subtitle(&tr("Give every command back the shortcut its profile says it has"));
    let reset_button = gtk::Button::with_label("Reset");
    reset_button.set_valign(gtk::Align::Center);
    {
        let shell = Rc::clone(shell);
        reset_button.connect_clicked(move |_| {
            shell.config.borrow_mut().clear_key_bindings();
            shell.save_config();
            reapply_keys();
        });
    }
    reset.add_suffix(&reset_button);
    profile_group.add(&reset);
    page.add(&profile_group);

    let commands = adw::PreferencesGroup::new();
    commands.set_title(&tr("Commands"));
    commands.set_description(Some(
        "An empty field gives the command back the shortcut its profile — or the \
         editor — says it has.",
    ));
    let bindings = {
        let config = shell.config.borrow();
        textchum_core::keys::effective(
            &config.keys_profile(),
            &config.key_profiles_json(),
            &config.keys_json(),
        )
    };
    let mut rows: Vec<(&'static str, adw::EntryRow)> = Vec::new();
    for (action, title) in crate::keyboard::commands() {
        let row = adw::EntryRow::new();
        row.set_title(&tr(title));
        row.set_text(
            bindings
                .get(action)
                .cloned()
                .or_else(|| crate::keyboard::default_spec(action))
                .unwrap_or_default()
                .as_str(),
        );
        row.set_show_apply_button(true);
        let shell = Rc::clone(shell);
        row.connect_apply(move |row| {
            let spec = row.text().trim().to_string();
            shell
                .config
                .borrow_mut()
                .set_key_binding(action, (!spec.is_empty()).then_some(spec.as_str()));
            shell.save_config();
            reapply_keys();
        });
        commands.add(&row);
        rows.push((action, row));
    }
    // Choosing a profile moves the shortcuts; the fields have to say so,
    // or picking one looks like it did nothing.
    let refresh: Rc<dyn Fn()> = {
        let shell = Rc::clone(shell);
        Rc::new(move || {
            let bindings = {
                let config = shell.config.borrow();
                textchum_core::keys::effective(
                    &config.keys_profile(),
                    &config.key_profiles_json(),
                    &config.keys_json(),
                )
            };
            for (action, row) in &rows {
                row.set_text(
                    bindings
                        .get(*action)
                        .cloned()
                        .or_else(|| crate::keyboard::default_spec(action))
                        .unwrap_or_default()
                        .as_str(),
                );
            }
        })
    };
    SHORTCUT_FIELDS.with(|slot| *slot.borrow_mut() = Some(Rc::clone(&refresh)));
    page.add(&commands);
    page
}

thread_local! {
    /// The open Keyboard tab's fields, so that choosing a profile can
    /// put the shortcuts it moved on screen.
    static SHORTCUT_FIELDS: RefCell<Option<Rc<dyn Fn()>>> = const { RefCell::new(None) };
}

/// Puts the shortcuts in force back into the Keyboard tab's fields.
fn refresh_shortcut_fields() {
    let refresh = SHORTCUT_FIELDS.with(|slot| slot.borrow().clone());
    if let Some(refresh) = refresh {
        refresh();
    }
}

/// One line saying whether a configured server command would start,
/// and what to install when it would not.
fn describe_server(language: &str, command: &str, overridden: bool) -> String {
    if command.trim().is_empty() {
        return "not configured".to_string();
    }
    if textchum_lsp::registry::executable_exists(command) {
        if overridden {
            return "found on PATH — overriding the built-in".to_string();
        }
        return "found on PATH".to_string();
    }
    match textchum_lsp::registry::server_for_language(language) {
        Some(spec) => format!("not installed — {}", spec.install_hint),
        None => "not installed".to_string(),
    }
}

/// Re-runs the spell checker over every open document, in every
/// window: the dictionary and the personal word list are application
/// settings, so accepting one word changes what every page shows.
/// The project roots of the documents that are open.
///
/// Adding a per-project entry meant knowing the root and typing it. The
/// roots are known: every open document has one.
fn open_project_roots() -> Vec<String> {
    let mut roots: Vec<String> = Vec::new();
    Workbench::for_each(|workbench| {
        for page in workbench.all_pages() {
            let Some(path) = page.path().borrow().clone() else {
                continue;
            };
            let Some(root) = workspace::project_root_for(Path::new(&path)) else {
                continue;
            };
            let root = root.to_string_lossy().into_owned();
            if !roots.contains(&root) {
                roots.push(root);
            }
        }
    });
    roots.sort();
    roots
}

/// Offers a list of paths on a row that still takes any text.
fn attach_path_choices(row: &adw::EntryRow, tooltip: &str, paths: Vec<String>) {
    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::None);
    list.set_activate_on_single_click(true);
    if paths.is_empty() {
        // An empty list is a gap the reader has to interpret; say why.
        let label = gtk::Label::new(Some("Nothing to offer"));
        label.set_margin_start(8);
        label.set_margin_end(8);
        label.set_margin_top(4);
        label.set_margin_bottom(4);
        label.set_sensitive(false);
        list.append(&label);
    }
    for path in &paths {
        let label = gtk::Label::new(Some(path));
        label.set_xalign(0.0);
        label.set_margin_start(8);
        label.set_margin_end(8);
        label.set_margin_top(4);
        label.set_margin_bottom(4);
        list.append(&label);
    }

    let scroller = gtk::ScrolledWindow::new();
    scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroller.set_max_content_height(320);
    scroller.set_propagate_natural_height(true);
    scroller.set_child(Some(&list));

    let popover = gtk::Popover::new();
    popover.set_child(Some(&scroller));

    let button = gtk::MenuButton::new();
    button.set_icon_name("pan-down-symbolic");
    button.set_tooltip_text(Some(tooltip));
    button.add_css_class("flat");
    button.set_valign(gtk::Align::Center);
    button.set_popover(Some(&popover));

    {
        let row = row.clone();
        let popover = popover.clone();
        list.connect_row_activated(move |_, activated| {
            if let Some(label) = activated.child().and_downcast::<gtk::Label>() {
                if label.is_sensitive() {
                    row.set_text(&label.text());
                }
            }
            popover.popdown();
        });
    }

    row.add_suffix(&button);
}

/// Redraws the occurrence marks everywhere: the settings that decide
/// them just changed.
fn refresh_every_page_occurrences() {
    Workbench::for_each(|workbench| {
        for page in workbench.all_pages() {
            crate::page::refresh_occurrences(&page);
        }
    });
}

fn recheck_every_page() {
    Workbench::for_each(|workbench| {
        for page in workbench.all_pages() {
            crate::spell::run(&page);
        }
    });
}

/// A menu label for text that came from data rather than from us.
///
/// GTK reads `_` in a menu label as the mnemonic marker, so a dictionary
/// called `en_US` appears in the menu as *enUS* with the U underlined,
/// and a file called `my_notes.md` loses its underscore too. The name is
/// still correct everywhere it matters — the action target carries the
/// real string — but the menu tells the reader something false about
/// what the thing is called. Doubling the underscore renders one.
pub fn menu_label(text: &str) -> String {
    text.replace('_', "__")
}

/// A toast built to be read: the text wraps over as many lines as it
/// needs, and it stays up until dismissed.
///
/// AdwToast's own title is a single ellipsized line with a few seconds
/// on the clock — a sentence naming a package to install does not
/// survive either. A custom title widget replaces the label, and a
/// timeout of zero replaces the clock.
pub fn long_toast(text: &str) -> adw::Toast {
    let label = gtk::Label::new(Some(text));
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_xalign(0.0);
    // Without a width to wrap against, the label asks for one very long
    // line and the toast grows past the window instead of wrapping.
    label.set_max_width_chars(60);
    let toast = adw::Toast::new("");
    toast.set_custom_title(Some(&label));
    toast.set_timeout(0);
    toast
}

/// Re-applies the global editor look after a configuration reload:
/// the font-size CSS, and every view's tab width and gutter.
pub fn apply_editor_look(font_size: f64, tab_width: u32, line_numbers: bool) {
    apply_font_size(font_size);
    for_all_views(|view| {
        view.set_tab_width(tab_width);
        view.set_show_line_numbers(line_numbers);
    });
}

/// One app-wide CSS provider carrying the editor font size.
fn apply_font_size(points: f64) {
    thread_local! {
        static PROVIDER: gtk::CssProvider = {
            let provider = gtk::CssProvider::new();
            if let Some(display) = gtk::gdk::Display::default() {
                gtk::style_context_add_provider_for_display(
                    &display,
                    &provider,
                    gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
                );
            }
            provider
        };
    }
    PROVIDER.with(|provider| {
        provider.load_from_string(&format!("textview {{ font-size: {points}pt; }}"));
    });
}

fn for_all_views(apply: impl Fn(&sourceview5::View)) {
    WORKBENCHES.with(|list| {
        for workbench in list.borrow().iter() {
            for page in workbench.pages.borrow().iter() {
                apply(&page.view);
            }
        }
    });
}

/// Offer the languages the build knows on a row that still accepts any
/// text.
///
/// A language field cannot be a closed picker: configuration may name a
/// language this build has no grammar for yet, and typing it must keep
/// working. So the row stays an entry and grows a list beside it.
fn attach_language_choices(row: &adw::EntryRow) {
    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::None);
    list.set_activate_on_single_click(true);
    for name in textchum_core::syntax::languages::selectable_names() {
        let label = gtk::Label::new(Some(name));
        label.set_xalign(0.0);
        label.set_margin_start(8);
        label.set_margin_end(8);
        label.set_margin_top(4);
        label.set_margin_bottom(4);
        list.append(&label);
    }

    let scroller = gtk::ScrolledWindow::new();
    scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroller.set_max_content_height(320);
    scroller.set_propagate_natural_height(true);
    scroller.set_child(Some(&list));

    let popover = gtk::Popover::new();
    popover.set_child(Some(&scroller));

    let button = gtk::MenuButton::new();
    button.set_icon_name("pan-down-symbolic");
    button.set_tooltip_text(Some("Known languages"));
    button.add_css_class("flat");
    button.set_valign(gtk::Align::Center);
    button.set_popover(Some(&popover));

    {
        let row = row.clone();
        let popover = popover.clone();
        list.connect_row_activated(move |_, activated| {
            if let Some(label) = activated.child().and_downcast::<gtk::Label>() {
                row.set_text(&label.text());
            }
            popover.popdown();
        });
    }

    row.add_suffix(&button);
}

/// The records that exist, with what they are about and when they were
/// last written — and the two ways to be rid of them.
///
/// A record holds what the files of a project remember about
/// themselves: how each is split, what is folded, what it was told it
/// is.
fn show_project_records(parent: &adw::PreferencesWindow) {
    let window = adw::Window::new();
    window.set_title(Some(&tr("Project Records")));
    window.set_default_size(560, 420);
    window.set_transient_for(Some(parent));
    window.set_modal(true);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    let page = adw::PreferencesPage::new();
    let group = adw::PreferencesGroup::new();
    group.set_title(&tr("Records"));

    let dir = crate::shell::project_state_dir();
    let records = textchum_core::project_state::records(&dir);
    if records.is_empty() {
        let empty = adw::ActionRow::new();
        empty.set_title(&tr("No project has anything recorded yet"));
        group.add(&empty);
    }
    for record in &records {
        let row = adw::ActionRow::new();
        let name = std::path::Path::new(&record.root)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| record.root.clone());
        row.set_title(&menu_label(&name));
        let when = when_written(record.updated);
        row.set_subtitle(&menu_label(&format!(
            "{}{} · {} files · {}",
            record.root,
            if record.missing { " (missing)" } else { "" },
            record.files,
            when
        )));
        let forget = gtk::Button::from_icon_name("user-trash-symbolic");
        forget.set_valign(gtk::Align::Center);
        forget.set_tooltip_text(Some(&tr("Forget this record")));
        forget.add_css_class("flat");
        {
            let path = record.path.clone();
            let row = row.clone();
            forget.connect_clicked(move |button| {
                if textchum_core::project_state::forget(std::path::Path::new(&path)) {
                    row.set_sensitive(false);
                    button.set_sensitive(false);
                }
            });
        }
        row.add_suffix(&forget);
        group.add(&row);
    }
    page.add(&group);

    let actions = adw::PreferencesGroup::new();
    let sweep_row = adw::ActionRow::new();
    let days = Shell::instance().config.borrow().project_state_keep_days();
    let sweep_title = if days == 0 {
        "Forget the records of projects that are gone".to_string()
    } else {
        format!("Forget records older than {days} days")
    };
    sweep_row.set_title(&sweep_title);
    let sweep = gtk::Button::with_label(&tr("Forget"));
    sweep.set_valign(gtk::Align::Center);
    sweep.add_css_class("destructive-action");
    {
        let window = window.clone();
        let parent = parent.clone();
        sweep.connect_clicked(move |_| {
            let gone = textchum_core::project_state::sweep(
                &crate::shell::project_state_dir(),
                days as u64,
            );
            let _ = gone;
            window.close();
            show_project_records(&parent);
        });
    }
    sweep_row.add_suffix(&sweep);
    actions.add(&sweep_row);
    page.add(&actions);

    toolbar.set_content(Some(&page));
    window.set_content(Some(&toolbar));
    window.present();
}

/// "3 days ago", for a record's own time.
fn when_written(updated: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0);
    let days = now.saturating_sub(updated) / (24 * 60 * 60);
    match days {
        0 => "today".to_string(),
        1 => "yesterday".to_string(),
        _ => format!("{days} days ago"),
    }
}
