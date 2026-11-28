//! Segmenta download engine.
//!
//! The crate is split into small, testable pieces:
//!
//! * [`planner`] — initial segmentation and the hot-split arithmetic.
//! * [`retry`] — retry classification and backoff.
//! * [`ratelimit`] — global token bucket.
//! * [`filename`] — header/URL name extraction and sanitization.
//! * [`urlssrf`] — scheme and private-address validation.
//! * [`probe`] — HTTP probing, redirects and capability classification.
//! * [`stream`] — public HLS/DASH parsing and merge planning.
//! * [`error`] — engine error taxonomy shared with the daemon and CLI.

pub mod batch;
pub mod checksum;
pub mod debrid;
pub mod disk;
pub mod dns;
pub mod engine;
pub mod error;
pub mod filename;
pub mod folder;
pub mod ftp;
pub mod ftps;
pub mod hostlimits;
pub mod netguard;
pub mod planner;
pub mod probe;
pub mod ratelimit;
pub mod retry;
pub mod rules;
pub mod siterules;
pub mod sites;
pub mod stream;
pub mod urlssrf;

#[cfg(feature = "testserver")]
pub mod testserver;

pub use error::{CoreError, Result};
pub use planner::{connections_for, initial_segments, SegmentRange};
