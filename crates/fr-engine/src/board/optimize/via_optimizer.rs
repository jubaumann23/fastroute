//! Port of `board/optimize/ViaOptimizer.java`: moving vias to better locations.

use fr_geom::{FloatLine, FloatPoint, IntPoint, Point, Side, Vector};
use fr_settings::ExpansionCostFactor;

use crate::ids::AngleRestriction;

use super::super::actions::drill_item_mover;
use super::super::item::ItemKey;
use super::super::routing_board::{jmin, RoutingBoard};
use super::super::selection_filter::{ItemSelectionFilter, SelectableChoices};

/// Java `isWithinTolerance(p1, p2, tolerance)`: Manhattan distance of the float points.
fn is_within_tolerance(p1: &Point, p2: &Point, tolerance: i32) -> bool {
    let fp1 = p1.to_float();
    let fp2 = p2.to_float();
    let dx = (fp1.x - fp2.x).abs();
    let dy = (fp1.y - fp2.y).abs();
    (dx + dy) <= tolerance as f64
}

/// Java `ViaOptimizer.optViaLocation(board, via, traceCosts, tracePullTightAccuracy,
/// maxRecursionDepth)`: optimizes the location of a via connected to at most 2 traces
/// according to the trace costs of their layers (`None`: horizontal and vertical costs 1).
/// Returns false if the via was not changed.
pub fn opt_via_location(
    board: &mut RoutingBoard,
    via: ItemKey,
    trace_costs: Option<&[ExpansionCostFactor]>,
    trace_pull_tight_accuracy: i32,
    max_recursion_depth: i32,
) -> bool {
    if board.item(via).is_shove_fixed_state() {
        return false;
    }
    if max_recursion_depth <= 0 {
        log::debug!("OptViaAlgo.opt_via_location: probably endless loop");
        return false;
    }
    let contacts: Vec<ItemKey> = board.normal_contacts(via).iter().collect();
    let mut is_plane_or_fanout_via = contacts.len() == 1;
    let mut first_trace: Option<ItemKey> = None;
    let mut second_trace: Option<ItemKey> = None;
    if !is_plane_or_fanout_via {
        if contacts.len() != 2 {
            return false;
        }
        for (i, &c) in contacts.iter().enumerate() {
            let item = board.item(c);
            if board.is_shove_fixed(c) || !item.is_trace() {
                if item.is_conduction_area() {
                    is_plane_or_fanout_via = true;
                } else {
                    return false;
                }
            } else if i == 0 {
                first_trace = Some(c);
            } else {
                second_trace = Some(c);
            }
        }
    }
    if is_plane_or_fanout_via {
        return opt_plane_or_fanout_via(board, via, trace_pull_tight_accuracy, max_recursion_depth);
    }
    let (first_trace, second_trace) = (first_trace.unwrap(), second_trace.unwrap());
    let via_center = board.item(via).center(board);
    let first_layer = board.item(first_trace).trace().layer();
    let second_layer = board.item(second_trace).trace().layer();
    // Use tolerance-based comparison to match connectivity detection logic
    let tolerance = (board.item(via).drill_min_width(board) / 2.0) as i32 + 1;
    let from_corner = |board: &RoutingBoard, t: ItemKey| -> Option<Point> {
        let item = board.item(t);
        let p = item.trace().polyline();
        if is_within_tolerance(&item.first_corner(), &via_center, tolerance) {
            Some(p.corner(1))
        } else if is_within_tolerance(&item.last_corner(), &via_center, tolerance) {
            Some(p.corner(p.corner_count() - 2))
        } else {
            // Via is not connected at trace endpoints - skip optimization
            None
        }
    };
    let Some(first_trace_from_corner) = from_corner(board, first_trace) else {
        return false;
    };
    let Some(second_trace_from_corner) = from_corner(board, second_trace) else {
        return false;
    };
    let (first_layer_trace_costs, second_layer_trace_costs) = match trace_costs {
        Some(costs) => (costs[first_layer as usize], costs[second_layer as usize]),
        None => {
            let c = ExpansionCostFactor { horizontal: 1.0, vertical: 1.0 };
            (c, c)
        }
    };
    let (ft, st) = (board.item(first_trace), board.item(second_trace));
    let first = TraceEnd {
        half_width: ft.trace().half_width(),
        cl: ft.clearance_class(),
        layer: ft.trace().layer(),
        costs: first_layer_trace_costs,
        from_corner: first_trace_from_corner,
    };
    let second = TraceEnd {
        half_width: st.trace().half_width(),
        cl: st.clearance_class(),
        layer: st.trace().layer(),
        costs: second_layer_trace_costs,
        from_corner: second_trace_from_corner,
    };
    let new_location = reposition_via_by_costs(board, via, &first, &second);
    let Some(new_location) = new_location else {
        return false;
    };
    if new_location == via_center {
        return false;
    }
    let delta = new_location.difference_by(&via_center);
    if !drill_item_mover::insert(board, via, &delta, 9, 9) {
        log::warn!("OptViaAlgo.opt_via_location: move via failed");
        return false;
    }
    let filter = ItemSelectionFilter::single(SelectableChoices::Traces);
    for layer in [first_layer, second_layer] {
        let picked = board.pick_items(&new_location, layer, Some(&filter));
        for t in picked.iter() {
            board.pull_tight_trace(t, true, trace_pull_tight_accuracy, None);
        }
    }
    let filter = ItemSelectionFilter::single(SelectableChoices::Vias);
    let picked = board.pick_items(&new_location, first_layer, Some(&filter));
    if let Some(v) = picked.first() {
        opt_via_location(board, v, trace_costs, trace_pull_tight_accuracy, max_recursion_depth - 1);
    }
    true
}

