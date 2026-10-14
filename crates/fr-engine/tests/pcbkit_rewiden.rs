//! pcbkit W5 (`router.rewiden`, default off): after routing, a trace the router left narrower than its net-class width
//! is put back at the class width by moving its neighbours, so a later clearance-only widen is not stuck against copper
//! packed at the minimum clearance.
//!
//! The boards are fully wired at load (nothing to route): net W (class width 600 um) is a straight 200 um wire, net S
//! (150 um) runs beside it at exactly the minimum clearance (150 um) over 10 mm.
//! - `data/rewiden_shove.dsn`: S has room to move: the shove pushes it away.
//! - `data/rewiden_rip.dsn`: a keepout sits 150 um beyond S, so the shove cannot move it; S is ripped and routed
//!   again elsewhere.
//! - `data/rewiden_pinch.dsn`: S is `(type fix)`: a real pinch, W stays narrow.

use fr_engine::board::{RoutingBoard, TimeLimitPolicy};
use fr_engine::datastructures::StopToken;
use fr_engine::drc::all_clearance_violations;
use fr_engine::pipeline::autorouter::RewidenReport;
use fr_engine::pipeline::{run_pipeline, BatchAutorouter, OptimizerMode, PipelineContext, StatsCache};

const SHOVE: &[u8] = include_bytes!("data/rewiden_shove.dsn");
const RIP: &[u8] = include_bytes!("data/rewiden_rip.dsn");
const PINCH: &[u8] = include_bytes!("data/rewiden_pinch.dsn");

fn load(dsn: &[u8], rewiden: Option<bool>) -> (RoutingBoard, fr_settings::RouterSettings) {
    let mut settings = fr_settings::default_settings(1);
    settings.optimizer.enabled = Some(false);
    settings.autorouter.max_passes = Some(4);
    settings.rewiden = rewiden;
    let mut board = fr_io::post_load::load_from_specctra_dsn(dsn, &mut settings).expect("load");
    fr_io::post_load::prepare_for_routing(&mut board, &mut settings, None);
    board.time_limits = TimeLimitPolicy::disabled();
    (board, settings)
}

fn ctx() -> PipelineContext {
    PipelineContext {
        stop: StopToken::new(),
        wall_clock_limits: false,
        deterministic: true,
        optimizer_mode: OptimizerMode::JavaCompat,
        enhancements: true,
        multi_start: 1,
        checkpoint: None,
        observer: None,
    }
}

fn route(dsn: &[u8], rewiden: Option<bool>) -> RoutingBoard {
    let (mut board, mut settings) = load(dsn, rewiden);
    run_pipeline(&mut board, &mut settings, &ctx());
    board
}

/// The stage alone on the board as loaded.
fn stage(dsn: &[u8]) -> RewidenReport {
    let (mut board, settings) = load(dsn, Some(true));
    let mut router = BatchAutorouter::for_job(&board, &settings);
    router.rewiden(&mut board, &settings, &ctx())
}

/// (narrowest, widest) trace width of `net` in board units.
fn widths(board: &RoutingBoard, net: &str) -> (i32, i32) {
    let n = (1..=board.rules.nets.max_net_number()).find(|&n| board.rules.nets.get(n).is_some_and(|x| x.name == net)).expect("net");
    let w: Vec<i32> = board
        .get_traces()
        .into_iter()
        .filter(|&k| board.item(k).net_numbers().contains(&n))
        .map(|k| 2 * board.item(k).trace().half_width())
        .collect();
    (*w.iter().min().expect("traces"), *w.iter().max().expect("traces"))
}

fn open(board: &RoutingBoard) -> i32 {
    StatsCache::new().score(board, &fr_settings::default_settings(1)).incomplete_count
}

fn check(board: &RoutingBoard) {
    assert_eq!(open(board), 0, "every connection stays made");
    assert!(all_clearance_violations(board).is_empty(), "no clearance violation");
}

/// Board units of `um` micrometres (resolution 10 per um).
const fn um(v: i32) -> i32 {
    v * 10
}

#[test]
fn without_the_setting_the_narrow_trace_stays_narrow() {
    for dsn in [SHOVE, RIP, PINCH] {
        let b = route(dsn, None);
        check(&b);
        assert_eq!(widths(&b, "W"), (um(200), um(200)));
    }
}

#[test]
fn the_shove_moves_a_neighbour_that_has_room() {
    let b = route(SHOVE, Some(true));
    check(&b);
    assert_eq!(widths(&b, "W"), (um(600), um(600)));
}

#[test]
fn a_neighbour_the_shove_cannot_move_is_routed_again() {
    let b = route(RIP, Some(true));
    check(&b);
    assert_eq!(widths(&b, "W"), (um(600), um(600)));
    assert_eq!(widths(&b, "S"), (um(150), um(150)), "S is re-routed at its own class width");
}

#[test]
fn a_real_pinch_stays_narrow_and_clean() {
    let b = route(PINCH, Some(true));
    check(&b);
    assert_eq!(widths(&b, "W").0, um(200), "fixed copper cannot move: W keeps its narrow width there");
}

#[test]
fn the_stage_is_deterministic() {
    let a = route(RIP, Some(true));
    let b = route(RIP, Some(true));
    let geom = |x: &RoutingBoard| {
        let mut v: Vec<String> = x.get_traces().into_iter().map(|k| format!("{:?}", x.item(k).trace().polyline().corners())).collect();
        v.sort();
        v
    };
    assert_eq!(geom(&a), geom(&b));
}

#[test]
fn the_report_names_the_step_that_widened() {
    let r = stage(SHOVE);
    assert_eq!((r.narrow_traces, r.widened_whole, r.ripped_segments, r.narrow_segments_left), (1, 1, 0, 0));
    let r = stage(RIP);
    assert_eq!((r.narrow_traces, r.widened_whole, r.shoved_segments, r.ripped_segments, r.narrow_segments_left), (1, 0, 0, 1, 0));
    let r = stage(PINCH);
    assert_eq!((r.narrow_traces, r.widened_whole, r.shoved_segments, r.ripped_segments, r.narrow_segments_left), (1, 0, 0, 0, 1));
}
