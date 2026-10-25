//! pcbkit F4 (`router.active_layer_ends`, default off): the maze search does not take a destination it reaches on a
//! layer the connection may not end on. A through via of a non-plane net reaches another through via of its net on a
//! power-plane layer at once when both sit in one free room there; Freerouting rejects that found connection only
//! after the search ("some of their layers are disabled"), so the connection fails in every pass although a
//! signal-layer path exists.
//!
//! `data/rp2040env_s48_vbus.dsn` (recorded, claude-pcb-rules docs/improve/round8/vbus-pinch/NOTES.md "U5.5 dangling
//! via: root cause"): the toolkit's DSN of rp2040env seed 48 with `PCBKIT_ROUTE_LINK_ESCAPE=1` (fixed escape vias for
//! U5.5 and J1's two VBUS pad pairs), plus the router's own wiring of every other net frozen as `(type fix)`, so only
//! VBUS is left to route. The VBUS connections between the three escape vias end on In1.Cu (GND plane) or In2.Cu
//! (VDD_3V3 plane) every time; the B.Cu join is free.

use fr_engine::board::{RoutingBoard, TimeLimitPolicy};
use fr_engine::datastructures::StopToken;
use fr_engine::pipeline::{run_pipeline, OptimizerMode, PipelineContext, StatsCache};

const S48_VBUS: &[u8] = include_bytes!("data/rp2040env_s48_vbus.dsn");
const B_CU: i32 = 3;

/// The toolkit's settings for this route (claude-pcb-rules `fastroute_serve.py`), with `active_layer_ends`.
fn route(setting: Option<bool>) -> RoutingBoard {
    let mut settings = fr_settings::default_settings(1);
    settings.optimizer.enabled = Some(false);
    settings.autorouter.max_passes = Some(8);
    settings.automatic_neckdown = Some(true);
    settings.min_trace_width_um = Some(100.0);
    settings.strict_width = Some(true);
    settings.neck_radius_um = Some(1000.0);
    settings.strict_last_resort = Some(true);
    settings.split_fixed_traces = Some(true);
    settings.active_layer_ends = setting;
    let mut board = fr_io::post_load::load_from_specctra_dsn(S48_VBUS, &mut settings).expect("load");
    fr_io::post_load::prepare_for_routing(&mut board, &mut settings, None);
    board.time_limits = TimeLimitPolicy::disabled();
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

/// Layers of the traces the router added (everything recorded is fixed).
fn routed_layers(board: &RoutingBoard) -> Vec<i32> {
    let mut layers: Vec<i32> = board
        .get_traces()
        .into_iter()
        .filter(|&k| board.item(k).fixed_state() == fr_engine::ids::FixedState::Unfixed)
        .map(|k| board.item(k).trace().layer())
        .collect();
    layers.sort_unstable();
    layers.dedup();
    layers
}

#[test]
fn without_the_setting_the_escape_vias_stay_apart() {
    assert_eq!(open(&route(None)), 1, "U5.5's via is reached from J1's link via on a plane layer only; that end is rejected");
}

#[test]
fn the_setting_joins_the_escape_vias_on_b_cu() {
    let routed = route(Some(true));
    let layers = routed_layers(&routed);
    assert_eq!(open(&routed), 0);
    assert!(layers.contains(&B_CU), "the join runs on B.Cu: {layers:?}");
    assert!(!layers.iter().any(|&l| l == 1 || l == 2), "nothing on the plane layers: {layers:?}");
}

#[test]
fn the_setting_is_deterministic() {
    let ids = |b: &RoutingBoard| -> Vec<String> {
        b.get_traces().into_iter().map(|k| format!("{:?} {:?}", b.item(k).first_corner(), b.item(k).last_corner())).collect()
    };
    assert_eq!(ids(&route(Some(true))), ids(&route(Some(true))));
}