/// The data of a trace connected to the via used by the cost based repositioning.
struct TraceEnd {
    half_width: i32,
    cl: i32,
    layer: i32,
    costs: ExpansionCostFactor,
    from_corner: Point,
}

/// Java `optPlaneOrFanoutVia(board, via, tracePullTightAccuracy, maxRecursionDepth)`:
/// optimizations for vias with only 1 connected trace (plane or fanout vias).
fn opt_plane_or_fanout_via(board: &mut RoutingBoard, via: ItemKey, trace_pull_tight_accuracy: i32, max_recursion_depth: i32) -> bool {
    if max_recursion_depth <= 0 {
        log::debug!("OptViaAlgo.opt_plane_or_fanout_via: probably endless loop");
        return false;
    }
    let contact_list = board.normal_contacts(via);
    if contact_list.is_empty() {
        return false;
    }
    let mut contact_plane: Option<ItemKey> = None;
    let mut contact_trace: Option<ItemKey> = None;
    for c in contact_list.iter() {
        let item = board.item(c);
        if item.is_conduction_area() {
            if contact_plane.is_some() {
                return false;
            }
            contact_plane = Some(c);
        } else if item.is_trace() {
            if board.is_shove_fixed(c) || contact_trace.is_some() {
                return false;
            }
            contact_trace = Some(c);
        } else {
            return false;
        }
    }
    let Some(contact_trace) = contact_trace else {
        return false;
    };
    let via_center = board.item(via).center(board);
    // Use tolerance based on via size, matching the logic in opt_via_location
    let tolerance = (board.item(via).drill_min_width(board) / 2.0) as i32 + 1;
    let ct = board.item(contact_trace);
    let at_first_corner = if is_within_tolerance(&ct.first_corner(), &via_center, tolerance) {
        true
    } else if is_within_tolerance(&ct.last_corner(), &via_center, tolerance) {
        false
    } else {
        // Via is not connected at trace endpoints - skip optimization
        return false;
    };
    let trace_polyline = ct.trace().polyline().clone();
    let check_corner = if at_first_corner { trace_polyline.corner(1) } else { trace_polyline.corner(trace_polyline.corner_count() - 2) };
    let rounded_check_corner = check_corner.to_float().round();
    let trace_half_width = ct.trace().half_width();
    let trace_layer = ct.trace().layer();
    let trace_cl_class = ct.clearance_class();
    let mut new_via_location = reposition_via_to(board, via, &rounded_check_corner, trace_half_width, trace_layer, trace_cl_class);
    if new_via_location.is_none() && trace_polyline.corner_count() >= 3 {
        // try to project the via to the previous line
        let prev_corner = if at_first_corner { trace_polyline.corner(2) } else { trace_polyline.corner(trace_polyline.corner_count() - 3) };
        let float_check_corner = check_corner.to_float();
        let float_via_center = via_center.to_float();
        let float_prev_corner = prev_corner.to_float();
        if float_check_corner.scalar_product(&float_via_center, &float_prev_corner) != 0.0 {
            let current_line = FloatLine::new(float_check_corner, float_prev_corner);
            let projection = Point::Int(current_line.perpendicular_projection(&float_via_center).round());
            let diff_vector = projection.difference_by(&via_center);
            let angle_restriction = board.rules.get_trace_angle_restriction();
            let projection_ok = !(projection == via_center
                || (angle_restriction == AngleRestriction::NinetyDegree && !diff_vector.is_orthogonal())
                || (angle_restriction == AngleRestriction::FortyfiveDegree && !diff_vector.is_multiple_of_45_degree()));
            if projection_ok && drill_item_mover::check(board, via, &diff_vector, 0, 0, None, None) {
                let nets = board.item(via).net_numbers().to_vec();
                let ok_length = board.check_trace_segment(&via_center, &projection, trace_layer, &nets, trace_half_width, trace_cl_class, false);
                if ok_length >= i32::MAX as f64 {
                    new_via_location = Some(projection);
                }
            }
        }
    }
    let Some(new_via_location) = new_via_location else {
        return false;
    };
    if let Some(plane) = contact_plane {
        // check, that the new location is inside the contact plane
        let filter = ItemSelectionFilter::single(SelectableChoices::Conduction);
        let plane_layer = board.item(plane).first_layer(board);
        let picked = board.pick_items(&new_via_location, plane_layer, Some(&filter));
        if !picked.iter().any(|k| k == plane) {
            return false;
        }
    }
    let diff_vector = new_via_location.difference_by(&via_center);
    if !drill_item_mover::insert(board, via, &diff_vector, 9, 9) {
        log::warn!("OptViaAlgo.opt_plane_or_fanout_via: move via failed");
        return false;
    }
    let filter = ItemSelectionFilter::single(SelectableChoices::Traces);
    let picked = board.pick_items(&new_via_location, trace_layer, Some(&filter));
    for t in picked.iter() {
        board.pull_tight_trace(t, true, trace_pull_tight_accuracy, None);
    }
    if new_via_location == check_corner {
        opt_plane_or_fanout_via(board, via, trace_pull_tight_accuracy, max_recursion_depth - 1);
    }
    true
}

