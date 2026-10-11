//! pcbkit P1-P3: the one place that builds a rayon pool.
//!
//! Every pool of the pipeline (autorouter pass, multi-start, optimizer) is an explicit local pool
//! with a bounded thread count, built here. A pool can fail to build (thread or address-space
//! limit); each caller then runs the same algorithm on the calling thread, which gives the same
//! result (the results never depend on thread timing). Nothing uses rayon's global pool.

/// Environment variable of the test hook: when set to `1` no pool can be built (as under a tight
/// address-space limit), so the sequential fallbacks run.
pub const FORCE_NO_POOL_ENV: &str = "FASTROUTE_TEST_NO_POOL";

/// Builds a pool of `threads` threads, or says why it cannot.
pub fn build(threads: usize) -> Result<rayon::ThreadPool, String> {
    if std::env::var(FORCE_NO_POOL_ENV).as_deref() == Ok("1") {
        return Err(format!("{FORCE_NO_POOL_ENV}=1"));
    }
    rayon::ThreadPoolBuilder::new().num_threads(threads.max(1)).build().map_err(|e| e.to_string())
}
