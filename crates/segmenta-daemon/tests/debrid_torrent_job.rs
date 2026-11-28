//! Integration tests for remote debrid torrent jobs.
//!
//! Contract: S3 of `odd/tasks/segmenta-hito3-debrid-archivos-torrents.md` (T5 b).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use segmenta_core::debrid::{DebridError, DebridHttp, DebridRequest, Provider};
use segmenta_core::engine::Canceller;
use segmenta_daemon::debrid_torrent_job::{
    run_debrid_torrent_job, DebridTorrentError, DownloadSink, JobClock, ManualClock,
};
use segmenta_daemon::events::EventBus;
use segmenta_daemon::ipc::RpcHandler;
use segmenta_daemon::manager::Manager;
use segmenta_daemon::secrets::{FileSecretStore, SecretStore};
use segmenta_daemon::store::Store;
use serde_json::{json, Value};

#[derive(Default)]
struct MockDebridHttp {
    calls: Mutex<Vec<DebridRequest>>,
    add_answer: Mutex<Option<(u16, String)>>,
    info_answers: Mutex<std::collections::VecDeque<(u16, String)>>,
    delete_answer: Mutex<Option<(u16, String)>>,
    delete_called: AtomicBool,
}

impl MockDebridHttp {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(Vec::new()),
            add_answer: Mutex::new(Some((201, r#"{"id":"RD1","uri":"magnet:?x"}"#.to_string()))),
            info_answers: Mutex::new(std::collections::VecDeque::new()),
            delete_answer: Mutex::new(Some((204, String::new()))),
            delete_called: AtomicBool::new(false),
        })
    }

    fn calls(&self) -> Vec<DebridRequest> {
        self.calls.lock().unwrap().clone()
    }

    fn delete_called(&self) -> bool {
        self.delete_called.load(Ordering::SeqCst)
    }

    fn push_info(&self, status: u16, body: &str) {
        self.info_answers
            .lock()
            .unwrap()
            .push_back((status, body.to_string()));
    }
}

#[async_trait::async_trait]
impl DebridHttp for MockDebridHttp {
    async fn send(&self, request: &DebridRequest) -> Result<(u16, String), DebridError> {
        self.calls.lock().unwrap().push(request.clone());

        if request.method == "DELETE"
            || (request.method == "POST" && request.body.contains("operation=delete"))
            || request.url.contains("/transfer/delete")
        {
            self.delete_called.store(true, Ordering::SeqCst);
            if let Some((s, b)) = self.delete_answer.lock().unwrap().clone() {
                return Ok((s, b));
            }
            return Ok((204, String::new()));
        }

        if request.url.contains("/torrents/addMagnet")
            || request.url.contains("/torrents/createtorrent")
            || request.url.contains("/transfer/create")
        {
            if let Some((s, b)) = self.add_answer.lock().unwrap().clone() {
                return Ok((s, b));
            }
        }

        if request.url.contains("/torrents/info/")
            || request.url.contains("/torrents/mylist")
            || request.url.contains("/transfer/list")
        {
            let next = self.info_answers.lock().unwrap().pop_front();
            if let Some((s, b)) = next {
                return Ok((s, b));
            }
        }

        Ok((404, String::new()))
    }
}

struct RecordingSink {
    urls: Mutex<Vec<String>>,
    fail: AtomicBool,
}