/// Java `repositionVia(board, via, toLocation, traceHalfWidth, traceLayer, traceClClass)`: moves
/// the via towards `to_location` as far as possible. The new location, or `None` if no move is
/// possible.
fn reposition_via_to(board: &mut RoutingBoard, via: ItemKey, to_location: &IntPoint, trace_half_width: i32, trace_layer: i32, trace_cl_class: i32) -> Option<Point> {
    let from_location = board.item(via).center(board);
    let to_point = Point::Int(*to_location);
    if from_location == to_point {
        return None;
    }
    let nets = board.item(via).net_numbers().to_vec();
    let mut ok_length = board.check_trace_segment(&from_location, &to_point, trace_layer, &nets, trace_half_width, trace_cl_class, false);
    if ok_length <= 0.0 {
        return None;
    }
    let float_from_location = from_location.to_float();
    let float_to_location = to_point.to_float();
    let new_float_to_location =
        if ok_length >= i32::MAX as f64 { float_to_location } else { float_from_location.change_length(&float_to_location, ok_length) };
    let new_to_location = Point::Int(new_float_to_location.round());
    let delta = new_to_location.difference_by(&from_location);
    if drill_item_mover::check(board, via, &delta, 0, 0, None, None) {
        return Some(new_to_location);
    }
    let min_length = 0.3 * trace_half_width as f64 + 1.0;
    ok_length = jmin(ok_length, float_from_location.distance(&float_to_location));
    let mut current_length = ok_length / 2.0;
    ok_length = 0.0;
    let mut result: Option<Point> = None;
    while current_length >= min_length {
        let check_point = Point::Int(float_from_location.change_length(&float_to_location, ok_length + current_length).round());
        let delta = check_point.difference_by(&from_location);
        if drill_item_mover::check(board, via, &delta, 0, 0, None, None) {
            ok_length += current_length;
            result = Some(check_point);
        }
        current_length /= 2.0;
    }
    result
}

