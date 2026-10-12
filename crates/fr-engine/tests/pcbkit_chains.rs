//! pcbkit rip-up chains (`router.chain_depth`, default off): a connection that is open only because a neighbour's
//! route is in its way is completed by ripping that neighbour and re-routing it elsewhere, recursively.
//!
//! Both boards have one signal layer (no vias) and a fixed GUARD wall around a 10 x 20 mm chamber. Net NA joins a pad
//! near the bottom to a pad at the top wall. Net NB's pads sit 0.2 mm from the left and right walls (too close for a
//! trace), and its pre-routed wire runs straight across the chamber, so NA cannot route; NB can also go round below
//! NA's bottom pad.
//! - `data/chain_block.dsn`: nothing else; one level of chain (rip NB, re-route it below) completes the board.
//! - `data/chain_block2.dsn`: net ND's pre-routed wire also closes the way below NA's pad (ND's pads sit at the bottom
//!   and left walls, and ND can hug that corner instead; the wire is shove-fixed, so NB cannot just push it aside), so
//!   NB can only be re-routed by ripping ND in turn: depth 2.

use fr_engine::board::{RoutingBoard, TimeLimitPolicy};
use fr_engine::datastructures::StopToken;
use fr_engine::pipeline::{BatchAutorouter, OptimizerMode, PipelineContext, StatsCache};

fn load(dsn: &[u8], configure: impl FnOnce(&mut fr_settings::RouterSettings)) -> (RoutingBoard, fr_settings::RouterSettings) {
    let mut settings = fr_settings::default_settings(1);
    settings.optimizer.enabled = Some(false);
    configure(&mut settings);
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

/// Runs only the chain stage on the loaded board (the pre-routed wiring is the state the routing stage left).
/// Returns (open before, open after, kept, routes, capped, clearance violations before, after).
fn chains(dsn: &[u8], depth: Option<f64>, max_routes: Option<f64>) -> (i32, i32, u32, u64, bool, i32, i32, RoutingBoard) {
    let (mut board, settings) = load(dsn, |s| {
        s.chain_depth = depth;
        s.chain_max_routes = max_routes;
    });
    let before = StatsCache::new().score(&board, &settings);
    let mut router = BatchAutorouter::for_job(&board, &settings);
    let r = router.chains(&mut board, &settings, &ctx());
    let after = StatsCache::new().score(&board, &settings);
    assert_eq!((r.open_before, r.open_after), (before.incomplete_count, after.incomplete_count));
    (r.open_before, r.open_after, r.chains_kept, r.routes, r.capped, before.clearance_violation_count, after.clearance_violation_count, board)
}

const ONE: &[u8] = include_bytes!("data/chain_block.dsn");
const TWO: &[u8] = include_bytes!("data/chain_block2.dsn");

#[test]
fn the_neighbour_keeps_the_connection_open_and_the_stage_is_off_by_default() {
    let (open, after, kept, routes, _, _, _, _) = chains(ONE, None, None);
    assert_eq!((open, after, kept, routes), (1, 1, 0, 0), "NA is open and nothing runs without router.chain_depth");
}

#[test]
fn a_chain_rips_the_neighbour_routes_the_connection_and_re_routes_the_neighbour() {
    let (open, after, kept, _, capped, cv0, cv1, _) = chains(ONE, Some(1.0), None);
    assert_eq!((open, after, kept, capped), (1, 0, 1, false));
    assert!(cv1 <= cv0, "no new clearance violation ({cv0} -> {cv1})");
}

#[test]
fn a_neighbour_that_must_rip_its_own_neighbour_needs_depth_two() {
    let (open, after, _, _, _, _, _, _) = chains(TWO, Some(1.0), None);
    assert_eq!((open, after), (1, 1), "at depth 1 NB can only be re-routed without rip-up, and ND is in its way");
    let (open, after, kept, _, _, cv0, cv1, _) = chains(TWO, Some(2.0), None);
    assert_eq!((open, after, kept), (1, 0, 1), "depth 2: NA rips NB, NB rips ND, ND hugs the corner");
    assert!(cv1 <= cv0, "no new clearance violation ({cv0} -> {cv1})");
}

#[test]
fn the_route_cap_ends_the_stage_and_a_failed_chain_leaves_the_board_unchanged() {
    let (open, after, kept, routes, capped, _, _, _) = chains(ONE, Some(1.0), Some(1.0));
    assert_eq!((open, after, kept, routes, capped), (1, 1, 0, 1, true), "one search: NA alone, which fails");
    let (_, _, _, _, _, _, _, board) = chains(TWO, Some(1.0), None);
    let (untouched, _) = load(TWO, |_| {});
    assert_eq!(item_ids(&board), item_ids(&untouched), "a chain that is not kept changes nothing");
}

fn item_ids(board: &RoutingBoard) -> Vec<(i32, i32)> {
    let mut items: Vec<(i32, i32)> = board.get_items().iter().filter(|&&k| board.item(k).net_count() > 0).map(|&k| (board.item(k).id().0, board.item(k).net_number(0))).collect();
    items.sort_unstable();
    items
}

#[test]
fn chains_are_deterministic() {
    let run = || item_ids(&chains(TWO, Some(2.0), None).7);
    assert_eq!(run(), run());
}

#[test]
fn the_settings_are_read_from_their_router_paths() {
    let mut s = fr_settings::default_settings(1);
    assert_eq!((s.get_chain_depth(), s.get_chain_max_routes()), (0, 0), "off by default");
    fr_settings::reflect::set_field_value(&mut s, "chain_depth", "2").expect("chain_depth");
    fr_settings::reflect::set_field_value(&mut s, "chain_max_routes", "3000").expect("chain_max_routes");
    assert_eq!((s.get_chain_depth(), s.get_chain_max_routes()), (2, 3000));
    fr_settings::reflect::set_field_value(&mut s, "chain_depth", "9").expect("chain_depth");
    assert_eq!(s.get_chain_depth(), 3, "the depth is capped at 3");
}

