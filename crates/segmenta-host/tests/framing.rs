//! Native Messaging framing contract of OpenSpec 03 §4 and ODD S1.
//!
//! These tests drive only the crate's public framing API. The proxy tests live in
//! `tests/proxy.rs`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::io::{self, Cursor, Read, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use segmenta_host::framing::{read_message, write_message, FrameError, MAX_MESSAGE_BYTES};
use serde_json::{json, Value};

/// Tracks the largest single allocation request of the whole test binary so the
/// oversized-header test can prove the payload is never buffered.
struct TrackingAllocator;

static PEAK_ALLOCATION: AtomicUsize = AtomicUsize::new(0);

// SAFETY: every method forwards to the system allocator unchanged; the counters are
// only observation.
unsafe impl GlobalAlloc for TrackingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        PEAK_ALLOCATION.fetch_max(layout.size(), Ordering::SeqCst);
        System.alloc(layout)
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        PEAK_ALLOCATION.fetch_max(layout.size(), Ordering::SeqCst);
        System.alloc_zeroed(layout)
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        PEAK_ALLOCATION.fetch_max(new_size, Ordering::SeqCst);
        System.realloc(ptr, layout, new_size)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
    }
}

#[global_allocator]
static ALLOCATOR: TrackingAllocator = TrackingAllocator;

/// Serializes the allocation measurement against the other tests of this binary.
static ALLOCATION_LOCK: Mutex<()> = Mutex::new(());

fn frame(value: &Value) -> Vec<u8> {
    let payload = serde_json::to_vec(value).expect("serialize");
    let mut bytes = (payload.len() as u32).to_le_bytes().to_vec();
    bytes.extend_from_slice(&payload);
    bytes
}

/// A reader that yields the 4 header bytes and then refuses to be read again, so a
/// frame reader that allocates and buffers the payload cannot pass the test.
struct HeaderOnlyReader {
    header: [u8; 4],
    position: usize,
}

impl HeaderOnlyReader {
    fn new(length: u32) -> HeaderOnlyReader {
        HeaderOnlyReader {
            header: length.to_le_bytes(),
            position: 0,
        }
    }
}

impl Read for HeaderOnlyReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.position >= self.header.len() {
            return Err(io::Error::other(
                "the payload must not be read after an oversized header",
            ));
        }
        let remaining = &self.header[self.position..];
        let take = remaining.len().min(buf.len());
        buf[..take].copy_from_slice(&remaining[..take]);
        self.position += take;
        Ok(take)
    }
}

#[test]
fn frame_roundtrip() {
    let value = json!({
        "jsonrpc": "2.0",
        "id": 7,
        "method": "media.probe",
        "params": { "url": "https://example.invalid/watch?v=1", "kind": "video" },
    });

    let mut encoded = Vec::new();
    write_message(&mut encoded, &value).expect("write");
    let declared = u32::from_le_bytes(encoded[..4].try_into().expect("header")) as usize;
    assert_eq!(declared, encoded.len() - 4);
    assert_eq!(
        &encoded[4..],
        serde_json::to_vec(&value).unwrap().as_slice()
    );

    let mut input = Cursor::new(encoded);
    assert_eq!(read_message(&mut input).expect("read"), Some(value.clone()));
    // The same reader is at EOF now: no trailing bytes were left behind.
    assert_eq!(read_message(&mut input).expect("eof"), None);
}

#[test]
fn an_empty_reader_is_eof_and_not_an_error() {
    let mut input = Cursor::new(Vec::new());
    assert_eq!(read_message(&mut input).expect("eof"), None);
}

#[test]
fn a_zero_length_is_the_empty_error() {
    let mut input = Cursor::new(vec![0u8, 0, 0, 0]);
    let err = read_message(&mut input).unwrap_err();
    assert!(matches!(err, FrameError::Empty), "got {err:?}");
    assert!(err.to_string().contains("ERR_FRAME_EMPTY"), "{err}");
}

#[test]
fn the_cap_is_one_mebibyte() {
    assert_eq!(MAX_MESSAGE_BYTES, 1024 * 1024);
}

