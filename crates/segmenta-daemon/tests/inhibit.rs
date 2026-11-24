//! Sleep inhibition: one helper process, released on pause or completion.

use segmenta_daemon::inhibit::SleepInhibitor;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Write a stub `systemd-inhibit` that records its argv and then waits.
fn stub(dir: &Path) -> PathBuf {
    let path = dir.join("systemd-inhibit");
    let log = dir.join("argv");
    std::fs::write(
        &path,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{log}'\nsleep 30\n",
            log = log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// An inhibitor that only looks inside the test's own directory.
fn inhibitor_in(dir: &Path) -> SleepInhibitor {
    SleepInhibitor::with_search(vec![dir.to_path_buf()])
}

#[test]
fn holds_one_helper_and_releases_it() {
    let dir = tempfile::tempdir().unwrap();
    stub(dir.path());

    let mut inhibitor = inhibitor_in(dir.path());
    assert!(!inhibitor.is_held());
    assert!(inhibitor.acquire(), "the stub must be found");
    assert!(inhibitor.is_held());
    assert!(inhibitor.acquire(), "acquiring twice keeps the same lock");

    let argv_path = dir.path().join("argv");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !argv_path.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    let argv = std::fs::read_to_string(&argv_path).unwrap();
    assert!(argv.contains("--what=sleep"), "{argv}");
    assert!(argv.contains("--mode=block"), "{argv}");
    assert!(argv.contains("infinity"), "{argv}");

    inhibitor.release();
    assert!(!inhibitor.is_held());
    inhibitor.release();
}

#[test]
fn degrades_silently_without_the_binary() {
    let dir = tempfile::tempdir().unwrap();

    let mut inhibitor = inhibitor_in(dir.path());
    assert!(!inhibitor.acquire());
    assert!(!inhibitor.is_held());
}

#[test]
fn notices_a_helper_that_died() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("systemd-inhibit");
    std::fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();

    let mut inhibitor = inhibitor_in(dir.path());
    // A freshly written executable can briefly answer `ETXTBSY`; retry instead of flaking.
    let retry = Instant::now() + Duration::from_secs(2);
    while !inhibitor.acquire() && Instant::now() < retry {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(inhibitor.is_held());
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if inhibitor.dropped() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!inhibitor.is_held(), "a dead helper is not a held lock");
}

#[test]
fn inhibition_follows_activity_and_the_setting() {
    use segmenta_daemon::scheduler::should_inhibit;

    assert!(should_inhibit(1, true));
    assert!(
        !should_inhibit(0, true),
        "an idle daemon never blocks sleep"
    );
    assert!(!should_inhibit(3, false), "the setting must be honoured");
}
