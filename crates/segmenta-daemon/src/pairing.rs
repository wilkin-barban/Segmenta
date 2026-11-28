//! Pairing codes: eight digits, one use, a short life and a small number of attempts.
//!
//! Contract: S4 of `odd/tasks/segmenta-hito2-captura-hosters-remoto.md` (T11).
//!
//! # Why a code and not a token
//!
//! Typing a 32-byte token on a phone keyboard is how tokens end up in chat messages. The
//! code lives ten minutes, works once, and dies after five wrong attempts, so a shoulder
//! surfer has a window measured in minutes instead of the life of the installation.

use std::sync::Arc;
use std::time::Duration;

/// How long a code is valid.
pub const CODE_TTL: Duration = Duration::from_secs(600);

/// Wrong attempts before the code is burned.
pub const MAX_ATTEMPTS: u32 = 5;

/// Digits in a code.
pub const CODE_DIGITS: usize = 8;

/// Why a code was not accepted.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PairError {
    /// No such code: never issued, already used, or burned by attempts.
    #[error("the pairing code is not valid")]
    Unknown,
    /// The code existed but expired.
    #[error("the pairing code expired; generate a new one")]
    Expired,
    /// Too many wrong attempts.
    #[error("too many attempts; generate a new pairing code")]
    Exhausted,
    /// No code was issued at all.
    #[error("no pairing code is active; run `segmenta remote pair`")]
    NoneIssued,
}

/// A clock the tests move by hand.
pub trait Clock: Send + Sync {
    /// Milliseconds since an arbitrary origin.
    fn elapsed_ms(&self) -> u64;
}

/// Production clock.
pub struct MonotonicClock {
    origin: std::time::Instant,
}

impl Default for MonotonicClock {
    fn default() -> Self {
        MonotonicClock {
            origin: std::time::Instant::now(),
        }
    }
}

impl Clock for MonotonicClock {
    fn elapsed_ms(&self) -> u64 {
        self.origin.elapsed().as_millis() as u64
    }
}

/// One outstanding code.
#[derive(Debug, Clone)]
struct Code {
    name: String,
    issued_ms: u64,
    attempts: u32,
}

/// The codes a daemon currently accepts.
pub struct PairingCodes {
    code: Option<Code>,
    clock: Arc<dyn Clock>,
    digits: String,
}

impl std::fmt::Debug for PairingCodes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The code itself is a credential for the next ten minutes.
        f.debug_struct("PairingCodes")
            .field("active", &self.code.is_some())
            .finish()
    }
}

impl PairingCodes {
    /// Empty registry on the monotonic clock.
    pub fn new() -> PairingCodes {
        PairingCodes {
            code: None,
            clock: Arc::new(MonotonicClock::default()),
            digits: String::new(),
        }
    }

    /// Registry with an injected clock, for tests.
    pub fn with_clock(clock: Arc<dyn Clock>) -> PairingCodes {
        PairingCodes {
            code: None,
            clock,
            digits: String::new(),
        }
    }

    /// Issue a new code for `name`, replacing any previous one.
    ///
    /// The digits are random: a predictable code would be worse than no code, because the
    /// user would trust it.
    pub fn issue(&mut self, name: &str) -> String {
        let digits = random_code();
        self.code = Some(Code {
            name: name.to_string(),
            issued_ms: self.clock.elapsed_ms(),
            attempts: 0,
        });
        self.digits = digits.clone();
        digits
    }

    /// The code currently displayed, for the CLI and the dialog to print.
    pub fn current(&self) -> Option<&str> {
        self.code.as_ref().map(|_| self.digits.as_str())
    }

