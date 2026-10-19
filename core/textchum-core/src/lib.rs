//! Platform-independent editor core.
//!
//! This crate is the single source of truth for document state. Platform
//! shells (macOS today) never own text; they render it and route every edit
//! through this crate. The public surface is deliberately small: [`Buffer`]
//! for text storage and editing, and [`App`] for asynchronous event delivery
//! back to the shell.
//!
//! Nothing in this crate may depend on a UI toolkit, an OS-specific API, or
//! perform any drawing. That boundary is what keeps the core portable and
//! testable headlessly.

mod app;
pub mod blame;
pub mod brackets;
pub mod calls;
pub mod vimregex;
mod buffer;
pub mod changes;
pub mod carets;
pub mod chords;
pub mod code_action;
pub mod completion;
pub mod definition;
pub mod edits;
pub mod everywhere;
pub mod diagnostics;
mod config;
mod document;
pub mod framing;
mod fsutil;
pub mod goto;
pub mod grammar;
mod history;
pub mod hugo;
pub mod i18n;
pub mod icons;
pub mod indent;
pub mod install;
pub mod inlay;
pub mod keys;
pub mod local_history;
pub mod markdown;
pub mod occurrences;
pub mod output;
pub mod motion;
pub mod notices;
pub mod pairs;
pub mod presets;
pub mod preview;
pub mod project_state;
pub mod refactoring;
pub mod references;
pub mod run;
pub mod runnables;
pub mod test_results;
pub mod search;
pub mod semantic;
pub mod signature;
pub mod snippet;
pub mod symbols;
pub mod syntax;
pub mod theme_import;
pub mod tools;
pub mod trail;
pub mod transform;
pub mod workspace;

pub use app::{App, Event, EventSender};
pub use buffer::{Buffer, BufferError};
pub use config::{
    switches, Appearance, Config, FileOverride, OpenTarget, ProjectParts, Switch,
    DEFAULT_FONT_SIZE, DEFAULT_TAB_WIDTH, FILE_OVERRIDE_MEMORY,
};
pub use document::{AppliedEdit, Document, DocumentError, Encoding};
pub use syntax::{theme, HighlightSpan};
