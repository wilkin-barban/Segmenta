//! Post-processing hook contract (A1: OpenSpec 12 §4 and §9).
//!
//! Every hook is a real script written into a tempdir and executed by path: no test goes
//! through a shell, no test depends on the network and no test asserts on wall-clock time
//! beyond a generous upper bound.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use segmenta_core::testserver::{RangeServer, ServerConfig};
use segmenta_daemon::events::EventBus;
use segmenta_daemon::ipc::RpcHandler;
use segmenta_daemon::manager::Manager;
use segmenta_daemon::postprocess::{
    expand_argument, run, HookExit, HookOptions, HookRun, HookTarget, DEFAULT_CAPTURE_LIMIT,
    DEFAULT_TIMEOUT_SECS,
};
use segmenta_daemon::secrets::{open_with, SecretStore};
use segmenta_daemon::store::{DownloadRecord, DownloadStatus, PostprocessRecord, Settings, Store};
use tokio::sync::watch;

/// A hook that records the argv it received (after the file it must write to) in the
/// file named by its first argument, one argument per line.
fn recording_hook(dir: &Path) -> PathBuf {
    script(
        dir,
        "record.sh",
        "out=$1\nshift\nprintf '%s\\n' \"$@\" > \"$out\"",
    )
}

fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    // Written under a private name and renamed into place: a `fork` of a concurrent test
    // would otherwise inherit the open write descriptor and make `execve` fail with
    // ETXTBSY.
    let staging = dir.join(format!(".{name}.staging"));
    std::fs::write(&staging, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&staging, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::rename(&staging, &path).unwrap();
    path
}

/// A hook invocation whose argv is the hook, the record file and the arguments under
/// test. The record file travels as argv so no path is interpolated into shell source.
fn command(hook: &Path, args_file: &Path, args: &[&str]) -> Vec<String> {
    let mut command = vec![hook.display().to_string(), args_file.display().to_string()];
    command.extend(args.iter().map(|argument| (*argument).to_string()));
    command
}

fn target_file(dir: &Path, name: &str) -> HookTarget {
    let file = dir.join(name);
    std::fs::write(&file, b"payload").unwrap();
    HookTarget::from_file(&file)
}

fn recorded(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect()
}

fn outcome_of(run: HookRun) -> segmenta_daemon::postprocess::HookOutcome {
    match run {
        HookRun::Ran(outcome) => outcome,
        HookRun::Skipped => panic!("the configured hook must run"),
    }
}

#[test]
fn no_hook_configured_skips_the_phase() {
    let dir = tempfile::tempdir().unwrap();
    let target = target_file(dir.path(), "file.bin");
    assert_eq!(
        run(&[], &target, &HookOptions::default()),
        HookRun::Skipped,
        "an empty command must skip post-processing instead of running anything"
    );
}

#[test]
fn placeholders_are_substituted_exactly() {
    let dir = tempfile::tempdir().unwrap();
    let args_file = dir.path().join("args.txt");
    let hook = recording_hook(dir.path());
    let target = target_file(dir.path(), "archivo final.bin");
    let file = target.file.display().to_string();
    let folder = target.folder.display().to_string();
    let argv = command(
        &hook,
        &args_file,
        &["--input=%file%", "%folder%", "%name%", "%name%.part"],
    );

    let outcome = outcome_of(run(&argv, &target, &HookOptions::default()));

    assert_eq!(outcome.exit, HookExit::Code(0), "{}", outcome.message);
    assert_eq!(
        recorded(&args_file),
        vec![
            format!("--input={file}"),
            folder,
            "archivo final.bin".to_string(),
            "archivo final.bin.part".to_string(),
        ]
    );
    assert_eq!(outcome.exit.code(), Some(0));
    assert!(outcome.exit.succeeded());
}

#[test]
fn unknown_placeholders_stay_literal() {
    let dir = tempfile::tempdir().unwrap();
    let target = target_file(dir.path(), "file.bin");

    for unknown in ["%unknown%", "%FILE%", "%foldr%x%asid%", "%%", "%folder"] {
        assert_eq!(
            expand_argument(unknown, &target),
            unknown,
            "{unknown} must reach the hook untouched"
        );
    }
    assert_eq!(
        expand_argument("%name%%unknown%", &target),
        "file.bin%unknown%"
    );
}

