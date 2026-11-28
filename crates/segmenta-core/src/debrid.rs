//! Debrid adapters: turn a hoster link into an unrestricted one.
//!
//! Contract: T4 of `odd/tasks/segmenta-v0.5.0-ecosistema.md`.
//!
//! # Shape
//!
//! An adapter only *describes* the request and *parses* the answer: [`resolve_request`]
//! builds the method, URL, headers and body, and [`parse_resolution`] reads the provider
//! payload. Executing the request belongs to the daemon, which owns the HTTP client, the
//! timeouts and the credential store. That keeps the whole contract testable without a
//! network and without a token in any fixture.
//!
//! # Secrets
//!
//! The token never appears in an error, in a `Debug` render or in a returned link. Errors
//! carry a code and a provider name only; the request struct is not `Debug`-printed by the
//! daemon, and [`DebridError::message`] is a fixed sentence per case.
//!
//! # Multi-step providers
//!
//! Real-Debrid and AllDebrid answer an unrestrict call with the link in one round trip.
//! TorBox does not: it creates a *web download* first and then asks for that download's
//! link, so [`resolve_with`] drives the whole exchange while the pure builders below keep
//! describing one request at a time.
//!
//! A provider whose flow is unknown still reports [`DebridError::Unsupported`] instead of
//! being guessed: a wrong endpoint wastes the user's quota and looks like a bug in the
//! service.

use std::fmt;

/// A debrid provider the daemon knows about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    /// Real-Debrid.
    RealDebrid,
    /// AllDebrid.
    AllDebrid,
    /// TorBox: a web download is created and then its link is requested.
    TorBox,
    /// Premiumize: one `directdl` call with the key in the header.
    Premiumize,
}

impl Provider {
    /// Parse the value of the `debrid_provider` setting.
    pub fn parse(value: &str) -> Option<Provider> {
        match value.trim().to_ascii_lowercase().as_str() {
            "realdebrid" | "real-debrid" | "real_debrid" => Some(Provider::RealDebrid),
            "alldebrid" | "all-debrid" | "all_debrid" => Some(Provider::AllDebrid),
            "torbox" | "tor-box" => Some(Provider::TorBox),
            "premiumize" => Some(Provider::Premiumize),
            _ => None,
        }
    }

    /// Stable name used by the settings, the IPC and the errors.
    pub fn as_str(self) -> &'static str {
        match self {
            Provider::RealDebrid => "realdebrid",
            Provider::AllDebrid => "alldebrid",
            Provider::TorBox => "torbox",
            Provider::Premiumize => "premiumize",
        }
    }
}

/// One HTTP request an adapter wants the daemon to perform.
#[derive(Clone, PartialEq, Eq)]
pub struct DebridRequest {
    /// Always `POST` today; kept explicit so an adapter can change it without a new type.
    pub method: &'static str,
    /// Absolute URL, `https` only.
    pub url: String,
    /// Headers to send. The token lives here and nowhere else.
    pub headers: Vec<(String, String)>,
    /// `application/x-www-form-urlencoded` body.
    pub body: String,
    /// The URL carries a credential in its query. Some providers design their API that way;
    /// the flag exists so the `Debug` render redacts it instead of trusting every caller.
    pub credential_in_url: bool,
}

impl DebridRequest {
    /// URL safe to print: the query disappears when it carries a credential.
    fn render_url(&self) -> String {
        if !self.credential_in_url {
            return self.url.clone();
        }
        match self.url.split_once('?') {
            Some((base, _)) => format!("{base}?<redacted>"),
            None => self.url.clone(),
        }
    }
}

impl fmt::Debug for DebridRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Never print the header values: one of them is the token.
        formatter
            .debug_struct("DebridRequest")
            .field("method", &self.method)
            .field("url", &self.render_url())
            .field("header_count", &self.headers.len())
            .finish()
    }
}

/// What went wrong, without the token and without the provider payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DebridError {
    /// The provider is recognized but this operation is not implemented for it.
    Unsupported(&'static str),
    /// The token is missing or the provider rejected it.
    Unauthorized,
    /// The link cannot be unrestrained (dead host, unsupported hoster, removed file).
    LinkUnavailable,
    /// The account ran out of traffic or hit a rate limit.
    QuotaExceeded,
    /// The provider answered with an unexpected shape.
    Malformed,
    /// The provider could not be reached at all.
    Unreachable,
}

