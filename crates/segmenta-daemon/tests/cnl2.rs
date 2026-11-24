//! Click'n'Load 2: decryption, sanitizing and the loopback listener.
//!
//! The ciphertext below was produced with `openssl enc -e -AES-128-CBC -nosalt -base64`,
//! which is the same construction sites use, so the test pins the real format.

use segmenta_daemon::cnl2::{
    decode_jk, decrypt, form_fields, host_is_local, import_from_form, import_from_plain,
    links_from_payload, parse_request, DEFAULT_PORT,
};
use std::collections::BTreeMap;

const KEY_HEX: &str = "31323334353637383930393837363534";
const CRYPTED: &str =
    "aVxLIHHorvSd8Vy9+C7p25MVR55JUuhfM25ppgYj/o7yLEGV/1mnTM5s1QDRokaT+HtQ8ocxldEhI4Xthy+jUQ==";

#[test]
fn decrypts_the_documented_construction() {
    let key = decode_jk(KEY_HEX).expect("bare hex key");
    let plain = decrypt(CRYPTED, &key).expect("decrypts");
    let text = String::from_utf8(plain).unwrap();
    assert_eq!(text, "http://127.0.0.1:1/a.bin\r\nhttp://127.0.0.1:1/b.bin");
}

#[test]
fn accepts_only_the_literal_jk_forms() {
    assert!(decode_jk("function f(){ return '31323334353637383930393837363534'; }").is_some());
    assert!(decode_jk("function f(){return \"31323334353637383930393837363534\"}").is_some());
    assert!(decode_jk(KEY_HEX).is_some());

    // Anything that would need a JavaScript engine is refused, never evaluated.
    assert!(decode_jk("function f(){ return window.location.cookie; }").is_none());
    assert!(
        decode_jk("function f(){ return '31323334353637383930393837363534'; alert(1); }").is_none()
    );
    assert!(decode_jk("function f(){ return 'zyxwvutsrqponmlk'; }").is_none());
    assert!(decode_jk("").is_none());
}

#[test]
fn keeps_only_http_links_and_drops_duplicates() {
    let payload = b"http://a.test/one\r\nftp://a.test/two\r\n\0https://a.test/three\r\nhttp://a.test/one\r\njavascript:alert(1)\r\nnot a url";
    assert_eq!(
        links_from_payload(payload),
        vec!["http://a.test/one", "https://a.test/three"]
    );
}

#[test]
fn builds_an_import_from_the_form_and_refuses_broken_input() {
    let body = format!(
        "passwords=secret&source=https%3A%2F%2Fsite.test%2Fdl&jk={}&crypted={}",
        percent_encode(&format!("function f(){{ return '{KEY_HEX}'; }}")),
        percent_encode(CRYPTED)
    );
    let fields = form_fields(body.as_bytes());
    let import = import_from_form(&fields).expect("imports");
    assert_eq!(import.links.len(), 2);
    assert_eq!(import.password.as_deref(), Some("secret"));
    assert_eq!(import.source.as_deref(), Some("https://site.test/dl"));

    let mut broken = fields.clone();
    broken.insert("crypted".to_string(), "not-base64".to_string());
    assert!(import_from_form(&broken).is_err());

    let plain = BTreeMap::from([(
        "urls".to_string(),
        "http://a.test/x\r\nhttp://a.test/y".to_string(),
    )]);
    assert_eq!(import_from_plain(&plain).unwrap().links.len(), 2);
}

#[test]
fn only_the_loopback_host_and_http1_are_accepted() {
    assert!(host_is_local("127.0.0.1:9666", DEFAULT_PORT));
    assert!(host_is_local("localhost:9666", DEFAULT_PORT));
    assert!(host_is_local("[::1]:9666", DEFAULT_PORT));
    assert!(host_is_local("127.0.0.1", DEFAULT_PORT));
    assert!(!host_is_local("evil.test:9666", DEFAULT_PORT));
    assert!(!host_is_local("127.0.0.1:1234", DEFAULT_PORT));

    let request =
        b"POST /flash/addcrypted2 HTTP/1.1\r\nHost: 127.0.0.1:9666\r\nContent-Length: 3\r\n\r\nabc";
    let parsed = parse_request(request).unwrap();
    assert_eq!(parsed.path, "/flash/addcrypted2");
    assert_eq!(parsed.body, b"abc");

    let truncated = b"POST /flash/addcrypted2 HTTP/1.1\r\nHost: 127.0.0.1:9666\r\nContent-Length: 10\r\n\r\nabc";
    assert!(parse_request(truncated).is_err());
}

fn percent_encode(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

#[tokio::test]
async fn serves_a_real_import_over_the_loopback_listener() {
    use segmenta_daemon::cnl2;
    use std::sync::Arc;

    let dir = tempfile::tempdir().unwrap();
    let store =
        Arc::new(segmenta_daemon::store::Store::open(&dir.path().join("segmenta.db")).unwrap());
    store
        .update_settings(
            serde_json::json!({
                "download_dir": dir.path().join("dl").display().to_string(),
                "allow_local_downloads": true,
            })
            .as_object()
            .unwrap(),
        )
        .unwrap();
    let secrets: Arc<dyn segmenta_daemon::SECRET_STORE_TRAIT> =
        Arc::from(segmenta_daemon::open_secret_store(dir.path(), None).unwrap());
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let manager = Arc::new(segmenta_daemon::manager::Manager::new(
        Arc::clone(&store),
        secrets,
        Arc::new(segmenta_daemon::events::EventBus::new()),
        shutdown_tx,
    ));

    let listener = cnl2::bind(0).await.expect("an ephemeral port");
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(cnl2::serve(
        listener,
        port,
        Arc::clone(&manager),
        shutdown_rx,
    ));

    let body = format!(
        "source=Bundle&jk={}&crypted={}",
        percent_encode("function f(){ return '31323334353637383930393837363534'; }"),
        percent_encode(CRYPTED)
    );
    let response = post(port, "/flash/addcrypted2", &body).await;
    assert!(response.contains("200 OK"), "{response}");
    assert!(response.contains("success"), "{response}");

    let packages = store.packages().unwrap();
    assert_eq!(packages.len(), 1);
    assert_eq!(packages[0].name, "Bundle");
    assert_eq!(packages[0].source, "cnl2");
    assert_eq!(packages[0].downloads, 2);

    // A hostile Host header is refused before anything is imported.
    let rejected = post_with_host(port, "/flash/addcrypted2", &body, "evil.test").await;
    assert!(rejected.contains("403"), "{rejected}");
    assert_eq!(store.packages().unwrap().len(), 1);

    // The port cannot be taken twice: that is the "JDownloader is running" case.
    assert!(cnl2::bind(port).await.is_err());
    drop(manager);
    server.abort();
}

async fn post(port: u16, path: &str, body: &str) -> String {
    post_with_host(port, path, body, &format!("127.0.0.1:{port}")).await
}

async fn post_with_host(port: u16, path: &str, body: &str, host: &str) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: {host}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).await.unwrap();
    response
}
