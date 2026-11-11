//! Local HTTP range server used by every network-facing test.
//!
//! Feature-gated behind `testserver`. No external network, no fixture files: the body
//! is generated in memory and the server can be told to lie in the specific ways the
//! contract has to survive (ignore ranges, chunked transfer, 429 with `Retry-After`,
//! changed ETag, truncated body, slow ranges).

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use url::Url;

/// How the server answers requests.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Full entity body.
    pub body: Vec<u8>,
    /// Emit `206` for `Range` requests. When false, go through to `200`.
    pub supports_ranges: bool,
    /// Current `ETag`, echoed and compared against `If-Range`.
    pub etag: Option<String>,
    /// Current `Last-Modified`.
    pub last_modified: Option<String>,
    /// Answer this status for every request (429, 503, 404...).
    pub force_status: Option<u16>,
    /// `Retry-After` sent with a forced status.
    pub retry_after: Option<String>,
    /// Send the body with `Transfer-Encoding: chunked` and no length.
    pub chunked: bool,
    /// Close the socket after this many body bytes.
    pub cut_after: Option<usize>,
    /// Delay between body chunks, to keep workers alive long enough to split.
    pub slow: Option<Duration>,
    /// Bytes per write.
    pub chunk_size: usize,
    /// Answer `200` to any `If-Range`, even when it matches.
    pub ignore_if_range: bool,
    /// Answer `302` with this `Location` instead of serving the body.
    pub redirect_to: Option<String>,
    /// Send `Content-Encoding` even though the body is stored as-is.
    pub content_encoding: Option<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig {
            body: Vec::new(),
            supports_ranges: true,
            etag: Some("\"v1\"".to_string()),
            last_modified: None,
            force_status: None,
            retry_after: None,
            chunked: false,
            cut_after: None,
            slow: None,
            chunk_size: 64 * 1024,
            ignore_if_range: false,
            redirect_to: None,
            content_encoding: None,
        }
    }
}

/// One request the server saw, for header assertions.
#[derive(Debug, Clone)]
pub struct RequestRecord {
    /// Request line target.
    pub path: String,
    /// Lowercased header names.
    pub headers: HashMap<String, String>,
}

impl RequestRecord {
    /// Header lookup by lowercase name.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(String::as_str)
    }
}

struct Shared {
    config: Mutex<ServerConfig>,
    requests: Mutex<Vec<RequestRecord>>,
    connections: AtomicUsize,
}

/// A running range server.
pub struct RangeServer {
    addr: SocketAddr,
    shared: Arc<Shared>,
    shutdown: tokio::sync::watch::Sender<bool>,
}

impl RangeServer {
    /// Start a server with an explicit configuration.
    pub async fn start(config: ServerConfig) -> RangeServer {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local addr");
        let shared = Arc::new(Shared {
            config: Mutex::new(config),
            requests: Mutex::new(Vec::new()),
            connections: AtomicUsize::new(0),
        });
        let (shutdown, mut shutdown_rx) = tokio::sync::watch::channel(false);
        let accept_shared = Arc::clone(&shared);
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = shutdown_rx.changed() => break,
                    accepted = listener.accept() => {
                        let Ok((stream, _peer)) = accepted else { continue };
                        let conn = Arc::clone(&accept_shared);
                        tokio::spawn(async move {
                            conn.connections.fetch_add(1, Ordering::Relaxed);
                            let _ = serve_connection(stream, conn).await;
                        });
                    }
                }
            }
        });
        RangeServer {
            addr,
            shared,
            shutdown,
        }
    }

    /// Start a server that serves `body` with working ranges.
    pub async fn with_body(body: Vec<u8>) -> RangeServer {
        RangeServer::start(ServerConfig {
            body,
            ..ServerConfig::default()
        })
        .await
    }

    /// Base URL of the served file.
    pub fn url(&self) -> Url {
        Url::parse(&format!("http://{}/file.bin", self.addr)).expect("valid url")
    }

    /// URL with a custom path.
    pub fn url_for(&self, path: &str) -> Url {
        Url::parse(&format!("http://{}{}", self.addr, path)).expect("valid url")
    }

    /// Every request received so far, in order.
    pub fn requests(&self) -> Vec<RequestRecord> {
        self.shared.requests.lock().unwrap().clone()
    }

    /// Number of requests received.
    pub fn request_count(&self) -> usize {
        self.shared.requests.lock().unwrap().len()
    }

    /// Number of accepted connections.
    pub fn connection_count(&self) -> usize {
        self.shared.connections.load(Ordering::Relaxed)
    }

    /// Replace the configuration between requests.
    pub fn update(&self, f: impl FnOnce(&mut ServerConfig)) {
        let mut config = self.shared.config.lock().unwrap();
        f(&mut config);
    }

    /// Replace the body and its validator, simulating a changed remote file.
    pub fn replace_body(&self, body: Vec<u8>, etag: &str) {
        let mut config = self.shared.config.lock().unwrap();
        config.body = body;
        config.etag = Some(etag.to_string());
    }

    /// Stop accepting connections.
    pub async fn shutdown(self) {
        let _ = self.shutdown.send(true);
    }
}

