//! Download engine integration tests against the local range server.
//!
//! Covers the engine specs S3-S12: probing, initial segmentation, hot split,
//! resume with `If-Range`, stale-cache restart, `Retry-After`, terminal statuses,
//! checksum verification, truncated bodies, the global token bucket and the
//! zero-byte case. No external network, no `$HOME`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use segmenta_core::checksum;
use segmenta_core::engine::{run, Canceller, DownloadSpec, EngineConfig, EngineEvent, TaskOutcome};
use segmenta_core::planner::SegmentRange;
use segmenta_core::probe::HttpConfig;
use segmenta_core::testserver::{RangeServer, ServerConfig};
use tokio::sync::mpsc;
use url::Url;

const MB: usize = 1024 * 1024;

fn payload(size: usize) -> Vec<u8> {
    (0..size).map(|i| ((i * 31 + 7) % 251) as u8).collect()
}

/// A different byte pattern, so a restart is visible in the file content.
fn other_payload(size: usize) -> Vec<u8> {
    (0..size).map(|i| ((i * 7 + 3) % 241) as u8).collect()
}

fn local_cfg(connections: u32) -> EngineConfig {
    EngineConfig {
        connections,
        http: HttpConfig {
            allow_local: true,
            ..HttpConfig::default()
        },
        ..EngineConfig::default()
    }
}

fn spec_for(url: Url, path: PathBuf) -> DownloadSpec {
    DownloadSpec {
        url,
        path,
        expected_sha256: None,
        resume: None,
        plan_override: None,
    }
}

/// Run one download to its end and collect every emitted event.
async fn run_to_end(cfg: EngineConfig, spec: DownloadSpec) -> (TaskOutcome, Vec<EngineEvent>) {
    let (tx, mut rx) = mpsc::channel(8192);
    let collector = tokio::spawn(async move {
        let mut events = Vec::new();
        while let Some(event) = rx.recv().await {
            events.push(event);
        }
        events
    });
    let outcome = run(cfg, spec, tx, Canceller::new()).await;
    (outcome, collector.await.expect("event collector"))
}

