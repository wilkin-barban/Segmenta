//! Torrent tests (`torrent.probe`, `torrent.add`, `torrent.status`, path hostility).

use segmenta_daemon::events::EventBus;
use segmenta_daemon::ipc::RpcHandler;
use segmenta_daemon::manager::Manager;
use segmenta_daemon::store::{Settings, Store};
use segmenta_daemon::torrent::sanitize_torrent_path;
use serde_json::json;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::watch;

#[test]
fn hostile_torrent_paths_are_rejected() {
    let base = Path::new("/tmp/downloads");
    assert!(sanitize_torrent_path(base, "../../etc/passwd").is_err());
    assert!(sanitize_torrent_path(base, "/etc/passwd").is_err());
    assert!(sanitize_torrent_path(base, "foo/../../etc/passwd").is_err());
    assert!(sanitize_torrent_path(base, "subdir/movie.mp4").is_ok());
}

#[tokio::test]
async fn torrent_rpc_surface_magnet() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(&dir.path().join("segmenta.db")).unwrap());
    let settings = Settings {
        download_dir: dir.path().join("downloads").display().to_string(),
        ..Settings::default()
    };
    store
        .update_settings(
            serde_json::to_value(&settings)
                .unwrap()
                .as_object()
                .unwrap(),
        )
        .unwrap();
    let secret_store: Arc<dyn segmenta_daemon::SECRET_STORE_TRAIT> =
        Arc::from(segmenta_daemon::open_secret_store(dir.path(), None).unwrap());
    let bus = Arc::new(EventBus::new());
    let (shutdown, _rx) = watch::channel(false);
    let manager = Arc::new(Manager::new(
        Arc::clone(&store),
        secret_store,
        Arc::clone(&bus),
        shutdown,
    ));

    let magnet = "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=Ubuntu+ISO";
    let probe = manager
        .call("torrent.probe", Some(json!({"url": magnet})))
        .await
        .expect("torrent probe");
    assert_eq!(probe["name"], "Ubuntu ISO");
    assert_eq!(probe["source_type"], "magnet");

    let added = manager
        .call("torrent.add", Some(json!({"url": magnet})))
        .await
        .expect("torrent add");
    let task_id = added["task_id"].as_str().unwrap();
    // The engine starts the transfer immediately, so the reply is "downloading"
    // (the pre-engine slice answered "pending").
    assert_eq!(added["status"], "downloading");

    let status = manager
        .call("torrent.status", Some(json!({"task_id": task_id})))
        .await
        .expect("torrent status");
    assert_eq!(status["status"], "pending");
    assert_eq!(status["files_count"], 1);
    assert_eq!(status["files"].as_array().unwrap().len(), 1);
}

/// S11: `torrent.add` and `torrent.status` report the directory the payload lands in and
/// the resolved path of the task; `task_added` carries both.
#[tokio::test]
async fn torrent_add_reports_the_effective_destination_and_final_path() {
    use std::time::Duration;

    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(&dir.path().join("segmenta.db")).unwrap());
    let settings = Settings {
        download_dir: dir.path().join("downloads").display().to_string(),
        ..Settings::default()
    };
    store
        .update_settings(
            serde_json::to_value(&settings)
                .unwrap()
                .as_object()
                .unwrap(),
        )
        .unwrap();
    let secret_store: Arc<dyn segmenta_daemon::SECRET_STORE_TRAIT> =
        Arc::from(segmenta_daemon::open_secret_store(dir.path(), None).unwrap());
    let bus = Arc::new(EventBus::new());
    let (shutdown, _rx) = watch::channel(false);
    let manager = Arc::new(Manager::new(
        Arc::clone(&store),
        secret_store,
        Arc::clone(&bus),
        shutdown,
    ));

    let destination = dir.path().join("torrentes");
    let magnet = "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=Ubuntu+ISO";
    let mut events = bus.subscribe();
    let added = manager
        .call(
            "torrent.add",
            Some(json!({
                "url": magnet,
                "destination_dir": destination.display().to_string(),
            })),
        )
        .await
        .expect("torrent add");
    let task_id = added["task_id"].as_str().unwrap().to_string();
    let expected_final = destination.join("Ubuntu ISO").display().to_string();
    assert_eq!(
        added["destination_dir"],
        json!(destination.display().to_string())
    );
    assert_eq!(added["final_path"], json!(expected_final));

    // The `task_added` event fires before the engine starts, but the transfer emitter may
    // already have published; scan until the add event shows up.
    let mut added_event = None;
    for _ in 0..20 {
        match tokio::time::timeout(Duration::from_millis(250), events.recv()).await {
            Ok(Ok(event)) if event.event_type == "task_added" => {
                added_event = Some(event);
                break;
            }
            Ok(Ok(_)) => continue,
            _ => break,
        }
    }
    let event = added_event.expect("task_added must be published");
    assert_eq!(event.task_id.as_deref(), Some(task_id.as_str()));
    assert_eq!(
        event.data["destination_dir"],
        json!(destination.display().to_string())
    );
    assert_eq!(event.data["final_path"], json!(expected_final));

    let status = manager
        .call("torrent.status", Some(json!({ "task_id": task_id })))
        .await
        .expect("torrent status");
    assert_eq!(
        status["destination_dir"],
        json!(destination.display().to_string())
    );
    assert_eq!(status["final_path"], json!(expected_final));
}