impl RecordingSink {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            urls: Mutex::new(Vec::new()),
            fail: AtomicBool::new(false),
        })
    }

    fn fail(self: &Arc<Self>) -> Arc<Self> {
        self.fail.store(true, Ordering::SeqCst);
        Arc::clone(self)
    }

    fn urls(&self) -> Vec<String> {
        self.urls.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl DownloadSink for RecordingSink {
    async fn enqueue(&self, url: &str) -> Result<(), String> {
        if self.fail.load(Ordering::SeqCst) {
            return Err("download sink error: storage full".to_string());
        }
        self.urls.lock().unwrap().push(url.to_string());
        Ok(())
    }
}

#[allow(dead_code)]
struct Harness {
    manager: Arc<Manager>,
    store: Arc<Store>,
    secrets: Arc<dyn SecretStore>,
    bus: Arc<EventBus>,
    mock: Arc<MockDebridHttp>,
    sink: Arc<RecordingSink>,
    clock: Arc<ManualClock>,
    _dir: tempfile::TempDir,
}

fn harness(
    mock: Arc<MockDebridHttp>,
    sink: Arc<RecordingSink>,
    clock: Arc<ManualClock>,
) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(&dir.path().join("segmenta.db")).unwrap());
    let secrets: Arc<dyn SecretStore> =
        Arc::new(FileSecretStore::open(&dir.path().join("secrets.json")).unwrap());
    let bus = Arc::new(EventBus::new());
    let (shutdown, _rx) = tokio::sync::watch::channel(false);
    let manager = Manager::new(
        Arc::clone(&store),
        Arc::clone(&secrets),
        Arc::clone(&bus),
        shutdown,
    )
    .with_debrid_http(Arc::clone(&mock) as Arc<dyn DebridHttp>)
    .with_download_sink(Arc::clone(&sink) as Arc<dyn DownloadSink>)
    .with_torrent_clock(Arc::clone(&clock) as Arc<dyn JobClock>);
    Harness {
        manager: Arc::new(manager),
        store,
        secrets,
        bus,
        mock,
        sink,
        clock,
        _dir: dir,
    }
}

async fn call(manager: &Arc<Manager>, method: &str, params: Value) -> Result<Value, (i64, String)> {
    manager
        .call(method, Some(params))
        .await
        .map_err(|err| (err.code, err.message))
}

#[tokio::test]
async fn settings_flag_torrent_via_debrid_defaults_off_and_persists() {
    let mock = MockDebridHttp::new();
    let sink = RecordingSink::new();
    let clock = Arc::new(ManualClock::new());
    let h = harness(mock, sink, clock);

    assert!(!h.store.settings().unwrap().torrent_via_debrid);

    call(
        &h.manager,
        "settings.update",
        json!({ "torrent_via_debrid": true }),
    )
    .await
    .unwrap();
    assert!(h.store.settings().unwrap().torrent_via_debrid);

    call(
        &h.manager,
        "settings.update",
        json!({ "torrent_via_debrid": false }),
    )
    .await
    .unwrap();
    assert!(!h.store.settings().unwrap().torrent_via_debrid);
}

#[tokio::test]
async fn a_job_that_goes_queued_downloading_ready_enqueues_every_link_and_deletes_remote() {
    let mock = MockDebridHttp::new();
    mock.push_info(
        200,
        r#"{"id":"RD1","filename":"pack","status":"queued","progress":0,"files":[],"links":[]}"#,
    );
    mock.push_info(
        200,
        r#"{"id":"RD1","filename":"pack","status":"downloading","progress":50,"files":[],"links":[]}"#,
    );
    mock.push_info(
        200,
        r#"{"id":"RD1","filename":"pack","status":"downloaded","progress":100,"files":[],"links":["https://cdn.example/file1.iso","https://cdn.example/file2.nfo"]}"#,
    );

    let sink = RecordingSink::new();
    let clock = Arc::new(ManualClock::new());
    let cancel = Canceller::new();

    let res = run_debrid_torrent_job(
        mock.as_ref(),
        Provider::RealDebrid,
        "token123",
        "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=Pack",
        sink.as_ref(),
        clock.as_ref(),
        &cancel,
        || Ok(()),
        |_| {},
    )
    .await;

    assert!(res.is_ok(), "job should succeed: {res:?}");
    assert_eq!(
        sink.urls(),
        vec![
            "https://cdn.example/file1.iso".to_string(),
            "https://cdn.example/file2.nfo".to_string()
        ]
    );
    assert!(mock.delete_called(), "delete request must be performed");
}