#[tokio::test]
async fn standalone_http_unlimited_rates_download_the_whole_file() {
    let body = payload(MB);
    let server = RangeServer::start(ServerConfig {
        body: body.clone(),
        ..ServerConfig::default()
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    for (index, rate) in [None, Some(0)].into_iter().enumerate() {
        let mut cfg = local_cfg(1);
        cfg.speed_limit_bps = rate;
        let path = dir.path().join(format!("unlimited-{index}.bin"));
        let (outcome, _) = tokio::time::timeout(
            Duration::from_secs(5),
            run_to_end(cfg, spec_for(server.url(), path.clone())),
        )
        .await
        .expect("unlimited HTTP download should complete promptly");
        expect_completed(outcome);
        assert_eq!(std::fs::read(path).unwrap(), body);
    }
    server.shutdown().await;
}

/// Run one download and cancel it as soon as the first 250 ms progress tick lands.
async fn run_until_first_progress(
    cfg: EngineConfig,
    spec: DownloadSpec,
) -> (TaskOutcome, Vec<EngineEvent>) {
    let cancel = Canceller::new();
    let (tx, mut rx) = mpsc::channel(8192);
    let stop = cancel.clone();
    let collector = tokio::spawn(async move {
        let mut events = Vec::new();
        while let Some(event) = rx.recv().await {
            if matches!(event, EngineEvent::Progress { .. }) {
                stop.cancel();
            }
            events.push(event);
        }
        events
    });
    let outcome = run(cfg, spec, tx, cancel).await;
    (outcome, collector.await.expect("event collector"))
}

/// Every segment range the event stream reported, after applying every split.
fn final_ranges(events: &[EngineEvent]) -> Vec<(u64, u64)> {
    let mut live: HashMap<u32, (u64, u64)> = HashMap::new();
    for event in events {
        match event {
            EngineEvent::SegmentStarted { id, start, end } => {
                live.insert(*id, (*start, *end));
            }
            EngineEvent::SegmentSplit {
                donor_id,
                new_id,
                midpoint,
                original_end,
            } => {
                if let Some(donor) = live.get_mut(donor_id) {
                    donor.1 = *midpoint;
                }
                live.insert(*new_id, (midpoint + 1, *original_end));
            }
            _ => {}
        }
    }
    let mut ranges: Vec<(u64, u64)> = live.into_values().collect();
    ranges.sort_unstable();
    ranges
}

/// Assert the reported ranges tile `[0, total)` exactly: no gap, no overlap.
#[track_caller]
fn assert_contiguous(ranges: &[(u64, u64)], total: u64) {
    assert!(!ranges.is_empty(), "no segment ranges were reported");
    assert_eq!(ranges[0].0, 0, "coverage must start at byte 0: {ranges:?}");
    for pair in ranges.windows(2) {
        assert_eq!(
            pair[0].1 + 1,
            pair[1].0,
            "segments must be adjacent with no gap or overlap: {ranges:?}"
        );
    }
    assert_eq!(
        ranges[ranges.len() - 1].1,
        total - 1,
        "coverage must end at the last byte: {ranges:?}"
    );
}

/// Assert that every published snapshot tiles `[0, total)` and that no `next` jumps
/// past its own `end`, which is the offset invariant a resume depends on.
#[track_caller]
fn assert_snapshots_are_sound(events: &[EngineEvent], total: u64) {
    for event in events {
        let segments = match event {
            EngineEvent::Progress { segments, .. } | EngineEvent::Checkpoint { segments, .. } => {
                segments
            }
            _ => continue,
        };
        let mut ranges: Vec<(u64, u64)> = Vec::new();
        for segment in segments {
            assert!(
                segment.next <= segment.end + 1 && segment.next >= segment.start,
                "segment offsets out of range: {segment:?}"
            );
            ranges.push((segment.start, segment.end));
        }
        ranges.sort_unstable();
        if !ranges.is_empty() {
            assert_contiguous(&ranges, total);
        }
    }
}

fn max_progress(events: &[EngineEvent]) -> u64 {
    events
        .iter()
        .filter_map(|event| match event {
            EngineEvent::Progress {
                downloaded_bytes, ..
            } => Some(*downloaded_bytes),
            _ => None,
        })
        .max()
        .unwrap_or(0)
}

#[track_caller]
fn expect_completed(outcome: TaskOutcome) -> (u64, Option<bool>) {
    match outcome {
        TaskOutcome::Completed {
            total_bytes,
            checksum_ok,
            ..
        } => (total_bytes, checksum_ok),
        other => panic!("expected Completed, got {other:?}"),
    }
}

// --- S3/S4: nominal segmented download -------------------------------------

#[tokio::test]
async fn segmented_download_over_eight_connections_matches_the_source() {
    let body = payload(4 * MB);
    let server = RangeServer::with_body(body.clone()).await;
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("file.bin");

    let total = body.len() as u64;
    let chunk = total / 8;
    let ranges: Vec<SegmentRange> = (0..8u64)
        .map(|i| SegmentRange {
            start: i * chunk,
            end: if i == 7 {
                total - 1
            } else {
                (i + 1) * chunk - 1
            },
        })
        .collect();
    let spec = DownloadSpec {
        plan_override: Some(ranges),
        expected_sha256: Some(checksum::sha256_hex(&body)),
        ..spec_for(server.url(), path.clone())
    };

    let (outcome, events) = run_to_end(local_cfg(8), spec).await;

    let (bytes, checksum_ok) = expect_completed(outcome);
    assert_eq!(bytes, total);
    assert_eq!(checksum_ok, Some(true));
    assert_eq!(
        checksum::sha256_hex(&std::fs::read(&path).expect("read destination")),
        checksum::sha256_hex(&body)
    );
    let started = events
        .iter()
        .filter(|event| matches!(event, EngineEvent::SegmentStarted { .. }))
        .count();
    assert_eq!(started, 8, "every planned connection starts a worker");
    assert_contiguous(&final_ranges(&events), total);
    assert_snapshots_are_sound(&events, total);
    assert_eq!(
        max_progress(&events),
        total,
        "progress must reach the total size"
    );
    server.shutdown().await;
}

#[tokio::test]
async fn planned_segments_tile_the_file_without_gaps() {
    let body = payload(6 * MB);
    let server = RangeServer::with_body(body.clone()).await;
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("planned.bin");
    let spec = DownloadSpec {
        expected_sha256: Some(checksum::sha256_hex(&body)),
        ..spec_for(server.url(), path.clone())
    };

    let (outcome, events) = run_to_end(local_cfg(8), spec).await;

    let (total, checksum_ok) = expect_completed(outcome);
    assert_eq!(total, body.len() as u64);
    assert_eq!(checksum_ok, Some(true));
    // 5 MB <= S < 50 MB plans four connections (S4).
    let started = events
        .iter()
        .filter(|event| matches!(event, EngineEvent::SegmentStarted { .. }))
        .count();
    assert_eq!(started, 4);
    assert_contiguous(&final_ranges(&events), total);
    assert_eq!(std::fs::read(&path).expect("read destination"), body);
    server.shutdown().await;
}

// --- S3: fallbacks ---------------------------------------------------------

#[tokio::test]
async fn single_stream_fallback_writes_the_whole_file() {
    let body = payload(MB);
    let server = RangeServer::start(ServerConfig {
        body: body.clone(),
        supports_ranges: false,
        ..ServerConfig::default()
    })
    .await;
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("single.bin");
    let spec = DownloadSpec {
        expected_sha256: Some(checksum::sha256_hex(&body)),
        ..spec_for(server.url(), path.clone())
    };

    let (outcome, _events) = run_to_end(local_cfg(8), spec).await;

    let (total, checksum_ok) = expect_completed(outcome);
    assert_eq!(total, body.len() as u64);
    assert_eq!(checksum_ok, Some(true));
    assert_eq!(std::fs::read(&path).expect("read destination"), body);
    assert_eq!(
        server.request_count(),
        2,
        "one probe plus exactly one sequential body request"
    );
    server.shutdown().await;
}

#[tokio::test]
async fn chunked_response_streams_until_eof() {
    let body = payload(512 * 1024);
    let server = RangeServer::start(ServerConfig {
        body: body.clone(),
        chunked: true,
        ..ServerConfig::default()
    })
    .await;
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("chunked.bin");

    let (outcome, events) = run_to_end(local_cfg(8), spec_for(server.url(), path.clone())).await;

    let probed = events
        .iter()
        .find_map(|event| match event {
            EngineEvent::ProbingFinished {
                total_bytes,
                supports_ranges,
                ..
            } => Some((*total_bytes, *supports_ranges)),
            _ => None,
        })
        .expect("ProbingFinished event");
    assert_eq!(probed, (None, false), "chunked transfers declare no size");
    let (total, checksum_ok) = expect_completed(outcome);
    assert_eq!(total, body.len() as u64);
    assert_eq!(checksum_ok, None);
    assert_eq!(std::fs::read(&path).expect("read destination"), body);
    server.shutdown().await;
}

// --- S5: hot split ---------------------------------------------------------

#[tokio::test]
async fn hot_split_moves_bytes_to_an_idle_worker_without_gaps() {
    let body = payload(3 * MB);
    let total = body.len() as u64;
    let server = RangeServer::start(ServerConfig {
        body: body.clone(),
        // Keep the donor busy long enough for the handshake to be deterministic.
        slow: Some(Duration::from_millis(8)),
        ..ServerConfig::default()
    })
    .await;
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("split.bin");
    let spec = DownloadSpec {
        plan_override: Some(vec![
            SegmentRange {
                start: 0,
                end: 1024,
            },
            SegmentRange {
                start: 1025,
                end: total - 1,
            },
        ]),
        expected_sha256: Some(checksum::sha256_hex(&body)),
        ..spec_for(server.url(), path.clone())
    };

    let (outcome, events) = run_to_end(local_cfg(8), spec).await;

    let (bytes, _) = expect_completed(outcome);
    assert_eq!(bytes, total);
    assert_eq!(
        std::fs::read(&path).expect("read destination"),
        body,
        "the split halves must not lose or duplicate a byte"
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, EngineEvent::SegmentSplit { .. })),
        "the idle worker must take half of the running donor"
    );
    assert_contiguous(&final_ranges(&events), total);
    // Every progress/checkpoint snapshot must already be complete: the split handshake
    // registers the new range with the donor's clamp, so no observer sees a hole.
    assert_snapshots_are_sound(&events, total);
    server.shutdown().await;
}

