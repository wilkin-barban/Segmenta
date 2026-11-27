//! Segmented FTP client (plain `ftp://`). FTPS arrives in its own unit.
//!
//! Contract: S2 of `odd/tasks/segmenta-hito2-captura-hosters-remoto.md`.
//!
//! # Why a hand-written client
//!
//! The engine's segmentation, preallocation and per-host limits already exist; what was
//! missing was a transport that speaks FTP. This module reuses `planner`, `disk` and
//! `hostlimits` and adds only the protocol: one control connection to probe, then one
//! control + one data connection per segment with `REST <offset>` + `RETR`.
//!
//! # Safety
//!
//! * The control target goes through `urlssrf` before the socket, like every other request.
//! * The address a `227` answer advertises is **ignored**: the data connection goes to the
//!   host the user asked for. That is both the NAT fix and the classic FTP-bounce defence.
//! * A configured `vpn_interface` is honoured; a dead tunnel refuses the transfer.

use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use url::Url;

use crate::disk::DiskWriter;
use crate::error::{CoreError, Result};
use crate::hostlimits::HostLimits;
use crate::planner::{self, SegmentRange};

/// What the server says about the object before any byte is transferred.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FtpProbe {
    /// Size reported by `SIZE`, when the server supports it.
    pub size: Option<u64>,
    /// Modification time from `MDTM`, used the way HTTP uses `Last-Modified`.
    pub modified: Option<String>,
}

/// How a transfer should behave.
#[derive(Debug, Clone)]
pub struct FtpConfig {
    /// Segments to open when the size allows it.
    pub connections: u32,
    /// Allow loopback/private targets (tests and the user's own switch).
    pub allow_local: bool,
    /// Interface every socket is bound to, if any.
    pub interface: Option<String>,
    /// Timeout for one control or data operation.
    pub timeout: Duration,
}

impl Default for FtpConfig {
    fn default() -> Self {
        FtpConfig {
            connections: 4,
            allow_local: false,
            interface: None,
            timeout: Duration::from_secs(30),
        }
    }
}

/// Credentials and location taken from the link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FtpTarget {
    /// Host to connect to.
    pub host: String,
    /// Control port.
    pub port: u16,
    /// User name; `anonymous` when the link carries none.
    pub user: String,
    /// Password; `anonymous@` for the anonymous user.
    pub password: String,
    /// Absolute path of the file.
    pub path: String,
}

impl FtpTarget {
    /// Read `ftp://[user:pass@]host[:port]/path`.
    pub fn parse(url: &Url) -> Option<FtpTarget> {
        if url.scheme() != "ftp" {
            return None;
        }
        let host = url.host_str()?.to_string();
        let user = if url.username().is_empty() {
            "anonymous".to_string()
        } else {
            percent_decode(url.username())
        };
        let password = match url.password() {
            Some(value) => percent_decode(value),
            None if user == "anonymous" => "anonymous@".to_string(),
            None => String::new(),
        };
        let path = percent_decode(url.path());
        if path.is_empty() {
            return None;
        }
        Some(FtpTarget {
            host,
            port: url.port().unwrap_or(21),
            user,
            password,
            path,
        })
    }
}

fn percent_decode(value: &str) -> String {
    percent_encoding::percent_decode_str(value)
        .decode_utf8_lossy()
        .to_string()
}

/// The reply to one command: code plus message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    /// Three-digit code.
    pub code: u16,
    /// Text after the code, joined across continuation lines.
    pub message: String,
}

impl Reply {
    /// Codes 2xx are success for the commands used here.
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.code)
    }
}

/// One control connection.
pub struct Control {
    reader: BufReader<tokio::io::ReadHalf<TcpStream>>,
    writer: tokio::io::WriteHalf<TcpStream>,
    timeout: Duration,
}

