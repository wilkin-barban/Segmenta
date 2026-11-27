//! Google Drive: the id in the URL is enough, so nothing is fetched.
//!
//! Two shapes matter: a regular file, which downloads through `drive.usercontent`, and a
//! Docs/Sheets/Slides document, which has no file at all until it is exported. Guessing
//! the export format is what makes a Drive link usable instead of a `.gdoc` stub.

use url::Url;

use super::{Extractor, Resolved, SiteError, SiteOutcome};

/// `drive.google.com` files and `docs.google.com` documents.
pub struct GoogleDrive;

/// A document kind and the Office format it exports to.
const DOCS: &[(&str, &str)] = &[
    ("document", "docx"),
    ("spreadsheets", "xlsx"),
    ("presentation", "pptx"),
];

impl GoogleDrive {
    /// The `d/<id>` segment of a Docs or Drive path.
    fn id_after_d(url: &Url) -> Option<String> {
        let mut parts = url.path_segments()?;
        let mut previous = "";
        for part in parts.by_ref() {
            if previous == "d" && !part.is_empty() {
                return Some(part.to_string());
            }
            previous = part;
        }
        None
    }

    /// Which Docs product the path names, if any.
    fn docs_kind(url: &Url) -> Option<(String, &'static str)> {
        let first = url.path_segments()?.next()?.to_string();
        let (_, format) = DOCS.iter().find(|(kind, _)| *kind == first)?;
        Some((first, format))
    }
}

#[async_trait::async_trait]
impl Extractor for GoogleDrive {
    fn name(&self) -> &'static str {
        "google-drive"
    }

    fn matches(&self, url: &Url) -> bool {
        match url.host_str() {
            Some("drive.google.com") => url.path().contains("/file/"),
            Some("docs.google.com") => Self::docs_kind(url).is_some(),
            _ => false,
        }
    }

    async fn resolve(
        &self,
        _fetcher: &dyn super::Fetcher,
        url: &Url,
    ) -> Result<SiteOutcome, SiteError> {
        if url.host_str() == Some("docs.google.com") {
            let (Some((kind, format)), Some(id)) = (Self::docs_kind(url), Self::id_after_d(url))
            else {
                return Ok(SiteOutcome::Changed {
                    service: self.name(),
                    reason: "the document id is missing from the link".to_string(),
                });
            };
            let target = format!("https://docs.google.com/{kind}/d/{id}/export?format={format}");
            let Ok(target) = Url::parse(&target) else {
                return Ok(SiteOutcome::Changed {
                    service: self.name(),
                    reason: "the export URL could not be built".to_string(),
                });
            };
            return Ok(SiteOutcome::Resolved(Resolved {
                url: target,
                headers: Vec::new(),
                filename: None,
                sha256: None,
            }));
        }
        let Some(id) = Self::id_after_d(url) else {
            return Ok(SiteOutcome::Changed {
                service: self.name(),
                reason: "the file id is missing from the link".to_string(),
            });
        };
        // `confirm=t` is the answer the virus-scan interstitial submits for a large file;
        // asking for it directly avoids parsing that page at all.
        let target = format!(
            "https://drive.usercontent.google.com/download?id={id}&export=download&confirm=t"
        );
        let Ok(target) = Url::parse(&target) else {
            return Ok(SiteOutcome::Changed {
                service: self.name(),
                reason: "the download URL could not be built".to_string(),
            });
        };
        Ok(SiteOutcome::Resolved(Resolved {
            url: target,
            headers: Vec::new(),
            filename: None,
            sha256: None,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(url: &str) -> Option<String> {
        GoogleDrive::id_after_d(&Url::parse(url).unwrap())
    }

    #[test]
    fn the_id_is_taken_after_the_d_segment() {
        assert_eq!(
            id("https://drive.google.com/file/d/ABC/view"),
            Some("ABC".to_string())
        );
        assert_eq!(
            id("https://docs.google.com/document/d/DOC/edit"),
            Some("DOC".to_string())
        );
        assert_eq!(id("https://drive.google.com/drive/folders/XYZ"), None);
    }

    #[test]
    fn only_drive_files_and_docs_are_claimed() {
        for url in [
            "https://drive.google.com/file/d/A/view",
            "https://docs.google.com/document/d/B/edit",
            "https://docs.google.com/spreadsheets/d/C/edit",
            "https://docs.google.com/presentation/d/D/edit",
        ] {
            assert!(GoogleDrive.matches(&Url::parse(url).unwrap()), "{url}");
        }
        for url in [
            "https://drive.google.com/drive/folders/A",
            "https://docs.google.com/forms/d/e/A/viewform",
            "https://example.test/file/d/A/view",
        ] {
            assert!(!GoogleDrive.matches(&Url::parse(url).unwrap()), "{url}");
        }
    }
}
