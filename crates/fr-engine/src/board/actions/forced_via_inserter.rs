//! Port of `board/actions/ForcedViaInserter.java`: checking and inserting forced vias.

use fr_geom::limits::SQRT2;
use fr_geom::{Circle, FloatPoint, Point, Shape, Simplex, TileShape};

use crate::ids::{AngleRestriction, ClearanceClassNo, FixedState, LayerNo, NetNo};
use crate::library::Padstack;
use crate::rules::ViaInfo;
use crate::structure::ShapeEntrySide;

use super::super::routing_board::RoutingBoard;
use super::forced_pad_router::{self, CheckDrillResult};

/// The tile shape used for a shape: its bounding box in 90 degree mode, else its bounding
/// octagon.
fn tile_of(board: &RoutingBoard, shape: &Shape) -> TileShape {
    if board.rules.get_trace_angle_restriction() == AngleRestriction::NinetyDegree {
        TileShape::IntBox(shape.bounding_box())
    } else {
        TileShape::IntOctagon(shape.bounding_octagon().expect("ForcedViaInserter: bounding octagon"))
    }
}

/// Java `ForcedViaInserter.checkLayer(viaRadius, clearanceClassIndex, attachSmdAllowed,
/// roomShape, location, layer, netNumbers, maxRecursionDepth, maxViaRecursionDepth, board,
/// traceHalfWidth, traceClearanceClass)`: checks if a via is possible on the layer after shoving
/// obstacle traces aside. `room_shape` is used for calculating the from side.
#[allow(clippy::too_many_arguments)]
pub fn check_layer(
    board: &mut RoutingBoard,
    via_radius: f64,
    clearance_class: ClearanceClassNo,
    attach_smd_allowed: bool,
    room_shape: &TileShape,
    location: &Point,
    layer: LayerNo,
    net_numbers: &[NetNo],
    max_recursion_depth: i32,
    max_via_recursion_depth: i32,
    trace_half_width: i32,
    trace_clearance_class: ClearanceClassNo,
) -> CheckDrillResult {
    if via_radius <= 0.0 {
        return CheckDrillResult::Drillable;
    }
    let Point::Int(int_location) = location else {
        return CheckDrillResult::NotDrillable;
    };
    let via_shape = Shape::Circle(Circle::new(*int_location, via_radius.ceil() as i32));
    let check_radius =
        via_radius + 0.5 * board.clearance_value(clearance_class, clearance_class, layer) as f64 + board.min_trace_half_width() as f64;
    let is_90_degree = board.rules.get_trace_angle_restriction() == AngleRestriction::NinetyDegree;
    let tile_shape = tile_of(board, &via_shape);
    let Some(from_side) = calculate_from_side(&location.to_float(), &tile_shape, &room_shape.to_simplex(), check_radius, is_90_degree) else {
        return CheckDrillResult::NotDrillable;
    };
    let via_result = forced_pad_router::check_forced_pad(
        board,
        &tile_shape,
        Some(&from_side),
        layer,
        net_numbers,
        clearance_class,
        attach_smd_allowed,
        None,
        max_recursion_depth,
        max_via_recursion_depth,
        false,
        None,
    );
    if via_result == CheckDrillResult::NotDrillable {
        return via_result;
    }
    if trace_half_width <= 0 {
        return via_result;
    }
    let start_trace_shape = tile_of(board, &Shape::Circle(Circle::new(*int_location, trace_half_width)));
    let trace_result = forced_pad_router::check_forced_pad(
        board,
        &start_trace_shape,
        Some(&from_side),
        layer,
        net_numbers,
        trace_clearance_class,
        true,
        None,
        max_recursion_depth,
        max_via_recursion_depth,
        false,
        None,
    );
    if trace_result == CheckDrillResult::NotDrillable {
        return trace_result;
    }
    if via_result == CheckDrillResult::DrillableWithAttachSmd || trace_result == CheckDrillResult::DrillableWithAttachSmd {
        return CheckDrillResult::DrillableWithAttachSmd;
    }
    CheckDrillResult::Drillable
}

/// The pad shape of the via on `layer` (translated to `location`), or the hole check shape on a
/// layer without pad, with its clearance class (Java loop head of `check` and `insert`).
fn via_layer_shape(padstack: &Padstack, layer: LayerNo, location: &Point, hole_shape: &Option<Shape>, via_clearance_class: ClearanceClassNo) -> Option<(Shape, ClearanceClassNo)> {
    match padstack.get_shape(layer) {
        None => {
            // The drill hole itself must keep hole clearance from copper on this layer.
            hole_shape.as_ref().map(|h| (h.clone(), 0))
        }
        Some(pad) => {
            let translate_vector = location.difference_by(&Point::ZERO);
            Some((pad.to_shape().translate_by(&translate_vector), via_clearance_class))
        }
    }
}

