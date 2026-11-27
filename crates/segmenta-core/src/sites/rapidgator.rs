//! Rapidgator: the page announces the real link in a small script.
//!
//! The link is time-limited, so it is read at resolve time and never cached. A page
//! without it means the file expired, was removed, or the flow changed: all three are
//! reported, none of them becomes an HTML download.

use url::Url;

use super::{Extractor, Resolved, SiteError, SiteOutcome};

/// `rapidgator.net` file pages.
pub struct Rapidgator;

impl Rapidgator {
    /// The URL inside `location.href = '...'`, bounded to the same statement.
    fn announced_link(page: &str) -> Option<&str> {
        let marker = page.find("location.href")?;
        let rest = &page[marker..];
        let quote = rest.find(['\'', '"'])?;
        let delimiter = rest[quote..].chars().next()?;
        let after = &rest[quote + delimiter.len_utf8()..];
        let end = after.find(delimiter)?;
        let candidate = &after[..end];
        candidate.starts_with("http").then_some(candidate)
    }
}

#[async_trait::async_trait]
impl Extractor for Rapidgator {
    fn name(&self) -> &'static str {
        "rapidgator"
    }

    fn matches(&self, url: &Url) -> bool {
        matches!(
            url.host_str(),
            Some("rapidgator.net") | Some("www.rapidgator.net")
        ) && url.path().starts_with("/file/")
    }

    async fn resolve(
        &self,
        fetcher: &dyn super::Fetcher,
        url: &Url,
    ) -> Result<SiteOutcome, SiteError> {
        let (status, page) = fetcher.page(url).await?;
        if !(200..300).contains(&status) {
            return Ok(SiteOutcome::Changed {
                service: self.name(),
                reason: format!("the page answered HTTP {status}"),
            });
        }
        let Some(link) = Self::announced_link(&page) else {
            return Ok(SiteOutcome::Changed {
                service: self.name(),
                reason: "the page announces no download link (expired or removed)".to_string(),
            });
        };
        let Ok(target) = Url::parse(link) else {
            return Ok(SiteOutcome::Changed {
                service: self.name(),
                reason: "the announced link is not a URL".to_string(),
            });
        };
        Ok(SiteOutcome::Resolved(Resolved {
            filename: url
                .path_segments()
                .and_then(|mut parts| parts.next_back().map(str::to_string))
                .filter(|name| !name.is_empty() && name != "file"),
            headers: Vec::new(),
            sha256: None,
            url: target,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_announced_link_is_read_with_either_quote() {
        assert_eq!(
            Rapidgator::announced_link("<script>location.href = 'https://x/a.zip';</script>"),
            Some("https://x/a.zip")
        );
        assert_eq!(
            Rapidgator::announced_link(r#"location.href="https://x/b.zip";"#),
            Some("https://x/b.zip")
        );
        assert_eq!(Rapidgator::announced_link("location.href = ''"), None);
        assert_eq!(Rapidgator::announced_link("no script here"), None);
    }

    #[test]
    fn only_file_pages_are_claimed() {
        assert!(
            Rapidgator.matches(&Url::parse("https://rapidgator.net/file/abc/name.zip").unwrap())
        );
        assert!(!Rapidgator.matches(&Url::parse("https://rapidgator.net/").unwrap()));
        assert!(!Rapidgator.matches(&Url::parse("https://example.test/file/abc/x.zip").unwrap()));
    }
}
