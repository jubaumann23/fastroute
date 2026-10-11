//! pcbkit D1: the parallel optimizer gives the same board when its thread pool cannot be built.
//!
//! The serve router ran under an address-space limit; when the optimizer's threads failed to start
//! (EAGAIN), `BatchOptimizer` fell back to the Java-compatible evaluation, which keeps one worker
//! board and picks other candidates than the parallel mode. Whether the pool could be built
//! depended on the run, so the same request gave one of two SES files. Without a pool the parallel
//! algorithm now runs on the calling thread.

use fr_engine::board::{RoutingBoard, TimeLimitPolicy};
use fr_engine::datastructures::StopToken;
use fr_engine::pipeline::{run_pipeline, BatchOptimizer, OptimizerMode, PipelineContext, StatsCache};

/// A synthetic 2-layer board: 4 x 3 0603 resistors on a 6 mm grid; net `k` joins pin 2 of part
/// `k` to pin 1 of part `5k mod 12`, so the nets cross and the optimizer has work.
const GRID: &[u8] = include_bytes!("data/grid4x3.dsn");

fn ctx(mode: OptimizerMode) -> PipelineContext {
    PipelineContext {
        stop: StopToken::new(),
        wall_clock_limits: false,
        deterministic: true,
        optimizer_mode: mode,
        enhancements: true,
        multi_start: 1,
        checkpoint: None,
        observer: None,
    }
}

/// The board after the routing stage (no optimizer), as the optimizer's input.
fn routed(dsn: &[u8]) -> (RoutingBoard, fr_settings::RouterSettings) {
    let mut settings = fr_settings::default_settings(4);
    settings.optimizer.enabled = Some(false);
    let mut board = fr_io::post_load::load_from_specctra_dsn(dsn, &mut settings).expect("load");
    fr_io::post_load::prepare_for_routing(&mut board, &mut settings, None);
    board.time_limits = TimeLimitPolicy::disabled();
    run_pipeline(&mut board, &mut settings, &ctx(OptimizerMode::JavaCompat));
    settings.optimizer.enabled = Some(true);
    (board, settings)
}

fn optimized(board: &RoutingBoard, settings: &fr_settings::RouterSettings, optimizer: BatchOptimizer, mode: OptimizerMode) -> Vec<u8> {
    let (mut b, mut s) = (board.deep_copy(), settings.clone());
    let mut optimizer = optimizer;
    optimizer.run_batch_loop(&mut b, &mut s, &ctx(mode), &mut StatsCache::new());
    fr_io::ses_writer::ses_bytes(&b, "t")
}

#[test]
fn without_its_pool_the_parallel_optimizer_gives_the_same_board() {
    let mode = OptimizerMode::Parallel { threads: 4 };
    let (board, settings) = routed(GRID);
    let pooled = optimized(&board, &settings, BatchOptimizer::new(mode), mode);
    let alone = optimized(&board, &settings, BatchOptimizer::new(mode).without_pool(), mode);
    assert!(pooled != fr_io::ses_writer::ses_bytes(&board, "t"), "the optimizer must change this board");
    assert!(pooled == alone, "the parallel optimizer without its pool gave another board");
}

/// The precondition of the test above: on this board the Java-compatible evaluation (the old
/// fallback) picks another result than the parallel mode.
#[test]
fn the_java_compatible_evaluation_differs_on_this_board() {
    let (board, settings) = routed(GRID);
    let mode = OptimizerMode::Parallel { threads: 4 };
    let pooled = optimized(&board, &settings, BatchOptimizer::new(mode), mode);
    let compat = optimized(&board, &settings, BatchOptimizer::new(OptimizerMode::JavaCompat), OptimizerMode::JavaCompat);
    assert!(pooled != compat);
}