// --- S7: pause and resume --------------------------------------------------

#[tokio::test]
async fn pause_then_resume_continues_from_the_published_offsets() {
    let body = payload(2 * MB);
    let server = RangeServer::start(ServerConfig {
        body: body.clone(),
        slow: Some(Duration::from_millis(10)),
        chunk_size: 16 * 1024,
        ..ServerConfig::default()
    })
    .await;
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("resumable.bin");
    let url = server.url();

    let (outcome, events) =
        run_until_first_progress(local_cfg(8), spec_for(url.clone(), path.clone())).await;

    let TaskOutcome::Paused { state } = outcome else {
        panic!("expected Paused, got {outcome:?}");
    };
    assert!(state.supports_ranges);
    assert_eq!(state.etag.as_deref(), Some("\"v1\""));
    assert_eq!(state.segments.len(), 1, "2 MB plans a single connection");
    let written: u64 = state
        .segments
        .iter()
        .map(|segment| segment.next - segment.start)
        .sum();
    assert!(
        written > 0 && written < body.len() as u64,
        "the pause must land mid-stream, wrote {written}"
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, EngineEvent::Checkpoint { .. })),
        "pausing must checkpoint before returning the state"
    );

    let resumed = DownloadSpec {
        resume: Some(state),
        ..spec_for(url, path.clone())
    };
    let (outcome, resumed_events) = run_to_end(local_cfg(8), resumed).await;

    expect_completed(outcome);
    assert_eq!(std::fs::read(&path).expect("read destination"), body);
    assert!(
        !resumed_events
            .iter()
            .any(|event| matches!(event, EngineEvent::Restarted { .. })),
        "a matching validator must continue instead of restarting"
    );
    assert_eq!(
        resumed_events
            .iter()
            .filter(|event| matches!(event, EngineEvent::SegmentStarted { .. }))
            .count(),
        1,
        "the resumed plan keeps its single segment"
    );
    let asked_from_offset = server.requests().iter().any(|request| {
        request
            .header("range")
            .and_then(|value| value.trim_start_matches("bytes=").split('-').next())
            .and_then(|start| start.parse::<u64>().ok())
            .map(|start| start > 1)
            .unwrap_or(false)
    });
    assert!(
        asked_from_offset,
        "the resumed worker must request `Range: bytes=<next>-...`"
    );
    server.shutdown().await;
}

