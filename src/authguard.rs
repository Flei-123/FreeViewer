//! Brute-force protection for the host's password proof check.
//!
//! Every proof check costs up to `pwlist::MAX + 1` Argon2id runs (tens of
//! MB of RAM and a noticeable slice of CPU each). Without a limit an
//! attacker could guess passwords at relay speed and burn the host's CPU.
//!
//! Two layers, both pure logic (clock is passed in, so it is unit-testable):
//! - per viewer key (the address the relay reports): after `FAILS_BEFORE_LOCK`
//!   wrong proofs the key is locked out, with exponentially growing backoff
//!   up to `MAX_LOCK`.
//! - global budget: at most `GLOBAL_RUNS` proof checks per `GLOBAL_WINDOW`
//!   from anyone. This bounds the Argon2 CPU/RAM load even when the attacker
//!   rotates addresses (or the relay hides them behind one proxy address).
//!
//! A correct proof clears the failure state of its key.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Wrong proofs in a row before a key is locked.
pub const FAILS_BEFORE_LOCK: u32 = 5;
/// First lock duration; doubles with every further failure.
pub const BASE_LOCK: Duration = Duration::from_secs(30);
/// Upper bound for a lock.
pub const MAX_LOCK: Duration = Duration::from_secs(15 * 60);
/// Proof checks (each up to 11 Argon2 runs) allowed per window, all keys.
pub const GLOBAL_RUNS: usize = 12;
pub const GLOBAL_WINDOW: Duration = Duration::from_secs(60);
/// Forget keys that were quiet this long (also bounds memory).
const FORGET_AFTER: Duration = Duration::from_secs(60 * 60);
const MAX_KEYS: usize = 512;

#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Go ahead and run Argon2.
    Allow,
    /// This key is locked for the given time.
    Locked(Duration),
    /// Too many checks overall; try again after the given time.
    Busy(Duration),
}

struct KeyState {
    fails: u32,
    locked_until: Option<Instant>,
    last_seen: Instant,
}

#[derive(Default)]
pub struct AuthGuard {
    keys: HashMap<String, KeyState>,
    runs: Vec<Instant>,
}

impl AuthGuard {
    pub fn new() -> Self {
        Self::default()
    }

    /// Lock duration after `fails` failures in a row (0 below the threshold).
    pub fn lock_for(fails: u32) -> Duration {
        if fails < FAILS_BEFORE_LOCK {
            return Duration::ZERO;
        }
        let shift = (fails - FAILS_BEFORE_LOCK).min(16);
        let d = BASE_LOCK.saturating_mul(1u32 << shift);
        d.min(MAX_LOCK)
    }

    /// Ask before running Argon2. On `Allow` the run is counted against the
    /// global budget already.
    pub fn check(&mut self, key: &str, now: Instant) -> Verdict {
        self.prune(now);
        if let Some(st) = self.keys.get_mut(key) {
            st.last_seen = now;
            if let Some(until) = st.locked_until {
                if until > now {
                    return Verdict::Locked(until - now);
                }
            }
        }
        self.runs.retain(|t| now.duration_since(*t) < GLOBAL_WINDOW);
        if self.runs.len() >= GLOBAL_RUNS {
            let oldest = self.runs[0];
            let wait = GLOBAL_WINDOW.saturating_sub(now.duration_since(oldest));
            return Verdict::Busy(wait.max(Duration::from_secs(1)));
        }
        self.runs.push(now);
        Verdict::Allow
    }

    /// A wrong proof. Returns the new lock duration if this tripped one.
    pub fn failure(&mut self, key: &str, now: Instant) -> Option<Duration> {
        if self.keys.len() >= MAX_KEYS && !self.keys.contains_key(key) {
            // Table full: drop the oldest entry instead of growing.
            if let Some(oldest) = self
                .keys
                .iter()
                .min_by_key(|(_, s)| s.last_seen)
                .map(|(k, _)| k.clone())
            {
                self.keys.remove(&oldest);
            }
        }
        let st = self.keys.entry(key.to_string()).or_insert(KeyState {
            fails: 0,
            locked_until: None,
            last_seen: now,
        });
        st.fails = st.fails.saturating_add(1);
        st.last_seen = now;
        let d = Self::lock_for(st.fails);
        if d.is_zero() {
            None
        } else {
            st.locked_until = Some(now + d);
            Some(d)
        }
    }

    /// A correct proof clears the key.
    pub fn success(&mut self, key: &str) {
        self.keys.remove(key);
    }

