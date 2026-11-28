//! Remote control: token pairing, authenticated reads, actions and revocation.

mod common;

use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Daemon {
    child: Child,
    dir: tempfile::TempDir,
    port: u16,
}

impl Daemon {
    /// Start one daemon and enable remote control on a free port.
    fn start() -> Daemon {
        let dir = tempfile::tempdir().unwrap();
        let child = Command::new(env!("CARGO_BIN_EXE_segmenta-daemon"))
            .env("SEGMENTA_SOCKET", dir.path().join("daemon.sock"))
            .env("XDG_RUNTIME_DIR", dir.path())
            .env("SEGMENTA_SECRET_TOOL", common::fake_secret_tool(dir.path()))
            .env("XDG_DATA_HOME", dir.path().join("data"))
            .env("XDG_CONFIG_HOME", dir.path().join("config"))
            .env("XDG_STATE_HOME", dir.path().join("state"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let daemon = Daemon {
            child,
            dir,
            port: 0,
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        while !daemon.socket().exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(daemon.socket().exists(), "the daemon never started");
        daemon.call(
            "settings.update",
            json!({ "download_dir": daemon.dir.path().join("dl").display().to_string() }),
        );
        daemon
    }

    fn socket(&self) -> PathBuf {
        self.dir.path().join("daemon.sock")
    }

    fn request(&self, method: &str, params: Value) -> Value {
        use std::io::{BufRead, BufReader, Write};
        let mut stream = std::os::unix::net::UnixStream::connect(self.socket()).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(30)))
            .unwrap();
        writeln!(
            stream,
            "{}",
            json!({"jsonrpc":"2.0","id":1,"method":method,"params":params})
        )
        .unwrap();
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    }

    fn call(&self, method: &str, params: Value) -> Value {
        let reply = self.request(method, params);
        assert!(reply.get("error").is_none(), "{method}: {reply}");
        reply["result"].clone()
    }

    /// Enable the API and return the token from the pairing payload.
    ///
    /// Two tests running at once can be handed the same free port by the kernel, and then
    /// one of them talks to the other's daemon. The token check below catches exactly that
    /// case and retries on a different port instead of failing for the wrong reason.
    fn enable(&mut self) -> String {
        for _ in 0..5 {
            let port = free_port();
            let reply = self.call(
                "remote.enable",
                json!({ "port": port, "bind": "127.0.0.1" }),
            );
            self.port = port;
            let pairing = reply["pairing"].as_str().unwrap().to_string();
            let token = pairing
                .split("token=")
                .nth(1)
                .expect("the pairing payload carries the token")
                .to_string();
            let deadline = Instant::now() + Duration::from_secs(20);
            while Instant::now() < deadline {
                if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
                    // The daemon serves HTTPS only: the test pins the certificate it
                    // generated in its own data directory, exactly like a phone would.
                    let (status, _) = self.http("GET", "/api/downloads", Some(&token), "");
                    if status == 200 {
                        return token;
                    }
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        panic!("the remote API never answered with the token it just minted");
    }

    fn http(&self, method: &str, path: &str, token: Option<&str>, body: &str) -> (u16, String) {
        // The daemon only serves HTTPS. The client pins the certificate the daemon wrote in
        // its own data directory: anything else would be trusting whatever answers.
        let tls = segmenta_daemon::remote_tls::RemoteTls::ensure(
            &self.dir.path().join("data").join("segmenta"),
        )
        .expect("the daemon generated a certificate");
        let mut roots = tokio_rustls::rustls::RootCertStore::empty();
        for der in pem_certificates(tls.cert_pem()) {
            roots
                .add(tokio_rustls::rustls::pki_types::CertificateDer::from(der))
                .unwrap();
        }
        let _ = tokio_rustls::rustls::crypto::ring::default_provider().install_default();
        let config = std::sync::Arc::new(
            tokio_rustls::rustls::ClientConfig::builder()
                .with_root_certificates(roots)
                .with_no_client_auth(),
        );
        let connector = tokio_rustls::TlsConnector::from(config);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let name = tokio_rustls::rustls::pki_types::ServerName::try_from("127.0.0.1").unwrap();
        let mut stream = runtime.block_on(async {
            let tcp = tokio::net::TcpStream::connect(("127.0.0.1", self.port))
                .await
                .unwrap();
            connector
                .connect(name, tcp)
                .await
                .expect("pinned handshake")
        });
        let auth = token
            .map(|token| format!("Authorization: Bearer {token}\r\n"))
            .unwrap_or_default();
        let request = format!(
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n{auth}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let mut response = String::new();
        // One runtime drives the whole exchange: the stream is async even though the rest
        // of this harness is not.
        let _ = runtime.block_on(async {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            stream.write_all(request.as_bytes()).await?;
            let mut buffer = Vec::new();
            let _ = stream.read_to_end(&mut buffer).await;
            response = String::from_utf8_lossy(&buffer).to_string();
            Ok::<(), std::io::Error>(())
        });
        let status = response
            .split_whitespace()
            .nth(1)
            .and_then(|code| code.parse().ok())
            .unwrap_or(0);
        let body = response
            .split("\r\n\r\n")
            .nth(1)
            .unwrap_or_default()
            .to_string();
        (status, body)
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Tests in one binary run in parallel, and two of them can be handed the same free port
/// by the kernel before either daemon binds it. Serializing them removes that whole class
/// of false failures instead of chasing the symptom.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|err| err.into_inner())
}

fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    port
}

#[test]
fn the_api_answers_only_with_the_active_token() {
    let _guard = serial();
    let mut daemon = Daemon::start();
    let token = daemon.enable();

    // No token at all.
    let (status, body) = daemon.http("GET", "/api/downloads", None, "");
    assert_eq!(status, 401, "{body}");
    assert!(body.contains("ERR_REMOTE_UNAUTHORIZED"));

    // A wrong token is refused the same way; the answer never says which part failed.
    let (status, _) = daemon.http("GET", "/api/downloads", Some("deadbeef"), "");
    assert_eq!(status, 401);

    // The real token works and returns the list.
    let (status, body) = daemon.http("GET", "/api/downloads", Some(&token), "");
    assert_eq!(status, 200, "{body}");
    let rows: Value = serde_json::from_str(&body).unwrap();
    assert!(rows.as_array().is_some());

    // Unknown routes are still behind the token and then 404.
    let (status, _) = daemon.http("GET", "/api/nope", Some(&token), "");
    assert_eq!(status, 404);
}

#[test]
fn revoking_the_token_closes_the_door_immediately() {
    let _guard = serial();
    let mut daemon = Daemon::start();
    let token = daemon.enable();
    let (status, _) = daemon.http("GET", "/api/downloads", Some(&token), "");
    assert_eq!(status, 200);

    daemon.call("remote.revoke", json!({}));
    let (status, body) = daemon.http("GET", "/api/downloads", Some(&token), "");
    assert_eq!(status, 401, "the revoked token must not work: {body}");

    let status = daemon.call("remote.status", json!({}));
    assert_eq!(status["enabled"], false);
    assert_eq!(status["paired"], false);
}

#[test]
fn an_action_reaches_the_queue_and_unknown_actions_are_rejected() {
    let _guard = serial();
    let mut daemon = Daemon::start();
    let token = daemon.enable();

    // No task exists yet, so the action fails cleanly instead of pretending.
    let (status, body) = daemon.http(
        "POST",
        "/api/action",
        Some(&token),
        &json!({"task_id": "missing", "action": "pause"}).to_string(),
    );
    assert_eq!(status, 400, "{body}");

    let (status, body) = daemon.http(
        "POST",
        "/api/action",
        Some(&token),
        &json!({"task_id": "x", "action": "explode"}).to_string(),
    );
    assert_eq!(status, 400, "{body}");
    assert!(body.contains("ERR_UNKNOWN_ACTION"));
}

#[test]
fn enabling_returns_the_token_once_and_status_never_repeats_it() {
    let _guard = serial();
    let mut daemon = Daemon::start();
    let token = daemon.enable();
    assert_eq!(token.len(), 64);

    let status = daemon.call("remote.status", json!({}));
    assert_eq!(status["enabled"], true);
    assert_eq!(status["paired"], true);
    assert!(
        !status.to_string().contains(&token),
        "status must never leak the token"
    );

    // Enabling again mints a new token: the previous one stops working.
    let second = daemon.enable();
    assert_ne!(second, token);
    let (status, _) = daemon.http("GET", "/api/downloads", Some(&token), "");
    assert_eq!(status, 401);
}

/// DER bodies of the certificate blocks in a PEM bundle.
fn pem_certificates(pem: &str) -> Vec<Vec<u8>> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
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
            let mut decoded = Vec::new();
            let mut buffer = 0u32;
            let mut bits = 0u8;
            for byte in body.bytes() {
                if byte == b'=' {
                    continue;
                }
                let Some(value) = TABLE.iter().position(|c| *c == byte) else {
                    continue;
                };
                buffer = (buffer << 6) | value as u32;
                bits += 6;
                if bits >= 8 {
                    bits -= 8;
                    decoded.push((buffer >> bits) as u8);
                }
            }
            if !decoded.is_empty() {
                out.push(decoded);
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
