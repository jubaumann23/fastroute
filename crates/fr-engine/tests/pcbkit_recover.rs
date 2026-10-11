//! pcbkit recovery stage (`router.recover_attempts`, default off): a connection the routing stage leaves open is
//! completed by a targeted retry, and nothing changes while the setting is off.
//!
//! `data/neck_gap.dsn`: net N1 has to pass a 0.39 mm gap in a fixed wall (both layers; the wall's four pre-existing
//! clearance violations against the board outline are part of the board). The routing stage cannot get a trace of the
//! class width (0.2 mm, clearance 0.15 mm) or its automatic neck-down through; at the 0.08 mm minimum trace width it
//! needs 0.38 mm and fits.

use fr_engine::board::{RoutingBoard, TimeLimitPolicy};
use fr_engine::datastructures::StopToken;
use fr_engine::pipeline::{run_pipeline, OptimizerMode, PipelineContext, StatsCache};

const DSN: &[u8] = include_bytes!("data/neck_gap.dsn");

fn load(configure: impl FnOnce(&mut fr_settings::RouterSettings)) -> (RoutingBoard, fr_settings::RouterSettings) {
    let mut settings = fr_settings::default_settings(1);
    settings.optimizer.enabled = Some(false);
    settings.autorouter.max_passes = Some(6);
    configure(&mut settings);
    let mut board = fr_io::post_load::load_from_specctra_dsn(DSN, &mut settings).expect("load");
    fr_io::post_load::prepare_for_routing(&mut board, &mut settings, None);
    board.time_limits = TimeLimitPolicy::disabled();
    (board, settings)
}

/// The pipeline as `fastroute serve` runs it (enhancements on, deterministic, no wall clock).
fn route(board: &mut RoutingBoard, settings: &mut fr_settings::RouterSettings) {
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
    run_pipeline(board, settings, &ctx);
}

fn open_connections(board: &RoutingBoard, settings: &fr_settings::RouterSettings) -> i32 {
    StatsCache::new().score(board, settings).incomplete_count
}

#[test]
fn the_wall_leaves_the_connection_open_without_recovery() {
    let (mut board, mut settings) = load(|s| s.min_trace_width_um = Some(80.0));
    route(&mut board, &mut settings);
    assert_eq!(open_connections(&board, &settings), 1, "the gap is too narrow for the class width");
}

#[test]
fn recovery_completes_it_at_the_minimum_trace_width() {
    let (mut board, mut settings) = load(|s| {
        s.min_trace_width_um = Some(80.0);
        s.recover_attempts = Some(2.0);
    });
    route(&mut board, &mut settings);
    assert_eq!(open_connections(&board, &settings), 0);
    let (mut open_board, mut open_settings) = load(|s| s.min_trace_width_um = Some(80.0));
    route(&mut open_board, &mut open_settings);
    assert_eq!(
        StatsCache::new().score(&board, &settings).clearance_violation_count,
        StatsCache::new().score(&open_board, &open_settings).clearance_violation_count,
        "the recovered route adds no clearance violation"
    );
}

#[test]
fn recovery_does_nothing_without_a_narrower_width_to_use() {
    let (mut board, mut settings) = load(|s| s.recover_attempts = Some(2.0));
    route(&mut board, &mut settings);
    assert_eq!(open_connections(&board, &settings), 1, "no minimum trace width: the wall stays closed");
}

#[test]
fn recovery_is_deterministic() {
    let run = || {
        let (mut board, mut settings) = load(|s| {
            s.min_trace_width_um = Some(80.0);
            s.recover_attempts = Some(3.0);
        });
        route(&mut board, &mut settings);
        let mut items: Vec<(i32, i32)> = board.get_items().iter().filter(|&&k| board.item(k).net_count() > 0).map(|&k| (board.item(k).id().0, board.item(k).net_number(0))).collect();
        items.sort_unstable();
        items
    };
    assert_eq!(run(), run());
}

#[test]
fn the_settings_are_read_from_their_router_paths() {
    let mut s = fr_settings::default_settings(1);
    assert_eq!((s.get_recover_attempts(), s.get_ripup_history_pct()), (0, 0), "off by default");
    fr_settings::reflect::set_field_value(&mut s, "recover_attempts", "5").expect("recover_attempts");
    fr_settings::reflect::set_field_value(&mut s, "ripup_history", "50").expect("ripup_history");
    assert_eq!((s.get_recover_attempts(), s.get_ripup_history_pct()), (5, 50));
}

#[test]
fn the_rip_up_history_is_off_by_default_and_does_not_outlive_the_routing_stage() {
    for pct in [None, Some(50.0)] {
        let (mut board, mut settings) = load(|s| {
            s.min_trace_width_um = Some(80.0);
            s.ripup_history = pct;
        });
        route(&mut board, &mut settings);
        assert!(board.rip_history.is_none(), "the optimizer and recovery route without the history (pct {pct:?})");
    }
}
