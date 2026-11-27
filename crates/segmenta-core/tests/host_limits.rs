//! Per-host adaptive connection limits (T7): the ceiling tightens on 429/503 and recovers
//! one slot at a time as the penalty decays. The clock is injected so no test sleeps.

use segmenta_core::hostlimits::{HostLimitPolicy, HostLimits, MAX_CONNECTIONS};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// A clock the test moves by hand.
#[derive(Default)]
struct FakeClock(AtomicU64);

impl FakeClock {
    fn advance(&self, by: Duration) {
        self.0.fetch_add(by.as_millis() as u64, Ordering::SeqCst);
    }
}

impl segmenta_core::hostlimits::Clock for FakeClock {
    fn elapsed_ms(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

fn harness(initial: usize, ceiling: usize) -> (HostLimits, Arc<FakeClock>) {
    let clock = Arc::new(FakeClock::default());
    let limits = HostLimits::new(
        HostLimitPolicy {
            initial,
            ceiling,
            penalty: Duration::from_secs(30),
            decay: Duration::from_secs(10),
        },
        clock.clone(),
    );
    (limits, clock)
}

#[test]
fn a_fresh_host_starts_at_the_initial_limit_clamped_to_the_ceiling() {
    let (limits, _clock) = harness(6, MAX_CONNECTIONS);
    assert_eq!(limits.limit("example.com"), 6);
    // The ceiling is a hard bound, not a suggestion.
    let (over, _clock) = harness(10_000, MAX_CONNECTIONS);
    assert_eq!(over.limit("example.com"), MAX_CONNECTIONS);
    // And a zero from a stored rule means one, never none: a host with zero would stall.
    let (zero, _clock) = harness(0, MAX_CONNECTIONS);
    assert_eq!(zero.limit("example.com"), 1);
}

#[test]
fn a_429_or_503_halves_the_limit_and_never_reaches_zero() {
    let (limits, _clock) = harness(8, MAX_CONNECTIONS);
    limits.on_throttled("example.com");
    assert_eq!(limits.limit("example.com"), 4);
    limits.on_throttled("example.com");
    assert_eq!(limits.limit("example.com"), 2);
    limits.on_throttled("example.com");
    assert_eq!(limits.limit("example.com"), 1);
    limits.on_throttled("example.com");
    assert_eq!(
        limits.limit("example.com"),
        1,
        "one connection is the floor"
    );
}

#[test]
fn the_limit_recovers_one_slot_per_decay_period() {
    let (limits, clock) = harness(8, MAX_CONNECTIONS);
    limits.on_throttled("example.com");
    limits.on_throttled("example.com");
    assert_eq!(limits.limit("example.com"), 2);

    // The penalty itself must pass first: recovery starts after it, one decay at a time.
    clock.advance(Duration::from_secs(39));
    assert_eq!(limits.limit("example.com"), 2, "still inside the penalty");

    clock.advance(Duration::from_secs(1));
    assert_eq!(
        limits.limit("example.com"),
        3,
        "one slot after the first decay"
    );

    // Enough periods restore the initial value and no more.
    clock.advance(Duration::from_secs(120));
    assert_eq!(limits.limit("example.com"), 8);
}

#[test]
fn a_penalty_expires_and_the_host_returns_to_its_initial_limit() {
    let (limits, clock) = harness(4, MAX_CONNECTIONS);
    limits.on_throttled("example.com");
    assert_eq!(limits.limit("example.com"), 2);
    // Past the penalty and every decay period, nothing is left holding it down.
    clock.advance(Duration::from_secs(30 + 10 * 4));
    assert_eq!(limits.limit("example.com"), 4);
}

#[test]
fn hosts_do_not_inherit_each_others_penalties() {
    let (limits, _clock) = harness(8, MAX_CONNECTIONS);
    limits.on_throttled("slow.example");
    assert_eq!(limits.limit("slow.example"), 4);
    assert_eq!(limits.limit("fast.example"), 8);
    // A subdomain is a different host: the engine keys by exact host.
    assert_eq!(limits.limit("cdn.slow.example"), 8);
}

#[test]
fn the_ceiling_accepts_at_most_one_hundred_and_twenty_eight() {
    assert_eq!(MAX_CONNECTIONS, 128);
    let (limits, _clock) = harness(MAX_CONNECTIONS, MAX_CONNECTIONS);
    assert_eq!(limits.limit("example.com"), 128);
    // A policy that asks for more is clamped, so no caller can widen it.
    let (wide, _clock) = harness(MAX_CONNECTIONS + 50, MAX_CONNECTIONS + 50);
    assert_eq!(wide.limit("example.com"), MAX_CONNECTIONS);
}

#[test]
fn a_tracked_host_can_be_forgotten() {
    let (limits, _clock) = harness(8, MAX_CONNECTIONS);
    limits.on_throttled("example.com");
    assert!(limits.tracked() >= 1);
    limits.forget("example.com");
    assert_eq!(limits.limit("example.com"), 8, "the penalty is gone");
}
