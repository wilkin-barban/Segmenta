//! Locale-aware rendering of the CLI output. Spec S21.
//!
//! The renderers and the error constructors take an explicit `Locale`, so these tests
//! never mutate the process environment and stay independent of the developer's `LANG`.
//! Values owned by the daemon (task ids, URLs, paths, queue names, protocol status words)
//! and stable codes must stay verbatim in every locale.

use std::path::Path;

use segmenta_cli::client::ClientError;
use segmenta_cli::render::{render_queues, render_status, render_table};
use segmenta_daemon::rpc::{DownloadView, SegmentView};
use segmenta_i18n::Locale;

fn view() -> DownloadView {
    DownloadView {
        media_kind: None,
        playlist_title: None,
        task_id: "c1f7a28e-8b1b-4f9e-9d22-29f123456789".to_string(),
        url: "https://example.test/ubuntu.iso".to_string(),
        final_path: "/home/u/Downloads/ubuntu.iso".to_string(),
        filename: "ubuntu.iso".to_string(),
        status: "downloading".to_string(),
        total_bytes: Some(10 * 1024 * 1024),
        downloaded_bytes: 5 * 1024 * 1024,
        speed_bps: 1024 * 1024,
        eta_seconds: Some(5),
        retry_count: 0,
        supports_ranges: true,
        queue_name: "main".to_string(),
        created_at: None,
        completed_at: None,
        error_code: None,
        error_message: None,
        quarantine_path: None,
        segments: vec![SegmentView {
            id: 1,
            start_offset: 0,
            end_offset: 1023,
            next_offset: 512,
            status: "active".to_string(),
            speed_bps: 0,
        }],
    }
}

fn header(text: &str) -> &str {
    text.lines().next().unwrap_or_default()
}

#[test]
fn the_list_header_is_localized() {
    let rows = [view()];
    let en = render_table(Locale::En, &rows);
    let es = render_table(Locale::Es, &rows);
    let pt = render_table(Locale::PtBr, &rows);

    assert!(header(&en).contains("STATUS"), "{en}");
    assert!(header(&en).contains("SIZE"), "{en}");
    assert!(header(&en).contains("SPEED"), "{en}");
    assert!(header(&es).contains("ESTADO"), "{es}");
    assert!(header(&es).contains("TAMAÑO"), "{es}");
    assert!(header(&es).contains("VELOCIDAD"), "{es}");
    assert!(header(&pt).contains("STATUS"), "{pt}");
    assert!(header(&pt).contains("TAMANHO"), "{pt}");
    assert!(header(&pt).contains("VELOCIDADE"), "{pt}");

    assert_ne!(header(&en), header(&es), "es and en headers must differ");
    assert_ne!(header(&en), header(&pt), "pt and en headers must differ");

    // Identifiers and values stay verbatim; the status word is localized (S21).
    for text in [&en, &es, &pt] {
        assert!(text.contains("c1f7a28e"), "{text}");
        assert!(text.contains("ubuntu.iso"), "{text}");
        assert!(text.contains("10.0 MiB"), "{text}");
        assert!(!text.contains("downloading"), "{text}");
    }
    assert!(en.contains("Downloading"), "{en}");
    assert!(es.contains("Descargando"), "{es}");
    assert!(pt.contains("Baixando"), "{pt}");

    assert_eq!(render_table(Locale::En, &[]), "No downloads.\n");
    assert_eq!(render_table(Locale::Es, &[]), "No hay descargas.\n");
    assert_eq!(render_table(Locale::PtBr, &[]), "Não há downloads.\n");
}

#[test]
fn the_status_block_is_localized() {
    let en = render_status(Locale::En, &view());
    let es = render_status(Locale::Es, &view());
    let pt = render_status(Locale::PtBr, &view());

    assert!(en.contains("file:"), "{en}");
    assert!(es.contains("archivo:"), "{es}");
    assert!(pt.contains("arquivo:"), "{pt}");
    assert!(es.contains("estado:"), "{es}");
    assert!(es.contains("Segmentos:"), "{es}");
    assert_ne!(en, es, "the es status block must differ from en");
    assert_ne!(en, pt, "the pt-BR status block must differ from en");

    // Paths, ids, URLs, status words and queue names are never translated.
    for text in [&en, &es, &pt] {
        assert!(
            text.contains("c1f7a28e-8b1b-4f9e-9d22-29f123456789"),
            "{text}"
        );
        assert!(text.contains("https://example.test/ubuntu.iso"), "{text}");
        assert!(text.contains("/home/u/Downloads/ubuntu.iso"), "{text}");
        assert!(text.contains("main"), "{text}");
        assert!(text.contains("[####....]"), "{text}");
    }

    // An unknown total size is a human word and is localized too.
    let mut unknown = view();
    unknown.total_bytes = None;
    assert!(render_status(Locale::En, &unknown).contains("unknown"));
    assert!(render_status(Locale::Es, &unknown).contains("desconocido"));
    assert!(render_status(Locale::PtBr, &unknown).contains("desconhecido"));
}

