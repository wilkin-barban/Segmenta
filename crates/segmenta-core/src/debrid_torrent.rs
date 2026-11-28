//! Remote torrent jobs: hand a magnet to the provider, follow it, get the files back.
//!
//! Contract: S3 of `odd/tasks/segmenta-hito3-debrid-archivos-torrents.md` (T5).
//!
//! # Why this is not one flow with a parameter
//!
//! Real-Debrid, TorBox and Premiumize model a remote torrent with different endpoints,
//! different selection rules and different cleanup calls. The shared part is only the shape
//! of the job ([`TorrentJob`]) and the vocabulary of its state ([`TorrentState`]); the rest
//! is three adapters, each with its own canonical answers in tests.
//!
//! # What this module does not do
//!
//! It never polls on its own and never downloads: it builds the requests and reads the
//! answers. The loop, the deadline and the hand-off to the HTTP engine belong to the daemon,
//! which owns the clock, the HTTP layer and the files.

use serde_json::Value;
use url::Url;

use crate::debrid::{encode, DebridError, DebridRequest, Provider};

/// Where a remote job stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TorrentState {
    /// Accepted by the provider, not started yet.
    Queued,
    /// Still transferring inside the provider's cloud.
    Downloading,
    /// Complete: the files can be fetched.
    Ready,
    /// The provider reported a failure.
    Failed,
    /// The provider dropped the job (expired, removed, out of quota).
    Expired,
}

impl TorrentState {
    /// Stable name for the UI and for logs.
    pub fn as_str(self) -> &'static str {
        match self {
            TorrentState::Queued => "queued",
            TorrentState::Downloading => "downloading",
            TorrentState::Ready => "ready",
            TorrentState::Failed => "failed",
            TorrentState::Expired => "expired",
        }
    }

    /// Terminal states do not need another poll.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            TorrentState::Ready | TorrentState::Failed | TorrentState::Expired
        )
    }
}

/// One file the provider will serve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TorrentFile {
    /// Path inside the torrent.
    pub path: String,
    /// Size in bytes.
    pub bytes: u64,
    /// Whether the provider selected it for download.
    pub selected: bool,
}

/// A remote job as the provider describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TorrentJob {
    /// Identifier the provider uses.
    pub id: String,
    /// Name the provider shows, when it has one.
    pub name: Option<String>,
    /// Progress in whole percent, as reported.
    pub progress_pct: u8,
    /// State derived from the provider's status string.
    pub state: TorrentState,
    /// Files the provider lists.
    pub files: Vec<TorrentFile>,
    /// Direct links, once the job is ready.
    pub links: Vec<String>,
}

/// Which provider owns a remote torrent flow.
fn supports_torrents(provider: Provider) -> Result<(), DebridError> {
    match provider {
        Provider::RealDebrid | Provider::TorBox | Provider::Premiumize => Ok(()),
        other => Err(DebridError::Unsupported(other.as_str())),
    }
}

/// Request that adds `magnet` to the provider's cloud.
pub fn add_request(
    provider: Provider,
    token: &str,
    magnet: &str,
) -> Result<DebridRequest, DebridError> {
    supports_torrents(provider)?;
    if token.trim().is_empty() {
        return Err(DebridError::Unauthorized);
    }
    if !magnet.starts_with("magnet:?") && !magnet.starts_with("https://") {
        return Err(DebridError::LinkUnavailable);
    }
    let auth = (
        "Authorization".to_string(),
        format!("Bearer {}", token.trim()),
    );
    let form = (
        "Content-Type".to_string(),
        "application/x-www-form-urlencoded".to_string(),
    );
    Ok(match provider {
        Provider::RealDebrid => DebridRequest {
            method: "POST",
            url: "https://api.real-debrid.com/rest/1.0/torrents/addMagnet".to_string(),
            headers: vec![auth, form],
            body: format!("magnet={}", encode(magnet)),
            credential_in_url: false,
        },
        Provider::TorBox => DebridRequest {
            method: "POST",
            url: "https://api.torbox.app/v1/api/torrents/createtorrent".to_string(),
            headers: vec![auth, form],
            // TorBox documents magnet on this endpoint; the file variant is multipart and is
            // not attempted here.
            body: format!("magnet={}", encode(magnet)),
            credential_in_url: false,
        },
        Provider::Premiumize => DebridRequest {
            method: "POST",
            url: "https://www.premiumize.me/api/transfer/create".to_string(),
            headers: vec![auth, form],
            body: format!("src={}", encode(magnet)),
            credential_in_url: false,
        },
        _ => unreachable!("checked above"),
    })
}

