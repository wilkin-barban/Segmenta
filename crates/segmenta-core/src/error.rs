//! Engine error taxonomy.
//!
//! Every variant maps to the numeric `error_code` stored in SQLite and sent over
//! JSON-RPC `data.code`, plus the `ERR_*` identifier shown to users. Codes 1001-1010
//! come from OpenSpec 07; the engine codes 1021-1031 are documented in OpenSpec 06 §4.3.

use std::time::Duration;

/// Result alias used across the engine.
pub type Result<T> = std::result::Result<T, CoreError>;

/// A failure that stops or pauses a download.
#[derive(Debug, Clone, thiserror::Error)]
pub enum CoreError {
    /// The URL is not `http`/`https` or does not resolve.
    #[error("invalid URL: {0}")]
    InvalidUrl(String),
    /// DNS resolution succeeded but every address is private/link-local.
    #[error("blocked private address: {0}")]
    BlockedPrivateIp(String),
    /// More than ten redirects.
    #[error("too many redirects")]
    TooManyRedirects,
    /// 404/410: the resource is gone, never retried.
    #[error("resource gone: {0}")]
    ResourceGone(String),
    /// 401/403: credentials are missing or expired.
    #[error("authentication required")]
    AuthRequired,
    /// 407: the proxy demands credentials.
    #[error("proxy authentication required")]
    ProxyAuthRequired,
    /// `posix_fallocate` returned `ENOSPC`.
    #[error("disk full")]
    DiskFull,
    /// `EACCES`/`EPERM` on the destination directory.
    #[error("destination unwritable: {0}")]
    DestinationUnwritable(String),
    /// The server answered `416` for a range that is not the file end.
    #[error("range out of bounds")]
    RangeOutOfBounds,
    /// The final digest does not match the expected one.
    #[error("checksum mismatch")]
    ChecksumMismatch,
    /// Bytes on disk do not match the probed size.
    #[error("size mismatch")]
    SizeMismatch,
    /// A `200` body ended before `Content-Length`.
    #[error("truncated response")]
    TruncatedResponse,
    /// A response the engine cannot use (compressed transfer, unusable headers).
    #[error("unsupported response: {0}")]
    Unsupported(String),
    /// A header or `Content-Range` value is malformed.
    #[error("malformed header: {0}")]
    MalformedHeader(String),
    /// Transport failure (reset, timeout, DNS, TLS).
    #[error("network error: {0}")]
    Network(String),
    /// The task was cancelled or paused by the user.
    #[error("cancelled")]
    Cancelled,
    /// Local filesystem failure, kept cloneable for events and logs.
    #[error("io error: {message}")]
    Io {
        /// Original `ErrorKind`, used to map `EACCES` to `DESTINATION_UNWRITABLE`.
        kind: std::io::ErrorKind,
        /// Human-readable message.
        message: String,
    },
}

impl From<std::io::Error> for CoreError {
    fn from(err: std::io::Error) -> Self {
        CoreError::Io {
            kind: err.kind(),
            message: err.to_string(),
        }
    }
}

impl CoreError {
    /// Numeric code stored in `downloads.error_code` and sent as `data.code`.
    pub fn code(&self) -> i64 {
        match self {
            CoreError::InvalidUrl(_) => 1004,
            CoreError::DestinationUnwritable(_) => 1005,
            CoreError::DiskFull => 1006,
            CoreError::BlockedPrivateIp(_) => 1007,
            CoreError::ChecksumMismatch => 1008,
            CoreError::TooManyRedirects => 1021,
            CoreError::ResourceGone(_) => 1022,
            CoreError::AuthRequired => 1023,
            CoreError::ProxyAuthRequired => 1024,
            CoreError::RangeOutOfBounds => 1025,
            CoreError::SizeMismatch => 1026,
            CoreError::TruncatedResponse => 1027,
            CoreError::Unsupported(_) => 1028,
            CoreError::Network(_) => 1029,
            CoreError::Cancelled => 1030,
            CoreError::MalformedHeader(_) => 1032,
            CoreError::Io { kind, .. } => match kind {
                std::io::ErrorKind::PermissionDenied => 1005,
                _ => 1029,
            },
        }
    }

