//! The routing pipeline (porting unit U9): Java `autoroute/pipeline/RoutingPipeline` and the
//! stages it runs.
//!
//! | Module | Java |
//! |---|---|
//! | [`autorouter`] | `BatchAutorouter`, `AutorouteBatchLoop`, `AutoroutePassRunner.runSingleThread`, strict DRC of `AutorouteConnectionRouter` |
//! | [`fanout`] | `BatchFanout` |
//! | [`optimizer`] | `BatchOptimizer` (`OptimizeCandidateTask`, `WorkerBoardState`, `ReadSortedRouteItems`), `ItemRouteResult` |
//! | [`history`] | `BoardHistory`, `BasicBoard.getHash` |
//! | [`stats`] | memoized `new BoardStatistics(board)` |
//!
//! Not ported: events, GUI hooks, analytics, profiling, the result manifest and the dead
//! multi-threaded autorouter (`runMultiThread`, `BatchAutorouterThread`).
//!
//! The Java `StoppableThread` of the job is a [`StopToken`]: `requestStopAutoRouter` (max
//! passes reached, stagnation) stops the autorouter only, and — like in Java — also the
//! autorouting passes inside the optimizer candidates (`autoroutePassesForOptimizingItem`
//! checks `isStopAutoRouterRequested`).
//!
//! The Java `TimeLimit`s of the board algorithms come from `RoutingBoard::time_limits`: for
//! parity runs the deterministic count mode of the Java parity jar
//! ([`TimeLimit::Count`](crate::datastructures::TimeLimit::Count), `limit_ms * 10` calls of
//! `limitExceeded()` per instance; the call sites are `AutorouteEngine.isStopRequested`,
//! `TraceTightener.isStopRequested`, `TraceShover.check`, `DrillItemMover.check`, called
//! exactly as often as in Java). The other wall clock limits (fanout / optimizer
//! `timeout_string`, job timeout) are only active with [`PipelineContext::wall_clock_limits`].
//!
//! Parity status (`scripts/parity-route.sh`, see the U9 report): SES and per-pass scores
//! identical to the Java parity build on all benchmark boards tried, autorouter only and full
//! pipeline (optimizer `java-compat`).

pub mod autorouter;
pub mod fanout;
pub mod history;
pub mod optimizer;
pub mod stats;

use fr_settings::{RouterSettings, ALGORITHM_CURRENT};

use crate::board::RoutingBoard;
use crate::datastructures::StopToken;

pub use autorouter::BatchAutorouter;
pub use history::{board_hash, BoardHistory};
pub use optimizer::{BatchOptimizer, OptimizerMode};
pub use stats::StatsCache;

/// The job state shared by the stages.
#[derive(Clone, Debug)]
pub struct PipelineContext {
    /// Java `job.thread` stop requests.
    pub stop: StopToken,
    /// True to honour the wall clock stage limits (`fanout.timeout_string`,
    /// `optimizer.timeout_string`); false for deterministic runs.
    pub wall_clock_limits: bool,
    /// How the optimizer evaluates its candidates.
    pub optimizer_mode: OptimizerMode,
    /// fastroute improvements that change results compared with Freerouting
    /// (see docs/IMPROVEMENTS.md). Off for `--parity`.
    pub enhancements: bool,
}

impl Default for PipelineContext {
    fn default() -> Self {
        PipelineContext {
            stop: StopToken::new(),
            wall_clock_limits: true,
            optimizer_mode: OptimizerMode::JavaCompat,
            enhancements: false,
        }
    }
}

/// The outcome of [`run_pipeline`].
#[derive(Clone, Copy, Debug, Default)]
pub struct PipelineOutcome {
    pub fanout_timed_out: bool,
    pub optimizer_timed_out: bool,
}

/// Java `RoutingPipeline.create(job).run()`: fanout + autorouter, `finishAutoroute`, then the
/// optimizer (if enabled). The board may be replaced (history restores, optimizer results).
pub fn run_pipeline(board: &mut RoutingBoard, settings: &mut RouterSettings, ctx: &PipelineContext) -> PipelineOutcome {
    // normalizeRouterAlgorithm
    if settings.autorouter.algorithm.as_deref() != Some(ALGORITHM_CURRENT) {
        log::warn!(
            "The algorithm '{}' is not supported. The default algorithm '{ALGORITHM_CURRENT}' will be used instead.",
            settings.autorouter.algorithm.as_deref().unwrap_or("null")
        );
        settings.autorouter.algorithm = Some(ALGORITHM_CURRENT.to_string());
    }
    let run_optimizer = settings.get_run_optimizer();
    if run_optimizer && settings.optimizer.algorithm.as_deref() != Some("freerouting-optimizer") {
        log::warn!("The optimizer algorithm is not supported; the default algorithm 'freerouting-optimizer' will be used instead.");
        settings.optimizer.algorithm = Some("freerouting-optimizer".to_string());
    }
    let mut autorouter = BatchAutorouter::for_job(board, settings);
    let mut outcome = PipelineOutcome::default();

    // runRoutingStage
    let router_enabled = settings.get_run_router() && settings.autorouter.max_passes.map(|m| m >= 0).unwrap_or(true);
    if router_enabled && !ctx.stop.is_stop_autorouter_requested() {
        autorouter.run_batch_loop(board, settings, ctx);
    } else if settings.is_fanout_enabled() && !ctx.stop.is_stop_autorouter_requested() {
        let original = settings.autorouter.max_passes;
        settings.autorouter.max_passes = Some(0);
        autorouter.run_batch_loop(board, settings, ctx);
        settings.autorouter.max_passes = original;
    }
    outcome.fanout_timed_out = autorouter.fanout_timed_out;
    board.finish_autoroute();

    // runOptimizationStage
    if run_optimizer && !ctx.stop.is_stop_requested() {
        let mut optimizer = BatchOptimizer::new(ctx.optimizer_mode);
        let mut stats = StatsCache::new();
        optimizer.run_batch_loop(board, settings, ctx, &mut stats);
        outcome.optimizer_timed_out = optimizer.timed_out;
    }
    outcome
}

/// Java `HeadlessBoardManager.scheduleDeferredPostLoadProcessing`: the board part (awaited by
/// `RoutingPipeline.run` through `awaitPostLoad`): the pre-existing and unfixable clearance
/// violation counts of the loaded board.
pub fn deferred_post_load_processing(board: &mut RoutingBoard) {
    let violations = crate::drc::all_clearance_violations(board);
    board.pre_existing_clearance_violations_count = violations.len() as i32;
    let unfixable = violations.iter().filter(|v| v.is_unfixable(board)).count() as i32;
    board.unfixable_clearance_violations_count = unfixable;
    if !violations.is_empty() {
        log::warn!(
            "Board has {} pre-existing clearance violation(s) in the loaded design ({} unfixable).",
            violations.len(),
            unfixable
        );
    }
}
