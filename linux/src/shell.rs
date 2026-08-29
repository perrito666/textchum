//! The shell's app-wide machinery: the configuration (the same
//! `config.json` contract as everywhere else — GUI-managed, hand
//! editable, broken files never clobbered), one language-server pool
//! for every window, an event pump that marshals server events from
//! their threads onto the GTK main loop, a registry of open pages by
//! path, and the response router for request/reply traffic.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, TryRecvError};

use adw::prelude::*;
use gtk::glib;
use textchum_core::{theme, Config, Event};
use textchum_lsp::Pool;

/// Everything the pump needs to reach one open document's page.
/// A document's identity: stable, and independent of where it is shown
/// or whether it has a path yet.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct DocumentId(u64);

/// One open document — what the file is, not where it is shown.
///
/// Views come and go; this is what they are views of. Everything here
/// belongs to the document, so two views of one file cannot disagree
/// about it: they share the buffer, so they share the text, the
/// history and every edit.
pub struct OpenDocument {
    pub id: DocumentId,
    /// The text every view of this document shares.
    pub buffer: sourceview5::Buffer,
    /// The core's document: text, history, syntax.
    pub state: Rc<RefCell<crate::page::State>>,
    /// Where it lives on disk, once it lives anywhere.
    pub path: RefCell<Option<String>>,
    /// Watches that file for changes made elsewhere.
    pub monitor: RefCell<Option<gtk::gio::FileMonitor>>,
    /// The line ranges folded away, by the line each one opens. Folding
    /// a function folds it in every view of the file.
    pub folded: RefCell<Vec<(i32, i32)>>,
    /// What the server last said about it, kept so it can be read as
    /// well as underlined. An underline nobody can read is a
    /// notification with the message taken out.
    pub diagnostics: RefCell<Vec<Diagnostic>>,
}

/// Where a document is shown: the widgets around one view of it.
///
/// The document half is [`OpenDocument`], reached through `document`.
/// What is left here is the window, the tab and the chrome — the
/// things that are about this showing of it rather than about the file.
pub struct PageHandles {
    pub window: adw::ApplicationWindow,
    pub tab_view: adw::TabView,
    pub tab_page: adw::TabPage,
    pub view: sourceview5::View,
    pub toasts: adw::ToastOverlay,
    pub title: adw::WindowTitle,
    /// "language" and "N errors" halves of the subtitle.
    pub language: RefCell<String>,
    pub problems: RefCell<String>,
    /// "encoding · size" half, refreshed with the chrome.
    pub detail: RefCell<String>,
    /// The document this is a view of.
    pub document: Rc<OpenDocument>,
}

/// One finding, as a balloon needs it.
#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub line: i32,
    pub character: usize,
    pub end_line: i32,
    pub end_character: usize,
    /// 1 = error, 2 = warning, 3 = information, 4 = hint.
    pub severity: u64,
    pub message: String,
}

impl Diagnostic {
    /// What kind of finding it is, in words. The gutter says it in
    /// colour; a balloon has to say it too, or a warning reads like an
    /// error.
    pub fn kind(&self) -> &'static str {
        match self.severity {
            1 => "Error",
            2 => "Warning",
            3 => "Information",
            4 => "Hint",
            _ => "Diagnostic",
        }
    }
}

pub struct Shell {
    pub config: RefCell<Config>,
    pub pool: RefCell<Pool>,
    events: RefCell<Receiver<Event>>,
    pub pages: RefCell<HashMap<String, Rc<PageHandles>>>,
    callbacks: RefCell<HashMap<u64, Box<dyn FnOnce(&str)>>>,
    /// Paths this process just wrote, so the file monitor can tell the
    /// app's own saves from external changes.
    own_saves: RefCell<HashMap<String, std::time::Instant>>,
    /// When this process last wrote config.json, for the same reason.
    own_config_save: std::cell::Cell<Option<std::time::Instant>>,
    /// The last hundred server status transitions, oldest first:
    /// (when, server, root, "status — message").
    pub status_log: RefCell<Vec<(std::time::SystemTime, String, String, String)>>,
    /// Keeps the config-file monitor alive.
    config_monitor: RefCell<Option<gtk::gio::FileMonitor>>,
    /// Every open document, by id, and the paths that name them. A
    /// document is here once however many views show it.
    documents: RefCell<HashMap<DocumentId, Rc<OpenDocument>>>,
    documents_by_path: RefCell<HashMap<String, DocumentId>>,
    next_document_id: std::cell::Cell<u64>,
}

