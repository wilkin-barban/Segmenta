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
        quarantine_path: None,
        postprocess_exit: None,
        postprocess_message: None,
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

/// Whether `status` is one of the states that occupies a `max_active` slot (OpenSpec 12 §3).
fn holds_slot(status: DownloadStatus) -> bool {
    matches!(
        status,
        DownloadStatus::Downloading
            | DownloadStatus::Probing
            | DownloadStatus::Retrying
            | DownloadStatus::Verifying
            | DownloadStatus::Finalizing
    )
}

/// Pending tasks of a queue, read through the same query the scheduler uses.
fn pending_len(h: &Harness, queue: &str) -> usize {
    h.store.pending_for_queue(queue, 100).unwrap().len()
}

/// Poll a condition in real time, for at most ~15 s.
async fn wait_until(mut check: impl FnMut() -> bool) -> bool {
    for _ in 0..600 {
        if check() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    check()
}

/// Cancel every task of a queue, leaving no engine behind for the next test file.
async fn cancel_all(h: &Harness, ids: &[&str]) {
    for id in ids {
        let _ = call(
            &h.manager,
            "download.cancel",
            json!({ "task_id": id, "delete_file": false }),
        )
        .await;
    }
}

/// A slow server keeps a task in flight long enough to observe the slot it holds.
async fn slow_server() -> RangeServer {
    RangeServer::start(ServerConfig {
        body: vec![4u8; 128 * 1024],
        chunk_size: 2048,
        slow: Some(Duration::from_millis(15)),
        ..ServerConfig::default()
    })
    .await
}

#[tokio::test]
async fn max_active_one_starts_one_task_and_the_next_only_after_it_ends() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(dir.path());
    let server = slow_server().await;
    allow_local_downloads(&h, dir.path()).await;
    create_queue(&h, "uno", 1).await;
    for id in ["first", "second"] {
        h.store
            .insert_download(&record(id, "uno", server.url().as_str(), dir.path()))
            .unwrap();
    }

    // One hour between ticks: only a completion may start the second task.
    let sched = QueueScheduler::new(
        Arc::clone(&h.manager),
        host_actions(&h),
        Duration::from_secs(3600),
    );
    let handle = tokio::spawn(sched.run(h.shutdown.clone()));

    let mut started_after_the_first_ended = false;
    for _ in 0..600 {
        let first = status_of(&h, "first");
        let second = status_of(&h, "second");
        assert!(
            h.store.active_count("uno").unwrap() <= 1,
            "max_active 1 was exceeded: first={first:?} second={second:?}"
        );
        if holds_slot(second) {
            assert!(
                !holds_slot(first),
                "the second task started while the first still held the slot"
            );
            started_after_the_first_ended = true;
        }
        if matches!(
            first,
            DownloadStatus::Completed | DownloadStatus::Failed | DownloadStatus::Cancelled
        ) && matches!(
            second,
            DownloadStatus::Completed | DownloadStatus::Failed | DownloadStatus::Cancelled
        ) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    assert!(
        started_after_the_first_ended,
        "the second task never started after the first freed its slot"
    );
    assert_eq!(status_of(&h, "first"), DownloadStatus::Completed);
    assert_eq!(status_of(&h, "second"), DownloadStatus::Completed);
    // Both tasks are terminal in the database, so neither may keep an in-flight entry:
    // a leaked entry would keep counting against the queue limit forever.
    assert!(
        wait_until(|| h.manager.running_count() == 0).await,
        "a finished task must free its in-flight entry, found {}",
        h.manager.running_count()
    );

    h.manager.request_shutdown();
    handle.await.unwrap();
    server.shutdown().await;
}

#[tokio::test]
async fn max_active_two_starts_two_of_three_pending_tasks() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(dir.path());
    let server = slow_server().await;
    allow_local_downloads(&h, dir.path()).await;
    create_queue(&h, "dos", 2).await;
    let ids = ["a", "b", "c"];
    for id in ids {
        h.store
            .insert_download(&record(id, "dos", server.url().as_str(), dir.path()))
            .unwrap();
    }

    let mut sched = scheduler(&h);
    sched.tick();

    assert_eq!(
        h.store.active_count("dos").unwrap(),
        2,
        "max_active 2 must start exactly two tasks"
    );
    assert_eq!(pending_len(&h, "dos"), 1, "one task must still wait");
    assert_eq!(h.manager.running_count(), 2);

    cancel_all(&h, &ids).await;
    server.shutdown().await;
}

#[tokio::test]
async fn max_active_above_the_pending_count_starts_them_all() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(dir.path());
    let server = slow_server().await;
    allow_local_downloads(&h, dir.path()).await;
    create_queue(&h, "ocho", 8).await;
    let ids = ["only", "other"];
    for id in ids {
        h.store
            .insert_download(&record(id, "ocho", server.url().as_str(), dir.path()))
            .unwrap();
    }

    let mut sched = scheduler(&h);
    sched.tick();

    assert_eq!(
        h.store.active_count("ocho").unwrap(),
        2,
        "a limit above the pending count must not hold tasks back"
    );
    assert_eq!(pending_len(&h, "ocho"), 0);

    cancel_all(&h, &ids).await;
    server.shutdown().await;
}

