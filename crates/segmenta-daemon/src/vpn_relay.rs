//! Internal VPN relay: a loopback proxy whose outbound sockets are bound to the interface.
//!
//! Contract: T1 of `odd/tasks/segmenta-hito1-red-inmunidad.md`.
//!
//! # Why a relay
//!
//! `yt-dlp`, `ffmpeg`, `gallery-dl` and the RSS poller open their own sockets, so the daemon
//! cannot bind them itself. They do understand an HTTP proxy, so the daemon publishes one on
//! loopback: the tool talks to `127.0.0.1`, and the relay opens the real connection already
//! bound to `vpn_interface`.
//!
//! # Guards
//!
//! * **Auth**: every request needs `Proxy-Authorization` with the session token; without it
//!   the relay answers `407` and never dials.
//! * **Interface**: `netguard::ensure_up` runs **before each dial**, so a tunnel that went
//!   down stops traffic instead of letting it leave through the default route.
//! * **SSRF**: the target is resolved and checked with the same rules as the engine; loopback,
//!   link-local and private addresses are refused unless `allow_local` was configured. Without
//!   this, a proxied tool could pivot back into the machine through the relay.
//!
//! # Transport
//!
//! `CONNECT host:port` for HTTPS and absolute-form requests (`GET http://host/path`) for plain
//! HTTP. Both directions are pumped with `copy_bidirectional`, so no blocking call ever runs on
//! the Tokio worker.

use std::io;
use std::net::SocketAddr;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Environment variable a child process must receive to use the relay.
pub const PROXY_ENV: &str = "SEGMENTA_VPN_PROXY";

/// Largest request head the relay accepts, in bytes.
const MAX_HEAD: usize = 16 * 1024;

/// Largest plain-HTTP body forwarded in one piece.
const MAX_BODY: usize = 1024 * 1024;

/// One running relay.
pub struct RelayHandle {
    /// Loopback port the tools must use.
    pub port: u16,
    /// Session token, sent as the proxy user or password.
    pub token: String,
    /// `http://token@127.0.0.1:port` for `--proxy` and the proxy env vars.
    pub url: String,
    /// Stops the accept loop when dropped.
    shutdown: tokio::sync::watch::Sender<bool>,
}

impl Drop for RelayHandle {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
    }
}

/// Bind a relay on a free loopback port and serve it in the background.
pub async fn spawn(interface: String, allow_local: bool) -> io::Result<RelayHandle> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
    let port = listener.local_addr()?.port();
    let token = crate::remote::new_token();
    let (shutdown, mut stopped) = tokio::sync::watch::channel(false);
    let serve_token = token.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = stopped.changed() => break,
                accepted = listener.accept() => {
                    let Ok((stream, _peer)) = accepted else { continue };
                    let interface = interface.clone();
                    let token = serve_token.clone();
                    tokio::spawn(async move {
                        let _ = handle(stream, &interface, &token, allow_local).await;
                    });
                }
            }
        }
    });
    Ok(RelayHandle {
        port,
        url: format!("http://{token}@127.0.0.1:{port}"),
        token,
        shutdown,
    })
}

/// Serve one client: authenticate, parse the target and tunnel to it.
pub async fn handle(
    mut client: TcpStream,
    interface: &str,
    token: &str,
    allow_local: bool,
) -> io::Result<()> {
    let head = read_head(&mut client).await?;
    let Some(request) = Request::parse(&head) else {
        return refuse(&mut client, "400 Bad Request").await;
    };
    if !request.authorized(token) {
        return refuse(&mut client, "407 Proxy Authentication Required").await;
    }

    // Kill switch and SSRF run before a socket exists, so a down tunnel or a forbidden
    // target never produces a connection at all.
    if segmenta_core::netguard::ensure_up(interface).is_err() {
        return refuse(&mut client, "503 VPN interface is not up").await;
    }
    let Some(upstream) = dial(interface, &request.authority, allow_local).await else {
        return refuse(&mut client, "502 Bad Gateway").await;
    };

    match request.kind {
        Kind::Connect => {
            client
                .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                .await?;
            let mut upstream = upstream;
            let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
        }
        Kind::Forward => {
            // Forward the head in origin form, then let the byte pump carry the body and
            // the whole response: no blocking read, no buffering of the payload.
            let mut upstream = upstream;
            upstream
                .write_all(request.forwarded_head().as_bytes())
                .await?;
            if let Some(body) = request.body.as_deref() {
                upstream.write_all(body).await?;
            }
            let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
        }
    }
    Ok(())
}

