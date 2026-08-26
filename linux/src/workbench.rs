//! The workbench window: tabs (AdwTabView), a project file tree in a
//! sidebar, the search bar, the primary menu, preferences, and every
//! action — over pages that each mirror one core document.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use sourceview5::prelude::*;
use textchum_core::{workspace, Appearance};

use crate::page::{self, Page};
use crate::shell::{PageHandles, Shell};

pub struct Workbench {
    pub window: adw::ApplicationWindow,
    pub tab_view: adw::TabView,
    title: adw::WindowTitle,
    toasts: adw::ToastOverlay,
    split: adw::OverlaySplitView,
    buffers_box: gtk::Box,
    tree_box: gtk::Box,
    /// Aligned with the buffer list's rows: None for group headers.
    buffer_rows: RefCell<Vec<Option<Rc<Page>>>>,
    /// (title, dirty, selected) per page — rebuild only on real change.
    buffer_signature: RefCell<Vec<(String, bool, bool)>>,
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
}

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
        let tab_bar = adw::TabBar::new();
        tab_bar.set_view(Some(&tab_view));

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
                Some(name),
                Some(&format!("win.new-with-format('{name}')")),
            );
        }
        file_section.append_submenu(Some("New with Format"), &formats_menu);
        file_section.append(Some("New Window"), Some("win.new"));
        file_section.append(Some("Open…"), Some("win.open"));
        // The desktop's shared recent-files list, newest first.
        let recent_menu = gtk::gio::Menu::new();
        let mut recent: Vec<gtk::RecentInfo> = gtk::RecentManager::default().items();
        recent.sort_by_key(|info| std::cmp::Reverse(info.modified()));
        for info in recent.iter().take(10) {
            let uri = info.uri();
            if let Some(path) = uri.strip_prefix("file://") {
                let path = percent_decode(path);
                if std::path::Path::new(&path).is_file() {
                    let name = info.display_name();
                    recent_menu.append(
                        Some(&name),
                        Some(&format!("win.open-recent('{}')", path.replace('\'', "\\'"))),
                    );
                }
            }
        }
        file_section.append_submenu(Some("Open Recent"), &recent_menu);
        file_section.append(Some("Open Quickly…"), Some("win.quick-open"));
        file_section.append(Some("Save"), Some("win.save"));
        file_section.append(Some("Save As…"), Some("win.save-as"));
        file_section.append(Some("Revert to Saved"), Some("win.revert"));
        let edit_section = gtk::gio::Menu::new();
        edit_section.append(Some("Undo"), Some("win.undo"));
        edit_section.append(Some("Redo"), Some("win.redo"));
        edit_section.append(Some("Find…"), Some("win.find"));
        edit_section.append(Some("Find in Project…"), Some("win.find-in-project"));
        edit_section.append(Some("Run Save Preprocessors"), Some("win.preprocess"));
        edit_section.append(Some("Redraw"), Some("win.redraw"));
        let go_section = gtk::gio::Menu::new();
        go_section.append(Some("Jump to Definition"), Some("win.definition"));
        go_section.append(Some("Go Back"), Some("win.back"));
        go_section.append(Some("Go Forward"), Some("win.forward"));
        go_section.append(Some("Find References"), Some("win.references"));
        go_section.append(Some("Rename Symbol…"), Some("win.rename"));
        go_section.append(Some("Format Document"), Some("win.format"));
        go_section.append(Some("Document Outline…"), Some("win.outline"));
        go_section.append(Some("Show Documentation for Symbol"), Some("win.hover"));
        go_section.append(Some("Go to Block Start"), Some("win.block-start"));
        go_section.append(Some("Go to Block End"), Some("win.block-end"));
        go_section.append(Some("Command Palette…"), Some("win.palette"));
        go_section.append(Some("Language Server Status"), Some("win.server-status"));
        go_section.append(Some("Toggle Path Display"), Some("win.paths"));
        go_section.append(Some("Toggle File Tree"), Some("win.sidebar"));
        go_section.append(Some("Toggle Markdown Preview"), Some("win.preview"));
        let app_section = gtk::gio::Menu::new();
        app_section.append(Some("Preferences…"), Some("win.preferences"));
        app_section.append(Some("About Textchum"), Some("win.about"));
        app_section.append(Some("Close Tab"), Some("win.close-tab"));
        app_section.append(Some("Close Window"), Some("window.close"));
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
        search_entry.set_placeholder_text(Some("Find in file…"));
        search_entry.set_hexpand(true);
        let search_case = gtk::ToggleButton::with_label("Aa");
        search_case.set_tooltip_text(Some("Match case"));
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
        replace_entry.set_placeholder_text(Some("Replace with…"));
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
        content_box.append(&tab_bar);
        content_box.append(&tab_view);
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
            title,
            toasts,
            split,
            buffers_box,
            tree_box,
            buffer_rows: RefCell::new(Vec::new()),
            buffer_signature: RefCell::new(Vec::new()),
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
        });
        WORKBENCHES.with(|list| list.borrow_mut().push(Rc::clone(&workbench)));

        // Selected-page switches drive the chrome and the sidebar.
        {
            let workbench = Rc::downgrade(&workbench);
            tab_view.connect_selected_page_notify(move |_| {
                if let Some(workbench) = workbench.upgrade() {
                    workbench.refresh_chrome();
                    workbench.refresh_sidebar();
                    workbench.apply_search_options();
                }
            });
        }
        // Closing a tab retires its document from the pool.
        {
            let workbench = Rc::downgrade(&workbench);
            tab_view.connect_close_page(move |view, tab_page| {
                if let Some(workbench) = workbench.upgrade() {
                    workbench.forget(tab_page);
                }
                view.close_page_finish(tab_page, true);
                glib::Propagation::Stop
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
                    Ok(()) => workbench.toast("Replaced every occurrence."),
                    Err(error) => workbench.toast(&format!("Replace failed: {error}")),
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
            .find(|page| page.path.borrow().as_deref() == Some(path))
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
        let path = page.path.borrow().clone()?;
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

    pub fn selected(&self) -> Option<Rc<Page>> {
        let selected = self.tab_view.selected_page()?;
        self.pages
            .borrow()
            .iter()
            .find(|page| {
                self.tab_view.page(&page.root).as_ptr() == selected.as_ptr()
            })
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
        let tab_page = self.tab_view.append(&page.root);
        tab_page.set_title(&page.display_name());
        self.pages.borrow_mut().push(Rc::clone(&page));
        if let Some(path) = page.path.borrow().clone() {
            let handles = Rc::new(PageHandles {
                window: self.window.clone(),
                tab_view: self.tab_view.clone(),
                tab_page: tab_page.clone(),
                buffer: page.buffer.clone(),
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
        self.tab_view.set_selected_page(&tab_page);
        if let Some((line, character)) = at {
            if let Some(path) = page.path.borrow().clone() {
                if let Some(handles) = Shell::instance().pages.borrow().get(&path).cloned() {
                    page::reveal(&handles, line, character);
                }
            }
        }
        self.refresh_chrome();
        self.refresh_sidebar();
        // The desktop's shared recent-files list learns about it too.
        if let Some(path) = self.selected().and_then(|page| page.path.borrow().clone()) {
            let uri = gtk::gio::File::for_path(&path).uri();
            let _ = gtk::RecentManager::default().add_item(&uri);
        }
        crate::session::save();
    }

    fn forget(&self, tab_page: &adw::TabPage) {
        let mut pages = self.pages.borrow_mut();
        if let Some(index) = pages
            .iter()
            .position(|page| self.tab_view.page(&page.root).as_ptr() == tab_page.as_ptr())
        {
            let page = pages.remove(index);
            let path = page.path.borrow().clone();
            if let Some(path) = path {
                Shell::instance().pages.borrow_mut().remove(&path);
                Shell::instance()
                    .pool
                    .borrow_mut()
                    .did_close(Path::new(&path));
            }
            drop(pages);
            crate::session::save();
        }
    }

    // MARK: Chrome

    /// The page's display name, with enough parent path to tell it
    /// apart when another open file shares its bare name.
    pub fn disambiguated_name(&self, page: &Rc<Page>) -> String {
        if self.show_full_paths.get() {
            if let Some(path) = page.path.borrow().clone() {
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
        let Some(path) = page.path.borrow().clone() else { return name };
        Path::new(&path)
            .parent()
            .and_then(Path::file_name)
            .map(|parent| format!("{}/{name}", parent.to_string_lossy()))
            .unwrap_or(name)
    }

    pub fn refresh_chrome(&self) {
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
        if let Some(tab_page) = self.tab_view.selected_page() {
            tab_page.set_title(&format!("{dirty}{shown}"));
        }
        let detail = format!(
            "{} · {} bytes",
            state.document.encoding().name(),
            state.document.len_bytes()
        );
        drop(state);
        if let Some(path) = page.path.borrow().clone() {
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
        let heading = gtk::Label::new(Some("Open Files"));
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
                .path
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
                    let tab_page = workbench.tab_view.page(&page.root);
                    workbench.tab_view.set_selected_page(&tab_page);
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
                let Some(path) = page.and_then(|page| page.path.borrow().clone()) else {
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
            page.path
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
fn build_file_tree(root: &Path) -> gtk::ListView {
    fn children_of(path: &Path) -> gtk::gio::ListStore {
        let store = gtk::gio::ListStore::new::<gtk::StringObject>();
        let mut entries: Vec<(bool, String)> = std::fs::read_dir(path)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') {
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

    let root_model = children_of(root);
    let tree_model = gtk::TreeListModel::new(root_model, false, false, |item| {
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
                // The desktop's own icon for the file's content type,
                // guessed from the name — the system icons the Mac
                // shell gets from NSWorkspace.
                let (content_type, _) = gtk::gio::content_type_guess(
                    Some(std::path::Path::new(&path)),
                    &[],
                );
                let themed = gtk::gio::content_type_get_symbolic_icon(&content_type);
                icon.set_from_gicon(&themed);
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
        dialog.open(
            Some(&workbench.window.clone()),
            gtk::gio::Cancellable::NONE,
            move |result| {
                if let Ok(file) = result {
                    if let Some(path) = file.path() {
                        workbench.open(Some(path), None);
                    }
                }
            },
        );
    });
    add("save", workbench, |workbench, _| {
        let Some(page) = workbench.selected() else { return };
        let workbench = Rc::clone(workbench);
        preprocess_gate(&workbench.clone(), &page, move |workbench, page| {
            let saved = page.state.borrow_mut().document.save().is_ok();
            if saved {
                if let Some(path) = page.path.borrow().as_deref() {
                    Shell::instance().note_own_save(path);
                }
                workbench.refresh_chrome();
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
            .path
            .borrow()
            .clone()
            .or_else(|| {
                workbench
                    .all_pages()
                    .iter()
                    .find_map(|other| other.path.borrow().clone())
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
            None => workbench.toast("Not a Markdown document."),
        }
    });
    add("sidebar", workbench, |workbench, _| {
        workbench
            .split
            .set_show_sidebar(!workbench.split.shows_sidebar());
    });
    add("close-tab", workbench, |workbench, _| {
        if let Some(selected) = workbench.tab_view.selected_page() {
            workbench.tab_view.close_page(&selected);
        }
    });
    add("definition", workbench, |workbench, _| {
        let Some(page) = workbench.selected() else { return };
        let Some(path) = page.path.borrow().clone() else {
            workbench.toast("Save the file first — untitled documents have no server.");
            return;
        };
        let (line, character) = page::lsp_caret(&page.buffer);
        let shell = Shell::instance();
        let id = shell
            .pool
            .borrow_mut()
            .definition(Path::new(&path), line, character);
        if id == 0 {
            // No server: the ctags fallback gets its chance before the
            // explanation does.
            if !ctags_jump(workbench, &page, &path) {
                workbench.toast("No language server is running for this document.");
            }
            return;
        }
        let weak = Rc::downgrade(workbench);
        let fallback_page = Rc::clone(&page);
        shell.expect_response(id, move |json| {
            if let Some(workbench) = weak.upgrade() {
                if !open_definition(&workbench, json) {
                    // The server answered but had nothing; consult the
                    // index for projects that opted in.
                    if !ctags_jump(&workbench, &fallback_page, &path) {
                        workbench.toast("No definition found.");
                    }
                }
            }
        });
    });
    add("find-in-project", workbench, |workbench, _| {
        let root = workbench
            .selected()
            .and_then(|page| {
                page.path.borrow().as_deref().map(Path::new).and_then(|path| {
                    workspace::project_root_for(path)
                        .or_else(|| path.parent().map(Path::to_owned))
                })
            })
            .unwrap_or_else(glib::home_dir);
        show_grep(workbench, root);
    });
    add("quick-open", workbench, |workbench, _| {
        let root = workbench
            .selected()
            .and_then(|page| {
                page.path.borrow().as_deref().map(Path::new).and_then(|path| {
                    workspace::project_root_for(path)
                        .or_else(|| path.parent().map(Path::to_owned))
                })
            })
            .unwrap_or_else(glib::home_dir);
        show_quick_open(workbench, root);
    });
    add("preferences", workbench, |workbench, _| {
        show_preferences(&workbench.window);
    });
    add("about", workbench, |workbench, _| {
        // The real build version comes from git at compile time (the
        // tag in CI); the rest is the who/where/under-what.
        let about = adw::AboutWindow::builder()
            .transient_for(&workbench.window)
            .application_name("Textchum")
            .application_icon("to.perri.textchum")
            .version(env!("TEXTCHUM_BUILD_VERSION"))
            .comments(
                "A text editor in the spirit of TextMate: native, fast, and                  focused on editing.",
            )
            .developer_name("Horacio Duran")
            .website("https://perri.to")
            .issue_url("https://github.com/perrito666/textchum/issues")
            .license_type(gtk::License::MitX11)
            .build();
        about.add_link("Repository", "https://github.com/perrito666/textchum");
        about.present();
    });
    add("block-start", workbench, |workbench, _| move_to_block_edge(workbench, true));
    add("block-end", workbench, |workbench, _| move_to_block_edge(workbench, false));
    add("palette", workbench, |workbench, _| show_palette(workbench));
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
                    None => workbench.toast("Not in a git repository with a remote."),
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
            workbench.toast("No save preprocessors configured for this language.");
            return;
        }
        if let Err(failure) = run_preprocessor_chain(&page) {
            workbench.toast(&format!(
                "Preprocessor failed: {} — {}",
                failure.command, failure.details
            ));
        }
    });
    add("revert", workbench, |workbench, _| {
        let Some(page) = workbench.selected() else { return };
        if page.path.borrow().is_none() {
            workbench.toast("Untitled documents have no file to revert to.");
            return;
        }
        if !page.state.borrow().document.is_dirty() {
            page.reload_from_disk();
            return;
        }
        let dialog = adw::AlertDialog::new(
            Some("Revert to Saved?"),
            Some("Local changes will be replaced with the file on disk."),
        );
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("revert", "Revert");
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
            None => workbench.toast("Nowhere to go back to."),
        }
    });
    add("forward", workbench, |workbench, _| {
        let target = workbench.jumps.borrow_mut().forward();
        match target {
            Some((path, line, character)) => {
                jump_without_trail(workbench, &path, line, character);
            }
            None => workbench.toast("Nowhere to go forward to."),
        }
    });
    add("references", workbench, |workbench, _| {
        let Some((page, path)) = workbench
            .selected()
            .and_then(|page| {
                let path = page.path.borrow().clone();
                path.map(|path| (page, path))
            })
        else {
            workbench.toast("Save the file first — untitled documents have no server.");
            return;
        };
        let (line, character) = page::lsp_caret(&page.buffer);
        let shell = Shell::instance();
        let id = shell
            .pool
            .borrow_mut()
            .references(Path::new(&path), line, character);
        if id == 0 {
            workbench.toast("No language server is running for this document.");
            return;
        }
        let weak = Rc::downgrade(workbench);
        shell.expect_response(id, move |json| {
            if let Some(workbench) = weak.upgrade() {
                show_locations(&workbench, "References", json);
            }
        });
    });
    add("rename", workbench, |workbench, _| {
        let Some((page, path)) = workbench
            .selected()
            .and_then(|page| {
                let path = page.path.borrow().clone();
                path.map(|path| (page, path))
            })
        else {
            workbench.toast("Save the file first — untitled documents have no server.");
            return;
        };
        let (line, character) = page::lsp_caret(&page.buffer);
        let dialog = adw::AlertDialog::new(Some("Rename Symbol"), None);
        let entry = gtk::Entry::new();
        entry.set_placeholder_text(Some("New name"));
        dialog.set_extra_child(Some(&entry));
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("rename", "Rename");
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
                    workbench.toast("No language server is running for this document.");
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
        let _ = page;
    });
    add("format", workbench, |workbench, _| {
        let Some((page, path)) = workbench
            .selected()
            .and_then(|page| {
                let path = page.path.borrow().clone();
                path.map(|path| (page, path))
            })
        else {
            workbench.toast("Save the file first — untitled documents have no server.");
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
            workbench.toast("No language server is running for this document.");
            return;
        }
        let weak = Rc::downgrade(workbench);
        shell.expect_response(id, move |json| {
            let Some(workbench) = weak.upgrade() else { return };
            let edits = crate::lsp_edits::text_edits(json);
            if edits.is_empty() {
                workbench.toast("The server had no formatting to offer.");
                return;
            }
            let Some(page) = workbench.page_for(&path) else { return };
            apply_edits_to_page(&page, edits);
            workbench.refresh_chrome();
        });
    });
    add("outline", workbench, |workbench, _| {
        let Some(path) = workbench
            .selected()
            .and_then(|page| page.path.borrow().clone())
        else {
            workbench.toast("Save the file first — untitled documents have no server.");
            return;
        };
        let shell = Shell::instance();
        let id = shell.pool.borrow_mut().document_symbols(Path::new(&path));
        if id == 0 {
            workbench.toast("No language server is running for this document.");
            return;
        }
        let weak = Rc::downgrade(workbench);
        shell.expect_response(id, move |json| {
            if let Some(workbench) = weak.upgrade() {
                show_outline(&workbench, &path, json);
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
}

/// Parses a definition result (Location, Location[], or LocationLink[])
/// and navigates there. Returns whether there was anywhere to go — the
/// caller decides between the ctags fallback and an explanation.
fn open_definition(workbench: &Rc<Workbench>, json: &str) -> bool {
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(json) else {
        return false;
    };
    let candidate = match &parsed {
        serde_json::Value::Array(items) => items.first().cloned(),
        serde_json::Value::Object(_) => Some(parsed.clone()),
        _ => None,
    };
    let Some(candidate) = candidate else { return false };
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
        return false;
    };
    // Servers percent-encode paths (a space is %20); decode before use.
    let path = percent_decode(path);
    let line = range["start"]["line"].as_i64().unwrap_or(0) as i32;
    let character = range["start"]["character"].as_u64().unwrap_or(0) as usize;
    workbench.open(Some(PathBuf::from(path)), Some((line, character)));
    true
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
                "The ctags fallback needs Universal Ctags — install the                  universal-ctags package.",
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
                Some(&format!("Move to “{title}”")),
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
    let tab_page = source.tab_view.page(&page.root);
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
            buffer: page.buffer.clone(),
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
        workbench.toast("No enclosing block here.");
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
    ("New Window", "win.new"),
    ("Open…", "win.open"),
    ("Open Quickly…", "win.quick-open"),
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
    ("Go to Block Start", "win.block-start"),
    ("Go to Block End", "win.block-end"),
    ("Language Server Status", "win.server-status"),
    ("Toggle File Tree", "win.sidebar"),
    ("Toggle Markdown Preview", "win.preview"),
    ("Preferences…", "win.preferences"),
    ("About Textchum", "win.about"),
    ("Close Tab", "win.close-tab"),
];

/// A fuzzy-filterable list over every menu action; ⏎ runs the
/// selection. The shortcut for when the shortcut escapes memory.
fn show_palette(workbench: &Rc<Workbench>) {
    let entry = gtk::SearchEntry::new();
    entry.set_placeholder_text(Some("Type a command…"));
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
            lines.push("Full trail: ~/.local/state/textchum/lsp.log".into());
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
    let previous = page.path.borrow().clone();
    if page.state.borrow_mut().document.save_as(path).is_err() {
        workbench.toast("Could not save the document.");
        return false;
    }
    let key = path.to_string_lossy().into_owned();
    let shell = Shell::instance();
    shell.note_own_save(&key);
    *page.path.borrow_mut() = Some(key.clone());
    if let Some(previous) = previous.filter(|previous| *previous != key) {
        shell.pages.borrow_mut().remove(&previous);
        shell.pool.borrow_mut().did_close(Path::new(&previous));
    }
    page::refresh_style_tags(&page.buffer);
    page::recolor(&page.buffer);
    page::apply_highlights(&page.buffer, &page.state.borrow().document);
    let tab_page = workbench.tab_view.page(&page.root);
    let handles = Rc::new(crate::shell::PageHandles {
        window: workbench.window.clone(),
        tab_view: workbench.tab_view.clone(),
        tab_page,
        buffer: page.buffer.clone(),
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

/// The configured preprocessor chain for a page, if any.
fn preprocessor_chain(page: &Rc<Page>) -> Option<(Vec<String>, Option<PathBuf>, String)> {
    let path = page.path.borrow().clone()?;
    let language = page.state.borrow().document.language_name()?;
    let root = workspace::project_root_for(Path::new(&path));
    let root_string = root.as_deref().map(|r| r.to_string_lossy().into_owned());
    let commands = Shell::instance()
        .config
        .borrow()
        .preprocessor_commands(root_string.as_deref(), language);
    if commands.is_empty() {
        return None;
    }
    Some((commands, root, path))
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
            dialog.add_response("cancel", "Cancel");
            dialog.add_response("save", "Save Without Preprocessing");
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
        workbench.toast("No references found.");
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
        workbench.toast("No references found.");
        return;
    }
    present_picker(
        workbench,
        title,
        rows.iter()
            .map(|(path, line, _)| {
                let name = Path::new(path)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.clone());
                format!("{name}:{}", line + 1)
            })
            .collect(),
        move |workbench, index| {
            let (path, line, character) = rows[index].clone();
            workbench.open(Some(PathBuf::from(path)), Some((line, character)));
        },
    );
}

/// The document's symbols, flattened; activating a row jumps.
fn show_outline(workbench: &Rc<Workbench>, path: &str, json: &str) {
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(json) else {
        workbench.toast("The server offered no outline.");
        return;
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
        workbench.toast("The server offered no outline.");
        return;
    }
    let path = path.to_owned();
    let labels: Vec<String> = rows.iter().map(|(label, _, _)| label.clone()).collect();
    present_picker(workbench, "Document Outline", labels, move |workbench, index| {
        let (_, line, character) = rows[index];
        workbench.open(Some(PathBuf::from(&path)), Some((line, character)));
    });
}

/// A small modal list; activating a row runs `choose` and closes.
fn present_picker(
    workbench: &Rc<Workbench>,
    title: &str,
    labels: Vec<String>,
    choose: impl Fn(&Rc<Workbench>, usize) + 'static,
) {
    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::Browse);
    for label in &labels {
        let row_label = gtk::Label::new(Some(label));
        row_label.set_xalign(0.0);
        row_label.set_margin_start(10);
        row_label.set_margin_end(10);
        row_label.set_margin_top(4);
        row_label.set_margin_bottom(4);
        list.append(&row_label);
    }
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
        workbench.toast("The server had no rename to offer.");
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
        .transient_for(&workbench.window)
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
    entry.set_placeholder_text(Some("regular expression…"));
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
    pattern.set_placeholder_text(Some("filter text…"));
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

// MARK: Preferences

/// The preferences window, over the same config.json contract as
/// everywhere else: every change applies immediately and saves.
fn show_preferences(parent: &adw::ApplicationWindow) {
    let shell = Shell::instance();
    let window = adw::PreferencesWindow::new();
    window.set_transient_for(Some(parent));
    window.set_modal(false);
    window.set_title(Some("Preferences"));

    let general = adw::PreferencesPage::new();
    general.set_title("General");
    general.set_icon_name(Some("preferences-system-symbolic"));

    let appearance_group = adw::PreferencesGroup::new();
    appearance_group.set_title("Appearance");
    let appearance_row = adw::ComboRow::new();
    appearance_row.set_title("Appearance");
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
    theme_row.set_title("Theme");
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
    general.add(&appearance_group);

    let editor_group = adw::PreferencesGroup::new();
    editor_group.set_title("Editor");
    let font_row = adw::SpinRow::with_range(6.0, 72.0, 1.0);
    font_row.set_title("Font size");
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
    tab_row.set_title("Tab width");
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
    lines_row.set_title("Show line numbers");
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
    let hover_row = adw::SwitchRow::new();
    hover_row.set_title("Hover documentation");
    hover_row.set_subtitle("Show server documentation when the mouse rests on a symbol");
    hover_row.set_active(shell.config.borrow().hover_docs());
    {
        let shell = Rc::clone(&shell);
        hover_row.connect_active_notify(move |row| {
            shell.config.borrow_mut().set_hover_docs(row.is_active());
            shell.save_config();
        });
    }
    editor_group.add(&hover_row);
    let spell_row = adw::EntryRow::new();
    spell_row.set_title("Spell check prose (off, auto, or a dictionary like es_ES)");
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
        "Checks comments in code, and everything in Markdown, git commit          messages, and plain text. Needs hunspell and its dictionaries.",
    ));
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
            for handles in Shell::instance().pages.borrow().values() {
                let _ = handles;
            }
            // Re-check every open page under the new choice.
            Workbench::for_each(|workbench| {
                for page in workbench.all_pages() {
                    crate::spell::run(&page);
                }
            });
        });
    }
    editor_group.add(&spell_row);
    general.add(&editor_group);
    window.add(&general);

    // Language servers: the defaults section of the same lsp config the
    // macOS Settings edit — language → command line, applied to the
    // pool for servers started afterwards.
    let servers = adw::PreferencesPage::new();
    servers.set_title("Language Servers");
    servers.set_icon_name(Some("network-workgroup-symbolic"));
    let servers_group = adw::PreferencesGroup::new();
    servers_group.set_title("Default server commands");
    servers_group.set_description(Some(
        "Override which command serves a language (for every project). \
         Unlisted languages use the built-in registry.",
    ));

    let existing: Vec<(String, String)> = serde_json::from_str::<serde_json::Value>(
        &shell.config.borrow().lsp_json(),
    )
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
    for (language, command) in existing {
        let row = adw::EntryRow::new();
        row.set_title(&language);
        row.set_text(&command);
        let shell = Rc::clone(&shell);
        let language = language.clone();
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
        });
        row.set_show_apply_button(true);
        servers_group.add(&row);
    }

    let projects_group = adw::PreferencesGroup::new();
    projects_group.set_title("Per-project overrides");
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
    add_root.set_title("project root path");
    let add_project_language = adw::EntryRow::new();
    add_project_language.set_title("language");
    let add_project_command = adw::EntryRow::new();
    add_project_command.set_title("command");
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
    add_language.set_title("language (e.g. python)");
    let add_command = adw::EntryRow::new();
    add_command.set_title("command (e.g. pylsp)");
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
    preprocessors_group.set_title("Save preprocessors");
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
    add_preprocessor_root.set_title("project root (empty = default for all projects)");
    let add_preprocessor_language = adw::EntryRow::new();
    add_preprocessor_language.set_title("language (e.g. python)");
    let add_preprocessor_chain = adw::EntryRow::new();
    add_preprocessor_chain.set_title("commands (e.g. ruff check --fix - ;; black -)");
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
    projects_page.set_title("Projects");
    projects_page.set_icon_name(Some("folder-symbolic"));
    let workspace_defaults = adw::PreferencesGroup::new();
    workspace_defaults.set_title("Defaults (all projects)");
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
    projects_page.add(&workspace_defaults);

    let workspace_overrides = adw::PreferencesGroup::new();
    workspace_overrides.set_title("Per-project overrides");
    if let Some(projects) = workspace["projects"].as_object() {
        for (root, flags) in projects {
            let expander = adw::ExpanderRow::new();
            let basename = std::path::Path::new(root)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| root.clone());
            expander.set_title(&basename);
            expander.set_subtitle(root);
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
        }
    }
    let add_workspace_root = adw::EntryRow::new();
    add_workspace_root.set_title("add project root path");
    add_workspace_root.set_show_apply_button(true);
    {
        let shell = Rc::clone(&shell);
        add_workspace_root.connect_apply(move |row| {
            let root = row.text().trim().to_string();
            if root.is_empty() {
                return;
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
        });
    }
    workspace_overrides.add(&add_workspace_root);
    projects_page.add(&workspace_overrides);
    window.add(&projects_page);

    window.present();
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