impl DebridError {
    /// Stable code the daemon can map to a task error.
    pub fn code(&self) -> &'static str {
        match self {
            DebridError::Unsupported(_) => "ERR_DEBRID_UNSUPPORTED",
            DebridError::Unauthorized => "ERR_DEBRID_UNAUTHORIZED",
            DebridError::LinkUnavailable => "ERR_DEBRID_LINK_UNAVAILABLE",
            DebridError::QuotaExceeded => "ERR_DEBRID_QUOTA",
            DebridError::Malformed => "ERR_DEBRID_BAD_RESPONSE",
            DebridError::Unreachable => "ERR_DEBRID_UNREACHABLE",
        }
    }

    /// Fixed sentence for the task message; never includes a token or a payload.
    pub fn message(&self) -> String {
        match self {
            DebridError::Unsupported(provider) => {
                format!("{provider} cannot resolve hoster links yet")
            }
            DebridError::Unauthorized => "the provider rejected the token".to_string(),
            DebridError::LinkUnavailable => "the provider cannot unrestrict this link".to_string(),
            DebridError::QuotaExceeded => {
                "the provider quota or rate limit was reached".to_string()
            }
            DebridError::Malformed => "the provider answered with an unexpected shape".to_string(),
            DebridError::Unreachable => "the provider could not be reached".to_string(),
        }
    }
}

/// Build the request that resolves `link` with `token`.
///
/// Rejects anything that is not an `http`/`https` link before a token is attached, so a
/// malformed input cannot leak the credential to an unrelated host.
pub fn resolve_request(
    provider: Provider,
    token: &str,
    link: &str,
) -> Result<DebridRequest, DebridError> {
    if token.trim().is_empty() {
        return Err(DebridError::Unauthorized);
    }
    if !is_http_link(link) {
        return Err(DebridError::LinkUnavailable);
    }
    let token = token.trim();
    match provider {
        Provider::RealDebrid => Ok(DebridRequest {
            method: "POST",
            url: "https://api.real-debrid.com/rest/1.0/unrestrict/link".to_string(),
            headers: vec![
                ("Authorization".to_string(), format!("Bearer {token}")),
                (
                    "Content-Type".to_string(),
                    "application/x-www-form-urlencoded".to_string(),
                ),
            ],
            body: format!("link={}", encode(link)),
            credential_in_url: false,
        }),
        Provider::AllDebrid => Ok(DebridRequest {
            method: "POST",
            url: "https://api.alldebrid.com/v4/link/unlock".to_string(),
            headers: vec![
                ("Authorization".to_string(), format!("Bearer {token}")),
                (
                    "Content-Type".to_string(),
                    "application/x-www-form-urlencoded".to_string(),
                ),
            ],
            body: format!("link={}", encode(link)),
            credential_in_url: false,
        }),
        Provider::Premiumize => Ok(DebridRequest {
            method: "POST",
            url: "https://www.premiumize.me/api/transfer/directdl".to_string(),
            headers: vec![
                ("Authorization".to_string(), format!("Bearer {token}")),
                (
                    "Content-Type".to_string(),
                    "application/x-www-form-urlencoded".to_string(),
                ),
            ],
            body: format!("src={}", encode(link)),
            credential_in_url: false,
        }),
        Provider::TorBox => Ok(DebridRequest {
            method: "POST",
            url: "https://api.torbox.app/v1/api/webdl/createwebdownload".to_string(),
            headers: vec![
                ("Authorization".to_string(), format!("Bearer {token}")),
                (
                    "Content-Type".to_string(),
                    "application/x-www-form-urlencoded".to_string(),
                ),
            ],
            body: format!("link={}", encode(link)),
            credential_in_url: false,
        }),
    }
}

/// Read the unrestricted link out of `body`, which the provider returned with `status`.
pub fn parse_resolution(
    provider: Provider,
    status: u16,
    body: &str,
) -> Result<String, DebridError> {
    match status {
        401 | 403 => return Err(DebridError::Unauthorized),
        404 | 410 => return Err(DebridError::LinkUnavailable),
        429 => return Err(DebridError::QuotaExceeded),
        500..=599 => return Err(DebridError::Unreachable),
        _ => {}
    }
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|_| DebridError::Malformed)?;
    let link = match provider {
        Provider::RealDebrid => {
            if let Some(code) = value.get("error_code").and_then(|code| code.as_u64()) {
                return Err(map_real_debrid(code));
            }
            value
                .get("download")
                .and_then(|link| link.as_str())
                .map(str::to_string)
        }
        Provider::AllDebrid => {
            let status = value.get("status").and_then(|s| s.as_str()).unwrap_or("");
            if status != "success" {
                return Err(map_all_debrid(&value));
            }
            value
                .get("data")
                .and_then(|data| data.get("link"))
                .and_then(|link| link.as_str())
                .map(str::to_string)
        }
        Provider::Premiumize => {
            if value.get("status").and_then(|s| s.as_str()) != Some("success") {
                return Err(DebridError::LinkUnavailable);
            }
            value
                .get("content")
                .and_then(|content| content.as_array())
                .and_then(|list| list.first())
                .and_then(|first| first.get("link"))
                .and_then(|link| link.as_str())
                .map(str::to_string)
        }
        // TorBox needs a second call; a single answer has no link to return.
        Provider::TorBox => return Err(DebridError::Malformed),
    };
    // Two different failures, deliberately kept apart: a provider that answers successfully
    // without a link is telling us the hoster is unavailable (the user can try another one);
    // a provider that hands back an insecure or credential-carrying URL broke the contract,
    // and retrying the same call would not help.
    let link = link.ok_or(DebridError::LinkUnavailable)?;
    sanitize_link(&link).ok_or(DebridError::Malformed)
}

