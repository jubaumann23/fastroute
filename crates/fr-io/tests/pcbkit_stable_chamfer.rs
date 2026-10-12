//! pcbkit R6: end-corner smoothing that re-created its own cut every round converges with
//! `router.tighten_stable_chamfer`.
//!
//! Two traces of a net that run along each other from a shared corner (a hairpin) meet at an acute angle. The
//! 45 degree acute-angle smoothing (`build_smoothened_start`) cuts the corner with a chamfer and
//! `split_traces` cuts the contact trace at the new corner. Two cases re-create the cut every round:
//! - the chamfer is clipped by a short contact jog (`other_dist`): its new corner lands at the jog's far end and
//!   a jog of the same length is cut off the leg behind it, so the tip moves one jog (15-87 units) per round;
//!   these were all 13 `router.max_tighten_rounds` hits of router bench B2 (64-473 rounds per pass);
//! - the contact has another width (a neck-down beside its full-width twin): the pieces cut off it never merge
//!   with the cut trace, so the leg turns into a staircase of stubs (49 rounds, just under the cap).
//!
//! The fixtures are recorded: the session at the start of the pass (the fork's route, toolkit settings), cut down
//! to a box around the hairpin with `(wiring)` from the session.
//! - [`SMALL8_DSN`]: cypress_small8 energy-1 (toolkit router bench; NVlabs/Cypress, Apache-2.0), net
//!   S_HVDD_PL0_TAO_R_3 on B.Cu, both legs 1500 um wide, one a dead-end tail 16 units off the via; jog zipper.
//!   Upstream: 473 rounds.
//! - [`STM32IO_DSN`]: stm32io energy-9 (toolkit's own board), net USB_D_N on F.Cu, legs 1124 um and the
//!   1000 um neck-down; jog zipper with stubs. Upstream: 64 rounds and 61 stubs.
//! - [`STM32IO_P_DSN`]: the same board, net USB_D_P, a 1124 um leg beside its 1500 um twin; staircase.
//!   Upstream: 49 rounds and 42 stubs.

use fr_engine::board::RoutingBoard;
use fr_geom::FloatPoint;
use std::sync::atomic::Ordering;

/// Rounds after which the tests stop the upstream pass (the round cap, `router.max_tighten_rounds`).
const CAP: u32 = 60;

struct Case {
    dsn: &'static str,
    net: &'static str,
    /// The changed area of the recorded pass: layer and box (board units).
    area: (i32, [f64; 4]),
}

const SMALL8: Case = Case { dsn: SMALL8_DSN, net: "S_HVDD_PL0_TAO_R_3", area: (1, [160617.0, -307167.0, 164155.0, -303645.0]) };
const STM32IO_P: Case = Case { dsn: STM32IO_P_DSN, net: "USB_D_P", area: (0, [576539.0, -386834.0, 637875.0, -342814.0]) };
const STM32IO: Case = Case { dsn: STM32IO_DSN, net: "USB_D_N", area: (0, [603586.0, -402219.0, 668050.0, -375265.0]) };

fn load(case: &Case) -> (RoutingBoard, i32, Vec<fr_settings::ExpansionCostFactor>) {
    let mut settings = fr_settings::default_settings(1);
    let mut board = fr_io::post_load::load_from_specctra_dsn(case.dsn.as_bytes(), &mut settings).expect("load");
    fr_io::post_load::prepare_for_routing(&mut board, &mut settings, None);
    fr_engine::pipeline::deferred_post_load_processing(&mut board);
    let net = board.rules.nets.get_by_name(case.net, 1).expect("net").net_number;
    (board, net, settings.get_trace_costs())
}

#[derive(Debug)]
struct Pass {
    rounds: u64,
    caps: u64,
    skipped: u64,
    items_before: usize,
    items: usize,
}

