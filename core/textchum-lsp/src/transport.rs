//! The framing the language-server client speaks, which lives in the
//! core because the debug-adapter client speaks it too.

pub use textchum_core::framing::{read_message, write_message};