/// Anything that can perform a [`DebridRequest`]: the daemon's HTTP client in production,
/// a canned answer in tests.
#[async_trait::async_trait]
pub trait DebridHttp: Send + Sync {
    /// Perform the request and return the status and body.
    async fn send(&self, request: &DebridRequest) -> Result<(u16, String), DebridError>;
}

/// Resolve `link` with `token`, driving every step the provider needs.
///
/// Real-Debrid and AllDebrid are one call. TorBox is two: create the web download, then ask
/// for its file link. The token always travels in a header, never in a URL, so a link that
/// ends up in a log cannot carry the credential with it.
pub async fn resolve_with(
    http: &dyn DebridHttp,
    provider: Provider,
    token: &str,
    link: &str,
) -> Result<String, DebridError> {
    let first = resolve_request(provider, token, link)?;
    let (status, body) = http.send(&first).await?;
    match provider {
        Provider::RealDebrid | Provider::AllDebrid | Provider::Premiumize => {
            parse_resolution(provider, status, &body)
        }
        Provider::TorBox => {
            let id = torbox_download_id(status, &body)?;
            let request = torbox_link_request(token, id)?;
            let (status, body) = http.send(&request).await?;
            torbox_link(status, &body)
        }
    }
}

/// Read the id of the web download TorBox just created.
fn torbox_download_id(status: u16, body: &str) -> Result<u64, DebridError> {
    match status {
        401 | 403 => return Err(DebridError::Unauthorized),
        429 => return Err(DebridError::QuotaExceeded),
        500..=599 => return Err(DebridError::Unreachable),
        _ => {}
    }
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|_| DebridError::Malformed)?;
    if value.get("success").and_then(|flag| flag.as_bool()) == Some(false) {
        // The provider accepts the shape and refuses the link: that is availability, not a
        // malformed answer, and the user can act on it.
        return Err(DebridError::LinkUnavailable);
    }
    value
        .get("data")
        .and_then(|data| {
            data.get("webdownload_id")
                .or_else(|| data.get("web_id"))
                .or_else(|| data.get("id"))
        })
        .and_then(serde_json::Value::as_u64)
        .ok_or(DebridError::Malformed)
}

/// Second TorBox step: ask for the link of the created download.
fn torbox_link_request(token: &str, id: u64) -> Result<DebridRequest, DebridError> {
    if token.trim().is_empty() {
        return Err(DebridError::Unauthorized);
    }
    Ok(DebridRequest {
        method: "GET",
        url: format!("https://api.torbox.app/v1/api/webdl/requestdl?web_id={id}"),
        headers: vec![(
            "Authorization".to_string(),
            format!("Bearer {}", token.trim()),
        )],
        body: String::new(),
        credential_in_url: false,
    })
}

/// Read the link out of the TorBox answer.
fn torbox_link(status: u16, body: &str) -> Result<String, DebridError> {
    match status {
        401 | 403 => return Err(DebridError::Unauthorized),
        404 | 410 => return Err(DebridError::LinkUnavailable),
        429 => return Err(DebridError::QuotaExceeded),
        500..=599 => return Err(DebridError::Unreachable),
        _ => {}
    }
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|_| DebridError::Malformed)?;
    let link = value
        .get("data")
        .and_then(|data| match data {
            serde_json::Value::String(link) => Some(link.clone()),
            other => other
                .get("url")
                .and_then(|url| url.as_str())
                .map(str::to_string),
        })
        .ok_or(DebridError::Malformed)?;
    sanitize_link(&link).ok_or(DebridError::Malformed)
}

