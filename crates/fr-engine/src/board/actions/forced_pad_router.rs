//! Port of `board/actions/ForcedPadRouter.java`: checking and inserting pads while shoving
//! obstacle traces aside.

use fr_geom::{Direction, Line, Point, Polyline, TileShape};

use crate::datastructures::TimeLimit;
use crate::ids::{ClearanceClassNo, LayerNo, NetNo};
use crate::structure::ShapeEntrySide;

use super::super::item::{ItemKey, StopConnectionOption};
use super::super::optimize::trace_shover::{shape_and_entry_side, shove_obstacles, TraceShover};
use super::super::routing_board::RoutingBoard;
use super::super::shape_trace_entries::ShapeTraceEntries;
use super::drill_item_mover;

/// Java `ForcedPadRouter.CheckDrillResult`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CheckDrillResult {
    Drillable,
    DrillableWithAttachSmd,
    NotDrillable,
}

/// Java `calcCheckShapeForFromSide(shape, shapeCenter, borderLine)`.
fn calc_check_shape_for_from_side(shape_center: &Point, border_line: &Line) -> TileShape {
    let offset_projection = shape_center.to_float().projection_approx(border_line);
    // Make sure, that direction restrictions are retained.
    let current_direction = border_line.direction();
    let lines = vec![
        Line::from_point_direction(shape_center.clone(), current_direction.clone()),
        Line::from_point_direction(shape_center.clone(), current_direction.turn_45_degree(2)),
        Line::from_point_direction(Point::Int(offset_projection.round()), current_direction),
    ];
    let check_line = Polyline::from_lines(lines);
    check_line.offset_shape(1, 0).expect("calcCheckShapeForFromSide: offset shape")
}