#[tokio::test]
async fn stale_validator_restarts_from_zero_and_downloads_the_new_body() {
    let body = payload(MB);
    let server = RangeServer::start(ServerConfig {
        body: body.clone(),
        slow: Some(Duration::from_millis(10)),
        chunk_size: 16 * 1024,
        etag: Some("\"v1\"".to_string()),
        ..ServerConfig::default()
    })
    .await;
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("stale.bin");
    let url = server.url();

    let (outcome, _events) =
        run_until_first_progress(local_cfg(8), spec_for(url.clone(), path.clone())).await;
    let TaskOutcome::Paused { state } = outcome else {
        panic!("expected Paused, got {outcome:?}");
    };

    let new_body = other_payload(256 * 1024);
    server.replace_body(new_body.clone(), "\"v2\"");
    server.update(|cfg| cfg.slow = None);

    let resumed = DownloadSpec {
        resume: Some(state),
        ..spec_for(url, path.clone())
    };
    let (outcome, events) = run_to_end(local_cfg(8), resumed).await;

    let (total, _) = expect_completed(outcome);
    assert_eq!(total, new_body.len() as u64);
    assert!(
        events
            .iter()
            .any(|event| matches!(event, EngineEvent::Restarted { .. })),
        "a changed validator must restart the download"
    );
    assert_eq!(
        std::fs::read(&path).expect("read destination"),
        new_body,
        "no byte of the old generation may survive the restart"
    );
    server.shutdown().await;
}

// --- S8: retries and status classification --------------------------------

