//! Gofile: the share code is looked up in the public contents API.
//!
//! Since 2023 the API answers `401` without an account token for most shares. That is not
//! a broken page: it is a service that needs credentials, so it is reported as such and
//! the user can add a token in Site Logins. No token is hardcoded here.

use url::Url;

use super::{Extractor, Resolved, SiteError, SiteOutcome};

/// `gofile.io/d/<code>` share pages.
pub struct Gofile;

impl Gofile {
    /// Share code from `/d/<code>`, when it is the whole path.
    fn share_code(url: &Url) -> Option<String> {
        let mut parts = url.path_segments()?;
        if parts.next()? != "d" {
            return None;
        }
        let code = parts.next()?;
        if parts.next().is_some() || code.is_empty() {
            return None;
        }
        Some(code.to_string())
    }

    /// Biggest file inside the answer, with the direct URL the API already provides.
    fn pick_file(contents: &serde_json::Value) -> Option<(String, String, u64)> {
        let children = contents
            .get("data")
            .and_then(|data| data.get("children"))
            .and_then(|children| children.as_object())?;
        let mut best: Option<(String, String, u64)> = None;
        for child in children.values() {
            if child.get("type").and_then(|value| value.as_str()) != Some("file") {
                continue;
            }
            let (Some(name), Some(link)) = (
                child.get("name").and_then(|value| value.as_str()),
                child.get("link").and_then(|value| value.as_str()),
            ) else {
                continue;
            };
            let size = child
                .get("size")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            if best
                .as_ref()
                .map_or(true, |(_, _, current)| size > *current)
            {
                best = Some((name.to_string(), link.to_string(), size));
            }
        }
        best
    }
}

#[async_trait::async_trait]
impl Extractor for Gofile {
    fn name(&self) -> &'static str {
        "gofile"
    }

    fn matches(&self, url: &Url) -> bool {
        matches!(url.host_str(), Some("gofile.io") | Some("www.gofile.io"))
            && Self::share_code(url).is_some()
    }

    async fn resolve(
        &self,
        fetcher: &dyn super::Fetcher,
        url: &Url,
    ) -> Result<SiteOutcome, SiteError> {
        let Some(code) = Self::share_code(url) else {
            return Ok(SiteOutcome::Unsupported);
        };
        let api = format!("https://api.gofile.io/contents/{code}");
        let Ok(api) = Url::parse(&api) else {
            return Ok(SiteOutcome::Changed {
                service: self.name(),
                reason: "the contents URL could not be built".to_string(),
            });
        };
        let token = std::env::var("SEGMENTA_GOFILE_TOKEN").ok();
        let headers: Vec<(String, String)> = token
            .filter(|value| !value.trim().is_empty())
            .map(|value| vec![("Authorization".to_string(), format!("Bearer {value}"))])
            .unwrap_or_default();
        let (status, body) = fetcher.page_with(&api, &headers).await?;
        if status == 401 || status == 403 {
            return Ok(SiteOutcome::Changed {
                service: self.name(),
                reason: "the share needs a gofile account token in Site Logins".to_string(),
            });
        }
        if !(200..300).contains(&status) {
            return Ok(SiteOutcome::Changed {
                service: self.name(),
                reason: format!("the contents API answered HTTP {status}"),
            });
        }
        let contents: serde_json::Value = match serde_json::from_str(&body) {
            Ok(value) => value,
            Err(_) => {
                return Ok(SiteOutcome::Changed {
                    service: self.name(),
                    reason: "the contents answer is not JSON".to_string(),
                })
            }
        };
        let Some((name, link, _size)) = Self::pick_file(&contents) else {
            return Ok(SiteOutcome::Changed {
                service: self.name(),
                reason: "the share lists no file".to_string(),
            });
        };
        let Ok(target) = Url::parse(&link) else {
            return Ok(SiteOutcome::Changed {
                service: self.name(),
                reason: "the file entry carries no usable URL".to_string(),
            });
        };
        Ok(SiteOutcome::Resolved(Resolved {
            url: target,
            headers,
            filename: Some(name),
            sha256: None,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_share_links_are_claimed() {
        assert!(Gofile.matches(&Url::parse("https://gofile.io/d/AbC123").unwrap()));
        assert!(!Gofile.matches(&Url::parse("https://gofile.io/d/AbC123/extra").unwrap()));
        assert!(!Gofile.matches(&Url::parse("https://gofile.io/").unwrap()));
        assert!(!Gofile.matches(&Url::parse("https://example.test/d/AbC").unwrap()));
    }

    #[test]
    fn the_biggest_file_of_the_share_is_chosen() {
        let contents = serde_json::json!({
            "data": { "children": {
                "one": {"type":"file","name":"small.txt","link":"https://store1/a","size":10},
                "two": {"type":"file","name":"big.mp4","link":"https://store1/b","size":900},
                "dir": {"type":"folder","name":"sub"}
            }}
        });
        assert_eq!(
            Gofile::pick_file(&contents),
            Some(("big.mp4".to_string(), "https://store1/b".to_string(), 900))
        );
        assert_eq!(Gofile::pick_file(&serde_json::json!({})), None);
    }
}