#[tokio::test]
async fn a_sink_that_fails_leaves_the_remote_job_untouched() {
    let mock = MockDebridHttp::new();
    mock.push_info(
        200,
        r#"{"id":"RD1","filename":"pack","status":"downloaded","progress":100,"files":[],"links":["https://cdn.example/file1.iso"]}"#,
    );

    let sink = RecordingSink::new();
    sink.fail();
    let clock = Arc::new(ManualClock::new());
    let cancel = Canceller::new();

    let res = run_debrid_torrent_job(
        mock.as_ref(),
        Provider::RealDebrid,
        "token123",
        "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=Pack",
        sink.as_ref(),
        clock.as_ref(),
        &cancel,
        || Ok(()),
        |_| {},
    )
    .await;

    assert!(res.is_err(), "job must report error when sink fails");
    assert!(
        matches!(res, Err(DebridTorrentError::Sink(_))),
        "expected Sink error, got {res:?}"
    );
    assert!(
        !mock.delete_called(),
        "failing sink must leave remote job untouched"
    );
}

#[tokio::test]
async fn an_unknown_provider_status_surfaces_unknown_state() {
    let mock = MockDebridHttp::new();
    mock.push_info(
        200,
        r#"{"id":"RD1","filename":"pack","status":"some_wild_unknown_status","progress":0,"files":[],"links":[]}"#,
    );

    let sink = RecordingSink::new();
    let clock = Arc::new(ManualClock::new());
    let cancel = Canceller::new();

    let res = run_debrid_torrent_job(
        mock.as_ref(),
        Provider::RealDebrid,
        "token123",
        "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=Pack",
        sink.as_ref(),
        clock.as_ref(),
        &cancel,
        || Ok(()),
        |_| {},
    )
    .await;

    assert!(res.is_err(), "unknown status must return error");
    match res {
        Err(DebridTorrentError::Core(DebridError::UnknownState(ref status))) => {
            assert_eq!(status, "some_wild_unknown_status");
        }
        other => panic!("expected Core(UnknownState), got {other:?}"),
    }
    assert!(!mock.delete_called(), "must not delete on error");
}

#[tokio::test]
async fn the_deadline_stops_the_loop() {
    let mock = MockDebridHttp::new();
    // Return downloading on every poll
    for _ in 0..10 {
        mock.push_info(
            200,
            r#"{"id":"RD1","filename":"pack","status":"downloading","progress":10,"files":[],"links":[]}"#,
        );
    }

    let sink = RecordingSink::new();
    let clock = Arc::new(ManualClock::new());
    // Advance clock by 1 hour on every poll so the 6 hour deadline is reached quickly
    clock.set_auto_advance(Duration::from_secs(3600));
    let cancel = Canceller::new();

    let res = run_debrid_torrent_job(
        mock.as_ref(),
        Provider::RealDebrid,
        "token123",
        "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=Pack",
        sink.as_ref(),
        clock.as_ref(),
        &cancel,
        || Ok(()),
        |_| {},
    )
    .await;

    assert!(
        matches!(res, Err(DebridTorrentError::DeadlineExceeded)),
        "expected DeadlineExceeded, got {res:?}"
    );
    assert!(!mock.delete_called(), "must not delete on deadline");
}

#[tokio::test]
async fn an_expired_job_stops_polling() {
    let mock = MockDebridHttp::new();
    mock.push_info(
        200,
        r#"{"id":"RD1","filename":"pack","status":"magnet_error","progress":0,"files":[],"links":[]}"#,
    );

    let sink = RecordingSink::new();
    let clock = Arc::new(ManualClock::new());
    let cancel = Canceller::new();

    let res = run_debrid_torrent_job(
        mock.as_ref(),
        Provider::RealDebrid,
        "token123",
        "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=Pack",
        sink.as_ref(),
        clock.as_ref(),
        &cancel,
        || Ok(()),
        |_| {},
    )
    .await;

    assert!(
        matches!(res, Err(DebridTorrentError::Expired)),
        "expected Expired, got {res:?}"
    );
    assert!(!mock.delete_called(), "must not delete on expired");
    // Only one poll was made: stopped immediately
    let info_calls = mock
        .calls()
        .into_iter()
        .filter(|req| req.url.contains("/info/"))
        .count();
    assert_eq!(info_calls, 1, "expired job must stop polling immediately");
}