#[test]
fn a_path_with_spaces_stays_one_argument() {
    let dir = tempfile::tempdir().unwrap();
    let args_file = dir.path().join("args.txt");
    let hook = recording_hook(dir.path());
    let target = target_file(dir.path(), "con espacios y  dobles.bin");
    let argv = command(&hook, &args_file, &["%file%"]);

    let outcome = outcome_of(run(&argv, &target, &HookOptions::default()));

    assert_eq!(outcome.exit, HookExit::Code(0), "{}", outcome.message);
    assert_eq!(
        recorded(&args_file),
        vec![target.file.display().to_string()]
    );
}

#[test]
fn a_file_name_with_shell_syntax_is_never_interpreted() {
    let dir = tempfile::tempdir().unwrap();
    let args_file = dir.path().join("args.txt");
    let hook = recording_hook(dir.path());
    // `$(...)`, backticks and `;` are ordinary bytes in a file name: none of them may
    // reach a shell, and none of the side effects below may happen.
    let target = target_file(dir.path(), "a$(touch pwned)`touch pwned2`;touch pwned3.bin");
    let argv = command(&hook, &args_file, &["%file%", "%name%"]);

    let outcome = outcome_of(run(&argv, &target, &HookOptions::default()));

    assert_eq!(outcome.exit, HookExit::Code(0), "{}", outcome.message);
    assert_eq!(
        recorded(&args_file),
        vec![
            target.file.display().to_string(),
            "a$(touch pwned)`touch pwned2`;touch pwned3.bin".to_string(),
        ]
    );
    for side_effect in ["pwned", "pwned2", "pwned3"] {
        assert!(
            !dir.path().join(side_effect).exists(),
            "`{side_effect}` exists: something ran a shell"
        );
    }
}

#[test]
fn a_hanging_hook_is_killed_by_the_timeout() {
    let dir = tempfile::tempdir().unwrap();
    let hook = script(dir.path(), "hang.sh", "sleep 30");
    let target = target_file(dir.path(), "file.bin");
    let options = HookOptions {
        timeout: Duration::from_millis(300),
        ..HookOptions::default()
    };

    let started = Instant::now();
    let outcome = outcome_of(run(&[hook.display().to_string()], &target, &options));
    let elapsed = started.elapsed();

    assert_eq!(outcome.exit, HookExit::Timeout);
    assert_eq!(outcome.exit.code(), None);
    assert!(!outcome.exit.succeeded());
    assert!(
        elapsed < Duration::from_secs(10),
        "the hook was not killed in bounded time: {elapsed:?}"
    );
    assert!(
        outcome.message.to_lowercase().contains("timeout"),
        "the record must explain the kill: {}",
        outcome.message
    );
    assert_eq!(
        DEFAULT_TIMEOUT_SECS, 30,
        "the documented constant is the fallback timeout"
    );
}

#[test]
fn captured_output_is_bounded() {
    let dir = tempfile::tempdir().unwrap();
    // ~20 KiB on each stream, well above the capture limit.
    let hook = script(
        dir.path(),
        "flood.sh",
        "i=0\nwhile [ \"$i\" -lt 200 ]; do printf '%0100d' 0; printf '%0100d' 0 >&2; i=$((i+1)); done",
    );
    let target = target_file(dir.path(), "file.bin");

    let outcome = outcome_of(run(
        &[hook.display().to_string()],
        &target,
        &HookOptions::default(),
    ));

    assert_eq!(outcome.exit, HookExit::Code(0), "{}", outcome.message);
    assert!(
        outcome.stdout.len() <= DEFAULT_CAPTURE_LIMIT,
        "stdout is unbounded: {} bytes",
        outcome.stdout.len()
    );
    assert!(
        outcome.stderr.len() <= DEFAULT_CAPTURE_LIMIT,
        "stderr is unbounded: {} bytes",
        outcome.stderr.len()
    );
    assert!(!outcome.stdout.is_empty(), "stdout is captured");
}

