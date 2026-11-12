//! Native queue scheduler contract (S1: OpenSpec 12 §2 and §9).
//!
//! No test waits on a real window or on the host clock: the wall clock is injected
//! through [`Clock`] and tokio's clock stays paused. No test suspends or powers off the
//! machine: the host actions are recorded by a fake.

use std::path::Path;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use segmenta_core::testserver::{RangeServer, ServerConfig};
use segmenta_daemon::events::EventBus;
use segmenta_daemon::ipc::RpcHandler;
use segmenta_daemon::manager::Manager;
use segmenta_daemon::scheduler::{
    window_from_bounds, Clock, HostActions, QueueScheduler, TICK_INTERVAL,
};
use segmenta_daemon::secrets::{open_with, SecretStore};
use segmenta_daemon::store::{DownloadRecord, DownloadStatus, Settings, Store};
use serde_json::{json, Value};
use tokio::sync::watch;

/// Noon, the minute the tests use when they want a `22:00–06:00` window closed.
const NOON: u32 = 12 * 60;
/// 23:00, the minute the tests use when they want that same window open.
const LATE: u32 = 23 * 60;

/// Injectable wall clock: the minute of the day a window is evaluated against.
struct TestClock(AtomicU32);

impl TestClock {
    fn new(minute: u32) -> TestClock {
        TestClock(AtomicU32::new(minute))
    }

    fn set(&self, minute: u32) {
        self.0.store(minute, Ordering::SeqCst);
    }
}

impl Clock for TestClock {
    fn minute_of_day(&self) -> u32 {
        self.0.load(Ordering::SeqCst)
    }
}

/// Host actions the tests record instead of running.
#[derive(Default)]
struct RecordingActions {
    suspend: AtomicUsize,
    power_off: AtomicUsize,
}

impl RecordingActions {
    fn suspends(&self) -> usize {
        self.suspend.load(Ordering::SeqCst)
    }

    fn power_offs(&self) -> usize {
        self.power_off.load(Ordering::SeqCst)
    }
}

