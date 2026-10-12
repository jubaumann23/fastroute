//! pcbkit F3 (`router.split_fixed_traces`, default off): a via, pin or trace end of a net on the inside of one of its
//! `(type fix)` traces is in contact with that trace only once the trace is split there. Freerouting never splits a
//! fixed trace (it cannot be deleted), while the maze search regards the item as reached, so the connection is
//! reported "routed" in every pass and stays open (router bench: hb200 I2C_BUS_SDA).
//!
//! - `data/via_on_corner.dsn`: net N (pads TA, TB, TC) is fully pre-routed with fixed wiring, but the via under TC's
//!   stub sits on a corner of the fixed B.Cu wire from TA to TB (the hb200 geometry): TC is open at load.
//! - `data/fixed_tjoin.dsn`: a fixed wire joins TA and TB; TC is not wired. Its route ends on the inside of the fixed
//!   wire (a T junction made while routing).

use fr_engine::board::{RoutingBoard, TimeLimitPolicy};
use fr_engine::datastructures::StopToken;
use fr_engine::ids::FixedState;
use fr_engine::pipeline::{run_pipeline, OptimizerMode, PipelineContext, StatsCache};

const CORNER: &[u8] = include_bytes!("data/via_on_corner.dsn");
const TJOIN: &[u8] = include_bytes!("data/fixed_tjoin.dsn");

fn load(dsn: &[u8], split: Option<bool>) -> (RoutingBoard, fr_settings::RouterSettings) {
    let mut settings = fr_settings::default_settings(1);
    settings.optimizer.enabled = Some(false);
    settings.autorouter.max_passes = Some(4);
    settings.split_fixed_traces = split;
    let mut board = fr_io::post_load::load_from_specctra_dsn(dsn, &mut settings).expect("load");
    fr_io::post_load::prepare_for_routing(&mut board, &mut settings, None);
    board.time_limits = TimeLimitPolicy::disabled();
    (board, settings)
}

fn route(dsn: &[u8], split: Option<bool>) -> RoutingBoard {
    let (mut board, mut settings) = load(dsn, split);
    let ctx = PipelineContext {
        stop: StopToken::new(),
        wall_clock_limits: false,
        deterministic: true,
        optimizer_mode: OptimizerMode::JavaCompat,
        enhancements: true,
        multi_start: 1,
        checkpoint: None,
        observer: None,
    };
    run_pipeline(&mut board, &mut settings, &ctx);
    board
}

fn open(board: &RoutingBoard) -> i32 {
    let settings = fr_settings::default_settings(1);
    StatsCache::new().score(board, &settings).incomplete_count
}

/// (count, total length) of the fixed traces.
fn fixed_traces(board: &RoutingBoard) -> (usize, f64) {
    let fixed: Vec<_> = board.get_traces().into_iter().filter(|&k| board.item(k).fixed_state() == FixedState::SystemFixed).collect();
    (fixed.len(), fixed.iter().map(|&k| board.item(k).trace().length()).sum())
}

#[test]
fn a_via_on_the_inside_of_a_fixed_trace_stays_open_without_the_setting() {
    let (board, _) = load(CORNER, None);
    assert_eq!(open(&board), 1, "TC's via is on a corner of the fixed wire, not at an end");
    assert_eq!(open(&route(CORNER, None)), 1, "the search reaches the fixed wire at once, the count never drops");
    assert_eq!(open(&route(TJOIN, None)), 1, "TC's route ends on the inside of the fixed wire");
}

#[test]
fn the_setting_splits_the_fixed_trace_at_the_via_and_keeps_its_copper_fixed() {
    let (before, _) = load(CORNER, Some(true));
    let routed = route(CORNER, Some(true));
    assert_eq!(open(&routed), 0);
    let (n0, len0) = fixed_traces(&before);
    let (n1, len1) = fixed_traces(&routed);
    assert_eq!(n1, n0 + 1, "the B.Cu wire is split in two at the via");
    assert!((len1 - len0).abs() < 1.0, "same fixed copper: {len0} -> {len1}");
    let added = routed.get_traces().len() - n1;
    assert_eq!(added, 0, "nothing had to be routed");
}

#[test]
fn the_setting_connects_a_route_that_ends_on_the_inside_of_a_fixed_trace() {
    let (before, _) = load(TJOIN, Some(true));
    let routed = route(TJOIN, Some(true));
    assert_eq!(open(&routed), 0);
    let (n0, len0) = fixed_traces(&before);
    let (n1, len1) = fixed_traces(&routed);
    // (fixed pieces are not combined again, so a junction an earlier pass left behind stays a split point)
    assert!(n0 == 1 && n1 >= 2, "the fixed wire is split at the T junction: {n0} -> {n1} pieces");
    assert!((len1 - len0).abs() < 1.0, "same fixed copper: {len0} -> {len1}");
}

#[test]
fn the_setting_is_deterministic() {
    let ids = |b: &RoutingBoard| -> Vec<String> {
        b.get_traces().into_iter().map(|k| format!("{:?} {:?}", b.item(k).first_corner(), b.item(k).last_corner())).collect()
    };
    assert_eq!(ids(&route(TJOIN, Some(true))), ids(&route(TJOIN, Some(true))));
}