/// A request the relay understands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// Method as written by the client.
    pub method: String,
    /// `CONNECT` or a plain forward.
    pub kind: Kind,
    /// `host:port` to dial.
    pub authority: String,
    /// Path in origin form (`/file.bin`), empty for `CONNECT`.
    pub path: String,
    /// Headers, without the proxy ones.
    pub headers: Vec<(String, String)>,
    /// Credentials presented in `Proxy-Authorization`, when the header was Basic.
    pub credentials: Option<(String, String)>,
    /// Body for forwards that carry one.
    pub body: Option<Vec<u8>>,
}

/// How the proxy is used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `CONNECT host:port`, used for HTTPS.
    Connect,
    /// `GET http://host/path`, used for plain HTTP.
    Forward,
}

impl Request {
    /// Parse the request head (and body, when the head carries a length).
    pub fn parse(raw: &[u8]) -> Option<Request> {
        let split = raw.windows(4).position(|window| window == b"\r\n\r\n")?;
        let head = std::str::from_utf8(&raw[..split]).ok()?;
        let mut lines = head.split("\r\n");
        let request_line = lines.next()?;
        let mut parts = request_line.split_whitespace();
        let method = parts.next()?.to_string();
        let target = parts.next()?.to_string();
        let version = parts.next()?;
        if !version.starts_with("HTTP/1.") {
            return None;
        }
        let mut headers = Vec::new();
        let mut token: Option<(String, String)> = None;
        let mut length = 0usize;
        for line in lines {
            let Some((name, value)) = line.split_once(':') else {
                continue;
            };
            let name = name.trim().to_string();
            let value = value.trim().to_string();
            if name.eq_ignore_ascii_case("proxy-authorization") {
                token = basic_credentials(&value);
                continue;
            }
            if name.eq_ignore_ascii_case("proxy-connection") {
                continue;
            }
            if name.eq_ignore_ascii_case("content-length") {
                length = value.parse().unwrap_or(0);
            }
            headers.push((name, value));
        }
        length = length.min(MAX_BODY);
        let body = (split + 4 + length <= raw.len() && length > 0)
            .then(|| raw[split + 4..split + 4 + length].to_vec());

        if method.eq_ignore_ascii_case("CONNECT") {
            return Some(Request {
                method,
                kind: Kind::Connect,
                authority: with_default_port(&target, 443),
                path: String::new(),
                headers,
                credentials: token,
                body: None,
            });
        }
        // Absolute form: `http://host[:port]/path`.
        let rest = target
            .strip_prefix("http://")
            .or_else(|| target.strip_prefix("https://"))?;
        let (authority, path) = match rest.find('/') {
            Some(index) => (&rest[..index], &rest[index..]),
            None => (rest, "/"),
        };
        Some(Request {
            method,
            kind: Kind::Forward,
            authority: with_default_port(authority, 80),
            path: path.to_string(),
            headers,
            credentials: token,
            body,
        })
    }

    /// Whether the request carries the relay token, in the user or the password part.
    ///
    /// Tools disagree on where the secret goes (`http://token@host` versus
    /// `http://user:token@host`), so both halves are accepted and compared exactly.
    pub fn authorized(&self, token: &str) -> bool {
        match &self.credentials {
            Some((user, password)) => user == token || password == token,
            None => false,
        }
    }