#[test]
fn a_non_zero_exit_is_recorded_without_failing_the_task() {
    let dir = tempfile::tempdir().unwrap();
    let hook = script(
        dir.path(),
        "fail.sh",
        "echo 'boom: converter missing' >&2\nexit 3",
    );
    let target = target_file(dir.path(), "file.bin");
    let options = HookOptions::default();

    let outcome = outcome_of(run(&[hook.display().to_string()], &target, &options));

    assert_eq!(outcome.exit, HookExit::Code(3));
    assert_eq!(outcome.exit.code(), Some(3));
    assert!(!outcome.exit.succeeded());
    assert!(
        outcome.message.contains('3') && outcome.message.contains("boom"),
        "the record must carry the code and the message: {}",
        outcome.message
    );
    // Decision of OpenSpec 12 §4: a non-zero hook exit is a warning, not a failure,
    // unless the user opted into failing the task.
    let run = HookRun::Ran(outcome.clone());
    assert!(!run.fails_task(false), "the download stays completed");
    assert!(run.fails_task(true), "an opted-in hook may fail the task");
    assert!(
        target.file.exists(),
        "a hook failure never removes the downloaded file"
    );

    // The record is retained (in memory while schema v4 owns the columns).
    let store = Store::open(&dir.path().join("segmenta.db")).unwrap();
    store.record_postprocess(
        "task-1",
        PostprocessRecord {
            exit_code: outcome.exit.code(),
            message: outcome.message.clone(),
        },
    );
    let stored = store.postprocess("task-1").expect("a record");
    assert_eq!(stored.exit_code, Some(3));
    assert_eq!(stored.message, outcome.message);
    assert!(store.postprocess("task-2").is_none());
}

#[test]
fn a_missing_hook_binary_is_recorded() {
    let dir = tempfile::tempdir().unwrap();
    let target = target_file(dir.path(), "file.bin");
    let missing = dir.path().join("no-such-hook");

    let outcome = outcome_of(run(
        &[missing.display().to_string(), "%file%".to_string()],
        &target,
        &HookOptions::default(),
    ));

    match &outcome.exit {
        HookExit::Spawn { binary, .. } => assert_eq!(binary, &missing.display().to_string()),
        other => panic!("a missing binary must be recorded as a spawn failure: {other:?}"),
    }
    assert_eq!(outcome.exit.code(), None);
    assert!(
        outcome.message.contains("no-such-hook"),
        "the record must name the binary: {}",
        outcome.message
    );
    assert!(!HookRun::Ran(outcome).fails_task(false));
}

struct Harness {
    manager: Manager,
    store: std::sync::Arc<Store>,
    _shutdown: watch::Receiver<bool>,
}

fn harness(dir: &Path) -> Harness {
    let store = std::sync::Arc::new(Store::open(&dir.join("segmenta.db")).unwrap());
    let secrets: std::sync::Arc<dyn SecretStore> =
        std::sync::Arc::from(open_with(dir, None).unwrap());
    let bus = std::sync::Arc::new(EventBus::new());
    let (shutdown_tx, shutdown) = watch::channel(false);
    let manager = Manager::new(std::sync::Arc::clone(&store), secrets, bus, shutdown_tx);
    Harness {
        manager,
        store,
        _shutdown: shutdown,
    }
}

fn record(id: &str, url: &str, dir: &Path) -> DownloadRecord {
    DownloadRecord {
        id: id.to_string(),
        url: url.to_string(),
        final_url: None,
        final_path: dir.join(format!("{id}.bin")).display().to_string(),
        temp_path: None,
        total_bytes: None,
        downloaded_bytes: 0,
        status: DownloadStatus::Pending,
        supports_ranges: true,
        etag: None,
        last_modified: None,
        content_type: None,
        expected_checksum: None,
        error_code: None,
        error_message: None,
        retry_count: 0,
        priority: 0,
        queue_name: "main".to_string(),
        download_type: "direct".to_string(),
        created_at: None,
        completed_at: None,
        filename: format!("{id}.bin"),
        media_kind: None,
        media_format: None,
        playlist_title: None,
        playlist_count: None,
        batch_id: None,
        quarantine_path: None,
        postprocess_exit: None,
        postprocess_message: None,
    }
}

async fn configure(h: &Harness, dir: &Path, command: Vec<String>) {
    let settings = Settings {
        allow_local_downloads: true,
        download_dir: dir.display().to_string(),
        max_connections: 2,
        postprocess_command: command,
        ..Settings::default()
    };
    h.manager
        .call(
            "settings.update",
            Some(serde_json::to_value(&settings).unwrap()),
        )
        .await
        .unwrap();
}

