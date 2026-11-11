//! HTTP probing, redirect handling and capability classification (OpenSpec 06).
//!
//! Every request goes through [`Session`], which resolves the host itself, rejects
//! private addresses, pins the validated address into the client and sends
//! `Accept-Encoding: identity`. Redirects are followed manually so each hop is
//! revalidated.

use std::net::IpAddr;
use std::time::{Duration, SystemTime};

use reqwest::header::{
    ACCEPT_ENCODING, AUTHORIZATION, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE,
    COOKIE, ETAG, HeaderMap, HeaderName, HeaderValue, LAST_MODIFIED, LOCATION, RANGE,
    TRANSFER_ENCODING,
};
use reqwest::redirect::Policy;
use reqwest::{Client, Response, StatusCode};
use url::Url;

use crate::error::{CoreError, Result, StatusDecision};
use crate::retry::parse_retry_after;
use crate::urlssrf::{resolve_checked, validate_scheme};

/// Maximum redirects before the task fails with `ERR_TOO_MANY_REDIRECTS`.
pub const MAX_REDIRECTS: usize = 10;

/// How the server wants the file downloaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferMode {
    /// `206` with a known total: multi-connection segmentation is possible.
    Segmented {
        /// Size of the whole entity in bytes.
        total: u64,
    },
    /// One sequential stream; the total may be unknown (chunked).
    SingleStream {
        /// Declared size, when the server sent one.
        total: Option<u64>,
    },
}

impl TransferMode {
    /// Whether ranges are usable.
    pub fn supports_ranges(&self) -> bool {
        matches!(self, TransferMode::Segmented { .. })
    }

    /// Declared total size, if any.
    pub fn total_bytes(&self) -> Option<u64> {
        match self {
            TransferMode::Segmented { total } => Some(*total),
            TransferMode::SingleStream { total } => *total,
        }
    }
}

/// What the probe learned about the resource.
#[derive(Debug, Clone)]
pub struct ProbeInfo {
    /// Segmentation capability.
    pub mode: TransferMode,
    /// Strong validator, used for `If-Range`.
    pub etag: Option<String>,
    /// Weak fallback validator.
    pub last_modified: Option<String>,
    /// `Content-Type`, when present.
    pub content_type: Option<String>,
    /// Raw `Content-Disposition`, sanitized later by the caller.
    pub content_disposition: Option<String>,
    /// URL after redirects.
    pub final_url: Url,
}

/// Result of one probe attempt.
#[derive(Debug)]
pub enum ProbeOutcome {
    /// The server answered something usable.
    Ready(ProbeInfo),
    /// Retryable status; `retry_after` carries the parsed header when present.
    Retry { retry_after: Option<Duration> },
    /// Needs the user (auth) and keeps its state.
    Pause(CoreError),
    /// Terminal.
    Fail(CoreError),
}

/// HTTP behaviour shared by the probe and the workers.
#[derive(Debug, Clone)]
pub struct HttpConfig {
    /// `User-Agent` sent with every request.
    pub user_agent: String,
    /// Allow loopback/private targets (tests and explicit user opt-in).
    pub allow_local: bool,
    /// Optional proxy URL from settings.
    pub proxy: Option<String>,
    /// Headers that may be sent cross-origin.
    pub headers: HeaderMap,
    /// `Authorization`/`Cookie`: dropped on cross-origin redirects.
    pub sensitive_headers: HeaderMap,
    /// TCP connect timeout.
    pub connect_timeout: Duration,
    /// Maximum silence on a body read.
    pub read_timeout: Duration,
}

impl Default for HttpConfig {
    fn default() -> Self {
        HttpConfig {
            user_agent: concat!("Segmenta/", env!("CARGO_PKG_VERSION")).to_string(),
            allow_local: false,
            proxy: None,
            headers: HeaderMap::new(),
            sensitive_headers: HeaderMap::new(),
            connect_timeout: Duration::from_secs(15),
            read_timeout: Duration::from_secs(30),
        }
    }
}

/// A client pinned to the validated address of one host.
#[derive(Debug, Clone)]
pub struct Session {
    client: Client,
    url: Url,
    headers: HeaderMap,
    sensitive_headers: HeaderMap,
    pub read_timeout: Duration,
}

impl Session {
    /// Resolve, validate and pin `url`, then build a reusable client.
    pub async fn open(cfg: &HttpConfig, url: &Url) -> Result<Session> {
        validate_scheme(url)?;
        let addrs = resolve_checked(url, cfg.allow_local).await?;
        let mut builder = Client::builder()
            .redirect(Policy::none())
            .connect_timeout(cfg.connect_timeout)
            .user_agent(cfg.user_agent.clone());
        if let Some(proxy) = &cfg.proxy {
            builder = builder.proxy(reqwest::Proxy::all(proxy).map_err(|err| {
                CoreError::InvalidUrl(format!("invalid proxy `{proxy}`: {err}"))
            })?);
        }
        let host = url.host_str().unwrap_or_default();
        if host.parse::<IpAddr>().is_err() {
            // reqwest would resolve the name again; pin the address we validated.
            builder = builder.resolve(host, addrs[0]);
        }
        let client = builder.build().map_err(|err| CoreError::Network(err.to_string()))?;
        Ok(Session {
            client,
            url: url.clone(),
            headers: cfg.headers.clone(),
            sensitive_headers: cfg.sensitive_headers.clone(),
            read_timeout: cfg.read_timeout,
        })
    }

