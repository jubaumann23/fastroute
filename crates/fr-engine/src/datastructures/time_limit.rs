//! Port of `datastructures/TimeLimit.java` (cancelling an algorithm after a time limit) and of
//! the stop requests of `core/StoppableThread.java`.
//!
//! A wall-clock limit makes results depend on machine speed. For exact parity runs the engine
//! uses [`TimeLimit::Count`], the deterministic budget of the Java parity build
//! (`docs/parity/TimeLimit.patch`, `-Dfreerouting.parity.timeLimitMode=count`, selected by
//! `-Dfreerouting.parity.disableTimeLimits=true`): the limit fires once `limitExceeded()` has
//! been called more than `limit_ms * factor` times on the instance (Java object identity =
//! the clones of one `TimeLimit` share the counter). [`TimeLimit::Disabled`] never fires (the
//! replay vectors were generated like that). Every limit can carry a "fired" flag
//! ([`TimeLimit::with_fired_flag`]) so a run in which any limit actually fired can be flagged.

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Pluggable time budget replacing Java `TimeLimit`.
#[derive(Clone, Debug)]
pub enum TimeLimit {
    /// Java `new TimeLimit(milliSeconds)`: exceeded when more than `limit_ms` milliseconds have
    /// passed since `start`.
    WallClock {
        start: Instant,
        limit_ms: i32,
        fired: Option<Arc<AtomicBool>>,
    },
    /// Never exceeded (deterministic mode). Java code paths that pass `null` instead of a
    /// `TimeLimit` should use `Option<TimeLimit>`/`Disabled` as appropriate.
    Disabled {
        /// Kept so `limit_ms`/`multiply` behave like for a wall clock limit.
        limit_ms: i32,
    },
    /// Deterministic call budget (Java parity build, count mode): exceeded once
    /// [`limit_exceeded`](Self::limit_exceeded) has been called more than
    /// `max(0, limit_ms) * factor` times on this instance or its clones.
    Count {
        state: Arc<CountState>,
        factor: i64,
        fired: Option<Arc<AtomicBool>>,
    },
}

/// The shared state of a [`TimeLimit::Count`] (the Java object's `timeLimit` and `calls`).
#[derive(Debug)]
pub struct CountState {
    limit_ms: AtomicI32,
    calls: AtomicI64,
    reported: AtomicBool,
}

/// The default factor of the count mode (Java `freerouting.parity.timeLimitFactor`).
pub const DEFAULT_COUNT_FACTOR: i64 = 10;

impl TimeLimit {
    /// Java `new TimeLimit(milliSeconds)`: starts the clock now.
    pub fn wall_clock(milli_seconds: i32) -> Self {
        TimeLimit::WallClock { start: Instant::now(), limit_ms: milli_seconds, fired: None }
    }

    /// A limit that never fires.
    pub fn disabled(milli_seconds: i32) -> Self {
        TimeLimit::Disabled { limit_ms: milli_seconds }
    }

    /// A deterministic call budget of `milli_seconds * factor` calls.
    pub fn count(milli_seconds: i32, factor: i64) -> Self {
        TimeLimit::Count {
            state: Arc::new(CountState {
                limit_ms: AtomicI32::new(milli_seconds),
                calls: AtomicI64::new(0),
                reported: AtomicBool::new(false),
            }),
            factor,
            fired: None,
        }
    }

    /// Creates a wall clock limit if `enabled`, otherwise a disabled one.
    pub fn new(milli_seconds: i32, enabled: bool) -> Self {
        if enabled {
            Self::wall_clock(milli_seconds)
        } else {
            Self::disabled(milli_seconds)
        }
    }

    /// Attaches a flag that is set to `true` whenever [`limit_exceeded`](Self::limit_exceeded)
    /// returns true (parity flagging). Several limits may share one flag.
    pub fn with_fired_flag(mut self, flag: Arc<AtomicBool>) -> Self {
        match &mut self {
            TimeLimit::WallClock { fired, .. } | TimeLimit::Count { fired, .. } => *fired = Some(flag),
            TimeLimit::Disabled { .. } => {}
        }
        self
    }

