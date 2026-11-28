//! Remote control: one revocable session token and a small read/act API.
//!
//! Contract: T4 of `odd/tasks/segmenta-v0.6.0-red-remoto.md`.
//!
//! # Trust model
//!
//! The server is off unless the user turns it on. Turning it on mints a **single** session
//! token, shown once so it can be paired (the QR payload); only its SHA-256 digest is kept,
//! in the secret store, so a database or settings dump never holds a usable credential.
//! Revoking deletes the digest, and every later request is refused — including one that
//! carries the token that was valid a second ago.
//!
//! # Transport
//!
//! Plain HTTP on the address the user chose: this is a LAN control surface, not a
//! replacement for the Unix socket, and it is documented as such. Every route requires the
//! bearer token, and there is no anonymous endpoint, not even a health probe.

use std::time::Duration;

/// Secret alias that holds the digest of the active session token.
pub const TOKEN_ALIAS: &str = "remote-session-token";

/// Longest accepted token, to bound the work a hostile header can cause.
const MAX_TOKEN_LEN: usize = 128;

/// One authenticated request, already parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteRequest {
    /// `GET` or `POST`.
    pub method: String,
    /// Path without the query string.
    pub path: String,
    /// Bearer token presented by the client, if any.
    pub token: Option<String>,
    /// Parsed JSON body for `POST`.
    pub body: Option<serde_json::Value>,
}

/// Why a request was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteRefusal {
    /// No `Authorization: Bearer` header.
    MissingToken,
    /// The token does not match the active digest, or was revoked.
    InvalidToken,
    /// The request line or body is malformed.
    BadRequest,
    /// The route does not exist.
    NotFound,
    /// Body larger than the accepted maximum.
    TooLarge,
}

impl RemoteRefusal {
    /// HTTP status for the refusal.
    pub fn status(self) -> &'static str {
        match self {
            RemoteRefusal::MissingToken | RemoteRefusal::InvalidToken => "401 Unauthorized",
            RemoteRefusal::BadRequest | RemoteRefusal::TooLarge => "400 Bad Request",
            RemoteRefusal::NotFound => "404 Not Found",
        }
    }

    /// Stable code carried in the JSON body.
    pub fn code(self) -> &'static str {
        match self {
            RemoteRefusal::MissingToken => "ERR_REMOTE_UNAUTHORIZED",
            RemoteRefusal::InvalidToken => "ERR_REMOTE_UNAUTHORIZED",
            RemoteRefusal::BadRequest => "ERR_REMOTE_BAD_REQUEST",
            RemoteRefusal::NotFound => "ERR_REMOTE_NOT_FOUND",
            RemoteRefusal::TooLarge => "ERR_REMOTE_TOO_LARGE",
        }
    }
}

