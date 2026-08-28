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
mod buffer;
pub mod changes;
mod config;
mod document;
mod fsutil;
pub mod goto;
mod history;
pub mod hugo;
pub mod icons;
pub mod markdown;
pub mod references;
pub mod search;
pub mod snippet;
pub mod syntax;
pub mod theme_import;
pub mod workspace;

pub use app::{App, Event, EventSender};
pub use buffer::{Buffer, BufferError};
pub use config::{
    Appearance, Config, FileOverride, OpenTarget, DEFAULT_FONT_SIZE, DEFAULT_TAB_WIDTH,
    FILE_OVERRIDE_MEMORY,
};
pub use document::{AppliedEdit, Document, DocumentError, Encoding};
pub use syntax::{theme, HighlightSpan};
