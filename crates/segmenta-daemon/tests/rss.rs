//! RSS feeds: filters, deduplication and survival across restarts.

use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct FeedServer {
    child: Child,
    port: u16,
    dir: tempfile::TempDir,
}

impl FeedServer {
    /// Serve `body` on a free loopback port.
    fn start(body: &str) -> FeedServer {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("feed.xml"), body).unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let mut child = Command::new("python3")
            .arg("-m")
            .arg("http.server")
            .arg(port.to_string())
            .arg("--bind")
            .arg("127.0.0.1")
            .arg("--directory")
            .arg(dir.path())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        // Wait until it answers; on failure the child is reaped before the panic, so the
        // suite never leaves a zombie behind.
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
                return FeedServer { child, port, dir };
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = child.kill();
        let _ = child.wait();
        panic!("the feed server never started");
    }

    fn url(&self) -> String {
        format!("http://127.0.0.1:{}/feed.xml", self.port)
    }
}

impl Drop for FeedServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = self.dir.path();
    }
}

struct Daemon {
    child: Child,
    base: PathBuf,
}

impl Daemon {
    /// Start one daemon over `base`; two calls with the same base share the database.
    fn start(base: &std::path::Path) -> Daemon {
        // A previous run leaves its socket file behind; waiting on it would race ahead of
        // the new daemon actually binding.
        let _ = std::fs::remove_file(base.join("daemon.sock"));
        let child = Command::new(env!("CARGO_BIN_EXE_segmenta-daemon"))
            .env("SEGMENTA_SOCKET", base.join("daemon.sock"))
            .env("XDG_RUNTIME_DIR", base)
            .env("XDG_DATA_HOME", base.join("data"))
            .env("XDG_CONFIG_HOME", base.join("config"))
            .env("XDG_STATE_HOME", base.join("state"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let daemon = Daemon {
            child,
            base: base.to_path_buf(),
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        while !daemon.socket().exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(daemon.socket().exists(), "the daemon never started");
        daemon
    }

    fn fresh() -> Daemon {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().to_path_buf();
        std::mem::forget(dir);
        let daemon = Daemon::start(&base);
        daemon.call(
            "settings.update",
            json!({
                "download_dir": daemon.base.join("dl").display().to_string(),
                "allow_local_downloads": true
            }),
        );
        daemon
    }

    fn socket(&self) -> PathBuf {
        self.base.join("daemon.sock")
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

fn feed_body() -> String {
    format!(
        r#"<?xml version="1.0"?>
<rss version="2.0"><channel>
  <title>Releases</title>
  <item><title>Tool 1080p</title><guid>g-1</guid><link>http://127.0.0.1:{port}/one.bin</link></item>
  <item><title>Tool 720p</title><guid>g-2</guid><link>http://127.0.0.1:{port}/two.bin</link></item>
</channel></rss>"#,
        port = 9
    )
}

#[test]
fn imports_once_filters_and_survives_a_restart() {
    let server = FeedServer::start(&feed_body());
    let daemon = Daemon::fresh();
    let feed = daemon.call(
        "rss.feed.add",
        json!({ "url": server.url(), "name": "Releases", "filter": "1080p" }),
    );
    assert!(feed["feed_id"].as_str().is_some());

    let first = daemon.call("rss.check_now", json!({}));
    assert_eq!(first["checked"], 1, "{first}");
    // Only the item that matches the filter is imported.
    assert_eq!(first["added"], 1, "{first}");

    // A second poll finds the same GUID and adds nothing.
    let second = daemon.call("rss.check_now", json!({}));
    assert_eq!(second["added"], 0, "{second}");

    let feeds = daemon.call("rss.feed.list", json!({}));
    assert_eq!(feeds[0]["name"], "Releases");
    assert_eq!(feeds[0]["filter"], "1080p");
    assert!(feeds[0]["last_checked_at"].as_i64().is_some());
}

#[test]
fn a_restart_does_not_duplicate_what_was_already_imported() {
    let server = FeedServer::start(&feed_body());
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().to_path_buf();
    {
        let daemon = Daemon::start(&base);
        daemon.call(
            "settings.update",
            json!({
                "download_dir": base.join("dl").display().to_string(),
                "allow_local_downloads": true
            }),
        );
        daemon.call("rss.feed.add", json!({ "url": server.url() }));
        let first = daemon.call("rss.check_now", json!({}));
        assert_eq!(first["added"], 2, "{first}");
    }
    // The same base directory: feeds and GUIDs must come back from the database.
    let reopened = Daemon::start(&base);
    let result = reopened.request("rss.check_now", json!({}));
    assert_eq!(
        result["result"]["added"], 0,
        "a restart must not reimport: {result}"
    );
}

#[test]
fn an_unsafe_or_unknown_feed_is_rejected() {
    let daemon = Daemon::fresh();
    let refused = daemon.request("rss.feed.add", json!({ "url": "file:///etc/passwd" }));
    assert_eq!(refused["error"]["code"], 1085, "{refused}");
    let unknown = daemon.request("rss.feed.remove", json!({ "feed_id": "nope" }));
    assert!(unknown.get("error").is_some());
}
