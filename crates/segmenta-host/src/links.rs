//! Chunked link list reassembly for the browser extension (Hito 2, T8).
//!
//! When the browser extension captures a large link list, the payload can exceed the
//! 1 MiB Native Messaging frame limit enforced by [`crate::framing::MAX_MESSAGE_BYTES`].
//! The extension splits the list into indexed chunks carrying the message method
//! [`METHOD_LINKS_CHUNK`].
//!
//! [`LinksPart`] accumulates these chunks per `capture_id` and yields the complete,
//! ordered list of links once all declared chunks have arrived. Chunks may arrive out
//! of order. Duplicate chunks are idempotent and do not duplicate links.
//!
//! Hard bounds enforced by [`LinksLimits`]:
//!
//! * maximum links per capture ([`ERR_LINKS_MAX_LINKS_EXCEEDED`]);
//! * maximum total UTF-8 link bytes per capture ([`ERR_LINKS_MAX_BYTES_EXCEEDED`]);
//! * maximum concurrent in-flight captures ([`ERR_LINKS_MAX_CONCURRENT_CAPTURES`]);
//! * inactivity expiry per capture ([`ERR_LINKS_EXPIRED`]).
//!
//! An expired or explicitly evicted capture returns a typed [`LinksError`] naming the
//! reason, never a silent empty list.

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Message method for chunked links.
pub const METHOD_LINKS_CHUNK: &str = "links.chunk";

/// Stable identifier of [`LinksError::InvalidMethod`].
pub const ERR_LINKS_INVALID_METHOD: &str = "ERR_LINKS_INVALID_METHOD";
/// Stable identifier of [`LinksError::InvalidPayload`].
pub const ERR_LINKS_INVALID_PAYLOAD: &str = "ERR_LINKS_INVALID_PAYLOAD";
/// Stable identifier of [`LinksError::IndexOutOfRange`].
pub const ERR_LINKS_INDEX_OUT_OF_RANGE: &str = "ERR_LINKS_INDEX_OUT_OF_RANGE";
/// Stable identifier of [`LinksError::MismatchedTotal`].
pub const ERR_LINKS_MISMATCHED_TOTAL: &str = "ERR_LINKS_MISMATCHED_TOTAL";
/// Stable identifier of [`LinksError::MaxLinksExceeded`].
pub const ERR_LINKS_MAX_LINKS_EXCEEDED: &str = "ERR_LINKS_MAX_LINKS_EXCEEDED";
/// Stable identifier of [`LinksError::MaxBytesExceeded`].
pub const ERR_LINKS_MAX_BYTES_EXCEEDED: &str = "ERR_LINKS_MAX_BYTES_EXCEEDED";
/// Stable identifier of [`LinksError::MaxConcurrentCapturesExceeded`].
pub const ERR_LINKS_MAX_CONCURRENT_CAPTURES: &str = "ERR_LINKS_MAX_CONCURRENT_CAPTURES";
/// Stable identifier of [`LinksError::Expired`].
pub const ERR_LINKS_EXPIRED: &str = "ERR_LINKS_EXPIRED";
/// Stable identifier of [`LinksError::Evicted`].
pub const ERR_LINKS_EVICTED: &str = "ERR_LINKS_EVICTED";

const MAX_DISPOSITION_HISTORY: usize = 512;

/// What a caller can move in tests: milliseconds since an arbitrary origin.
pub trait Clock: Send + Sync {
    /// Milliseconds elapsed since the clock started.
    fn elapsed_ms(&self) -> u64;
}

/// Production clock: a monotonic instant captured when the clock is created.
pub struct MonotonicClock {
    origin: Instant,
}

impl Default for MonotonicClock {
    fn default() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}

impl Clock for MonotonicClock {
    fn elapsed_ms(&self) -> u64 {
        self.origin.elapsed().as_millis() as u64
    }
}

/// Limits governing in-flight link captures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinksLimits {
    /// Maximum total links a single capture may contain.
    pub max_links_per_capture: usize,
    /// Maximum total bytes across all link URLs in a single capture.
    pub max_bytes_per_capture: usize,
    /// Maximum simultaneous captures being assembled in memory.
    pub max_concurrent_captures: usize,
    /// Inactivity timeout after which an abandoned capture is discarded.
    pub expiry: Duration,
}

impl Default for LinksLimits {
    fn default() -> Self {
        Self {
            max_links_per_capture: 10_000,
            max_bytes_per_capture: 10 * 1024 * 1024,
            max_concurrent_captures: 16,
            expiry: Duration::from_secs(300),
        }
    }
}

/// A chunk message received from the extension.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkMessage {
    /// Method identifier; must be [`METHOD_LINKS_CHUNK`].
    pub method: String,
    /// Chunk payload parameters.
    pub params: ChunkParams,
}

impl ChunkMessage {
    /// Build a new chunk message with the standard method.
    pub fn new(
        capture_id: impl Into<String>,
        index: usize,
        total: usize,
        links: Vec<String>,
    ) -> Self {
        Self {
            method: METHOD_LINKS_CHUNK.to_string(),
            params: ChunkParams {
                capture_id: capture_id.into(),
                index,
                total,
                links,
            },
        }
    }
}

impl From<ChunkParams> for ChunkMessage {
    fn from(params: ChunkParams) -> Self {
        Self {
            method: METHOD_LINKS_CHUNK.to_string(),
            params,
        }
    }
}