// ---------------------------------------------------------------------------
// R2: `torrent.probe` on real bencode metainfo (multi-file, single-file,
// hostile paths and inconsistent piece budgets).
// ---------------------------------------------------------------------------

/// Append a bencode byte string.
fn bencode_str(out: &mut Vec<u8>, value: &str) {
    out.extend_from_slice(value.len().to_string().as_bytes());
    out.push(b':');
    out.extend_from_slice(value.as_bytes());
}

/// Append a bencode integer.
fn bencode_int(out: &mut Vec<u8>, value: u64) {
    out.push(b'i');
    out.extend_from_slice(value.to_string().as_bytes());
    out.push(b'e');
}

/// A real bencode `.torrent` body, with sorted keys inside every dict.
///
/// `files` is `Some` for multi-file metainfo, `length` is `Some` for single-file;
/// `pieces_bytes` is the raw byte length of the `pieces` string so a test can declare a
/// hash budget that does not match the summed lengths.
fn torrent_bytes(
    name: &str,
    piece_length: u64,
    length: Option<u64>,
    files: Option<&[(Vec<&str>, u64)]>,
    pieces_bytes: usize,
) -> Vec<u8> {
    let mut info = vec![b'd'];
    if let Some(files) = files {
        bencode_str(&mut info, "files");
        info.push(b'l');
        for (path, len) in files {
            info.push(b'd');
            bencode_str(&mut info, "length");
            bencode_int(&mut info, *len);
            bencode_str(&mut info, "path");
            info.push(b'l');
            for component in path {
                bencode_str(&mut info, component);
            }
            info.push(b'e');
            info.push(b'e');
        }
        info.push(b'e');
    }
    if let Some(length) = length {
        bencode_str(&mut info, "length");
        bencode_int(&mut info, length);
    }
    bencode_str(&mut info, "name");
    bencode_str(&mut info, name);
    bencode_str(&mut info, "piece length");
    bencode_int(&mut info, piece_length);
    bencode_str(&mut info, "pieces");
    info.extend_from_slice(pieces_bytes.to_string().as_bytes());
    info.push(b':');
    info.extend(std::iter::repeat(b'x').take(pieces_bytes));
    info.push(b'e');

    let mut body = vec![b'd'];
    bencode_str(&mut body, "announce");
    bencode_str(&mut body, "http://127.0.0.1:1/announce");
    bencode_str(&mut body, "info");
    body.extend_from_slice(&info);
    body.push(b'e');
    body
}

/// Serve `body` over the local range server and probe it through the real HTTP path.
async fn probe_body(body: Vec<u8>) -> Result<serde_json::Value, segmenta_daemon::rpc::RpcError> {
    let server = segmenta_core::testserver::RangeServer::with_body(body).await;
    let client = reqwest::Client::new();
    let result = segmenta_daemon::torrent::probe_torrent(&client, server.url().as_str()).await;
    server.shutdown().await;
    result
}

