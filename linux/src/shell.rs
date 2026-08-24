//! The shell's app-wide machinery: one language-server pool for every
//! window (the one-instance-per-project behavior comes from the linked
//! crate), an event pump that marshals server events from their threads
//! onto the GTK main loop, a registry of open windows by path, and the
//! response router for request/reply traffic (definitions, and later
//! hover and completion).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, TryRecvError};

use adw::prelude::*;
use gtk::glib;
use textchum_core::Event;
use textchum_lsp::Pool;

/// Everything the pump needs to reach one window.
pub struct WindowHandles {
    pub window: adw::ApplicationWindow,
    pub buffer: sourceview5::Buffer,
    pub view: sourceview5::View,
    pub title: adw::WindowTitle,
    pub toasts: adw::ToastOverlay,
    /// The subtitle before any problem count is appended.
    pub base_subtitle: RefCell<String>,
}

pub struct Shell {
    pub pool: RefCell<Pool>,
    events: RefCell<Receiver<Event>>,
    pub windows: RefCell<HashMap<String, Rc<WindowHandles>>>,
    callbacks: RefCell<HashMap<u64, Box<dyn FnOnce(&str)>>>,
}

thread_local! {
    static SHELL: RefCell<Option<Rc<Shell>>> = const { RefCell::new(None) };
}

impl Shell {
    /// The process-wide shell, started on first use — which also arms
    /// the 50 ms pump that drains server events into the main loop.
    pub fn instance() -> Rc<Shell> {
        SHELL.with(|cell| {
            if let Some(shell) = cell.borrow().as_ref() {
                return Rc::clone(shell);
            }
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
                pool: RefCell::new(pool),
                events: RefCell::new(receiver),
                windows: RefCell::new(HashMap::new()),
                callbacks: RefCell::new(HashMap::new()),
            });
            let pump = Rc::clone(&shell);
            glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
                pump.pump();
                glib::ControlFlow::Continue
            });
            *cell.borrow_mut() = Some(Rc::clone(&shell));
            shell
        })
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
        // shell (a definition reply opens a window), so no borrow may
        // be held across it.
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
                    let handles = self.windows.borrow().get(&path).cloned();
                    if let Some(handles) = handles {
                        crate::editor::apply_diagnostics(&handles, &json);
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
                        let handles = self.windows.borrow().values().next().cloned();
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
