//! Remote torrent jobs: builders and parsers for three providers, with canonical answers.

use segmenta_core::debrid::Provider;
use segmenta_core::debrid_torrent::{
    add_request, delete_request, is_magnet, parse_add, parse_info, TorrentState,
};

#[test]
fn the_three_providers_describe_their_add_request() {
    let rd = add_request(Provider::RealDebrid, "tok", "magnet:?xt=urn:btih:abc").unwrap();
    assert!(rd.url.ends_with("/torrents/addMagnet"), "{}", rd.url);
    assert!(rd.body.contains("magnet="));
    assert!(!format!("{rd:?}").contains("tok"));

    let tb = add_request(Provider::TorBox, "tok", "magnet:?xt=urn:btih:abc").unwrap();
    assert!(
        tb.url.ends_with("/v1/api/torrents/createtorrent"),
        "{}",
        tb.url
    );

    let pm = add_request(Provider::Premiumize, "tok", "magnet:?xt=urn:btih:abc").unwrap();
    assert!(pm.url.ends_with("/api/transfer/create"), "{}", pm.url);

    // A provider without a torrent flow, a missing token and a bogus link are all refused
    // before any request exists.
    assert!(add_request(Provider::AllDebrid, "tok", "magnet:?xt=urn:btih:abc").is_err());
    assert!(add_request(Provider::RealDebrid, " ", "magnet:?xt=urn:btih:abc").is_err());
    assert!(add_request(Provider::RealDebrid, "tok", "not a link").is_err());
}

#[test]
fn each_provider_reports_its_identifier_its_own_way() {
    assert_eq!(
        parse_add(
            Provider::RealDebrid,
            201,
            r#"{"id":"RD1","uri":"magnet:?x"}"#
        )
        .unwrap(),
        "RD1"
    );
    assert_eq!(
        parse_add(
            Provider::TorBox,
            200,
            r#"{"success":true,"data":{"torrent_id":42}}"#
        )
        .unwrap(),
        "42"
    );
    assert_eq!(
        parse_add(
            Provider::Premiumize,
            200,
            r#"{"status":"success","id":"PM1"}"#
        )
        .unwrap(),
        "PM1"
    );
    assert!(parse_add(Provider::RealDebrid, 401, "{}").is_err());
}

#[test]
fn real_debrid_progress_and_files_are_read() {
    let job = parse_info(
        Provider::RealDebrid,
        200,
        r#"{"id":"RD1","filename":"pack","status":"downloading","progress":42.5,
            "files":[{"id":1,"path":"/pack/a.bin","bytes":100,"selected":1},
                     {"id":2,"path":"/pack/b.bin","bytes":200,"selected":0}],
            "links":[]}"#,
    )
    .unwrap();
    assert_eq!(job.state, TorrentState::Downloading);
    assert_eq!(job.progress_pct, 42);
    assert_eq!(job.files.len(), 2);
    assert!(job.files[0].selected);
    assert!(!job.files[1].selected);
    assert_eq!(job.name.as_deref(), Some("pack"));

    let ready = parse_info(
        Provider::RealDebrid,
        200,
        r#"{"id":"RD1","status":"downloaded","progress":100,"links":["https://cdn/x"]}"#,
    )
    .unwrap();
    assert_eq!(ready.state, TorrentState::Ready);
    assert!(ready.state.is_terminal());
    assert_eq!(ready.links.len(), 1);

    let dead = parse_info(Provider::RealDebrid, 200, r#"{"id":"RD1","status":"dead"}"#).unwrap();
    assert_eq!(dead.state, TorrentState::Failed);
}

#[test]
fn torbox_reports_fractions_and_wraps_data() {
    let job = parse_info(
        Provider::TorBox,
        200,
        r#"{"success":true,"data":{"id":7,"name":"pack","download_state":"downloading",
             "progress":0.25,"files":[{"name":"a.bin","size":10}]}}"#,
    )
    .unwrap();
    assert_eq!(job.state, TorrentState::Downloading);
    assert_eq!(job.progress_pct, 25, "a fraction must become a percentage");
    assert_eq!(job.files[0].path, "a.bin");

    let done = parse_info(
        Provider::TorBox,
        200,
        r#"{"success":true,"data":{"id":7,"download_state":"completed","progress":1}}"#,
    )
    .unwrap();
    assert_eq!(done.state, TorrentState::Ready);
}

#[test]
fn premiumize_lists_transfers() {
    let job = parse_info(
        Provider::Premiumize,
        200,
        r#"{"status":"success","transfers":[{"id":"PM1","name":"pack","status":"running","progress":0.5}]}"#,
    )
    .unwrap();
    assert_eq!(job.state, TorrentState::Downloading);
    assert_eq!(job.progress_pct, 50);
    let finished = parse_info(
        Provider::Premiumize,
        200,
        r#"{"status":"success","transfers":[{"id":"PM1","status":"finished","progress":1}]}"#,
    )
    .unwrap();
    assert_eq!(finished.state, TorrentState::Ready);
}

#[test]
fn deletion_uses_each_provider_cleanup_call() {
    assert_eq!(
        delete_request(Provider::RealDebrid, "tok", "RD1")
            .unwrap()
            .method,
        "DELETE"
    );
    let torbox = delete_request(Provider::TorBox, "tok", "7").unwrap();
    assert!(torbox.url.ends_with("/controltorrent"));
    assert!(torbox.body.contains("operation=delete"));
    let premiumize = delete_request(Provider::Premiumize, "tok", "PM1").unwrap();
    assert!(premiumize.url.ends_with("/transfer/delete"));
}

#[test]
fn a_magnet_is_recognized_without_guessing() {
    assert!(is_magnet("magnet:?xt=urn:btih:abc"));
    assert!(!is_magnet("https://example.test/a.torrent"));
    assert!(!is_magnet("not a link"));
}
