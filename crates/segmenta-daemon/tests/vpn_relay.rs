//! VPN relay: authentication, SSRF guards and a working CONNECT tunnel.
//!
//! Hermetic by construction: the relay only listens on `127.0.0.1` with a port the kernel
//! picks, and the upstream in these tests is a server the test itself started.

use segmenta_daemon::vpn_relay::{basic_credentials, spawn, Kind, Request};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// A loopback server that answers one request and closes.
fn echo_server() -> (u16, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buffer = [0u8; 1024];
            let read = stream.read(&mut buffer).unwrap_or(0);
            let request = String::from_utf8_lossy(&buffer[..read]).to_string();
            let body = format!("tunnel-ok:{}", request.lines().count());
            let _ = stream.write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                )
                .as_bytes(),
            );
        }
    });
    (port, handle)
}

async fn read_response(stream: &mut tokio::net::TcpStream) -> String {
    let mut data = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        match tokio::time::timeout(Duration::from_secs(5), stream.read(&mut chunk)).await {
            Ok(Ok(0)) | Err(_) => break,
            Ok(Ok(read)) => data.extend_from_slice(&chunk[..read]),
            Ok(Err(_)) => break,
        }
        if data.len() > 8192 {
            break;
        }
    }
    String::from_utf8_lossy(&data).into_owned()
}

#[test]
fn parses_connect_and_absolute_forms() {
    let connect = Request::parse(
        b"CONNECT example.test:443 HTTP/1.1\r\nProxy-Authorization: Basic dG9rZW46eA==\r\n\r\n",
    )
    .unwrap();
    assert_eq!(connect.kind, Kind::Connect);
    assert_eq!(connect.authority, "example.test:443");
    assert_eq!(
        connect.credentials,
        Some(("token".to_string(), "x".to_string()))
    );
    assert!(connect.authorized("token"));
    assert!(!connect.authorized("other"));

    let forward = Request::parse(
        b"GET http://example.test:8080/file.bin HTTP/1.1\r\nHost: example.test\r\n\r\n",
    )
    .unwrap();
    assert_eq!(forward.kind, Kind::Forward);
    assert_eq!(forward.authority, "example.test:8080");
    assert_eq!(forward.path, "/file.bin");
    assert_eq!(forward.credentials, None);

    // The head sent upstream drops the proxy credential and keeps the rest.
    let head = forward.forwarded_head();
    assert!(head.starts_with("GET /file.bin HTTP/1.1\r\n"));
    assert!(!head.to_lowercase().contains("proxy-authorization"));

    assert_eq!(
        basic_credentials("Basic dG9rZW46eA=="),
        Some(("token".to_string(), "x".to_string()))
    );
    assert_eq!(basic_credentials("Bearer token"), None);
}

#[tokio::test]
async fn a_connect_tunnel_needs_the_token_and_reaches_the_target() {
    let (target, server) = echo_server();
    let relay = spawn("lo".to_string(), true).await.expect("relay");

    // No credential: 407 and no dial.
    let mut anonymous = tokio::net::TcpStream::connect(("127.0.0.1", relay.port))
        .await
        .unwrap();
    anonymous
        .write_all(format!("CONNECT 127.0.0.1:{target} HTTP/1.1\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let refused = read_response(&mut anonymous).await;
    assert!(refused.starts_with("HTTP/1.1 407"), "{refused}");

    // With the token the relay opens the tunnel and copies bytes both ways.
    let credential = base64("segmenta", &relay.token);
    let mut client = tokio::net::TcpStream::connect(("127.0.0.1", relay.port))
        .await
        .unwrap();
    client
        .write_all(
            format!(
                "CONNECT 127.0.0.1:{target} HTTP/1.1\r\nProxy-Authorization: Basic {credential}\r\n\r\n"
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let established = read_response(&mut client).await;
    assert!(established.starts_with("HTTP/1.1 200"), "{established}");

    client
        .write_all(b"GET / HTTP/1.1\r\nHost: test\r\n\r\n")
        .await
        .unwrap();
    let reply = read_response(&mut client).await;
    assert!(reply.contains("tunnel-ok"), "{reply}");
    server.join().unwrap();
}

#[tokio::test]
async fn a_down_interface_refuses_instead_of_dialing() {
    let (target, _server) = echo_server();
    let relay = spawn("vpnx0".to_string(), true).await.expect("relay");
    let credential = base64("segmenta", &relay.token);

    let mut client = tokio::net::TcpStream::connect(("127.0.0.1", relay.port))
        .await
        .unwrap();
    client
        .write_all(
            format!(
                "CONNECT 127.0.0.1:{target} HTTP/1.1\r\nProxy-Authorization: Basic {credential}\r\n\r\n"
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let reply = read_response(&mut client).await;
    assert!(
        reply.starts_with("HTTP/1.1 503"),
        "a missing interface must fail closed: {reply}"
    );
}

#[tokio::test]
async fn the_relay_never_pivots_into_loopback_without_permission() {
    let (target, _server) = echo_server();
    // `allow_local = false` is the production default: a proxied tool must not reach the
    // machine itself through the relay.
    let relay = spawn("lo".to_string(), false).await.expect("relay");
    let credential = base64("segmenta", &relay.token);

    let mut client = tokio::net::TcpStream::connect(("127.0.0.1", relay.port))
        .await
        .unwrap();
    client
        .write_all(
            format!(
                "CONNECT 127.0.0.1:{target} HTTP/1.1\r\nProxy-Authorization: Basic {credential}\r\n\r\n"
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let reply = read_response(&mut client).await;
    assert!(
        reply.starts_with("HTTP/1.1 502"),
        "loopback must be refused as an SSRF target: {reply}"
    );
}

fn base64(user: &str, password: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(format!("{user}:{password}"))
}