/// Java `repositionVia(board, via, toLocation, traceHalfWidth1, traceLayer1, traceClClass1,
/// connectLocation, traceHalfWidth2, traceLayer2, traceClClass2)`: checks if the via can be moved
/// to `to_location` with free connections from the old location and to `connect_location`.
#[allow(clippy::too_many_arguments)]
fn reposition_via_connect(
    board: &mut RoutingBoard,
    via: ItemKey,
    to_location: &IntPoint,
    trace_half_width1: i32,
    trace_layer1: i32,
    trace_cl_class1: i32,
    connect_location: &IntPoint,
    trace_half_width2: i32,
    trace_layer2: i32,
    trace_cl_class2: i32,
) -> bool {
    let from_location = board.item(via).center(board);
    let to_point = Point::Int(*to_location);
    if from_location == to_point {
        log::trace!("OptViaAlgo.reposition_via: fromLocation equal toLocation");
        return false;
    }
    let delta = to_point.difference_by(&from_location);
    if board.rules.get_trace_angle_restriction() == AngleRestriction::None && delta.length_approx() <= 1.5 {
        // reduce_corners of the any angle tightener may not be able to remove the new generated
        // overlap because of numerical stability problems (endless loop).
        return false;
    }
    let nets = board.item(via).net_numbers().to_vec();
    let ok_length = board.check_trace_segment(&from_location, &to_point, trace_layer1, &nets, trace_half_width1, trace_cl_class1, false);
    if ok_length < i32::MAX as f64 {
        return false;
    }
    let ok_length =
        board.check_trace_segment(&to_point, &Point::Int(*connect_location), trace_layer2, &nets, trace_half_width2, trace_cl_class2, false);
    if ok_length < i32::MAX as f64 {
        return false;
    }
    drill_item_mover::check(board, via, &delta, 0, 0, None, None)
}

