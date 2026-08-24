//! Application handle and asynchronous event delivery.
//!
//! Shells cannot poll the core for everything: some information (a language
//! server reporting diagnostics, a background parse finishing) originates on
//! core-owned worker threads. [`App`] is the channel for that direction.
//!
//! The delivery contract is intentionally strict:
//!
//! * The shell registers exactly one callback at construction time.
//! * The core invokes it from exactly one dedicated dispatch thread — never
//!   from the caller's thread, never concurrently with itself.
//! * The shell is responsible for hopping to its UI thread inside the
//!   callback.
//!
//! Keeping a single dispatch thread means shells only ever write one small
//! piece of thread-marshalling code, and the core never has to reason about
//! reentrancy into the shell.

use std::sync::mpsc;
use std::thread::JoinHandle;

/// An event pushed from the core to the shell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Reply to [`App::ping`]; carries the caller-supplied sequence number.
    /// Exists to exercise and verify the async delivery path end to end.
    Pong { seq: u64 },
    /// A language server published diagnostics for a file. `json` is a
    /// compact array of `{line, character, endLine, endCharacter,
    /// severity, message}` objects (positions in LSP convention: zero-based
    /// line, UTF-16 column).
    Diagnostics { path: String, json: String },
    /// A language-server instance changed state. `status` is one of
    /// `starting`, `running`, `not-found`, `failed`, `exited`.
    ServerStatus {
        server: String,
        root: String,
        status: String,
        message: String,
    },
}

/// A handle for pushing events into the app's delivery channel from
/// core-owned subsystems (language servers, background work).
pub type EventSender = mpsc::Sender<Event>;

/// The root handle for a core instance.
///
/// Owns the event dispatch thread. Dropping the `App` shuts the thread down
/// after any already-queued events have been delivered.
pub struct App {
    sender: Option<mpsc::Sender<Event>>,
    dispatcher: Option<JoinHandle<()>>,
}

impl App {
    /// Creates an app whose events are delivered to `on_event`.
    ///
    /// `on_event` runs on a dedicated dispatch thread owned by the core; see
    /// the module docs for the exact contract.
    pub fn new<F>(on_event: F) -> Self
    where
        F: Fn(Event) + Send + 'static,
    {
        let (sender, receiver) = mpsc::channel::<Event>();
        let dispatcher = std::thread::Builder::new()
            .name("textchum-events".into())
            .spawn(move || {
                while let Ok(event) = receiver.recv() {
                    on_event(event);
                }
            })
            .expect("failed to spawn event dispatch thread");
        Self {
            sender: Some(sender),
            dispatcher: Some(dispatcher),
        }
    }

    /// A sender that subsystems (e.g. the language-server pool) use to push
    /// events into the same delivery channel. Events sent after the app is
    /// dropped are silently discarded.
    pub fn sender(&self) -> EventSender {
        self.sender
            .clone()
            .expect("sender only vacated during drop")
    }

    /// Requests an asynchronous [`Event::Pong`] carrying `seq`.
    pub fn ping(&self, seq: u64) {
        if let Some(sender) = &self.sender {
            // The only send error is a disconnected receiver, which can only
            // happen mid-drop; losing the pong there is fine.
            let _ = sender.send(Event::Pong { seq });
        }
    }
}

impl Drop for App {
    fn drop(&mut self) {
        // Closing the channel ends the dispatcher's recv loop; join so no
        // callback can run after the shell has torn down its side.
        drop(self.sender.take());
        if let Some(handle) = self.dispatcher.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn ping_delivers_pong_asynchronously() {
        let (tx, rx) = mpsc::channel();
        let app = App::new(move |event| tx.send(event).unwrap());
        app.ping(7);
        app.ping(8);
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            Event::Pong { seq: 7 }
        );
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            Event::Pong { seq: 8 }
        );
    }

    #[test]
    fn drop_delivers_queued_events_then_stops() {
        let (tx, rx) = mpsc::channel();
        let app = App::new(move |event| {
            let _ = tx.send(event);
        });
        app.ping(1);
        drop(app);
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            Event::Pong { seq: 1 }
        );
        assert!(rx.recv().is_err(), "channel should close after drop");
    }
}
