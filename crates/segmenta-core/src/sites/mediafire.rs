//! MediaFire: the page carries the CDN URL in the download button.

use url::Url;

use super::{Extractor, Resolved, SiteError, SiteOutcome};

/// `mediafire.com` public file pages.
pub struct MediaFire;

impl MediaFire {
    fn is_file_page(url: &Url) -> bool {
        matches!(
            url.host_str(),
            Some("mediafire.com") | Some("www.mediafire.com") | Some("m.mediafire.com")
        ) && url.path().starts_with("/file/")
    }
}

/// Pull the `href` that follows the download button marker.
///
/// The page is HTML from a third party, so this looks for the marker and then the first
/// quoted attribute after it, bounded to the same tag. It never builds a URL from page
/// text: the href is parsed by `Url`.
fn download_href(page: &str) -> Option<&str> {
    let marker = page.find("id=\"downloadButton\"")?;
    // The attribute may come before the id; search both directions within one tag.
    let tag_start = page[..marker].rfind('<').unwrap_or(0);
    let tag_end = page[marker..].find('>').map(|end| marker + end)?;
    let tag = &page[tag_start..tag_end];
    let href = tag.split("href=\"").nth(1)?;
    href.split('"').next()
}

#[async_trait::async_trait]
impl Extractor for MediaFire {
    fn name(&self) -> &'static str {
        "mediafire"
    }

    fn matches(&self, url: &Url) -> bool {
        Self::is_file_page(url)
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
        let Some(href) = download_href(&page) else {
            return Ok(SiteOutcome::Changed {
                service: self.name(),
                reason: "the download button is gone from the page".to_string(),
            });
        };
        let Ok(target) = Url::parse(href) else {
            return Ok(SiteOutcome::Changed {
                service: self.name(),
                reason: "the download button does not carry a URL".to_string(),
            });
        };
        Ok(SiteOutcome::Resolved(Resolved {
            filename: target
                .path_segments()
                .and_then(|mut parts| parts.next_back().map(str::to_string))
                .filter(|name| !name.is_empty()),
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
    fn the_marker_can_come_before_or_after_the_href() {
        let after = r#"<a id="downloadButton" href="https://cdn.example/f.zip">x</a>"#;
        assert_eq!(download_href(after), Some("https://cdn.example/f.zip"));
        let before = r#"<a href="https://cdn.example/f.zip" class="x" id="downloadButton">x</a>"#;
        assert_eq!(download_href(before), Some("https://cdn.example/f.zip"));
        assert_eq!(download_href("<a href=\"x\">no button</a>"), None);
    }

    #[test]
    fn only_file_pages_are_claimed() {
        for url in [
            "https://www.mediafire.com/file/abc123/name.zip",
            "https://mediafire.com/file/abc/name.zip",
        ] {
            assert!(MediaFire.matches(&Url::parse(url).unwrap()), "{url}");
        }
        for url in [
            "https://www.mediafire.com/",
            "https://mediafire.com/folder/abc",
            "https://example.test/file/abc/x.zip",
        ] {
            assert!(!MediaFire.matches(&Url::parse(url).unwrap()), "{url}");
        }
    }
}
