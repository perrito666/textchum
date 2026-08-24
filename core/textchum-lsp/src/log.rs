//! The LSP debug log: one append-only file recording every decision the
//! pool makes and every status transition an instance goes through, so
//! "why do I get no language server?" is answerable by reading a file
//! instead of guessing.
//!
//! Disabled until a path is set; the shell points it at a file under the
//! user's log directory. Lines are `[unix-millis] message`.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

static SINK: Mutex<Option<File>> = Mutex::new(None);

/// Opens (appending, creating directories as needed) the log file. An
/// unopenable path silently disables logging — the log must never take
/// the editor down.
pub fn set_path(path: &Path) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let file = OpenOptions::new().create(true).append(true).open(path).ok();
    if let Ok(mut sink) = SINK.lock() {
        *sink = file;
    }
}

/// Appends one line; a no-op while no path is set.
pub fn log(message: &str) {
    let Ok(mut sink) = SINK.lock() else { return };
    let Some(file) = sink.as_mut() else { return };
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let _ = writeln!(file, "[{millis}] {message}");
}
