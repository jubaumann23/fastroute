//! pcbkit R3/R4: a pull-tight pass that crept for minutes converges with `router.tighten_min_move_um`,
//! and its rounds are counted in `RoutingBoard::work_stats`.
//!
//! [`HAIRPIN_DSN`] is recorded from stm32io (energy-9 placement with escape lanes, the fanout of
//! U3 pin 1, net PWR.VSYS), cut down to U3 and the four traces the fanout had inserted: a path that
//! runs out and back as a hairpin whose legs differ in width (601 um and the 453 um neck-down), so
//! `normalize_traces` cannot join them and they meet only at the tip. Upstream, each round of
//! `opt_changed_area` lets `smoothen_end_corners_at_trace` move the tip by 7 units (0.7 um) and
//! `split_traces` cut a 7-unit stub off the other leg; stubs of alternating width never merge, so
//! every round adds an item and is slower than the last. The unlimited pass ran 544 rounds in
//! 263 s and left 541 stubs (measured 2026-10-11, release build); in the full route it was the
//! 289 s fanout of that pin.

use fr_engine::board::RoutingBoard;
use fr_io::post_load;
use std::sync::atomic::Ordering;

/// Rounds after which the test stops the upstream pass (the round cap, `router.max_tighten_rounds`).
const CAP: u32 = 60;

fn load() -> (RoutingBoard, i32, Vec<fr_settings::ExpansionCostFactor>) {
    let mut settings = fr_settings::default_settings(1);
    let mut board = post_load::load_from_specctra_dsn(HAIRPIN_DSN.as_bytes(), &mut settings).expect("load");
    post_load::prepare_for_routing(&mut board, &mut settings, None);
    fr_engine::pipeline::deferred_post_load_processing(&mut board);
    let net = board.rules.nets.get_by_name("PWR.VSYS", 1).expect("net").net_number;
    (board, net, settings.get_trace_costs())
}

/// One pull-tight pass over the net, as `autoroute::router` runs it after a fanout; returns the
/// work counters and the item count after the pass.
fn tighten(min_move_um: f64) -> (Counters, usize) {
    let (mut board, net, costs) = load();
    let mut s = fr_settings::default_settings(1);
    s.max_tighten_rounds = Some(f64::from(CAP));
    s.tighten_min_move_um = Some(min_move_um);
    board.apply_tighten_settings(&s);
    board.mark_all_changed_area();
    board.opt_changed_area(&[net], None, 500, Some(&costs), None, 0);
    let w = &board.work_stats;
    let c = Counters {
        passes: w.tighten_passes.load(Ordering::Relaxed),
        rounds: w.max_tighten_rounds.load(Ordering::Relaxed),
        caps: w.tighten_caps.load(Ordering::Relaxed),
        negligible: w.negligible_moves.load(Ordering::Relaxed),
    };
    (c, board.get_items().len())
}

#[derive(Debug)]
struct Counters {
    passes: u64,
    rounds: u64,
    caps: u64,
    negligible: u64,
}

#[test]
fn upstream_pass_creeps_until_the_round_cap() {
    let items_before = load().0.get_items().len();
    let (c, items) = tighten(0.0);
    assert_eq!((c.passes, c.rounds, c.caps, c.negligible), (1, u64::from(CAP), 1, 0), "{c:?}");
    // one stub split off per round
    assert!(items >= items_before + CAP as usize - 5, "{items_before} -> {items} items");
}

#[test]
fn min_move_makes_the_pass_converge() {
    let items_before = load().0.get_items().len();
    let (c, items) = tighten(1.0);
    assert_eq!((c.passes, c.caps), (1, 0), "{c:?}");
    assert!(c.rounds <= 2, "{c:?}");
    assert!(c.negligible >= 1, "{c:?}");
    assert_eq!(items, items_before);
}

#[test]
fn min_move_is_converted_to_board_units() {
    let (mut board, _, _) = load();
    let mut s = fr_settings::default_settings(1);
    s.tighten_min_move_um = Some(1.0);
    board.apply_tighten_settings(&s);
    assert_eq!(board.tighten_min_move, 10.0); // (resolution um 10)
    s.tighten_min_move_um = None;
    board.apply_tighten_settings(&s);
    assert_eq!((board.tighten_min_move, board.tighten_round_cap), (0.0, 0));
}

#[test]
fn work_stats_are_shared_with_board_clones() {
    let (board, _, _) = load();
    let clone = board.clone();
    clone.work_stats.record_search(7, true);
    clone.work_stats.record_tighten(3, false);
    let snap = board.work_stats.snapshot();
    assert_eq!(snap.iter().find(|(k, _)| *k == "max_expansions").map(|p| p.1), Some(7));
    assert_eq!(snap.iter().find(|(k, _)| *k == "expansion_caps").map(|p| p.1), Some(1));
    assert_eq!(snap.iter().find(|(k, _)| *k == "max_tighten_rounds").map(|p| p.1), Some(3));
}

