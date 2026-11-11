//! Segmenta download engine.
//!
//! The crate is split into small, testable pieces:
//!
//! * [`planner`] — initial segmentation and the hot-split arithmetic.
//! * [`retry`] — retry classification and backoff.
//! * [`ratelimit`] — global token bucket.
//! * [`filename`] — header/URL name extraction and sanitization.
//! * [`urlssrf`] — scheme and private-address validation.
//! * [`error`] — engine error taxonomy shared with the daemon and CLI.

pub mod checksum;
pub mod disk;
pub mod engine;
pub mod error;
pub mod filename;
pub mod planner;
pub mod probe;
pub mod ratelimit;
pub mod retry;
pub mod urlssrf;

#[cfg(feature = "testserver")]
pub mod testserver;

pub use error::{CoreError, Result};
pub use planner::{connections_for, initial_segments, SegmentRange};
