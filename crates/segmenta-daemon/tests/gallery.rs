//! Gallery imports through `gallery-dl`.
//!
//! The extractor is a stub here: the daemon is started with `SEGMENTA_GALLERY_PATH`
//! pointing at a directory that holds a script, so no network and no real extractor are
//! needed to check the contract.

mod common;

use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Daemon {
    child: Child,
    dir: tempfile::TempDir,
}

impl Daemon {
    fn start(tool: Option<&str>) -> Daemon {
        let dir = tempfile::tempdir().unwrap();
        let tools = dir.path().join("tools");
        std::fs::create_dir(&tools).unwrap();
        if let Some(body) = tool {
            use std::os::unix::fs::PermissionsExt;
            let path = tools.join("gallery-dl");
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let downloads = dir.path().join("dl");
        std::fs::create_dir(&downloads).unwrap();
        let child = Command::new(env!("CARGO_BIN_EXE_segmenta-daemon"))
            .env("SEGMENTA_SOCKET", dir.path().join("daemon.sock"))
            .env("SEGMENTA_GALLERY_PATH", &tools)
            .env("XDG_RUNTIME_DIR", dir.path())
            .env("SEGMENTA_SECRET_TOOL", common::fake_secret_tool(dir.path()))
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
        daemon.call(
            "settings.update",
            json!({ "download_dir": daemon.dir.path().join("dl").display().to_string() }),
        );
        daemon
    }

    fn socket(&self) -> PathBuf {
        self.dir.path().join("daemon.sock")
    }

    fn request(&self, method: &str, params: Value) -> Value {
        use std::io::{BufRead, BufReader, Write};
        let mut stream = std::os::unix::net::UnixStream::connect(self.socket()).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(60)))
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

/// The stub writes two files into the `--dest` directory it was given.
const WRITING_TOOL: &str = r#"
dest='.'
last=''
for arg in "$@"; do
  if [ "$last" = '-dest' ] || [ "$last" = '--dest' ]; then dest="$arg"; fi
  last="$arg"
done
mkdir -p "$dest/gallery"
printf 'one' > "$dest/gallery/one.jpg"
printf 'two' > "$dest/gallery/two.jpg"
"#;

#[test]
fn groups_the_extracted_files_in_one_package() {
    let daemon = Daemon::start(Some(WRITING_TOOL));
    let status = daemon.call("gallery.status", json!({}));
    assert_eq!(status["available"], true);

    let added = daemon.call("gallery.add", json!({"url": "https://site.test/album/1"}));
    assert_eq!(added["files"], 2);
    let package_id = added["package_id"].as_str().unwrap().to_string();
    assert_eq!(added["name"], "site.test");

    let packages = daemon.call("package.list", json!({}));
    assert_eq!(packages[0]["package_id"], package_id.as_str());
    assert_eq!(packages[0]["downloads"], 2);
    assert_eq!(packages[0]["source"], "gallery");

    let list = daemon.call("download.list", json!({"limit": 10}));
    let tasks = list.as_array().unwrap();
    assert_eq!(tasks.len(), 2);
    for task in tasks {
        assert_eq!(task["status"], "completed");
    }
}

#[test]
fn a_missing_extractor_degrades_without_touching_the_queue() {
    let daemon = Daemon::start(None);
    assert_eq!(daemon.call("gallery.status", json!({}))["available"], false);

    let refused = daemon.request("gallery.add", json!({"url": "https://site.test/album"}));
    assert_eq!(refused["error"]["code"], 1070, "{refused}");
    assert!(daemon
        .call("package.list", json!({}))
        .as_array()
        .unwrap()
        .is_empty());
    assert!(daemon
        .call("download.list", json!({}))
        .as_array()
        .unwrap()
        .is_empty());
}

#[test]
fn a_failing_or_empty_extractor_reports_its_own_code() {
    let failing = Daemon::start(Some("exit 3"));
    let failed = failing.request("gallery.add", json!({"url": "https://site.test/a"}));
    assert_eq!(failed["error"]["code"], 1072, "{failed}");

    let empty = Daemon::start(Some("exit 0"));
    let nothing = empty.request("gallery.add", json!({"url": "https://site.test/a"}));
    assert_eq!(nothing["error"]["code"], 1071, "{nothing}");

    // A url that is not http never reaches the extractor.
    let bad = empty.request("gallery.add", json!({"url": "file:///etc"}));
    assert_eq!(bad["error"]["code"], 1072, "{bad}");
}
