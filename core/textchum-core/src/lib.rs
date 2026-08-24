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
mod buffer;
mod config;
mod document;
mod fsutil;
mod history;
pub mod syntax;
pub mod workspace;

pub use app::{App, Event, EventSender};
pub use buffer::{Buffer, BufferError};
pub use config::{Appearance, Config, DEFAULT_FONT_SIZE, DEFAULT_TAB_WIDTH};
pub use document::{AppliedEdit, Document, DocumentError, Encoding};
pub use syntax::{theme, HighlightSpan};
