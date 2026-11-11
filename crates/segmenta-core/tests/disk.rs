//! `pwrite_all`, preallocation and atomic finalization. Spec S6.

use std::io::{Error, ErrorKind};
use std::os::unix::fs::FileExt;

use segmenta_core::disk::{DiskWriter, PreallocOutcome, pwrite_all_with, preallocate_with};

fn eintr() -> Error {
    Error::from(ErrorKind::Interrupted)
}

fn errno(code: i32) -> Error {
    Error::from_raw_os_error(code)
}

#[test]
fn pwrite_all_finishes_the_buffer_through_eintr_and_partial_writes() {
    let payload: Vec<u8> = (0..10_000u32).map(|i| (i % 251) as u8).collect();
    let mut written = Vec::new();
    let mut calls = 0usize;
    let result = pwrite_all_with(&payload, 4096, |buf, offset| {
        calls += 1;
        // Signal interruption twice, then write in small pieces.
        if calls <= 2 {
            return Err(eintr());
        }
        assert_eq!(offset, 4096 + written.len() as u64);
        let take = buf.len().min(777);
        written.extend_from_slice(&buf[..take]);
        Ok(take)
    });
    assert!(result.is_ok());
    assert_eq!(written, payload);
    assert!(calls > 3, "the writer had to be called more than once");
}

#[test]
fn pwrite_all_rejects_a_zero_length_write_instead_of_looping() {
    let err = pwrite_all_with(b"abc", 0, |_, _| Ok(0)).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::WriteZero);
}

#[test]
fn pwrite_all_reports_the_failing_call() {
    let err = pwrite_all_with(b"abc", 0, |_, _| Err(errno(28))).unwrap_err();
    assert_eq!(err.raw_os_error(), Some(28));
}

#[test]
fn preallocation_uses_fallocate_when_the_filesystem_supports_it() {
    let outcome = preallocate_with(1024, 4096, || Ok(()), || Ok(())).unwrap();
    assert_eq!(outcome, PreallocOutcome::Fallocate);
}

#[test]
fn preallocation_fails_fast_without_space() {
    let err = preallocate_with(8192, 1024, || Err(errno(28)), || Ok(())).unwrap_err();
    assert_eq!(err.err_name(), "ERR_DISK_FULL");
}

#[test]
fn preallocation_falls_back_when_the_filesystem_cannot_fallocate() {
    let mut truncated = false;
    let outcome = preallocate_with(
        4096,
        1_000_000,
        || Err(errno(95)),
        || {
            truncated = true;
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(outcome, PreallocOutcome::Truncate { free_bytes: 1_000_000 });
    assert!(truncated, "the fallback must set the file length");

    // The same fallback refuses to continue when the space is not there.
    let err = preallocate_with(4096, 100, || Err(errno(95)), || Ok(())).unwrap_err();
    assert_eq!(err.err_name(), "ERR_DISK_FULL");
}

#[test]
fn offsets_are_honoured_without_a_seek_cursor() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("out.bin");
    let (writer, prealloc) = DiskWriter::create_new(&path, Some(64)).unwrap();
    assert_eq!(prealloc, PreallocOutcome::Fallocate);

    // Write out of order, as two workers would.
    writer.pwrite_all(b"world", 5).unwrap();
    writer.pwrite_all(b"hello", 0).unwrap();
    writer.sync_data().unwrap();
    assert_eq!(&std::fs::read(&path).unwrap()[..10], b"helloworld");

    // A writer keeps its own descriptor: writing elsewhere does not move this one.
    use std::io::Write as _;
    let mut other = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
    other.write_all(b"!").unwrap();
    writer.pwrite_all(b"X", 5).unwrap();
    assert_eq!(&std::fs::read(&path).unwrap()[..10], b"helloXorld");
}

#[test]
fn resume_does_not_truncate_existing_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("partial.bin");
    std::fs::write(&path, b"0123456789").unwrap();

    let (writer, _) = DiskWriter::open_resume(&path, 10).unwrap();
    writer.pwrite_all(b"AB", 4).unwrap();
    writer.sync_all().unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"0123AB6789");
    assert_eq!(writer.len().unwrap(), 10);
}

#[test]
fn truncate_and_size_are_visible_to_the_caller() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("trunc.bin");
    let (writer, _) = DiskWriter::create_new(&path, Some(10)).unwrap();
    writer.pwrite_all(b"0123456789", 0).unwrap();
    assert_eq!(writer.len().unwrap(), 10);
    writer.truncate().unwrap();
    assert_eq!(writer.len().unwrap(), 0);
}

#[test]
fn create_new_never_clobbers_an_existing_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("same.bin");
    std::fs::write(&path, b"keep me").unwrap();
    let err = DiskWriter::create_new(&path, Some(10)).unwrap_err();
    assert_eq!(err.io_kind(), Some(ErrorKind::AlreadyExists));
    assert_eq!(std::fs::read(&path).unwrap(), b"keep me");
}

#[test]
fn file_offset_writes_are_visible_through_std_reads() {
    // Guards against using the descriptor cursor instead of the explicit offset.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shared.bin");
    let (writer, _) = DiskWriter::create_new(&path, Some(8)).unwrap();
    writer.pwrite_all(b"aa", 0).unwrap();
    writer.pwrite_all(b"bb", 2).unwrap();
    writer.pwrite_all(b"cc", 4).unwrap();
    writer.sync_data().unwrap();
    let file = std::fs::File::open(&path).unwrap();
    let mut buf = [0u8; 6];
    file.read_exact_at(&mut buf, 0).unwrap();
    assert_eq!(&buf, b"aabbcc");
}