/// What the provider says about the account behind a token.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AccountInfo {
    /// Name the provider shows, when it has one.
    pub username: Option<String>,
    /// Whether the account can unrestrict today.
    pub is_premium: bool,
    /// Expiry as the provider reports it, ISO-8601 or epoch seconds as text.
    pub expires_at: Option<String>,
}

/// Build the request that reads the account behind `token`.
pub fn account_request(provider: Provider, token: &str) -> Result<DebridRequest, DebridError> {
    if token.trim().is_empty() {
        return Err(DebridError::Unauthorized);
    }
    let auth = format!("Bearer {}", token.trim());
    let url = match provider {
        Provider::RealDebrid => "https://api.real-debrid.com/rest/1.0/user",
        Provider::AllDebrid => "https://api.alldebrid.com/v4/user",
        Provider::TorBox => "https://api.torbox.app/v1/api/user/me",
        Provider::Premiumize => "https://www.premiumize.me/api/account/info",
    };
    Ok(DebridRequest {
        method: "GET",
        url: url.to_string(),
        headers: vec![("Authorization".to_string(), auth)],
        body: String::new(),
        credential_in_url: false,
    })
}

/// Read account metadata out of the provider answer.
pub fn parse_account(
    provider: Provider,
    status: u16,
    body: &str,
) -> Result<AccountInfo, DebridError> {
    match status {
        401 | 403 => return Err(DebridError::Unauthorized),
        429 => return Err(DebridError::QuotaExceeded),
        500..=599 => return Err(DebridError::Unreachable),
        _ => {}
    }
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|_| DebridError::Malformed)?;
    let account = match provider {
        Provider::RealDebrid => AccountInfo {
            username: value
                .get("username")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            is_premium: value.get("type").and_then(|v| v.as_str()) == Some("premium"),
            expires_at: value
                .get("expiration")
                .and_then(|v| v.as_str())
                .map(str::to_string),
        },
        Provider::AllDebrid => {
            let user = value.get("user").ok_or(DebridError::Malformed)?;
            AccountInfo {
                username: user
                    .get("username")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                is_premium: user
                    .get("isPremium")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                expires_at: user
                    .get("premiumUntil")
                    .and_then(|v| v.as_u64())
                    .map(|epoch| epoch.to_string()),
            }
        }
        Provider::Premiumize => {
            if value.get("status").and_then(|s| s.as_str()) != Some("success") {
                return Err(DebridError::Unauthorized);
            }
            let until = value
                .get("premium_until")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            AccountInfo {
                username: value
                    .get("customer_id")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                // A zero epoch means "never subscribed", not "expired long ago".
                is_premium: until > 0,
                expires_at: (until > 0).then(|| until.to_string()),
            }
        }
        Provider::TorBox => {
            if value.get("success").and_then(|flag| flag.as_bool()) == Some(false) {
                return Err(DebridError::Unauthorized);
            }
            let data = value.get("data").ok_or(DebridError::Malformed)?;
            AccountInfo {
                username: data
                    .get("email")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                // Plan 0 is free; every paid plan is above zero.
                is_premium: data
                    .get("plan")
                    .and_then(|v| v.as_u64())
                    .map(|plan| plan > 0)
                    .unwrap_or(false),
                expires_at: data
                    .get("premium_expires_at")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
            }
        }
    };
    Ok(account)
}

/// Real-Debrid documents its failures as numeric codes.
fn map_real_debrid(code: u64) -> DebridError {
    match code {
        // 8: bad token, 9: permission denied, 15: account locked.
        8 | 9 | 15 => DebridError::Unauthorized,
        // 16: unavailable hoster, 19: file unavailable, 20: unsupported hoster, 21: no server.
        16 | 19 | 20 | 21 | 22 => DebridError::LinkUnavailable,
        // 4: too many requests, 24: traffic exhausted, 34: too many requests.
        4 | 24 | 34 => DebridError::QuotaExceeded,
        _ => DebridError::Malformed,
    }
}

/// AllDebrid nests an error object under `error`.
fn map_all_debrid(value: &serde_json::Value) -> DebridError {
    let code = value
        .get("error")
        .and_then(|error| error.get("code"))
        .and_then(|code| code.as_str())
        .unwrap_or("");
    match code {
        "AUTH_BAD_APIKEY" | "AUTH_USER_BANNED" | "AUTH_MISSING_APIKEY" => DebridError::Unauthorized,
        "LINK_DOWN" | "LINK_HOST_NOT_SUPPORTED" | "LINK_HOST_UNAVAILABLE" | "NO_SERVER" => {
            DebridError::LinkUnavailable
        }
        "MUST_WAIT_BEFORE_LINK_CONVERSION" | "LINK_TOO_MANY_DOWNLOADS" => {
            DebridError::QuotaExceeded
        }
        _ => DebridError::Malformed,
    }
}