/// Read the id the provider assigned to a new job.
pub fn parse_add(provider: Provider, status: u16, body: &str) -> Result<String, DebridError> {
    supports_torrents(provider)?;
    ok_status(status)?;
    let value: Value = serde_json::from_str(body).map_err(|_| DebridError::Malformed)?;
    let id = match provider {
        Provider::RealDebrid => value.get("id").and_then(|v| v.as_str()).map(str::to_string),
        // TorBox wraps everything in `data`.
        Provider::TorBox => boxed_id(&value),
        Provider::Premiumize => value.get("id").and_then(|v| v.as_str()).map(str::to_string),
        _ => None,
    };
    id.ok_or(DebridError::Malformed)
}

/// TorBox nests the identifier under `data`, as a string or as a number.
///
/// The number is rendered into an owned string: this keeps a `Box::leak` out of a function
/// that runs on every poll.
fn boxed_id(value: &Value) -> Option<String> {
    let data = value.get("data")?;
    let candidate = data.get("torrent_id").or_else(|| data.get("id"))?;
    match candidate {
        Value::String(text) => Some(text.clone()),
        other => other.as_u64().map(|id| id.to_string()),
    }
}

/// Request that asks the provider how the job is doing.
pub fn info_request(
    provider: Provider,
    token: &str,
    id: &str,
) -> Result<DebridRequest, DebridError> {
    supports_torrents(provider)?;
    if token.trim().is_empty() {
        return Err(DebridError::Unauthorized);
    }
    let auth = (
        "Authorization".to_string(),
        format!("Bearer {}", token.trim()),
    );
    Ok(match provider {
        Provider::RealDebrid => DebridRequest {
            method: "GET",
            url: format!("https://api.real-debrid.com/rest/1.0/torrents/info/{id}"),
            headers: vec![auth],
            body: String::new(),
            credential_in_url: false,
        },
        Provider::TorBox => DebridRequest {
            method: "GET",
            url: format!("https://api.torbox.app/v1/api/torrents/mylist?id={id}"),
            headers: vec![auth],
            body: String::new(),
            credential_in_url: false,
        },
        Provider::Premiumize => DebridRequest {
            method: "GET",
            url: format!("https://www.premiumize.me/api/transfer/list?id={id}"),
            headers: vec![auth],
            body: String::new(),
            credential_in_url: false,
        },
        _ => unreachable!("checked above"),
    })
}

/// Read the job state out of an info answer.
pub fn parse_info(provider: Provider, status: u16, body: &str) -> Result<TorrentJob, DebridError> {
    supports_torrents(provider)?;
    ok_status(status)?;
    let value: Value = serde_json::from_str(body).map_err(|_| DebridError::Malformed)?;
    match provider {
        Provider::RealDebrid => real_debrid_job(&value),
        Provider::TorBox => torbox_job(&value),
        Provider::Premiumize => premiumize_job(&value),
        _ => Err(DebridError::Unsupported("no torrent flow")),
    }
}