#[tokio::test]
async fn with_the_flag_off_a_magnet_never_reaches_the_debrid_layer() {
    let mock = MockDebridHttp::new();
    let sink = RecordingSink::new();
    let clock = Arc::new(ManualClock::new());
    let h = harness(Arc::clone(&mock), sink, clock);

    // Ensure torrent_via_debrid is false
    assert!(!h.store.settings().unwrap().torrent_via_debrid);

    // Configure a debrid account so one exists
    call(
        &h.manager,
        "debrid.account.add",
        json!({ "provider": "realdebrid", "token": "t1", "id": "acc1", "priority": 1 }),
    )
    .await
    .unwrap();

    let magnet = "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=Ubuntu+ISO";
    let _ = call(&h.manager, "torrent.add", json!({ "url": magnet })).await;

    // Check that debrid mock never received any calls related to torrents
    let debrid_torrent_calls = mock
        .calls()
        .into_iter()
        .filter(|req| req.url.contains("/torrents/"))
        .count();
    assert_eq!(
        debrid_torrent_calls, 0,
        "magnet must never reach debrid layer when torrent_via_debrid is false"
    );
}

#[tokio::test]
async fn local_completion_failure_leaves_remote_job_untouched() {
    let mock = MockDebridHttp::new();
    mock.push_info(
        200,
        r#"{"id":"RD1","filename":"pack","status":"downloaded","progress":100,"files":[],"links":["https://cdn.example/file1.iso"]}"#,
    );

    let sink = RecordingSink::new();
    let clock = Arc::new(ManualClock::new());
    let cancel = Canceller::new();

    let res = run_debrid_torrent_job(
        mock.as_ref(),
        Provider::RealDebrid,
        "token123",
        "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=Pack",
        sink.as_ref(),
        clock.as_ref(),
        &cancel,
        || Err("local file write error".to_string()),
        |_| {},
    )
    .await;

    // Queued links are a success from the user's side; the remote job is what must survive.
    assert!(res.is_ok(), "queued links are not a failure: {res:?}");
    assert!(
        !mock.delete_called(),
        "local completion failure must leave remote job untouched"
    );
}

#[tokio::test]
async fn with_the_flag_on_a_magnet_runs_through_debrid_end_to_end() {
    let mock = MockDebridHttp::new();
    mock.push_info(
        200,
        r#"{"id":"RD1","filename":"pack","status":"downloaded","progress":100,"files":[],"links":["https://cdn.example/file1.iso"]}"#,
    );

    let sink = RecordingSink::new();
    let clock = Arc::new(ManualClock::new());
    let h = harness(Arc::clone(&mock), Arc::clone(&sink), Arc::clone(&clock));

    // Subscribe before action to capture task_completed
    let mut events = h.bus.subscribe();

    // Turn on torrent_via_debrid
    call(
        &h.manager,
        "settings.update",
        json!({ "torrent_via_debrid": true }),
    )
    .await
    .unwrap();

    // Configure a debrid account
    call(
        &h.manager,
        "debrid.account.add",
        json!({ "provider": "realdebrid", "token": "t1", "id": "acc1", "priority": 1 }),
    )
    .await
    .unwrap();

    let magnet = "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=Ubuntu+ISO";
    let added = call(&h.manager, "torrent.add", json!({ "url": magnet }))
        .await
        .unwrap();
    let task_id = added["task_id"].as_str().unwrap().to_string();

    // Wait for the task to complete
    let mut completed = false;
    for _ in 0..50 {
        if let Ok(event) = events.try_recv() {
            if event.task_id.as_deref() == Some(&task_id) && event.event_type == "task_completed" {
                completed = true;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(completed, "debrid torrent task must publish task_completed");

    assert_eq!(
        sink.urls(),
        vec!["https://cdn.example/file1.iso".to_string()]
    );
    // Production does not confirm local completion yet, so the remote job is kept on
    // purpose: deleting it before the files are home would leave the user with nothing.
    assert!(
        !mock.delete_called(),
        "the remote job must survive an unconfirmed local completion"
    );
}