/// The recorded pass: all nets (the optimizer's `opt_changed_area(&[], ..)`) for small8, the routed net for
/// the stm32io cases; `router.tighten_min_move_um` 1 as the toolkit sends it.
fn tighten(case: &Case, stable_chamfer: Option<bool>, all_nets: bool) -> Pass {
    let (mut board, net, costs) = load(case);
    let mut s = fr_settings::default_settings(1);
    s.max_tighten_rounds = Some(f64::from(CAP));
    s.tighten_min_move_um = Some(1.0);
    s.tighten_stable_chamfer = stable_chamfer;
    board.apply_tighten_settings(&s);
    let items_before = board.get_items().len();
    let (layer, [x1, y1, x2, y2]) = case.area;
    board.start_marking_changed_area();
    for (x, y) in [(x1, y1), (x2, y1), (x2, y2), (x1, y2)] {
        board.join_changed_area(&FloatPoint::new(x, y), layer);
    }
    let nets: Vec<i32> = if all_nets { vec![] } else { vec![net] };
    board.opt_changed_area(&nets, None, 500, Some(&costs), None, 0);
    let w = &board.work_stats;
    Pass {
        rounds: w.max_tighten_rounds.load(Ordering::Relaxed),
        caps: w.tighten_caps.load(Ordering::Relaxed),
        skipped: w.negligible_moves.load(Ordering::Relaxed),
        items_before,
        items: board.get_items().len(),
    }
}

#[test]
fn upstream_zips_the_small8_hairpin_until_the_round_cap() {
    let p = tighten(&SMALL8, None, true);
    assert_eq!((p.rounds, p.caps), (u64::from(CAP), 1), "{p:?}");
}

#[test]
fn the_stable_chamfer_converges_the_small8_pass() {
    let p = tighten(&SMALL8, Some(true), true);
    assert_eq!(p.caps, 0, "{p:?}");
    assert!(p.rounds <= 2, "{p:?}");
    assert!(p.skipped >= 1, "{p:?}");
    assert_eq!(p.items, p.items_before, "{p:?}");
}

#[test]
fn upstream_leaves_a_stub_staircase_at_the_stm32io_neck_hairpin() {
    let p = tighten(&STM32IO, None, false);
    assert_eq!((p.rounds, p.caps), (u64::from(CAP), 1), "{p:?}");
    // two stubs of alternating width per jog step
    assert!(p.items >= p.items_before + 50, "{p:?}");
}

#[test]
fn the_stable_chamfer_converges_the_stm32io_pass_without_stubs() {
    let p = tighten(&STM32IO, Some(true), false);
    assert_eq!(p.caps, 0, "{p:?}");
    assert!(p.rounds <= 2, "{p:?}");
    assert_eq!(p.items, p.items_before, "{p:?}");
}

#[test]
fn upstream_turns_the_usb_d_p_twin_leg_into_a_staircase() {
    let p = tighten(&STM32IO_P, None, false);
    assert!(p.rounds >= 45 && p.caps == 0, "{p:?}");
    assert!(p.items >= p.items_before + 40, "{p:?}");
}

#[test]
fn the_stable_chamfer_leaves_the_usb_d_p_legs_alone() {
    let p = tighten(&STM32IO_P, Some(true), false);
    assert_eq!(p.caps, 0, "{p:?}");
    assert!(p.rounds <= 3, "{p:?}");
    assert!(p.items <= p.items_before, "{p:?}");
}

#[test]
fn the_key_is_off_unless_set_and_reaches_the_board() {
    let (mut board, _, _) = load(&SMALL8);
    assert!(!board.tighten_stable_chamfer);
    let mut s = fr_settings::default_settings(1);
    fr_settings::reflect::set_field_value(&mut s, "tighten_stable_chamfer", "true").expect("known key");
    board.apply_tighten_settings(&s);
    assert!(board.tighten_stable_chamfer);
    s.tighten_stable_chamfer = None;
    board.apply_tighten_settings(&s);
    assert!(!board.tighten_stable_chamfer);
}

