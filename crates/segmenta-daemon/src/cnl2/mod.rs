//! Click'n'Load 2: a LOCAL listener that imports a package of links.
//!
//! Contract: T3 of `odd/tasks/segmenta-v0.5.0-ecosistema.md`, and the protocol published
//! by JDownloader: a `POST http://127.0.0.1:9666/flash/addcrypted2` form with `passwords`,
//! `source`, `jk` and `crypted`. `jk` is JavaScript that returns the key; this module
//! **never** evaluates it: only the literal hex form is accepted.
//!
//! # Crypto
//!
//! AES-128-CBC with the key and the IV set to the same 16 bytes, no salt and no padding;
//! the plaintext is a list of URLs separated by CRLF, with NUL bytes discarded.
//!
//! # Exposure
//!
//! Any page open in the browser can reach a loopback port, so the listener is off unless
//! the user turns it on, it only answers a loopback peer, it validates `Host` against the
//! port it is bound to, it bounds the body and the number of links, it imports only
//! `http`/`https` URLs, and it never starts a download by itself: the links land in a
//! package, which the queue then schedules like any other task.

use aes::cipher::{block_padding::NoPadding, BlockDecryptMut, KeyIvInit};
use base64::Engine;
use std::collections::BTreeMap;

/// Port the protocol fixes, and the one browsers and sites hardcode.
pub const DEFAULT_PORT: u16 = 9666;

/// Longest accepted request body, in bytes.
pub const MAX_BODY: usize = 64 * 1024;

/// Longest accepted header block, in bytes.
pub const MAX_HEADERS: usize = 16 * 1024;

/// Most links one package may carry.
pub const MAX_LINKS: usize = 1000;

/// Longest accepted link, in characters.
pub const MAX_LINK: usize = 4096;

/// What one accepted request imports.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Import {
    /// Sanitized, de-duplicated links.
    pub links: Vec<String>,
    /// Archive password sent by the site, kept in memory only.
    pub password: Option<String>,
    /// Referer that names the package.
    pub source: Option<String>,
}

/// Why a request was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// Not HTTP/1.x, missing `Content-Length`, or a malformed line.
    BadRequest(&'static str),
    /// The peer is not loopback.
    Forbidden,
    /// The body is larger than [`MAX_BODY`].
    TooLarge,
    /// `jk` or `crypted` cannot be decoded.
    Crypt,
    /// The payload held no usable link.
    NoLinks,
}

impl Refusal {
    /// HTTP status for the refusal.
    pub fn status(&self) -> &'static str {
        match self {
            Refusal::BadRequest(_) => "400 Bad Request",
            Refusal::Forbidden => "403 Forbidden",
            Refusal::TooLarge => "413 Payload Too Large",
            Refusal::Crypt | Refusal::NoLinks => "400 Bad Request",
        }
    }

    /// Body text the client sees.
    pub fn message(&self) -> String {
        match self {
            Refusal::BadRequest(reason) => format!("bad request: {reason}\r\n"),
            Refusal::Forbidden => "forbidden: loopback only\r\n".to_string(),
            Refusal::TooLarge => "payload too large\r\n".to_string(),
            Refusal::Crypt => "cannot decrypt the payload\r\n".to_string(),
            Refusal::NoLinks => "no usable link\r\n".to_string(),
        }
    }
}

/// Decode the `jk` field into the 16-byte key.
///
/// Accepted forms: a bare 32-character hex string, or JavaScript whose only statement is
/// `return '<hex>'`. Anything else is refused, so no script ever runs.
pub fn decode_jk(jk: &str) -> Option<[u8; 16]> {
    let trimmed = jk.trim();
    let hex = match find_return_literal(trimmed) {
        Some(literal) => literal,
        None if is_hex32(trimmed) => trimmed,
        None => return None,
    };
    let bytes = hex::decode(hex).ok()?;
    bytes.try_into().ok()
}

