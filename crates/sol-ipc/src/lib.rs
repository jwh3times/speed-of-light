//! The service <-> client protocol: newline-delimited JSON over a Windows named pipe.
//!
//! JSON lines keeps the protocol debuggable and language-neutral, so a future GUI in any
//! stack, a Stream Deck plugin or a script can talk to the service without linking Rust code.
//! Bump [`PROTOCOL_VERSION`] on any breaking change. Clients send `Hello` first and refuse a
//! mismatch.

pub mod framing;
#[cfg(windows)]
pub mod pipe;
pub mod protocol;

pub use protocol::{DeviceStatus, FanModeView, PROTOCOL_VERSION, Request, Response};

/// Default pipe name. The service creates it; clients connect to it.
pub const PIPE_NAME: &str = r"\\.\pipe\speed-of-light";
