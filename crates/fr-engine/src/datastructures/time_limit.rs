//! Port of `datastructures/TimeLimit.java` (cancelling an algorithm after a time limit) and of
//! the stop requests of `core/StoppableThread.java`.
//!
//! A wall-clock limit makes results depend on machine speed. For exact parity runs the engine
//! uses [`TimeLimit::Disabled`] (never fires), and every limit can carry a "fired" flag
//! ([`TimeLimit::with_fired_flag`]) so a run in which any limit actually fired can be flagged
//! as not comparable.

use std::sync::atomic::{AtomicBool, Ordering};
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
}

impl TimeLimit {
    /// Java `new TimeLimit(milliSeconds)`: starts the clock now.
    pub fn wall_clock(milli_seconds: i32) -> Self {
        TimeLimit::WallClock { start: Instant::now(), limit_ms: milli_seconds, fired: None }
    }

    /// A limit that never fires.
    pub fn disabled(milli_seconds: i32) -> Self {
        TimeLimit::Disabled { limit_ms: milli_seconds }
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
        if let TimeLimit::WallClock { fired, .. } = &mut self {
            *fired = Some(flag);
        }
        self
    }

    /// The limit in milliseconds.
    pub fn limit_ms(&self) -> i32 {
        match self {
            TimeLimit::WallClock { limit_ms, .. } | TimeLimit::Disabled { limit_ms } => *limit_ms,
        }
    }

    /// Java `limitExceeded()`: `currentTime - timeStamp > timeLimit` in whole milliseconds.
    pub fn limit_exceeded(&self) -> bool {
        match self {
            TimeLimit::Disabled { .. } => false,
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
        let limit = match self {
            TimeLimit::WallClock { limit_ms, .. } | TimeLimit::Disabled { limit_ms } => limit_ms,
        };
        let mut new_limit = factor * *limit as f64;
        // Java Math.min propagates NaN; (int) NaN == 0.
        if !new_limit.is_nan() {
            new_limit = new_limit.min(i32::MAX as f64);
        }
        *limit = new_limit as i32;
    }

    /// Time left until the limit fires (`None` for a disabled limit).
    pub fn remaining(&self) -> Option<Duration> {
        match self {
            TimeLimit::Disabled { .. } => None,
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
