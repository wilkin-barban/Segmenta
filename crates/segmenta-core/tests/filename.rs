//! Path traversal, header and URL name sanitization. Spec S11.

use std::fs;

use segmenta_core::filename::{
    filename_from_content_disposition, resolve_unique_path, sanitize_filename, suggest_filename,
};

#[test]
fn path_traversal_is_reduced_to_the_base_name() {
    assert_eq!(
        sanitize_filename("../../etc/passwd").as_deref(),
        Some("passwd")
    );
    assert_eq!(
        sanitize_filename("..\\..\\windows\\evil.exe").as_deref(),
        Some("evil.exe")
    );
    assert_eq!(sanitize_filename("a/b/c.txt").as_deref(), Some("c.txt"));
    assert_eq!(
        sanitize_filename("/absolute/path.iso").as_deref(),
        Some("path.iso")
    );
    assert_eq!(
        sanitize_filename("..%2fescape").as_deref(),
        Some("..%2fescape")
    );
}

#[test]
fn control_characters_are_removed_and_nul_becomes_underscore() {
    assert_eq!(
        sanitize_filename("with\u{0}nul.txt").as_deref(),
        Some("with_nul.txt")
    );
    assert_eq!(
        sanitize_filename("tab\there.txt").as_deref(),
        Some("tabhere.txt")
    );
    assert_eq!(
        sanitize_filename("bell\u{7}.bin").as_deref(),
        Some("bell.bin")
    );
    assert_eq!(
        sanitize_filename("del\u{7f}.bin").as_deref(),
        Some("del.bin")
    );
}

#[test]
fn names_that_collapse_to_nothing_are_rejected() {
    assert_eq!(sanitize_filename(""), None);
    assert_eq!(sanitize_filename("."), None);
    assert_eq!(sanitize_filename(".."), None);
    assert_eq!(sanitize_filename("/"), None);
    assert_eq!(sanitize_filename("..."), None);
}

#[test]
fn names_are_truncated_to_255_bytes_on_a_char_boundary() {
    let long = format!("{}.bin", "a".repeat(400));
    let name = sanitize_filename(&long).expect("still a name");
    assert!(name.len() <= 255, "len={}", name.len());
    assert!(name.ends_with(".bin"));

    // 300 two-byte characters: truncation must not split a code point.
    let multibyte = "é".repeat(300);
    let name = sanitize_filename(&multibyte).expect("still a name");
    assert!(name.len() <= 255);
    assert_eq!(name.chars().count(), 127);
    assert!(std::str::from_utf8(name.as_bytes()).is_ok());
}

#[test]
fn content_disposition_prefers_filename_star() {
    let header = "attachment; filename=\"fallback.txt\"; filename*=UTF-8''na%C3%AFve%20file.txt";
    assert_eq!(
        filename_from_content_disposition(header).as_deref(),
        Some("naïve file.txt")
    );
}

#[test]
fn content_disposition_handles_plain_and_unquoted_names() {
    assert_eq!(
        filename_from_content_disposition("attachment; filename=\"report.pdf\"").as_deref(),
        Some("report.pdf")
    );
    assert_eq!(
        filename_from_content_disposition("attachment; filename=report.pdf").as_deref(),
        Some("report.pdf")
    );
    assert_eq!(filename_from_content_disposition("inline").as_deref(), None);
    // A traversal attempt in the header is sanitized, never returned raw.
    assert_eq!(
        filename_from_content_disposition("attachment; filename=\"../../etc/passwd\"").as_deref(),
        Some("passwd")
    );
}

#[test]
fn url_suggests_a_decoded_base_name() {
    let url = url::Url::parse("https://example.com/files/ubuntu%2026.04.iso").unwrap();
    assert_eq!(suggest_filename(&url).as_deref(), Some("ubuntu 26.04.iso"));
    let url = url::Url::parse("https://example.com/").unwrap();
    assert_eq!(suggest_filename(&url), None);
    let url = url::Url::parse("https://example.com/dir/").unwrap();
    assert_eq!(suggest_filename(&url), None);
}

#[test]
fn existing_files_are_never_overwritten() {
    let dir = tempfile::tempdir().unwrap();
    let target = resolve_unique_path(dir.path(), "file.iso").unwrap();
    assert_eq!(target, dir.path().join("file.iso"));
    fs::write(&target, b"first").unwrap();

    let second = resolve_unique_path(dir.path(), "file.iso").unwrap();
    assert_eq!(second, dir.path().join("file (1).iso"));
    fs::write(&second, b"second").unwrap();

    let third = resolve_unique_path(dir.path(), "file.iso").unwrap();
    assert_eq!(third, dir.path().join("file (2).iso"));
    assert_eq!(fs::read(dir.path().join("file.iso")).unwrap(), b"first");
}

#[test]
fn collision_suffix_works_without_extension_and_for_dotfiles() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("archive"), b"x").unwrap();
    assert_eq!(
        resolve_unique_path(dir.path(), "archive").unwrap(),
        dir.path().join("archive (1)")
    );

    fs::write(dir.path().join(".bashrc"), b"x").unwrap();
    assert_eq!(
        resolve_unique_path(dir.path(), ".bashrc").unwrap(),
        dir.path().join(".bashrc (1)")
    );
}
