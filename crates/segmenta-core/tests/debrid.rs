// ---------------------------------------------------------------------------
// T1 of the third milestone: TorBox plus account metadata for every provider.
// ---------------------------------------------------------------------------

use segmenta_core::debrid::{
    account_request, parse_account, resolve_with, AccountInfo, DebridHttp, Provider,
};
use std::sync::Mutex;

/// Canned debrid API: records requests and answers by URL path.
struct Canned {
    answers: std::collections::HashMap<String, (u16, String)>,
    seen: Mutex<Vec<(String, String)>>,
}

impl Canned {
    fn new(answers: &[(&str, u16, &str)]) -> Canned {
        Canned {
            answers: answers
                .iter()
                .map(|(path, status, body)| (path.to_string(), (*status, body.to_string())))
                .collect(),
            seen: Mutex::new(Vec::new()),
        }
    }

    fn seen(&self) -> Vec<(String, String)> {
        self.seen.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl DebridHttp for Canned {
    async fn send(
        &self,
        request: &segmenta_core::debrid::DebridRequest,
    ) -> Result<(u16, String), segmenta_core::debrid::DebridError> {
        let path = request
            .url
            .split("://")
            .nth(1)
            .and_then(|rest| rest.find('/').map(|index| rest[index..].to_string()))
            .unwrap_or_default();
        let base = path.split('?').next().unwrap_or_default().to_string();
        self.seen
            .lock()
            .unwrap()
            .push((request.method.to_string(), request.url.clone()));
        Ok(self
            .answers
            .get(&base)
            .cloned()
            .unwrap_or((404, String::new())))
    }
}

#[tokio::test]
async fn torbox_creates_a_web_download_then_asks_for_its_link() {
    let http = Canned::new(&[
        (
            "/v1/api/webdl/createwebdownload",
            200,
            r#"{"success":true,"data":{"webdownload_id":42,"hash":"abc"}}"#,
        ),
        (
            "/v1/api/webdl/requestdl",
            200,
            r#"{"success":true,"data":"https://store1.torbox.app/file.zip"}"#,
        ),
    ]);
    let link = resolve_with(
        &http,
        Provider::TorBox,
        "token",
        "https://hoster.example/file.zip",
    )
    .await
    .unwrap();
    assert_eq!(link, "https://store1.torbox.app/file.zip");
    let seen = http.seen();
    assert_eq!(seen.len(), 2, "{seen:?}");
    // The second call carries the id of the download the first one created.
    assert!(seen[1].1.contains("web_id=42"), "{seen:?}");
    // And the token never appears in the URL: it travels in the header.
    assert!(!seen[0].1.contains("token"), "{seen:?}");
}

#[tokio::test]
async fn a_torbox_creation_that_fails_names_the_reason() {
    let http = Canned::new(&[(
        "/v1/api/webdl/createwebdownload",
        200,
        r#"{"success":false,"error":"LINK_UNSUPPORTED"}"#,
    )]);
    let refused = resolve_with(&http, Provider::TorBox, "token", "https://x.example/a").await;
    assert_eq!(
        refused.unwrap_err(),
        segmenta_core::debrid::DebridError::LinkUnavailable
    );
}

#[tokio::test]
async fn real_debrid_still_resolves_in_one_call() {
    let http = Canned::new(&[(
        "/rest/1.0/unrestrict/link",
        200,
        r#"{"download":"https://cdn.example/direct.zip"}"#,
    )]);
    let link = resolve_with(&http, Provider::RealDebrid, "token", "https://x.example/a")
        .await
        .unwrap();
    assert_eq!(link, "https://cdn.example/direct.zip");
    assert_eq!(http.seen().len(), 1);
}

#[test]
fn every_provider_describes_its_account_request() {
    for provider in [Provider::RealDebrid, Provider::AllDebrid, Provider::TorBox] {
        let request = account_request(provider, "token").unwrap();
        assert!(request.url.starts_with("https://"), "{provider:?}");
        assert!(
            request
                .headers
                .iter()
                .any(|(name, value)| name == "Authorization" && value == "Bearer token"),
            "{provider:?}"
        );
    }
    assert_eq!(
        account_request(Provider::TorBox, "  ").unwrap_err(),
        segmenta_core::debrid::DebridError::Unauthorized
    );
}

#[test]
fn account_metadata_is_read_for_every_provider() {
    let real = parse_account(
        Provider::RealDebrid,
        200,
        r#"{"username":"alice","type":"premium","expiration":"2026-01-01T00:00:00Z"}"#,
    )
    .unwrap();
    assert_eq!(real.username.as_deref(), Some("alice"));
    assert!(real.is_premium);
    assert_eq!(real.expires_at.as_deref(), Some("2026-01-01T00:00:00Z"));

    let all = parse_account(
        Provider::AllDebrid,
        200,
        r#"{"status":"success","user":{"username":"bob","isPremium":true,"premiumUntil":1234567890}}"#,
    )
    .unwrap();
    assert_eq!(all.username.as_deref(), Some("bob"));
    assert!(all.is_premium);

    // TorBox reports the plan as a number: 0 is free, anything else is paid.
    let torbox = parse_account(
        Provider::TorBox,
        200,
        r#"{"success":true,"data":{"email":"c@example.test","plan":2,"premium_expires_at":"2026-05-05T00:00:00Z"}}"#,
    )
    .unwrap();
    assert_eq!(torbox.username.as_deref(), Some("c@example.test"));
    assert!(torbox.is_premium);
    let free = parse_account(
        Provider::TorBox,
        200,
        r#"{"success":true,"data":{"email":"d@example.test","plan":0}}"#,
    )
    .unwrap();
    assert!(!free.is_premium);

    assert_eq!(
        parse_account(Provider::TorBox, 401, "{}").unwrap_err(),
        segmenta_core::debrid::DebridError::Unauthorized
    );
    assert_eq!(
        AccountInfo::default(),
        AccountInfo {
            username: None,
            is_premium: false,
            expires_at: None
        }
    );
}

#[tokio::test]
async fn premiumize_resolves_through_directdl_and_keeps_the_key_out_of_the_url() {
    let http = Canned::new(&[(
        "/api/transfer/directdl",
        200,
        r#"{"status":"success","content":[{"link":"https://cdn.premiumize.me/f.zip"}]}"#,
    )]);
    let link = resolve_with(
        &http,
        Provider::Premiumize,
        "key",
        "https://hoster.example/f.zip",
    )
    .await
    .unwrap();
    assert_eq!(link, "https://cdn.premiumize.me/f.zip");
    let seen = http.seen();
    assert_eq!(seen.len(), 1);
    // The key travels in the header, never in the query: its own documentation recommends
    // exactly this so the credential does not end up in logs or a Referer.
    assert!(!seen[0].1.contains("key="), "{seen:?}");
    assert!(!seen[0].1.contains("apikey"), "{seen:?}");
}

#[tokio::test]
async fn premiumize_reports_an_empty_content_list_as_unavailable() {
    let http = Canned::new(&[(
        "/api/transfer/directdl",
        200,
        r#"{"status":"success","content":[]}"#,
    )]);
    let refused = resolve_with(
        &http,
        Provider::Premiumize,
        "key",
        "https://hoster.example/f.zip",
    )
    .await;
    assert_eq!(
        refused.unwrap_err(),
        segmenta_core::debrid::DebridError::LinkUnavailable
    );
}

#[test]
fn premiumize_account_metadata_reads_the_premium_until_epoch() {
    let paid = parse_account(
        Provider::Premiumize,
        200,
        r#"{"status":"success","customer_id":"42","premium_until":1900000000}"#,
    )
    .unwrap();
    assert_eq!(paid.username.as_deref(), Some("42"));
    assert!(paid.is_premium);
    assert_eq!(paid.expires_at.as_deref(), Some("1900000000"));

    let free = parse_account(
        Provider::Premiumize,
        200,
        r#"{"status":"success","customer_id":"43","premium_until":0}"#,
    )
    .unwrap();
    assert!(!free.is_premium, "a zero epoch is not a subscription");
}

#[test]
fn a_request_that_carries_a_secret_in_its_url_redacts_it_when_printed() {
    // Mega-Debrid puts its session token in the query by design, so the Debug render must
    // not become the leak that the header rule was protecting against.
    let request = segmenta_core::debrid::DebridRequest {
        method: "GET",
        url: "https://www.mega-debrid.eu/api.php?action=getLink&token=SECRET&link=x".to_string(),
        headers: Vec::new(),
        body: String::new(),
        credential_in_url: true,
    };
    let printed = format!("{request:?}");
    assert!(!printed.contains("SECRET"), "{printed}");
    assert!(printed.contains("mega-debrid"), "{printed}");
}