/// cypress_small8 energy-1 (see the module comment).
const SMALL8_DSN: &str = r#"(pcb cut
  (parser
    (string_quote ")
    (space_in_quoted_tokens on)
    (host_cad "KiCad's Pcbnew")
    (host_version "8.0 pcbkit"))
  (resolution um 10)
  (unit um)
  (structure
    (layer F.Cu (type signal) (property (index 0)))
    (layer B.Cu (type signal) (property (index 1)))
    (boundary (path pcb 0 10000 -10000 45338.1 -10000 45338.1 -34160.9 10000 -34160.9 10000 -10000))
    (via "Via[0-1]_700:300_um")
    (rule (width 150) (clearance 150) (clearance 37.5 (type smd_smd))))
  (placement)
  (library (padstack "Via[0-1]_700:300_um" (shape (circle F.Cu 700)) (shape (circle B.Cu 700)) (attach off)))
  (network
    (net S_DVDD_PL0_SENSE_P_R_3)
    (net S_DVDD_PL0_TAO_R_3)
    (net S_DVDD_S0_ASIC1_SENSE_N)
    (net S_HVDD_DVDD_PL0_ADDR_PMBUS_3)
    (net S_HVDD_PL0_TAO_R_3)
    (class kicad_default S_DVDD_PL0_SENSE_P_R_3 S_DVDD_PL0_TAO_R_3 S_DVDD_S0_ASIC1_SENSE_N S_HVDD_DVDD_PL0_ADDR_PMBUS_3 S_HVDD_PL0_TAO_R_3
      (circuit (use_via "Via[0-1]_700:300_um"))
      (rule (width 150) (clearance 150))))
  (wiring
    (wire (path F.Cu 150 15500 -30600 15826.7 -30600) (net "S_DVDD_PL0_SENSE_P_R_3"))
    (wire
      (path F.Cu 150 15826.7 -30600 15826.7 -32012.5 15700.9 -32138.3 14988.8 -32138.3 14554.6 -31704.1)
      (net "S_DVDD_PL0_SENSE_P_R_3"))
    (wire (path F.Cu 150 15961.1 -29074.8 16472 -29585.7) (net "S_DVDD_PL0_TAO_R_3"))
    (wire (path B.Cu 150 11552.6 -23954.5 11552.6 -24666.3 16472 -29585.7) (net "S_DVDD_PL0_TAO_R_3"))
    (wire (path F.Cu 112.4 15500 -29000 15807.9 -29000) (net "S_DVDD_PL0_TAO_R_3"))
    (wire (path F.Cu 112.4 15961.1 -29074.8 15882.7 -29074.8 15807.9 -29000) (net "S_DVDD_PL0_TAO_R_3"))
    (via "Via[0-1]_700:300_um" 16472 -29585.7 (net "S_DVDD_PL0_TAO_R_3"))
    (wire (path F.Cu 112.4 15500 -31000 15192.1 -31000) (net "S_DVDD_S0_ASIC1_SENSE_N"))
    (wire (path F.Cu 112.4 14200.2 -30925.2 15117.3 -30925.2 15192.1 -31000) (net "S_DVDD_S0_ASIC1_SENSE_N"))
    (wire
      (path F.Cu 150 15500 -29800 14500.4 -29800 14422.1 -29878.3 13577.8 -29878.3 13475.9 -29776.4 13475.9 -29240.4)
      (net "S_HVDD_DVDD_PL0_ADDR_PMBUS_3"))
    (wire (path B.Cu 150 16061.7 -30364.5 16063.3 -30364.5 16415.5 -30716.7) (net "S_HVDD_PL0_TAO_R_3"))
    (wire (path B.Cu 150 16061.7 -30364.5 12161.8 -26464.6) (net "S_HVDD_PL0_TAO_R_3"))
    (wire (path B.Cu 150 16415.5 -30718.3 16061.7 -30364.5) (net "S_HVDD_PL0_TAO_R_3"))
    (wire (path F.Cu 150 15884.8 -29474.8 15884.8 -30186 16415.5 -30716.7) (net "S_HVDD_PL0_TAO_R_3"))
    (wire (path F.Cu 112.4 15500 -29400 15807.9 -29400) (net "S_HVDD_PL0_TAO_R_3"))
    (wire (path F.Cu 112.4 15884.8 -29474.8 15882.7 -29474.8 15807.9 -29400) (net "S_HVDD_PL0_TAO_R_3"))
    (via "Via[0-1]_700:300_um" 16415.5 -30716.7 (net "S_HVDD_PL0_TAO_R_3"))))
"#;