impl Control {
    /// Open and authenticate a control connection.
    pub async fn open(target: &FtpTarget, cfg: &FtpConfig) -> Result<Control> {
        let addr = resolve(&target.host, target.port, cfg).await?;
        let stream = connect(addr, cfg).await?;
        let (read, write) = tokio::io::split(stream);
        let mut control = Control {
            reader: BufReader::new(read),
            writer: write,
            timeout: cfg.timeout,
        };
        control.expect_success().await?;
        control
            .command(&format!("USER {}", target.user))
            .await
            .and_then(|reply| {
                if reply.code == 331 {
                    Ok(reply)
                } else {
                    Err(CoreError::Network(format!(
                        "ftp user rejected: {} {}",
                        reply.code, reply.message
                    )))
                }
            })?;
        control
            .command(&format!("PASS {}", target.password))
            .await
            .and_then(require_success)?;
        // Binary mode: without it the server may translate line endings in the payload.
        control.command("TYPE I").await.and_then(require_success)?;
        Ok(control)
    }

    /// Send one command and read its reply.
    pub async fn command(&mut self, line: &str) -> Result<Reply> {
        let mut payload = line.as_bytes().to_vec();
        payload.extend_from_slice(b"\r\n");
        let write = self.writer.write_all(&payload);
        tokio::time::timeout(self.timeout, write)
            .await
            .map_err(|_| CoreError::Network("ftp control write timed out".to_string()))?
            .map_err(|err| CoreError::Network(err.to_string()))?;
        self.read_reply().await
    }

    /// Read one (possibly multi-line) reply.
    pub async fn read_reply(&mut self) -> Result<Reply> {
        let mut message = Vec::new();
        loop {
            let mut line = String::new();
            let read = self.reader.read_line(&mut line);
            let read = tokio::time::timeout(self.timeout, read)
                .await
                .map_err(|_| CoreError::Network("ftp control read timed out".to_string()))?
                .map_err(|err| CoreError::Network(err.to_string()))?;
            if read == 0 {
                return Err(CoreError::Network(
                    "ftp server closed the control connection".to_string(),
                ));
            }
            let trimmed = line.trim_end_matches(['\r', '\n']);
            if message.is_empty() {
                let code = trimmed
                    .get(..3)
                    .and_then(|value| value.parse::<u16>().ok())
                    .ok_or_else(|| CoreError::Network(format!("ftp reply `{trimmed}`")))?;
                let rest = trimmed.get(4..).unwrap_or_default().to_string();
                // `123-` opens a multi-line reply that ends at `123 `.
                if trimmed.as_bytes().get(3) == Some(&b'-') {
                    message.push(rest);
                    continue;
                }
                return Ok(Reply {
                    code,
                    message: rest,
                });
            }
            message.push(trimmed.to_string());
            if trimmed
                .get(4..)
                .map(|_| trimmed.as_bytes().get(3) == Some(&b' '))
                .unwrap_or(false)
                && trimmed
                    .get(..3)
                    .and_then(|value| value.parse::<u16>().ok())
                    .is_some()
            {
                let code = trimmed[..3].parse::<u16>().unwrap_or(0);
                return Ok(Reply {
                    code,
                    message: message.join(" "),
                });
            }
        }
    }

    /// Read a reply that must be a success.
    pub async fn expect_success(&mut self) -> Result<Reply> {
        let reply = self.read_reply().await?;
        require_success(reply)
    }

    /// `SIZE <path>`: the size in bytes, when the server implements it.
    pub async fn size(&mut self, path: &str) -> Result<Option<u64>> {
        let reply = self.command(&format!("SIZE {path}")).await?;
        if !reply.is_success() {
            return Ok(None);
        }
        Ok(reply.message.trim().parse::<u64>().ok())
    }

    /// `MDTM <path>`: the modification time, when the server implements it.
    pub async fn modified(&mut self, path: &str) -> Result<Option<String>> {
        let reply = self.command(&format!("MDTM {path}")).await?;
        if !reply.is_success() {
            return Ok(None);
        }
        let value = reply.message.trim();
        Ok((!value.is_empty()).then(|| value.to_string()))
    }

