//! Retry/backoff arithmetic and `Retry-After` parsing. Spec S8.

use std::time::{Duration, SystemTime};

use segmenta_core::retry::{backoff_secs, parse_retry_after, BACKOFF_MAX_SECS, MAX_RETRIES};

#[test]
fn backoff_is_exponential_with_a_cap() {
    assert!((backoff_secs(0, 0.0) - 1.0).abs() < 1e-9);
    assert!((backoff_secs(1, 0.0) - 2.0).abs() < 1e-9);
    assert!((backoff_secs(3, 0.0) - 8.0).abs() < 1e-9);
    assert!((backoff_secs(10, 0.0) - BACKOFF_MAX_SECS).abs() < 1e-9);
    assert_eq!(MAX_RETRIES, 5);
}

#[test]
fn jitter_stays_within_half_of_the_wait() {
    for attempt in 0..8 {
        let base = backoff_secs(attempt, 0.0);
        assert!((backoff_secs(attempt, -0.5) - base * 0.5).abs() < 1e-9);
        assert!((backoff_secs(attempt, 0.5) - base * 1.5).abs() < 1e-9);
    }
}

#[test]
fn retry_after_accepts_seconds() {
    let now = SystemTime::UNIX_EPOCH;
    assert_eq!(
        parse_retry_after(Some("2"), now),
        Some(Duration::from_secs(2))
    );
    assert_eq!(
        parse_retry_after(Some("0"), now),
        Some(Duration::from_secs(0))
    );
    assert_eq!(
        parse_retry_after(Some(" 7 "), now),
        Some(Duration::from_secs(7))
    );
}

#[test]
fn retry_after_accepts_http_dates() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    let future = httpdate::fmt_http_date(now + Duration::from_secs(30));
    let parsed = parse_retry_after(Some(&future), now).expect("a date is accepted");
    assert!(
        parsed.as_secs() >= 29 && parsed.as_secs() <= 30,
        "got {parsed:?}"
    );
}

#[test]
fn retry_after_falls_back_to_one_second() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    let past = httpdate::fmt_http_date(now - Duration::from_secs(60));
    assert_eq!(
        parse_retry_after(Some(&past), now),
        Some(Duration::from_secs(1))
    );
    assert_eq!(
        parse_retry_after(Some("soon"), now),
        Some(Duration::from_secs(1))
    );
    assert_eq!(
        parse_retry_after(Some(""), now),
        Some(Duration::from_secs(1))
    );
}

#[test]
fn retry_after_is_absent_without_the_header() {
    assert_eq!(parse_retry_after(None, SystemTime::UNIX_EPOCH), None);
}
