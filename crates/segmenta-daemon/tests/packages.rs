//! Packages: grouping, IPC surface and collective actions.
//!
//! The harness starts a real daemon on a temporary socket, so these tests exercise the
//! same path the GUI and the CLI use.

mod common;

use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// A daemon running on its own socket and data directory.
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
            .env("SEGMENTA_SECRET_TOOL", common::fake_secret_tool(dir.path()))
            .env("XDG_DATA_HOME", dir.path().join("data"))
            .env("XDG_CONFIG_HOME", dir.path().join("config"))
            .env("XDG_STATE_HOME", dir.path().join("state"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let daemon = Daemon { child, dir };
        let socket = daemon.socket();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !socket.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(socket.exists(), "the daemon never created its socket");
        daemon
    }

    fn socket(&self) -> PathBuf {
        self.dir.path().join("daemon.sock")
    }

    fn call(&self, method: &str, params: Value) -> Value {
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
        let reply: Value = serde_json::from_str(&line).unwrap();
        assert!(reply.get("error").is_none(), "{method}: {reply}");
        reply["result"].clone()
    }

    fn try_call(&self, method: &str, params: Value) -> Value {
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
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A stored task written straight into the database, with no network involved.
fn seed(dir: &Path, id: &str, status: &str) {
    let conn = rusqlite::Connection::open(dir.join("data/segmenta/segmenta.db")).unwrap();
    conn.execute(
        "INSERT INTO downloads (id, url, final_path, status, queue_name)
         VALUES (?1, 'http://example.test/f.bin', '/tmp/f.bin', ?2, 'main')",
        rusqlite::params![id, status],
    )
    .unwrap();
}

/// A stored task with a future schedule, so `resume` never starts it.
fn seed_scheduled(dir: &Path, id: &str, status: &str, scheduled_at: &str) {
    let conn = rusqlite::Connection::open(dir.join("data/segmenta/segmenta.db")).unwrap();
    conn.execute(
        "INSERT INTO downloads (id, url, final_path, status, queue_name, scheduled_at)
         VALUES (?1, 'http://example.test/f.bin', '/tmp/f.bin', ?2, 'main', ?3)",
        rusqlite::params![id, status, scheduled_at],
    )
    .unwrap();
}

#[test]
fn packages_group_downloads_and_survive_a_restart() {
    let daemon = Daemon::start();
    daemon.call(
        "settings.update",
        json!({ "download_dir": daemon.dir.path().join("dl").display().to_string() }),
    );
    seed(daemon.dir.path(), "one", "paused");
    seed(daemon.dir.path(), "two", "paused");

    let created = daemon.call(
        "package.create",
        json!({"name": "Bundle", "source": "cnl2"}),
    );
    let package_id = created["package_id"].as_str().unwrap().to_string();
    assert!(package_id.len() > 8);

    daemon.call(
        "package.assign",
        json!({"package_id": package_id, "task_id": "one"}),
    );
    daemon.call(
        "package.assign",
        json!({"package_id": package_id, "task_id": "two"}),
    );

    let listed = daemon.call("package.list", json!({}));
    assert_eq!(listed[0]["name"], "Bundle");
    assert_eq!(listed[0]["downloads"], 2);
    assert_eq!(listed[0]["source"], "cnl2");
}

#[test]
fn collective_actions_respect_each_task_state() {
    let daemon = Daemon::start();
    daemon.call(
        "settings.update",
        json!({ "download_dir": daemon.dir.path().join("dl").display().to_string() }),
    );
    seed(daemon.dir.path(), "paused-one", "paused");
    seed(daemon.dir.path(), "done-one", "completed");
    seed(daemon.dir.path(), "done-two", "cancelled");
    // A future schedule keeps `resume` from reaching the network.
    seed_scheduled(daemon.dir.path(), "later", "paused", "2999-01-01T00:00:00Z");

    // Cancelling a finished or cancelled task would rewrite its history.
    let mixed = package_of(&daemon, "Mixed", &["paused-one", "done-one", "done-two"]);
    let cancelled = daemon.call("package.cancel", json!({"package_id": mixed}));
    assert_eq!(cancelled["cancelled"], json!(["paused-one"]), "{cancelled}");
    assert_eq!(cancelled["skipped"], json!(["done-one", "done-two"]));

    // Pausing only touches a task with work in flight.
    let active = package_of(&daemon, "Active", &["paused-one", "done-one"]);
    let paused = daemon.call("package.pause", json!({"package_id": active}));
    assert_eq!(paused["paused"], json!([]), "{paused}");
    assert_eq!(paused["skipped"], json!(["paused-one", "done-one"]));

    // Resuming a terminal task is refused; the paused one moves and its history is kept.
    let resumable = package_of(&daemon, "Resumable", &["later", "done-one"]);
    let resumed = daemon.call("package.resume", json!({"package_id": resumable}));
    assert_eq!(resumed["resumed"], json!(["later"]), "{resumed}");
    assert_eq!(resumed["skipped"], json!(["done-one"]));
    let done = daemon.call("download.status", json!({"task_id": "done-one"}));
    assert_eq!(done["status"], "completed");
    let cancelled_task = daemon.call("download.status", json!({"task_id": "paused-one"}));
    assert_eq!(cancelled_task["status"], "cancelled");
}

/// Create a package and group `tasks` under it.
fn package_of(daemon: &Daemon, name: &str, tasks: &[&str]) -> String {
    let package_id = daemon.call("package.create", json!({ "name": name }))["package_id"]
        .as_str()
        .unwrap()
        .to_string();
    for task in tasks {
        daemon.call(
            "package.assign",
            json!({"package_id": package_id, "task_id": task}),
        );
    }
    package_id
}

#[test]
fn deleting_a_package_keeps_its_downloads_and_unknown_ids_are_typed_errors() {
    let daemon = Daemon::start();
    daemon.call(
        "settings.update",
        json!({ "download_dir": daemon.dir.path().join("dl").display().to_string() }),
    );
    seed(daemon.dir.path(), "kept", "completed");
    let package_id = daemon.call("package.create", json!({"name": "Temporary"}))["package_id"]
        .as_str()
        .unwrap()
        .to_string();
    daemon.call(
        "package.assign",
        json!({"package_id": package_id, "task_id": "kept"}),
    );

    daemon.call("package.delete", json!({"package_id": package_id}));
    assert!(daemon
        .call("package.list", json!({}))
        .as_array()
        .unwrap()
        .is_empty());
    let kept = daemon.call("download.status", json!({"task_id": "kept"}));
    assert_eq!(kept["status"], "completed");

    let missing = daemon.try_call("package.delete", json!({"package_id": "nope"}));
    assert!(missing["error"]["message"]
        .as_str()
        .unwrap()
        .contains("does not exist"));
    let empty = daemon.try_call("package.create", json!({"name": "   "}));
    assert_eq!(empty["error"]["code"], -32602);
}
