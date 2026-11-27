//! Hugging Face: a model or dataset page is a tree of files, and its LFS entries carry the
//! SHA-256 of the bytes. That digest is what makes a 40 GiB weights file verifiable.

use url::Url;

use super::{Extractor, Resolved, SiteError, SiteOutcome};

/// How many tree pages are read before giving up.
const MAX_PAGES: usize = 10;

/// `huggingface.co/<owner>/<repo>` model and dataset pages.
pub struct HuggingFace;

/// One file of the tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeFile {
    /// Path inside the repository.
    pub path: String,
    /// Size in bytes.
    pub size: u64,
    /// SHA-256 from the LFS pointer, when the file is stored in LFS.
    pub sha256: Option<String>,
}

impl HuggingFace {
    /// `owner/repo` and whether the link names a dataset.
    fn parse(url: &Url) -> Option<(String, String)> {
        if !matches!(
            url.host_str(),
            Some("huggingface.co") | Some("www.huggingface.co")
        ) {
            return None;
        }
        let parts: Vec<&str> = url.path_segments()?.collect();
        // `/datasets/<owner>/<repo>` or `/<owner>/<repo>`.
        let rest = if parts.first() == Some(&"datasets") {
            &parts[1..]
        } else {
            &parts[..]
        };
        if rest.len() < 2 || rest[0].is_empty() || rest[1].is_empty() {
            return None;
        }
        Some((rest[0].to_string(), rest[1].to_string()))
    }

    /// API URL listing the tree of the default revision.
    fn tree_url(url: &Url, owner: &str, repo: &str) -> String {
        let is_dataset = url.path_segments().and_then(|mut p| p.next()) == Some("datasets");
        let kind = if is_dataset { "datasets" } else { "models" };
        format!("https://huggingface.co/api/{kind}/{owner}/{repo}/tree/main?recursive=true")
    }
}

/// Files of one page, biggest first. `lfs.oid` is the SHA-256 of an LFS object.
pub fn tree_files(body: &str) -> Option<Vec<TreeFile>> {
    let entries: serde_json::Value = serde_json::from_str(body).ok()?;
    let entries = entries.as_array()?;
    let mut files: Vec<TreeFile> = Vec::new();
    for entry in entries {
        if entry.get("type").and_then(|value| value.as_str()) != Some("file") {
            continue;
        }
        let Some(path) = entry.get("path").and_then(|value| value.as_str()) else {
            continue;
        };
        let size = entry
            .get("size")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        let sha256 = entry
            .get("lfs")
            .and_then(|lfs| lfs.get("oid"))
            .and_then(|value| value.as_str())
            .map(|value| value.to_ascii_lowercase());
        files.push(TreeFile {
            path: path.to_string(),
            size,
            sha256,
        });
    }
    files.sort_by(|a, b| b.size.cmp(&a.size));
    Some(files)
}

#[async_trait::async_trait]
impl Extractor for HuggingFace {
    fn name(&self) -> &'static str {
        "hugging-face"
    }

    fn matches(&self, url: &Url) -> bool {
        Self::parse(url).is_some()
    }

    async fn resolve(
        &self,
        fetcher: &dyn super::Fetcher,
        url: &Url,
    ) -> Result<SiteOutcome, SiteError> {
        let Some((owner, repo)) = Self::parse(url) else {
            return Ok(SiteOutcome::Unsupported);
        };
        let token = std::env::var("SEGMENTA_HF_TOKEN").ok();
        let headers: Vec<(String, String)> = token
            .filter(|value| !value.trim().is_empty())
            .map(|value| vec![("Authorization".to_string(), format!("Bearer {value}"))])
            .unwrap_or_default();
        let mut target =
            Url::parse(&Self::tree_url(url, &owner, &repo)).map_err(|_| SiteError::Config {
                service: self.name(),
                detail: "the tree URL could not be built".to_string(),
            })?;
        // Every page contributes: a repository large enough to paginate must not lose the
        // biggest file just because it sits on the second page.
        let mut files: Vec<TreeFile> = Vec::new();
        for _ in 0..MAX_PAGES {
            let answer = fetcher.fetch(&target, &headers).await?;
            if answer.status == 401 || answer.status == 403 {
                return Ok(SiteOutcome::Changed {
                    service: self.name(),
                    reason: "the repository needs a token in Site Logins".to_string(),
                });
            }
            if answer.status == 404 {
                return Ok(SiteOutcome::Changed {
                    service: self.name(),
                    reason: "the repository does not exist".to_string(),
                });
            }
            if !answer.is_success() {
                return Ok(SiteOutcome::Changed {
                    service: self.name(),
                    reason: format!("the tree API answered HTTP {}", answer.status),
                });
            }
            files.extend(tree_files(&answer.body).unwrap_or_default());
            match super::next_link(&answer.headers) {
                Some(next) => target = next,
                None => break,
            }
        }
        files.sort_by(|a, b| b.size.cmp(&a.size));
        let Some(best) = files.into_iter().next() else {
            return Ok(SiteOutcome::Changed {
                service: self.name(),
                reason: "the repository lists no file".to_string(),
            });
        };
        let prefix = if url.path().starts_with("/datasets/") {
            format!("datasets/{owner}")
        } else {
            owner.clone()
        };
        let download = format!(
            "https://huggingface.co/{prefix}/{repo}/resolve/main/{}",
            best.path
        );
        let Ok(target) = Url::parse(&download) else {
            return Ok(SiteOutcome::Changed {
                service: self.name(),
                reason: "the download URL could not be built".to_string(),
            });
        };
        Ok(SiteOutcome::Resolved(Resolved {
            url: target,
            headers,
            filename: Some(best.path),
            sha256: best.sha256,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_and_dataset_links_are_read() {
        let parsed = |url: &str| HuggingFace::parse(&Url::parse(url).unwrap());
        assert_eq!(
            parsed("https://huggingface.co/org/model"),
            Some(("org".into(), "model".into()))
        );
        assert_eq!(
            parsed("https://huggingface.co/datasets/org/data"),
            Some(("org".into(), "data".into()))
        );
        assert_eq!(parsed("https://huggingface.co/only-owner"), None);
        assert_eq!(parsed("https://example.test/org/model"), None);
    }

    #[test]
    fn lfs_entries_carry_the_digest_and_files_are_sorted() {
        let body = r#"[
            {"type":"file","path":"small.json","size":10},
            {"type":"file","path":"model.safetensors","size":900,"lfs":{"oid":"AABBCC"}},
            {"type":"directory","path":"sub"}
        ]"#;
        let files = tree_files(body).unwrap();
        assert_eq!(files[0].path, "model.safetensors");
        assert_eq!(files[0].sha256.as_deref(), Some("aabbcc"));
        assert_eq!(files[1].sha256, None);
        assert_eq!(tree_files("{}"), None);
    }
}