/// stm32io energy-9 (see the module comment).
const STM32IO_DSN: &str = r#"(pcb cut
  (parser
    (string_quote ")
    (space_in_quoted_tokens on)
    (host_cad "KiCad's Pcbnew")
    (host_version "10.0 pcbkit"))
  (resolution um 10)
  (unit um)
  (structure
    (layer F.Cu (type signal) (property (index 0)))
    (layer In1.Cu (type power) (property (index 1)))
    (layer In2.Cu (type power) (property (index 2)))
    (layer B.Cu (type signal) (property (index 3)))
    (boundary (path pcb 0 10000 -10000 75000 -10000 75000 -60000 10000 -60000 10000 -10000))
    (via "Via[0-3]_700:300_um" "Via[0-3]_500:200_um")
    (rule (width 150) (clearance 150) (clearance 37.5 (type smd_smd))))
  (placement
    (component "example_stm32io_components_USBLC6-2SC6:SOT-23-6"
      (place U6 62650 -39200 front 0 (PN "USBLC6-2SC6"))))
  (library
    (image "example_stm32io_components_USBLC6-2SC6:SOT-23-6"
      (outline
        (polygon signal 0 -2045 1495 -1045 1495 -1045 1695 1045 1695 1045 1495 2045 1495 2045 -1495 1045 -1495 1045 -1695 -1045 -1695 -1045 -1495 -2045 -1495))
      (outline (path signal 120 -910 1560 910 1560))
      (outline (path signal 120 -910 1510 -910 1560))
      (outline (path signal 120 -910 -1560 -910 -1510))
      (outline (path signal 120 910 1560 910 1510))
      (outline (path signal 120 910 -1510 910 -1560))
      (outline (path signal 120 910 -1560 -910 -1560))
      (pin RoundRect[T]Pad_1325.000000x600.000000_150.571000_um_0.000000_0 1 -1137.5 950)
      (pin RoundRect[T]Pad_1325.000000x600.000000_150.571000_um_0.000000_0 2 -1137.5 0)
      (pin RoundRect[T]Pad_1325.000000x600.000000_150.571000_um_0.000000_0 3 -1137.5 -950)
      (pin RoundRect[T]Pad_1325.000000x600.000000_150.571000_um_0.000000_0 4 1137.5 -950)
      (pin RoundRect[T]Pad_1325.000000x600.000000_150.571000_um_0.000000_0 5 1137.5 0)
      (pin RoundRect[T]Pad_1325.000000x600.000000_150.571000_um_0.000000_0 6 1137.5 950))
    (padstack RoundRect[T]Pad_1325.000000x600.000000_150.571000_um_0.000000_0
      (shape
        (polygon F.Cu 0 -663.071 150 -651.609 207.621 -618.97 256.47 -570.121 289.109 -512.5 300.571 512.5 300.571 570.121 289.109 618.97 256.47 651.609 207.621 663.071 150 663.071 -150 651.609 -207.621 618.97 -256.47 570.121 -289.109 512.5 -300.571 -512.5 -300.571 -570.121 -289.109 -618.97 -256.47 -651.609 -207.621 -663.071 -150 -663.071 150))
      (attach off))
    (padstack "Via[0-3]_700:300_um"
      (shape (circle F.Cu 700))
      (shape (circle In1.Cu 700))
      (shape (circle In2.Cu 700))
      (shape (circle B.Cu 700))
      (attach off))
    (padstack "Via[0-3]_500:200_um"
      (shape (circle F.Cu 500))
      (shape (circle In1.Cu 500))
      (shape (circle In2.Cu 500))
      (shape (circle B.Cu 500))
      (attach off)))
  (network
    (net GND (pins U6-2))
    (net VBUS (pins U6-5))
    (net USB_D_P (pins U6-1 U6-6))
    (net USB_D_N (pins U6-3 U6-4))
    (class kicad_default USB_D_N USB_D_P
      (circuit (use_via "Via[0-3]_700:300_um"))
      (rule (width 150) (clearance 150)))
    (class power VBUS (circuit (use_via "Via[0-3]_700:300_um")) (rule (width 604.26) (clearance 150)))
    (class ground GND (circuit (use_via "Via[0-3]_700:300_um")) (rule (width 300) (clearance 150))))
  (wiring
    (wire
      (path F.Cu 150 52914.3 -29774.5 60666.4 -37526.5 60358.6 -37834.3 60358.6 -38181.1 60524.3 -38346.8 60758.7 -38346.8 60924.4 -38181.1 60924.4 -38068.7 60968.7 -38024.4 61164.2 -38024.4 61389.8 -38250 61512.5 -38250)
      (net "USB_D_P"))
    (wire
      (path F.Cu 100 62650 -38824.4 62297.6 -38824.4 62198.2 -38725 62377.3 -38545.9 62377.3 -37954.1 62170.9 -37747.7 60854.1 -37747.7 60647.7 -37954.1 60647.7 -38066.5)
      (net "USB_D_N"))
    (wire (path F.Cu 100 62650 -38824.4 62650 -38226.8) (net "USB_D_N"))
    (wire
      (path F.Cu 112.4 62650 -38226.8 62650 -38218 62173.5 -37741.5 60851.5 -37741.5 60641.5 -37951.5 60641.5 -38063.9)
      (net "USB_D_N"))
    (wire (path F.Cu 150 63787.5 -40150 63438.8 -40150 62650 -39361.2 62650 -38824.4) (net "USB_D_N"))))