    /// URL this session is bound to.
    pub fn url(&self) -> &Url {
        &self.url
    }

    /// Non-sensitive headers.
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// Sensitive headers.
    pub fn sensitive_headers(&self) -> &HeaderMap {
        &self.sensitive_headers
    }

    /// Replace the sensitive headers (used when the caller resolves a secret).
    pub fn set_sensitive_headers(&mut self, headers: HeaderMap) {
        self.sensitive_headers = headers;
    }

    /// Issue a `GET`, optionally with an inclusive byte range and `If-Range`.
    pub async fn request(
        &self,
        range: Option<(u64, u64)>,
        if_range: Option<&str>,
    ) -> Result<Response> {
        let mut headers = self.headers.clone();
        for (name, value) in self.sensitive_headers.iter() {
            headers.insert(name, value.clone());
        }
        headers.insert(
            ACCEPT_ENCODING,
            HeaderValue::from_static("identity"),
        );
        if let Some((start, end)) = range {
            let value = format!("bytes={start}-{end}");
            headers.insert(
                RANGE,
                HeaderValue::from_str(&value)
                    .map_err(|err| CoreError::MalformedHeader(err.to_string()))?,
            );
        }
        if let Some(validator) = if_range {
            headers.insert(
                HeaderName::from_static("if-range"),
                HeaderValue::from_str(validator)
                    .map_err(|err| CoreError::MalformedHeader(err.to_string()))?,
            );
        }
        let response = self
            .client
            .get(self.url.clone())
            .headers(headers)
            .send()
            .await
            .map_err(|err| CoreError::Network(err.to_string()))?;
        Ok(response)
    }
}

fn header_string(headers: &HeaderMap, name: HeaderName) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

/// Parse `Content-Range: bytes start-end/total` (`*` means unknown total).
pub fn parse_content_range(value: &str) -> Result<(u64, u64, Option<u64>)> {
    let rest = value
        .trim()
        .strip_prefix("bytes ")
        .ok_or_else(|| CoreError::MalformedHeader(format!("content-range `{value}`")))?;
    let (range, total) = rest
        .split_once('/')
        .ok_or_else(|| CoreError::MalformedHeader(format!("content-range `{value}`")))?;
    let (start, end) = range
        .split_once('-')
        .ok_or_else(|| CoreError::MalformedHeader(format!("content-range `{value}`")))?;
    let start: u64 = start
        .trim()
        .parse()
        .map_err(|_| CoreError::MalformedHeader(format!("content-range start `{start}`")))?;
    let end: u64 = end
        .trim()
        .parse()
        .map_err(|_| CoreError::MalformedHeader(format!("content-range end `{end}`")))?;
    if end < start {
        return Err(CoreError::MalformedHeader(format!(
            "content-range end before start: `{value}`"
        )));
    }
    let total = match total.trim() {
        "*" => None,
        digits => Some(
            digits
                .parse()
                .map_err(|_| CoreError::MalformedHeader(format!("content-range total `{digits}`")))?,
        ),
    };
    Ok((start, end, total))
}

/// Probe `url`, following redirects and revalidating each hop.
pub async fn probe(cfg: &HttpConfig, url: &Url) -> Result<ProbeOutcome> {
    let mut current = url.clone();
    let mut sensitive = cfg.sensitive_headers.clone();
    for hop in 0..=MAX_REDIRECTS {
        if hop == MAX_REDIRECTS {
            return Ok(ProbeOutcome::Fail(CoreError::TooManyRedirects));
        }
        let mut hop_cfg = cfg.clone();
        hop_cfg.sensitive_headers = sensitive.clone();
        let session = Session::open(&hop_cfg, &current).await?;
        let response = session.request(Some((0, 1)), None).await?;
        let status = response.status();
        if status.is_redirection() {
            let Some(location) = header_string(response.headers(), LOCATION) else {
                return Ok(ProbeOutcome::Fail(CoreError::Unsupported(
                    "redirect without Location".to_string(),
                )));
            };
            let next = current.join(&location).map_err(|err| {
                CoreError::InvalidUrl(format!("redirect `{location}`: {err}"))
            })?;
            if next.origin() != current.origin() {
                // Cross-origin: never forward credentials.
                sensitive = HeaderMap::new();
            }
            current = next;
            continue;
        }
        return Ok(classify_probe(response, current).await?);
    }
    Ok(ProbeOutcome::Fail(CoreError::TooManyRedirects))
}