/// Java `ForcedViaInserter.check(viaInfo, location, netNumbers, maxRecursionDepth,
/// maxViaRecursionDepth, board, tracePenHalfwidthArr, traceClearanceClassIndex)`: checks if a via
/// is possible after shoving obstacle traces aside. `trace_pen_half_widths`: Java `null` is an
/// empty slice.
#[allow(clippy::too_many_arguments)]
pub fn check(
    board: &mut RoutingBoard,
    via_info: &ViaInfo,
    location: &Point,
    net_numbers: &[NetNo],
    max_recursion_depth: i32,
    max_via_recursion_depth: i32,
    trace_pen_half_widths: Option<&[i32]>,
    trace_clearance_class: ClearanceClassNo,
) -> bool {
    let calc_from_side_offset = board.min_trace_half_width();
    let via_padstack = board.library.padstacks.get(via_info.get_padstack()).expect("ForcedViaInserter: padstack").clone();
    let hole_shape = hole_check_shape(board, &via_padstack, location);
    for i in via_padstack.from_layer()..=via_padstack.to_layer() {
        let Some((current_pad_shape, current_cl)) = via_layer_shape(&via_padstack, i, location, &hole_shape, via_info.get_clearance_class_index()) else {
            continue;
        };
        let tile_shape = tile_of(board, &current_pad_shape);
        let from_side = forced_pad_router::calc_from_side(board, &tile_shape, location, i, calc_from_side_offset, current_cl);
        if forced_pad_router::check_forced_pad(
            board,
            &tile_shape,
            Some(&from_side),
            i,
            net_numbers,
            current_cl,
            via_info.attach_smd_allowed(),
            None,
            max_recursion_depth,
            max_via_recursion_depth,
            false,
            None,
        ) == CheckDrillResult::NotDrillable
        {
            board.set_shove_failing_layer(i);
            return false;
        }
        if current_cl != 0 {
            if let Some(hole) = &hole_shape {
                // The drill hole must ALSO keep hole clearance from other-net copper on layers
                // where the pad exists.
                let hole_tile = tile_of(board, hole);
                if forced_pad_router::check_forced_pad(
                    board,
                    &hole_tile,
                    Some(&from_side),
                    i,
                    net_numbers,
                    0,
                    via_info.attach_smd_allowed(),
                    None,
                    max_recursion_depth,
                    max_via_recursion_depth,
                    false,
                    None,
                ) == CheckDrillResult::NotDrillable
                {
                    board.set_shove_failing_layer(i);
                    return false;
                }
            }
        }
        if let (Some(pen), Point::Int(trace_point)) = (trace_pen_half_widths, location) {
            if (i as usize) < pen.len() && pen[i as usize] > 0 {
                let start_trace_shape = tile_of(board, &Shape::Circle(Circle::new(*trace_point, pen[i as usize])));
                if forced_pad_router::check_forced_pad(
                    board,
                    &start_trace_shape,
                    Some(&from_side),
                    i,
                    net_numbers,
                    trace_clearance_class,
                    true,
                    None,
                    max_recursion_depth,
                    max_via_recursion_depth,
                    false,
                    None,
                ) == CheckDrillResult::NotDrillable
                {
                    board.set_shove_failing_layer(i);
                    return false;
                }
            }
        }
    }
    true
}

