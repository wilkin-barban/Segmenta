//! Extraction behaviour: containment, original preservation and graceful degradation.
//!
//! Every test drives a stub extractor, so nothing depends on a real `7z` or `unzip`
//! being installed on the machine that runs the suite.

use segmenta_daemon::extract::{extract, is_archive, Extraction};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// Write an executable stub that appends its argv to `log` and then runs `body`.
fn stub(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    let log = dir.join(format!("{name}.argv"));
    std::fs::write(
        &path,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" >> '{log}'\n{body}\n",
            log = log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn argv_of(dir: &Path, name: &str) -> String {
    std::fs::read_to_string(dir.join(format!("{name}.argv"))).unwrap_or_default()
}

/// Stub that answers the listing and extracts `payload` into the `-o` directory.
fn extracting_stub(dir: &Path, listing: &str, payload: &[&str]) -> PathBuf {
    let mut body = String::from("if [ \"$1\" = l ]; then\n");
    // Real `7z l -slt` output: the archive's own path first, then the member separator.
    body.push_str("  printf '%s\\n' 'Path = /absolute/archive.zip'\n");
    body.push_str("  printf '%s\\n' '----------'\n");
    for name in listing.lines() {
        body.push_str(&format!("  printf '%s\\n' 'Path = {name}'\n"));
    }
    body.push_str("  exit 0\nfi\n");
    body.push_str(
        "out='.'\nfor arg in \"$@\"; do case \"$arg\" in -o*) out=\"${arg#-o}\";; esac; done\n",
    );
    for name in payload {
        body.push_str(&format!("printf '%s' payload > \"$out/{name}\"\n"));
    }
    stub(dir, "7z", &body)
}

#[test]
fn recognises_archive_names_only() {
    assert!(is_archive(Path::new("/tmp/a.zip")));
    assert!(is_archive(Path::new("/tmp/a.7z")));
    assert!(is_archive(Path::new("/tmp/a.part1.rar")));
    assert!(!is_archive(Path::new("/tmp/a.mkv")));
    assert!(!is_archive(Path::new("/tmp/a.zip.txt")));
}

#[test]
fn extracts_into_the_destination_and_preserves_the_source() {
    let dir = tempfile::tempdir().unwrap();
    let staging = dir.path().join("downloads");
    std::fs::create_dir(&staging).unwrap();
    let archive = staging.join("bundle.zip");
    std::fs::write(&archive, b"archive").unwrap();
    extracting_stub(dir.path(), "safe.txt", &["safe.txt"]);

    let outcome = extract(&archive, true, None, &[dir.path().to_path_buf()]);

    assert!(
        matches!(outcome, Extraction::Succeeded { .. }),
        "{outcome:?}"
    );
    assert_eq!(
        std::fs::read_to_string(staging.join("safe.txt")).unwrap(),
        "payload"
    );
    assert!(archive.exists(), "the original archive must survive");
    let argv = argv_of(dir.path(), "7z");
    assert!(argv.starts_with("l\n"), "{argv}");
    assert!(argv.contains("\nx\n"), "{argv}");
}

#[test]
fn rejects_parent_traversal_before_extracting_anything() {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("evil.zip");
    std::fs::write(&archive, b"archive").unwrap();
    extracting_stub(dir.path(), "../escaped.txt", &["escaped.txt"]);

    let outcome = extract(&archive, true, None, &[dir.path().to_path_buf()]);

    assert!(
        matches!(outcome, Extraction::Rejected { .. }),
        "{outcome:?}"
    );
    assert!(!dir.path().join("escaped.txt").exists());
    assert!(!dir.path().parent().unwrap().join("escaped.txt").exists());
    let argv = argv_of(dir.path(), "7z");
    assert!(!argv.contains("\nx\n"), "extraction must not run: {argv}");
}

#[test]
fn rejects_absolute_entries() {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("absolute.zip");
    std::fs::write(&archive, b"archive").unwrap();
    extracting_stub(dir.path(), "/etc/passwd", &["passwd"]);

    let outcome = extract(&archive, true, None, &[dir.path().to_path_buf()]);

    assert!(
        matches!(outcome, Extraction::Rejected { .. }),
        "{outcome:?}"
    );
    assert!(!argv_of(dir.path(), "7z").contains("\nx\n"));
}

#[test]
fn degrades_without_an_available_tool() {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("bundle.zip");
    std::fs::write(&archive, b"archive").unwrap();
    let empty = tempfile::tempdir().unwrap();

    let outcome = extract(&archive, false, None, &[empty.path().to_path_buf()]);

    assert!(
        matches!(outcome, Extraction::Unavailable { .. }),
        "{outcome:?}"
    );
    assert!(
        archive.exists(),
        "a missing tool must never delete the source"
    );
}

#[test]
fn removes_the_source_only_when_asked_and_successful() {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("bundle.zip");
    std::fs::write(&archive, b"archive").unwrap();
    extracting_stub(dir.path(), "safe.txt", &["safe.txt"]);

    let kept = extract(&archive, false, None, &[dir.path().to_path_buf()]);
    assert!(matches!(kept, Extraction::Succeeded { .. }), "{kept:?}");
    assert!(
        !archive.exists(),
        "successful extraction may drop the source"
    );

    std::fs::write(&archive, b"archive").unwrap();
    let failing = stub(dir.path(), "unzip", "exit 2");
    assert!(failing.exists());
    let dir_without_7z = tempfile::tempdir().unwrap();
    let archive_failure = dir_without_7z.path().join("bundle.zip");
    std::fs::write(&archive_failure, b"archive").unwrap();
    stub(
        dir_without_7z.path(),
        "7z",
        "if [ \"$1\" = l ]; then printf '%s\\n' '----------' 'Path = safe.txt'; exit 0; fi\nexit 2",
    );
    let failed = extract(
        &archive_failure,
        false,
        None,
        &[dir_without_7z.path().to_path_buf()],
    );
    assert!(matches!(failed, Extraction::Failed { .. }), "{failed:?}");
    assert!(
        archive_failure.exists(),
        "a failed extraction must never delete the source"
    );
}

#[test]
fn rejects_a_staged_symlink() {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("link.zip");
    std::fs::write(&archive, b"archive").unwrap();
    stub(
        dir.path(),
        "7z",
        "if [ \"$1\" = l ]; then printf '%s\\n' '----------' 'Path = link'; exit 0; fi\nout='.'\nfor arg in \"$@\"; do case \"$arg\" in -o*) out=\"${arg#-o}\";; esac; done\nln -s /etc \"$out/link\"",
    );

    let outcome = extract(&archive, true, None, &[dir.path().to_path_buf()]);

    assert!(
        matches!(outcome, Extraction::Rejected { .. }),
        "{outcome:?}"
    );
    assert!(!dir.path().join("link").exists());
}
