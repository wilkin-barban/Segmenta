//! The loopback HTTP listener of Click'n'Load 2.
//!
//! It owns no policy: parsing, decryption and the link rules live in the parent module,
//! and the import itself belongs to the manager.

use super::{
    form_fields, host_is_local, import_from_form, import_from_plain, parse_request, Refusal,
    MAX_BODY, MAX_HEADERS,
};
use std::io;
use std::sync::Arc;

/// Serve the listener until `shutdown` fires.
///
/// Every connection is handled on its own task; a malformed request closes only its own
/// connection, and an import error never stops the listener.
pub async fn serve(
    listener: tokio::net::TcpListener,
    port: u16,
    manager: Arc<crate::manager::Manager>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            accepted = listener.accept() => {
                let Ok((mut stream, peer)) = accepted else { continue };
                if !peer.ip().is_loopback() {
                    let _ = respond(&mut stream, Refusal::Forbidden).await;
                    continue;
                }
                let manager = Arc::clone(&manager);
                tokio::spawn(async move {
                    let _ = handle_connection(&mut stream, port, &manager).await;
                });
            }
        }
    }
}

/// Read one request from `stream`, answer it, and report the import when it happened.
async fn handle_connection(
    stream: &mut tokio::net::TcpStream,
    port: u16,
    manager: &Arc<crate::manager::Manager>,
) -> io::Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut raw = Vec::new();
    let mut chunk = [0u8; 4096];
    // Read until the header terminator, then until the declared body is complete.
    loop {
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        raw.extend_from_slice(&chunk[..read]);
        if raw.len() > MAX_HEADERS + MAX_BODY {
            return respond(stream, Refusal::TooLarge).await;
        }
        if let Some(split) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
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
    let request = match parse_request(&raw) {
        Ok(request) => request,
        Err(refusal) => return respond(stream, refusal).await,
    };
    if let Some(host) = &request.host {
        if !host_is_local(host, port) {
            return respond(stream, Refusal::Forbidden).await;
        }
    }
    if request.method == "GET" {
        let body = match request.path.as_str() {
            "/flash/check" | "/jdcheck.js" => "JDownloader",
            _ => "Segmenta Click'n'Load 2",
        };
        stream
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            )
            .await?;
        return Ok(());
    }
    let fields = form_fields(&request.body);
    let imported = match request.path.as_str() {
        "/flash/addcrypted2" => import_from_form(&fields),
        "/flash/add" => import_from_plain(&fields),
        _ => return respond(stream, Refusal::BadRequest("unknown path")).await,
    };
    match imported {
        Ok(import) => match manager.import_cnl2(import) {
            Ok(added) => {
                let body = format!("success\r\n{added}\r\n");
                stream
                    .write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes())
                    .await?;
                Ok(())
            }
            Err(reason) => {
                let body = format!("{reason}\r\n");
                stream
                    .write_all(format!("HTTP/1.1 500 Internal Server Error\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes())
                    .await
            }
        },
        Err(refusal) => respond(stream, refusal).await,
    }
}

async fn respond(stream: &mut tokio::net::TcpStream, refusal: Refusal) -> io::Result<()> {
    use tokio::io::AsyncWriteExt;
    let body = refusal.message();
    let head = format!(
        "HTTP/1.1 {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        refusal.status(),
        body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body.as_bytes()).await
}

/// Bind the listener on `port`, reporting whether the port was already taken.
pub async fn bind(port: u16) -> Result<tokio::net::TcpListener, io::Error> {
    tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await
}
