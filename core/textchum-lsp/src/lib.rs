//! Language Server Protocol client and per-project instance pool.
//!
//! The one behavioral promise this crate exists for: **one server process
//! per (server, project root)**. See [`pool::Pool`] for the mechanics and
//! the module docs of [`instance`] for the threading model. Everything a
//! server reports flows to the shell through the core's single event
//! channel; nothing here blocks the caller.

mod instance;
pub mod log;
pub mod pool;
pub mod registry;
mod transport;
mod uri;

pub use pool::{Pool, ServerConfig};
pub use registry::{server_for_language, ServerSpec};