/// Java `inFrontOfPad(line, padShape, fromSide, width, withSides)`: checks if the line is in
/// front of the pad when shoving from `from_side` (only implemented for octagons).
fn in_front_of_pad(line: &Line, pad_shape: &TileShape, from_side: i32, width: i32, with_sides: bool) -> bool {
    if !pad_shape.is_int_octagon() {
        // only implemented for octagons
        return true;
    }
    let pad = pad_shape.bounding_octagon().expect("inFrontOfPad: bounding octagon");
    let (Point::Int(a), Point::Int(b)) = (&line.a, &line.b) else {
        // not implemented
        return true;
    };
    let w = width;
    let diag_width = width as f64 * 2f64.sqrt();
    let (ax, ay, bx, by) = (a.x, a.y, b.x, b.y);
    // Java int arithmetic, compared with double diagonal widths
    let d = |v: i32| v as f64;
    let mut result;
    match from_side {
        0 => {
            result = ay.min(by) >= pad.top_y + w
                || d((ax - ay).max(bx - by)) <= d(pad.upper_left_diagonal_x) - diag_width
                || d((ax + ay).min(bx + bx)) >= d(pad.upper_right_diagonal_x) + diag_width;
            if with_sides && !result {
                result = (ax.max(bx) <= pad.left_x - w && d((ax - ay).min(bx - by)) <= d(pad.upper_left_diagonal_x) - diag_width)
                    || (ax.min(bx) >= pad.right_x + w && d((ax + ay).min(bx + by)) >= d(pad.upper_right_diagonal_x) + diag_width);
            }
        }
        1 => {
            result = ay.min(by) >= pad.top_y + w
                || d((ax - ay).max(bx - by)) <= d(pad.upper_left_diagonal_x) - diag_width
                || ax.max(bx) <= pad.left_x - w;
            if with_sides && !result {
                result = (ax.min(bx) <= pad.left_x - w && d((ax + ay).max(bx + by)) <= d(pad.lower_left_diagonal_x) - diag_width)
                    || (ay.max(by) >= pad.top_y + w && d((ax + ay).min(bx + by)) >= d(pad.upper_right_diagonal_x) + diag_width);
            }
        }
        2 => {
            result = ax.max(bx) <= pad.left_x - w
                || d((ax - ay).max(bx - by)) <= d(pad.upper_left_diagonal_x) - diag_width
                || d((ax + ay).max(bx + by)) <= d(pad.lower_left_diagonal_x) - diag_width;
            if with_sides && !result {
                result = (ay.max(by) <= pad.bottom_y - w && d((ax + ay).min(bx + by)) <= d(pad.lower_left_diagonal_x) - diag_width)
                    || (ay.min(by) >= pad.top_y + w && d((ax - ay).min(bx - by)) <= d(pad.upper_left_diagonal_x) - diag_width);
            }
        }
        3 => {
            result = ax.max(bx) <= pad.left_x - w
                || ay.max(by) <= pad.bottom_y - w
                || d((ax + ay).max(bx + by)) <= d(pad.lower_left_diagonal_x) - diag_width;
            if with_sides && !result {
                result = (ay.min(by) <= pad.bottom_y - w && d((ax - ay).min(bx - by)) >= d(pad.lower_right_diagonal_x) + diag_width)
                    || (ax.min(bx) <= pad.left_x - w && d((ax - ay).max(bx - by)) <= d(pad.upper_left_diagonal_x) - diag_width);
            }
        }
        4 => {
            result = ay.max(by) <= pad.bottom_y - w
                || d((ax + ay).max(bx + by)) <= d(pad.lower_left_diagonal_x) - diag_width
                || d((ax - ay).min(bx - by)) >= d(pad.lower_right_diagonal_x) + diag_width;
            if with_sides && !result {
                result = (ax.min(bx) >= pad.right_x + w && d((ax - ay).max(bx - by)) >= d(pad.lower_right_diagonal_x) + diag_width)
                    || (ax.max(bx) <= pad.left_x - w && d((ax + ay).min(bx + by)) <= d(pad.lower_left_diagonal_x) - diag_width);
            }
        }
        5 => {
            result = ay.max(by) <= pad.bottom_y - w
                || ax.min(bx) >= pad.right_x + w
                || d((ax - ay).min(bx - by)) >= d(pad.lower_right_diagonal_x) + diag_width;
            if with_sides && !result {
                result = (ax.max(bx) >= pad.right_x + w && d((ax + ay).min(bx + by)) >= d(pad.upper_right_diagonal_x) + diag_width)
                    || (ay.min(by) <= pad.bottom_y - w && d((ax + ay).max(bx + by)) <= d(pad.lower_left_diagonal_x) - diag_width);
            }
        }
        6 => {
            result = ax.min(bx) >= pad.right_x + w
                || d((ax + ay).min(bx + by)) >= d(pad.upper_right_diagonal_x) + diag_width
                || d((ax - ay).min(bx - by)) >= d(pad.lower_right_diagonal_x) + diag_width;
            if with_sides && !result {
                result = (ay.max(by) <= pad.bottom_y - w && d((ax - ay).max(bx - by)) >= d(pad.lower_right_diagonal_x) + diag_width)
                    || (ay.min(by) >= pad.top_y + w && d((ax + ay).max(bx + by)) >= d(pad.upper_right_diagonal_x) + diag_width);
            }
        }
        7 => {
            result = ay.min(by) >= pad.top_y + w
                || d((ax + ay).min(bx + by)) >= d(pad.upper_right_diagonal_x) + diag_width
                || ax.min(bx) >= pad.right_x + w;
            if with_sides && !result {
                result = (ay.max(by) >= pad.top_y + w && d((ax - ay).max(bx - by)) <= d(pad.upper_left_diagonal_x) - diag_width)
                    || (ax.max(bx) >= pad.right_x + w && d((ax - ay).min(bx - by)) >= d(pad.lower_right_diagonal_x) + diag_width);
            }
        }
        _ => {
            log::warn!("ForcedPadAlgo.in_front_of_pad: fromSide out of range");
            result = true;
        }
    }
    result
}

