//! Content epochs for result caches (performance only, results are unchanged).
//!
//! An epoch is a process-wide unique number assigned to a data structure whenever its content
//! changes (every `&mut` access that can change it draws a new number). Clones keep the epoch
//! of the original: equal epochs therefore always denote equal content, even across board
//! clones and threads, and a cache keyed by epochs never needs explicit invalidation.

use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_BLOCK: AtomicU64 = AtomicU64::new(1);
const BLOCK: u64 = 1 << 16;

thread_local! {
    /// (next, end) of the block of epochs reserved by this thread.
    static LOCAL: Cell<(u64, u64)> = const { Cell::new((0, 0)) };
}

/// A new, never used epoch.
#[inline]
pub(crate) fn next_epoch() -> u64 {
    LOCAL.with(|c| {
        let (next, end) = c.get();
        if next < end {
            c.set((next + 1, end));
            next
        } else {
            let base = NEXT_BLOCK.fetch_add(1, Ordering::Relaxed) * BLOCK;
            c.set((base + 1, base + BLOCK));
            base
        }
    })
}