/// Parameters for one chunk of links.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkParams {
    /// Unique identifier of this capture session.
    pub capture_id: String,
    /// 0-based chunk index.
    pub index: usize,
    /// Declared total number of chunks.
    pub total: usize,
    /// Links carried by this chunk.
    pub links: Vec<String>,
}

/// Failure during chunk processing or reassembly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinksError {
    /// The message method was not [`METHOD_LINKS_CHUNK`].
    InvalidMethod {
        /// The unexpected method string received.
        method: String,
    },
    /// The message payload could not be deserialized.
    InvalidPayload {
        /// Detail of the deserialization failure.
        detail: String,
    },
    /// The chunk index is out of range for the declared total.
    IndexOutOfRange {
        /// The received index.
        index: usize,
        /// The total number of chunks.
        total: usize,
    },
    /// The chunk declared a total different from the previously recorded total.
    MismatchedTotal {
        /// Total recorded by the first chunk seen.
        expected: usize,
        /// Total declared in the current chunk.
        actual: usize,
    },
    /// Total link count in the capture would exceed [`LinksLimits::max_links_per_capture`].
    MaxLinksExceeded {
        /// Links accumulated plus the new chunk.
        count: usize,
        /// Configured limit.
        limit: usize,
    },
    /// Total link bytes in the capture would exceed [`LinksLimits::max_bytes_per_capture`].
    MaxBytesExceeded {
        /// Bytes accumulated plus the new chunk.
        bytes: usize,
        /// Configured limit.
        limit: usize,
    },
    /// Maximum concurrent captures reached.
    MaxConcurrentCapturesExceeded {
        /// Configured limit.
        limit: usize,
    },
    /// The capture expired due to inactivity.
    Expired {
        /// The capture identifier.
        capture_id: String,
        /// Explanation of the expiry.
        reason: String,
    },
    /// The capture was evicted or cancelled.
    Evicted {
        /// The capture identifier.
        capture_id: String,
        /// Reason for eviction.
        reason: String,
    },
}

impl LinksError {
    /// Stable `ERR_LINKS_*` identifier of this failure.
    pub fn identifier(&self) -> &'static str {
        match self {
            Self::InvalidMethod { .. } => ERR_LINKS_INVALID_METHOD,
            Self::InvalidPayload { .. } => ERR_LINKS_INVALID_PAYLOAD,
            Self::IndexOutOfRange { .. } => ERR_LINKS_INDEX_OUT_OF_RANGE,
            Self::MismatchedTotal { .. } => ERR_LINKS_MISMATCHED_TOTAL,
            Self::MaxLinksExceeded { .. } => ERR_LINKS_MAX_LINKS_EXCEEDED,
            Self::MaxBytesExceeded { .. } => ERR_LINKS_MAX_BYTES_EXCEEDED,
            Self::MaxConcurrentCapturesExceeded { .. } => ERR_LINKS_MAX_CONCURRENT_CAPTURES,
            Self::Expired { .. } => ERR_LINKS_EXPIRED,
            Self::Evicted { .. } => ERR_LINKS_EVICTED,
        }
    }

    /// Explanatory reason if this error represents an expired or evicted capture.
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Expired { reason, .. } | Self::Evicted { reason, .. } => Some(reason),
            _ => None,
        }
    }
}

impl fmt::Display for LinksError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let id = self.identifier();
        match self {
            Self::InvalidMethod { method } => {
                write!(
                    f,
                    "{id}: unexpected method {method:?}, expected {METHOD_LINKS_CHUNK}"
                )
            }
            Self::InvalidPayload { detail } => {
                write!(f, "{id}: invalid payload: {detail}")
            }
            Self::IndexOutOfRange { index, total } => {
                write!(f, "{id}: index {index} is out of range for total {total}")
            }
            Self::MismatchedTotal { expected, actual } => {
                write!(
                    f,
                    "{id}: total {actual} does not match expected total {expected}"
                )
            }
            Self::MaxLinksExceeded { count, limit } => {
                write!(f, "{id}: {count} links exceeds limit of {limit}")
            }
            Self::MaxBytesExceeded { bytes, limit } => {
                write!(f, "{id}: {bytes} bytes exceeds limit of {limit}")
            }
            Self::MaxConcurrentCapturesExceeded { limit } => {
                write!(
                    f,
                    "{id}: maximum concurrent captures limit of {limit} reached"
                )
            }
            Self::Expired { capture_id, reason } => {
                write!(f, "{id}: capture {capture_id:?} expired: {reason}")
            }
            Self::Evicted { capture_id, reason } => {
                write!(f, "{id}: capture {capture_id:?} evicted: {reason}")
            }
        }
    }
}

impl std::error::Error for LinksError {}

#[derive(Debug, Clone)]
enum Disposition {
    Expired(String),
    Evicted(String),
}

struct ActiveCapture {
    total: usize,
    chunks: HashMap<usize, Vec<String>>,
    total_links: usize,
    total_bytes: usize,
    last_updated_ms: u64,
}

/// Accumulates link chunks per `capture_id` and reassembles them upon completion.
pub struct LinksPart {
    limits: LinksLimits,
    clock: Arc<dyn Clock>,
    captures: HashMap<String, ActiveCapture>,
    dispositions: HashMap<String, Disposition>,
    disposition_order: VecDeque<String>,
}
