//! pcbkit W4 (`router.strict_width`): the maze plans a trace at the distance the inserter needs.
//!
//! The maze keeps a trace centre `compensated_trace_half_width + TRACE_WIDTH_TOLERANCE` from a room border, and the
//! border is the obstacle grown by its clearance compensation, so it plans a trace `clearance + 2` units from fixed
//! copper. The inserter's shove check keeps `clearance + CLEARANCE_SAFETY_MARGIN` (16 units). On a real board
//! (gpio4l-energy-2, strict) most "new connection could not be inserted" failures were this case: the planned segment
//! failed the insert at the class width and passed 2 um narrower. The defaults hide it with a micro neck-down anywhere,
//! which strict width forbids; under strict width the maze now plans with the margin.
//!
//! Board: a fixed pad of no net (a 6 mm wall) and net N1. A horizontal N1 segment is placed along the wall's top edge
//! at the distance the maze plans for the control of each mode, and offered to the inserter's check and insert.

use fr_engine::autoroute::{AutorouteControl, TRACE_WIDTH_TOLERANCE};
use fr_engine::board::{ForcedTraceEnd, RoutingBoard, TimeLimitPolicy};
use fr_geom::{IntPoint, Point, Polyline};

/// The wall pad spans x 2000..8000 um, y -4300..-4000 um (top edge at y = -4000 um); N1 joins two pads far away.
const DSN: &str = r#"(pcb plan.dsn (parser (string_quote ") (space_in_quoted_tokens on) (host_cad "KiCad's Pcbnew") (host_version "10.0.5"))
  (resolution um 10) (unit um)
  (structure
    (layer F.Cu (type signal) (property (index 0))) (layer B.Cu (type signal) (property (index 1)))
    (boundary (path pcb 0  0 0  10000 0  10000 -8000  0 -8000  0 0))
    (via "Via[0-1]_600:300_um")
    (rule (width 300) (clearance 160)))
  (placement (component "WALL" (place U1 5000 -4150 front 0 (PN w)))
             (component "PAD" (place J1 1000 -7000 front 0 (PN a)) (place J2 9000 -7000 front 0 (PN b))))
  (library
    (image "WALL" (pin W 1 0 0))
    (image "PAD" (pin P 1 0 0))
    (padstack W (shape (polygon F.Cu 0 -3000 -150 3000 -150 3000 150 -3000 150)) (attach off))
    (padstack P (shape (polygon F.Cu 0 -300 300 300 300 300 -300 -300 -300)) (attach off))
    (padstack "Via[0-1]_600:300_um" (shape (circle F.Cu 600)) (shape (circle B.Cu 600)) (attach off)))
  (network (net N1 (pins J1-1 J2-1))
    (class kicad_default N1 (circuit (use_via "Via[0-1]_600:300_um"))
      (rule (width 300) (clearance 160)))))"#;

/// The wall's top edge in board units (resolution 10 per um).
const WALL_TOP: i32 = -40_000;
const LAYER: i32 = 0;

fn load(strict: bool) -> (RoutingBoard, AutorouteControl) {
    let mut settings = fr_settings::default_settings(1);
    settings.strict_width = Some(strict);
    let mut board = fr_io::post_load::load_from_specctra_dsn(DSN.as_bytes(), &mut settings).expect("load");
    fr_io::post_load::prepare_for_routing(&mut board, &mut settings, None);
    board.time_limits = TimeLimitPolicy::disabled();
    let net = board.rules.nets.get_by_name("N1", 1).expect("net N1").net_number;
    let ctrl = AutorouteControl::new_default(&board, net, &settings);
    (board, ctrl)
}

/// The horizontal segment across the wall whose centre lies where the maze of `ctrl` plans it: the room border is the
/// wall grown by its clearance compensation, the centre `compensated_trace_half_width + TRACE_WIDTH_TOLERANCE` above it.
fn planned_segment(board: &RoutingBoard, ctrl: &AutorouteControl) -> Polyline {
    let wall_compensation = board.rules.clearance_matrix.clearance_compensation_value(ctrl.trace_clearance_class, LAYER);
    let y = WALL_TOP + wall_compensation + ctrl.compensated_trace_half_width[LAYER as usize] + TRACE_WIDTH_TOLERANCE;
    Polyline::from_points(&[Point::Int(IntPoint::new(30_000, y)), Point::Int(IntPoint::new(70_000, y))])
}

/// (shove check passes, forced insert reaches the end) for the planned segment at the class width.
fn insertable(strict: bool) -> (bool, bool) {
    let (mut board, ctrl) = load(strict);
    let segment = planned_segment(&board, &ctrl);
    let half_width = ctrl.trace_half_width[LAYER as usize];
    let nets = [ctrl.net_number];
    let checked = board.check_forced_trace_polyline(
        &segment,
        half_width,
        LAYER,
        &nets,
        ctrl.trace_clearance_class,
        ctrl.max_shove_trace_recursion_depth,
        ctrl.max_shove_via_recursion_depth,
        ctrl.max_spring_over_recursion_depth,
    );
    let end = board.insert_forced_trace_polyline(
        &segment,
        half_width,
        LAYER,
        &nets,
        ctrl.trace_clearance_class,
        ctrl.max_shove_trace_recursion_depth,
        ctrl.max_shove_via_recursion_depth,
        ctrl.max_spring_over_recursion_depth,
        i32::MAX,
        ctrl.pull_tight_accuracy,
        true,
        None,
    );
    (checked, matches!(end, ForcedTraceEnd::To(_)))
}

#[test]
fn default_planning_puts_a_trace_closer_than_the_inserter_accepts() {
    // the defaults rely on a micro neck-down here (Freerouting behaviour, unchanged)
    assert_eq!(insertable(false), (false, false));
}

#[test]
fn strict_planning_puts_a_trace_where_the_inserter_accepts_it_at_the_class_width() {
    assert_eq!(insertable(true), (true, true));
}