async fn wait_terminal(h: &Harness, id: &str) -> DownloadStatus {
    let mut status = h.store.get(id).unwrap().unwrap().status;
    for _ in 0..300 {
        if matches!(
            status,
            DownloadStatus::Completed | DownloadStatus::Failed | DownloadStatus::Cancelled
        ) {
            return status;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        status = h.store.get(id).unwrap().unwrap().status;
    }
    status
}

#[tokio::test]
async fn the_manager_runs_the_hook_before_completing_the_task() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(dir.path());
    let server = RangeServer::start(ServerConfig {
        body: vec![7u8; 64 * 1024],
        ..ServerConfig::default()
    })
    .await;
    let args_file = dir.path().join("args.txt");
    let hook = recording_hook(dir.path());
    configure(
        &h,
        dir.path(),
        command(&hook, &args_file, &["%file%", "%name%"]),
    )
    .await;
    let task = record("hooked", server.url().as_str(), dir.path());
    let final_path = task.final_path.clone();
    h.store.insert_download(&task).unwrap();

    assert_eq!(h.manager.start_pending_for_queue("main").unwrap(), 1);
    assert_eq!(wait_terminal(&h, "hooked").await, DownloadStatus::Completed);

    assert_eq!(
        recorded(&args_file),
        vec![final_path.clone(), "hooked.bin".to_string(),],
        "the hook saw the final path of the completed download"
    );
    let stored = h.store.postprocess("hooked").expect("a record");
    assert_eq!(stored.exit_code, Some(0));
    assert!(Path::new(&final_path).exists(), "the file is in place");
    server.shutdown().await;
}

#[tokio::test]
async fn a_failing_hook_leaves_the_download_completed_and_the_file_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(dir.path());
    let server = RangeServer::start(ServerConfig {
        body: vec![9u8; 32 * 1024],
        ..ServerConfig::default()
    })
    .await;
    let hook = script(dir.path(), "fail.sh", "echo 'no encoder' >&2\nexit 1");
    configure(
        &h,
        dir.path(),
        vec![hook.display().to_string(), "%file%".to_string()],
    )
    .await;
    let task = record("warning", server.url().as_str(), dir.path());
    let final_path = task.final_path.clone();
    h.store.insert_download(&task).unwrap();

    assert_eq!(h.manager.start_pending_for_queue("main").unwrap(), 1);
    assert_eq!(
        wait_terminal(&h, "warning").await,
        DownloadStatus::Completed
    );

    let stored = h.store.postprocess("warning").expect("a record");
    assert_eq!(stored.exit_code, Some(1));
    assert!(stored.message.contains("no encoder"), "{}", stored.message);
    let record = h.store.get("warning").unwrap().unwrap();
    assert_eq!(record.error_code, None, "the task keeps no error");
    assert!(Path::new(&final_path).exists(), "the file is in place");
    let bytes = std::fs::read(&final_path).unwrap();
    assert_eq!(bytes.len(), 32 * 1024);
    server.shutdown().await;
}