#[tokio::test]
async fn raising_max_active_starts_the_pending_tasks_that_now_fit() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(dir.path());
    let server = slow_server().await;
    allow_local_downloads(&h, dir.path()).await;
    create_queue(&h, "tres", 1).await;
    let ids = ["one", "two", "three"];
    for id in ids {
        h.store
            .insert_download(&record(id, "tres", server.url().as_str(), dir.path()))
            .unwrap();
    }

    let mut sched = scheduler(&h);
    sched.tick();
    assert_eq!(h.store.active_count("tres").unwrap(), 1);
    assert_eq!(pending_len(&h, "tres"), 2);

    // The limit lives in the `queues` row: raising it must be seen by the next start,
    // with no stale value cached in the manager.
    h.store.set_queue_max_active("tres", 3).unwrap();
    sched.tick();

    assert_eq!(
        h.store.active_count("tres").unwrap(),
        3,
        "the raised limit must start the two waiting tasks"
    );
    assert_eq!(pending_len(&h, "tres"), 0);

    cancel_all(&h, &ids).await;
    server.shutdown().await;
}

#[tokio::test]
async fn a_cancelled_task_frees_its_slot_for_the_next_pending_task() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(dir.path());
    let server = slow_server().await;
    allow_local_downloads(&h, dir.path()).await;
    create_queue(&h, "cancel", 1).await;
    for id in ["first", "second"] {
        h.store
            .insert_download(&record(id, "cancel", server.url().as_str(), dir.path()))
            .unwrap();
    }

    // One hour between ticks: only the cancellation wake-up can start `second`.
    let sched = QueueScheduler::new(
        Arc::clone(&h.manager),
        host_actions(&h),
        Duration::from_secs(3600),
    );
    let handle = tokio::spawn(sched.run(h.shutdown.clone()));

    assert!(
        wait_until(|| status_of(&h, "first") == DownloadStatus::Downloading).await,
        "the first task never started"
    );
    assert_eq!(status_of(&h, "second"), DownloadStatus::Pending);

    call(
        &h.manager,
        "download.cancel",
        json!({ "task_id": "first", "delete_file": false }),
    )
    .await
    .unwrap();

    let started = wait_until(|| status_of(&h, "second") != DownloadStatus::Pending).await;
    assert!(
        started,
        "a cancelled task did not free its slot for the next pending task"
    );
    let finished = wait_terminal(&h, "second").await;
    assert_eq!(finished, DownloadStatus::Completed, "second: {finished:?}");

    h.manager.request_shutdown();
    handle.await.unwrap();
    server.shutdown().await;
}

#[tokio::test]
async fn a_failed_task_frees_its_slot_for_the_next_pending_task() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(dir.path());
    let broken = RangeServer::start(ServerConfig {
        force_status: Some(404),
        ..ServerConfig::default()
    })
    .await;
    let server = slow_server().await;
    let settings = Settings {
        allow_local_downloads: true,
        download_dir: dir.path().display().to_string(),
        max_connections: 2,
        max_retries: 0,
        ..Settings::default()
    };
    call(
        &h.manager,
        "settings.update",
        serde_json::to_value(&settings).unwrap(),
    )
    .await
    .unwrap();
    create_queue(&h, "fallo", 1).await;
    h.store
        .insert_download(&record(
            "broken",
            "fallo",
            broken.url().as_str(),
            dir.path(),
        ))
        .unwrap();
    h.store
        .insert_download(&record(
            "healthy",
            "fallo",
            server.url().as_str(),
            dir.path(),
        ))
        .unwrap();

    // One hour between ticks: only the failure wake-up can start `healthy`.
    let sched = QueueScheduler::new(
        Arc::clone(&h.manager),
        host_actions(&h),
        Duration::from_secs(3600),
    );
    let handle = tokio::spawn(sched.run(h.shutdown.clone()));

    let started = wait_until(|| status_of(&h, "healthy") != DownloadStatus::Pending).await;
    assert!(
        started,
        "a failed task did not free its slot for the next pending task"
    );
    let failed = wait_terminal(&h, "broken").await;
    assert_eq!(failed, DownloadStatus::Failed, "broken: {failed:?}");
    let finished = wait_terminal(&h, "healthy").await;
    assert_eq!(finished, DownloadStatus::Completed, "healthy: {finished:?}");

    h.manager.request_shutdown();
    handle.await.unwrap();
    broken.shutdown().await;
    server.shutdown().await;
}

#[tokio::test]
async fn download_add_never_starts_more_than_the_queue_limit() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(dir.path());
    let server = slow_server().await;
    allow_local_downloads(&h, dir.path()).await;
    create_queue(&h, "manual", 1).await;

    // `start_immediately` is the default: two adds into one slot.
    let mut ids = Vec::new();
    for _ in 0..2 {
        let added = call(
            &h.manager,
            "download.add",
            json!({ "url": server.url().as_str(), "queue_name": "manual" }),
        )
        .await
        .unwrap();
        ids.push(added["task_id"].as_str().unwrap().to_string());
    }

    assert_eq!(
        h.store.active_count("manual").unwrap(),
        1,
        "`download.add` started more tasks than `max_active` allows"
    );
    assert_eq!(pending_len(&h, "manual"), 1, "the second add must wait");

    let sched = QueueScheduler::new(
        Arc::clone(&h.manager),
        host_actions(&h),
        Duration::from_secs(3600),
    );
    let handle = tokio::spawn(sched.run(h.shutdown.clone()));
    for id in &ids {
        let finished = wait_terminal(&h, id).await;
        assert_eq!(finished, DownloadStatus::Completed, "{id}: {finished:?}");
    }

    h.manager.request_shutdown();
    handle.await.unwrap();
    server.shutdown().await;
}