    fn prune(&mut self, now: Instant) {
        self.keys.retain(|_, s| {
            let locked = s.locked_until.map(|u| u > now).unwrap_or(false);
            locked || now.duration_since(s.last_seen) < FORGET_AFTER
        });
    }
}

static GUARD: OnceLock<Mutex<AuthGuard>> = OnceLock::new();

/// Process-wide guard (survives relay reconnects of the host loop).
pub fn global() -> std::sync::MutexGuard<'static, AuthGuard> {
    GUARD
        .get_or_init(|| Mutex::new(AuthGuard::new()))
        .lock()
        .unwrap_or_else(|p| p.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn lock_grows_and_is_capped() {
        assert_eq!(AuthGuard::lock_for(0), Duration::ZERO);
        assert_eq!(AuthGuard::lock_for(FAILS_BEFORE_LOCK - 1), Duration::ZERO);
        assert_eq!(AuthGuard::lock_for(FAILS_BEFORE_LOCK), BASE_LOCK);
        assert_eq!(AuthGuard::lock_for(FAILS_BEFORE_LOCK + 1), BASE_LOCK * 2);
        assert_eq!(AuthGuard::lock_for(FAILS_BEFORE_LOCK + 2), BASE_LOCK * 4);
        assert_eq!(AuthGuard::lock_for(1000), MAX_LOCK);
    }

    #[test]
    fn locks_after_threshold_and_unlocks_later() {
        let mut g = AuthGuard::new();
        let t0 = Instant::now();
        for i in 0..FAILS_BEFORE_LOCK {
            assert_eq!(g.check("a", t0 + secs(i as u64)), Verdict::Allow);
            let r = g.failure("a", t0 + secs(i as u64));
            assert_eq!(r.is_some(), i + 1 == FAILS_BEFORE_LOCK);
        }
        let t = t0 + secs(10);
        match g.check("a", t) {
            Verdict::Locked(d) => assert!(d <= BASE_LOCK && d > Duration::ZERO),
            v => panic!("expected lock, got {:?}", v),
        }
        // other keys are unaffected
        assert_eq!(g.check("b", t), Verdict::Allow);
        // after the lock expired it is allowed again
        assert_eq!(g.check("a", t0 + secs(4) + BASE_LOCK + secs(1)), Verdict::Allow);
    }

    #[test]
    fn next_failure_after_lock_locks_longer() {
        let mut g = AuthGuard::new();
        let t0 = Instant::now();
        for _ in 0..FAILS_BEFORE_LOCK {
            g.failure("a", t0);
        }
        let t1 = t0 + BASE_LOCK + secs(1);
        assert_eq!(g.check("a", t1), Verdict::Allow);
        assert_eq!(g.failure("a", t1), Some(BASE_LOCK * 2));
    }

    #[test]
    fn success_clears_failures() {
        let mut g = AuthGuard::new();
        let t0 = Instant::now();
        for _ in 0..FAILS_BEFORE_LOCK - 1 {
            g.failure("a", t0);
        }
        g.success("a");
        for _ in 0..FAILS_BEFORE_LOCK - 1 {
            assert_eq!(g.failure("a", t0), None);
        }
    }

    #[test]
    fn global_budget_limits_argon2_runs_across_keys() {
        let mut g = AuthGuard::new();
        let t0 = Instant::now();
        for i in 0..GLOBAL_RUNS {
            assert_eq!(g.check(&format!("k{}", i), t0), Verdict::Allow);
        }
        match g.check("another", t0 + secs(1)) {
            Verdict::Busy(w) => assert!(w <= GLOBAL_WINDOW),
            v => panic!("expected busy, got {:?}", v),
        }
        // window passes -> allowed again
        assert_eq!(g.check("another", t0 + GLOBAL_WINDOW + secs(1)), Verdict::Allow);
    }

    #[test]
    fn locked_check_does_not_use_global_budget() {
        let mut g = AuthGuard::new();
        let t0 = Instant::now();
        for _ in 0..FAILS_BEFORE_LOCK {
            g.failure("a", t0);
        }
        for _ in 0..100 {
            assert!(matches!(g.check("a", t0 + secs(1)), Verdict::Locked(_)));
        }
        assert_eq!(g.check("b", t0 + secs(1)), Verdict::Allow);
    }

    #[test]
    fn key_table_is_bounded() {
        let mut g = AuthGuard::new();
        let t0 = Instant::now();
        for i in 0..(MAX_KEYS + 50) {
            g.failure(&format!("k{}", i), t0 + Duration::from_millis(i as u64));
        }
        assert!(g.keys.len() <= MAX_KEYS);
    }
}
