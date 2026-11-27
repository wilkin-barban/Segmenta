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
async fn an_internet_archive_item_resolves_through_its_metadata() {
    // `archive.org/details/<id>` becomes `/download/<id>/<file>`; the file chooser uses the
    // metadata API instead of guessing.
    let fetcher = Canned::new(&[(
        "/metadata/episode-one",
        200,
        r#"{"files":[
             {"name":"__ia_thumb.jpg","format":"Thumbnail","size":"1234"},
             {"name":"episode one.mp4","format":"MPEG4","size":"1048576"}
           ]}"#,
    )]);
    let url = Url::parse("https://archive.org/details/episode-one").unwrap();
    match resolve_with(&fetcher, &url).await.unwrap() {
        SiteOutcome::Resolved(resolved) => {
            assert_eq!(
                resolved.url.as_str(),
                "https://archive.org/download/episode-one/episode%20one.mp4"
            );
            assert_eq!(resolved.filename.as_deref(), Some("episode one.mp4"));
        }
        other => panic!("the archive item must resolve, got {other:?}"),
    }
    assert_eq!(fetcher.asked(), vec!["/metadata/episode-one".to_string()]);
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

#[tokio::test]
async fn a_redirect_to_a_private_host_is_never_followed() {
    use segmenta_core::sites::Fetcher;

    // The production fetcher must guard every hop, not just the first URL: a hoster page
    // that answers 302 to loopback would otherwise make this process connect to itself.
    let http = segmenta_core::probe::HttpConfig {
        allow_local: false,
        ..segmenta_core::probe::HttpConfig::default()
    };
    let fetcher = segmenta_core::sites::HttpFetcher::new(&http).unwrap();
    let refused = fetcher
        .page(&Url::parse("http://127.0.0.1:9/private").unwrap())
        .await;
    assert!(
        refused.is_err(),
        "a private target must be refused before the socket: {refused:?}"
    );
}

#[tokio::test]
async fn a_redirect_is_followed_when_local_downloads_are_allowed() {
    use segmenta_core::sites::Fetcher;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    // Two hops on loopback: the first answers 302 to the second. With the switch on, the
    // guard must allow both; without it the same chain is refused (previous test).
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let mut chunk = [0u8; 1024];
            let _ = stream.read(&mut chunk).await;
            let request = String::from_utf8_lossy(&chunk).to_string();
            let response = if request.contains("GET /hop ") {
                "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_string()
            } else {
                format!(
                    "HTTP/1.1 302 Found\r\nLocation: http://{addr}/hop\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                )
            };
            let _ = stream.write_all(response.as_bytes()).await;
        }
    });

    let http = segmenta_core::probe::HttpConfig {
        allow_local: true,
        ..segmenta_core::probe::HttpConfig::default()
    };
    let fetcher = segmenta_core::sites::HttpFetcher::new(&http).unwrap();
    let (status, body) = fetcher
        .page(&Url::parse(&format!("http://{addr}/start")).unwrap())
        .await
        .unwrap();
    assert_eq!(status, 200, "the redirect must be followed by hand");
    assert_eq!(body, "ok");
}

#[tokio::test]
async fn a_google_drive_file_becomes_the_download_endpoint() {
    // No page is fetched: the id in the URL is all the endpoint needs. The confirm
    // parameter is what the "file too large to scan" interstitial ends up submitting.
    let fetcher = Canned::new(&[]);
    let url = Url::parse("https://drive.google.com/file/d/1AbCdEfGhIjK/view?usp=sharing").unwrap();
    match resolve_with(&fetcher, &url).await.unwrap() {
        SiteOutcome::Resolved(resolved) => assert_eq!(
            resolved.url.as_str(),
            "https://drive.usercontent.google.com/download?id=1AbCdEfGhIjK&export=download&confirm=t"
        ),
        other => panic!("google drive must resolve, got {other:?}"),
    }
    assert!(fetcher.asked().is_empty(), "no page is read for a file id");
}

#[tokio::test]
async fn a_google_doc_becomes_its_office_export() {
    let fetcher = Canned::new(&[]);
    for (url, expected) in [
        (
            "https://docs.google.com/document/d/DOC1/edit",
            "https://docs.google.com/document/d/DOC1/export?format=docx",
        ),
        (
            "https://docs.google.com/spreadsheets/d/SHEET1/edit",
            "https://docs.google.com/spreadsheets/d/SHEET1/export?format=xlsx",
        ),
        (
            "https://docs.google.com/presentation/d/PRES1/edit",
            "https://docs.google.com/presentation/d/PRES1/export?format=pptx",
        ),
    ] {
        let parsed = Url::parse(url).unwrap();
        match resolve_with(&fetcher, &parsed).await.unwrap() {
            SiteOutcome::Resolved(resolved) => assert_eq!(resolved.url.as_str(), expected, "{url}"),
            other => panic!("{url} must resolve, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn rapidgator_takes_the_link_the_page_announces() {
    let fetcher = Canned::new(&[(
        "/file/abc123/name.zip",
        200,
        r#"<script>location.href = 'https://rg.to/download/xyz/name.zip';</script>"#,
    )]);
    let url = Url::parse("https://rapidgator.net/file/abc123/name.zip").unwrap();
    match resolve_with(&fetcher, &url).await.unwrap() {
        SiteOutcome::Resolved(resolved) => {
            assert_eq!(resolved.url.as_str(), "https://rg.to/download/xyz/name.zip")
        }
        other => panic!("rapidgator must resolve, got {other:?}"),
    }
}

#[tokio::test]
async fn a_rapidgator_page_without_a_download_link_is_reported() {
    let fetcher = Canned::new(&[("/file/abc/name.zip", 200, "<html>File not found</html>")]);
    let url = Url::parse("https://rapidgator.net/file/abc/name.zip").unwrap();
    assert!(matches!(
        resolve_with(&fetcher, &url).await.unwrap(),
        SiteOutcome::Changed { .. }
    ));
}