    /// `EPSV`, falling back to `PASV`; returns the data port on the control host.
    pub async fn passive_port(&mut self) -> Result<u16> {
        let reply = self.command("EPSV").await?;
        if reply.is_success() {
            if let Some(port) = parse_epsv(&reply.message) {
                return Ok(port);
            }
        }
        let reply = self.command("PASV").await?;
        if !reply.is_success() {
            return Err(CoreError::Network(format!(
                "ftp refused passive mode: {} {}",
                reply.code, reply.message
            )));
        }
        parse_pasv(&reply.message)
            .ok_or_else(|| CoreError::Network(format!("ftp malformed PASV: {}", reply.message)))
    }

    /// `QUIT`, ignoring whatever the server answers.
    pub async fn quit(mut self) {
        let _ = self.command("QUIT").await;
    }
}

fn require_success(reply: Reply) -> Result<Reply> {
    if reply.is_success() {
        Ok(reply)
    } else {
        Err(CoreError::Network(format!(
            "ftp error: {} {}",
            reply.code, reply.message
        )))
    }
}

/// Port from `227 Entering Passive Mode (h1,h2,h3,h4,p1,p2)`.
///
/// Only the port is taken: the advertised address is ignored on purpose, because it is
/// wrong behind NAT and it is the lever of an FTP bounce attack.
pub fn parse_pasv(message: &str) -> Option<u16> {
    let open = message.find('(')?;
    let close = message[open..].find(')')? + open;
    let numbers: Vec<u16> = message[open + 1..close]
        .split(',')
        .filter_map(|part| part.trim().parse::<u16>().ok())
        .collect();
    if numbers.len() != 6 {
        return None;
    }
    Some(numbers[4] * 256 + numbers[5])
}

/// Port from `229 Entering Extended Passive Mode (|||6446|)`.
pub fn parse_epsv(message: &str) -> Option<u16> {
    let open = message.find('(')?;
    let close = message[open..].find(')')? + open;
    let body = &message[open + 1..close];
    let delimiter = body.chars().next()?;
    body.split(delimiter)
        .rfind(|part| !part.is_empty())?
        .parse::<u16>()
        .ok()
}

/// Probe a file: size and modification time of one control connection.
pub async fn probe(url: &Url, cfg: &FtpConfig) -> Result<FtpProbe> {
    let target = FtpTarget::parse(url).ok_or_else(|| CoreError::InvalidUrl(url.to_string()))?;
    let mut control = Control::open(&target, cfg).await?;
    let size = control.size(&target.path).await?;
    let modified = control.modified(&target.path).await?;
    control.quit().await;
    Ok(FtpProbe { size, modified })
}

/// Download `url` into `path`, segmented when the size is known.
///
/// Returns the bytes written. A server that refuses `SIZE` still transfers: the whole file
/// is read in one stream and written sequentially.
pub async fn download(
    url: &Url,
    path: &Path,
    cfg: &FtpConfig,
    limits: Option<&HostLimits>,
    cancel: &crate::engine::Canceller,
) -> Result<u64> {
    let target = FtpTarget::parse(url).ok_or_else(|| CoreError::InvalidUrl(url.to_string()))?;
    let probe = probe(url, cfg).await?;
    let total = probe.size;
    let (_writer, _outcome) = DiskWriter::create_new(path, total)?;
    let writer = std::sync::Arc::new(_writer);
    let ranges: Vec<SegmentRange> = match total {
        Some(total) => planner::initial_segments(total, capped(cfg, limits, &target.host)),
        None => vec![SegmentRange {
            start: 0,
            end: u64::MAX,
        }],
    };
    let mut handles = Vec::new();
    for range in ranges {
        let target = target.clone();
        let cfg = cfg.clone();
        let writer = std::sync::Arc::clone(&writer);
        let cancel = cancel.clone();
        handles.push(tokio::spawn(async move {
            transfer_segment(&target, range, &cfg, writer, &cancel).await
        }));
    }
    let mut written = 0u64;
    for handle in handles {
        written += handle
            .await
            .map_err(|err| CoreError::Network(format!("ftp segment task: {err}")))??;
    }
    writer.sync_all()?;
    Ok(written)
}