    /// The limit in milliseconds.
    pub fn limit_ms(&self) -> i32 {
        match self {
            TimeLimit::WallClock { limit_ms, .. } | TimeLimit::Disabled { limit_ms } => *limit_ms,
            TimeLimit::Count { state, .. } => state.limit_ms.load(Ordering::Relaxed),
        }
    }

    /// Number of [`limit_exceeded`](Self::limit_exceeded) calls of a count limit (0 otherwise).
    pub fn calls(&self) -> i64 {
        match self {
            TimeLimit::Count { state, .. } => state.calls.load(Ordering::Relaxed),
            _ => 0,
        }
    }

    /// Java `limitExceeded()`: `currentTime - timeStamp > timeLimit` in whole milliseconds.
    pub fn limit_exceeded(&self) -> bool {
        match self {
            TimeLimit::Disabled { .. } => false,
            TimeLimit::Count { state, factor, fired } => {
                let calls = state.calls.fetch_add(1, Ordering::Relaxed) + 1;
                let limit = state.limit_ms.load(Ordering::Relaxed);
                let exceeded = calls > (limit.max(0) as i64).wrapping_mul(*factor);
                if exceeded {
                    if let Some(flag) = fired {
                        flag.store(true, Ordering::Relaxed);
                    }
                    if !state.reported.swap(true, Ordering::Relaxed) {
                        log::warn!(target: "parity", "PARITY_TIME_LIMIT mode=count limit_ms={limit} calls={calls}");
                    }
                }
                exceeded
            }
            TimeLimit::WallClock { start, limit_ms, fired } => {
                let elapsed_ms = start.elapsed().as_millis().min(i64::MAX as u128) as i64;
                let exceeded = elapsed_ms > *limit_ms as i64;
                if exceeded {
                    if let Some(flag) = fired {
                        flag.store(true, Ordering::Relaxed);
                    }
                }
                exceeded
            }
        }
    }

    /// Java `multiply(factor)`: `limit = (int) min(factor * limit, Integer.MAX_VALUE)`; ignored
    /// for `factor <= 0`.
    pub fn multiply(&mut self, factor: f64) {
        if factor <= 0.0 {
            return;
        }
        let mut new_limit = factor * self.limit_ms() as f64;
        // Java Math.min propagates NaN; (int) NaN == 0.
        if !new_limit.is_nan() {
            new_limit = new_limit.min(i32::MAX as f64);
        }
        match self {
            TimeLimit::WallClock { limit_ms, .. } | TimeLimit::Disabled { limit_ms } => *limit_ms = new_limit as i32,
            // (shared with the clones, like the Java object)
            TimeLimit::Count { state, .. } => state.limit_ms.store(new_limit as i32, Ordering::Relaxed),
        }
    }

    /// Time left until the limit fires (`None` for a disabled limit).
    pub fn remaining(&self) -> Option<Duration> {
        match self {
            TimeLimit::Disabled { .. } | TimeLimit::Count { .. } => None,
            TimeLimit::WallClock { start, limit_ms, .. } => {
                let limit = Duration::from_millis((*limit_ms).max(0) as u64);
                Some(limit.saturating_sub(start.elapsed()))
            }
        }
    }
}

/// Stop requests of a running job (Java `StoppableThread`: state NONE / AUTO_ROUTER_ONLY / ALL).
///
/// Cloning shares the flags, so a controller can keep a clone and request a stop.
#[derive(Clone, Debug, Default)]
pub struct StopToken {
    stop: Arc<AtomicBool>,
    stop_autorouter: Arc<AtomicBool>,
}

impl StopToken {
    pub fn new() -> Self {
        Self::default()
    }

    /// A token that shares this token's full stop (`request_stop`) but has its own
    /// autorouter stop: parallel autorouter runs stop on their own stagnation rules without
    /// stopping each other, and all of them stop on a job stop (time limit, Ctrl+C).
    pub fn child(&self) -> Self {
        StopToken { stop: self.stop.clone(), stop_autorouter: Arc::new(AtomicBool::new(false)) }
    }

