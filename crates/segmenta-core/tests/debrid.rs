// ---------------------------------------------------------------------------
// T1 of the third milestone: TorBox plus account metadata for every provider.
// ---------------------------------------------------------------------------

use segmenta_core::debrid::{
    account_request, device_poll_request, device_start_request, next_interval, parse_account,
    parse_device_start, parse_poll, parse_token_exchange, resolve_with, token_exchange_request,
    AccountInfo, DebridHttp, PollOutcome, Provider, MAX_POLL_INTERVAL_SECS,
};
use std::sync::Mutex;

/// Canned debrid API: records requests and answers by URL path, in order.
struct Canned {
    answers: Mutex<std::collections::HashMap<String, std::collections::VecDeque<(u16, String)>>>,
    seen: Mutex<Vec<(String, String)>>,
}

impl Canned {
    fn new(answers: &[(&str, u16, &str)]) -> Canned {
        let mut map: std::collections::HashMap<String, std::collections::VecDeque<(u16, String)>> =
            std::collections::HashMap::new();
        for (path, status, body) in answers {
            map.entry(path.to_string())
                .or_default()
                .push_back((*status, body.to_string()));
        }
        Canned {
            answers: Mutex::new(map),
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
        // Answers are consumed in order: Mega-Debrid answers the login and the link request
        // on the same path, so a single answer per path would hide the second step.
        let mut answers = self.answers.lock().unwrap();
        match answers.get_mut(&base) {
            Some(queue) if queue.len() > 1 => Ok(queue.pop_front().unwrap()),
            Some(queue) => Ok(queue.front().cloned().unwrap_or((404, String::new()))),
            None => Ok((404, String::new())),
        }
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

#[tokio::test]
async fn mega_debrid_logs_in_with_credentials_then_asks_for_the_link() {
    let http = Canned::new(&[
        (
            "/api.php",
            200,
            r#"{"response_code":"ok","token":"session-token","vip":"1900000000"}"#,
        ),
        (
            "/api.php",
            200,
            r#"{"response_code":"ok","link":"https://dl.mega-debrid.eu/f.zip"}"#,
        ),
    ]);
    let link = resolve_with(
        &http,
        Provider::MegaDebrid,
        "user:pass",
        "https://hoster.example/f.zip",
    )
    .await
    .unwrap();
    assert_eq!(link, "https://dl.mega-debrid.eu/f.zip");
    let seen = http.seen();
    assert_eq!(seen.len(), 2, "{seen:?}");
    // The session token travels in the query — that is the provider's design — so the
    // request must be marked as carrying a credential there.
    assert!(seen[1].1.contains("token=session-token"), "{seen:?}");
    assert!(!seen[0].1.contains("password"), "{seen:?}");
}

#[tokio::test]
async fn a_free_mega_debrid_account_is_not_reported_as_a_bad_token() {
    // `vip_end` means the account cannot use the API at all. Telling the user their token is
    // wrong sends them to look at the wrong thing.
    let http = Canned::new(&[(
        "/api.php",
        200,
        r#"{"response_code":"vip_end","response_text":"only premium"}"#,
    )]);
    let refused = resolve_with(
        &http,
        Provider::MegaDebrid,
        "user:pass",
        "https://hoster.example/f.zip",
    )
    .await;
    assert_eq!(
        refused.unwrap_err(),
        segmenta_core::debrid::DebridError::AccountNotPremium
    );
}

#[tokio::test]
async fn mega_debrid_credentials_without_a_password_are_refused_before_any_request() {
    let http = Canned::new(&[]);
    let refused = resolve_with(
        &http,
        Provider::MegaDebrid,
        "onlyuser",
        "https://x.example/a",
    )
    .await;
    assert_eq!(
        refused.unwrap_err(),
        segmenta_core::debrid::DebridError::Unauthorized
    );
    assert!(http.seen().is_empty());
}

#[test]
fn mega_debrid_account_reads_the_vip_epoch() {
    let paid = parse_account(
        Provider::MegaDebrid,
        200,
        r#"{"response_code":"ok","token":"t","vip":"1900000000"}"#,
    )
    .unwrap();
    assert!(paid.is_premium);
    assert_eq!(paid.expires_at.as_deref(), Some("1900000000"));
    let free = parse_account(
        Provider::MegaDebrid,
        200,
        r#"{"response_code":"vip_end","response_text":"only premium"}"#,
    )
    .unwrap_err();
    assert_eq!(free, segmenta_core::debrid::DebridError::AccountNotPremium);
}

#[test]
fn a_device_flow_start_carries_the_code_the_user_must_type() {
    let request = device_start_request(Provider::RealDebrid, "client-1").unwrap();
    assert!(
        request.url.contains("/oauth/v2/device/code"),
        "{}",
        request.url
    );
    assert!(request.url.contains("client_id=client-1"));
    assert!(request.url.contains("new_credentials=yes"));

    let flow = parse_device_start(
        Provider::RealDebrid,
        200,
        r#"{"device_code":"dev","user_code":"ABCD","verification_url":"https://real-debrid.com/device","interval":5,"expires_in":1800}"#,
    )
    .unwrap();
    assert_eq!(flow.device_code, "dev");
    assert_eq!(flow.user_code, "ABCD");
    assert_eq!(flow.verification_url, "https://real-debrid.com/device");
    assert_eq!(flow.interval_secs, 5);
    assert_eq!(flow.expires_in_secs, 1800);
}

#[test]
fn only_providers_with_a_real_device_flow_offer_one() {
    // AllDebrid and TorBox authenticate with an API key, and Premiumize needs a registered
    // OAuth client: inventing a flow for them would fail on the first call.
    for provider in [Provider::AllDebrid, Provider::TorBox, Provider::MegaDebrid] {
        assert!(
            device_start_request(provider, "client-1").is_err(),
            "{provider:?}"
        );
    }
    assert!(device_start_request(Provider::Premiumize, "client-1").is_err());
    // An empty client id is a configuration mistake, not a reason to call the provider.
    assert!(device_start_request(Provider::RealDebrid, "  ").is_err());
}

#[test]
fn polling_reports_pending_denied_and_granted_without_guessing() {
    let flow = parse_device_start(
        Provider::RealDebrid,
        200,
        r#"{"device_code":"dev","user_code":"ABCD","verification_url":"https://x/device","interval":5,"expires_in":900}"#,
    )
    .unwrap();
    let request = device_poll_request(&flow, "client-1").unwrap();
    assert!(request.url.contains("device_code=dev"), "{}", request.url);

    // 403 is what the provider answers while the user has not authorized yet.
    assert_eq!(
        parse_poll(Provider::RealDebrid, 403, "").unwrap(),
        PollOutcome::Pending
    );
    // `slow_down` asks for a longer interval instead of an immediate retry.
    assert_eq!(
        parse_poll(
            Provider::RealDebrid,
            400,
            r#"{"error":"slow_down","error_code":16}"#
        )
        .unwrap(),
        PollOutcome::SlowDown
    );
    assert_eq!(
        parse_poll(
            Provider::RealDebrid,
            400,
            r#"{"error":"access_denied","error_code":17}"#
        )
        .unwrap(),
        PollOutcome::Denied
    );
    assert_eq!(
        parse_poll(
            Provider::RealDebrid,
            400,
            r#"{"error":"expired_token","error_code":18}"#
        )
        .unwrap(),
        PollOutcome::Expired
    );
    // A granted poll hands over the per-user credentials, not the token yet.
    let granted = parse_poll(
        Provider::RealDebrid,
        200,
        r#"{"client_id":"user-client","client_secret":"user-secret"}"#,
    )
    .unwrap();
    assert!(matches!(granted, PollOutcome::Granted { .. }));
    // The secret must never render.
    assert!(!format!("{granted:?}").contains("user-secret"));
}

#[test]
fn the_next_poll_interval_respects_slow_down_and_the_cap() {
    assert_eq!(next_interval(5, false), 5);
    assert_eq!(next_interval(5, true), 10);
    assert_eq!(next_interval(50, true), MAX_POLL_INTERVAL_SECS);
}

#[test]
fn the_authorized_pair_is_exchanged_for_an_access_token() {
    let request = token_exchange_request("client-1", "secret-1", "dev-code").unwrap();
    assert!(request.url.ends_with("/oauth/v2/token"), "{}", request.url);
    assert!(request.body.contains("grant_type=http"), "{}", request.body);
    assert!(request.body.contains("code=dev-code"), "{}", request.body);
    // Both halves of the pair travel in the body, never in the URL.
    assert!(!request.url.contains("secret-1"));
    let printed = format!("{request:?}");
    assert!(!printed.contains("secret-1"), "{printed}");

    let token = parse_token_exchange(
        200,
        r#"{"access_token":"the-token","refresh_token":"r","expires_in":3600,"token_type":"Bearer"}"#,
    )
    .unwrap();
    assert_eq!(token, "the-token");
    assert_eq!(
        parse_token_exchange(200, r#"{"expires_in":3600}"#).unwrap_err(),
        segmenta_core::debrid::DebridError::Malformed
    );
    assert_eq!(
        parse_token_exchange(401, "{}").unwrap_err(),
        segmenta_core::debrid::DebridError::Unauthorized
    );
}
