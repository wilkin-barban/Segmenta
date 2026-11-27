//! Native extractors for clouds, hosters and repositories.
//!
//! Contract: S1 of `odd/tasks/segmenta-hito2-captura-hosters-remoto.md`.
//!
//! # Why
//!
//! A hoster page is not a file. Today `https://www.mediafire.com/file/...` downloads an
//! HTML document named like the file, and the user only notices when opening it. Each
//! module here turns one page into the real URL, and says so out loud when the page no
//! longer looks like what it expects: returning the page URL instead would hide a broken
//! extractor behind a saved `.html`.
//!
//! # Safety
//!
//! An extractor is a redirect machine fed by a remote page, so it must never widen where
//! the engine can connect. Every answer is checked here for scheme `http`/`https`; the
//! caller still runs `urlssrf::resolve_checked` on the result before opening a socket.

use url::Url;

use crate::probe::HttpConfig;

pub mod gofile;
pub mod google_drive;
pub mod internet_archive;
pub mod mediafire;
pub mod rapidgator;

/// What an extractor produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    /// Real file URL.
    pub url: Url,
    /// Headers the CDN expects on the download request.
    pub headers: Vec<(String, String)>,
    /// Name the service declares, when it does.
    pub filename: Option<String>,
    /// Digest the service publishes, ready for `checksum`.
    pub sha256: Option<String>,
}

/// Outcome of asking the registry about a URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SiteOutcome {
    /// The URL is not for a supported service; the engine keeps its normal path.
    Unsupported,
    /// The real file URL.
    Resolved(Resolved),
    /// The service answered something this build does not understand.
    Changed {
        /// Service that failed.
        service: &'static str,
        /// What was missing or unexpected, in one sentence.
        reason: String,
    },
}

impl SiteOutcome {
    /// True when the caller should keep the original URL instead of this answer.
    pub fn is_unsupported(&self) -> bool {
        matches!(self, SiteOutcome::Unsupported)
    }
}

/// How an extractor reads a page. Production uses HTTP; tests use a canned answer, which
/// keeps the host matching real while nothing leaves the machine.
#[async_trait::async_trait]
pub trait Fetcher: Send + Sync {
    /// GET `url` and return its status and body.
    async fn page(&self, url: &Url) -> Result<(u16, String), SiteError>;

    /// GET `url` with extra headers, used where a service wants an account token.
    ///
    /// The default ignores the headers so a test fetcher only implements what it needs.
    async fn page_with(
        &self,
        url: &Url,
        _headers: &[(String, String)],
    ) -> Result<(u16, String), SiteError> {
        self.page(url).await
    }
}

/// HTTP fetcher shared by every extractor: same network rules as the engine.
pub struct HttpFetcher {
    service: &'static str,
    client: reqwest::Client,
    /// Whether a private target is acceptable (the user's local-downloads switch).
    allow_local: bool,
}

impl HttpFetcher {
    /// Build it from the engine's HTTP configuration.
    pub fn new(http: &HttpConfig) -> Result<HttpFetcher, SiteError> {
        // Redirects are followed by hand: each hop goes through `urlssrf` first. Letting
        // reqwest follow them would let a hoster page point this fetch at loopback, which
        // is exactly the guard the engine applies to every other request.
        let mut builder = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(http.connect_timeout)
            .timeout(http.read_timeout)
            .user_agent(http.user_agent.clone());
        if let Some(proxy) = &http.proxy {
            builder =
                builder.proxy(reqwest::Proxy::all(proxy).map_err(|err| SiteError::Config {
                    service: "proxy",
                    detail: err.to_string(),
                })?);
        }
        Ok(HttpFetcher {
            service: "http",
            allow_local: http.allow_local,
            client: builder.build().map_err(|err| SiteError::Network {
                service: "client",
                source: err,
            })?,
        })
    }
}

/// How many redirects a page may cost before the extractor gives up.
const MAX_HOPS: usize = 5;

#[async_trait::async_trait]
impl Fetcher for HttpFetcher {
    async fn page(&self, url: &Url) -> Result<(u16, String), SiteError> {
        self.page_with(url, &[]).await
    }