#[test]
fn the_queue_table_is_localized() {
    let queues = serde_json::json!([{
        "name": "main",
        "max_active": 3,
        "active": 1,
        "pending": 2,
        "paused": 0,
    }]);
    let es = render_queues(Locale::Es, &queues);
    assert!(header(&es).contains("NOMBRE"), "{es}");
    let pt = render_queues(Locale::PtBr, &queues);
    assert!(header(&pt).contains("NOME"), "{pt}");
    for text in [&es, &pt] {
        assert!(text.contains("main"), "{text}");
    }

    assert_eq!(
        render_queues(Locale::En, &serde_json::json!([])),
        "No queues.\n"
    );
    assert_eq!(
        render_queues(Locale::Es, &serde_json::json!([])),
        "No hay colas.\n"
    );
    assert_eq!(
        render_queues(Locale::PtBr, &serde_json::json!([])),
        "Não há filas.\n"
    );
}

#[test]
fn a_rpc_error_is_localized_from_its_stable_code() {
    let en = ClientError::rpc(Locale::En, 1006, "disk full").to_string();
    let es = ClientError::rpc(Locale::Es, 1006, "disk full").to_string();
    let pt = ClientError::rpc(Locale::PtBr, 1006, "disk full").to_string();

    assert_eq!(en, "Not enough disk space (code 1006)");
    assert_eq!(es, "No hay espacio en disco (code 1006)");
    assert_eq!(pt, "Não há espaço em disco (code 1006)");

    // A domain code such as 1011 must resolve even when the daemon message differs.
    let pausable = ClientError::rpc(Locale::Es, 1011, "task `x` is not running").to_string();
    assert_eq!(
        pausable,
        "Esta tarea no está en marcha y no se puede pausar (code 1011)"
    );

    // An unknown code falls back to the daemon message, still with the code visible.
    let unknown = ClientError::rpc(Locale::Es, 4242, "boom").to_string();
    assert_eq!(unknown, "boom (code 4242)");
}

#[test]
fn an_unavailable_daemon_error_names_the_socket_and_the_command() {
    let path = Path::new("/run/user/1000/segmenta/daemon.sock");
    let en = ClientError::unavailable(
        Locale::En,
        path,
        std::io::Error::new(std::io::ErrorKind::NotFound, "No such file or directory"),
    )
    .to_string();
    let es = ClientError::unavailable(
        Locale::Es,
        path,
        std::io::Error::new(std::io::ErrorKind::NotFound, "No such file or directory"),
    )
    .to_string();
    let pt = ClientError::unavailable(
        Locale::PtBr,
        path,
        std::io::Error::new(std::io::ErrorKind::NotFound, "No such file or directory"),
    )
    .to_string();

    // The path and the command are never translated.
    for text in [&en, &es, &pt] {
        assert!(
            text.contains("/run/user/1000/segmenta/daemon.sock"),
            "{text}"
        );
        assert!(
            text.contains("systemctl --user enable --now segmenta.socket"),
            "{text}"
        );
    }
    assert!(en.contains("The daemon is not reachable"), "{en}");
    assert!(es.contains("No se puede contactar con el daemon"), "{es}");
    assert!(pt.contains("Não é possível contatar o daemon"), "{pt}");
    assert_ne!(en, es);
    assert_ne!(en, pt);
}

#[test]
fn json_payloads_are_never_localized() {
    // `--json` prints the daemon payload verbatim: the serialized view has no label of
    // its own, so no localized word can leak into machine-readable output.
    let payload = serde_json::to_string(&view()).expect("the view is serializable");
    for localized in [
        render_table(Locale::Es, &[view()]),
        render_status(Locale::Es, &view()),
    ] {
        assert!(!localized.is_empty());
    }
    for word in ["ESTADO", "TAMAÑO", "archivo:", "Segmentos:", "añadida"] {
        assert!(!payload.contains(word), "JSON leaked `{word}`: {payload}");
    }
}
