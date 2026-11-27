//! VPN kill switch: a configured interface is enforced before anything is dialed.

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
        let deadline = Instant::now() + Duration::from_secs(10);
        while !daemon.socket().exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(daemon.socket().exists(), "the daemon never started");
        daemon.call(
            "settings.update",
            json!({
                "download_dir": daemon.dir.path().join("dl").display().to_string(),
                "allow_local_downloads": true
            }),
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
fn a_missing_interface_blocks_the_download_before_it_is_dialed() {
    let daemon = Daemon::start();
    daemon.call("settings.update", json!({"vpn_interface": "vpnx0"}));

    let refused = daemon.request(
        "download.add",
        json!({"url": "http://127.0.0.1:1/file.bin"}),
    );
    assert_eq!(refused["error"]["code"], 1081, "{refused}");
    assert!(refused["error"]["message"]
        .as_str()
        .unwrap()
        .contains("does not exist"));
    // Fail-closed: no task was created, so nothing can retry through the default route.
    assert!(daemon
        .call("download.list", json!({}))
        .as_array()
        .unwrap()
        .is_empty());
}

#[test]
fn an_invalid_name_is_rejected_by_the_settings() {
    let daemon = Daemon::start();
    let invalid = daemon.request("settings.update", json!({"vpn_interface": "../etc"}));
    assert!(invalid.get("error").is_some(), "{invalid}");
}

#[test]
fn a_torrent_under_a_dead_interface_is_refused_before_it_starts() {
    let daemon = Daemon::start();
    daemon.call("settings.update", json!({"vpn_interface": "vpnx0"}));
    let refused = daemon.request(
        "torrent.add",
        json!({"url": "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567"}),
    );
    // 1081: the interface is missing, so no session is built at all.
    assert_eq!(refused["error"]["code"], 1081, "{refused}");
}

#[test]
fn a_chrome_tls_profile_is_refused_by_a_build_without_impersonation() {
    let daemon = Daemon::start();
    let refused = daemon.request("settings.update", json!({"tls_profile": "chrome"}));
    let message = refused["error"]["message"].as_str().unwrap_or_default();
    assert!(message.contains("impersonate"), "{refused}");

    // The default profile keeps working and a typo is still rejected.
    daemon.call("settings.update", json!({"tls_profile": "standard"}));
    assert!(daemon
        .request("settings.update", json!({"tls_profile": "netscape"}))
        .get("error")
        .is_some());
}

#[test]
fn the_daemon_reports_the_interface_state_for_the_window() {
    let daemon = Daemon::start();
    let none = daemon.call("vpn.status", json!({}));
    assert_eq!(none["state"], "none");
    assert_eq!(none["configured"], false);

    // `lo` exists on every Linux host that runs these tests.
    daemon.call("settings.update", json!({"vpn_interface": "lo"}));
    let active = daemon.call("vpn.status", json!({}));
    assert_eq!(active["state"], "active");
    assert_eq!(active["interface"], "lo");

    daemon.call("settings.update", json!({"vpn_interface": "vpnx0"}));
    let missing = daemon.call("vpn.status", json!({}));
    assert_eq!(missing["state"], "missing");
    assert_eq!(missing["configured"], true);
}

#[test]
fn torrent_sessions_bind_to_the_interface_and_hide_from_the_lan() {
    // The effective options are what librqbit will apply to DHT, BT-UDP, BT-TCP, trackers
    // and LSD; this test pins the three decisions T3 depends on.
    let plain = segmenta_daemon::torrent::session_options(None);
    assert_eq!(plain.bind_device_name, None);
    // Without a VPN the options are untouched, whatever librqbit's own defaults are.
    assert!(!plain.disable_local_service_discovery);

    let bound = segmenta_daemon::torrent::session_options(Some("wg0"));
    assert_eq!(bound.bind_device_name.as_deref(), Some("wg0"));
    assert!(
        bound.disable_local_service_discovery,
        "LSD multicast must not escape the tunnel"
    );
    assert!(
        bound.listen.is_none(),
        "no inbound listener and no UPnP request under a VPN"
    );

    // An empty setting means "no VPN", not "bind to the empty name".
    let empty = segmenta_daemon::torrent::session_options(Some("   "));
    assert_eq!(empty.bind_device_name, None);
}
