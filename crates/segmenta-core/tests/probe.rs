//! Probing against a local range server: 206, 200, chunked, 416, 429, redirects,
//! stale validators and compressed responses. Spec S3 and S11.

use std::time::Duration;

use segmenta_core::probe::{HttpConfig, ProbeOutcome, Session, TransferMode, probe};
use segmenta_core::testserver::{RangeServer, ServerConfig};
use reqwest::header::HeaderValue;
use url::Url;

fn local_config() -> HttpConfig {
    HttpConfig {
        allow_local: true,
        ..HttpConfig::default()
    }
}

fn payload(size: usize) -> Vec<u8> {
    (0..size).map(|i| (i % 251) as u8).collect()
}

#[tokio::test]
async fn probe_detects_range_support_and_sends_the_contract_headers() {
    let server = RangeServer::with_body(payload(1024)).await;
    let outcome = probe(&local_config(), &server.url()).await.unwrap();
    let ProbeOutcome::Ready(info) = outcome else {
        panic!("expected Ready, got {outcome:?}");
    };
    assert_eq!(info.mode, TransferMode::Segmented { total: 1024 });
    assert_eq!(info.etag.as_deref(), Some("\"v1\""));
    assert_eq!(info.content_type.as_deref(), Some("application/octet-stream"));
    assert!(info.content_disposition.is_some());
    assert_eq!(info.final_url, server.url());

    let request = &server.requests()[0];
    assert_eq!(request.header("range"), Some("bytes=0-1"));
    assert_eq!(request.header("accept-encoding"), Some("identity"));
    server.shutdown().await;
}

#[tokio::test]
async fn probe_falls_back_to_single_stream_when_ranges_are_ignored() {
    let server = RangeServer::start(ServerConfig {
        body: payload(4096),
        supports_ranges: false,
        ..ServerConfig::default()
    })
    .await;
    let ProbeOutcome::Ready(info) = probe(&local_config(), &server.url()).await.unwrap() else {
        panic!("expected Ready");
    };
    assert_eq!(info.mode, TransferMode::SingleStream { total: Some(4096) });
    server.shutdown().await;
}

#[tokio::test]
async fn probe_detects_chunked_transfers_without_a_length() {
    let server = RangeServer::start(ServerConfig {
        body: payload(8192),
        chunked: true,
        ..ServerConfig::default()
    })
    .await;
    let ProbeOutcome::Ready(info) = probe(&local_config(), &server.url()).await.unwrap() else {
        panic!("expected Ready");
    };
    assert_eq!(info.mode, TransferMode::SingleStream { total: None });
    server.shutdown().await;
}

#[tokio::test]
async fn probe_reads_retry_after_for_429() {
    let server = RangeServer::start(ServerConfig {
        body: payload(10),
        force_status: Some(429),
        retry_after: Some("2".to_string()),
        ..ServerConfig::default()
    })
    .await;
    let outcome = probe(&local_config(), &server.url()).await.unwrap();
    let ProbeOutcome::Retry { retry_after } = outcome else {
        panic!("expected Retry, got {outcome:?}");
    };
    assert_eq!(retry_after, Some(Duration::from_secs(2)));
    server.shutdown().await;
}

#[tokio::test]
async fn probe_classifies_server_failures_and_missing_resources() {
    for (status, expected) in [(503u16, "retry"), (404, "gone"), (410, "gone"), (401, "auth")] {
        let server = RangeServer::start(ServerConfig {
            body: payload(10),
            force_status: Some(status),
            ..ServerConfig::default()
        })
        .await;
        let outcome = probe(&local_config(), &server.url()).await.unwrap();
        match (expected, outcome) {
            ("retry", ProbeOutcome::Retry { .. }) => {}
            ("gone", ProbeOutcome::Fail(err)) if err.err_name() == "ERR_RESOURCE_GONE" => {}
            ("auth", ProbeOutcome::Pause(err)) if err.err_name() == "ERR_AUTH_REQUIRED" => {}
            (_, other) => panic!("status {status}: unexpected {other:?}"),
        }
        server.shutdown().await;
    }
}

