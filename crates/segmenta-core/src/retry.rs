//! Retry policy: exponential backoff with jitter and `Retry-After` handling.
//!
//! Constants and formulas come from OpenSpec 06 §4.2.

use std::time::{Duration, SystemTime};

/// Base wait of the exponential series, in seconds.
pub const BACKOFF_BASE_SECS: f64 = 1.0;
/// Upper bound of one wait, in seconds.
pub const BACKOFF_MAX_SECS: f64 = 60.0;
/// Consecutive failures tolerated before the task pauses.
pub const MAX_RETRIES: u32 = 5;
/// Wait used when `Retry-After` is invalid, in the past, or empty.
pub const RETRY_AFTER_FALLBACK: Duration = Duration::from_secs(1);

/// `min(60, 1 * 2^attempt)` scaled by a jitter factor in `[-0.5, 0.5]`.
///
/// A jitter of `0.0` is the un-jittered series, which keeps the arithmetic testable.
pub fn backoff_secs(attempt: u32, jitter: f64) -> f64 {
    let exponent = f64::from(attempt.min(16));
    let base = (BACKOFF_BASE_SECS * 2f64.powf(exponent)).min(BACKOFF_MAX_SECS);
    (base * (1.0 + jitter.clamp(-0.5, 0.5))).max(0.0)
}

/// One wait drawn with random jitter, ready to be `tokio::time::sleep`-ed.
pub fn backoff_delay(attempt: u32) -> Duration {
    let magnitude = rand::random::<f64>() * 0.5;
    let jitter = if rand::random::<bool>() { magnitude } else { -magnitude };
    Duration::from_secs_f64(backoff_secs(attempt, jitter))
}

/// Parse a `Retry-After` value (delta-seconds or HTTP-date).
///
/// Returns `None` when the header is absent, and the 1 s fallback when the value is
/// invalid or already in the past, as OpenSpec 06 §3 requires.
pub fn parse_retry_after(value: Option<&str>, now: SystemTime) -> Option<Duration> {
    let raw = value?.trim();
    if let Ok(secs) = raw.parse::<u64>() {
        return Some(Duration::from_secs(secs));
    }
    if let Ok(when) = httpdate::parse_http_date(raw) {
        return Some(match when.duration_since(now) {
            Ok(wait) if !wait.is_zero() => wait,
            _ => RETRY_AFTER_FALLBACK,
        });
    }
    Some(RETRY_AFTER_FALLBACK)
}