thread_local! {
    static SHELL: RefCell<Option<Rc<Shell>>> = const { RefCell::new(None) };
}

/// `~/.config/textchum/config.json` — the Linux home of the same file.
/// `--data-dir` moves it, along with the rest of the profile; see
/// [`crate::paths`].
pub fn config_path() -> PathBuf {
    crate::paths::config_path()
}

/// User theme JSON files, one per theme, named by their file stem.
pub fn themes_dir() -> PathBuf {
    crate::paths::themes_dir()
}

/// Every selectable theme name: the built-ins, then user files that do
/// not shadow one.
pub fn theme_names() -> Vec<String> {
    let mut names: Vec<String> = theme::builtin_names().map(str::to_owned).collect();
    if let Ok(entries) = std::fs::read_dir(themes_dir()) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "json") {
                if let Some(stem) = path.file_stem().map(|s| s.to_string_lossy().into_owned()) {
                    if !names.contains(&stem) {
                        names.push(stem);
                    }
                }
            }
        }
    }
    names
}

impl Shell {
    /// The process-wide shell, started on first use — which loads the
    /// configuration, applies it, and arms the 50 ms pump that drains
    /// server events into the main loop.
    pub fn instance() -> Rc<Shell> {
        SHELL.with(|cell| {
            if let Some(shell) = cell.borrow().as_ref() {
                return Rc::clone(shell);
            }
            let (config, warning) = Config::load(&config_path());
            if let Some(warning) = warning {
                eprintln!("textchum: {warning}");
            }
            // The same debug log the macOS shell keeps, at the Linux
            // conventional spot.
            let log_path = crate::paths::lsp_log_path();
            if let Some(parent) = log_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            textchum_lsp::log::set_path(&log_path);
            let (sender, receiver) = std::sync::mpsc::channel();
            let mut pool = Pool::new(sender);
            // Screenshot/demo hook: route rust at the scripted server.
            if let Some(script) = std::env::var_os("TEXTCHUM_FAKE_LSP") {
                pool.add_override(textchum_lsp::ServerConfig {
                    id: "fake".into(),
                    command: "python3".into(),
                    args: vec![script.to_string_lossy().into_owned()],
                    languages: vec!["rust".into()],
                    install_hint: "n/a".into(),
                });
            }
            let shell = Rc::new(Shell {
                config: RefCell::new(config),
                pool: RefCell::new(pool),
                events: RefCell::new(receiver),
                pages: RefCell::new(HashMap::new()),
                callbacks: RefCell::new(HashMap::new()),
                own_saves: RefCell::new(HashMap::new()),
                own_config_save: std::cell::Cell::new(None),
                status_log: RefCell::new(Vec::new()),
                config_monitor: RefCell::new(None),
                documents: RefCell::new(HashMap::new()),
                documents_by_path: RefCell::new(HashMap::new()),
                next_document_id: std::cell::Cell::new(1),
            });
            shell.apply_appearance();
            shell.apply_theme();
            shell.apply_icon_pack();
            shell.reconfigure_pool();
            let pump = Rc::clone(&shell);
            glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
                pump.pump();
                glib::ControlFlow::Continue
            });
            *cell.borrow_mut() = Some(Rc::clone(&shell));
            shell
        })
    }

    /// Pushes the configuration's `lsp` + `workspace` sections into the
    /// pool, exactly as the macOS shell does.
    /// Registers a document and hands back its entry.
    ///
    /// A document is opened once however many views end up showing it,
    /// so a path already open gives back the entry it already has.
    pub fn open_document(
        &self,
        buffer: &sourceview5::Buffer,
        state: &Rc<RefCell<crate::page::State>>,
        path: Option<&str>,
    ) -> Rc<OpenDocument> {
        if let Some(existing) = path.and_then(|path| self.document_for_path(path)) {
            return existing;
        }
        let id = DocumentId(self.next_document_id.get());
        self.next_document_id.set(id.0 + 1);
        let document = Rc::new(OpenDocument {
            id,
            buffer: buffer.clone(),
            state: Rc::clone(state),
            path: RefCell::new(path.map(str::to_owned)),
            monitor: RefCell::new(None),
            folded: RefCell::new(Vec::new()),
            diagnostics: RefCell::new(Vec::new()),
        });
        self.documents.borrow_mut().insert(id, Rc::clone(&document));
        if let Some(path) = path {
            self.documents_by_path
                .borrow_mut()
                .insert(path.to_owned(), id);
        }
        document
    }

    pub fn document(&self, id: DocumentId) -> Option<Rc<OpenDocument>> {
        self.documents.borrow().get(&id).cloned()
    }

    /// The document a path names, while it is open.
    pub fn document_for_path(&self, path: &str) -> Option<Rc<OpenDocument>> {
        let id = *self.documents_by_path.borrow().get(path)?;
        self.document(id)
    }

    /// Follows a document that has just been given a path, or a new
    /// one — the index is by path, and the path moved.
    pub fn rename_document(&self, id: DocumentId, from: Option<&str>, to: &str) {
        {
            let mut index = self.documents_by_path.borrow_mut();
            if let Some(from) = from {
                index.remove(from);
            }
            index.insert(to.to_owned(), id);
        }
        if let Some(document) = self.document(id) {
            *document.path.borrow_mut() = Some(to.to_owned());
        }
    }

    /// Forgets a document. Its views are gone by the time this is
    /// called; what happens to the ones with unsaved changes is the
    /// caller's business.
    pub fn close_document(&self, id: DocumentId) {
        self.documents.borrow_mut().remove(&id);
        self.documents_by_path
            .borrow_mut()
            .retain(|_, known| *known != id);
    }

    /// How many documents are open, for the tests.
    pub fn document_count(&self) -> usize {
        self.documents.borrow().len()
    }

    pub fn reconfigure_pool(&self) {
        let config = self.config.borrow();
        let combined = format!(
            "{{\"lsp\":{},\"workspace\":{}}}",
            config.lsp_json(),
            config.workspace_json()
        );
        drop(config);
        self.pool.borrow_mut().configure(&combined);
    }

    /// Applies the configured appearance through libadwaita.
    pub fn apply_appearance(&self) {
        use textchum_core::Appearance;
        let scheme = match self.config.borrow().appearance() {
            Appearance::System => adw::ColorScheme::Default,
            Appearance::Light => adw::ColorScheme::ForceLight,
            Appearance::Dark => adw::ColorScheme::ForceDark,
        };
        adw::StyleManager::default().set_color_scheme(scheme);
        // The text area's background comes from the source view's own
        // scheme, which does not follow the colour scheme by itself.
        crate::workbench::Workbench::for_each(|workbench| {
            for page in workbench.all_pages() {
                crate::page::apply_source_scheme(&page.buffer);
                crate::page::recolor(&page.buffer);
            }
        });
    }

    /// Activates the configured theme — a built-in, or a user JSON
    /// file from `~/.config/textchum/themes/` — and recolors every
    /// open buffer's tags.
    pub fn apply_theme(&self) {
        let name = self.config.borrow().theme();
        let chosen = theme::Theme::builtin(&name).or_else(|| {
            let file = themes_dir().join(format!("{name}.json"));
            std::fs::read_to_string(file)
                .ok()
                .and_then(|json| theme::Theme::from_json(&json).ok())
        });
        if let Some(chosen) = chosen {
            theme::set_active(chosen);
        }
        for handles in self.pages.borrow().values() {
            crate::page::refresh_style_tags(&handles.document.buffer);
            crate::page::recolor(&handles.document.buffer);
        }
    }

    /// Loads the configured file-icon pack, or clears it. A pack that
    /// cannot be read is reported to the terminal and the tree keeps
    /// the desktop's icons — the same escape hatch a broken theme gets,
    /// and for the same reason: a pack someone moved should not stop
    /// the editor.
    pub fn apply_icon_pack(&self) {
        match self.config.borrow().icon_pack() {
            Some(path) => {
                match textchum_core::icons::set_active_from(std::path::Path::new(&path)) {
                    Ok(summary) => eprintln!("textchum: icons {summary}"),
                    Err(error) => {
                        textchum_core::icons::clear_active();
                        eprintln!("textchum: the icon pack could not be used: {error}");
                    }
                }
            }
            None => textchum_core::icons::clear_active(),
        }
    }

    /// Remembers that this process wrote `path` just now, so the file
    /// monitor does not offer to reload the app's own save.
    pub fn note_own_save(&self, path: &str) {
        self.own_saves
            .borrow_mut()
            .insert(path.to_owned(), std::time::Instant::now());
    }

    /// Whether a monitor event for `path` is the echo of our own save.
    pub fn is_own_save(&self, path: &str) -> bool {
        self.own_saves
            .borrow()
            .get(path)
            .is_some_and(|at| at.elapsed() < std::time::Duration::from_secs(2))
    }

    pub fn save_config(&self) {
        self.own_config_save.set(Some(std::time::Instant::now()));
        if let Err(error) = self.config.borrow_mut().save() {
            eprintln!("textchum: could not save configuration: {error}");
        }
    }

    /// Follows external edits to config.json while running: the file is
    /// reloaded wholesale and `reapply` runs the same pipeline a
    /// Preferences change does. The app's own saves are ignored.
    pub fn watch_config(self: &Rc<Self>, reapply: impl Fn() + 'static) {
        let file = gtk::gio::File::for_path(config_path());
        let Ok(monitor) = file.monitor_file(
            gtk::gio::FileMonitorFlags::NONE,
            gtk::gio::Cancellable::NONE,
        ) else {
            return;
        };
        let shell = Rc::clone(self);
        monitor.connect_changed(move |_, _, _, event| {
            use gtk::gio::FileMonitorEvent;
            if !matches!(
                event,
                FileMonitorEvent::ChangesDoneHint | FileMonitorEvent::Created
            ) {
                return;
            }
            if shell
                .own_config_save
                .get()
                .is_some_and(|at| at.elapsed() < std::time::Duration::from_secs(2))
            {
                return;
            }
            if let Some(warning) = shell.config.borrow_mut().reload() {
                eprintln!("textchum: config reload: {warning}");
            }
            shell.apply_appearance();
            shell.apply_theme();
            shell.apply_icon_pack();
            shell.reconfigure_pool();
            reapply();
        });
        *self.config_monitor.borrow_mut() = Some(monitor);
    }

    /// Registers a request's continuation; the pump calls it when the
    /// matching [`Event::LspResponse`] arrives.
    pub fn expect_response(&self, id: u64, callback: impl FnOnce(&str) + 'static) {
        if id != 0 {
            self.callbacks.borrow_mut().insert(id, Box::new(callback));
        }
    }

    fn pump(&self) {
        // Drain first, dispatch after: dispatching can re-enter the
        // shell (a definition reply opens a page), so no borrow may be
        // held across it.
        let mut drained = Vec::new();
        loop {
            match self.events.borrow().try_recv() {
                Ok(event) => drained.push(event),
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
            }
        }
        for event in drained {
            match event {
                Event::Diagnostics { path, json } => {
                    let handles = self.pages.borrow().get(&path).cloned();
                    if let Some(handles) = handles {
                        crate::page::apply_diagnostics(&handles, &json);
                    }
                }
                Event::LspResponse { id, json } => {
                    let callback = self.callbacks.borrow_mut().remove(&id);
                    if let Some(callback) = callback {
                        callback(&json);
                    }
                }
                Event::ServerStatus { status, message, server, root } => {
                    let line = if message.is_empty() {
                        status.clone()
                    } else {
                        format!("{status} — {message}")
                    };
                    {
                        let mut log = self.status_log.borrow_mut();
                        log.push((
                            std::time::SystemTime::now(),
                            server.clone(),
                            root.clone(),
                            line,
                        ));
                        let overflow = log.len().saturating_sub(100);
                        if overflow > 0 {
                            log.drain(..overflow);
                        }
                    }
                    if status == "not-found" || status == "failed" {
                        let text = if message.is_empty() {
                            format!("{server}: {status}")
                        } else {
                            message
                        };
                        // A missing server is a sentence naming a
                        // package to install, and it arrives while the
                        // user is looking at their file rather than at
                        // the notification — so it wraps and waits to be
                        // dismissed instead of ellipsizing and fading.
                        // It goes to the window the user is in, not to
                        // whichever page happens to be first in the map.
                        if let Some(workbench) = crate::workbench::Workbench::active() {
                            workbench.explain(&text);
                        }
                    }
                }
                Event::Pong { .. } => {}
            }
        }
    }
}