/// Real-Debrid reports `status` as a phrase and `progress` as a number.
fn real_debrid_job(value: &Value) -> Result<TorrentJob, DebridError> {
    let status = value
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let state = match status {
        "magnet_conversion" | "waiting_files_selection" | "queued" => TorrentState::Queued,
        "downloading" | "compressing" | "uploading" => TorrentState::Downloading,
        "downloaded" => TorrentState::Ready,
        "error" | "virus" | "dead" => TorrentState::Failed,
        "magnet_error" => TorrentState::Expired,
        // An unknown status is not "malformed": the provider is speaking a dialect this
        // build does not know, and guessing one of the five states would be worse than
        // saying so.
        other => return Err(DebridError::UnknownState(other.to_string())),
    };
    Ok(TorrentJob {
        id: value
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        name: value
            .get("filename")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        progress_pct: value
            .get("progress")
            .and_then(Value::as_f64)
            .unwrap_or(0.0)
            .clamp(0.0, 100.0) as u8,
        state,
        files: value
            .get("files")
            .and_then(Value::as_array)
            .map(|files| {
                files
                    .iter()
                    .filter_map(|file| {
                        Some(TorrentFile {
                            path: file.get("path").and_then(|v| v.as_str())?.to_string(),
                            bytes: file.get("bytes").and_then(Value::as_u64).unwrap_or(0),
                            selected: file.get("selected").and_then(Value::as_u64).unwrap_or(1)
                                == 1,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default(),
        links: strings(value.get("links")),
    })
}

/// TorBox reports `download_state` and `progress` as a fraction.
fn torbox_job(value: &Value) -> Result<TorrentJob, DebridError> {
    let data = value
        .get("data")
        .and_then(|data| data.as_array().and_then(|list| list.first()).or(Some(data)))
        .ok_or(DebridError::Malformed)?;
    let status = data
        .get("download_state")
        .or_else(|| data.get("state"))
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let state = match status {
        "queued" | "checking" | "metadata" => TorrentState::Queued,
        "downloading" | "paused" => TorrentState::Downloading,
        "completed" | "uploading" | "seeding" => TorrentState::Ready,
        "error" | "failed" => TorrentState::Failed,
        "expired" => TorrentState::Expired,
        _ => TorrentState::Queued,
    };
    let progress = data.get("progress").and_then(Value::as_f64).unwrap_or(0.0);
    Ok(TorrentJob {
        id: data
            .get("id")
            .map(|value| match value {
                Value::String(text) => text.clone(),
                other => other.to_string(),
            })
            .unwrap_or_default(),
        name: data
            .get("name")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        // TorBox sends a fraction; Real-Debrid sends a percentage.
        progress_pct: (progress * 100.0).round().clamp(0.0, 100.0) as u8,
        state,
        files: data
            .get("files")
            .and_then(Value::as_array)
            .map(|files| {
                files
                    .iter()
                    .filter_map(|file| {
                        Some(TorrentFile {
                            path: file
                                .get("name")
                                .or_else(|| file.get("path"))?
                                .as_str()?
                                .to_string(),
                            bytes: file.get("size").and_then(Value::as_u64).unwrap_or(0),
                            selected: true,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default(),
        links: strings(data.get("links")),
    })
}

/// Premiumize lists transfers with `status` among `finished`/`running`.
fn premiumize_job(value: &Value) -> Result<TorrentJob, DebridError> {
    let list = value
        .get("transfers")
        .and_then(Value::as_array)
        .ok_or(DebridError::Malformed)?;
    let transfer = list.first().ok_or(DebridError::LinkUnavailable)?;
    let status = transfer
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let state = match status {
        "running" | "queued" => TorrentState::Downloading,
        "finished" => TorrentState::Ready,
        "deleted" => TorrentState::Expired,
        "error" | "timeout" => TorrentState::Failed,
        _ => TorrentState::Queued,
    };
    let progress = transfer
        .get("progress")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    Ok(TorrentJob {
        id: transfer
            .get("id")
            .map(|value| match value {
                Value::String(text) => text.clone(),
                other => other.to_string(),
            })
            .unwrap_or_default(),
        name: transfer
            .get("name")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        progress_pct: (progress * 100.0).round().clamp(0.0, 100.0) as u8,
        state,
        files: Vec::new(),
        links: strings(transfer.get("links")),
    })
}

/// Request that deletes the remote job once the files are home.
pub fn delete_request(
    provider: Provider,
    token: &str,
    id: &str,
) -> Result<DebridRequest, DebridError> {
    supports_torrents(provider)?;
    if token.trim().is_empty() {
        return Err(DebridError::Unauthorized);
    }
    let auth = (
        "Authorization".to_string(),
        format!("Bearer {}", token.trim()),
    );
    Ok(match provider {
        Provider::RealDebrid => DebridRequest {
            method: "DELETE",
            url: format!("https://api.real-debrid.com/rest/1.0/torrents/delete/{id}"),
            headers: vec![auth],
            body: String::new(),
            credential_in_url: false,
        },
        Provider::TorBox => DebridRequest {
            method: "POST",
            url: "https://api.torbox.app/v1/api/torrents/controltorrent".to_string(),
            headers: vec![
                auth,
                (
                    "Content-Type".to_string(),
                    "application/x-www-form-urlencoded".to_string(),
                ),
            ],
            body: format!("torrent_id={}&operation=delete", encode(id)),
            credential_in_url: false,
        },
        Provider::Premiumize => DebridRequest {
            method: "POST",
            url: "https://www.premiumize.me/api/transfer/delete".to_string(),
            headers: vec![
                auth,
                (
                    "Content-Type".to_string(),
                    "application/x-www-form-urlencoded".to_string(),
                ),
            ],
            body: format!("id={}", encode(id)),
            credential_in_url: false,
        },
        _ => unreachable!("checked above"),
    })
}

fn ok_status(status: u16) -> Result<(), DebridError> {
    match status {
        401 | 403 => Err(DebridError::Unauthorized),
        404 | 410 => Err(DebridError::LinkUnavailable),
        429 => Err(DebridError::QuotaExceeded),
        500..=599 => Err(DebridError::Unreachable),
        200..=299 => Ok(()),
        _ => Err(DebridError::Malformed),
    }
}

fn strings(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// Sanity check for a magnet the user pasted.
pub fn is_magnet(link: &str) -> bool {
    Url::parse(link)
        .map(|url| url.scheme() == "magnet")
        .unwrap_or(false)
}