#[tokio::test]
async fn retry_after_is_honoured_before_retrying() {
    let body = payload(256 * 1024);
    let server = RangeServer::start(ServerConfig {
        body: body.clone(),
        force_status: Some(429),
        retry_after: Some("1".to_string()),
        ..ServerConfig::default()
    })
    .await;
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("throttled.bin");
    let spec = spec_for(server.url(), path.clone());

    let started = Instant::now();
    let ((outcome, _events), ()) = tokio::join!(run_to_end(local_cfg(8), spec), async {
        loop {
            if server.request_count() > 0 {
                server.update(|cfg| cfg.force_status = None);
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    });
    let elapsed = started.elapsed();

    expect_completed(outcome);
    assert!(
        elapsed >= Duration::from_secs(1),
        "`Retry-After: 1` must be slept before the retry, took {elapsed:?}"
    );
    assert_eq!(std::fs::read(&path).expect("read destination"), body);
    server.shutdown().await;
}

#[tokio::test]
async fn resource_gone_fails_without_retrying() {
    let server = RangeServer::start(ServerConfig {
        body: payload(1024),
        force_status: Some(404),
        ..ServerConfig::default()
    })
    .await;
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("gone.bin");

    let (outcome, _events) = run_to_end(local_cfg(8), spec_for(server.url(), path.clone())).await;

    match outcome {
        TaskOutcome::Failed { error } => {
            assert_eq!(error.err_name(), "ERR_RESOURCE_GONE", "got {error:?}");
        }
        other => panic!("expected Failed, got {other:?}"),
    }
    assert!(
        server.request_count() <= 2,
        "a 404 must not be retried, saw {} requests",
        server.request_count()
    );
    assert!(!path.exists(), "a failed download leaves no file behind");
    server.shutdown().await;
}

// --- S9: checksum ---------------------------------------------------------

#[tokio::test]
async fn checksum_mismatch_renames_the_file_to_corrupt() {
    let body = payload(MB);
    let server = RangeServer::with_body(body.clone()).await;
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("corrupt.bin");
    let spec = DownloadSpec {
        expected_sha256: Some(checksum::sha256_hex(b"not the file")),
        ..spec_for(server.url(), path.clone())
    };

    let (outcome, events) = run_to_end(local_cfg(4), spec).await;

    match outcome {
        TaskOutcome::Failed { error } => {
            assert_eq!(error.err_name(), "ERR_CHECKSUM_MISMATCH", "got {error:?}");
        }
        other => panic!("expected Failed, got {other:?}"),
    }
    assert!(
        events
            .iter()
            .any(|event| matches!(event, EngineEvent::Verifying)),
        "the engine must announce verification"
    );
    assert!(!path.exists(), "the corrupt file must not keep its name");
    let mut corrupt = path.clone().into_os_string();
    corrupt.push(".corrupt");
    assert!(
        PathBuf::from(corrupt).exists(),
        "the corrupt file must be renamed with a .corrupt suffix"
    );
    server.shutdown().await;
}

// --- S6/S8: truncated body ------------------------------------------------

#[tokio::test]
async fn truncated_body_is_retried_from_the_last_written_offset() {
    let body = payload(256 * 1024);
    let server = RangeServer::start(ServerConfig {
        body: body.clone(),
        cut_after: Some(1024),
        ..ServerConfig::default()
    })
    .await;
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("truncated.bin");
    let spec = DownloadSpec {
        expected_sha256: Some(checksum::sha256_hex(&body)),
        ..spec_for(server.url(), path.clone())
    };

    let ((outcome, _events), ()) = tokio::join!(run_to_end(local_cfg(4), spec), async {
        // Serve the full body from the first retry onwards.
        loop {
            if server.request_count() >= 2 {
                server.update(|cfg| cfg.cut_after = None);
                return;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    });

    expect_completed(outcome);
    assert!(
        server.request_count() >= 2,
        "a cut body must be retried, saw {} requests",
        server.request_count()
    );
    assert_eq!(std::fs::read(&path).expect("read destination"), body);
    server.shutdown().await;
}

// --- S10: global speed limit ----------------------------------------------

#[tokio::test]
async fn global_speed_limit_throttles_the_download() {
    let body = payload(512 * 1024);
    let server = RangeServer::with_body(body.clone()).await;
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("slow.bin");
    // 512 KiB at 1 MiB/s: the bucket starts with a 256 KiB burst, so the remaining
    // 256 KiB need a quarter second at least.
    let cfg = EngineConfig {
        speed_limit_bps: Some(1024 * 1024),
        ..local_cfg(4)
    };

    let started = Instant::now();
    let (outcome, _events) = run_to_end(cfg, spec_for(server.url(), path.clone())).await;
    let elapsed = started.elapsed();

    expect_completed(outcome);
    assert_eq!(std::fs::read(&path).expect("read destination"), body);
    assert!(
        elapsed >= Duration::from_millis(200),
        "a 1 MiB/s limit must slow 512 KiB down, took {elapsed:?}"
    );
    server.shutdown().await;
}

// --- S3: zero-byte entity -------------------------------------------------

#[tokio::test]
async fn zero_byte_download_creates_an_empty_file() {
    let server = RangeServer::with_body(Vec::new()).await;
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("empty.bin");

    let (outcome, _events) = run_to_end(local_cfg(4), spec_for(server.url(), path.clone())).await;

    let (total, checksum_ok) = expect_completed(outcome);
    assert_eq!(total, 0);
    assert_eq!(checksum_ok, None);
    assert_eq!(
        std::fs::metadata(&path).expect("destination exists").len(),
        0
    );
    server.shutdown().await;
}

/// A host that answers 429 lowers the shared limit, and the next generation starts smaller.
///
/// The registry is observed through the same `Arc` the engine used, so this is the wiring
/// and not a unit test of the module (which has its own suite).
#[tokio::test]
async fn a_429_lowers_the_host_limit_shared_by_the_daemon() {
    use segmenta_core::hostlimits::{HostLimitPolicy, HostLimits};
    use std::sync::Arc;
    use std::time::Duration;

    struct Frozen;
    impl segmenta_core::hostlimits::Clock for Frozen {
        fn elapsed_ms(&self) -> u64 {
            0
        }
    }

    let limits = Arc::new(HostLimits::new(
        HostLimitPolicy {
            initial: 8,
            ceiling: 128,
            penalty: Duration::from_secs(60),
            decay: Duration::from_secs(15),
        },
        Arc::new(Frozen),
    ));
    let server = RangeServer::start(ServerConfig {
        body: payload(2 * MB),
        force_status: Some(429),
        ..ServerConfig::default()
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = local_cfg(4);
    cfg.max_retries = 1;
    cfg.host_limits = Some(Arc::clone(&limits));
    let host = server.url().host_str().unwrap().to_string();

    let outcome = run_to_end(cfg, spec_for(server.url(), dir.path().join("f.bin")))
        .await
        .0;
    assert!(
        matches!(
            outcome,
            TaskOutcome::Failed { .. } | TaskOutcome::Paused { .. }
        ),
        "a permanent 429 must not look like success: {outcome:?}"
    );
    // The probe tries once and retries once (max_retries = 1), so the host refused twice
    // and each refusal halves: 8 -> 4 -> 2. Going lower with more refusals is the point.
    assert_eq!(
        limits.limit(&host),
        2,
        "every refusal must halve the shared limit"
    );
}

/// The per-host limit caps the workers of a generation even when the caller asks for more.
#[tokio::test]
async fn a_lowered_host_limit_caps_the_workers_of_the_next_generation() {
    use segmenta_core::hostlimits::{HostLimitPolicy, HostLimits};
    use std::sync::Arc;
    use std::time::Duration;

    struct Frozen;
    impl segmenta_core::hostlimits::Clock for Frozen {
        fn elapsed_ms(&self) -> u64 {
            0
        }
    }

    let limits = Arc::new(HostLimits::new(
        HostLimitPolicy {
            initial: 8,
            ceiling: 128,
            penalty: Duration::from_secs(60),
            decay: Duration::from_secs(15),
        },
        Arc::new(Frozen),
    ));
    let server = RangeServer::start(ServerConfig {
        body: payload(8 * MB),
        slow: Some(Duration::from_millis(20)),
        ..ServerConfig::default()
    })
    .await;
    let host = server.url().host_str().unwrap().to_string();
    // Two refusals leave the host at two connections before the run starts.
    limits.on_throttled(&host);
    limits.on_throttled(&host);
    assert_eq!(limits.limit(&host), 2);

    let dir = tempfile::tempdir().unwrap();
    let mut cfg = local_cfg(8);
    cfg.host_limits = Some(Arc::clone(&limits));
    let (outcome, events) = run_to_end(cfg, spec_for(server.url(), dir.path().join("f.bin"))).await;
    assert!(
        matches!(outcome, TaskOutcome::Completed { .. }),
        "{outcome:?}"
    );
    // The plan is what bounds the workers: at most `limit` segments may start, and no
    // split may add one. Counting sockets would measure retries instead of concurrency.
    let started = events
        .iter()
        .filter(|event| matches!(event, EngineEvent::SegmentStarted { .. }))
        .count();
    let splits = events
        .iter()
        .filter(|event| matches!(event, EngineEvent::SegmentSplit { .. }))
        .count();
    assert!(
        started <= 2 && splits == 0,
        "a host limited to 2 started {started} segments with {splits} splits"
    );
}