async fn classify_probe(response: Response, url: Url) -> Result<ProbeOutcome> {
    let status = response.status();
    let headers = response.headers().clone();
    match StatusDecision::from_status(status.as_u16()) {
        StatusDecision::Proceed => {}
        StatusDecision::Retry(_) => {
            return Ok(ProbeOutcome::Retry {
                retry_after: parse_retry_after(
                    header_string(&headers, reqwest::header::RETRY_AFTER).as_deref(),
                    SystemTime::now(),
                ),
            });
        }
        StatusDecision::Pause(err) => return Ok(ProbeOutcome::Pause(err)),
        StatusDecision::Fail(err) => return Ok(ProbeOutcome::Fail(err)),
    }

    let details = response_details(&headers, url);

    if status == StatusCode::PARTIAL_CONTENT {
        let content_encoding = header_string(&headers, reqwest::header::CONTENT_ENCODING);
        if content_encoding
            .as_deref()
            .map(|value| !value.eq_ignore_ascii_case("identity"))
            .unwrap_or(false)
        {
            return Ok(ProbeOutcome::Fail(CoreError::Unsupported(format!(
                "server compressed a range response (Content-Encoding: {})",
                content_encoding.unwrap_or_default()
            ))));
        }
        let content_range = header_string(&headers, CONTENT_RANGE).ok_or_else(|| {
            CoreError::MalformedHeader("206 without Content-Range".to_string())
        })?;
        let (_, _, total) = parse_content_range(&content_range)?;
        return Ok(ProbeOutcome::Ready(ProbeInfo {
            mode: match total {
                Some(total) if total > 0 => TransferMode::Segmented { total },
                // Unknown total: nothing to segment.
                _ => TransferMode::SingleStream { total: None },
            },
            etag: details.etag,
            last_modified: details.last_modified,
            content_type: details.content_type,
            content_disposition: details.content_disposition,
            final_url: details.final_url,
        }));
    }

    // 200: the server ignored (or never saw) the range.
    let content_encoding = header_string(&headers, reqwest::header::CONTENT_ENCODING);
    if content_encoding
        .as_deref()
        .map(|value| !value.eq_ignore_ascii_case("identity"))
        .unwrap_or(false)
    {
        // Decompressing on the fly is a 0.1.0 feature; refuse rather than write
        // compressed bytes at file offsets.
        return Ok(ProbeOutcome::Fail(CoreError::Unsupported(format!(
            "server compressed the response (Content-Encoding: {}); decompression is not implemented",
            content_encoding.unwrap_or_default()
        ))));
    }
    let chunked = header_string(&headers, TRANSFER_ENCODING)
        .map(|value| value.to_ascii_lowercase().contains("chunked"))
        .unwrap_or(false);
    let total = if chunked {
        None
    } else {
        header_string(&headers, CONTENT_LENGTH).and_then(|value| value.parse::<u64>().ok())
    };
    Ok(ProbeOutcome::Ready(ProbeInfo {
        mode: TransferMode::SingleStream { total },
        etag: details.etag,
        last_modified: details.last_modified,
        content_type: details.content_type,
        content_disposition: details.content_disposition,
        final_url: details.final_url,
    }))
}

struct ResponseDetails {
    etag: Option<String>,
    last_modified: Option<String>,
    content_type: Option<String>,
    content_disposition: Option<String>,
    final_url: Url,
}

fn response_details(headers: &HeaderMap, url: Url) -> ResponseDetails {
    ResponseDetails {
        etag: header_string(headers, ETAG),
        last_modified: header_string(headers, LAST_MODIFIED),
        content_type: header_string(headers, CONTENT_TYPE),
        content_disposition: header_string(headers, CONTENT_DISPOSITION),
        final_url: url,
    }
}

/// Header names that must never cross an origin boundary.
pub fn is_sensitive_header(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name == AUTHORIZATION.as_str() || name == COOKIE.as_str() || name == "proxy-authorization"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_content_range_parsing() {
        assert_eq!(
            parse_content_range("bytes 0-1/1048576").unwrap(),
            (0, 1, Some(1_048_576))
        );
        assert_eq!(parse_content_range("bytes 10-19/*").unwrap(), (10, 19, None));
        assert!(parse_content_range("items 0-1/10").is_err());
        assert!(parse_content_range("bytes 5-1/10").is_err());
        assert!(parse_content_range("bytes x-y/10").is_err());
    }

    #[test]
    fn sensitive_headers_are_recognised_case_insensitively() {
        assert!(is_sensitive_header("Authorization"));
        assert!(is_sensitive_header("cookie"));
        assert!(!is_sensitive_header("Referer"));
        assert!(!is_sensitive_header("User-Agent"));
    }
}