/// Java `checkForcedPad(padShape, fromSide, layer, netNumbers, clearanceClassIndex,
/// copperSharingAllowed, ignoreItems, maxRecursionDepth, maxViaRecursionDepth, checkOnlyFront,
/// timeLimit)`: checks if the obstacle traces can be shoved aside so that the pad can be inserted.
#[allow(clippy::too_many_arguments)]
pub fn check_forced_pad(
    board: &mut RoutingBoard,
    pad_shape: &TileShape,
    from_side: Option<&ShapeEntrySide>,
    layer: LayerNo,
    net_numbers: &[NetNo],
    clearance_class: ClearanceClassNo,
    copper_sharing_allowed: bool,
    ignore_items: Option<&[ItemKey]>,
    max_recursion_depth: i32,
    max_via_recursion_depth: i32,
    check_only_front: bool,
    time_limit: Option<&TimeLimit>,
) -> CheckDrillResult {
    if !pad_shape.is_contained_in(&board.bounding_box()) {
        let outline = board.get_outline();
        board.set_shove_failing_obstacle(outline);
        return CheckDrillResult::NotDrillable;
    }
    let mut shape_entries = ShapeTraceEntries::new(pad_shape.clone(), layer, net_numbers, clearance_class, from_side.cloned());
    let mut obstacle_keys: Vec<ItemKey> = shove_obstacles(board, pad_shape, layer, clearance_class).iter().collect();
    if let Some(ignore) = ignore_items {
        obstacle_keys.retain(|k| !ignore.contains(k));
    }
    let obstacles_shovable = shape_entries.store_items(board, &obstacle_keys, true, copper_sharing_allowed);
    if !obstacles_shovable {
        board.set_shove_failing_obstacle(shape_entries.found_obstacle());
        return CheckDrillResult::NotDrillable;
    }
    // check, if the obstacle vias can be shoved
    for via in shape_entries.shove_via_list.clone() {
        if max_via_recursion_depth <= 0 {
            board.set_shove_failing_obstacle(Some(via));
            return CheckDrillResult::NotDrillable;
        }
        let new_via_center = drill_item_mover::try_shove_via_points(board, pad_shape, layer, via, clearance_class, false);
        if new_via_center.is_empty() {
            board.set_shove_failing_obstacle(Some(via));
            return CheckDrillResult::NotDrillable;
        }
        let delta = new_via_center[0].difference_by(&board.item(via).center(board));
        if !drill_item_mover::check(board, via, &delta, max_recursion_depth, max_via_recursion_depth - 1, Some(Vec::new()), time_limit) {
            return CheckDrillResult::NotDrillable;
        }
    }
    let mut result = CheckDrillResult::Drillable;
    if copper_sharing_allowed && obstacle_keys.iter().any(|k| board.item(*k).is_pin()) {
        result = CheckDrillResult::DrillableWithAttachSmd;
    }
    let trace_piece_count = shape_entries.substitute_trace_count();
    if trace_piece_count == 0 {
        return result;
    }
    if max_recursion_depth <= 0 {
        board.set_shove_failing_obstacle(shape_entries.found_obstacle());
        return CheckDrillResult::NotDrillable;
    }
    if shape_entries.stack_depth() > 1 {
        board.set_shove_failing_obstacle(shape_entries.found_obstacle());
        return CheckDrillResult::NotDrillable;
    }
    let is_orthogonal_mode = matches!(pad_shape, TileShape::IntBox(_));
    let from_side_no = from_side.map(|f| f.no);
    while let Some(substitute) = shape_entries.next_substitute_trace_piece(board) {
        let nets = substitute.net_numbers().to_vec();
        let cl = substitute.clearance_class();
        let st = substitute.trace();
        for i in 0..st.tile_shape_count() {
            let current_line = &st.polyline().lines[(i + 1) as usize];
            let current_direction: Direction = current_line.direction();
            let is_in_front =
                if check_only_front { in_front_of_pad(current_line, pad_shape, from_side_no.expect("checkForcedPad: fromSide is null"), st.half_width(), true) } else { true };
            if is_in_front {
                let current = shape_and_entry_side(board, &substitute, i, is_orthogonal_mode, true);
                if !TraceShover::check(
                    board,
                    &current.shape,
                    current.from_side.as_ref(),
                    Some(&current_direction),
                    layer,
                    &nets,
                    cl,
                    max_recursion_depth - 1,
                    max_via_recursion_depth,
                    0,
                    time_limit,
                ) {
                    return CheckDrillResult::NotDrillable;
                }
            }
        }
    }
    result
}

