//! End-to-end CLI tests against a real daemon binary and a local range server.
//!
//! Each test gets its own XDG_DATA_HOME/XDG_RUNTIME_DIR/HOME and its own socket, so the
//! developer's real daemon is never touched. No external network.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;
use segmenta_core::testserver::{RangeServer, ServerConfig};

const MB: usize = 1024 * 1024;

fn binary(name: &str) -> PathBuf {
    let exe = std::env::current_exe().expect("test binary path");
    let dir = exe.parent().expect("deps dir").parent().expect("target dir");
    let candidate = dir.join(name);
    assert!(
        candidate.exists(),
        "{name} is not built at {}; run `cargo build --workspace` or `cargo test --workspace`",
        candidate.display()
    );
    candidate
}

struct Sandbox {
    root: tempfile::TempDir,
}

impl Sandbox {
    fn new() -> Sandbox {
        let root = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(root.path().join("data")).unwrap();
        std::fs::create_dir_all(root.path().join("run")).unwrap();
        std::fs::create_dir_all(root.path().join("home")).unwrap();
        std::fs::create_dir_all(root.path().join("downloads")).unwrap();
        // The runtime dir must be private for the socket.
        use std::os::unix::fs::PermissionsExt;
        let run = root.path().join("run");
        let mut permissions = std::fs::metadata(&run).unwrap().permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&run, permissions).unwrap();
        Sandbox { root }
    }

    fn socket(&self) -> PathBuf {
        self.root.path().join("run").join("daemon.sock")
    }

    fn downloads(&self) -> PathBuf {
        self.root.path().join("downloads")
    }

    fn data_dir(&self) -> PathBuf {
        self.root.path().join("data").join("segmenta")
    }

    fn daemon(&self) -> Daemon {
        Daemon::start(self)
    }

    fn cli(&self, args: &[&str]) -> Output {
        let output = Command::new(binary("segmenta-cli"))
            .env("XDG_DATA_HOME", self.root.path().join("data"))
            .env("XDG_RUNTIME_DIR", self.root.path().join("run"))
            .env("HOME", self.root.path().join("home"))
            .env("SEGMENTA_SOCKET", self.socket())
            .args(["--json"])
            .args(args)
            .output()
            .expect("run segmenta-cli");
        Output {
            code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    }

    fn cli_one(&self, args: &[&str]) -> Output {
        let output = Command::new(binary("segmenta-cli"))
            .env("XDG_DATA_HOME", self.root.path().join("data"))
            .env("XDG_RUNTIME_DIR", self.root.path().join("run"))
            .env("HOME", self.root.path().join("home"))
            .env("SEGMENTA_SOCKET", self.socket())
            .args(args)
            .output()
            .expect("run segmenta-cli");
        Output {
            code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    }

    /// Wait until `check` returns something, or panic with the daemon log.
    fn wait<T>(&self, label: &str, timeout: Duration, mut check: impl FnMut() -> Option<T>) -> T {
        let started = Instant::now();
        loop {
            if let Some(value) = check() {
                return value;
            }
            if started.elapsed() > timeout {
                panic!("timed out waiting for {label}");
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    fn list(&self) -> Vec<Value> {
        let output = self.cli(&["list", "--filter", "all"]);
        assert_eq!(output.code, 0, "list failed: {}", output.stderr);
        serde_json::from_str::<Vec<Value>>(output.stdout.trim())
            .unwrap_or_else(|err| panic!("list is not JSON: {err}: {}", output.stdout))
    }

    fn task(&self, task_id: &str) -> Value {
        let output = self.cli(&["status", task_id]);
        assert_eq!(output.code, 0, "status failed: {}", output.stderr);
        serde_json::from_str::<Value>(output.stdout.trim()).expect("status JSON")
    }

    fn configure(&self) {
        let downloads = self.downloads();
        let output = self.cli_one(&[
            "settings",
            "set",
            "allow_local_downloads",
            "true",
            "download_dir",
            downloads.to_str().unwrap(),
            "max_connections",
            "4",
            "max_retries",
            "2",
        ]);
        assert_eq!(output.code, 0, "settings failed: {}", output.stderr);
    }
}

struct Output {
    code: i32,
    stdout: String,
    stderr: String,
}

struct Daemon {
    child: Child,
    log: PathBuf,
}

impl Daemon {
    fn start(sandbox: &Sandbox) -> Daemon {
        let log = sandbox.root.path().join("daemon.log");
        let file = std::fs::File::create(&log).unwrap();
        let child = Command::new(binary("segmenta-daemon"))
            .env("XDG_DATA_HOME", sandbox.root.path().join("data"))
            .env("XDG_RUNTIME_DIR", sandbox.root.path().join("run"))
            .env("HOME", sandbox.root.path().join("home"))
            .args([
                "--socket",
                sandbox.socket().to_str().unwrap(),
                "--data-dir",
                sandbox.data_dir().to_str().unwrap(),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::from(file))
            .spawn()
            .expect("spawn segmenta-daemon");
        let daemon = Daemon { child, log };
        sandbox.wait("the daemon socket", Duration::from_secs(10), || {
            sandbox.socket().exists().then_some(())
        });
        // system.hello proves the socket accepts and the peer check passes.
        sandbox.wait("system.hello", Duration::from_secs(10), || {
            let output = sandbox.cli_one(&["hello"]);
            (output.code == 0).then_some(())
        });
        daemon
    }

    fn kill_hard(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    fn stop(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if self.child.try_wait().is_err() {
            eprintln!("daemon log kept at {}", self.log.display());
        }
    }
}

fn payload(size: usize) -> Vec<u8> {
    (0..size).map(|i| (i % 251) as u8).collect()
}

fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(bytes))
}

fn slow_server(body: Vec<u8>, chunk: usize, delay_ms: u64) -> RangeServer {
    // The server is started on a dedicated runtime thread: the CLI under test is a
    // separate process, and the test itself stays synchronous.
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async move {
            let server = RangeServer::start(ServerConfig {
                body,
                chunk_size: chunk,
                slow: Some(Duration::from_millis(delay_ms)),
                ..ServerConfig::default()
            })
            .await;
            tx.send(server).expect("hand the server to the test");
            // Keep the runtime alive for the whole test; the sender side is dropped when
            // the test ends and closes the channel.
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        });
    });
    rx.recv().expect("server started")
}

#[test]
fn settings_round_trip_and_validation_errors_exit_one() {
    let sandbox = Sandbox::new();
    let _daemon = sandbox.daemon();

    let output = sandbox.cli(&["settings", "get"]);
    assert_eq!(output.code, 0, "{}", output.stderr);
    let settings: Value = serde_json::from_str(output.stdout.trim()).unwrap();
    assert_eq!(settings["max_connections"], 16);
    assert_eq!(settings["proxy_mode"], "system");

    sandbox.configure();
    let settings: Value =
        serde_json::from_str(sandbox.cli(&["settings", "get"]).stdout.trim()).unwrap();
    assert_eq!(settings["max_connections"], 4);
    assert_eq!(settings["allow_local_downloads"], true);

    // Out of range and unknown keys are rejected with a usage-level failure.
    let bad = sandbox.cli_one(&["settings", "set", "max_connections", "99"]);
    assert_eq!(bad.code, 1, "stdout: {}", bad.stdout);
    assert!(bad.stderr.contains("max_connections"), "stderr: {}", bad.stderr);

    let unknown = sandbox.cli_one(&["settings", "set", "nonsense", "1"]);
    assert_eq!(unknown.code, 1);
    assert!(unknown.stderr.contains("unknown setting"), "{}", unknown.stderr);
}

#[test]
fn add_pause_resume_and_cancel_round_trip() {
    let sandbox = Sandbox::new();
    let _daemon = sandbox.daemon();
    sandbox.configure();

    let body = payload(4 * MB);
    let expected = sha256(&body);
    let server = slow_server(body.clone(), 8 * 1024, 2);

    let output = sandbox.cli(&["add", server.url().as_str()]);
    assert_eq!(output.code, 0, "add failed: {}", output.stderr);
    let added: Value = serde_json::from_str(output.stdout.trim()).unwrap();
    let task_id = added["task_id"].as_str().expect("task id").to_string();

    // Wait for real progress, then pause.
    sandbox.wait("progress", Duration::from_secs(20), || {
        let task = sandbox.task(&task_id);
        (task["downloaded_bytes"].as_u64().unwrap_or(0) > 0).then_some(())
    });
    let paused = sandbox.cli_one(&["pause", &task_id]);
    assert_eq!(paused.code, 0, "pause failed: {}", paused.stderr);
    let task = sandbox.task(&task_id);
    assert_eq!(task["status"], "paused");
    let paused_bytes = task["downloaded_bytes"].as_u64().unwrap_or(0);
    assert!(paused_bytes > 0);

    let resumed = sandbox.cli_one(&["resume", &task_id]);
    assert_eq!(resumed.code, 0, "resume failed: {}", resumed.stderr);

    let task = sandbox.wait("completion", Duration::from_secs(60), || {
        let task = sandbox.task(&task_id);
        (task["status"] == "completed").then_some(task)
    });
    assert_eq!(task["total_bytes"].as_u64(), Some((4 * MB) as u64));
    let path = Path::new(task["final_path"].as_str().unwrap());
    let downloaded = std::fs::read(path).expect("downloaded file");
    assert_eq!(downloaded.len(), 4 * MB);
    assert_eq!(sha256(&downloaded), expected, "the file must match the source");

    // Segments are contiguous and cover the whole file.
    let segments = task["segments"].as_array().expect("segments");
    assert!(!segments.is_empty());
    let mut expected_start = 0u64;
    for segment in segments {
        assert_eq!(segment["start_offset"].as_u64(), Some(expected_start));
        let end = segment["end_offset"].as_u64().unwrap();
        assert_eq!(segment["next_offset"].as_u64(), Some(end + 1));
        expected_start = end + 1;
    }
    assert_eq!(expected_start, (4 * MB) as u64);

    let listed = sandbox.list();
    assert_eq!(listed.len(), 1, "list must show the completed task");

    // A second add with the same name must not overwrite the first file.
    let output = sandbox.cli(&["add", server.url().as_str()]);
    assert_eq!(output.code, 0);
    let added: Value = serde_json::from_str(output.stdout.trim()).unwrap();
    let second_id = added["task_id"].as_str().unwrap().to_string();
    sandbox.wait("second completion", Duration::from_secs(60), || {
        let task = sandbox.task(&second_id);
        (task["status"] == "completed").then_some(())
    });
    let second = sandbox.task(&second_id);
    assert_ne!(
        second["final_path"].as_str().unwrap(),
        task["final_path"].as_str().unwrap(),
        "the collision policy must pick a new name"
    );

    // Cancel with file deletion.
    let output = sandbox.cli(&["add", server.url().as_str(), "--connections", "1"]);
    let added: Value = serde_json::from_str(output.stdout.trim()).unwrap();
    let third_id = added["task_id"].as_str().unwrap().to_string();
    sandbox.wait("third progress", Duration::from_secs(20), || {
        let task = sandbox.task(&third_id);
        (task["downloaded_bytes"].as_u64().unwrap_or(0) > 0).then_some(())
    });
    let cancelled = sandbox.cli_one(&["cancel", &third_id, "--delete-file"]);
    assert_eq!(cancelled.code, 0, "cancel failed: {}", cancelled.stderr);
    let task = sandbox.task(&third_id);
    assert_eq!(task["status"], "cancelled");
    assert!(
        !Path::new(task["final_path"].as_str().unwrap()).exists(),
        "cancel --delete-file must remove the partial file"
    );
}

#[test]
fn a_ten_megabyte_download_survives_a_daemon_kill_and_matches_its_checksum() {
    // The release acceptance test: segmented download, pause by killing the daemon,
    // resume from the checkpoint and byte-identical output.
    let sandbox = Sandbox::new();
    let mut daemon = sandbox.daemon();
    sandbox.configure();

    let body = payload(10 * MB);
    let expected = sha256(&body);
    let server = slow_server(body, 16 * 1024, 3);

    let output = sandbox.cli(&["add", server.url().as_str()]);
    assert_eq!(output.code, 0, "add failed: {}", output.stderr);
    let added: Value = serde_json::from_str(output.stdout.trim()).unwrap();
    let task_id = added["task_id"].as_str().unwrap().to_string();

    // Let a checkpoint or two land, then kill the daemon without any shutdown protocol.
    sandbox.wait("a few megabytes written", Duration::from_secs(30), || {
        let task = sandbox.task(&task_id);
        (task["downloaded_bytes"].as_u64().unwrap_or(0) >= 2 * MB as u64).then_some(())
    });
    daemon.kill_hard();

    // Restart: the in-flight task must be reconciled to paused, with its bytes kept.
    let _daemon = sandbox.daemon();
    let task = sandbox.wait("the task after restart", Duration::from_secs(10), || {
        let listed = sandbox.list();
        let task = listed.into_iter().find(|row| row["task_id"] == task_id.as_str());
        task.filter(|task| task["status"] == "paused")
    });
    let recovered_bytes = task["downloaded_bytes"].as_u64().unwrap_or(0);
    assert!(
        recovered_bytes > 0,
        "the checkpoint must survive the kill, got {recovered_bytes}"
    );
    assert!(
        Path::new(task["final_path"].as_str().unwrap()).exists(),
        "the partial file must still exist"
    );

    let resumed = sandbox.cli_one(&["resume", &task_id]);
    assert_eq!(resumed.code, 0, "resume failed: {}", resumed.stderr);

    let task = sandbox.wait("completion", Duration::from_secs(120), || {
        let task = sandbox.task(&task_id);
        match task["status"].as_str() {
            Some("completed") => Some(task),
            Some("failed") => panic!("the download failed: {}", task["error_message"]),
            _ => None,
        }
    });
    let path = Path::new(task["final_path"].as_str().unwrap());
    let downloaded = std::fs::read(path).expect("downloaded file");
    assert_eq!(downloaded.len(), 10 * MB);
    assert_eq!(
        sha256(&downloaded),
        expected,
        "resumed bytes must not be corrupted"
    );
    let status = sandbox.cli_one(&["status", &task_id]);
    assert_eq!(status.code, 0);
    assert!(status.stdout.contains("completed"));
}

#[test]
fn an_unknown_task_reports_code_1001() {
    let sandbox = Sandbox::new();
    let _daemon = sandbox.daemon();
    let output = sandbox.cli_one(&["status", "00000000-0000-0000-0000-000000000000"]);
    assert_eq!(output.code, 1, "stdout: {}", output.stdout);
    assert!(
        output.stderr.contains("1001") || output.stderr.to_lowercase().contains("no task"),
        "stderr: {}",
        output.stderr
    );
}

#[test]
fn a_private_target_is_refused_unless_local_downloads_are_enabled() {
    let sandbox = Sandbox::new();
    let _daemon = sandbox.daemon();
    // No configure(): allow_local_downloads stays false.
    let body = payload(64 * 1024);
    let server = slow_server(body, 8 * 1024, 1);
    let output = sandbox.cli(&["add", server.url().as_str()]);
    assert_eq!(output.code, 0, "{}", output.stderr);
    let added: Value = serde_json::from_str(output.stdout.trim()).unwrap();
    let task_id = added["task_id"].as_str().unwrap().to_string();
    let task = sandbox.wait("failure", Duration::from_secs(20), || {
        let task = sandbox.task(&task_id);
        (task["status"] == "failed").then_some(task)
    });
    assert_eq!(task["error_code"].as_i64(), Some(1007));
}
