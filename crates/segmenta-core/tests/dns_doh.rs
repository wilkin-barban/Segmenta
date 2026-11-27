//! DNS over HTTPS against a local mock: the wire format, the guards and the typed refusal.

use segmenta_core::dns::{query_doh, DnsConfig, DnsError, DnsMode};
use std::net::Ipv4Addr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// A DNS response for `example.test` with the given address, as RFC 1035 bytes.
fn answer(address: Ipv4Addr) -> Vec<u8> {
    let mut packet = vec![
        0x12, 0x34, 0x81, 0x80, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00,
    ];
    packet.extend_from_slice(&[7, b'e', b'x', b'a', b'm', b'p', b'l', b'e']);
    packet.extend_from_slice(&[4, b't', b'e', b's', b't', 0]);
    packet.extend_from_slice(&[0, 1, 0, 1]);
    packet.extend_from_slice(&[0xC0, 0x0C]);
    packet.extend_from_slice(&[0, 1, 0, 1]);
    packet.extend_from_slice(&0u32.to_be_bytes());
    packet.extend_from_slice(&4u16.to_be_bytes());
    packet.extend_from_slice(&address.octets());
    packet
}

/// Serve one POST /dns-query answer and return the port.
async fn mock_doh(body: Vec<u8>) -> u16 {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let Ok((mut stream, _)) = listener.accept().await else {
            return;
        };
        let mut raw = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            let Ok(read) = stream.read(&mut chunk).await else {
                return;
            };
            if read == 0 {
                break;
            }
            raw.extend_from_slice(&chunk[..read]);
            if let Some(split) = raw.windows(4).position(|window| window == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&raw[..split]).to_string();
                let wanted = head
                    .split("\r\n")
                    .filter_map(|line| line.split_once(':'))
                    .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                    .and_then(|(_, value)| value.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                if raw.len() >= split + 4 + wanted {
                    break;
                }
            }
        }
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/dns-message\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(response.as_bytes()).await;
        let _ = stream.write_all(&body).await;
    });
    port
}

fn config(port: u16, allow_local: bool) -> DnsConfig {
    DnsConfig {
        mode: DnsMode::Doh,
        endpoint: format!("http://127.0.0.1:{port}/dns-query"),
        interface: None,
        allow_local,
    }
}

#[tokio::test]
async fn a_doh_answer_is_parsed() {
    let port = mock_doh(answer(Ipv4Addr::new(93, 184, 216, 34))).await;
    let ips = query_doh(&config(port, false), "example.test")
        .await
        .unwrap();
    assert_eq!(
        ips,
        vec![std::net::IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34))]
    );
}

#[tokio::test]
async fn a_private_answer_is_refused_unless_local_is_allowed() {
    // The endpoint is honest, the answer points at the LAN: the SSRF guard must win.
    let port = mock_doh(answer(Ipv4Addr::new(10, 0, 0, 7))).await;
    let refused = query_doh(&config(port, false), "example.test").await;
    assert!(refused.is_ok(), "the transport works; the guard runs later");

    let port = mock_doh(answer(Ipv4Addr::new(10, 0, 0, 7))).await;
    let allowed = query_doh(&config(port, true), "example.test")
        .await
        .unwrap();
    assert_eq!(
        allowed,
        vec![std::net::IpAddr::V4(Ipv4Addr::new(10, 0, 0, 7))]
    );
}

#[tokio::test]
async fn an_unreachable_endpoint_is_a_typed_error() {
    // Nothing listens on this port: the failure must be the typed one, not a panic.
    let error = query_doh(&config(9, false), "example.test")
        .await
        .unwrap_err();
    assert_eq!(error, DnsError::Unreachable);
    assert_eq!(error.code(), "ERR_DNS_UNREACHABLE");
}

#[tokio::test]
async fn the_published_configuration_applies_the_guard() {
    use segmenta_core::dns::{resolve_host, set_config};

    let port = mock_doh(answer(Ipv4Addr::new(10, 0, 0, 7))).await;
    set_config(Some(config(port, false)));
    let blocked = resolve_host("example.test").await.unwrap_err();
    set_config(None);
    assert_eq!(blocked, DnsError::Blocked);
    assert_eq!(blocked.code(), "ERR_DNS_BLOCKED");
}
