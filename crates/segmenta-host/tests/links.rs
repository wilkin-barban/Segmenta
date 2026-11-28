//! Tests for chunked links reassembly in `segmenta-host`.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use segmenta_host::links::{
    ChunkMessage, ChunkParams, Clock, LinksError, LinksLimits, LinksPart, METHOD_LINKS_CHUNK,
};
use serde_json::json;

struct ManualClock {
    now_ms: AtomicU64,
}

impl ManualClock {
    fn new(initial_ms: u64) -> Self {
        Self {
            now_ms: AtomicU64::new(initial_ms),
        }
    }

    fn advance(&self, duration: Duration) {
        self.now_ms
            .fetch_add(duration.as_millis() as u64, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn elapsed_ms(&self) -> u64 {
        self.now_ms.load(Ordering::SeqCst)
    }
}

fn make_chunk(capture_id: &str, index: usize, total: usize, links: &[&str]) -> ChunkMessage {
    ChunkMessage {
        method: METHOD_LINKS_CHUNK.to_string(),
        params: ChunkParams {
            capture_id: capture_id.to_string(),
            index,
            total,
            links: links.iter().map(|s| s.to_string()).collect(),
        },
    }
}

#[test]
fn chunks_arriving_out_of_order_reassemble_into_original_list() {
    let mut part = LinksPart::new(LinksLimits::default());

    // Send index 2 of 3
    let res = part
        .push(make_chunk("cap-1", 2, 3, &["https://example.com/c"]))
        .expect("push chunk 2");
    assert_eq!(res, None);

    // Send index 0 of 3
    let res = part
        .push(make_chunk("cap-1", 0, 3, &["https://example.com/a"]))
        .expect("push chunk 0");
    assert_eq!(res, None);

    // Send index 1 of 3
    let complete = part
        .push(make_chunk("cap-1", 1, 3, &["https://example.com/b"]))
        .expect("push chunk 1");
    assert_eq!(
        complete,
        Some(vec![
            "https://example.com/a".to_string(),
            "https://example.com/b".to_string(),
            "https://example.com/c".to_string(),
        ])
    );
}

#[test]
fn duplicate_chunk_does_not_duplicate_links() {
    let mut part = LinksPart::new(LinksLimits::default());

    // Send chunk 0
    let res = part
        .push(make_chunk("cap-dup", 0, 2, &["https://example.com/1"]))
        .expect("push chunk 0");
    assert_eq!(res, None);

    // Send chunk 0 again (duplicate chunk)
    let res = part
        .push(make_chunk("cap-dup", 0, 2, &["https://example.com/1"]))
        .expect("duplicate chunk 0");
    assert_eq!(res, None);

    // Send chunk 1
    let complete = part
        .push(make_chunk("cap-dup", 1, 2, &["https://example.com/2"]))
        .expect("push chunk 1");
    assert_eq!(
        complete,
        Some(vec![
            "https://example.com/1".to_string(),
            "https://example.com/2".to_string(),
        ])
    );
}

#[test]
fn out_of_range_index_and_mismatched_total_are_refused() {
    let mut part = LinksPart::new(LinksLimits::default());

    // Index == total is out of range
    let err = part
        .push(make_chunk("cap-oor", 3, 3, &["https://example.com/1"]))
        .unwrap_err();
    assert_eq!(err, LinksError::IndexOutOfRange { index: 3, total: 3 });

    // Index > total is out of range
    let err = part
        .push(make_chunk("cap-oor", 5, 3, &["https://example.com/1"]))
        .unwrap_err();
    assert_eq!(err, LinksError::IndexOutOfRange { index: 5, total: 3 });

    // Total == 0 is out of range
    let err = part
        .push(make_chunk("cap-oor", 0, 0, &["https://example.com/1"]))
        .unwrap_err();
    assert_eq!(err, LinksError::IndexOutOfRange { index: 0, total: 0 });

    // Mismatched total:
    // First chunk establishes total = 3
    let res = part
        .push(make_chunk("cap-mismatch", 0, 3, &["https://example.com/1"]))
        .expect("push chunk 0");
    assert_eq!(res, None);

    // Second chunk arrives with total = 4
    let err = part
        .push(make_chunk("cap-mismatch", 1, 4, &["https://example.com/2"]))
        .unwrap_err();
    assert_eq!(
        err,
        LinksError::MismatchedTotal {
            expected: 3,
            actual: 4,
        }
    );
}

#[test]
fn link_count_bound_and_byte_bound_each_trip_with_own_typed_error() {
    // 1. Link count bound
    let limits = LinksLimits {
        max_links_per_capture: 3,
        max_bytes_per_capture: 10_000,
        max_concurrent_captures: 10,
        expiry: Duration::from_secs(60),
    };
    let mut part = LinksPart::new(limits);

    let res = part
        .push(make_chunk(
            "cap-links",
            0,
            2,
            &["https://a.com", "https://b.com"],
        ))
        .expect("push chunk 0");
    assert_eq!(res, None);

    // Second chunk would make total links 4, exceeding limit 3
    let err = part
        .push(make_chunk(
            "cap-links",
            1,
            2,
            &["https://c.com", "https://d.com"],
        ))
        .unwrap_err();
    assert_eq!(err, LinksError::MaxLinksExceeded { count: 4, limit: 3 });

    // 2. Byte bound
    let limits = LinksLimits {
        max_links_per_capture: 100,
        max_bytes_per_capture: 40,
        max_concurrent_captures: 10,
        expiry: Duration::from_secs(60),
    };
    let mut part = LinksPart::new(limits);

    let long_url = "https://example.com/path/that/is/long/12345";
    assert!(long_url.len() > 40);
    let err = part
        .push(make_chunk("cap-bytes", 0, 1, &[long_url]))
        .unwrap_err();
    assert_eq!(
        err,
        LinksError::MaxBytesExceeded {
            bytes: long_url.len(),
            limit: 40,
        }
    );
}

#[test]
fn expired_capture_is_refused_with_expiry_reason() {
    let clock = Arc::new(ManualClock::new(1_000));
    let limits = LinksLimits {
        max_links_per_capture: 100,
        max_bytes_per_capture: 10_000,
        max_concurrent_captures: 10,
        expiry: Duration::from_millis(500),
    };
    let mut part = LinksPart::with_clock(limits, clock.clone());

    // Chunk 0 arrives at t = 1000 ms
    let res = part
        .push(make_chunk("cap-exp", 0, 2, &["https://example.com/1"]))
        .expect("push chunk 0");
    assert_eq!(res, None);

    // Advance clock past 500 ms expiry
    clock.advance(Duration::from_millis(600));

    // Chunk 1 arrives at t = 1600 ms -> expired!
    let err = part
        .push(make_chunk("cap-exp", 1, 2, &["https://example.com/2"]))
        .unwrap_err();

    match err {
        LinksError::Expired { capture_id, reason } => {
            assert_eq!(capture_id, "cap-exp");
            assert!(
                reason.to_lowercase().contains("expired"),
                "reason should name expiry: {reason}"
            );
        }
        other => panic!("expected LinksError::Expired, got: {other:?}"),
    }
}

#[test]
fn concurrent_capture_bound_refuses_one_more_capture_rather_than_growing_without_limit() {
    let limits = LinksLimits {
        max_links_per_capture: 100,
        max_bytes_per_capture: 10_000,
        max_concurrent_captures: 2,
        expiry: Duration::from_secs(60),
    };
    let mut part = LinksPart::new(limits);

    // Capture 1 starts
    let res = part
        .push(make_chunk("c1", 0, 2, &["https://example.com/1"]))
        .expect("push c1");
    assert_eq!(res, None);
    assert_eq!(part.active_captures(), 1);

    // Capture 2 starts
    let res = part
        .push(make_chunk("c2", 0, 2, &["https://example.com/2"]))
        .expect("push c2");
    assert_eq!(res, None);
    assert_eq!(part.active_captures(), 2);

    // Capture 3 attempts to start when capacity is 2 -> refused!
    let err = part
        .push(make_chunk("c3", 0, 2, &["https://example.com/3"]))
        .unwrap_err();
    assert_eq!(err, LinksError::MaxConcurrentCapturesExceeded { limit: 2 });
    assert_eq!(part.active_captures(), 2);

    // Existing active capture "c1" can still receive chunks
    let c1_res = part
        .push(make_chunk("c1", 1, 2, &["https://example.com/1b"]))
        .expect("complete c1");
    assert!(c1_res.is_some());
    // "c1" is now completed, active captures drops to 1
    assert_eq!(part.active_captures(), 1);

    // Now capture "c3" can be started
    let res = part
        .push(make_chunk("c3", 0, 2, &["https://example.com/3"]))
        .expect("push c3 after c1 completed");
    assert_eq!(res, None);
    assert_eq!(part.active_captures(), 2);
}

#[test]
fn json_message_shape_roundtrip_and_push_json() {
    let mut part = LinksPart::new(LinksLimits::default());
    let raw = json!({
        "method": "links.chunk",
        "params": {
            "capture_id": "json-test",
            "index": 0,
            "total": 1,
            "links": ["https://example.com/only"]
        }
    });

    let res = part.push_json(&raw).expect("push_json");
    assert_eq!(res, Some(vec!["https://example.com/only".to_string()]));
}

#[test]
fn wrong_method_in_json_is_refused() {
    let mut part = LinksPart::new(LinksLimits::default());
    let raw = json!({
        "method": "not.links.chunk",
        "params": {
            "capture_id": "json-test",
            "index": 0,
            "total": 1,
            "links": ["https://example.com/only"]
        }
    });

    let err = part.push_json(&raw).unwrap_err();
    assert_eq!(
        err,
        LinksError::InvalidMethod {
            method: "not.links.chunk".to_string()
        }
    );
}

#[test]
fn evicted_capture_returns_typed_error_naming_reason() {
    let mut part = LinksPart::new(LinksLimits::default());
    let res = part
        .push(make_chunk("evict-me", 0, 2, &["https://example.com/1"]))
        .expect("push chunk 0");
    assert_eq!(res, None);

    assert!(part.evict("evict-me", "cancelled by user"));

    let err = part
        .push(make_chunk("evict-me", 1, 2, &["https://example.com/2"]))
        .unwrap_err();
    assert_eq!(
        err,
        LinksError::Evicted {
            capture_id: "evict-me".to_string(),
            reason: "cancelled by user".to_string(),
        }
    );
}

#[test]
fn expired_capture_pruned_before_subsequent_chunk_still_reports_expired() {
    let clock = Arc::new(ManualClock::new(1_000));
    let limits = LinksLimits {
        max_links_per_capture: 100,
        max_bytes_per_capture: 10_000,
        max_concurrent_captures: 10,
        expiry: Duration::from_millis(500),
    };
    let mut part = LinksPart::with_clock(limits, clock.clone());

    // cap-a starts at t=1000
    part.push(make_chunk("cap-a", 0, 2, &["https://a.com/1"]))
        .expect("push cap-a chunk 0");

    // Advance clock past expiry
    clock.advance(Duration::from_millis(600));

    // cap-b starts at t=1600; this prune cleans up cap-a
    part.push(make_chunk("cap-b", 0, 1, &["https://b.com/1"]))
        .expect("push cap-b chunk 0");

    // cap-a sends chunk 1; it should still return Expired!
    let err = part
        .push(make_chunk("cap-a", 1, 2, &["https://a.com/2"]))
        .unwrap_err();
    match err {
        LinksError::Expired { capture_id, .. } => {
            assert_eq!(capture_id, "cap-a");
        }
        other => panic!("expected Expired, got {other:?}"),
    }
}
