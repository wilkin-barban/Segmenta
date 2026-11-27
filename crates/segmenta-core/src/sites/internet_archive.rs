//! Internet Archive: an item's metadata names its files; the download URL is derived.

use url::Url;

use super::{encode_segment, Extractor, Resolved, SiteError, SiteOutcome};

/// `archive.org/details/<id>` item pages.
pub struct InternetArchive;

impl InternetArchive {
    /// Item identifier from a details URL, when the path has exactly one segment.
    fn item_id(url: &Url) -> Option<String> {
        let mut parts = url.path_segments()?;
        let first = parts.next()?;
        if first != "details" {
            return None;
        }
        let id = parts.next()?;
        if parts.next().is_some() || id.is_empty() {
            return None;
        }
        Some(id.to_string())
    }

    /// Metadata endpoint for an item, on the same origin the user opened.
    fn metadata_url(url: &Url, id: &str) -> Option<Url> {
        let mut target = url.clone();
        target.set_path(&format!("/metadata/{id}"));
        target.set_query(None);
        Some(target)
    }

    /// The file to download: the largest one that is not a derivative thumbnail.
    fn pick_file(metadata: &serde_json::Value) -> Option<(String, u64)> {
        let files = metadata.get("files")?.as_array()?;
        let mut best: Option<(String, u64)> = None;
        for file in files {
            let Some(name) = file.get("name").and_then(|value| value.as_str()) else {
                continue;
            };
            let format = file
                .get("format")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            if format.eq_ignore_ascii_case("thumbnail") || name.starts_with("__ia_") {
                continue;
            }
            let size = file
                .get("size")
                .and_then(|value| value.as_str())
                .and_then(|value| value.parse::<u64>().ok())
                .or_else(|| file.get("size").and_then(serde_json::Value::as_u64))
                .unwrap_or(0);
            if best.as_ref().map_or(true, |(_, current)| size > *current) {
                best = Some((name.to_string(), size));
            }
        }
        best
    }
}

#[async_trait::async_trait]
impl Extractor for InternetArchive {
    fn name(&self) -> &'static str {
        "internet-archive"
    }

    fn matches(&self, url: &Url) -> bool {
        matches!(
            url.host_str(),
            Some("archive.org") | Some("www.archive.org")
        ) && Self::item_id(url).is_some()
    }

    async fn resolve(
        &self,
        fetcher: &dyn super::Fetcher,
        url: &Url,
    ) -> Result<SiteOutcome, SiteError> {
        let Some(id) = Self::item_id(url) else {
            return Ok(SiteOutcome::Unsupported);
        };
        let Some(metadata) = Self::metadata_url(url, &id) else {
            return Ok(SiteOutcome::Unsupported);
        };
        let (status, body) = fetcher.page(&metadata).await?;
        if !(200..300).contains(&status) {
            return Ok(SiteOutcome::Changed {
                service: self.name(),
                reason: format!("the metadata answered HTTP {status}"),
            });
        }
        let metadata: serde_json::Value = match serde_json::from_str(&body) {
            Ok(value) => value,
            Err(_) => {
                return Ok(SiteOutcome::Changed {
                    service: self.name(),
                    reason: "the metadata is not JSON".to_string(),
                })
            }
        };
        let Some((name, _size)) = Self::pick_file(&metadata) else {
            return Ok(SiteOutcome::Changed {
                service: self.name(),
                reason: "the item lists no downloadable file".to_string(),
            });
        };
        let mut target = url.clone();
        target.set_path(&format!("/download/{id}/{}", encode_segment(&name)));
        target.set_query(None);
        Ok(SiteOutcome::Resolved(Resolved {
            url: target,
            headers: Vec::new(),
            filename: Some(name),
            sha256: None,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(url: &str) -> Option<String> {
        InternetArchive::item_id(&Url::parse(url).unwrap())
    }

    #[test]
    fn only_single_segment_items_are_claimed() {
        assert_eq!(
            item("https://archive.org/details/episode-one"),
            Some("episode-one".to_string())
        );
        assert_eq!(item("https://archive.org/details/"), None);
        assert_eq!(item("https://archive.org/download/x/y"), None);
        assert_eq!(item("https://archive.org/details/x/extra"), None);
        // The item id is path-only; the host gate lives in `matches`.
        assert_eq!(
            item("https://example.test/details/x"),
            Some("x".to_string())
        );
        assert!(!InternetArchive.matches(&Url::parse("https://example.test/details/x").unwrap()));
    }

    #[test]
    fn the_largest_real_file_wins_and_thumbnails_are_skipped() {
        let metadata = serde_json::json!({
            "files": [
                {"name": "__ia_thumb.jpg", "format": "Thumbnail", "size": "9"},
                {"name": "small.txt", "format": "Text", "size": "10"},
                {"name": "big.mp4", "format": "MPEG4", "size": "1000"}
            ]
        });
        assert_eq!(
            InternetArchive::pick_file(&metadata),
            Some(("big.mp4".to_string(), 1000))
        );
        assert_eq!(InternetArchive::pick_file(&serde_json::json!({})), None);
    }
}
