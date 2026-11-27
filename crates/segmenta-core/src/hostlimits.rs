//! Per-host adaptive connection limits.
//!
//! Contract: T7 of `odd/tasks/segmenta-hito1-red-inmunidad.md`.
//!
//! A host that answers 429 or 503 is asking for fewer connections, not for a blind retry.
//! This module keeps that number per host: the first refusal halves the allowance, a floor
//! of one prevents a deadlock, and every `decay` period of calm adds one slot back until
//! the initial value is restored. The ceiling is a hard bound so no stored rule, setting or
//! policy can widen it past [`MAX_CONNECTIONS`].
//!
//! The clock is injected: production reads a monotonic counter, tests move it by hand, and
//! neither sleeps.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Hard upper bound on simultaneous connections to one host.
pub const MAX_CONNECTIONS: usize = 128;

/// What a caller can move in tests: milliseconds since an arbitrary origin.
pub trait Clock: Send + Sync {
    /// Milliseconds elapsed since the clock started.
    fn elapsed_ms(&self) -> u64;
}

/// Production clock: a monotonic instant captured when the limits are created.
pub struct MonotonicClock {
    origin: Instant,
}

impl Default for MonotonicClock {
    fn default() -> Self {
        MonotonicClock {
            origin: Instant::now(),
        }
    }
}

impl Clock for MonotonicClock {
    fn elapsed_ms(&self) -> u64 {
        self.origin.elapsed().as_millis() as u64
    }
}

/// How the allowance behaves over time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostLimitPolicy {
    /// Connections a host may use before any refusal.
    pub initial: usize,
    /// Hard bound, clamped to [`MAX_CONNECTIONS`].
    pub ceiling: usize,
    /// How long a refusal keeps the allowance down before recovery starts.
    pub penalty: Duration,
    /// Time of calm that buys back one slot.
    pub decay: Duration,
}

impl Default for HostLimitPolicy {
    fn default() -> Self {
        HostLimitPolicy {
            initial: 8,
            ceiling: MAX_CONNECTIONS,
            penalty: Duration::from_secs(60),
            decay: Duration::from_secs(15),
        }
    }
}

/// One host's state: the allowance it has left and when it was last pushed down.
#[derive(Debug, Clone, Copy)]
struct HostState {
    limit: usize,
    /// Clock reading of the refusal that set `limit`; `None` before the first one.
    last_penalty_ms: Option<u64>,
    /// Slots already given back since that refusal.
    recovered: usize,
}

/// The registry the engine consults before opening a connection.
pub struct HostLimits {
    policy: HostLimitPolicy,
    clock: Arc<dyn Clock>,
    hosts: Mutex<HashMap<String, HostState>>,
}

impl std::fmt::Debug for HostLimits {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The map is behind a mutex and the clock is opaque; only the policy is useful in
        // a log line, and it never carries host names.
        f.debug_struct("HostLimits")
            .field("policy", &self.policy)
            .field("tracked", &self.tracked())
            .finish()
    }
}

impl HostLimits {
    /// Build the registry. The ceiling is clamped once, here, so every later caller reads
    /// a value that cannot exceed [`MAX_CONNECTIONS`].
    pub fn new(policy: HostLimitPolicy, clock: Arc<dyn Clock>) -> HostLimits {
        let policy = HostLimitPolicy {
            initial: policy.initial.clamp(1, MAX_CONNECTIONS),
            ceiling: policy.ceiling.clamp(1, MAX_CONNECTIONS),
            ..policy
        };
        HostLimits {
            policy,
            clock,
            hosts: Mutex::new(HashMap::new()),
        }
    }

    /// Registry with the default policy on the monotonic clock.
    pub fn with_defaults() -> HostLimits {
        HostLimits::new(
            HostLimitPolicy::default(),
            Arc::new(MonotonicClock::default()),
        )
    }

    /// Policy in force.
    pub fn policy(&self) -> HostLimitPolicy {
        self.policy
    }