/// The engine path is process-wide, so the antivirus tests take turns instead of racing
/// on `SEGMENTA_ANTIVIRUS_PATH`.
static ANTIVIRUS_ENV: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Fake `clamscan`: exit 1 when the file contains the EICAR marker, 0 otherwise.
fn fake_clamscan(dir: &Path) -> PathBuf {
    let path = dir.join("clamscan");
    std::fs::write(
        &path,
        "#!/bin/sh\nlast=\"\"\nfor arg in \"$@\"; do last=\"$arg\"; done\n\
         if grep -q 'EICAR-STANDARD-ANTIVIRUS-TEST-FILE' \"$last\"; then\n\
         \tprintf '%s: Eicar-Test-Signature FOUND\\n' \"$last\"\n\texit 1\nfi\n\
         printf '%s: OK\\n' \"$last\"\nexit 0\n",
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

async fn configure_antivirus(
    h: &Harness,
    dir: &Path,
    quarantine: &Path,
    engine_dir: Option<&Path>,
) {
    let settings = Settings {
        allow_local_downloads: true,
        download_dir: dir.display().to_string(),
        max_connections: 2,
        antivirus_enabled: true,
        antivirus_timeout_secs: 30,
        antivirus_quarantine_dir: quarantine.display().to_string(),
        ..Settings::default()
    };
    h.manager
        .call(
            "settings.update",
            Some(serde_json::to_value(&settings).unwrap()),
        )
        .await
        .unwrap();
    // The scan searches only where the test says: no real ClamAV is ever involved.
    match engine_dir {
        Some(path) => std::env::set_var("SEGMENTA_ANTIVIRUS_PATH", path),
        None => std::env::set_var("SEGMENTA_ANTIVIRUS_PATH", dir.join("no-engine")),
    }
}

/// The Ubuntu QA scenario: antivirus on, no hook configured, an infected file.
#[tokio::test]
async fn an_infected_download_fails_with_1046_and_leaves_downloads() {
    let _guard = ANTIVIRUS_ENV.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let h = harness(dir.path());
    let downloads = dir.path().join("Descargas");
    let quarantine = dir.path().join("cuarentena");
    std::fs::create_dir_all(&downloads).unwrap();
    let engine_dir = dir.path().join("bin");
    std::fs::create_dir_all(&engine_dir).unwrap();
    fake_clamscan(&engine_dir);

    let body = b"X5O!P%@AP[4\\PZX54(P^)7CC)7}$EICAR-STANDARD-ANTIVIRUS-TEST-FILE!$H+H*".to_vec();
    let server = RangeServer::start(ServerConfig {
        body,
        ..ServerConfig::default()
    })
    .await;
    configure_antivirus(&h, &downloads, &quarantine, Some(&engine_dir)).await;

    let task = record("infected", server.url().as_str(), &downloads);
    h.store.insert_download(&task).unwrap();
    assert_eq!(h.manager.start_pending_for_queue("main").unwrap(), 1);

    assert_eq!(wait_terminal(&h, "infected").await, DownloadStatus::Failed);
    let stored = h.store.get("infected").unwrap().unwrap();
    assert_eq!(
        stored.error_code,
        Some(1046),
        "the code names the infection"
    );
    let quarantined = stored.quarantine_path.expect("the quarantine path");
    assert!(
        std::path::Path::new(&quarantined).exists(),
        "the file is in quarantine: {quarantined}"
    );
    assert!(
        !std::path::Path::new(&task.final_path).exists(),
        "the infected file must not stay in the downloads folder"
    );
    server.shutdown().await;
    std::env::remove_var("SEGMENTA_ANTIVIRUS_PATH");
}

#[tokio::test]
async fn a_clean_download_completes_with_the_scanner_running() {
    let _guard = ANTIVIRUS_ENV.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let h = harness(dir.path());
    let downloads = dir.path().join("Descargas");
    let quarantine = dir.path().join("cuarentena");
    std::fs::create_dir_all(&downloads).unwrap();
    let engine_dir = dir.path().join("bin");
    std::fs::create_dir_all(&engine_dir).unwrap();
    fake_clamscan(&engine_dir);
    let server = RangeServer::start(ServerConfig {
        body: vec![3u8; 32 * 1024],
        ..ServerConfig::default()
    })
    .await;
    configure_antivirus(&h, &downloads, &quarantine, Some(&engine_dir)).await;

    let task = record("clean", server.url().as_str(), &downloads);
    h.store.insert_download(&task).unwrap();
    assert_eq!(h.manager.start_pending_for_queue("main").unwrap(), 1);

    assert_eq!(wait_terminal(&h, "clean").await, DownloadStatus::Completed);
    let stored = h.store.get("clean").unwrap().unwrap();
    assert_eq!(stored.error_code, None);
    assert!(std::path::Path::new(&task.final_path).exists());
    server.shutdown().await;
    std::env::remove_var("SEGMENTA_ANTIVIRUS_PATH");
}

#[tokio::test]
async fn antivirus_without_an_engine_fails_with_1047() {
    let _guard = ANTIVIRUS_ENV.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let h = harness(dir.path());
    let downloads = dir.path().join("Descargas");
    let quarantine = dir.path().join("cuarentena");
    std::fs::create_dir_all(&downloads).unwrap();
    let server = RangeServer::start(ServerConfig {
        body: vec![1u8; 8 * 1024],
        ..ServerConfig::default()
    })
    .await;
    configure_antivirus(&h, &downloads, &quarantine, None).await;

    let task = record("no-engine", server.url().as_str(), &downloads);
    h.store.insert_download(&task).unwrap();
    assert_eq!(h.manager.start_pending_for_queue("main").unwrap(), 1);

    assert_eq!(wait_terminal(&h, "no-engine").await, DownloadStatus::Failed);
    let stored = h.store.get("no-engine").unwrap().unwrap();
    assert_eq!(
        stored.error_code,
        Some(1047),
        "a missing engine is not an infection"
    );
    assert_eq!(stored.quarantine_path, None);
    server.shutdown().await;
    std::env::remove_var("SEGMENTA_ANTIVIRUS_PATH");
}