fn capped(cfg: &FtpConfig, limits: Option<&HostLimits>, host: &str) -> u32 {
    let requested = cfg.connections.max(1);
    match limits {
        Some(limits) => requested.min(limits.limit(host).max(1) as u32),
        None => requested,
    }
}

/// Move one range: control → passive → data → REST → RETR → write at offsets.
async fn transfer_segment(
    target: &FtpTarget,
    range: SegmentRange,
    cfg: &FtpConfig,
    writer: std::sync::Arc<DiskWriter>,
    cancel: &crate::engine::Canceller,
) -> Result<u64> {
    let mut control = Control::open(target, cfg).await?;
    let port = control.passive_port().await?;
    let data_addr = resolve(&target.host, port, cfg).await?;
    let mut data = connect(data_addr, cfg).await?;
    if range.start > 0 {
        let reply = control.command(&format!("REST {}", range.start)).await?;
        if reply.code != 350 {
            return Err(CoreError::Network(format!(
                "ftp refused REST {}: {} {}",
                range.start, reply.code, reply.message
            )));
        }
    }
    // `150` (or `125`) is the preliminary answer that opens the data connection; the final
    // `226` only arrives once the stream ends. Requiring 2xx here would refuse every
    // well-behaved server.
    let reply = control.command(&format!("RETR {}", target.path)).await?;
    if !(reply.code == 125 || reply.code == 150 || reply.is_success()) {
        return Err(CoreError::Network(format!(
            "ftp refused RETR: {} {}",
            reply.code, reply.message
        )));
    }

    let mut offset = range.start;
    let mut buffer = vec![0u8; 64 * 1024];
    let mut written = 0u64;
    loop {
        if cancel.is_cancelled() {
            break;
        }
        let read = tokio::time::timeout(cfg.timeout, data.read(&mut buffer))
            .await
            .map_err(|_| CoreError::Network("ftp data read timed out".to_string()))?
            .map_err(|err| CoreError::Network(err.to_string()))?;
        if read == 0 {
            break;
        }
        let chunk = &buffer[..read];
        // A single range may be shorter than the stream when the server ignores REST.
        let allowed = if range.end == u64::MAX {
            chunk.len() as u64
        } else {
            (range.end.saturating_sub(offset) + 1).min(chunk.len() as u64)
        };
        if allowed == 0 {
            break;
        }
        writer.pwrite_all(&chunk[..allowed as usize], offset)?;
        offset += allowed;
        written += allowed;
        if range.end != u64::MAX && offset > range.end {
            break;
        }
    }
    // The data connection is closed before the control one: a server keeps sending the
    // rest of the file into a socket nobody reads, and waiting for its `226` would block
    // for the whole timeout. For a range, closing early is the point; the bytes counted
    // above are what actually landed.
    drop(data);
    control.quit().await;
    Ok(written)
}

/// Resolve and check the destination before any socket exists.
///
/// `urlssrf::resolve_checked` speaks http(s) URLs, so the same rules are applied here
/// directly: every candidate address is checked against the blocked ranges, and a private
/// answer is refused unless the user allowed local downloads.
async fn resolve(host: &str, port: u16, cfg: &FtpConfig) -> Result<SocketAddr> {
    if let Some(interface) = cfg.interface.as_deref() {
        crate::netguard::ensure_up(interface).map_err(|problem| {
            CoreError::Network(format!("vpn interface `{interface}`: {}", problem.code()))
        })?;
    }
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host, port))
        .await
        .map_err(|err| CoreError::InvalidUrl(format!("{host}:{port}: {err}")))?
        .collect();
    if addrs.is_empty() {
        return Err(CoreError::InvalidUrl(format!(
            "{host}:{port} did not resolve"
        )));
    }
    if !cfg.allow_local {
        if let Some(blocked) = addrs
            .iter()
            .find(|addr| crate::urlssrf::is_blocked_ip(addr.ip()))
        {
            return Err(CoreError::BlockedPrivateIp(format!(
                "{host} resolves to {}; enable local downloads to allow it",
                blocked.ip()
            )));
        }
    }
    Ok(addrs[0])
}