/// Generate a fresh session token: 32 random bytes, hex encoded.
pub fn new_token() -> String {
    let mut bytes = [0u8; 32];
    // `rand::rngs::OsRng` is the same source the daemon already uses for ids.
    use rand::RngCore;
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Digest stored for a token; the token itself is never persisted.
pub fn token_digest(token: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

/// Compare the presented token with the stored digest.
///
/// The comparison folds every byte instead of returning early, so the time it takes does
/// not depend on where the first difference is.
pub fn authorize(stored_digest: &str, presented: &str) -> Result<(), RemoteRefusal> {
    if presented.is_empty() || presented.len() > MAX_TOKEN_LEN {
        return Err(RemoteRefusal::InvalidToken);
    }
    let candidate = token_digest(presented);
    let left = candidate.as_bytes();
    let right = stored_digest.as_bytes();
    if left.len() != right.len() {
        return Err(RemoteRefusal::InvalidToken);
    }
    let mut difference = 0u8;
    for (a, b) in left.iter().zip(right.iter()) {
        difference |= a ^ b;
    }
    if difference == 0 {
        Ok(())
    } else {
        Err(RemoteRefusal::InvalidToken)
    }
}

/// Pairing payload the QR encodes: the URL plus the one-time token.
pub fn pairing_payload(host: &str, port: u16, token: &str) -> String {
    format!("http://{host}:{port}/?token={token}")
}

/// Parse the request line, the bearer token and an optional JSON body.
pub fn parse_request(raw: &[u8], body_limit: usize) -> Result<RemoteRequest, RemoteRefusal> {
    let split = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or(RemoteRefusal::BadRequest)?;
    let head = std::str::from_utf8(&raw[..split]).map_err(|_| RemoteRefusal::BadRequest)?;
    let mut lines = head.split("\r\n");
    let request_line = lines.next().ok_or(RemoteRefusal::BadRequest)?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().ok_or(RemoteRefusal::BadRequest)?.to_string();
    let target = parts.next().ok_or(RemoteRefusal::BadRequest)?.to_string();
    let version = parts.next().unwrap_or_default();
    if !version.starts_with("HTTP/1.") || !matches!(method.as_str(), "GET" | "POST") {
        return Err(RemoteRefusal::BadRequest);
    }
    let path = target.split('?').next().unwrap_or(&target).to_string();
    let mut token = None;
    let mut length = 0usize;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        if name.eq_ignore_ascii_case("authorization") {
            token = value
                .strip_prefix("Bearer ")
                .or_else(|| value.strip_prefix("bearer "))
                .map(|token| token.trim().to_string());
        } else if name.eq_ignore_ascii_case("content-length") {
            length = value
                .parse::<usize>()
                .map_err(|_| RemoteRefusal::BadRequest)?;
        }
    }
    if length > body_limit {
        return Err(RemoteRefusal::TooLarge);
    }
    let body_start = split + 4;
    let available = raw.len().saturating_sub(body_start);
    if length > available {
        return Err(RemoteRefusal::BadRequest);
    }
    let body = if length == 0 {
        None
    } else {
        serde_json::from_slice(&raw[body_start..body_start + length])
            .map(Some)
            .map_err(|_| RemoteRefusal::BadRequest)?
    };
    Ok(RemoteRequest {
        method,
        path,
        token,
        body,
    })
}

/// Deadline of one remote action, mirrored from the daemon's own IPC timeouts.
pub const ACTION_TIMEOUT: Duration = Duration::from_secs(30);

/// Serve the remote API until the listener is dropped.
///
/// Every connection is handled on its own task; one malformed request closes its own
/// connection only. The digest is read per request, so a revoke takes effect immediately
/// instead of at the next restart.
pub async fn serve(
    listener: tokio::net::TcpListener,
    manager: std::sync::Arc<crate::manager::Manager>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            accepted = listener.accept() => {
                let Ok((mut stream, _peer)) = accepted else { continue };
                let manager = std::sync::Arc::clone(&manager);
                tokio::spawn(async move {
                    let _ = handle(&mut stream, &manager).await;
                });
            }
        }
    }
}

/// Serve the remote API over TLS.
///
/// The HTTP parsing is the same code as the plain listener: encryption is a transport, not
/// a second protocol. The token check still runs inside `handle`, because a TLS handshake
/// authenticates the server, not the client.
pub async fn serve_tls(
    listener: tokio::net::TcpListener,
    tls: crate::remote_tls::RemoteTls,
    manager: std::sync::Arc<crate::manager::Manager>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    let acceptor = match tls.acceptor() {
        Ok(acceptor) => acceptor,
        Err(err) => {
            tracing::error!(%err, "the remote control cannot start TLS");
            return;
        }
    };
    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            accepted = listener.accept() => {
                let Ok((stream, _peer)) = accepted else { continue };
                let acceptor = acceptor.clone();
                let manager = std::sync::Arc::clone(&manager);
                tokio::spawn(async move {
                    // A failed handshake is not an error worth logging with detail: it is a
                    // client that did not trust this certificate.
                    let Ok(mut stream) = acceptor.accept(stream).await else {
                        return;
                    };
                    let _ = handle(&mut stream, &manager).await;
                });
            }
        }
    }
}