    /// Java `requestStop()`: stops everything (fanout, autorouter and optimizer).
    pub fn request_stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
        self.stop_autorouter.store(true, Ordering::SeqCst);
    }

    /// Java `isStopRequested()`.
    pub fn is_stop_requested(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }

    /// Java `requestStopAutoRouter()`: stops the autorouter, other tasks continue.
    pub fn request_stop_autorouter(&self) {
        self.stop_autorouter.store(true, Ordering::SeqCst);
    }

    /// Withdraws a `request_stop_autorouter` (a full `request_stop` stays in effect).
    pub fn clear_stop_autorouter(&self) {
        self.stop_autorouter.store(false, Ordering::SeqCst);
    }

    /// Java `isStopAutoRouterRequested()`: true after either request.
    pub fn is_stop_autorouter_requested(&self) -> bool {
        self.stop_autorouter.load(Ordering::SeqCst) || self.stop.load(Ordering::SeqCst)
    }
}

impl fr_geom::Stoppable for StopToken {
    fn request_stop(&self) {
        StopToken::request_stop(self)
    }

    fn is_stop_requested(&self) -> bool {
        StopToken::is_stop_requested(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_never_fires() {
        let t = TimeLimit::disabled(-5);
        assert!(!t.limit_exceeded());
        assert_eq!(t.remaining(), None);
    }

    #[test]
    fn wall_clock_negative_limit_fires_and_sets_flag() {
        let flag = Arc::new(AtomicBool::new(false));
        let t = TimeLimit::wall_clock(-1).with_fired_flag(flag.clone());
        assert!(t.limit_exceeded());
        assert!(flag.load(Ordering::Relaxed));
    }

    #[test]
    fn wall_clock_large_limit_does_not_fire() {
        let flag = Arc::new(AtomicBool::new(false));
        let t = TimeLimit::wall_clock(1_000_000).with_fired_flag(flag.clone());
        assert!(!t.limit_exceeded());
        assert!(!flag.load(Ordering::Relaxed));
    }

    #[test]
    fn multiply_like_java() {
        let mut t = TimeLimit::disabled(1000);
        t.multiply(0.0);
        assert_eq!(t.limit_ms(), 1000);
        t.multiply(-2.0);
        assert_eq!(t.limit_ms(), 1000);
        t.multiply(2.5);
        assert_eq!(t.limit_ms(), 2500);
        t.multiply(1.0e12);
        assert_eq!(t.limit_ms(), i32::MAX);
        let mut t = TimeLimit::disabled(3);
        t.multiply(0.1);
        assert_eq!(t.limit_ms(), 0); // (int) 0.30000000000000004
        t.multiply(f64::NAN);
        assert_eq!(t.limit_ms(), 0);
        let mut t = TimeLimit::disabled(7);
        t.multiply(f64::NAN);
        assert_eq!(t.limit_ms(), 0);
    }

    #[test]
    fn count_mode_budget_is_shared_by_clones() {
        let flag = Arc::new(AtomicBool::new(false));
        let t = TimeLimit::count(3, 10).with_fired_flag(flag.clone());
        let c = t.clone();
        for i in 0..30 {
            // calls 1..=30 do not exceed 3 * 10
            let x = if i % 2 == 0 { &t } else { &c };
            assert!(!x.limit_exceeded());
        }
        assert!(!flag.load(Ordering::Relaxed));
        assert!(c.limit_exceeded()); // call 31
        assert!(t.limit_exceeded());
        assert!(flag.load(Ordering::Relaxed));
        assert_eq!(t.calls(), 32);
        // independent instance
        let u = TimeLimit::count(0, 10);
        assert!(u.limit_exceeded()); // 1 > 0
        let mut v = TimeLimit::count(1, 10);
        let w = v.clone();
        v.multiply(2.0);
        assert_eq!(w.limit_ms(), 2);
        let n = TimeLimit::count(-5, 10);
        assert!(n.limit_exceeded()); // max(0, -5) * 10 = 0
    }

    #[test]
    fn stop_token_states() {
        let s = StopToken::new();
        let c = s.clone();
        assert!(!s.is_stop_requested() && !s.is_stop_autorouter_requested());
        c.request_stop_autorouter();
        assert!(!s.is_stop_requested() && s.is_stop_autorouter_requested());
        c.request_stop();
        assert!(s.is_stop_requested() && s.is_stop_autorouter_requested());
    }
}