async fn serve_connection(mut stream: TcpStream, shared: Arc<Shared>) -> std::io::Result<()> {
    let mut buf = Vec::with_capacity(4096);
    let mut chunk = [0u8; 1024];
    loop {
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&chunk[..read]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
        if buf.len() > 64 * 1024 {
            return Ok(());
        }
    }
    let text = String::from_utf8_lossy(&buf).to_string();
    let mut lines = text.split("\r\n");
    let request_line = lines.next().unwrap_or_default().to_string();
    let path = request_line.split_whitespace().nth(1).unwrap_or("/").to_string();
    let mut headers = HashMap::new();
    for line in lines {
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }
    shared.requests.lock().unwrap().push(RequestRecord {
        path,
        headers: headers.clone(),
    });

    let (config, etag, last_modified) = {
        let config = shared.config.lock().unwrap();
        (config.clone(), config.etag.clone(), config.last_modified.clone())
    };

    if let Some(status) = config.force_status {
        let reason = match status {
            429 => "Too Many Requests",
            503 => "Service Unavailable",
            404 => "Not Found",
            _ => "Error",
        };
        let mut head = format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Length: 0\r\nConnection: close\r\n"
        );
        if let Some(retry_after) = &config.retry_after {
            head.push_str(&format!("Retry-After: {retry_after}\r\n"));
        }
        head.push_str("\r\n");
        stream.write_all(head.as_bytes()).await?;
        return stream.shutdown().await;
    }

    if let Some(location) = &config.redirect_to {
        let head = format!(
            "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        stream.write_all(head.as_bytes()).await?;
        return stream.shutdown().await;
    }

    let total = config.body.len() as u64;
    if total == 0 {
        let mut head = String::from(
            "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nAccept-Ranges: bytes\r\n",
        );
        if let Some(etag) = &etag {
            head.push_str(&format!("ETag: {etag}\r\n"));
        }
        head.push_str("Connection: close\r\n\r\n");
        stream.write_all(head.as_bytes()).await?;
        return stream.shutdown().await;
    }
    let requested = headers
        .get("range")
        .and_then(|value| parse_range_header(value));
    let if_range = headers.get("if-range").cloned();

    // A stale validator turns a range request into a full 200, as RFC 9110 allows.
    let validator_stale = if config.ignore_if_range {
        false
    } else {
        match (&if_range, &etag) {
            (Some(sent), Some(current)) => sent != current,
            (Some(_), None) => true,
            _ => false,
        }
    };

    let use_range = config.supports_ranges && !config.chunked && requested.is_some() && !validator_stale;
    let (status, body_start, body_end) = if use_range {
        let (start, end) = requested.unwrap();
        let end = end.unwrap_or(total.saturating_sub(1)).min(total.saturating_sub(1));
        if total == 0 || start >= total || start > end {
            let head = format!(
                "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{total}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
            stream.write_all(head.as_bytes()).await?;
            return stream.shutdown().await;
        }
        (206u16, start, end)
    } else {
        (200u16, 0, total.saturating_sub(1))
    };

    let body_len = if total == 0 { 0 } else { body_end - body_start + 1 };
    let mut head = format!("HTTP/1.1 {status} {}\r\n", if status == 206 { "Partial Content" } else { "OK" });
    head.push_str("Accept-Ranges: bytes\r\n");
    if status == 206 {
        head.push_str(&format!(
            "Content-Range: bytes {body_start}-{body_end}/{total}\r\n"
        ));
    }
    if let Some(etag) = &etag {
        head.push_str(&format!("ETag: {etag}\r\n"));
    }
    if let Some(last_modified) = &last_modified {
        head.push_str(&format!("Last-Modified: {last_modified}\r\n"));
    }
    if let Some(encoding) = &config.content_encoding {
        head.push_str(&format!("Content-Encoding: {encoding}\r\n"));
    }
    head.push_str("Content-Type: application/octet-stream\r\n");
    head.push_str("Content-Disposition: attachment; filename=\"file.bin\"\r\n");
    if config.chunked {
        head.push_str("Transfer-Encoding: chunked\r\n");
    } else {
        head.push_str(&format!("Content-Length: {body_len}\r\n"));
    }
    head.push_str("Connection: close\r\n\r\n");
    stream.write_all(head.as_bytes()).await?;

    if total == 0 {
        if config.chunked {
            stream.write_all(b"0\r\n\r\n").await?;
        }
        return stream.shutdown().await;
    }

    let slice = &config.body[body_start as usize..=body_end as usize];
    let mut sent = 0usize;
    let limit = config.cut_after.unwrap_or(usize::MAX);
    while sent < slice.len() && sent < limit {
        let take = config.chunk_size.min(slice.len() - sent).min(limit - sent);
        if config.chunked {
            let header = format!("{take:x}\r\n");
            stream.write_all(header.as_bytes()).await?;
            stream.write_all(&slice[sent..sent + take]).await?;
            stream.write_all(b"\r\n").await?;
        } else {
            stream.write_all(&slice[sent..sent + take]).await?;
        }
        sent += take;
        if let Some(delay) = config.slow {
            tokio::time::sleep(delay).await;
        }
    }
    if config.chunked && sent == slice.len() {
        stream.write_all(b"0\r\n\r\n").await?;
    }
    // A cut body closes without a final chunk and before Content-Length is satisfied.
    stream.shutdown().await
}

/// Parse `bytes=start-end`, `bytes=start-` and `bytes=-suffix`.
fn parse_range_header(value: &str) -> Option<(u64, Option<u64>)> {
    let value = value.trim().strip_prefix("bytes=")?;
    let (start, end) = value.split_once('-')?;
    if start.is_empty() {
        let suffix: u64 = end.parse().ok()?;
        return Some((suffix.saturating_sub(1), None));
    }
    let start: u64 = start.parse().ok()?;
    let end = if end.is_empty() {
        None
    } else {
        Some(end.parse().ok()?)
    };
    Some((start, end))
}
