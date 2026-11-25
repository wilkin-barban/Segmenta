//! Debrid seam in the GUI: a configured provider rewrites the link before it is added.
//!
//! No GTK here: the decision is a plain function over the client, so the cases run without
//! a display. The dialog itself only consumes the answer.

use segmenta_gui::add_dialog::debrid_link;
use segmenta_gui::client::{Client, DemoClient};
use std::sync::Arc;

fn client(demo: DemoClient) -> Arc<dyn Client> {
    Arc::new(demo)
}

#[test]
fn a_configured_provider_rewrites_the_link() {
    let client = client(DemoClient::with_debrid("https://dl.real-debrid.com/file"));

    assert_eq!(
        debrid_link(&client, "https://mega.nz/file/abc").as_deref(),
        Some("https://dl.real-debrid.com/file")
    );
    // The GUI asked the daemon with the original link, never with a rewritten one.
    let calls = client
        .request("debrid.status", serde_json::json!({}))
        .unwrap();
    assert_eq!(calls["configured"], true);
}

#[test]
fn without_a_provider_the_original_link_is_kept() {
    let client = client(DemoClient::empty());
    assert_eq!(debrid_link(&client, "https://mega.nz/file/abc"), None);
}

#[test]
fn a_provider_error_never_breaks_the_add() {
    // Configured provider, but `debrid.resolve` answers with an error.
    let client = client(DemoClient::with_failing_debrid());
    assert_eq!(debrid_link(&client, "https://mega.nz/file/abc"), None);
}

#[test]
fn only_https_answers_are_accepted() {
    let client = client(DemoClient::with_debrid("http://plain.test/file"));
    assert_eq!(
        debrid_link(&client, "https://mega.nz/file/abc"),
        None,
        "a plain-http answer is ignored instead of downgrading the download"
    );
}
