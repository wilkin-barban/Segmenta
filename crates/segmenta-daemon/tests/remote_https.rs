//! Remote control over TLS: the certificate is pinned, and encryption is not authentication.

use std::sync::Arc;

use segmenta_daemon::events::EventBus;
use segmenta_daemon::manager::Manager;
use segmenta_daemon::remote;
use segmenta_daemon::remote_tls::RemoteTls;
use segmenta_daemon::secrets::FileSecretStore;
use segmenta_daemon::store::Store;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_rustls::rustls::pki_types::CertificateDer;
use tokio_rustls::rustls::{ClientConfig, RootCertStore};
use tokio_rustls::TlsConnector;

struct Harness {
    manager: Arc<Manager>,
    tls: RemoteTls,
    _dir: tempfile::TempDir,
}

fn harness() -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(&dir.path().join("segmenta.db")).unwrap());
    let secrets: Arc<dyn segmenta_daemon::secrets::SecretStore> =
        Arc::new(FileSecretStore::open(&dir.path().join("secrets.json")).unwrap());
    let bus = Arc::new(EventBus::new());
    let (shutdown, _rx) = tokio::sync::watch::channel(false);
    let manager = Arc::new(Manager::new(Arc::clone(&store), secrets, bus, shutdown));
    let tls = RemoteTls::ensure(dir.path()).unwrap();
    Harness {
        manager,
        tls,
        _dir: dir,
    }
}

/// A client that trusts only `cert`.
fn pinned(cert_pem: &str) -> Arc<ClientConfig> {
    use std::sync::Once;
    static PROVIDER: Once = Once::new();
    PROVIDER.call_once(|| {
        let _ = tokio_rustls::rustls::crypto::ring::default_provider().install_default();
    });
    let mut roots = RootCertStore::empty();
    for der in pem_certs(cert_pem) {
        roots.add(CertificateDer::from(der)).unwrap();
    }
    Arc::new(
        ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    )
}

fn pem_certs(pem: &str) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut body = String::new();
    let mut inside = false;
    for line in pem.lines() {
        let line = line.trim();
        if line.contains("BEGIN CERTIFICATE") {
            inside = true;
            continue;
        }
        if line.contains("END CERTIFICATE") {
            if let Ok(der) = base64(&body) {
                out.push(der);
            }
            inside = false;
            body.clear();
            continue;
        }
        if inside {
            body.push_str(line);
        }
    }
    out
}

fn base64(input: &str) -> Result<Vec<u8>, ()> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::new();
    let mut buffer = 0u32;
    let mut bits = 0u8;
    for byte in input.bytes() {
        if byte == b'=' {
            continue;
        }
        let value = TABLE.iter().position(|c| *c == byte).ok_or(())? as u32;
        buffer = (buffer << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    Ok(out)
}

#[tokio::test]
async fn a_pinned_client_reaches_the_api_and_still_needs_its_token() {
    let h = harness();
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let manager = Arc::clone(&h.manager);
    let tls = h.tls.clone();
    let server = tokio::spawn(async move {
        remote::serve_tls(listener, tls, manager, shutdown_rx).await;
    });

    let connector = TlsConnector::from(pinned(h.tls.cert_pem()));
    let stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    let name = tokio_rustls::rustls::pki_types::ServerName::try_from("127.0.0.1").unwrap();
    let mut tls = connector
        .connect(name, stream)
        .await
        .expect("pinned handshake");
    tls.write_all(b"GET /api/downloads HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .await
        .unwrap();
    // The server closes without a TLS close_notify (it writes the response and drops the
    // socket); the bytes are what matter, so an UnexpectedEof after data is not a failure.
    let mut response = Vec::new();
    if let Err(err) = tls.read_to_end(&mut response).await {
        assert!(!response.is_empty(), "no bytes before the close: {err}");
    }
    let text = String::from_utf8_lossy(&response);
    // TLS authenticates the server, not the client: the token is still mandatory.
    assert!(
        text.contains("401") || text.contains("403"),
        "an unauthenticated request must be refused over TLS too: {text}"
    );
    let _ = shutdown_tx.send(true);
    server.abort();
}

#[tokio::test]
async fn a_client_that_pins_another_certificate_is_refused() {
    let h = harness();
    let other_dir = tempfile::tempdir().unwrap();
    let other = RemoteTls::ensure(other_dir.path()).unwrap();
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let (_shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let manager = Arc::clone(&h.manager);
    let tls = h.tls.clone();
    let server = tokio::spawn(async move {
        remote::serve_tls(listener, tls, manager, shutdown_rx).await;
    });

    let connector = TlsConnector::from(pinned(other.cert_pem()));
    let stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    let name = tokio_rustls::rustls::pki_types::ServerName::try_from("127.0.0.1").unwrap();
    let refused = connector.connect(name, stream).await;
    assert!(
        refused.is_err(),
        "a client pinning a different certificate must not connect"
    );
    server.abort();
}

#[tokio::test]
async fn plain_http_against_the_tls_port_gets_nothing() {
    let h = harness();
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let (_shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let manager = Arc::clone(&h.manager);
    let tls = h.tls.clone();
    let server = tokio::spawn(async move {
        remote::serve_tls(listener, tls, manager, shutdown_rx).await;
    });

    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    stream
        .write_all(b"GET /api/downloads HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .await
        .unwrap();
    let mut response = Vec::new();
    let _ = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        stream.read_to_end(&mut response),
    )
    .await;
    let text = String::from_utf8_lossy(&response);
    assert!(
        !text.contains("HTTP/1.1 200"),
        "a plain client must never get an API answer: {text}"
    );
    server.abort();
}
