//! Debrid credentials and error translation over the real IPC surface.
//!
//! No test reaches a provider: the adapters and their payloads live in `segmenta-core`, and
//! here only the credential handling and the error mapping are exercised.

use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Daemon {
    child: Child,
    dir: tempfile::TempDir,
}

impl Daemon {
    fn start() -> Daemon {
        let dir = tempfile::tempdir().unwrap();
        let child = Command::new(env!("CARGO_BIN_EXE_segmenta-daemon"))
            .env("SEGMENTA_SOCKET", dir.path().join("daemon.sock"))
            .env("XDG_RUNTIME_DIR", dir.path())
            .env("XDG_DATA_HOME", dir.path().join("data"))
            .env("XDG_CONFIG_HOME", dir.path().join("config"))
            .env("XDG_STATE_HOME", dir.path().join("state"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let daemon = Daemon { child, dir };
        let deadline = Instant::now() + Duration::from_secs(10);
        while !daemon.socket().exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(daemon.socket().exists(), "the daemon never started");
        daemon
    }

    fn socket(&self) -> PathBuf {
        self.dir.path().join("daemon.sock")
    }

    fn request(&self, method: &str, params: Value) -> Value {
        use std::io::{BufRead, BufReader, Write};
        let mut stream = std::os::unix::net::UnixStream::connect(self.socket()).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(30)))
            .unwrap();
        writeln!(
            stream,
            "{}",
            json!({"jsonrpc":"2.0","id":1,"method":method,"params":params})
        )
        .unwrap();
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    }

    fn call(&self, method: &str, params: Value) -> Value {
        let reply = self.request(method, params);
        assert!(reply.get("error").is_none(), "{method}: {reply}");
        reply["result"].clone()
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn the_token_is_stored_but_never_returned() {
    let daemon = Daemon::start();
    let secret = "rd-super-secret-token";

    let status = daemon.call("debrid.status", json!({}));
    assert_eq!(status["provider"], "none");
    assert_eq!(status["has_token"], false);

    // The status reports the token of the provider the user selected.
    daemon.call("settings.update", json!({"debrid_provider": "realdebrid"}));

    let stored = daemon.call(
        "debrid.token.set",
        json!({"provider": "realdebrid", "token": secret}),
    );
    assert_eq!(stored["stored"], true);
    assert!(
        !stored.to_string().contains(secret),
        "the reply must not echo the token"
    );

    let status = daemon.call("debrid.status", json!({}));
    assert_eq!(status["has_token"], true);
    assert!(!status.to_string().contains(secret));

    let cleared = daemon.call("debrid.token.clear", json!({"provider": "realdebrid"}));
    assert_eq!(cleared["cleared"], true);
    assert_eq!(daemon.call("debrid.status", json!({}))["has_token"], false);

    let unknown = daemon.request(
        "debrid.token.set",
        json!({"provider": "mystery", "token": "x"}),
    );
    assert_eq!(unknown["error"]["code"], -32602);
}

#[test]
fn every_failure_maps_to_a_stable_code_without_leaking_anything() {
    let daemon = Daemon::start();

    // No provider configured.
    let none = daemon.request("debrid.resolve", json!({"link": "https://hoster.test/f"}));
    assert_eq!(none["error"]["code"], 1060, "{none}");

    // Provider configured, no token yet.
    daemon.call("settings.update", json!({"debrid_provider": "alldebrid"}));
    let no_token = daemon.request("debrid.resolve", json!({"link": "https://hoster.test/f"}));
    assert_eq!(no_token["error"]["code"], 1061, "{no_token}");

    // A foreign scheme is refused before any request leaves the daemon.
    daemon.call(
        "debrid.token.set",
        json!({"provider": "alldebrid", "token": "ad-token"}),
    );
    let bad_link = daemon.request("debrid.resolve", json!({"link": "ftp://hoster.test/f"}));
    assert_eq!(bad_link["error"]["code"], 1062, "{bad_link}");
    assert!(!bad_link.to_string().contains("ad-token"));

    // TorBox is recognized but not committed: unsupported, not a silent success.
    daemon.call("settings.update", json!({"debrid_provider": "torbox"}));
    let torbox = daemon.request("debrid.resolve", json!({"link": "https://hoster.test/f"}));
    assert_eq!(torbox["error"]["code"], 1060, "{torbox}");

    // The settings reject a provider nobody knows.
    let invalid = daemon.request("settings.update", json!({"debrid_provider": "mystery"}));
    assert!(invalid.get("error").is_some(), "{invalid}");
}
