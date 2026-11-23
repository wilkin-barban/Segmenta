//! Global download speed limiter.
//!
//! One token bucket is shared by every worker of every task: the limit is global, not
//! per connection. A rate of `None`/`0` means unlimited.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tokio::sync::Notify;

/// Tokens accumulated between two refills, capped by the burst capacity.
pub fn refill(tokens: f64, rate_bps: f64, elapsed: Duration, capacity: f64) -> f64 {
    (tokens + rate_bps * elapsed.as_secs_f64()).min(capacity)
}

/// Burst capacity for a rate: a quarter second of traffic, with a 1 KB floor so slow
/// limits do not starve the workers.
pub fn capacity_for(rate_bps: u64) -> f64 {
    (rate_bps as f64 / 4.0).max(1000.0)
}

struct BucketState {
    tokens: f64,
    last: Instant,
}

/// Shared token bucket. `0` means "no limit" and makes [`TokenBucket::acquire`] return
/// immediately.
pub struct TokenBucket {
    rate: AtomicU64,
    state: Mutex<BucketState>,
    rate_changed: Notify,
}

impl TokenBucket {
    /// Create a bucket, unlimited when `rate_bps` is `None` or `0`.
    pub fn new(rate_bps: Option<u64>) -> Self {
        let rate = rate_bps.unwrap_or(0);
        TokenBucket {
            rate: AtomicU64::new(rate),
            state: Mutex::new(BucketState {
                tokens: if rate == 0 { 0.0 } else { capacity_for(rate) },
                last: Instant::now(),
            }),
            rate_changed: Notify::new(),
        }
    }

    /// Change the limit while downloads are running.
    pub fn set_rate(&self, rate_bps: Option<u64>) {
        let rate = rate_bps.unwrap_or(0);
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if self.rate.load(Ordering::Relaxed) == rate {
            return;
        }
        self.rate.store(rate, Ordering::Relaxed);
        state.tokens = state.tokens.min(capacity_for(rate));
        state.last = Instant::now();
        drop(state);
        self.rate_changed.notify_waiters();
    }

    /// Wait until `bytes` may be sent.
    pub async fn acquire(&self, bytes: u64) {
        let mut remaining = bytes as f64;
        while remaining > 0.0 {
            // Register before checking the rate so an update cannot be lost between
            // calculating a sleep and starting to wait.
            let changed = self.rate_changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let wait = {
                let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
                let rate = self.rate.load(Ordering::Relaxed);
                if rate == 0 {
                    return;
                }
                let capacity = capacity_for(rate);
                let now = Instant::now();
                state.tokens = refill(
                    state.tokens,
                    rate as f64,
                    now.saturating_duration_since(state.last),
                    capacity,
                );
                state.last = now;
                let need = remaining.min(capacity);
                if state.tokens >= need {
                    state.tokens -= need;
                    remaining -= need;
                    None
                } else {
                    // Sleep only for the deficit: a rate change is picked up next round.
                    Some(
                        Duration::from_secs_f64((need - state.tokens) / rate as f64)
                            .max(Duration::from_millis(1)),
                    )
                }
            };
            match wait {
                None if remaining <= 0.0 => return,
                None => continue,
                Some(wait) => {
                    tokio::select! {
                        _ = tokio::time::sleep(wait) => {},
                        _ = &mut changed => {},
                    }
                }
            }
        }
    }
}