    /// Stable identifier for user-facing messages and logs.
    pub fn err_name(&self) -> &'static str {
        match self {
            CoreError::InvalidUrl(_) => "ERR_INVALID_URL",
            CoreError::BlockedPrivateIp(_) => "ERR_BLOCKED_PRIVATE_IP",
            CoreError::TooManyRedirects => "ERR_TOO_MANY_REDIRECTS",
            CoreError::ResourceGone(_) => "ERR_RESOURCE_GONE",
            CoreError::AuthRequired => "ERR_AUTH_REQUIRED",
            CoreError::ProxyAuthRequired => "ERR_PROXY_AUTH",
            CoreError::DiskFull => "ERR_DISK_FULL",
            CoreError::DestinationUnwritable(_) => "ERR_DESTINATION_UNWRITABLE",
            CoreError::RangeOutOfBounds => "ERR_RANGE_OUT_OF_BOUNDS",
            CoreError::ChecksumMismatch => "ERR_CHECKSUM_MISMATCH",
            CoreError::SizeMismatch => "ERR_SIZE_MISMATCH",
            CoreError::TruncatedResponse => "ERR_TRUNCATED_RESPONSE",
            CoreError::Unsupported(_) => "ERR_UNSUPPORTED",
            CoreError::MalformedHeader(_) => "ERR_MALFORMED_HEADER",
            CoreError::Network(_) => "ERR_NETWORK",
            CoreError::Cancelled => "ERR_CANCELLED",
            CoreError::Io { .. } => "ERR_IO",
        }
    }

    /// Underlying `ErrorKind` when this is an I/O failure.
    pub fn io_kind(&self) -> Option<std::io::ErrorKind> {
        match self {
            CoreError::Io { kind, .. } => Some(*kind),
            _ => None,
        }
    }

    /// Whether the worker may retry the same range.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            CoreError::Network(_) | CoreError::TruncatedResponse | CoreError::Io { .. }
        )
    }

    /// Whether the task should pause (and keep its state) instead of failing.
    pub fn pauses_task(&self) -> bool {
        matches!(
            self,
            CoreError::AuthRequired | CoreError::ProxyAuthRequired | CoreError::DiskFull
        )
    }
}

/// Retry decision derived from an HTTP status code.
#[derive(Debug, Clone)]
pub enum StatusDecision {
    /// Proceed with the body.
    Proceed,
    /// Retryable; the optional value is the server's `Retry-After`.
    Retry(Option<Duration>),
    /// Pause and ask the user for credentials.
    Pause(CoreError),
    /// Terminal failure.
    Fail(CoreError),
}

impl PartialEq for StatusDecision {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (StatusDecision::Proceed, StatusDecision::Proceed) => true,
            (StatusDecision::Retry(a), StatusDecision::Retry(b)) => a == b,
            (StatusDecision::Pause(a), StatusDecision::Pause(b)) => a.code() == b.code(),
            (StatusDecision::Fail(a), StatusDecision::Fail(b)) => a.code() == b.code(),
            _ => false,
        }
    }
}

impl StatusDecision {
    /// Classify one HTTP status code.
    pub fn from_status(status: u16) -> StatusDecision {
        match status {
            200 | 206 => StatusDecision::Proceed,
            416 => StatusDecision::Fail(CoreError::RangeOutOfBounds),
            429 | 500 | 502 | 503 | 504 => StatusDecision::Retry(None),
            401 | 403 => StatusDecision::Pause(CoreError::AuthRequired),
            407 => StatusDecision::Pause(CoreError::ProxyAuthRequired),
            404 | 410 => StatusDecision::Fail(CoreError::ResourceGone(status.to_string())),
            // Anything else is a server bug from the engine's point of view: do not loop.
            other => StatusDecision::Fail(CoreError::Unsupported(format!("HTTP {other}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_codes_match_openspec_07() {
        assert_eq!(CoreError::InvalidUrl("x".into()).code(), 1004);
        assert_eq!(CoreError::DestinationUnwritable("x".into()).code(), 1005);
        assert_eq!(CoreError::DiskFull.code(), 1006);
        assert_eq!(CoreError::BlockedPrivateIp("x".into()).code(), 1007);
        assert_eq!(CoreError::ChecksumMismatch.code(), 1008);
        assert_eq!(CoreError::DiskFull.err_name(), "ERR_DISK_FULL");
    }

    #[test]
    fn retryable_and_pausing_errors_are_disjoint_per_spec() {
        assert!(CoreError::Network("reset".into()).is_retryable());
        assert!(!CoreError::AuthRequired.is_retryable());
        assert!(CoreError::AuthRequired.pauses_task());
        assert!(CoreError::DiskFull.pauses_task());
        assert!(!CoreError::ResourceGone("404".into()).pauses_task());
    }

    #[test]
    fn status_classification_follows_matrix() {
        assert_eq!(StatusDecision::from_status(206), StatusDecision::Proceed);
        assert!(matches!(
            StatusDecision::from_status(429),
            StatusDecision::Retry(None)
        ));
        assert!(matches!(
            StatusDecision::from_status(503),
            StatusDecision::Retry(None)
        ));
        assert!(matches!(
            StatusDecision::from_status(401),
            StatusDecision::Pause(CoreError::AuthRequired)
        ));
        assert!(matches!(
            StatusDecision::from_status(407),
            StatusDecision::Pause(CoreError::ProxyAuthRequired)
        ));
        assert!(matches!(
            StatusDecision::from_status(404),
            StatusDecision::Fail(CoreError::ResourceGone(_))
        ));
        assert!(matches!(
            StatusDecision::from_status(416),
            StatusDecision::Fail(CoreError::RangeOutOfBounds)
        ));
    }
}