/// Connect, bound to the configured interface when there is one.
///
/// `SO_BINDTODEVICE` has to be set before the handshake, so the socket is built with
/// socket2 and handed to Tokio once it is connected: the same shape the VPN relay uses.
async fn connect(addr: SocketAddr, cfg: &FtpConfig) -> Result<TcpStream> {
    use socket2::{Domain, Protocol, Socket, Type};
    let socket = Socket::new(Domain::for_address(addr), Type::STREAM, Some(Protocol::TCP))
        .map_err(|err| CoreError::Network(err.to_string()))?;
    if let Some(interface) = cfg.interface.as_deref() {
        socket
            .bind_device(Some(interface.as_bytes()))
            .map_err(|err| CoreError::Network(format!("cannot bind to {interface}: {err}")))?;
    }
    socket
        .set_nonblocking(true)
        .map_err(|err| CoreError::Network(err.to_string()))?;
    match socket.connect(&addr.into()) {
        Ok(()) => {}
        // A non-blocking connect reports EINPROGRESS; readiness below completes it.
        Err(err)
            if err.raw_os_error() == Some(libc::EINPROGRESS)
                || err.kind() == std::io::ErrorKind::WouldBlock => {}
        Err(err) => return Err(CoreError::Network(format!("ftp connect to {addr}: {err}"))),
    }
    let stream = TcpStream::from_std(std::net::TcpStream::from(socket))
        .map_err(|err| CoreError::Network(err.to_string()))?;
    tokio::time::timeout(cfg.timeout, stream.writable())
        .await
        .map_err(|_| CoreError::Network(format!("ftp connect to {addr} timed out")))?
        .map_err(|err| CoreError::Network(err.to_string()))?;
    if let Some(err) = stream
        .take_error()
        .map_err(|err| CoreError::Network(err.to_string()))?
    {
        return Err(CoreError::Network(format!("ftp connect to {addr}: {err}")));
    }
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(url: &str) -> Option<FtpTarget> {
        FtpTarget::parse(&Url::parse(url).unwrap())
    }

    #[test]
    fn a_link_carries_credentials_and_path() {
        let parsed = target("ftp://user:secret@example.test:2121/pub/file.zip").unwrap();
        assert_eq!(parsed.host, "example.test");
        assert_eq!(parsed.port, 2121);
        assert_eq!(parsed.user, "user");
        assert_eq!(parsed.password, "secret");
        assert_eq!(parsed.path, "/pub/file.zip");
        // Anonymous is the default and needs the conventional password.
        let anon = target("ftp://example.test/file.zip").unwrap();
        assert_eq!(anon.user, "anonymous");
        assert_eq!(anon.password, "anonymous@");
        assert_eq!(anon.port, 21);
        assert_eq!(target("http://example.test/x"), None);
    }

    #[test]
    fn passive_answers_are_read_without_trusting_their_address() {
        assert_eq!(
            parse_pasv("Entering Passive Mode (10,0,0,1,195,80)"),
            Some(195 * 256 + 80)
        );
        assert_eq!(parse_pasv("Entering Passive Mode (1,2,3)"), None);
        assert_eq!(
            parse_epsv("Entering Extended Passive Mode (|||6446|)"),
            Some(6446)
        );
        assert_eq!(parse_epsv("Entering Extended Passive Mode (!!!)"), None);
    }
}
