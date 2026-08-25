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

use gtk::glib;
use textchum_core::{theme, Config, Event};
use textchum_lsp::Pool;

/// Everything the pump needs to reach one open document's page.
pub struct PageHandles {
    pub window: adw::ApplicationWindow,
    pub tab_view: adw::TabView,
    pub tab_page: adw::TabPage,
    pub buffer: sourceview5::Buffer,
    pub view: sourceview5::View,
    pub toasts: adw::ToastOverlay,
    pub title: adw::WindowTitle,
    /// "language" and "N errors" halves of the subtitle.
    pub language: RefCell<String>,
    pub problems: RefCell<String>,
    /// "encoding · size" half, refreshed with the chrome.
    pub detail: RefCell<String>,
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
}

thread_local! {
    static SHELL: RefCell<Option<Rc<Shell>>> = const { RefCell::new(None) };
}

/// `~/.config/textchum/config.json` — the Linux home of the same file.
pub fn config_path() -> PathBuf {
    glib::user_config_dir().join("textchum/config.json")
}

/// `~/.config/textchum/themes/` — user theme JSON files, one per
/// theme, named by their file stem.
pub fn themes_dir() -> PathBuf {
    glib::user_config_dir().join("textchum/themes")
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
            let log_path = crate::session::state_dir().join("textchum/lsp.log");
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
            });
            shell.apply_appearance();
            shell.apply_theme();
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
            crate::page::refresh_style_tags(&handles.buffer);
            crate::page::recolor(&handles.buffer);
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
        if let Err(error) = self.config.borrow_mut().save() {
            eprintln!("textchum: could not save configuration: {error}");
        }
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
                Event::ServerStatus { status, message, server, .. } => {
                    if status == "not-found" || status == "failed" {
                        let text = if message.is_empty() {
                            format!("{server}: {status}")
                        } else {
                            message
                        };
                        let handles = self.pages.borrow().values().next().cloned();
                        if let Some(handles) = handles {
                            handles.toasts.add_toast(adw::Toast::new(&text));
                        }
                    }
                }
                Event::Pong { .. } => {}
            }
        }
    }
}