/// Java `repositionVia(board, via, firstTraceHalfWidth, ..., secondTraceFromCorner)`: tries to
/// move the via to a better location according to the trace costs. `None` if no better location
/// was found.
fn reposition_via_by_costs(board: &mut RoutingBoard, via: ItemKey, first: &TraceEnd, second: &TraceEnd) -> Option<Point> {
    let via_location = board.item(via).center(board);
    let first_delta: Vector = first.from_corner.difference_by(&via_location);
    let second_delta: Vector = second.from_corner.difference_by(&via_location);
    let scalar_product = first_delta.scalar_product(&second_delta);
    let float_via_location = via_location.to_float();
    let float_first = first.from_corner.to_float();
    let float_second = second.from_corner.to_float();
    let first_dist = float_via_location.distance(&float_first);
    let second_dist = float_via_location.distance(&float_second);
    let rounded_first = float_first.round();
    let rounded_second = float_second.round();
    // handle case of overlapping lines first
    if via_location.side_of(&first.from_corner, &second.from_corner) == Side::Collinear && scalar_product > 0.0 {
        if second_dist < first_dist {
            return reposition_via_to(board, via, &rounded_second, first.half_width, first.layer, first.cl);
        }
        return reposition_via_to(board, via, &rounded_first, second.half_width, second.layer, second.cl);
    }
    let wd = |p: &FloatPoint, q: &FloatPoint, c: &ExpansionCostFactor| p.weighted_distance(q, c.horizontal, c.vertical);
    let mut current_weighted_distance1 = wd(&float_via_location, &float_first, &first.costs);
    let mut current_weighted_distance2 = wd(&float_via_location, &float_first, &second.costs);
    if current_weighted_distance1 > current_weighted_distance2 {
        // try to move the via in direction of the first trace from corner
        if let Some(r) = reposition_via_to(board, via, &rounded_first, second.half_width, second.layer, second.cl) {
            return Some(r);
        }
    }
    current_weighted_distance1 = wd(&float_via_location, &float_second, &second.costs);
    current_weighted_distance2 = wd(&float_via_location, &float_second, &first.costs);
    if current_weighted_distance1 > current_weighted_distance2 {
        // try to move the via in direction of the second trace from corner
        if let Some(r) = reposition_via_to(board, via, &rounded_second, first.half_width, first.layer, first.cl) {
            return Some(r);
        }
    }
    if scalar_product > 0.0 && board.rules.get_trace_angle_restriction() != AngleRestriction::NinetyDegree {
        // acute angle
        let (to_point1, float_to_point1, to_point2, float_to_point2) = if first_dist < second_dist {
            let float_to_point2 = float_via_location.change_length(&float_second, first_dist);
            (rounded_first, float_first, float_to_point2.round(), float_to_point2)
        } else {
            let float_to_point1 = float_via_location.change_length(&float_first, second_dist);
            (float_to_point1.round(), float_to_point1, rounded_second, float_second)
        };
        current_weighted_distance1 = wd(&float_to_point1, &float_to_point2, &first.costs);
        current_weighted_distance2 = wd(&float_to_point1, &float_to_point2, &second.costs);
        let result = if current_weighted_distance1 > current_weighted_distance2 {
            // try moving the via first into the direction of to_point1
            reposition_via_to(board, via, &to_point1, second.half_width, second.layer, second.cl)
                .or_else(|| reposition_via_to(board, via, &to_point2, first.half_width, first.layer, first.cl))
        } else {
            // try moving the via first into the direction of to_point2
            reposition_via_to(board, via, &to_point2, first.half_width, first.layer, first.cl)
                .or_else(|| reposition_via_to(board, via, &to_point1, second.half_width, second.layer, second.cl))
        };
        if result.is_some() {
            return result;
        }
    }
    // try decomposition in axis parallel parts
    if !first_delta.is_orthogonal() {
        let current_weighted_distance1 = wd(&float_via_location, &float_first, &first.costs);
        for check in [FloatPoint::new(float_via_location.x, float_first.y), FloatPoint::new(float_first.x, float_via_location.y)] {
            let d2 = wd(&float_via_location, &check, &second.costs);
            let d3 = wd(&check, &float_first, &first.costs);
            if current_weighted_distance1 > d2 + d3 {
                let check_location = check.round();
                if reposition_via_connect(
                    board,
                    via,
                    &check_location,
                    second.half_width,
                    second.layer,
                    second.cl,
                    &rounded_first,
                    first.half_width,
                    first.layer,
                    first.cl,
                ) {
                    return Some(Point::Int(check_location));
                }
            }
        }
    }
    if !second_delta.is_orthogonal() {
        let current_weighted_distance1 = wd(&float_via_location, &float_second, &second.costs);
        for check in [FloatPoint::new(float_via_location.x, float_second.y), FloatPoint::new(float_second.x, float_via_location.y)] {
            let d2 = wd(&float_via_location, &check, &first.costs);
            let d3 = wd(&check, &float_second, &second.costs);
            if current_weighted_distance1 > d2 + d3 {
                let check_location = check.round();
                if reposition_via_connect(
                    board,
                    via,
                    &check_location,
                    first.half_width,
                    first.layer,
                    first.cl,
                    &rounded_second,
                    second.half_width,
                    second.layer,
                    second.cl,
                ) {
                    return Some(Point::Int(check_location));
                }
            }
        }
    }
    None
}