/// Accept only a plain HTTPS URL without credentials or whitespace.
fn sanitize_link(link: &str) -> Option<String> {
    let link = link.trim();
    if link.is_empty() || link.contains(char::is_whitespace) {
        return None;
    }
    let rest = link.strip_prefix("https://")?;
    if rest.is_empty() || rest.starts_with('/') || rest.contains('@') {
        return None;
    }
    Some(link.to_string())
}

fn is_http_link(link: &str) -> bool {
    let link = link.trim();
    link.len() <= 4096 && (link.starts_with("http://") || link.starts_with("https://"))
}

/// Percent-encode one form value.
fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_debrid_builds_the_documented_request() {
        let request =
            resolve_request(Provider::RealDebrid, "tok-123", "https://mega.nz/file/abc").unwrap();
        assert_eq!(request.method, "POST");
        assert_eq!(
            request.url,
            "https://api.real-debrid.com/rest/1.0/unrestrict/link"
        );
        assert_eq!(
            request.headers[0],
            ("Authorization".to_string(), "Bearer tok-123".to_string())
        );
        assert_eq!(request.body, "link=https%3A%2F%2Fmega.nz%2Ffile%2Fabc");
        // The Debug render must not leak the credential.
        assert!(!format!("{request:?}").contains("tok-123"));
    }

    #[test]
    fn parses_both_providers_and_maps_their_errors() {
        let rd = parse_resolution(
            Provider::RealDebrid,
            200,
            r#"{"download":"https://dl.real-debrid.com/file","filename":"f.bin"}"#,
        )
        .unwrap();
        assert_eq!(rd, "https://dl.real-debrid.com/file");
        assert_eq!(
            parse_resolution(Provider::RealDebrid, 200, r#"{"error_code":24}"#).unwrap_err(),
            DebridError::QuotaExceeded
        );
        assert_eq!(
            parse_resolution(Provider::RealDebrid, 401, "{}").unwrap_err(),
            DebridError::Unauthorized
        );

        let ad = parse_resolution(
            Provider::AllDebrid,
            200,
            r#"{"status":"success","data":{"link":"https://dl.alldebrid.com/x"}}"#,
        )
        .unwrap();
        assert_eq!(ad, "https://dl.alldebrid.com/x");
        assert_eq!(
            parse_resolution(
                Provider::AllDebrid,
                200,
                r#"{"status":"error","error":{"code":"LINK_DOWN"}}"#
            )
            .unwrap_err(),
            DebridError::LinkUnavailable
        );

        // A link with credentials or plain HTTP is never handed back.
        assert_eq!(
            parse_resolution(Provider::RealDebrid, 200, r#"{"download":"http://x/y"}"#)
                .unwrap_err(),
            DebridError::Malformed
        );
        assert_eq!(
            parse_resolution(
                Provider::RealDebrid,
                200,
                r#"{"download":"https://user:pw@x/y"}"#
            )
            .unwrap_err(),
            DebridError::Malformed
        );
        assert_eq!(
            parse_resolution(Provider::RealDebrid, 200, "not json").unwrap_err(),
            DebridError::Malformed
        );
    }

    #[test]
    fn torbox_starts_a_web_download_instead_of_unrestricting() {
        let request = resolve_request(Provider::TorBox, "tok", "https://x/file").unwrap();
        assert!(request.url.ends_with("/v1/api/webdl/createwebdownload"));
        assert!(request.body.contains("link="));
        // A single answer cannot carry the link: TorBox needs the second call.
        assert_eq!(
            parse_resolution(Provider::TorBox, 200, "{}").unwrap_err(),
            DebridError::Malformed
        );
        assert_eq!(Provider::parse("torbox"), Some(Provider::TorBox));
        assert_eq!(Provider::parse("nope"), None);
    }

    #[test]
    fn a_missing_token_or_foreign_link_never_builds_a_request() {
        assert_eq!(
            resolve_request(Provider::RealDebrid, "  ", "https://x/file").unwrap_err(),
            DebridError::Unauthorized
        );
        assert_eq!(
            resolve_request(Provider::RealDebrid, "tok", "ftp://x/file").unwrap_err(),
            DebridError::LinkUnavailable
        );
        // Errors carry a fixed code and a sentence, never the token.
        let error = resolve_request(Provider::AllDebrid, "tok", "file:///etc/passwd").unwrap_err();
        assert_eq!(error.code(), "ERR_DEBRID_LINK_UNAVAILABLE");
        assert!(!error.message().contains("tok"));
    }
}