    /// The head to send upstream: origin form, proxy headers removed.
    pub fn forwarded_head(&self) -> String {
        let mut head = format!("{} {} HTTP/1.1\r\n", self.method, self.path);
        for (name, value) in &self.headers {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
        head.push_str("\r\n");
        head
    }
}

/// Split a `Basic` credential into its user and password.
pub fn basic_credentials(value: &str) -> Option<(String, String)> {
    use base64::Engine;
    let encoded = value
        .strip_prefix("Basic ")
        .or_else(|| value.strip_prefix("basic "))?;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded.trim())
        .ok()?;
    let text = String::from_utf8(decoded).ok()?;
    let (user, password) = text.split_once(':').unwrap_or((&text, ""));
    Some((user.to_string(), password.to_string()))
}

/// Read the request head and any body the head announced.
async fn read_head(client: &mut TcpStream) -> io::Result<Vec<u8>> {
    let mut raw = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let read = client.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        raw.extend_from_slice(&chunk[..read]);
        if raw.len() > MAX_HEAD + MAX_BODY {
            break;
        }
        if let Some(split) = raw.windows(4).position(|window| window == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&raw[..split]).to_string();
            let wanted = head
                .split("\r\n")
                .filter_map(|line| line.split_once(':'))
                .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                .and_then(|(_, value)| value.trim().parse::<usize>().ok())
                .unwrap_or(0)
                .min(MAX_BODY);
            if raw.len() >= split + 4 + wanted {
                break;
            }
        }
    }
    Ok(raw)
}

/// Dial `authority` already bound to `interface`, after the SSRF checks.
async fn dial(interface: &str, authority: &str, allow_local: bool) -> Option<TcpStream> {
    let (host, port) = split_authority(authority)?;
    let mut addresses = tokio::net::lookup_host((host.as_str(), port)).await.ok()?;
    let address = loop {
        let candidate = addresses.next()?;
        if allow_local || !segmenta_core::urlssrf::is_blocked_ip(candidate.ip()) {
            break candidate;
        }
    };
    connect_bound(interface, address).await.ok()
}

/// Connect one address with `SO_BINDTODEVICE` set before the handshake.
async fn connect_bound(interface: &str, address: SocketAddr) -> io::Result<TcpStream> {
    use socket2::{Domain, Protocol, Socket, Type};
    let socket = Socket::new(
        Domain::for_address(address),
        Type::STREAM,
        Some(Protocol::TCP),
    )?;
    socket.bind_device(Some(interface.as_bytes()))?;
    socket.set_nonblocking(true)?;
    match socket.connect(&address.into()) {
        Ok(()) => {}
        // A non-blocking connect starts the handshake and reports EINPROGRESS; the
        // readiness below completes it without ever blocking a Tokio worker.
        Err(err)
            if err.raw_os_error() == Some(libc::EINPROGRESS)
                || err.kind() == io::ErrorKind::WouldBlock => {}
        Err(err) => return Err(err),
    }
    let stream = TcpStream::from_std(std::net::TcpStream::from(socket))?;
    stream.writable().await?;
    if let Some(err) = stream.take_error()? {
        return Err(err);
    }
    Ok(stream)
}

fn split_authority(authority: &str) -> Option<(String, u16)> {
    let (host, port) = authority.rsplit_once(':')?;
    let port = port.parse().ok()?;
    Some((host.trim_matches(['[', ']']).to_string(), port))
}

fn with_default_port(authority: &str, default: u16) -> String {
    if authority.contains(':')
        && authority
            .rsplit_once(':')
            .is_some_and(|(_, p)| p.parse::<u16>().is_ok())
    {
        authority.to_string()
    } else {
        format!("{authority}:{default}")
    }
}

async fn refuse(client: &mut TcpStream, status: &str) -> io::Result<()> {
    client
        .write_all(format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\n\r\n").as_bytes())
        .await
}
