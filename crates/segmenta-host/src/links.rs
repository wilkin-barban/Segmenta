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

impl LinksPart {
    /// Create a reassembler with the given limits and monotonic clock.
    pub fn new(limits: LinksLimits) -> Self {
        Self::with_clock(limits, Arc::new(MonotonicClock::default()))
    }

    /// Create a reassembler with the given limits and a caller-supplied clock.
    pub fn with_clock(limits: LinksLimits, clock: Arc<dyn Clock>) -> Self {
        Self {
            limits,
            clock,
            captures: HashMap::new(),
            dispositions: HashMap::new(),
            disposition_order: VecDeque::new(),
        }
    }

    /// Current limits in force.
    pub fn limits(&self) -> &LinksLimits {
        &self.limits
    }

    /// Number of active captures currently accumulating in memory.
    pub fn active_captures(&self) -> usize {
        self.captures.len()
    }

    /// Evicts expired captures based on the configured timeout.
    pub fn prune(&mut self) {
        let now = self.clock.elapsed_ms();
        let expiry_ms = self.limits.expiry.as_millis() as u64;

        let mut expired_ids = Vec::new();
        for (id, cap) in &self.captures {
            if now.saturating_sub(cap.last_updated_ms) >= expiry_ms {
                expired_ids.push(id.clone());
            }
        }

        for id in expired_ids {
            self.captures.remove(&id);
            self.record_disposition(
                id,
                Disposition::Expired("capture expired due to inactivity".to_string()),
            );
        }
    }

    /// Explicitly evict a capture session.
    pub fn evict(&mut self, capture_id: &str, reason: impl Into<String>) -> bool {
        let reason = reason.into();
        if self.captures.remove(capture_id).is_some() {
            self.record_disposition(capture_id.to_string(), Disposition::Evicted(reason));
            true
        } else {
            false
        }
    }

    fn record_disposition(&mut self, capture_id: String, disp: Disposition) {
        if let Some(existing) = self.dispositions.get_mut(&capture_id) {
            *existing = disp;
            return;
        }
        if self.disposition_order.len() >= MAX_DISPOSITION_HISTORY {
            if let Some(oldest) = self.disposition_order.pop_front() {
                self.dispositions.remove(&oldest);
            }
        }
        self.disposition_order.push_back(capture_id.clone());
        self.dispositions.insert(capture_id, disp);
    }