/// Java `forcedPad(padShape, fromSide, layer, netNumbers, clearanceClassIndex,
/// copperSharingAllowed, ignoreItems, maxRecursionDepth, maxViaRecursionDepth)`: shoves traces
/// aside so that the pad can be inserted. Returns false if the shove failed (the database may be
/// damaged, an undo is necessary).
#[allow(clippy::too_many_arguments)]
pub fn forced_pad(
    board: &mut RoutingBoard,
    pad_shape: &TileShape,
    from_side: Option<&ShapeEntrySide>,
    layer: LayerNo,
    net_numbers: &[NetNo],
    clearance_class: ClearanceClassNo,
    copper_sharing_allowed: bool,
    ignore_items: Option<&[ItemKey]>,
    max_recursion_depth: i32,
    max_via_recursion_depth: i32,
) -> bool {
    if pad_shape.is_empty() {
        log::warn!("ShoveTraceAux.forced_pad: padShape is empty");
        return true;
    }
    if !pad_shape.is_contained_in(&board.bounding_box()) {
        let outline = board.get_outline();
        board.set_shove_failing_obstacle(outline);
        return false;
    }
    if !drill_item_mover::shove_vias(
        board,
        pad_shape,
        from_side,
        layer,
        net_numbers,
        clearance_class,
        ignore_items,
        max_recursion_depth,
        max_via_recursion_depth,
        false,
    ) {
        return false;
    }
    let mut shape_entries = ShapeTraceEntries::new(pad_shape.clone(), layer, net_numbers, clearance_class, from_side.cloned());
    let mut obstacle_keys: Vec<ItemKey> = shove_obstacles(board, pad_shape, layer, clearance_class).iter().collect();
    if let Some(ignore) = ignore_items {
        obstacle_keys.retain(|k| !ignore.contains(k));
    }
    let obstacles_shovable =
        shape_entries.store_items(board, &obstacle_keys, true, copper_sharing_allowed) && shape_entries.shove_via_list.is_empty();
    if !obstacles_shovable {
        board.set_shove_failing_obstacle(shape_entries.found_obstacle());
        return false;
    }
    let trace_piece_count = shape_entries.substitute_trace_count();
    if trace_piece_count == 0 {
        return true;
    }
    if max_recursion_depth <= 0 {
        board.set_shove_failing_obstacle(shape_entries.found_obstacle());
        return false;
    }
    let tails_exist_before = board.contains_trace_tails(obstacle_keys.iter().copied(), net_numbers);
    shape_entries.cutout_traces(board, &obstacle_keys);
    let is_orthogonal_mode = matches!(pad_shape, TileShape::IntBox(_));
    while let Some(substitute) = shape_entries.next_substitute_trace_piece(board) {
        if substitute.first_corner() == substitute.last_corner() {
            continue;
        }
        let current_net_numbers = substitute.net_numbers().to_vec();
        let cl = substitute.clearance_class();
        for i in 0..substitute.trace().tile_shape_count() {
            let current = shape_and_entry_side(board, &substitute, i, is_orthogonal_mode, false);
            if !TraceShover::insert(
                board,
                &current.shape,
                current.from_side.as_ref(),
                layer,
                &current_net_numbers,
                cl,
                ignore_items,
                max_recursion_depth - 1,
                max_via_recursion_depth,
                0,
            ) {
                return false;
            }
        }
        let polyline = substitute.trace().polyline().clone();
        for i in 0..polyline.corner_count() {
            board.join_changed_area(&polyline.corner_approx(i), layer);
        }
        let end_corners = if !tails_exist_before { Some([substitute.first_corner(), substitute.last_corner()]) } else { None };
        let key = board.insert_item(substitute);
        let opt_area = board.changed_area.as_ref().map(|c| c.get_area(layer));
        board.normalize_trace(key, opt_area.as_ref());
        if let Some(end_corners) = end_corners {
            for corner in &end_corners {
                if let Some(tail) = board.get_trace_tail(corner, layer, &current_net_numbers) {
                    let items = board.get_connection_items(tail, StopConnectionOption::Via);
                    board.remove_items(items.iter().collect::<Vec<_>>());
                    for &n in &current_net_numbers {
                        board.combine_traces(n);
                    }
                }
            }
        }
    }
    true
}

/// Java `calcFromSide(shape, shapeCenter, layer, offset, clearanceClassIndex)`: a side of the
/// shape such that a trace line from the shape center to the side is free of obstacles.
pub fn calc_from_side(
    board: &RoutingBoard,
    shape: &TileShape,
    shape_center: &Point,
    layer: LayerNo,
    offset: i32,
    clearance_class: ClearanceClassNo,
) -> ShapeEntrySide {
    let offset_shape = shape.offset(offset as f64);
    for i in 0..offset_shape.border_line_count() {
        let check_shape = calc_check_shape_for_from_side(shape_center, &offset_shape.border_line(i));
        if board.check_trace_shape(&check_shape, layer, &[], clearance_class, None) {
            return ShapeEntrySide::new(i, None);
        }
    }
    // try second check without clearance
    for i in 0..offset_shape.border_line_count() {
        let check_shape = calc_check_shape_for_from_side(shape_center, &offset_shape.border_line(i));
        if board.check_trace_shape(&check_shape, layer, &[], 0, None) {
            return ShapeEntrySide::new(i, None);
        }
    }
    ShapeEntrySide::NOT_CALCULATED
}