/// Java `ForcedViaInserter.insert(viaInfo, location, netNumbers, traceClearanceClassIndex,
/// tracePenHalfwidthArr, maxRecursionDepth, maxViaRecursionDepth, board)`: shoves traces aside
/// and inserts the via. Returns false if that failed (the database may be damaged, an undo is
/// necessary). `trace_pen_half_widths` is indexed by layer (Java requires the array).
#[allow(clippy::too_many_arguments)]
pub fn insert(
    board: &mut RoutingBoard,
    via_info: &ViaInfo,
    location: &Point,
    net_numbers: &[NetNo],
    trace_clearance_class: ClearanceClassNo,
    trace_pen_half_widths: &[i32],
    max_recursion_depth: i32,
    max_via_recursion_depth: i32,
) -> bool {
    let calc_from_side_offset = board.min_trace_half_width();
    let via_padstack = board.library.padstacks.get(via_info.get_padstack()).expect("ForcedViaInserter: padstack").clone();
    let hole_shape = hole_check_shape(board, &via_padstack, location);
    for i in via_padstack.from_layer()..=via_padstack.to_layer() {
        let Some((current_pad_shape, current_cl)) = via_layer_shape(&via_padstack, i, location, &hole_shape, via_info.get_clearance_class_index()) else {
            continue;
        };
        let start_trace_circle = match location {
            Point::Int(p) if trace_pen_half_widths[i as usize] > 0 => Some(Circle::new(*p, trace_pen_half_widths[i as usize])),
            _ => None,
        };
        let tile_shape = tile_of(board, &current_pad_shape);
        let start_trace_shape = start_trace_circle.map(|c| tile_of(board, &Shape::Circle(c)));
        let from_side = forced_pad_router::calc_from_side(board, &tile_shape, location, i, calc_from_side_offset, current_cl);
        if !forced_pad_router::forced_pad(
            board,
            &tile_shape,
            Some(&from_side),
            i,
            net_numbers,
            current_cl,
            via_info.attach_smd_allowed(),
            None,
            max_recursion_depth,
            max_via_recursion_depth,
        ) {
            board.set_shove_failing_layer(i);
            return false;
        }
        if current_cl != 0 {
            if let Some(hole) = &hole_shape {
                let hole_tile = tile_of(board, hole);
                if !forced_pad_router::forced_pad(
                    board,
                    &hole_tile,
                    Some(&from_side),
                    i,
                    net_numbers,
                    0,
                    via_info.attach_smd_allowed(),
                    None,
                    max_recursion_depth,
                    max_via_recursion_depth,
                ) {
                    board.set_shove_failing_layer(i);
                    return false;
                }
            }
        }
        if let Some(start_trace_shape) = start_trace_shape {
            // necessary in case start_trace_shape is bigger than tile_shape
            if !forced_pad_router::forced_pad(
                board,
                &start_trace_shape,
                Some(&from_side),
                i,
                net_numbers,
                trace_clearance_class,
                true,
                None,
                max_recursion_depth,
                max_via_recursion_depth,
            ) {
                board.set_shove_failing_layer(i);
                return false;
            }
        }
    }
    board.insert_via(
        via_info.get_padstack(),
        location.clone(),
        net_numbers,
        via_info.get_clearance_class_index(),
        FixedState::Unfixed,
        via_info.attach_smd_allowed(),
    );
    true
}

/// Java `holeCheckShape(padstack, location, board)`: the hole clearance substitute shape of a
/// copper-less layer of the via (`None` if the rule is off or no drill radius is known).
fn hole_check_shape(board: &RoutingBoard, padstack: &Padstack, location: &Point) -> Option<Shape> {
    let hole_clearance = board.rules.get_hole_clearance();
    let Point::Int(center) = location else {
        return None;
    };
    if hole_clearance <= 0 {
        return None;
    }
    let drill_radius = padstack.get_drill_radius();
    if drill_radius <= 0.0 {
        return None;
    }
    // Inflate by the hole clearance itself and check with the null clearance class (0), so the
    // requirement is exact hole-to-copper spacing regardless of the neighbor's class.
    Some(Shape::Circle(Circle::new(*center, (drill_radius + hole_clearance as f64 + 10.0).ceil() as i32)))
}

/// Java `calculateFromSide(viaLocation, viaShape, roomShape, dist, is90Degree)`.
fn calculate_from_side(via_location: &FloatPoint, via_shape: &TileShape, room_shape: &Simplex, dist: f64, is_90_degree: bool) -> Option<ShapeEntrySide> {
    let via_box = via_shape.bounding_box();
    let room = TileShape::Simplex(room_shape.clone());
    for i in 0..4 {
        let (check_point, border_x, border_y) = match i {
            0 => (FloatPoint::new(via_location.x, via_location.y - dist), via_location.x, via_box.ll.y as f64),
            1 => (FloatPoint::new(via_location.x + dist, via_location.y), via_box.ur.x as f64, via_location.y),
            2 => (FloatPoint::new(via_location.x, via_location.y + dist), via_location.x, via_box.ur.y as f64),
            _ => (FloatPoint::new(via_location.x - dist, via_location.y), via_box.ll.x as f64, via_location.y),
        };
        if room.contains_float(&check_point) {
            let from_side_index = if is_90_degree { i } else { 2 * i };
            return Some(ShapeEntrySide::new(from_side_index, Some(FloatPoint::new(border_x, border_y))));
        }
    }
    if is_90_degree {
        return None;
    }
    // try the diagonal directions
    let dist = dist / SQRT2;
    let border_dist = via_box.max_width() / (2.0 * SQRT2);
    for i in 0..4 {
        let (check_point, border_x, border_y) = match i {
            0 => (FloatPoint::new(via_location.x + dist, via_location.y - dist), via_location.x + border_dist, via_location.y - border_dist),
            1 => (FloatPoint::new(via_location.x + dist, via_location.y + dist), via_location.x + border_dist, via_location.y + border_dist),
            2 => (FloatPoint::new(via_location.x - dist, via_location.y + dist), via_location.x - border_dist, via_location.y + border_dist),
            _ => (FloatPoint::new(via_location.x - dist, via_location.y - dist), via_location.x - border_dist, via_location.y - border_dist),
        };
        if room.contains_float(&check_point) {
            return Some(ShapeEntrySide::new(2 * i + 1, Some(FloatPoint::new(border_x, border_y))));
        }
    }
    None
}