    /// Redeem a code, returning the device name it was issued for.
    pub fn redeem(&mut self, presented: &str) -> Result<String, PairError> {
        let Some(code) = self.code.as_ref() else {
            return Err(PairError::NoneIssued);
        };
        let elapsed = self.clock.elapsed_ms().saturating_sub(code.issued_ms);
        if elapsed >= CODE_TTL.as_millis() as u64 {
            // An expired code is gone: retrying the same digits cannot work.
            self.code = None;
            self.digits.clear();
            return Err(PairError::Expired);
        }
        if presented != self.digits {
            let code = self.code.as_mut().expect("checked above");
            code.attempts += 1;
            if code.attempts >= MAX_ATTEMPTS {
                self.code = None;
                self.digits.clear();
                return Err(PairError::Exhausted);
            }
            return Err(PairError::Unknown);
        }
        let name = code.name.clone();
        // A code works once: keeping it alive would let a second phone pair with a code the
        // user already saw used.
        self.code = None;
        self.digits.clear();
        Ok(name)
    }

    /// How many attempts the current code has seen.
    pub fn attempts(&self) -> u32 {
        self.code.as_ref().map(|code| code.attempts).unwrap_or(0)
    }
}

impl Default for PairingCodes {
    fn default() -> Self {
        PairingCodes::new()
    }
}

/// Eight random digits.
fn random_code() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    (0..CODE_DIGITS)
        .map(|_| char::from(b'0' + rng.gen_range(0..10u8)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed(std::sync::atomic::AtomicU64);

    impl Clock for Fixed {
        fn elapsed_ms(&self) -> u64 {
            self.0.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    fn codes() -> (PairingCodes, Arc<Fixed>) {
        let clock = Arc::new(Fixed(std::sync::atomic::AtomicU64::new(0)));
        (PairingCodes::with_clock(clock.clone()), clock)
    }

    #[test]
    fn a_code_is_eight_digits_and_works_once() {
        let (mut set, _clock) = codes();
        let code = set.issue("phone");
        assert_eq!(code.len(), CODE_DIGITS);
        assert!(code.chars().all(|c| c.is_ascii_digit()));
        assert_eq!(set.redeem(&code), Ok("phone".to_string()));
        assert_eq!(set.redeem(&code), Err(PairError::NoneIssued));
    }

    #[test]
    fn an_expired_code_is_refused_and_disappears() {
        let (mut set, clock) = codes();
        let code = set.issue("phone");
        clock.0.fetch_add(
            CODE_TTL.as_millis() as u64,
            std::sync::atomic::Ordering::SeqCst,
        );
        assert_eq!(set.redeem(&code), Err(PairError::Expired));
        assert!(set.current().is_none());
    }

    #[test]
    fn five_wrong_attempts_burn_the_code() {
        let (mut set, _clock) = codes();
        let code = set.issue("phone");
        for _ in 0..MAX_ATTEMPTS - 1 {
            assert_eq!(set.redeem("00000000"), Err(PairError::Unknown));
        }
        assert_eq!(set.redeem("00000000"), Err(PairError::Exhausted));
        // Even the right digits cannot revive a burned code.
        assert_eq!(set.redeem(&code), Err(PairError::NoneIssued));
    }

    #[test]
    fn a_new_code_replaces_the_previous_one() {
        let (mut set, _clock) = codes();
        let first = set.issue("phone");
        let second = set.issue("tablet");
        assert_ne!(first, second);
        assert_eq!(set.current(), Some(second.as_str()));
        assert_eq!(set.redeem(&first), Err(PairError::Unknown));
        assert_eq!(set.redeem(&second), Ok("tablet".to_string()));
    }

    #[test]
    fn the_debug_never_prints_the_code() {
        let (mut set, _clock) = codes();
        let code = set.issue("phone");
        let printed = format!("{set:?}");
        assert!(!printed.contains(&code), "{printed}");
    }

    #[test]
    fn a_wrong_attempt_is_counted_without_consuming_the_code() {
        let (mut set, _clock) = codes();
        let code = set.issue("phone");
        assert_eq!(set.attempts(), 0);
        let _ = set.redeem("11111111");
        assert_eq!(set.attempts(), 1);
        assert_eq!(set.redeem(&code), Ok("phone".to_string()));
    }
}