    async fn page_with(
        &self,
        url: &Url,
        headers: &[(String, String)],
    ) -> Result<(u16, String), SiteError> {
        let mut current = url.clone();
        for _ in 0..=MAX_HOPS {
            // The guard runs before the socket, on every hop: a page that answers 302 to
            // `http://127.0.0.1:9/` never gets that request made.
            crate::urlssrf::resolve_checked(&current, self.allow_local)
                .await
                .map_err(|err| SiteError::Config {
                    service: self.service,
                    detail: err.to_string(),
                })?;
            let mut request = self.client.get(current.clone());
            for (name, value) in headers {
                request = request.header(name, value);
            }
            let response = request.send().await.map_err(|source| SiteError::Network {
                service: self.service,
                source,
            })?;
            let status = response.status();
            if status.is_redirection() {
                let location = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_string);
                match location.as_deref().map(|next| current.join(next)) {
                    Some(Ok(target)) => {
                        current = target;
                        continue;
                    }
                    _ => return Ok((status.as_u16(), String::new())),
                }
            }
            let code = status.as_u16();
            let body = response.text().await.map_err(|source| SiteError::Network {
                service: self.service,
                source,
            })?;
            return Ok((code, body));
        }
        Ok((508, String::new()))
    }
}

/// One service: how to recognize its links and how to read its answer.
#[async_trait::async_trait]
pub trait Extractor: Send + Sync {
    /// Service name used in errors and logs.
    fn name(&self) -> &'static str;

    /// Does this link belong here?
    fn matches(&self, url: &Url) -> bool;

    /// Turn the page into the file URL.
    async fn resolve(&self, fetcher: &dyn Fetcher, url: &Url) -> Result<SiteOutcome, SiteError>;
}

/// Why an extractor could not answer.
#[derive(Debug, thiserror::Error)]
pub enum SiteError {
    /// The request could not be made.
    #[error("cannot reach {service}: {source}")]
    Network {
        /// Service being read.
        service: &'static str,
        /// Transport error.
        #[source]
        source: reqwest::Error,
    },
    /// The client itself could not be built (a bad proxy, for instance).
    #[error("{service} is unusable: {detail}")]
    Config {
        /// Component that refused the configuration.
        service: &'static str,
        /// What was wrong.
        detail: String,
    },
    /// A non-success status on the page itself.
    #[error("{service} answered HTTP {status}")]
    Status {
        /// Service being read.
        service: &'static str,
        /// Status code returned by the page.
        status: u16,
    },
}

/// Every extractor, in the order they are asked.
fn registry() -> Vec<Box<dyn Extractor>> {
    vec![
        Box::new(mediafire::MediaFire),
        Box::new(google_drive::GoogleDrive),
        Box::new(rapidgator::Rapidgator),
        Box::new(gofile::Gofile),
        Box::new(internet_archive::InternetArchive),
    ]
}

/// Resolve `url` with the extractor that claims it.
///
/// An unclaimed URL is [`SiteOutcome::Unsupported`], which is a normal answer: the engine
/// downloads it as before.
pub async fn resolve(http: &HttpConfig, url: &Url) -> Result<SiteOutcome, SiteError> {
    let fetcher = HttpFetcher::new(http)?;
    resolve_with(&fetcher, url).await
}

/// Same as [`resolve`] with the page reader supplied by the caller.
pub async fn resolve_with(fetcher: &dyn Fetcher, url: &Url) -> Result<SiteOutcome, SiteError> {
    let Some(extractor) = registry().into_iter().find(|item| item.matches(url)) else {
        return Ok(SiteOutcome::Unsupported);
    };
    let outcome = extractor.resolve(fetcher, url).await?;
    Ok(check_result(outcome))
}

/// Accept only what the engine can download, and only from a real service answer.
fn check_result(outcome: SiteOutcome) -> SiteOutcome {
    match outcome {
        SiteOutcome::Resolved(resolved) => match resolved.url.scheme() {
            "http" | "https" => SiteOutcome::Resolved(resolved),
            other => SiteOutcome::Changed {
                service: "resolved",
                reason: format!("the page pointed at a `{other}` URL"),
            },
        },
        other => other,
    }
}

/// Percent-encode one path segment, keeping the characters a filename may carry.
///
/// `NON_ALPHANUMERIC` would turn every dot into `%2E`, which is valid but unreadable and
/// breaks servers that compare the literal name. Only the separators that would change the
/// path are escaped.
pub(crate) fn encode_segment(value: &str) -> String {
    const SAFE: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
        .remove(b'.')
        .remove(b'-')
        .remove(b'_')
        .remove(b'~');
    percent_encoding::utf8_percent_encode(value, SAFE).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_http_results_leave_the_registry() {
        let file = Resolved {
            url: Url::parse("file:///etc/passwd").unwrap(),
            headers: Vec::new(),
            filename: None,
            sha256: None,
        };
        assert!(matches!(
            check_result(SiteOutcome::Resolved(file)),
            SiteOutcome::Changed { .. }
        ));
        let ok = Resolved {
            url: Url::parse("https://cdn.example/file.bin").unwrap(),
            headers: Vec::new(),
            filename: None,
            sha256: None,
        };
        assert!(matches!(
            check_result(SiteOutcome::Resolved(ok)),
            SiteOutcome::Resolved(_)
        ));
    }
}