impl HostActions for RecordingActions {
    fn suspend(&self) -> Result<(), String> {
        self.suspend.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn power_off(&self) -> Result<(), String> {
        self.power_off.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

struct Harness {
    manager: Arc<Manager>,
    store: Arc<Store>,
    clock: Arc<TestClock>,
    actions: Arc<RecordingActions>,
    shutdown: watch::Receiver<bool>,
}

fn harness(dir: &Path) -> Harness {
    let store = Arc::new(Store::open(&dir.join("segmenta.db")).unwrap());
    let secrets: Arc<dyn SecretStore> = Arc::from(open_with(dir, None).unwrap());
    let bus = Arc::new(EventBus::new());
    let (shutdown_tx, shutdown) = watch::channel(false);
    let manager = Arc::new(Manager::new(Arc::clone(&store), secrets, bus, shutdown_tx));
    let clock = Arc::new(TestClock::new(NOON));
    let clock_dyn: Arc<dyn Clock> = clock.clone();
    manager.set_clock(clock_dyn);
    Harness {
        manager,
        store,
        clock,
        actions: Arc::new(RecordingActions::default()),
        shutdown,
    }
}

fn scheduler(h: &Harness) -> QueueScheduler {
    QueueScheduler::new(Arc::clone(&h.manager), host_actions(h), TICK_INTERVAL)
}

fn host_actions(h: &Harness) -> Arc<dyn HostActions> {
    let actions: Arc<dyn HostActions> = h.actions.clone();
    actions
}

async fn call(manager: &Arc<Manager>, method: &str, params: Value) -> Result<Value, (i64, String)> {
    manager
        .call(method, Some(params))
        .await
        .map_err(|err| (err.code, err.message))
}

fn record(id: &str, queue: &str, url: &str, dir: &Path) -> DownloadRecord {
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
        queue_name: queue.to_string(),
        download_type: "direct".to_string(),
        created_at: None,
        completed_at: None,
        filename: format!("{id}.bin"),
        media_kind: None,
        media_format: None,
        playlist_title: None,
        playlist_count: None,
        batch_id: None,
    }
}

async fn allow_local_downloads(h: &Harness, dir: &Path) {
    let settings = Settings {
        allow_local_downloads: true,
        download_dir: dir.display().to_string(),
        max_connections: 2,
        ..Settings::default()
    };
    call(
        &h.manager,
        "settings.update",
        serde_json::to_value(&settings).unwrap(),
    )
    .await
    .unwrap();
}

async fn create_queue(h: &Harness, name: &str, max_active: u32) {
    call(
        &h.manager,
        "queue.create",
        json!({ "name": name, "max_active": max_active }),
    )
    .await
    .unwrap();
}

fn status_of(h: &Harness, id: &str) -> DownloadStatus {
    h.store.get(id).unwrap().unwrap().status
}

/// Poll a task until it reaches a terminal state, in real time.
async fn wait_terminal(h: &Harness, id: &str) -> DownloadStatus {
    let mut status = status_of(h, id);
    for _ in 0..300 {
        if matches!(
            status,
            DownloadStatus::Completed | DownloadStatus::Failed | DownloadStatus::Cancelled
        ) {
            return status;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        status = status_of(h, id);
    }
    status
}

#[test]
fn a_window_is_evaluated_in_local_minutes() {
    let window = window_from_bounds(Some("08:00"), Some("17:00"))
        .unwrap()
        .expect("a window");
    assert!(window.is_open_at(8 * 60), "the start minute is open");
    assert!(window.is_open_at(12 * 60));
    assert!(!window.is_open_at(17 * 60), "the stop minute is closed");
    assert!(!window.is_open_at(7 * 60));

    // No bounds at all: the queue has no window and may run at any time.
    assert!(window_from_bounds(None, None).unwrap().is_none());
    // A single bound is honoured: `08:00` opens until midnight.
    let tail = window_from_bounds(Some("08:00"), None).unwrap().unwrap();
    assert!(tail.is_open_at(23 * 60));
    assert!(!tail.is_open_at(7 * 60));

    // Values the scheduler cannot evaluate are rejected instead of stored.
    assert!(window_from_bounds(Some("25:00"), Some("06:00")).is_err());
    assert!(window_from_bounds(Some("08:00"), Some("7:00")).is_err());
    assert!(window_from_bounds(Some("ocho"), Some("06:00")).is_err());
}

#[test]
fn a_window_that_crosses_midnight_is_open_on_both_sides() {
    let window = window_from_bounds(Some("22:00"), Some("06:00"))
        .unwrap()
        .expect("a window");
    for minute in [22 * 60, 23 * 60 + 59, 0, 3 * 60, 5 * 60 + 59] {
        assert!(window.is_open_at(minute), "{minute} must be open");
    }
    for minute in [6 * 60, NOON, 21 * 60 + 59] {
        assert!(!window.is_open_at(minute), "{minute} must be closed");
    }
}

#[tokio::test]
async fn queue_schedule_rejects_a_window_the_scheduler_cannot_evaluate() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(dir.path());

    for (start, stop) in [
        ("25:00", "06:00"),
        ("08:00", "7:00"),
        ("ocho", "06:00"),
        ("", "06:00"),
    ] {
        let err = call(
            &h.manager,
            "queue.schedule",
            json!({ "queue_name": "main", "start_time": start, "stop_time": stop }),
        )
        .await
        .expect_err("an invalid window must be rejected");
        assert_eq!(err.0, -32602, "{start}-{stop} was accepted: {err:?}");
    }
    assert!(
        h.store.queue_schedule("main").unwrap().is_none(),
        "an invalid schedule must not be persisted"
    );

    let stored = call(
        &h.manager,
        "queue.schedule",
        json!({ "queue_name": "main", "start_time": "22:00", "stop_time": "06:00" }),
    )
    .await
    .unwrap();
    assert_eq!(stored["schedule"]["start_time"], "22:00");
    assert!(h.store.queue_schedule("main").unwrap().is_some());
}

#[tokio::test]
async fn the_scheduler_starts_nothing_while_the_queue_is_closed() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(dir.path());
    let server = RangeServer::start(ServerConfig {
        body: vec![3u8; 64 * 1024],
        ..ServerConfig::default()
    })
    .await;
    allow_local_downloads(&h, dir.path()).await;
    create_queue(&h, "noche", 1).await;
    call(
        &h.manager,
        "queue.schedule",
        json!({ "queue_name": "noche", "start_time": "22:00", "stop_time": "06:00" }),
    )
    .await
    .unwrap();
    h.store
        .insert_download(&record(
            "closed",
            "noche",
            server.url().as_str(),
            dir.path(),
        ))
        .unwrap();

    // The injected clock is noon: `22:00–06:00` is closed.
    let mut sched = scheduler(&h);
    sched.tick();
    assert_eq!(status_of(&h, "closed"), DownloadStatus::Pending);
    assert_eq!(h.manager.running_count(), 0, "no engine may run");

    // The same queue with the clock inside the window starts on the next tick.
    h.clock.set(LATE);
    sched.tick();
    let started = status_of(&h, "closed");
    assert_ne!(
        started,
        DownloadStatus::Pending,
        "an open window must start the pending task"
    );
    let _ = call(
        &h.manager,
        "download.cancel",
        json!({ "task_id": "closed", "delete_file": true }),
    )
    .await;
    server.shutdown().await;
}

#[tokio::test]
async fn the_main_queue_honours_the_legacy_schedule_settings() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(dir.path());
    let server = RangeServer::start(ServerConfig {
        body: vec![5u8; 64 * 1024],
        ..ServerConfig::default()
    })
    .await;
    let settings = Settings {
        allow_local_downloads: true,
        download_dir: dir.path().display().to_string(),
        max_connections: 2,
        schedule_start: Some("22:00".to_string()),
        schedule_stop: Some("06:00".to_string()),
        ..Settings::default()
    };
    call(
        &h.manager,
        "settings.update",
        serde_json::to_value(&settings).unwrap(),
    )
    .await
    .unwrap();
    h.store
        .insert_download(&record("legacy", "main", server.url().as_str(), dir.path()))
        .unwrap();

