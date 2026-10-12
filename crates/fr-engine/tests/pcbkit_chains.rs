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


/// `data/chain_memo.dsn`: `chain_block.dsn` plus net NC, whose two pins sit each in a closed fixed box (nets BOX1 and
/// BOX2; walled in: no chain can route it). NC's pins come first in the placement, so the stage tries NA before NC.
const MEMO: &[u8] = include_bytes!("data/chain_memo.dsn");

/// Runs the chain stage with `configure` applied; returns (open before, open after, chains tried, kept, routes).
fn chain_report(dsn: &[u8], configure: impl FnOnce(&mut fr_settings::RouterSettings)) -> (i32, i32, u32, u32, u64) {
    let (mut board, settings) = load(dsn, configure);
    let mut router = BatchAutorouter::for_job(&board, &settings);
    let r = router.chains(&mut board, &settings, &ctx());
    (r.open_before, r.open_after, r.chains_tried, r.chains_kept, r.routes)
}

#[test]
fn a_chain_that_failed_is_not_retried_on_an_unchanged_board() {
    let (open, after, tried, kept, _) = chain_report(MEMO, |s| s.chain_depth = Some(1.0));
    assert_eq!((open, after, kept), (2, 1, 1), "NA is completed, the walled-in NC stays open");
    // round 1: NA (kept), then NC from each of its two pins (failed); round 2 would only retry NC on the board both
    // of its chains already failed on
    assert_eq!(tried, 3, "NC is not tried again in round 2");
}

#[test]
fn chain_reverse_searches_again_from_the_open_end_and_changes_nothing_else() {
    let off = chain_report(MEMO, |s| s.chain_depth = Some(1.0));
    let on = chain_report(MEMO, |s| {
        s.chain_depth = Some(1.0);
        s.chain_reverse = Some(true);
    });
    assert_eq!((on.0, on.1, on.2, on.3), (off.0, off.1, off.2, off.3), "the same chains, the same result");
    // NA: its plain search fails both ways before the rip-up variant routes it (+1); NC, from each of its two pins:
    // the plain search and the variant that finds no way are both repeated from the pin's end (+2 each)
    assert_eq!(on.4, off.4 + 5, "each depth-0 search that found no way is repeated once in reverse");
    let mut s = fr_settings::default_settings(1);
    assert!(!s.is_chain_reverse(), "off by default");
    fr_settings::reflect::set_field_value(&mut s, "chain_reverse", "true").expect("chain_reverse");
    assert!(s.is_chain_reverse());
}

#[test]
fn a_reversed_search_routes_from_the_item_end_to_the_rest_of_the_net() {
    use fr_engine::autoroute::router::{route_connection_noisy, RecoverTuning};
    // NA alone in the chamber (NB's wire removed): both directions route it
    let (board, settings) = load(ONE, |_| {});
    for reverse in [false, true] {
        let mut b = board.clone_for_worker();
        let nb: Vec<_> = b.get_items().into_iter().filter(|&k| b.item(k).is_trace() && b.item(k).net_count() == 1 && !b.item(k).is_user_fixed() && b.rules.nets.get(b.item(k).net_number(0)).is_some_and(|n| n.name == "NB")).collect();
        b.remove_items(nb);
        let na = b.get_items().into_iter().find(|&k| b.item(k).is_pin() && b.rules.nets.get(b.item(k).net_number(0)).is_some_and(|n| n.name == "NA")).expect("NA pin");
        let net = b.item(na).net_number(0);
        let params = BatchAutorouter::for_job(&b, &settings).connection_params();
        let before = StatsCache::new().score(&b, &settings).incomplete_count;
        let mut ripped = fr_engine::board::ItemSet::new();
        let tuning = RecoverTuning { noise_pass_no: 1, rip_zone_vias: false, rip_protect_above: Some(i32::MIN), reverse };
        b.start_marking_changed_area();
        let out = route_connection_noisy(&mut b, &settings, &params, na, net, &mut ripped, None, 1, Some(&tuning), None);
        let after = StatsCache::new().score(&b, &settings).incomplete_count;
        assert_eq!(out.result.state, fr_engine::board::AutorouteAttemptState::Routed, "reverse {reverse}");
        assert_eq!((before, after), (2, 1), "reverse {reverse}: NA is connected (NB, whose wire was removed, stays open)");
    }
}

/// `data/chain_strict.dsn`: `chain_block.dsn` with NA and NB in a 600 um class and NA's bottom pad 0.72 mm from the
/// wall. NB, ripped for NA, can only go round below NA's pad, through that gap: too narrow for 600 um plus clearances,
/// wide enough for the 100 um minimum width.
const STRICT: &[u8] = include_bytes!("data/chain_strict.dsn");

#[test]
fn under_strict_width_with_its_last_resort_the_chains_may_neck() {
    let run = |strict: bool, last_resort: bool| {
        chain_report(STRICT, |s| {
            s.chain_depth = Some(1.0);
            s.min_trace_width_um = Some(100.0);
            if strict {
                s.strict_width = Some(true);
                s.neck_radius_um = Some(300.0);
                s.strict_last_resort = Some(last_resort);
            }
        })
    };
    let plain = run(false, false);
    let strict = run(true, false);
    let last_resort = run(true, true);
    assert_eq!((plain.0, plain.1), (1, 0), "without strict width NB necks through the gap");
    assert_eq!((strict.0, strict.1), (1, 1), "strict width: NB cannot be re-routed, the chain fails");
    assert_eq!((last_resort.0, last_resort.1), (1, 0), "the last resort lets the chain neck as without strict width");
}

#[test]
fn chain_prune_drops_variants_that_cannot_settle_and_keeps_the_result() {
    for (name, dsn, depth) in [("one", ONE, 1.0), ("two@1", TWO, 1.0), ("two@2", TWO, 2.0), ("memo", MEMO, 1.0)] {
        let off = chain_report(dsn, |s| s.chain_depth = Some(depth));
        let on = chain_report(dsn, |s| {
            s.chain_depth = Some(depth);
            s.chain_prune = Some(true);
        });
        assert_eq!((on.0, on.1, on.3), (off.0, off.1, off.3), "{name}: the same chains are kept");
        assert!(on.4 <= off.4, "{name}: never more routes ({} vs {})", on.4, off.4);
        if name == "two@1" {
            // NB cannot be re-routed at depth 1, and every rip-up variant of NA rips the same NB
            assert!(on.4 < off.4, "two@1: the variants that rip NB again are skipped ({} vs {} routes)", on.4, off.4);
        }
    }
}
