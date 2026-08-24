//! The workbench window: tabs (AdwTabView), a project file tree in a
//! sidebar, the search bar, the primary menu, preferences, and every
//! action — over pages that each mirror one core document.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use sourceview5::prelude::*;
use textchum_core::{theme, workspace, Appearance};

use crate::page::{self, Page};
use crate::shell::{PageHandles, Shell};

pub struct Workbench {
    pub window: adw::ApplicationWindow,
    pub tab_view: adw::TabView,
    title: adw::WindowTitle,
    toasts: adw::ToastOverlay,
    split: adw::OverlaySplitView,
    sidebar_box: gtk::Box,
    search_bar: gtk::SearchBar,
    search_entry: gtk::SearchEntry,
    pages: RefCell<Vec<Rc<Page>>>,
    /// The root the sidebar currently shows.
    sidebar_root: RefCell<Option<PathBuf>>,
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
        go_section.append(Some("Toggle File Tree"), Some("win.sidebar"));
        let app_section = gtk::gio::Menu::new();
        app_section.append(Some("Preferences…"), Some("win.preferences"));
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
        let search_bar = gtk::SearchBar::new();
        search_bar.set_child(Some(&search_entry));
        search_bar.connect_entry(&search_entry);

        let content_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content_box.append(&tab_bar);
        content_box.append(&tab_view);
        tab_view.set_vexpand(true);

        // Sidebar: the project file tree of the selected page.
        let sidebar_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        sidebar_box.set_width_request(220);
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
            sidebar_box,
            search_bar,
            search_entry: search_entry.clone(),
            pages: RefCell::new(Vec::new()),
            sidebar_root: RefCell::new(None),
        });
        WORKBENCHES.with(|list| list.borrow_mut().push(Rc::clone(&workbench)));

        // Selected-page switches drive the chrome and the sidebar.
        {
            let workbench = Rc::downgrade(&workbench);
            tab_view.connect_selected_page_notify(move |_| {
                if let Some(workbench) = workbench.upgrade() {
                    workbench.refresh_chrome();
                    workbench.refresh_sidebar();
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
        install_actions(app, &workbench);
        workbench
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

    pub fn selected(&self) -> Option<Rc<Page>> {
        let selected = self.tab_view.selected_page()?;
        self.pages
            .borrow()
            .iter()
            .find(|page| {
                self.tab_view.page(&page.scrolled).as_ptr() == selected.as_ptr()
            })
            .cloned()
    }

    /// Opens `path` as a tab (or focuses the tab already showing it,
    /// anywhere), then optionally reveals a position.
    pub fn open(self: &Rc<Self>, path: Option<PathBuf>, at: Option<(i32, usize)>) {
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
        let tab_page = self.tab_view.append(&page.scrolled);
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
    }

    fn forget(&self, tab_page: &adw::TabPage) {
        let mut pages = self.pages.borrow_mut();
        if let Some(index) = pages
            .iter()
            .position(|page| self.tab_view.page(&page.scrolled).as_ptr() == tab_page.as_ptr())
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
        }
    }

    // MARK: Chrome

    pub fn refresh_chrome(&self) {
        let Some(page) = self.selected() else {
            self.title.set_title("Textchum");
            self.title.set_subtitle("");
            return;
        };
        let state = page.state.borrow();
        let dirty = if state.document.is_dirty() { "● " } else { "" };
        self.title
            .set_title(&format!("{dirty}{}", page.display_name()));
        if let Some(tab_page) = self.tab_view.selected_page() {
            tab_page.set_title(&format!("{dirty}{}", page.display_name()));
        }
        drop(state);
        if let Some(path) = page.path.borrow().clone() {
            if let Some(handles) = Shell::instance().pages.borrow().get(&path) {
                refresh_subtitle(handles);
                return;
            }
        }
        self.title.set_subtitle("");
    }

    // MARK: Sidebar (project file tree)

    pub fn refresh_sidebar(&self) {
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
        while let Some(child) = self.sidebar_box.first_child() {
            self.sidebar_box.remove(&child);
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
        self.sidebar_box.append(&header);

        let tree = build_file_tree(&root);
        let scrolled = gtk::ScrolledWindow::builder().child(&tree).vexpand(true).build();
        self.sidebar_box.append(&scrolled);
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
        let language = handles.language.borrow();
        let problems = handles.problems.borrow();
        let subtitle = match (language.is_empty(), problems.is_empty()) {
            (false, false) => format!("{language} · {problems}"),
            (false, true) => language.clone(),
            (true, false) => problems.clone(),
            (true, true) => String::new(),
        };
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
            icon.set_icon_name(Some(if path.is_dir() {
                "folder-symbolic"
            } else {
                "text-x-generic-symbolic"
            }));
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
        let saved = page.state.borrow_mut().document.save().is_ok();
        if saved {
            workbench.refresh_chrome();
        } else {
            let _ = gtk::prelude::WidgetExt::activate_action(
                &workbench.window,
                "win.save-as",
                None,
            );
        }
    });
    add("save-as", workbench, |workbench, _| {
        let Some(page) = workbench.selected() else { return };
        let dialog = gtk::FileDialog::new();
        let workbench = Rc::clone(workbench);
        dialog.save(
            Some(&workbench.window.clone()),
            gtk::gio::Cancellable::NONE,
            move |result| {
                if let Ok(file) = result {
                    if let Some(path) = file.path() {
                        let _ = page.state.borrow_mut().document.save_as(&path);
                        *page.path.borrow_mut() =
                            Some(path.to_string_lossy().into_owned());
                        workbench.refresh_chrome();
                        workbench.refresh_sidebar();
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
    add("definition", workbench, |workbench, app| {
        let Some(page) = workbench.selected() else { return };
        let Some(path) = page.path.borrow().clone() else { return };
        let (line, character) = page::lsp_caret(&page.buffer);
        let shell = Shell::instance();
        let id = shell
            .pool
            .borrow_mut()
            .definition(Path::new(&path), line, character);
        let app = app.clone();
        shell.expect_response(id, move |json| open_definition(&app, json));
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
/// and navigates there.
fn open_definition(_app: &adw::Application, json: &str) {
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
    if let Some(workbench) = Workbench::active() {
        workbench.open(Some(PathBuf::from(path)), Some((line, character)));
    }
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
    let names: Vec<&str> = theme::builtin_names().collect();
    let theme_model = gtk::StringList::new(&names);
    theme_row.set_model(Some(&theme_model));
    let current = shell.config.borrow().theme();
    if let Some(index) = names.iter().position(|name| *name == current) {
        theme_row.set_selected(index as u32);
    }
    {
        let shell = Rc::clone(&shell);
        let names: Vec<String> = names.iter().map(|s| s.to_string()).collect();
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
    window.add(&servers);

    window.present();
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
