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
mod document;
mod history;

pub use app::{App, Event};
pub use buffer::{Buffer, BufferError};
pub use document::{AppliedEdit, Document, DocumentError, Encoding};