    /// Ingest a chunk message, returning `Ok(Some(links))` when all chunks are present.
    pub fn push(&mut self, chunk: ChunkMessage) -> Result<Option<Vec<String>>, LinksError> {
        if chunk.method != METHOD_LINKS_CHUNK {
            return Err(LinksError::InvalidMethod {
                method: chunk.method,
            });
        }

        // Prune any abandoned captures before processing.
        self.prune();

        let capture_id = chunk.params.capture_id;
        let index = chunk.params.index;
        let total = chunk.params.total;
        let links = chunk.params.links;

        // Check if this capture was previously expired or evicted.
        if let Some(disposition) = self.dispositions.get(&capture_id) {
            return match disposition {
                Disposition::Expired(reason) => Err(LinksError::Expired {
                    capture_id,
                    reason: reason.clone(),
                }),
                Disposition::Evicted(reason) => Err(LinksError::Evicted {
                    capture_id,
                    reason: reason.clone(),
                }),
            };
        }

        let now = self.clock.elapsed_ms();
        let expiry_ms = self.limits.expiry.as_millis() as u64;

        if let Some(cap) = self.captures.get_mut(&capture_id) {
            // Check if this active capture expired.
            if now.saturating_sub(cap.last_updated_ms) >= expiry_ms {
                self.captures.remove(&capture_id);
                let reason = "capture expired due to inactivity".to_string();
                self.record_disposition(capture_id.clone(), Disposition::Expired(reason.clone()));
                return Err(LinksError::Expired { capture_id, reason });
            }

            if total != cap.total {
                return Err(LinksError::MismatchedTotal {
                    expected: cap.total,
                    actual: total,
                });
            }

            if index >= cap.total {
                return Err(LinksError::IndexOutOfRange {
                    index,
                    total: cap.total,
                });
            }

            // Duplicate chunks are idempotent.
            if cap.chunks.contains_key(&index) {
                cap.last_updated_ms = now;
                return Ok(None);
            }

            let new_links_count = links.len();
            let candidate_links = cap.total_links.saturating_add(new_links_count);
            if candidate_links > self.limits.max_links_per_capture {
                return Err(LinksError::MaxLinksExceeded {
                    count: candidate_links,
                    limit: self.limits.max_links_per_capture,
                });
            }

            let new_bytes: usize = links.iter().map(|l| l.len()).sum();
            let candidate_bytes = cap.total_bytes.saturating_add(new_bytes);
            if candidate_bytes > self.limits.max_bytes_per_capture {
                return Err(LinksError::MaxBytesExceeded {
                    bytes: candidate_bytes,
                    limit: self.limits.max_bytes_per_capture,
                });
            }

            cap.chunks.insert(index, links);
            cap.total_links = candidate_links;
            cap.total_bytes = candidate_bytes;
            cap.last_updated_ms = now;

            if cap.chunks.len() == cap.total {
                let total = cap.total;
                let total_links = cap.total_links;
                // The entry was borrowed above and is owned here; if it vanished there is
                // nothing to assemble and an error is better than a panic in a parser.
                let Some(mut cap) = self.captures.remove(&capture_id) else {
                    return Err(LinksError::Expired {
                        capture_id,
                        reason: "the capture went away while assembling".to_string(),
                    });
                };
                let mut assembled = Vec::with_capacity(total_links);
                for idx in 0..total {
                    if let Some(chunk_links) = cap.chunks.remove(&idx) {
                        assembled.extend(chunk_links);
                    }
                }
                return Ok(Some(assembled));
            }

            Ok(None)
        } else {
            // New capture session.
            if total == 0 || index >= total {
                return Err(LinksError::IndexOutOfRange { index, total });
            }

            let new_links_count = links.len();
            if new_links_count > self.limits.max_links_per_capture {
                return Err(LinksError::MaxLinksExceeded {
                    count: new_links_count,
                    limit: self.limits.max_links_per_capture,
                });
            }

            let new_bytes: usize = links.iter().map(|l| l.len()).sum();
            if new_bytes > self.limits.max_bytes_per_capture {
                return Err(LinksError::MaxBytesExceeded {
                    bytes: new_bytes,
                    limit: self.limits.max_bytes_per_capture,
                });
            }

            if self.captures.len() >= self.limits.max_concurrent_captures {
                return Err(LinksError::MaxConcurrentCapturesExceeded {
                    limit: self.limits.max_concurrent_captures,
                });
            }

            if total == 1 {
                // Immediate single-chunk completion.
                return Ok(Some(links));
            }

            let mut chunks = HashMap::new();
            chunks.insert(index, links);
            let active = ActiveCapture {
                total,
                chunks,
                total_links: new_links_count,
                total_bytes: new_bytes,
                last_updated_ms: now,
            };
            self.captures.insert(capture_id, active);
            Ok(None)
        }
    }

    /// Convenience helper for ingesting a chunk message.
    pub fn ingest(&mut self, chunk: ChunkMessage) -> Result<Option<Vec<String>>, LinksError> {
        self.push(chunk)
    }

    /// Ingest a [`serde_json::Value`] representing the chunk message.
    pub fn push_json(&mut self, value: &Value) -> Result<Option<Vec<String>>, LinksError> {
        let chunk: ChunkMessage =
            serde_json::from_value(value.clone()).map_err(|err| LinksError::InvalidPayload {
                detail: err.to_string(),
            })?;
        self.push(chunk)
    }

    /// Convenience helper for ingesting a [`serde_json::Value`].
    pub fn ingest_json(&mut self, value: &Value) -> Result<Option<Vec<String>>, LinksError> {
        self.push_json(value)
    }
}
