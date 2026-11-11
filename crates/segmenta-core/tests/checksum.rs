//! Final integrity checks. Spec S9.

use std::fs::File;
use std::io::Write;

use segmenta_core::checksum::{sha256_file, sha256_hex};

#[test]
fn sha256_of_known_bytes_matches_the_reference_digest() {
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[tokio::test]
async fn file_digest_matches_an_in_memory_digest_across_block_boundaries() {
    // 3 MiB + 17 bytes: forces several 1 MiB blocks and a short tail.
    let mut payload = Vec::with_capacity(3 * 1024 * 1024 + 17);
    for i in 0..(3 * 1024 * 1024 + 17) {
        payload.push((i % 251) as u8);
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("payload.bin");
    let mut file = File::create(&path).unwrap();
    file.write_all(&payload).unwrap();
    file.sync_all().unwrap();

    let digest = sha256_file(&path).await.unwrap();
    assert_eq!(digest, sha256_hex(&payload));
}

#[tokio::test]
async fn digest_of_an_empty_file_is_the_empty_digest() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("empty.bin");
    File::create(&path).unwrap();
    assert_eq!(
        sha256_file(&path).await.unwrap(),
        sha256_hex(b""),
    );
}