#[test]
fn an_oversized_header_is_rejected_without_reading_the_payload() {
    let _guard = ALLOCATION_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    PEAK_ALLOCATION.store(0, Ordering::SeqCst);

    let mut input = HeaderOnlyReader::new(100 * 1024 * 1024);
    let err = read_message(&mut input).unwrap_err();
    let peak = PEAK_ALLOCATION.load(Ordering::SeqCst);

    assert!(
        matches!(err, FrameError::TooLarge { length } if length == 100 * 1024 * 1024),
        "got {err:?}"
    );
    assert!(err.to_string().contains("ERR_FRAME_TOO_LARGE"), "{err}");
    assert!(
        peak < MAX_MESSAGE_BYTES,
        "the framing layer allocated {peak} bytes for a forged 100 MiB header"
    );
}

#[test]
fn a_header_over_the_cap_is_rejected_even_with_a_body_attached() {
    let mut bytes = ((MAX_MESSAGE_BYTES as u32) + 1).to_le_bytes().to_vec();
    bytes.extend_from_slice(b"{}");
    let err = read_message(&mut Cursor::new(bytes)).unwrap_err();
    assert!(matches!(err, FrameError::TooLarge { .. }), "got {err:?}");
}

#[test]
fn a_truncated_header_is_truncated() {
    for partial in [vec![1u8], vec![1u8, 0], vec![1u8, 0, 0]] {
        let err = read_message(&mut Cursor::new(partial.clone())).unwrap_err();
        assert!(matches!(err, FrameError::Truncated), "{partial:?}: {err:?}");
        assert!(err.to_string().contains("ERR_FRAME_TRUNCATED"), "{err}");
    }
}

#[test]
fn a_truncated_body_is_truncated() {
    let mut bytes = 64u32.to_le_bytes().to_vec();
    bytes.extend_from_slice(b"{\"jsonrpc\"");
    let err = read_message(&mut Cursor::new(bytes)).unwrap_err();
    assert!(matches!(err, FrameError::Truncated), "got {err:?}");
    assert!(err.to_string().contains("ERR_FRAME_TRUNCATED"), "{err}");
}

#[test]
fn invalid_json_and_invalid_utf8_are_invalid_json() {
    for body in [
        b"not json".as_slice(),
        b"{\"a\": }".as_slice(),
        &[0xff, 0xfe],
    ] {
        let mut bytes = (body.len() as u32).to_le_bytes().to_vec();
        bytes.extend_from_slice(body);
        let err = read_message(&mut Cursor::new(bytes)).unwrap_err();
        assert!(matches!(err, FrameError::InvalidJson { .. }), "got {err:?}");
        assert!(err.to_string().contains("ERR_FRAME_INVALID_JSON"), "{err}");
    }
}

#[test]
fn two_messages_back_to_back_are_read_one_at_a_time() {
    let first = json!({ "jsonrpc": "2.0", "id": 1, "method": "media.probe" });
    let second = json!({ "jsonrpc": "2.0", "id": 2, "method": "media.add" });
    let mut bytes = frame(&first);
    bytes.extend_from_slice(&frame(&second));

    let mut input = Cursor::new(bytes);
    assert_eq!(read_message(&mut input).expect("first"), Some(first));
    assert_eq!(read_message(&mut input).expect("second"), Some(second));
    assert_eq!(read_message(&mut input).expect("eof"), None);
}

#[test]
fn writing_a_message_over_the_cap_fails_before_emitting_bytes() {
    let value = json!({ "jsonrpc": "2.0", "id": 1, "blob": "a".repeat(MAX_MESSAGE_BYTES) });
    let mut encoded = Vec::new();
    let err = write_message(&mut encoded, &value).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    assert!(err.to_string().contains("ERR_FRAME_TOO_LARGE"), "{err}");
    assert!(encoded.is_empty(), "no bytes may reach the browser");
}

#[test]
fn writing_always_flushes_the_frame() {
    #[derive(Default)]
    struct FlushRecorder {
        written: usize,
        flushed: usize,
    }

    impl Write for FlushRecorder {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.written += buf.len();
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            self.flushed += 1;
            Ok(())
        }
    }

    let mut recorder = FlushRecorder::default();
    write_message(&mut recorder, &json!({ "jsonrpc": "2.0", "id": 1 })).expect("write");
    assert_eq!(recorder.flushed, 1);
    assert!(recorder.written > 4);
}
