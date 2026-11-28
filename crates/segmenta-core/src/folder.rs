//! Folder spidering: an `Index of /` page (HTTP) becomes a bounded list of files.
//!
//! Contract: S3 of `odd/tasks/segmenta-hito2-captura-hosters-remoto.md` (T6).
//!
//! # Why the limits are part of the result
//!
//! A crawler without bounds is an accidental scraper. Every stop is recorded in the last
//! entry, so the caller can tell the user *why* the list ends instead of presenting a
//! truncated list as if it were complete.
//!
//! # What is never followed
//!
//! * another host, even when the page links to it;
//! * `../` and anything that resolves above the starting directory;
//! * anything whose scheme is not `http`/`https`.
//!
//! The target of every request still goes through the same guards as the rest of the
//! engine: this module only decides *what* to ask for, never bypasses `urlssrf`.

use std::collections::{HashSet, VecDeque};

use url::Url;

use crate::sites::Fetcher;

/// Bounds of one crawl.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FolderLimits {
    /// How many directory levels below the start are visited.
    pub max_depth: usize,
    /// How many directory pages are read.
    pub max_folders: usize,
    /// How many files are collected.
    pub max_files: usize,
}

impl Default for FolderLimits {
    fn default() -> Self {
        FolderLimits {
            max_depth: 3,
            max_folders: 50,
            max_files: 2000,
        }
    }
}

/// Why a crawl ended early.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderStop {
    /// The depth bound was reached.
    Depth,
    /// The folder bound was reached.
    Folders,
    /// The file bound was reached.
    Files,
}

/// One file found in a listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderEntry {
    /// File URL.
    pub url: Url,
    /// Name as the listing shows it.
    pub name: String,
    /// Size when the listing states it (FTP-style listings do).
    pub size: Option<u64>,
    /// Why the crawl stopped here, when it did.
    pub stop: Option<FolderStop>,
}

/// Why a crawl could not run at all.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FolderError {
    /// The starting URL is not a directory.
    #[error("`{url}` is not a directory to crawl")]
    NotADirectory {
        /// The URL as given.
        url: String,
    },
    /// A page could not be read.
    #[error("cannot read {url}: {reason}")]
    Fetch {
        /// Page that failed.
        url: String,
        /// Transport reason.
        reason: String,
    },
}

/// Crawl `start` breadth-first, stopping at the first limit that trips.
pub async fn crawl(
    start: &Url,
    limits: FolderLimits,
    fetcher: &dyn Fetcher,
) -> Result<Vec<FolderEntry>, FolderError> {
    if !matches!(start.scheme(), "http" | "https") {
        return Err(FolderError::NotADirectory {
            url: start.to_string(),
        });
    }
    let origin = start.host_str().map(str::to_string);
    let base_path = directory_of(start);
    let mut found: Vec<FolderEntry> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<(Url, usize)> = VecDeque::new();
    queue.push_back((start.clone(), 0));
    seen.insert(start.to_string());
    let mut folders = 0usize;
    // A stop is remembered until the end: entries keep being collected after a folder is
    // skipped for depth, and the flag belongs to whatever the list ends with.
    let mut truncated: Option<FolderStop> = None;
    while let Some((page, depth)) = queue.pop_front() {
        folders += 1;
        if folders > limits.max_folders {
            return Ok(truncated_with(found, FolderStop::Folders));
        }
        let (status, body) = fetcher
            .page(&page)
            .await
            .map_err(|err| FolderError::Fetch {
                url: page.to_string(),
                reason: err.to_string(),
            })?;
        if !(200..300).contains(&status) {
            continue;
        }
        for link in links(&body) {
            let Ok(target) = page.join(&link) else {
                continue;
            };
            if !matches!(target.scheme(), "http" | "https") {
                continue;
            }
            // The listing must stay on the host the user pointed at: a link to a CDN or to
            // an unrelated site is not part of this folder.
            if target.host_str().map(str::to_string) != origin {
                continue;
            }
            if !target.path().starts_with(&base_path) {
                continue;
            }
            if !seen.insert(target.to_string()) {
                continue;
            }
            if is_directory(&target) {
                if depth >= limits.max_depth {
                    truncated = Some(FolderStop::Depth);
                    continue;
                }
                queue.push_back((target, depth + 1));
                continue;
            }
            found.push(FolderEntry {
                name: name_of(&target),
                size: None,
                stop: None,
                url: target,
            });
            if found.len() >= limits.max_files {
                return Ok(truncated_with(found, FolderStop::Files));
            }
        }
    }
    Ok(match truncated {
        Some(stop) => truncated_with(found, stop),
        None => found,
    })
}

/// Mark the list as ending because of `stop`.
fn truncated_with(mut found: Vec<FolderEntry>, stop: FolderStop) -> Vec<FolderEntry> {
    if let Some(last) = found.last_mut() {
        last.stop = Some(stop);
    }
    found
}

/// Directory part of a URL, always ending in `/`.
fn directory_of(url: &Url) -> String {
    let path = url.path();
    match path.rfind('/') {
        Some(0) | None => "/".to_string(),
        Some(index) => path[..=index].to_string(),
    }
}

/// A link that names a directory.
fn is_directory(url: &Url) -> bool {
    url.path().ends_with('/')
}

fn name_of(url: &Url) -> String {
    url.path_segments()
        .and_then(|mut parts| parts.next_back().map(str::to_string))
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| url.path().to_string())
}

/// Every `href` in the page, in order, with the entities a listing actually uses.
fn links(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = body;
    while let Some(index) = rest.find("href=\"") {
        let after = &rest[index + 6..];
        let Some(end) = after.find('"') else {
            break;
        };
        let raw = &after[..end];
        rest = &after[end..];
        let decoded = raw
            .replace("&amp;", "&")
            .replace("&quot;", "\"")
            .replace("&#39;", "'");
        if decoded.trim().is_empty() {
            continue;
        }
        // Parents and self-links are never part of a folder.
        if decoded.starts_with("..") || decoded == "./" || decoded == "." {
            continue;
        }
        out.push(decoded);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_directory_of_a_url_always_ends_in_a_slash() {
        assert_eq!(
            directory_of(&Url::parse("https://x.test/a/b/").unwrap()),
            "/a/b/"
        );
        assert_eq!(
            directory_of(&Url::parse("https://x.test/a/b").unwrap()),
            "/a/"
        );
        assert_eq!(directory_of(&Url::parse("https://x.test/").unwrap()), "/");
    }

    #[test]
    fn parent_and_self_links_are_dropped_and_entities_decoded() {
        let page = r#"<a href="../">up</a><a href="a&amp;b.zip">f</a><a href="./">.</a><a href="https://x/y">y</a>"#;
        assert_eq!(
            links(page),
            vec!["a&b.zip".to_string(), "https://x/y".to_string()]
        );
    }
}