/// The quoted literal of a `return '...'` statement, when that is all the code does.
///
/// Only a bare `return` of one hex literal is accepted; anything else after the closing
/// quote — another statement, a property access — disqualifies the whole field.
fn find_return_literal(code: &str) -> Option<&str> {
    let start = code.find("return")?;
    let rest = code[start + "return".len()..].trim_start();
    let quote = rest.chars().next().filter(|c| *c == '\'' || *c == '"')?;
    let rest = &rest[1..];
    let end = rest.find(quote)?;
    let literal = &rest[..end];
    if !is_hex32(literal) {
        return None;
    }
    let tail = rest[end + 1..].trim_start().trim_start_matches(';').trim();
    tail.chars()
        .all(|c| matches!(c, '}' | ')' | ' '))
        .then_some(literal)
}

fn is_hex32(value: &str) -> bool {
    value.len() == 32 && value.chars().all(|c| c.is_ascii_hexdigit())
}

/// Decrypt the `crypted` field: base64 AES-128-CBC, no padding, key and IV equal.
pub fn decrypt(crypted: &str, key: &[u8; 16]) -> Option<Vec<u8>> {
    let raw = base64::engine::general_purpose::STANDARD
        .decode(crypted.trim())
        .ok()?;
    if raw.is_empty() || raw.len() % 16 != 0 {
        return None;
    }
    let mut buffer = raw;
    let plain = cbc::Decryptor::<aes::Aes128>::new_from_slices(key, key)
        .ok()?
        .decrypt_padded_mut::<NoPadding>(&mut buffer)
        .ok()?;
    Some(strip_pkcs7(plain).to_vec())
}

/// Strip PKCS#7 padding when the sender added it.
///
/// The reference implementation encrypts without padding and pads the plaintext with NUL
/// bytes to reach a block boundary (those are dropped later); other clients use the usual
/// PKCS#7. Accepting both costs one check and keeps an ordinary sender from being refused.
fn strip_pkcs7(plain: &[u8]) -> &[u8] {
    let Some(&last) = plain.last() else {
        return plain;
    };
    let pad = last as usize;
    if pad == 0 || pad > 16 || pad > plain.len() {
        return plain;
    }
    if plain[plain.len() - pad..].iter().all(|byte| *byte == last) {
        &plain[..plain.len() - pad]
    } else {
        plain
    }
}

/// Split a decrypted payload into sanitized links, in the order the sender wrote them.
///
/// NUL bytes are discarded, the separator is CRLF, and only `http`/`https` URLs of a
/// bounded length survive. Duplicates are dropped so a repeated link cannot fill the queue.
pub fn links_from_payload(payload: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(payload).replace('\0', "");
    let mut seen = std::collections::HashSet::new();
    let mut links = Vec::new();
    for candidate in text.split("\r\n") {
        let link = candidate.trim();
        if link.is_empty() || link.chars().count() > MAX_LINK {
            continue;
        }
        let Some((scheme, _)) = link.split_once("://") else {
            continue;
        };
        if !matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https") {
            continue;
        }
        if !seen.insert(link.to_string()) {
            continue;
        }
        links.push(link.to_string());
        if links.len() == MAX_LINKS {
            break;
        }
    }
    links
}

/// Parse an `application/x-www-form-urlencoded` body.
pub fn form_fields(body: &[u8]) -> BTreeMap<String, String> {
    let text = String::from_utf8_lossy(body);
    let mut fields = BTreeMap::new();
    for pair in text.split('&') {
        let (key, value) = match pair.split_once('=') {
            Some((key, value)) => (key, value),
            None => (pair, ""),
        };
        let key = percent_decode(key);
        if key.is_empty() {
            continue;
        }
        fields.insert(key, percent_decode(value));
    }
    fields
}