#[tokio::test]
async fn probe_reports_every_file_of_a_multi_file_torrent() {
    // 5 + 7 bytes in 4-byte pieces needs 3 hashes.
    let body = torrent_bytes(
        "Ubuntu ISO",
        4,
        None,
        Some(&[(vec!["a.txt"], 5), (vec!["dir", "b.txt"], 7)]),
        3 * 20,
    );
    let probe = probe_body(body).await.expect("multi-file probe");
    assert_eq!(probe["name"], "Ubuntu ISO");
    assert_eq!(probe["source_type"], "torrent");
    assert_eq!(probe["total_bytes"], json!(12));

    let files = probe["files"].as_array().expect("files array");
    assert_eq!(files.len(), 2);
    for (index, file) in files.iter().enumerate() {
        assert_eq!(file["index"], json!(index));
        assert_eq!(file["selected"], json!(true));
    }
    assert_eq!(files[0]["path"], "a.txt");
    assert_eq!(files[0]["length"], json!(5));
    assert_eq!(files[1]["path"], "dir/b.txt");
    assert_eq!(files[1]["length"], json!(7));
}

#[tokio::test]
async fn probe_keeps_single_file_torrents_on_info_length() {
    // 9 bytes in 4-byte pieces needs 3 hashes.
    let body = torrent_bytes("ubuntu.iso", 4, Some(9), None, 3 * 20);
    let probe = probe_body(body).await.expect("single-file probe");
    assert_eq!(probe["total_bytes"], json!(9));

    let files = probe["files"].as_array().expect("files array");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0]["index"], json!(0));
    assert_eq!(files[0]["path"], "ubuntu.iso");
    assert_eq!(files[0]["length"], json!(9));
    assert_eq!(files[0]["selected"], json!(true));
}

#[tokio::test]
async fn probe_never_reports_a_path_that_escapes() {
    let base = Path::new("/tmp/downloads");
    // 3 bytes in 4-byte pieces needs 1 hash.
    let body = torrent_bytes(
        "hostile",
        4,
        None,
        Some(&[
            (vec!["../escape.bin"], 1),
            (vec!["/etc/passwd"], 1),
            (vec!["dir\\..\\evil.dll"], 1),
        ]),
        20,
    );
    let probe = probe_body(body)
        .await
        .expect("hostile components are repaired into the download directory");
    let files = probe["files"].as_array().expect("files array");
    assert_eq!(files.len(), 3);
    for file in files {
        let path = file["path"].as_str().expect("path string");
        assert!(!path.contains('/'), "{path} keeps a separator");
        assert!(!path.contains('\\'), "{path} keeps a Windows separator");
        let relative = Path::new(path);
        assert!(!relative.is_absolute(), "{path} is absolute");
        assert!(
            relative
                .components()
                .all(|component| matches!(component, std::path::Component::Normal(_))),
            "{path} keeps a hostile component"
        );
        let resolved = sanitize_torrent_path(base, path).expect("path resolves inside the base");
        assert!(resolved.starts_with(base), "{path} escaped the base");
    }

    // A literal `..` component has nothing left once sanitized: reject the metainfo
    // instead of inventing a name.
    let dotted = torrent_bytes(
        "hostile",
        4,
        None,
        Some(&[(vec!["..", "escape.bin"], 5)]),
        2 * 20,
    );
    let err = probe_body(dotted)
        .await
        .expect_err("a `..` component must be rejected");
    assert_eq!(err.code, -32602);
}

#[tokio::test]
async fn probe_rejects_pieces_that_do_not_cover_the_declared_lengths() {
    // 20 bytes of payload in 4-byte pieces is 5 hashes; the metainfo declares 2.
    let short = torrent_bytes(
        "liar",
        4,
        None,
        Some(&[(vec!["a.bin"], 10), (vec!["b.bin"], 10)]),
        2 * 20,
    );
    let err = probe_body(short)
        .await
        .expect_err("a piece budget that disagrees with the lengths must be rejected");
    assert_eq!(err.code, -32602);
    assert_eq!(err.data.as_ref().unwrap()["code"], "INVALID_PARAMS");
    assert!(
        err.message.contains("piece"),
        "unexpected message: {}",
        err.message
    );

    // A `pieces` string that is not a whole number of 20-byte hashes is equally invalid.
    let ragged = torrent_bytes(
        "liar",
        4,
        None,
        Some(&[(vec!["a.bin"], 10), (vec!["b.bin"], 10)]),
        5 * 20 + 3,
    );
    let err = probe_body(ragged)
        .await
        .expect_err("a ragged pieces string must be rejected");
    assert_eq!(err.code, -32602);

    // The single-file shape is checked the same way: 9 bytes in 4-byte pieces is 3 hashes.
    let single = torrent_bytes("liar.iso", 4, Some(9), None, 2 * 20);
    let err = probe_body(single)
        .await
        .expect_err("the single-file budget must be checked too");
    assert_eq!(err.code, -32602);
}