async fn handle<S>(
    stream: &mut S,
    manager: &std::sync::Arc<crate::manager::Manager>,
) -> std::io::Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    use tokio::io::AsyncReadExt;
    let mut raw = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        raw.extend_from_slice(&chunk[..read]);
        if raw.len() > 64 * 1024 {
            return reply(
                stream,
                "400 Bad Request",
                b"{\"error\":\"ERR_REMOTE_TOO_LARGE\"}",
            )
            .await;
        }
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
    let request = match parse_request(&raw, 64 * 1024) {
        Ok(request) => request,
        Err(refusal) => {
            return reply(
                stream,
                refusal.status(),
                format!("{{\"error\":\"{}\"}}", refusal.code()).as_bytes(),
            )
            .await
        }
    };
    let Some(digest) = manager.remote_digest() else {
        return reply(
            stream,
            "401 Unauthorized",
            b"{\"error\":\"ERR_REMOTE_UNAUTHORIZED\"}",
        )
        .await;
    };
    let Some(presented) = request.token.as_deref() else {
        return reply(
            stream,
            "401 Unauthorized",
            b"{\"error\":\"ERR_REMOTE_UNAUTHORIZED\"}",
        )
        .await;
    };
    if authorize(&digest, presented).is_err() {
        return reply(
            stream,
            "401 Unauthorized",
            b"{\"error\":\"ERR_REMOTE_UNAUTHORIZED\"}",
        )
        .await;
    }
    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/api/downloads") => match manager.remote_downloads() {
            Ok(value) => reply(stream, "200 OK", value.to_string().as_bytes()).await,
            Err(err) => {
                reply(
                    stream,
                    "500 Internal Server Error",
                    format!("{{\"error\":\"{err}\"}}").as_bytes(),
                )
                .await
            }
        },
        ("POST", "/api/action") => {
            let body = request.body.unwrap_or_default();
            let task_id = body.get("task_id").and_then(|v| v.as_str()).unwrap_or("");
            let action = body.get("action").and_then(|v| v.as_str()).unwrap_or("");
            match manager.remote_action(task_id, action).await {
                Ok(value) => reply(stream, "200 OK", value.to_string().as_bytes()).await,
                Err(err) => {
                    reply(
                        stream,
                        "400 Bad Request",
                        format!("{{\"error\":\"{}\"}}", err).as_bytes(),
                    )
                    .await
                }
            }
        }
        _ => {
            reply(
                stream,
                "404 Not Found",
                b"{\"error\":\"ERR_REMOTE_NOT_FOUND\"}",
            )
            .await
        }
    }
}

async fn reply<S>(stream: &mut S, status: &str, body: &[u8]) -> std::io::Result<()>
where
    S: tokio::io::AsyncWrite + Unpin,
{
    use tokio::io::AsyncWriteExt;
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_is_random_and_only_its_digest_is_kept() {
        let first = new_token();
        let second = new_token();
        assert_eq!(first.len(), 64);
        assert_ne!(first, second);

        let digest = token_digest(&first);
        assert_eq!(digest.len(), 64);
        assert_ne!(digest, first, "the digest must not be the token");
        assert_eq!(authorize(&digest, &first), Ok(()));
        assert_eq!(
            authorize(&digest, &second),
            Err(RemoteRefusal::InvalidToken)
        );
    }

    #[test]
    fn an_empty_or_oversized_token_is_refused() {
        let digest = token_digest("abc");
        assert_eq!(authorize(&digest, ""), Err(RemoteRefusal::InvalidToken));
        assert_eq!(
            authorize(&digest, &"a".repeat(MAX_TOKEN_LEN + 1)),
            Err(RemoteRefusal::InvalidToken)
        );
    }

    #[test]
    fn parses_the_bearer_token_and_the_body() {
        let raw = b"POST /api/action HTTP/1.1\r\nAuthorization: Bearer abc123\r\nContent-Length: 18\r\n\r\n{\"action\":\"pause\"}";
        let parsed = parse_request(raw, 4096).unwrap();
        assert_eq!(parsed.method, "POST");
        assert_eq!(parsed.path, "/api/action");
        assert_eq!(parsed.token.as_deref(), Some("abc123"));
        assert_eq!(parsed.body.unwrap()["action"], "pause");

        let anonymous = b"GET /api/downloads HTTP/1.1\r\nHost: x\r\n\r\n";
        assert_eq!(parse_request(anonymous, 4096).unwrap().token, None);

        let oversized = b"POST /api/action HTTP/1.1\r\nContent-Length: 999999\r\n\r\n";
        assert_eq!(parse_request(oversized, 4096), Err(RemoteRefusal::TooLarge));
        assert_eq!(
            parse_request(b"garbage", 4096),
            Err(RemoteRefusal::BadRequest)
        );
    }

    #[test]
    fn the_pairing_payload_carries_the_token_once() {
        let payload = pairing_payload("192.168.0.5", 8765, "deadbeef");
        assert_eq!(payload, "http://192.168.0.5:8765/?token=deadbeef");
    }
}