"#;

/// stm32io energy-9, USB_D_P (see the module comment).
const STM32IO_P_DSN: &str = r#"(pcb cut
  (parser
    (string_quote ")
    (space_in_quoted_tokens on)
    (host_cad "KiCad's Pcbnew")
    (host_version "10.0 pcbkit"))
  (resolution um 10)
  (unit um)
  (structure
    (layer F.Cu (type signal) (property (index 0)))
    (layer In1.Cu (type power) (property (index 1)))
    (layer In2.Cu (type power) (property (index 2)))
    (layer B.Cu (type signal) (property (index 3)))
    (boundary (path pcb 0 10000 -10000 75000 -10000 75000 -60000 10000 -60000 10000 -10000))
    (via "Via[0-3]_700:300_um" "Via[0-3]_500:200_um")
    (rule (width 150) (clearance 150) (clearance 37.5 (type smd_smd))))
  (placement
    (component "stdlib_kicad-footprints_Resistor_SMD:R_0402_1005Metric"
      (place R21 61650 -35500 front 90 (PN "5.1k 1%")))
    (component "stdlib_kicad-footprints_Capacitor_SMD:C_0603_1608Metric"
      (place C23 63350 -35500 front 90 (PN "4.7uF 10V X5R")))
    (component "example_stm32io_components_USBLC6-2SC6:SOT-23-6"
      (place U6 62650 -39200 front 0 (PN "USBLC6-2SC6"))))
  (library
    (image "stdlib_kicad-footprints_Resistor_SMD:R_0402_1005Metric"
      (outline (polygon signal 0 -925 465 925 465 925 -465 -925 -465))
      (outline (path signal 120 -153.641 380 153.641 380))
      (outline (path signal 120 -153.641 -380 153.641 -380))
      (outline (path signal 100 -525 270 525 270 525 -270 -525 -270 -525 270))
      (pin RoundRect[T]Pad_540.000000x640.000000_135.514000_um_0.000000_0 1 -510 0)
      (pin RoundRect[T]Pad_540.000000x640.000000_135.514000_um_0.000000_0 2 510 0))
    (image "stdlib_kicad-footprints_Capacitor_SMD:C_0603_1608Metric"
      (outline (polygon signal 0 -1475 725 1475 725 1475 -725 -1475 -725))
      (outline (path signal 120 -140.58 510 140.58 510))
      (outline (path signal 120 -140.58 -510 140.58 -510))
      (outline (path signal 100 -800 400 800 400 800 -400 -800 -400 -800 400))
      (pin RoundRect[T]Pad_900.000000x950.000000_225.856000_um_0.000000_0 1 -775 0)
      (pin RoundRect[T]Pad_900.000000x950.000000_225.856000_um_0.000000_0 2 775 0))
    (image "example_stm32io_components_USBLC6-2SC6:SOT-23-6"
      (outline
        (polygon signal 0 -2045 1495 -1045 1495 -1045 1695 1045 1695 1045 1495 2045 1495 2045 -1495 1045 -1495 1045 -1695 -1045 -1695 -1045 -1495 -2045 -1495))
      (outline (path signal 120 -910 1560 910 1560))
      (outline (path signal 120 -910 1510 -910 1560))
      (outline (path signal 120 -910 -1560 -910 -1510))
      (outline (path signal 120 910 1560 910 1510))
      (outline (path signal 120 910 -1510 910 -1560))
      (outline (path signal 120 910 -1560 -910 -1560))
      (pin RoundRect[T]Pad_1325.000000x600.000000_150.571000_um_0.000000_0 1 -1137.5 950)
      (pin RoundRect[T]Pad_1325.000000x600.000000_150.571000_um_0.000000_0 2 -1137.5 0)
      (pin RoundRect[T]Pad_1325.000000x600.000000_150.571000_um_0.000000_0 3 -1137.5 -950)
      (pin RoundRect[T]Pad_1325.000000x600.000000_150.571000_um_0.000000_0 4 1137.5 -950)
      (pin RoundRect[T]Pad_1325.000000x600.000000_150.571000_um_0.000000_0 5 1137.5 0)
      (pin RoundRect[T]Pad_1325.000000x600.000000_150.571000_um_0.000000_0 6 1137.5 950))
    (padstack RoundRect[T]Pad_540.000000x640.000000_135.514000_um_0.000000_0
      (shape
        (polygon F.Cu 0 -270.514 185 -260.199 236.859 -230.823 280.823 -186.859 310.199 -135 320.514 135 320.514 186.859 310.199 230.823 280.823 260.199 236.859 270.514 185 270.514 -185 260.199 -236.859 230.823 -280.823 186.859 -310.199 135 -320.514 -135 -320.514 -186.859 -310.199 -230.823 -280.823 -260.199 -236.859 -270.514 -185 -270.514 185))
      (attach off))
    (padstack RoundRect[T]Pad_1325.000000x600.000000_150.571000_um_0.000000_0
      (shape
        (polygon F.Cu 0 -663.071 150 -651.609 207.621 -618.97 256.47 -570.121 289.109 -512.5 300.571 512.5 300.571 570.121 289.109 618.97 256.47 651.609 207.621 663.071 150 663.071 -150 651.609 -207.621 618.97 -256.47 570.121 -289.109 512.5 -300.571 -512.5 -300.571 -570.121 -289.109 -618.97 -256.47 -651.609 -207.621 -663.071 -150 -663.071 150))
      (attach off))
    (padstack RoundRect[T]Pad_900.000000x950.000000_225.856000_um_0.000000_0
      (shape
        (polygon F.Cu 0 -450.856 250 -433.664 336.431 -384.704 409.704 -311.431 458.664 -225 475.856 225 475.856 311.431 458.664 384.704 409.704 433.664 336.431 450.856 250 450.856 -250 433.664 -336.431 384.704 -409.704 311.431 -458.664 225 -475.856 -225 -475.856 -311.431 -458.664 -384.704 -409.704 -433.664 -336.431 -450.856 -250 -450.856 250))
      (attach off))
    (padstack "Via[0-3]_700:300_um"
      (shape (circle F.Cu 700))
      (shape (circle In1.Cu 700))
      (shape (circle In2.Cu 700))
      (shape (circle B.Cu 700))
      (attach off))
    (padstack "Via[0-3]_500:200_um"
      (shape (circle F.Cu 500))
      (shape (circle In1.Cu 500))
      (shape (circle In2.Cu 500))
      (shape (circle B.Cu 500))
      (attach off)))
  (network
    (net GND (pins C23-2 U6-2 R21-2))
    (net VBUS (pins C23-1 U6-5))
    (net USB_D_P (pins U6-1 U6-6))
    (net USB_D_N (pins U6-3 U6-4))
    (net USBIN.CC2 (pins R21-1))
    (class kicad_default USBIN.CC2 USB_D_N USB_D_P
      (circuit (use_via "Via[0-3]_700:300_um"))
      (rule (width 150) (clearance 150)))
    (class power VBUS (circuit (use_via "Via[0-3]_700:300_um")) (rule (width 604.26) (clearance 150)))
    (class ground GND (circuit (use_via "Via[0-3]_700:300_um")) (rule (width 300) (clearance 150))))
  (wiring
    (wire (path F.Cu 601 64638.1 -39002.9 63984.6 -39002.9 63787.5 -39200) (net VBUS))
    (wire (path F.Cu 604.2 66805 -40200 65835.2 -40200 64638.1 -39002.9) (net VBUS))
    (wire (path F.Cu 604.2 63350 -36275 64790.9 -36275 65668.5 -35397.4) (net VBUS))
    (wire (path F.Cu 150 57733.7 -34471.2 61512.5 -38250) (net "USB_D_P"))
    (wire (path F.Cu 150 57733.7 -34281.4 57733.7 -34471.2) (net "USB_D_P"))
    (wire (path F.Cu 150 57733.7 -34471.2 53037 -29774.5 52914.3 -29774.5) (net "USB_D_P"))
    (wire (path F.Cu 112.4 62343.8 -38683.4 62135.7 -38683.4 57733.7 -34281.4) (net "USB_D_P"))
    (wire (path F.Cu 150 63787.5 -38250 62777.2 -38250 62343.8 -38683.4) (net "USB_D_P"))
    (wire (path F.Cu 150 63787.5 -38250 65391.5 -38250 65803.3 -37838.2) (net "USB_D_P"))))
"#;
