//! Native hoster extractors: a link becomes the real file URL, never an HTML page.
//!
//! Contract: T1 of `odd/tasks/segmenta-hito2-captura-hosters-remoto.md`.
//!
//! Every extractor is pure over an HTTP answer: the test server below serves the same
//! shapes the services serve, so nothing here reaches the network.

use segmenta_core::sites::{resolve_with, SiteOutcome};
use url::Url;

/// A fetcher that answers from a table keyed by path: the host matching stays real, so
/// the registry is exercised exactly as in production.
struct Canned {
    answers: std::collections::HashMap<String, (u16, String)>,
    asked: std::sync::Mutex<Vec<String>>,
}

impl Canned {
    fn new(answers: &[(&str, u16, &str)]) -> Canned {
        Canned {
            answers: answers
                .iter()
                .map(|(path, status, body)| (path.to_string(), (*status, body.to_string())))
                .collect(),
            asked: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl segmenta_core::sites::Fetcher for Canned {
    async fn page(&self, url: &Url) -> Result<(u16, String), segmenta_core::sites::SiteError> {
        self.asked.lock().unwrap().push(url.path().to_string());
        Ok(self
            .answers
            .get(url.path())
            .cloned()
            .unwrap_or((404, String::new())))
    }
}

#[tokio::test]
async fn mediafire_page_becomes_a_direct_download() {
    // The shape MediaFire serves for a public file: the anchor carries id="downloadButton"
    // and the href is the CDN URL, on another host.
    let fetcher = Canned::new(&[(
        "/file/abc123/file.zip",
        200,
        r#"<html><body><a class="input popsok" id="downloadButton"
              href="https://download2392.mediafire.com/abc123/file.zip">Download</a></body></html>"#,
    )]);
    let url = Url::parse("https://www.mediafire.com/file/abc123/file.zip").unwrap();
    match resolve_with(&fetcher, &url).await.unwrap() {
        SiteOutcome::Resolved(resolved) => {
            assert_eq!(
                resolved.url.as_str(),
                "https://download2392.mediafire.com/abc123/file.zip"
            );
            assert_eq!(resolved.filename.as_deref(), Some("file.zip"));
        }
        other => panic!("mediafire must be recognized, got {other:?}"),
    }
}

#[tokio::test]
async fn a_mediafire_page_without_the_button_is_reported_as_changed() {
    // A page that no longer carries the marker is not a download: returning the HTML URL
    // would save an .html file and hide the breakage.
    let fetcher = Canned::new(&[("/file/abc123/file.zip", 200, "<html>File removed</html>")]);
    let url = Url::parse("https://www.mediafire.com/file/abc123/file.zip").unwrap();
    let outcome = resolve_with(&fetcher, &url).await.unwrap();
    assert!(
        matches!(outcome, SiteOutcome::Changed { .. }),
        "a missing marker must be reported, got {outcome:?}"
    );
}

#[tokio::test]
async fn an_unknown_host_is_left_alone() {
    let fetcher = Canned::new(&[]);
    let url = Url::parse("https://example.test/file.bin").unwrap();
    assert!(matches!(
        resolve_with(&fetcher, &url).await.unwrap(),
        SiteOutcome::Unsupported
    ));
}

#[tokio::test]
async fn a_resolved_host_is_never_arbitrary() {
    // The extractor may only hand back http(s): a page pointing at `file://` or `javascript:`
    // is a broken or hostile answer, not a download.
    let fetcher = Canned::new(&[(
        "/file/abc/x.zip",
        200,
        r#"<a id="downloadButton" href="file:///etc/passwd">x</a>"#,
    )]);
    let url = Url::parse("https://www.mediafire.com/file/abc/x.zip").unwrap();
    assert!(matches!(
        resolve_with(&fetcher, &url).await.unwrap(),
        SiteOutcome::Changed { .. }
    ));
}