/// The recorded fixture (see the module comment).
const HAIRPIN_DSN: &str = r#"(pcb "hairpin"
  (parser
    (string_quote ")
    (space_in_quoted_tokens on)
    (host_cad " KiCad's Pcbnew")
    (host_version "10.0 pcbkit"))
  (resolution um 10)
  (unit um)
  (structure
    (layer F.Cu
      (type signal)
      (property
        (index 0)))
    (layer In1.Cu
      (type power)
      (property
        (index 1)))
    (layer In2.Cu
      (type power)
      (property
        (index 2)))
    (layer B.Cu
      (type signal)
      (property
        (index 3)))
    (boundary
      (path pcb 0 10000 -10000 75000 -10000 75000 -60000 10000 -60000 10000 -10000))
    (plane GND
      (polygon In1.Cu 0 9950 -9950 75050 -9950 75050 -60050 9950 -60050 9950 -9950))
    (plane VDD_3V3
      (polygon In2.Cu 0 9950 -9950 75050 -9950 75050 -60050 9950 -60050 9950 -9950))
    (via "Via[0-3]_700:300_um" "Via[0-3]_500:200_um")
    (rule
      (width 150)
      (clearance 150)
      (clearance 37.5
        (type smd_smd))))
  (placement
    (component "example_stm32io_components_ME6211C33M5G-N:SOT-23-5"
      (place U3 22300 -29650 front 0
        (PN "ME6211C33M5G-N"))))
  (library
    (image "example_stm32io_components_ME6211C33M5G-N:SOT-23-5"
      (outline
        (polygon signal 0 -2045 1495 -1045 1495 -1045 1695 1045 1695 1045 1495 2045 1495 2045 395 1045 395 1045 -395 2045 -395 2045 -1495 1045 -1495 1045 -1695 -1045 -1695 -1045 -1495 -2045 -1495))
      (outline
        (path signal 120 -910 1560 910 1560))
      (outline
        (path signal 120 -910 1510 -910 1560))
      (outline
        (path signal 120 -910 -1560 -910 -1510))
      (outline
        (path signal 120 910 1560 910 1510))
      (outline
        (path signal 120 910 390 910 -390))
      (outline
        (path signal 120 910 -1510 910 -1560))
      (outline
        (path signal 120 910 -1560 -910 -1560))
      (pin RoundRect[T]Pad_1325.000000x600.000000_150.571000_um_0.000000_0 1 -1137.5 950)
      (pin RoundRect[T]Pad_1325.000000x600.000000_150.571000_um_0.000000_0 2 -1137.5 0)
      (pin RoundRect[T]Pad_1325.000000x600.000000_150.571000_um_0.000000_0 3 -1137.5 -950)
      (pin RoundRect[T]Pad_1325.000000x600.000000_150.571000_um_0.000000_0 4 1137.5 -950)
      (pin RoundRect[T]Pad_1325.000000x600.000000_150.571000_um_0.000000_0 5 1137.5 950))
    (padstack RoundRect[T]Pad_1325.000000x600.000000_150.571000_um_0.000000_0
      (shape
        (polygon F.Cu 0 -663.071 150 -651.609 207.621 -618.97 256.47 -570.121 289.109 -512.5 300.571 512.5 300.571 570.121 289.109 618.97 256.47 651.609 207.621 663.071 150 663.071 -150 651.609 -207.621 618.97 -256.47 570.121 -289.109 512.5 -300.571 -512.5 -300.571 -570.121 -289.109 -618.97 -256.47 -651.609 -207.621 -663.071 -150 -663.071 150))
      (attach off))
    (padstack "Via[0-3]_700:300_um"
      (shape
        (circle F.Cu 700))
      (shape
        (circle In1.Cu 700))
      (shape
        (circle In2.Cu 700))
      (shape
        (circle B.Cu 700))
      (attach off))
    (padstack "Via[0-3]_500:200_um"
      (shape
        (circle F.Cu 500))
      (shape
        (circle In1.Cu 500))
      (shape
        (circle In2.Cu 500))
      (shape
        (circle B.Cu 500))
      (attach off)))
  (network
    (net VDD_3V3
      (pins U3-5))
    (net GND
      (pins U3-2))
    (net PWR.VSYS
      (pins U3-1 U3-3))
    (net PWR.U_LDO.NC_U_NC
      (pins U3-4))
    (class kicad_default PWR.U_LDO.NC_U_NC
      (circuit
        (use_via "Via[0-3]_700:300_um"))
      (rule
        (width 150)
        (clearance 150)))
    (class power_390um VDD_3V3
      (circuit
        (use_via "Via[0-3]_700:300_um"))
      (rule
        (width 390)
        (clearance 150)))
    (class power PWR.VSYS
      (circuit
        (use_via "Via[0-3]_700:300_um"))
      (rule
        (width 604.26)
        (clearance 150)))
    (class ground GND
      (circuit
        (use_via "Via[0-3]_700:300_um"))
      (rule
        (width 300)
        (clearance 150))))
  (wiring
    (wire
      (path F.Cu 604.2 21162.5 -30600 21055.4 -30600)
      (net PWR.VSYS))
    (wire
      (path F.Cu 453 21055.4 -30600 20858.3 -30402.9 20857.9 -30402.9)
      (net PWR.VSYS))
    (wire
      (path F.Cu 601 21047.1 -30592.4 20857.9 -30403.2 20857.9 -30402.9)
      (net PWR.VSYS))
    (wire
      (path F.Cu 604.2 21047.1 -30592.4 21929.9 -30592.4 22277.9 -30244.4)
      (net PWR.VSYS))))
"#;
