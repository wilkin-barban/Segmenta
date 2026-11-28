//! Folder spidering: an "Index of /" page becomes a bounded list of files.

use segmenta_core::folder::{crawl, FolderLimits, FolderStop};
use segmenta_core::sites::{Fetched, Fetcher, SiteError};
use std::collections::HashMap;
use std::sync::Mutex;
use url::Url;

/// Serves canned pages by path, the way the hoster tests do.
struct Pages {
    pages: HashMap<String, String>,
    asked: Mutex<Vec<String>>,
}

impl Pages {
    fn new(pages: &[(&str, &str)]) -> Pages {
        Pages {
            pages: pages
                .iter()
                .map(|(path, body)| (path.to_string(), body.to_string()))
                .collect(),
            asked: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait::async_trait]
impl Fetcher for Pages {
    async fn page(&self, url: &Url) -> Result<(u16, String), SiteError> {
        self.asked.lock().unwrap().push(url.path().to_string());
        Ok(match self.pages.get(url.path()) {
            Some(body) => (200, body.clone()),
            None => (404, String::new()),
        })
    }

    async fn fetch(&self, url: &Url, _headers: &[(String, String)]) -> Result<Fetched, SiteError> {
        let (status, body) = self.page(url).await?;
        Ok(Fetched {
            status,
            headers: Vec::new(),
            body,
        })
    }
}

fn limits() -> FolderLimits {
    FolderLimits {
        max_depth: 2,
        max_folders: 10,
        max_files: 50,
    }
}

#[tokio::test]
async fn a_listing_yields_its_files_and_descends_into_folders() {
    let pages = Pages::new(&[
        (
            "/index/",
            r#"<html><body><h1>Index of /</h1>
               <a href="../">../</a>
               <a href="one.bin">one.bin</a>
               <a href="sub/">sub/</a>
             </body></html>"#,
        ),
        (
            "/index/sub/",
            r#"<html><body><a href="../">../</a><a href="two.bin">two.bin</a></body></html>"#,
        ),
    ]);
    let found = crawl(
        &Url::parse("https://example.test/index/").unwrap(),
        limits(),
        &pages,
    )
    .await
    .unwrap();
    let names: Vec<String> = found.iter().map(|entry| entry.name.clone()).collect();
    assert!(names.contains(&"one.bin".to_string()), "{names:?}");
    assert!(names.contains(&"two.bin".to_string()), "{names:?}");
    // The parent link is never followed.
    assert!(!pages.asked.lock().unwrap().iter().any(|path| path == "/"));
}

#[tokio::test]
async fn a_link_to_another_host_is_never_followed() {
    let pages = Pages::new(&[(
        "/index/",
        r#"<a href="https://evil.test/file.bin">x</a><a href="local.bin">l</a>"#,
    )]);
    let found = crawl(
        &Url::parse("https://example.test/index/").unwrap(),
        limits(),
        &pages,
    )
    .await
    .unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name, "local.bin");
    assert!(pages
        .asked
        .lock()
        .unwrap()
        .iter()
        .all(|path| path.starts_with("/index/")));
}

#[tokio::test]
async fn the_depth_limit_stops_the_crawl_and_says_so() {
    // Every answer carries one more folder and one file: without the depth limit this
    // never ends, which is exactly what the limit exists for.
    struct Chain;
    #[async_trait::async_trait]
    impl Fetcher for Chain {
        async fn page(&self, url: &Url) -> Result<(u16, String), SiteError> {
            let depth = url.path().matches('/').count();
            Ok((
                200,
                format!(r#"<a href="d{depth}/">next</a><a href="f{depth}.bin">f</a>"#),
            ))
        }
    }
    let found = crawl(
        &Url::parse("https://example.test/index/").unwrap(),
        FolderLimits {
            max_depth: 1,
            max_folders: 10,
            max_files: 50,
        },
        &Chain,
    )
    .await
    .unwrap();
    assert_eq!(
        found.last().and_then(|entry| entry.stop),
        Some(FolderStop::Depth)
    );
}

#[tokio::test]
async fn the_file_limit_is_a_hard_stop() {
    struct Many;
    #[async_trait::async_trait]
    impl Fetcher for Many {
        async fn page(&self, _url: &Url) -> Result<(u16, String), SiteError> {
            Ok((
                200,
                (0..40)
                    .map(|index| format!(r#"<a href="f{index}.bin">f</a>"#))
                    .collect::<String>(),
            ))
        }
    }
    let found = crawl(
        &Url::parse("https://example.test/index/").unwrap(),
        FolderLimits {
            max_depth: 3,
            max_folders: 10,
            max_files: 15,
        },
        &Many,
    )
    .await
    .unwrap();
    assert!(
        found.len() <= 16,
        "the file limit must stop the crawl: {}",
        found.len()
    );
    assert_eq!(
        found.last().and_then(|entry| entry.stop),
        Some(FolderStop::Files)
    );
}

#[tokio::test]
async fn a_page_without_links_is_an_empty_list_and_a_404_is_not_a_crash() {
    let pages = Pages::new(&[("/index/", "<html>nothing</html>")]);
    let empty = crawl(
        &Url::parse("https://example.test/index/").unwrap(),
        limits(),
        &pages,
    )
    .await
    .unwrap();
    assert!(empty.is_empty());
    let missing = crawl(
        &Url::parse("https://example.test/missing/").unwrap(),
        limits(),
        &pages,
    )
    .await
    .unwrap();
    assert!(missing.is_empty());
}
