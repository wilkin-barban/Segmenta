//! Global token bucket arithmetic and pacing. Spec S10.

use std::time::{Duration, Instant};

use segmenta_core::ratelimit::{capacity_for, refill, TokenBucket};

#[test]
fn refill_is_linear_and_capped_by_capacity() {
    // 1 MB/s for half a second yields 512 KB of tokens.
    let tokens = refill(0.0, 1_000_000.0, Duration::from_millis(500), 1_000_000.0);
    assert!((tokens - 500_000.0).abs() < 1.0, "got {tokens}");
    // The bucket never holds more than its capacity.
    let tokens = refill(900_000.0, 1_000_000.0, Duration::from_secs(10), 1_000_000.0);
    assert_eq!(tokens, 1_000_000.0);
    // No elapsed time, no new tokens.
    assert_eq!(refill(10.0, 1_000_000.0, Duration::ZERO, 1_000_000.0), 10.0);
}

#[test]
fn capacity_is_a_quarter_second_of_burst_with_a_floor() {
    assert_eq!(capacity_for(1_000_000), 250_000.0);
    assert_eq!(capacity_for(100), 1_000.0);
}

#[test]
fn unlimited_bucket_does_not_block() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    rt.block_on(async {
        let bucket = TokenBucket::new(None);
        let started = Instant::now();
        bucket.acquire(50 * 1024 * 1024).await;
        assert!(started.elapsed() < Duration::from_millis(50));
    });
}

#[test]
fn limited_bucket_paces_a_burst() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    rt.block_on(async {
        // 1 MB/s with 200 KB outstanding: the burst capacity absorbs the first
        // 250 KB, so acquiring 800 KB must take at least ~0.55 s.
        let bucket = TokenBucket::new(Some(1_000_000));
        let started = Instant::now();
        bucket.acquire(800 * 1024).await;
        let elapsed = started.elapsed();
        assert!(
            elapsed >= Duration::from_millis(500),
            "bucket did not pace: {elapsed:?}"
        );
        assert!(
            elapsed < Duration::from_secs(3),
            "bucket over-paced: {elapsed:?}"
        );
    });
}

#[test]
fn sleeping_low_rate_waiters_wake_when_unlimited() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    rt.block_on(async {
        let bucket = TokenBucket::new(Some(1));
        let first = bucket.acquire(2000);
        let second = bucket.acquire(2000);
        tokio::pin!(first, second);
        assert!(tokio::time::timeout(Duration::from_millis(20), &mut first)
            .await
            .is_err());
        assert!(tokio::time::timeout(Duration::from_millis(20), &mut second)
            .await
            .is_err());
        bucket.set_rate(None);
        tokio::time::timeout(Duration::from_secs(1), async {
            tokio::join!(first, second);
        })
        .await
        .expect("both sleeping waiters must observe the unlimited rate");
    });
}

#[test]
fn rate_can_change_while_downloading() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    rt.block_on(async {
        let bucket = TokenBucket::new(Some(100_000));
        bucket.set_rate(None);
        let started = Instant::now();
        bucket.acquire(4 * 1024 * 1024).await;
        assert!(started.elapsed() < Duration::from_millis(50));
    });
}
