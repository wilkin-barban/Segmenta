//! Repositories that publish files: GitHub releases and, next, Hugging Face.
//!
//! A release page is a listing, not a download: the API says which asset is the artifact
//! and, since 2025, the digest it published. That digest is the reason this module exists —
//! the engine can verify what the service itself claims, without trusting the transfer.

use url::Url;

use super::{Extractor, Fetched, Resolved, SiteError, SiteOutcome};

/// How many listing pages are read before giving up.
const MAX_PAGES: usize = 5;

/// GitHub release pages, behind the public API.
pub struct GitHub;

impl GitHub {
    /// `owner/repo` plus the tag, when the link names one.
    fn parse(url: &Url) -> Option<(String, String, Option<String>)> {
        if !matches!(url.host_str(), Some("github.com") | Some("www.github.com")) {
            return None;
        }
        let parts: Vec<&str> = url.path_segments()?.collect();
        if parts.len() < 3 || parts[2] != "releases" {
            return None;
        }
        let owner = parts[0].to_string();
        let repo = parts[1].to_string();
        if owner.is_empty() || repo.is_empty() {
            return None;
        }
        let tag = match parts.get(3).copied() {
            Some("tag") => parts.get(4).map(|value| value.to_string()),
            Some("latest") => None,
            Some(_) => return None,
            None => None,
        };
        Some((owner, repo, tag))
    }

    /// API URL for the release the link names.
    fn api_url(owner: &str, repo: &str, tag: Option<&str>) -> String {
        match tag {
            Some(tag) => format!("https://api.github.com/repos/{owner}/{repo}/releases/tags/{tag}"),
            None => format!("https://api.github.com/repos/{owner}/{repo}/releases/latest"),
        }
    }
}

/// One downloadable asset of a release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    /// File name the service shows.
    pub name: String,
    /// Direct download URL.
    pub url: Url,
    /// Size in bytes.
    pub size: u64,
    /// Digest the service published, when it did.
    pub sha256: Option<String>,
}

/// Assets of a GitHub release answer, largest first.
fn release_assets(body: &str) -> Option<Vec<Asset>> {
    let release: serde_json::Value = serde_json::from_str(body).ok()?;
    let assets = release.get("assets")?.as_array()?;
    let mut out: Vec<Asset> = Vec::new();
    for asset in assets {
        let (Some(name), Some(link)) = (
            asset.get("name").and_then(|value| value.as_str()),
            asset
                .get("browser_download_url")
                .and_then(|value| value.as_str()),
        ) else {
            continue;
        };
        let Ok(url) = Url::parse(link) else {
            continue;
        };
        let size = asset
            .get("size")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        let sha256 = asset
            .get("digest")
            .and_then(|value| value.as_str())
            .and_then(|value| value.strip_prefix("sha256:"))
            .map(|value| value.to_ascii_lowercase());
        out.push(Asset {
            name: name.to_string(),
            url,
            size,
            sha256,
        });
    }
    out.sort_by(|a, b| b.size.cmp(&a.size));
    Some(out)
}

#[async_trait::async_trait]
impl Extractor for GitHub {
    fn name(&self) -> &'static str {
        "github"
    }

    fn matches(&self, url: &Url) -> bool {
        Self::parse(url).is_some()
    }

    async fn resolve(
        &self,
        fetcher: &dyn super::Fetcher,
        url: &Url,
    ) -> Result<SiteOutcome, SiteError> {
        let Some((owner, repo, tag)) = Self::parse(url) else {
            return Ok(SiteOutcome::Unsupported);
        };
        let token = std::env::var("SEGMENTA_GITHUB_TOKEN").ok();
        let headers: Vec<(String, String)> = token
            .filter(|value| !value.trim().is_empty())
            .map(|value| vec![("Authorization".to_string(), format!("Bearer {value}"))])
            .unwrap_or_default();
        let mut target =
            Url::parse(&Self::api_url(&owner, &repo, tag.as_deref())).map_err(|_| {
                SiteError::Config {
                    service: self.name(),
                    detail: "the API URL could not be built".to_string(),
                }
            })?;
        for _ in 0..MAX_PAGES {
            let answer: Fetched = fetcher.fetch(&target, &headers).await?;
            if answer.status == 404 {
                return Ok(SiteOutcome::Changed {
                    service: self.name(),
                    reason: "the release does not exist".to_string(),
                });
            }
            if !answer.is_success() {
                return Ok(SiteOutcome::Changed {
                    service: self.name(),
                    reason: format!("the API answered HTTP {}", answer.status),
                });
            }
            if let Some(assets) = release_assets(&answer.body) {
                if let Some(best) = assets.into_iter().next() {
                    return Ok(SiteOutcome::Resolved(Resolved {
                        url: best.url,
                        headers,
                        filename: Some(best.name),
                        sha256: best.sha256,
                    }));
                }
            }
            // A listing may answer an empty page with a next link; keep reading.
            match super::next_link(&answer.headers) {
                Some(next) => target = next,
                None => break,
            }
        }
        Ok(SiteOutcome::Changed {
            service: self.name(),
            reason: "the release publishes no downloadable asset".to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_release_link_names_owner_repo_and_tag() {
        let parsed = |url: &str| GitHub::parse(&Url::parse(url).unwrap());
        assert_eq!(
            parsed("https://github.com/o/r/releases/tag/v1.2.3"),
            Some(("o".into(), "r".into(), Some("v1.2.3".into())))
        );
        assert_eq!(
            parsed("https://github.com/o/r/releases/latest"),
            Some(("o".into(), "r".into(), None))
        );
        assert_eq!(parsed("https://github.com/o/r"), None);
        assert_eq!(parsed("https://example.test/o/r/releases/tag/v1"), None);
    }

    #[test]
    fn the_digest_is_read_and_assets_are_sorted_by_size() {
        let body = r#"{"assets":[
            {"name":"small.zip","browser_download_url":"https://github.com/o/r/small.zip","size":10},
            {"name":"big.zip","browser_download_url":"https://github.com/o/r/big.zip","size":900,
             "digest":"sha256:ABCDEF0123456789"}
        ]}"#;
        let assets = release_assets(body).unwrap();
        assert_eq!(assets[0].name, "big.zip");
        assert_eq!(assets[0].sha256.as_deref(), Some("abcdef0123456789"));
        assert_eq!(assets[1].sha256, None);
        assert_eq!(release_assets("{}"), None);
    }
}