#[tokio::test]
async fn probe_rejects_a_compressed_response() {
    let server = RangeServer::start(ServerConfig {
        body: payload(256),
        content_encoding: Some("gzip".to_string()),
        ..ServerConfig::default()
    })
    .await;
    let outcome = probe(&local_config(), &server.url()).await.unwrap();
    let ProbeOutcome::Fail(err) = outcome else {
        panic!("expected Fail, got {outcome:?}");
    };
    assert_eq!(err.err_name(), "ERR_UNSUPPORTED");
    server.shutdown().await;
}

#[tokio::test]
async fn probe_handles_a_zero_byte_file() {
    let server = RangeServer::with_body(Vec::new()).await;
    let ProbeOutcome::Ready(info) = probe(&local_config(), &server.url()).await.unwrap() else {
        panic!("expected Ready");
    };
    assert_eq!(info.mode, TransferMode::SingleStream { total: Some(0) });
    server.shutdown().await;
}

#[tokio::test]
async fn probe_follows_redirects_and_drops_credentials_across_origins() {
    let target = RangeServer::with_body(payload(2048)).await;
    let origin = RangeServer::start(ServerConfig {
        body: Vec::new(),
        redirect_to: Some(target.url().to_string()),
        ..ServerConfig::default()
    })
    .await;

    let mut cfg = local_config();
    cfg.headers.insert("referer", HeaderValue::from_static("https://origin.test/"));
    cfg.sensitive_headers
        .insert("authorization", HeaderValue::from_static("Bearer secret"));

    let ProbeOutcome::Ready(info) = probe(&cfg, &origin.url()).await.unwrap() else {
        panic!("expected Ready");
    };
    assert_eq!(info.final_url, target.url());
    assert_eq!(info.mode, TransferMode::Segmented { total: 2048 });

    let first = &origin.requests()[0];
    assert_eq!(first.header("authorization"), Some("Bearer secret"));
    let second = &target.requests()[0];
    assert_eq!(
        second.header("authorization"),
        None,
        "credentials must not cross origins"
    );
    assert_eq!(second.header("referer"), Some("https://origin.test/"));
    origin.shutdown().await;
    target.shutdown().await;
}

#[tokio::test]
async fn if_range_returns_206_for_the_current_validator_and_200_for_a_stale_one() {
    let server = RangeServer::with_body(payload(4096)).await;
    let session = Session::open(&local_config(), &server.url()).await.unwrap();

    let response = session.request(Some((0, 99)), Some("\"v1\"")).await.unwrap();
    assert_eq!(response.status(), 206);
    assert_eq!(
        response
            .headers()
            .get("content-range")
            .and_then(|v| v.to_str().ok()),
        Some("bytes 0-99/4096")
    );

    let stale = session.request(Some((0, 99)), Some("\"v0\"")).await.unwrap();
    assert_eq!(stale.status(), 200, "a stale validator restarts the download");
    server.shutdown().await;
}

#[tokio::test]
async fn a_range_beyond_the_end_is_416() {
    let server = RangeServer::with_body(payload(100)).await;
    let session = Session::open(&local_config(), &server.url()).await.unwrap();
    let response = session.request(Some((500, 600)), None).await.unwrap();
    assert_eq!(response.status(), 416);
    server.shutdown().await;
}

#[tokio::test]
async fn probe_refuses_private_hosts_when_local_downloads_are_off() {
    let server = RangeServer::with_body(payload(10)).await;
    let outcome = probe(&HttpConfig::default(), &server.url()).await;
    let err = outcome.unwrap_err();
    assert_eq!(err.err_name(), "ERR_BLOCKED_PRIVATE_IP");
    server.shutdown().await;
}

#[tokio::test]
async fn probe_refuses_non_http_schemes() {
    let err = probe(
        &local_config(),
        &Url::parse("ftp://example.com/file.iso").unwrap(),
    )
    .await
    .unwrap_err();
    assert_eq!(err.err_name(), "ERR_INVALID_URL");
}