/// Decode `%XX` sequences and `+` as a space, leaving malformed escapes literal.
fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                match u8::from_str_radix(&value[index + 1..index + 3], 16) {
                    Ok(byte) => {
                        out.push(byte);
                        index += 3;
                    }
                    Err(_) => {
                        out.push(b'%');
                        index += 1;
                    }
                }
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// One HTTP request, reduced to what the protocol needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// Uppercase method.
    pub method: String,
    /// Path without the query string.
    pub path: String,
    /// `Host` header as sent.
    pub host: Option<String>,
    /// Body bytes.
    pub body: Vec<u8>,
}

/// Parse the request line, headers and body of one request.
pub fn parse_request(raw: &[u8]) -> Result<Request, Refusal> {
    let split = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or(Refusal::BadRequest("missing header terminator"))?;
    let head = std::str::from_utf8(&raw[..split])
        .map_err(|_| Refusal::BadRequest("headers are not UTF-8"))?;
    let mut lines = head.split("\r\n");
    let request_line = lines
        .next()
        .ok_or(Refusal::BadRequest("missing request line"))?;
    let mut parts = request_line.split_whitespace();
    let method = parts
        .next()
        .ok_or(Refusal::BadRequest("missing method"))?
        .to_ascii_uppercase();
    let target = parts.next().ok_or(Refusal::BadRequest("missing target"))?;
    let version = parts.next().unwrap_or_default();
    if !version.starts_with("HTTP/1.") {
        return Err(Refusal::BadRequest("unsupported version"));
    }
    let path = target.split('?').next().unwrap_or(target).to_string();
    let mut host = None;
    let mut length: Option<usize> = None;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        if name.eq_ignore_ascii_case("host") {
            host = Some(value.to_string());
        } else if name.eq_ignore_ascii_case("content-length") {
            length = value.parse::<usize>().ok();
        }
    }
    let body_start = split + 4;
    let available = raw.len() - body_start;
    let wanted = length.unwrap_or(0);
    if wanted > MAX_BODY {
        return Err(Refusal::TooLarge);
    }
    if wanted > available {
        return Err(Refusal::BadRequest("truncated body"));
    }
    Ok(Request {
        method,
        path,
        host,
        body: raw[body_start..body_start + wanted].to_vec(),
    })
}

/// Build an [`Import`] from one `/flash/addcrypted2` form.
pub fn import_from_form(fields: &BTreeMap<String, String>) -> Result<Import, Refusal> {
    let crypted = fields.get("crypted").ok_or(Refusal::Crypt)?;
    let jk = fields.get("jk").ok_or(Refusal::Crypt)?;
    let key = decode_jk(jk).ok_or(Refusal::Crypt)?;
    let payload = decrypt(crypted, &key).ok_or(Refusal::Crypt)?;
    let links = links_from_payload(&payload);
    if links.is_empty() {
        return Err(Refusal::NoLinks);
    }
    Ok(Import {
        links,
        password: fields.get("passwords").filter(|v| !v.is_empty()).cloned(),
        source: fields.get("source").filter(|v| !v.is_empty()).cloned(),
    })
}

/// Build an [`Import`] from one plain `/flash/add` form.
pub fn import_from_plain(fields: &BTreeMap<String, String>) -> Result<Import, Refusal> {
    let body = fields.get("urls").cloned().unwrap_or_default().into_bytes();
    let links = links_from_payload(&body);
    if links.is_empty() {
        return Err(Refusal::NoLinks);
    }
    Ok(Import {
        links,
        password: fields.get("passwords").filter(|v| !v.is_empty()).cloned(),
        source: fields.get("source").filter(|v| !v.is_empty()).cloned(),
    })
}

/// Whether a `Host` header names the loopback address this listener serves.
pub fn host_is_local(host: &str, port: u16) -> bool {
    let expected = port.to_string();
    let (name, port_part) = match host.rsplit_once(':') {
        Some((name, port)) => (name, port),
        None => (host, ""),
    };
    let name_ok = matches!(name, "localhost" | "127.0.0.1" | "[::1]" | "::1");
    name_ok && (port_part.is_empty() || port_part == expected)
}

mod server;
pub use self::server::{bind, serve};
