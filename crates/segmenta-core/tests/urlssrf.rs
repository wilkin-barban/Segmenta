//! SSRF and scheme validation. Spec S11 and OpenSpec 06 §1.2.

use std::net::IpAddr;

use segmenta_core::urlssrf::{is_blocked_ip, resolve_checked, validate_scheme};
use url::Url;

fn ip(s: &str) -> IpAddr {
    s.parse().unwrap()
}

#[test]
fn loopback_private_and_link_local_addresses_are_blocked() {
    for addr in [
        "127.0.0.1",
        "127.9.9.9",
        "::1",
        "10.0.0.1",
        "10.255.255.255",
        "172.16.0.1",
        "172.31.255.255",
        "192.168.0.1",
        "192.168.255.254",
        "169.254.10.10",
        "fe80::1",
        "fc00::1",
        "fd12:3456::1",
    ] {
        assert!(is_blocked_ip(ip(addr)), "{addr} must be blocked");
    }
}

#[test]
fn public_addresses_are_allowed() {
    for addr in ["8.8.8.8", "1.1.1.1", "172.32.0.1", "2001:4860:4860::8888"] {
        assert!(!is_blocked_ip(ip(addr)), "{addr} must be allowed");
    }
}

#[test]
fn only_http_and_https_schemes_are_accepted() {
    assert!(validate_scheme(&Url::parse("https://example.com/a").unwrap()).is_ok());
    assert!(validate_scheme(&Url::parse("http://example.com/a").unwrap()).is_ok());
    for bad in [
        "ftp://example.com/a",
        "file:///etc/passwd",
        "data:text/plain,x",
    ] {
        let err = validate_scheme(&Url::parse(bad).unwrap()).unwrap_err();
        assert_eq!(err.err_name(), "ERR_INVALID_URL", "{bad}");
    }
}

#[tokio::test]
async fn localhost_is_blocked_unless_explicitly_allowed() {
    let url = Url::parse("http://localhost:8080/file").unwrap();
    let err = resolve_checked(&url, false).await.unwrap_err();
    assert_eq!(err.err_name(), "ERR_BLOCKED_PRIVATE_IP");

    let allowed = resolve_checked(&url, true).await.unwrap();
    assert!(!allowed.is_empty());
    assert!(allowed.iter().all(|addr| addr.ip().is_loopback()));
}

#[tokio::test]
async fn public_hosts_resolve_to_public_addresses() {
    // Numeric public address: no DNS lookup involved, deterministic offline.
    let url = Url::parse("http://8.8.8.8:80/x").unwrap();
    let addrs = resolve_checked(&url, false).await.unwrap();
    assert_eq!(addrs[0].ip().to_string(), "8.8.8.8");
    assert_eq!(addrs[0].port(), 80);
}

#[tokio::test]
async fn private_literal_address_is_blocked_without_dns() {
    let url = Url::parse("http://192.168.1.10/file").unwrap();
    let err = resolve_checked(&url, false).await.unwrap_err();
    assert_eq!(err.code(), 1007);
}