    fn states(&self) -> std::sync::MutexGuard<'_, HashMap<String, HostState>> {
        self.hosts.lock().unwrap_or_else(|err| err.into_inner())
    }

    /// Effective allowance for `host` right now.
    pub fn limit(&self, host: &str) -> usize {
        let key = normalize(host);
        let now = self.clock.elapsed_ms();
        let states = self.states();
        match states.get(&key) {
            Some(state) => self.effective(state, now),
            None => self.start(),
        }
    }

    /// Connections a host may use before any refusal.
    fn start(&self) -> usize {
        self.policy.initial.min(self.policy.ceiling).max(1)
    }

    /// Recompute the allowance after the elapsed calm.
    fn effective(&self, state: &HostState, now: u64) -> usize {
        let Some(penalized_at) = state.last_penalty_ms else {
            return self.start();
        };
        let decay_ms = self.policy.decay.as_millis().max(1) as u64;
        let penalty_ms = self.policy.penalty.as_millis() as u64;
        let since = now.saturating_sub(penalized_at);
        // Nothing is given back while the penalty is running.
        let periods = if since < penalty_ms {
            0
        } else {
            ((since - penalty_ms) / decay_ms) as usize
        };
        let banked = state.recovered + periods;
        state.limit.saturating_add(banked).min(self.start())
    }

    /// The host refused with 429 or 503: halve the allowance, never below one.
    ///
    /// Returns the new allowance so the caller can log or shrink its worker pool.
    pub fn on_throttled(&self, host: &str) -> usize {
        let key = normalize(host);
        let now = self.clock.elapsed_ms();
        let start = self.start();
        let mut states = self.states();
        let state = states.entry(key).or_insert(HostState {
            limit: start,
            last_penalty_ms: None,
            recovered: 0,
        });
        // The value in force is materialized before halving, so two quick refusals after a
        // long calm cannot halve a stale number.
        let current = state
            .last_penalty_ms
            .map_or(start, |_| state.limit.saturating_add(state.recovered));
        state.limit = (current / 2).max(1);
        state.recovered = 0;
        state.last_penalty_ms = Some(now);
        state.limit
    }

    /// The host answered well: the only effect is that the calm clock keeps running, so
    /// this is a no-op by design and exists to keep call sites readable.
    pub fn on_success(&self, _host: &str) {}

    /// Hosts currently tracked, for tests and diagnostics.
    pub fn tracked(&self) -> usize {
        self.states().len()
    }

    /// Drop everything learned about `host`.
    pub fn forget(&self, host: &str) {
        self.states().remove(&normalize(host));
    }
}

/// Hosts are compared lowercased and without a trailing dot, the same way DNS reports them.
fn normalize(host: &str) -> String {
    host.trim().trim_end_matches('.').to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed(u64);
    impl Clock for Fixed {
        fn elapsed_ms(&self) -> u64 {
            self.0
        }
    }

    #[test]
    fn an_empty_host_is_tracked_under_the_empty_key_without_panicking() {
        let limits = HostLimits::new(HostLimitPolicy::default(), Arc::new(Fixed(0)));
        assert_eq!(limits.limit(""), 8);
        limits.on_throttled("");
        assert_eq!(limits.limit(""), 4);
        assert_eq!(limits.tracked(), 1);
    }

    #[test]
    fn a_zero_decay_or_penalty_cannot_spin_forever() {
        // A stored policy with zeroes must still make progress instead of dividing by 0.
        // A zero decay is raised to one millisecond: recovery happens, but never in a
        // tight loop.
        struct Movable(std::sync::atomic::AtomicU64);
        impl Clock for Movable {
            fn elapsed_ms(&self) -> u64 {
                self.0.load(std::sync::atomic::Ordering::SeqCst)
            }
        }
        let clock = Arc::new(Movable(std::sync::atomic::AtomicU64::new(1_000)));
        let limits = HostLimits::new(
            HostLimitPolicy {
                initial: 4,
                ceiling: 8,
                penalty: Duration::ZERO,
                decay: Duration::ZERO,
            },
            clock.clone(),
        );
        limits.on_throttled("example.com");
        assert_eq!(
            limits.limit("example.com"),
            2,
            "halved immediately after the refusal"
        );
        clock.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(
            limits.limit("example.com"),
            3,
            "one slot per millisecond at most"
        );
    }
}
