//! pcbkit M1: caps the glibc malloc arenas of the process.
//!
//! The Rust heap is mimalloc, but every thread that touches the C allocator (thread-local
//! destructors, libc internals) gets a glibc arena, and glibc reserves 64 MB of address space per
//! arena, up to 8 per core. With the autorouter, optimizer and multi-start thread pools (about 50
//! threads on a 32-core host) a route reserved 4.4-5.4 GB of address space for 0.6-1.2 GB resident.
//! Under an address-space limit (the toolkit runs the router with `prlimit --as=4000000000`) the
//! threads of a pool then failed to start (EAGAIN) or an allocation failed (abort), depending on
//! the timing of the run. Two arenas are plenty for the little C allocation there is.

/// Arenas when `MALLOC_ARENA_MAX` is not set.
pub const ARENAS: i32 = 2;

/// glibc `M_ARENA_MAX` (malloc.h).
#[cfg(all(target_os = "linux", target_env = "gnu"))]
const M_ARENA_MAX: i32 = -8;

#[cfg(all(target_os = "linux", target_env = "gnu"))]
extern "C" {
    fn mallopt(param: i32, value: i32) -> i32;
}

/// Caps the glibc arenas at [`ARENAS`] unless `MALLOC_ARENA_MAX` is set (glibc reads that
/// itself). Call it first in `main`, before any thread exists. Returns true if the cap was set.
pub fn cap() -> bool {
    if std::env::var_os("MALLOC_ARENA_MAX").is_some() {
        return false;
    }
    cap_at(ARENAS)
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn cap_at(arenas: i32) -> bool {
    // SAFETY: `mallopt` only stores the parameter in glibc's malloc state (thread-safe, takes the
    // malloc lock); no pointer is passed and any value is valid for M_ARENA_MAX.
    unsafe { mallopt(M_ARENA_MAX, arenas) == 1 }
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
fn cap_at(_arenas: i32) -> bool {
    false
}

#[cfg(all(test, target_os = "linux", target_env = "gnu"))]
mod tests {
    use std::sync::{Arc, Barrier};

    extern "C" {
        fn malloc(size: usize) -> *mut u8;
        fn free(ptr: *mut u8);
    }

    fn vm_size_kb() -> u64 {
        let status = std::fs::read_to_string("/proc/self/status").expect("/proc/self/status");
        let line = status.lines().find(|l| l.starts_with("VmSize:")).expect("VmSize");
        line.split_whitespace().nth(1).and_then(|v| v.parse().ok()).expect("VmSize value")
    }

    /// 24 threads that each use the C allocator at the same time: without the cap glibc gives
    /// each its own arena (64 MB of address space each, about 1.5 GB); with it they share two.
    #[test]
    fn many_threads_share_the_capped_arenas() {
        assert!(super::cap_at(super::ARENAS));
        const THREADS: usize = 24;
        let before = vm_size_kb();
        let ready = Arc::new(Barrier::new(THREADS + 1));
        let done = Arc::new(Barrier::new(THREADS + 1));
        let handles: Vec<_> = (0..THREADS)
            .map(|_| {
                let (ready, done) = (ready.clone(), done.clone());
                std::thread::spawn(move || {
                    // SAFETY: a plain C allocation of 1 KB, freed below on the same thread.
                    let p = unsafe { malloc(1024) };
                    assert!(!p.is_null());
                    ready.wait();
                    done.wait();
                    // SAFETY: `p` came from `malloc` above and is freed once.
                    unsafe { free(p) };
                })
            })
            .collect();
        ready.wait();
        let grown_mb = vm_size_kb().saturating_sub(before) / 1024;
        done.wait();
        for h in handles {
            h.join().unwrap();
        }
        // thread stacks (2 MB each) plus two arenas; 24 arenas alone would be 1536 MB
        assert!(grown_mb < 512, "address space grew by {grown_mb} MB for {THREADS} threads");
    }
}
