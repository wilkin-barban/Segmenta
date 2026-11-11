//! Segmenta Native Messaging host library.
//!
//! [`framing`] implements the 4-byte little-endian framing of OpenSpec 03 §4 and
//! [`proxy`] forwards allowlisted JSON-RPC methods to the daemon socket.

pub mod framing;
pub mod proxy;

pub use framing::{read_message, write_message, FrameError, MAX_MESSAGE_BYTES};
pub use proxy::{
    is_allowed_method, serve, socket_path, Activator, HostOptions, NoopActivator,
    SystemctlActivator, DEFAULT_RESPONSE_TIMEOUT, ERR_DAEMON_UNAVAILABLE, SYSTEMD_UNIT,
};