    let mut sched = scheduler(&h);
    sched.tick();
    assert_eq!(status_of(&h, "legacy"), DownloadStatus::Pending);

    h.clock.set(LATE);
    sched.tick();
    assert_ne!(status_of(&h, "legacy"), DownloadStatus::Pending);
    let _ = call(
        &h.manager,
        "download.cancel",
        json!({ "task_id": "legacy", "delete_file": true }),
    )
    .await;
    server.shutdown().await;
}

#[tokio::test]
async fn a_completed_task_wakes_the_scheduler_for_the_next_pending_task() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(dir.path());
    let server = RangeServer::start(ServerConfig {
        body: vec![7u8; 128 * 1024],
        ..ServerConfig::default()
    })
    .await;
    allow_local_downloads(&h, dir.path()).await;
    // A single slot: the second task can only start when the first one finishes.
    create_queue(&h, "noche", 1).await;
    for id in ["first", "second"] {
        h.store
            .insert_download(&record(id, "noche", server.url().as_str(), dir.path()))
            .unwrap();
    }

    // One hour between ticks: only the completion wake-up can start `second`.
    let sched = QueueScheduler::new(
        Arc::clone(&h.manager),
        host_actions(&h),
        Duration::from_secs(3600),
    );
    let handle = tokio::spawn(sched.run(h.shutdown.clone()));

    let first = wait_terminal(&h, "first").await;
    let second = wait_terminal(&h, "second").await;
    assert_eq!(first, DownloadStatus::Completed, "first: {first:?}");
    assert_eq!(second, DownloadStatus::Completed, "second: {second:?}");

    h.manager.request_shutdown();
    handle.await.unwrap();
    server.shutdown().await;
}

#[tokio::test]
async fn action_on_complete_runs_once_when_the_queue_drains() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(dir.path());
    let server = RangeServer::start(ServerConfig {
        body: vec![1u8; 256 * 1024],
        chunk_size: 1024,
        slow: Some(Duration::from_millis(20)),
        ..ServerConfig::default()
    })
    .await;
    allow_local_downloads(&h, dir.path()).await;
    create_queue(&h, "noche", 1).await;
    call(
        &h.manager,
        "queue.schedule",
        json!({ "queue_name": "noche", "action_on_complete": "suspend" }),
    )
    .await
    .unwrap();
    h.store
        .insert_download(&record("solo", "noche", server.url().as_str(), dir.path()))
        .unwrap();

    let mut sched = scheduler(&h);
    sched.tick();
    assert_ne!(status_of(&h, "solo"), DownloadStatus::Pending);

    // The queue is not empty yet: the action must not run.
    sched.tick();
    assert_eq!(h.actions.suspends(), 0, "the queue still had work");

    let finished = wait_terminal(&h, "solo").await;
    assert_eq!(finished, DownloadStatus::Completed, "solo: {finished:?}");

    sched.tick();
    assert_eq!(h.actions.suspends(), 1, "the drain runs the action once");
    sched.tick();
    sched.tick();
    assert_eq!(h.actions.suspends(), 1, "a second drain did not happen");
    assert_eq!(h.actions.power_offs(), 0);
    server.shutdown().await;
}

#[tokio::test]
async fn the_exit_action_uses_the_existing_shutdown_channel() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(dir.path());
    let server = RangeServer::start(ServerConfig {
        body: vec![2u8; 128 * 1024],
        chunk_size: 1024,
        slow: Some(Duration::from_millis(20)),
        ..ServerConfig::default()
    })
    .await;
    allow_local_downloads(&h, dir.path()).await;
    create_queue(&h, "noche", 1).await;
    call(
        &h.manager,
        "queue.schedule",
        json!({ "queue_name": "noche", "action_on_complete": "exit" }),
    )
    .await
    .unwrap();
    h.store
        .insert_download(&record("solo", "noche", server.url().as_str(), dir.path()))
        .unwrap();

    let mut sched = scheduler(&h);
    sched.tick();
    assert!(!*h.shutdown.borrow(), "the daemon is still running");

    let finished = wait_terminal(&h, "solo").await;
    assert_eq!(finished, DownloadStatus::Completed, "solo: {finished:?}");
    sched.tick();
    assert!(
        *h.shutdown.borrow(),
        "`exit` must signal the daemon shutdown channel"
    );
    server.shutdown().await;
}
