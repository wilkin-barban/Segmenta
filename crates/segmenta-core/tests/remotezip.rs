//! Remote ZIP inspection: the central directory is read by `Range`, never the whole file.

use segmenta_core::remotezip::{open, ZipError, ZipMethod};
use std::sync::Mutex;
use url::Url;

/// Source that serves bytes from a buffer and records every range asked for.
struct Fixture {
    bytes: Vec<u8>,
    asked: Mutex<Vec<(u64, u64)>>,
}

impl Fixture {
    fn new(bytes: Vec<u8>) -> Fixture {
        Fixture {
            bytes,
            asked: Mutex::new(Vec::new()),
        }
    }

    fn ranges(&self) -> Vec<(u64, u64)> {
        self.asked.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl segmenta_core::remotezip::RangeSource for Fixture {
    async fn total_size(&self, _url: &Url) -> Result<Option<u64>, ZipError> {
        Ok(Some(self.bytes.len() as u64))
    }

    async fn range(&self, _url: &Url, start: u64, end: u64) -> Result<(u16, Vec<u8>), ZipError> {
        self.asked.lock().unwrap().push((start, end));
        let start = start as usize;
        let end = (end as usize).min(self.bytes.len().saturating_sub(1));
        if start >= self.bytes.len() {
            return Ok((416, Vec::new()));
        }
        Ok((206, self.bytes[start..=end].to_vec()))
    }
}

/// Build a ZIP by hand: two stored entries, then the central directory and the EOCD.
fn stored_zip() -> Vec<u8> {
    let files: [(&str, &[u8]); 2] = [("a.txt", b"hello"), ("dir/b.bin", &[0u8, 1, 2, 3])];
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in files {
        let offset = out.len() as u32;
        let crc = crc32(data);
        out.extend_from_slice(&0x04034b50u32.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes()); // version needed
        out.extend_from_slice(&0u16.to_le_bytes()); // flags
        out.extend_from_slice(&0u16.to_le_bytes()); // method: stored
        out.extend_from_slice(&0u16.to_le_bytes()); // time
        out.extend_from_slice(&0u16.to_le_bytes()); // date
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // extra
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);

        central.extend_from_slice(&0x02014b50u32.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes()); // version made by
        central.extend_from_slice(&20u16.to_le_bytes()); // version needed
        central.extend_from_slice(&0u16.to_le_bytes()); // flags
        central.extend_from_slice(&0u16.to_le_bytes()); // method
        central.extend_from_slice(&0u16.to_le_bytes()); // time
        central.extend_from_slice(&0u16.to_le_bytes()); // date
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(name.len() as u16).to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes()); // extra len
        central.extend_from_slice(&0u16.to_le_bytes()); // comment len
        central.extend_from_slice(&0u16.to_le_bytes()); // disk
        central.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
        central.extend_from_slice(&0u32.to_le_bytes()); // external attrs
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    let central_offset = out.len() as u32;
    let central_size = central.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x06054b50u32.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // disk
    out.extend_from_slice(&0u16.to_le_bytes()); // cd disk
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&central_size.to_le_bytes());
    out.extend_from_slice(&central_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // comment length
    out
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for byte in data {
        crc ^= *byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

#[tokio::test]
async fn the_central_directory_is_read_with_range_requests_only() {
    let bytes = stored_zip();
    let total = bytes.len() as u64;
    let fixture = Fixture::new(bytes);
    let zip = open(&fixture, &Url::parse("https://example.test/a.zip").unwrap())
        .await
        .unwrap();
    let names: Vec<String> = zip
        .entries()
        .iter()
        .map(|entry| entry.name.clone())
        .collect();
    assert_eq!(names, vec!["a.txt".to_string(), "dir/b.bin".to_string()]);
    assert_eq!(zip.entries()[0].uncompressed_size, 5);
    assert_eq!(zip.entries()[0].method, ZipMethod::Stored);
    // No request may be larger than the tail window: that is the whole point.
    const TAIL: u64 = 65_557;
    for (start, end) in fixture.ranges() {
        assert!(
            end - start < TAIL,
            "a range asked for {} bytes of a {total}-byte archive",
            end - start + 1
        );
    }
    // The tail and the central directory are what gets read.
    assert!(fixture.ranges().len() <= 3, "{:?}", fixture.ranges());
}

#[tokio::test]
async fn something_that_is_not_a_zip_is_reported() {
    let fixture = Fixture::new(vec![0u8; 4096]);
    let refused = open(&fixture, &Url::parse("https://example.test/a.zip").unwrap()).await;
    assert!(matches!(refused, Err(ZipError::NotAZip)));
}

#[tokio::test]
async fn a_short_body_is_not_mistaken_for_a_zip() {
    let fixture = Fixture::new(vec![0x50, 0x4b]);
    let refused = open(&fixture, &Url::parse("https://example.test/a.zip").unwrap()).await;
    assert!(matches!(refused, Err(ZipError::NotAZip)));
}

#[tokio::test]
async fn a_large_archive_is_read_from_its_tail_not_from_zero() {
    // A 300 KB entry pushes the archive past the tail window, so the end record can only be
    // found by ranging the tail. Reading from zero would still work on a small fixture and
    // hide the difference.
    let mut bytes = stored_zip();
    let filler = vec![0u8; 300_000];
    // Append a stored entry by rebuilding with the filler included is overkill; instead the
    // archive is padded with a comment, which moves the end record to the tail.
    let comment_len = 300_000usize.min(u16::MAX as usize);
    let comment = vec![b'c'; comment_len];
    let eocd_at = bytes.len() - 22;
    bytes[eocd_at + 20..eocd_at + 22].copy_from_slice(&(comment_len as u16).to_le_bytes());
    bytes.extend_from_slice(&comment);
    let total = bytes.len() as u64;
    let fixture = Fixture::new(bytes);
    let zip = open(
        &fixture,
        &Url::parse("https://example.test/big.zip").unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(zip.entries().len(), 2);
    let ranges = fixture.ranges();
    assert!(
        ranges.iter().any(|(start, _)| *start > 0),
        "the tail must be read from an offset: {ranges:?}"
    );
    for (start, end) in ranges {
        assert!(end - start + 1 < total, "{start}-{end} of {total}");
    }
    let _ = filler;
}

#[tokio::test]
async fn one_entry_is_read_by_range_and_its_crc_is_checked() {
    let bytes = stored_zip();
    let fixture = Fixture::new(bytes);
    let zip = open(&fixture, &Url::parse("https://example.test/a.zip").unwrap())
        .await
        .unwrap();
    let entry = zip.entries()[0].clone();
    let before = fixture.ranges().len();
    let data = segmenta_core::remotezip::read_entry(&fixture, &zip, &entry)
        .await
        .unwrap();
    assert_eq!(data, b"hello");
    // Two requests: the local header and the entry data.
    let asked: Vec<(u64, u64)> = fixture.ranges().split_off(before);
    assert_eq!(asked.len(), 2, "{asked:?}");
    assert!(
        asked[1].1 - asked[1].0 + 1 < 211,
        "only the entry, not the archive"
    );
}

#[tokio::test]
async fn a_corrupted_entry_fails_its_crc_instead_of_returning_bytes() {
    let mut bytes = stored_zip();
    // Flip one byte of the first entry's payload: the CRC in the header no longer matches.
    let payload_at = bytes
        .windows(5)
        .position(|window| window == b"hello")
        .expect("payload");
    bytes[payload_at] ^= 0xFF;
    let fixture = Fixture::new(bytes);
    let zip = open(&fixture, &Url::parse("https://example.test/a.zip").unwrap())
        .await
        .unwrap();
    let refused = segmenta_core::remotezip::read_entry(&fixture, &zip, &zip.entries()[0]).await;
    assert!(
        matches!(
            refused,
            Err(segmenta_core::remotezip::ZipError::Corrupt { .. })
        ),
        "{refused:?}"
    );
}

/// Corrupted archives must come back as errors, never as a panic: a parser that dies takes
/// the whole task with it and leaves the user with no message.
#[tokio::test]
async fn malformed_archives_never_panic() {
    let base = stored_zip();
    let url = Url::parse("https://example.test/a.zip").unwrap();
    let mut cases: Vec<Vec<u8>> = Vec::new();
    // Every truncation length that matters, not just one.
    for cut in [1usize, 4, 21, 22, 40, base.len() / 2, base.len() - 1] {
        cases.push(base[..cut.min(base.len())].to_vec());
    }
    // A count that claims far more entries than the directory holds.
    let mut lying = base.clone();
    let eocd = lying.len() - 22;
    lying[eocd + 10..eocd + 12].copy_from_slice(&u16::MAX.to_le_bytes());
    cases.push(lying);
    // A directory offset past the end of the body.
    let mut far = base.clone();
    far[eocd + 16..eocd + 20].copy_from_slice(&u32::MAX.to_le_bytes());
    cases.push(far);
    // A ZIP64 marker with no locator behind it.
    let mut zip64 = base.clone();
    zip64[eocd + 10..eocd + 12].copy_from_slice(&u16::MAX.to_le_bytes());
    zip64[eocd + 12..eocd + 16].copy_from_slice(&u32::MAX.to_le_bytes());
    cases.push(zip64);
    // Pure noise that happens to carry the signature.
    let mut noise = vec![0u8; 512];
    noise[..4].copy_from_slice(&0x0605_4b50u32.to_le_bytes());
    cases.push(noise);

    for case in cases {
        let fixture = Fixture::new(case);
        // Both outcomes are acceptable; a panic is not.
        let _ = open(&fixture, &url).await;
    }
}
