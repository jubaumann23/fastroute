//! Port of `board/optimize/TraceShover.java`: checking and inserting trace segments while
//! shoving obstacle traces and vias aside.
//!
//! Substitute traces (Java `PolylineTrace` objects created by
//! `ShapeTraceEntries.nextSubstituteTracePiece`, not yet on the board) are [`Item`] values here;
//! their tree shapes are calculated on demand like Java does for items outside the trees.


use fr_geom::{ConvexShape, Direction, IntBox, LineSegment, TileShape};

use crate::datastructures::TimeLimit;
use crate::ids::{AngleRestriction, ClearanceClassNo, LayerNo, NetNo};
use crate::structure::{ShapeAndEntrySide, ShapeEntrySide};

use super::super::actions::drill_item_mover;
use super::super::item::{Item, ItemKey, ItemKind, StopConnectionOption};
use super::super::item_list::ItemSet;
use super::super::routing_board::RoutingBoard;
use super::super::search_tree::{TreeObject, DEFAULT_TREE};
use super::super::shape_trace_entries::ShapeTraceEntries;
use super::tracked::{BorderLines, TLine, TPolyline};

/// Java `TraceShover` (stateless; the functions take the board).
pub struct TraceShover;

/// Java `trace.getCompensatedHalfWidth(defaultTree)`.
pub(crate) fn compensated_half_width(board: &RoutingBoard, trace: &Item) -> i32 {
    let t = trace.trace();
    t.half_width() + board.clearance_compensation_value(DEFAULT_TREE, trace.clearance_class(), t.layer())
}

/// Java `new ShapeAndEntrySide(substituteTrace, index, orthogonal, inShoveCheck)`.
pub(crate) fn shape_and_entry_side(board: &RoutingBoard, trace: &Item, index: i32, orthogonal: bool, in_shove_check: bool) -> ShapeAndEntrySide {
    // only shape `index` of `substitute_tree_shapes` (the tree shapes of a trace are computed
    // independently of each other)
    let tree = board.default_tree();
    let t = trace.trace();
    assert!(index >= 0 && index < t.tile_shape_count(), "ShapeAndEntrySide: shape index out of range");
    let offset_width = t.half_width + tree.clearance_compensation_value(&board.rules, trace.clearance_class, t.layer);
    let tree_shape = tree.offset_shape(&t.polyline, offset_width, index).expect("ShapeAndEntrySide: tree shape is null");
    ShapeAndEntrySide::new(&tree_shape, trace.trace().polyline(), compensated_half_width(board, trace), index, orthogonal, in_shove_check)
}

/// Java `changePolyline` of a substitute trace (`PolylineTrace.change` of a trace not on the
/// board only replaces the polyline).
fn set_polyline(trace: &mut Item, polyline: TPolyline) {
    if let ItemKind::Trace(t) = &mut trace.kind {
        t.set_tpolyline(polyline);
    }
}

/// The obstacles of a shove shape: `overlappingItemsWithClearance(shape, layer, new int[0],
/// clearanceClassIndex)` of the default tree.
pub(crate) fn shove_obstacles(board: &RoutingBoard, shape: &TileShape, layer: LayerNo, clearance_class: ClearanceClassNo) -> ItemSet {
    board.overlapping_items_with_clearance_in(DEFAULT_TREE, &ConvexShape::Tile(shape.clone()), layer, &[], clearance_class)
}

impl TraceShover {
    /// Java static `TraceShover.check(board, lineSegment, shoveToTheLeft, layer, netNumbers,
    /// traceHalfWidth, clearanceClassIndex, maxRecursionDepth, maxViaRecursionDepth)`: the
    /// maximal length of a trace from the start of the segment for which the shove succeeds
    /// (`i32::MAX` if it succeeds completely).
    #[allow(clippy::too_many_arguments)]
    pub fn check_line_segment(
        board: &mut RoutingBoard,
        line_segment: &LineSegment,
        shove_to_the_left: bool,
        layer: LayerNo,
        net_numbers: &[NetNo],
        trace_half_width: i32,
        clearance_class: ClearanceClassNo,
        max_recursion_depth: i32,
        max_via_recursion_depth: i32,
    ) -> f64 {
        let mut trace_half_width = trace_half_width;
        let compensated = board.default_tree().is_clearance_compensation_used();
        if compensated {
            trace_half_width += board.clearance_compensation_value(DEFAULT_TREE, clearance_class, layer);
        }
        let trace_shapes = line_segment.to_polyline().offset_shapes(trace_half_width);
        if trace_shapes.len() != 1 {
            log::warn!("TraceShover.check: traceShape count 1 expected");
            return 0.0;
        }
        let trace_shape = trace_shapes[0].clone();
        if trace_shape.is_empty() {
            log::warn!("TraceShover.check: traceShape is empty");
            return 0.0;
        }
        if !trace_shape.is_contained_in(&board.bounding_box()) {
            return 0.0;
        }
        let from_side = ShapeEntrySide::from_line_segment(line_segment, &trace_shape, shove_to_the_left);
        let mut shape_entries = ShapeTraceEntries::new(trace_shape.clone(), layer, net_numbers, clearance_class, Some(from_side));
        let obstacles: Vec<ItemKey> = shove_obstacles(board, &trace_shape, layer, clearance_class).iter().collect();
        let obstacles_shovable = shape_entries.store_items(board, &obstacles, false, true);
        if !obstacles_shovable || shape_entries.trace_tails_in_shape() {
            return 0.0;
        }
        let trace_piece_count = shape_entries.substitute_trace_count();
        if shape_entries.stack_depth() > 1 {
            return 0.0;
        }
        let start_corner = line_segment.start_point_approx();
        let end_corner = line_segment.end_point_approx();
        let segment_length = end_corner.distance(&start_corner);
        let mut result = i32::MAX as f64;
        // check, if the obstacle vias can be shoved
        for via in shape_entries.shove_via_list.clone() {
            if board.item(via).shares_net_no(net_numbers) {
                continue;
            }
            let mut shove_via_ok = false;
            if max_via_recursion_depth > 0 {
                let new_via_center = drill_item_mover::try_shove_via_points(board, &trace_shape, layer, via, clearance_class, false);
                if new_via_center.is_empty() {
                    return 0.0;
                }
                let delta = new_via_center[0].difference_by(&board.item(via).center(board));
                shove_via_ok =
                    drill_item_mover::check(board, via, &delta, max_recursion_depth, max_via_recursion_depth - 1, Some(Vec::new()), None);
            }
            if !shove_via_ok {
                let via_center = board.item(via).center(board).to_float();
                let mut projection = start_corner.scalar_product(&end_corner, &via_center);
                projection /= segment_length;
                let via_box = board.tree_shape_on_layer(DEFAULT_TREE, via, layer).expect("via tree shape").bounding_box();
                let via_radius = 0.5 * via_box.max_width();
                let mut current_ok_length = projection - via_radius - trace_half_width as f64;
                if !compensated {
                    current_ok_length -=
                        board.rules.clearance_matrix.get_value(clearance_class, board.item(via).clearance_class(), layer, true) as f64;
                }
                if current_ok_length <= 0.0 {
                    return 0.0;
                }
                result = super::super::routing_board::jmin(result, current_ok_length);
            }
        }
        if trace_piece_count == 0 {
            return result;
        }
        if max_recursion_depth <= 0 {
            return 0.0;
        }
        let line_direction = line_segment.get_line().direction();
        while let Some(substitute) = shape_entries.next_substitute_trace_piece(board) {
            let st = substitute.trace();
            for i in 0..st.tile_shape_count() {
                let mut current_line_segment = LineSegment::from_polyline(st.polyline(), i + 1).expect("line segment");
                if shove_to_the_left {
                    // swap the line segment to get the correct shove length in case it is
                    // smaller than the length of the whole line segment.
                    current_line_segment = current_line_segment.opposite();
                }
                let is_in_front = current_line_segment.get_line().direction() == line_direction;
                if is_in_front {
                    let shove_ok_length = Self::check_line_segment(
                        board,
                        &current_line_segment,
                        shove_to_the_left,
                        layer,
                        substitute.net_numbers(),
                        st.half_width(),
                        substitute.clearance_class(),
                        max_recursion_depth - 1,
                        max_via_recursion_depth,
                    );
                    if shove_ok_length < i32::MAX as f64 {
                        if shove_ok_length <= 0.0 {
                            return 0.0;
                        }
                        let p1 = start_corner.scalar_product(&end_corner, &current_line_segment.start_point_approx());
                        let p2 = start_corner.scalar_product(&end_corner, &current_line_segment.end_point_approx());
                        let mut projection = super::super::routing_board::jmin(p1, p2);
                        projection /= segment_length;
                        let mut current_ok_length = shove_ok_length + projection - trace_half_width as f64 - st.half_width() as f64;
                        if compensated {
                            current_ok_length -= board.clearance_compensation_value(DEFAULT_TREE, substitute.clearance_class(), layer) as f64;
                        } else {
                            current_ok_length -= board.rules.clearance_matrix.get_value(clearance_class, substitute.clearance_class(), layer, true)
                                as f64;
                        }
                        if current_ok_length <= 0.0 {
                            return 0.0;
                        }
                        result = super::super::routing_board::jmin(current_ok_length, result);
                    }
                    break;
                }
            }
        }
        result
    }

    /// Java `check(traceShape, fromSide, dir, layer, netNumbers, clearanceClassIndex,
    /// maxRecursionDepth, maxViaRecursionDepth, maxSpringOverRecursionDepth, timeLimit)`: checks
    /// if the shove is possible without clearance violations (`dir` prevents bouncing back).
    #[allow(clippy::too_many_arguments)]
    pub fn check(
        board: &mut RoutingBoard,
        trace_shape: &TileShape,
        from_side: Option<&ShapeEntrySide>,
        dir: Option<&Direction>,
        layer: LayerNo,
        net_numbers: &[NetNo],
        clearance_class: ClearanceClassNo,
        max_recursion_depth: i32,
        max_via_recursion_depth: i32,
        max_spring_over_recursion_depth: i32,
        time_limit: Option<&TimeLimit>,
    ) -> bool {
        if let Some(t) = time_limit {
            if t.limit_exceeded() {
                return false;
            }
        }
        if trace_shape.is_empty() {
            log::warn!("ShoveTraceAux.check: traceShape is empty");
            return true;
        }
        if !trace_shape.is_contained_in(&board.bounding_box()) {
            let outline = board.get_outline();
            board.set_shove_failing_obstacle(outline);
            return false;
        }
        let mut shape_entries = ShapeTraceEntries::new(trace_shape.clone(), layer, net_numbers, clearance_class, from_side.cloned());
        let mut obstacle_set = shove_obstacles(board, trace_shape, layer, clearance_class);
        let ignore = Self::ignore_items_at_tie_pins(board, trace_shape, layer, net_numbers);
        obstacle_set.remove_all(&ignore);
        let obstacles: Vec<ItemKey> = obstacle_set.iter().collect();
        let obstacles_shovable = shape_entries.store_items(board, &obstacles, false, true);
        if !obstacles_shovable {
            board.set_shove_failing_obstacle(shape_entries.found_obstacle());
            return false;
        }
        let trace_piece_count = shape_entries.substitute_trace_count();
        if shape_entries.stack_depth() > 1 {
            board.set_shove_failing_obstacle(shape_entries.found_obstacle());
            return false;
        }
        let shape_radius = 0.5 * trace_shape.bounding_box().min_width();
        // check, if the obstacle vias can be shoved
        for via in shape_entries.shove_via_list.clone() {
            if board.item(via).shares_net_no(net_numbers) {
                continue;
            }
            if max_via_recursion_depth <= 0 {
                board.set_shove_failing_obstacle(Some(via));
                return false;
            }
            let via_center = board.item(via).center(board);
            let current_shove_via_center = via_center.to_float();
            let try_via_centers = drill_item_mover::try_shove_via_points(board, trace_shape, layer, via, clearance_class, true);
            let max_dist =
                0.5 * board.item(via).drill_shape_on_layer(board, layer).expect("via shape").bounding_box().max_width() + shape_radius;
            let max_dist_square = max_dist * max_dist;
            let mut shove_via_ok = false;
            for (i, c) in try_via_centers.iter().enumerate() {
                if i == 0 || current_shove_via_center.distance_square(&c.to_float()) <= max_dist_square {
                    let delta = c.difference_by(&via_center);
                    if drill_item_mover::check(board, via, &delta, max_recursion_depth, max_via_recursion_depth - 1, Some(Vec::new()), time_limit) {
                        shove_via_ok = true;
                        break;
                    }
                }
            }
            if !shove_via_ok {
                return false;
            }
        }
        if trace_piece_count == 0 {
            return true;
        }
        if max_recursion_depth <= 0 {
            board.set_shove_failing_obstacle(shape_entries.found_obstacle());
            return false;
        }
        let is_orthogonal_mode = matches!(trace_shape, TileShape::IntBox(_));
        let mut max_spring_over_recursion_depth = max_spring_over_recursion_depth;
        while let Some(mut substitute) = shape_entries.next_substitute_trace_piece(board) {
            if max_spring_over_recursion_depth > 0 {
                let polyline = substitute.trace().tpolyline();
                let hw = compensated_half_width(board, &substitute);
                let Some(new_polyline) = Self::spring_over(
                    board,
                    &polyline,
                    hw,
                    layer,
                    substitute.net_numbers(),
                    substitute.clearance_class(),
                    false,
                    max_spring_over_recursion_depth,
                    None,
                ) else {
                    // spring_over did not work
                    return false;
                };
                if !new_polyline.same(&polyline) {
                    // spring_over changed something
                    max_spring_over_recursion_depth -= 1;
                    set_polyline(&mut substitute, new_polyline);
                }
            }
            let nets = substitute.net_numbers().to_vec();
            let cl = substitute.clearance_class();
            for i in 0..substitute.trace().tile_shape_count() {
                let current_direction = substitute.trace().polyline().lines[(i + 1) as usize].direction();
                let is_in_front = dir.map(|d| *d == current_direction).unwrap_or(true);
                if is_in_front {
                    let current = shape_and_entry_side(board, &substitute, i, is_orthogonal_mode, true);
                    if !Self::check(
                        board,
                        &current.shape,
                        current.from_side.as_ref(),
                        Some(&current_direction),
                        layer,
                        &nets,
                        cl,
                        max_recursion_depth - 1,
                        max_via_recursion_depth,
                        max_spring_over_recursion_depth,
                        time_limit,
                    ) {
                        return false;
                    }
                }
            }
        }
        true
    }

    /// Java `insert(traceShape, fromSide, layer, netNumbers, clearanceClassIndex, ignoreItems,
    /// maxRecursionDepth, maxViaRecursionDepth, maxSpringOverRecursionDepth)`: puts in a trace
    /// segment shoving obstacles out of the way. If the shove does not work the database may be
    /// damaged (call [`Self::check`] first).
    #[allow(clippy::too_many_arguments)]
    pub fn insert(
        board: &mut RoutingBoard,
        trace_shape: &TileShape,
        from_side: Option<&ShapeEntrySide>,
        layer: LayerNo,
        net_numbers: &[NetNo],
        clearance_class: ClearanceClassNo,
        ignore_items: Option<&[ItemKey]>,
        max_recursion_depth: i32,
        max_via_recursion_depth: i32,
        max_spring_over_recursion_depth: i32,
    ) -> bool {
        if trace_shape.is_empty() {
            log::warn!("ShoveTraceAux.insert: traceShape is empty");
            return true;
        }
        if !trace_shape.is_contained_in(&board.bounding_box()) {
            let outline = board.get_outline();
            board.set_shove_failing_obstacle(outline);
            return false;
        }
        if !drill_item_mover::shove_vias(
            board,
            trace_shape,
            from_side,
            layer,
            net_numbers,
            clearance_class,
            ignore_items,
            max_recursion_depth,
            max_via_recursion_depth,
            true,
        ) {
            return false;
        }
        let mut shape_entries = ShapeTraceEntries::new(trace_shape.clone(), layer, net_numbers, clearance_class, from_side.cloned());
        let mut obstacle_set = shove_obstacles(board, trace_shape, layer, clearance_class);
        let ignore = Self::ignore_items_at_tie_pins(board, trace_shape, layer, net_numbers);
        obstacle_set.remove_all(&ignore);
        let obstacles: Vec<ItemKey> = obstacle_set.iter().collect();
        let obstacles_shovable = shape_entries.store_items(board, &obstacles, false, true);
        if let Some(&first) = shape_entries.shove_via_list.first() {
            board.set_shove_failing_obstacle(Some(first));
            return false;
        }
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
        let tails_exist_before = board.contains_trace_tails(obstacles.iter().copied(), net_numbers);
        shape_entries.cutout_traces(board, &obstacles);
        let is_orthogonal_mode = matches!(trace_shape, TileShape::IntBox(_));
        let mut max_spring_over_recursion_depth = max_spring_over_recursion_depth;
        while let Some(mut substitute) = shape_entries.next_substitute_trace_piece(board) {
            if substitute.first_corner() == substitute.last_corner() {
                continue;
            }
            if max_spring_over_recursion_depth > 0 {
                let polyline = substitute.trace().tpolyline();
                let hw = compensated_half_width(board, &substitute);
                let Some(new_polyline) = Self::spring_over(
                    board,
                    &polyline,
                    hw,
                    layer,
                    substitute.net_numbers(),
                    substitute.clearance_class(),
                    false,
                    max_spring_over_recursion_depth,
                    None,
                ) else {
                    // spring_over did not work
                    return false;
                };
                if !new_polyline.same(&polyline) {
                    // spring_over changed something
                    max_spring_over_recursion_depth -= 1;
                    set_polyline(&mut substitute, new_polyline);
                }
            }
            let current_net_numbers = substitute.net_numbers().to_vec();
            let cl = substitute.clearance_class();
            for i in 0..substitute.trace().tile_shape_count() {
                let current = shape_and_entry_side(board, &substitute, i, is_orthogonal_mode, false);
                if !Self::insert(
                    board,
                    &current.shape,
                    current.from_side.as_ref(),
                    layer,
                    &current_net_numbers,
                    cl,
                    ignore_items,
                    max_recursion_depth - 1,
                    max_via_recursion_depth,
                    max_spring_over_recursion_depth,
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
            // Java: `normalize(board.changedArea.getArea(layer))` in a try block (a missing
            // changed area throws and is logged)
            match board.changed_area.as_ref().map(|c| c.get_area(layer)) {
                Some(area) => {
                    board.normalize_trace(key, Some(&area));
                }
                None => log::error!("Couldn't normalize trace."),
            }
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

    /// Java `getIgnoreItemsAtTiePins(traceShape, layer, netNumbers)`.
    pub(crate) fn ignore_items_at_tie_pins(board: &RoutingBoard, trace_shape: &TileShape, layer: LayerNo, net_numbers: &[NetNo]) -> ItemSet {
        let overlaps = board.overlapping_objects(&ConvexShape::Tile(trace_shape.clone()), layer);
        let mut result = ItemSet::new();
        for o in overlaps {
            let TreeObject::Item { key, .. } = o else { continue };
            let item = board.item(key);
            if item.is_pin() && item.shares_net_no(net_numbers) {
                result.extend_from(&board.all_contacts(key, Some(layer)));
            }
        }
        result
    }

    /// Java private `springOver(polyline, halfWidth, layer, netNumbers, clearanceClassIndex,
    /// overConnectedPins, recursionDepth, contactPins)`: wraps the polyline around the obstacles in
    /// its way (counter clockwise). `None` if that is not possible; `polyline` itself (same line
    /// array) if there are no obstacles.
    #[allow(clippy::too_many_arguments)]
    fn spring_over(
        board: &mut RoutingBoard,
        tpolyline: &TPolyline,
        half_width: i32,
        layer: LayerNo,
        net_numbers: &[NetNo],
        clearance_class: ClearanceClassNo,
        over_connected_pins: bool,
        recursion_depth: i32,
        contact_pins: Option<&ItemSet>,
    ) -> Option<TPolyline> {
        let polyline = &tpolyline.polyline;
        let mut found_obstacle: Option<ItemKey> = None;
        let mut found_obstacle_bounding_box = IntBox::EMPTY;
        let check_net_no_arr: Vec<NetNo> = if contact_pins.is_none() { net_numbers.to_vec() } else { Vec::new() };
        for i in 0..polyline.lines.len() as i32 - 2 {
            let current_shape = polyline.offset_shape(half_width, i).expect("springOver: offset shape");
            let obstacles = board.overlapping_items_with_clearance_in(
                DEFAULT_TREE,
                &ConvexShape::Tile(current_shape),
                layer,
                &check_net_no_arr,
                clearance_class,
            );
            for current_key in obstacles.iter() {
                let current_item = board.item(current_key);
                let is_obstacle = if current_item.shares_net_no(net_numbers) {
                    // to avoid acid traps
                    current_item.is_pin() && contact_pins.map(|p| !p.contains(current_item.id())).unwrap_or(false)
                } else if let Some(c) = current_item.as_conduction_area() {
                    c.is_obstacle()
                } else if current_item.is_board_outline() {
                    board.outline_blocks_nets(net_numbers)
                } else if current_item.is_via_obstacle_area() || current_item.is_component_obstacle_area() {
                    false
                } else if current_item.is_trace() {
                    if board.is_shove_fixed(current_key) {
                        // check for a shove fixed trace exit stub, which has to be ignored at a
                        // tie pin.
                        let mut obstacle = true;
                        for c in board.normal_contacts(current_key).iter() {
                            if board.item(c).shares_net_no(net_numbers) {
                                obstacle = false;
                            }
                        }
                        obstacle
                    } else {
                        // an unfixed trace can be pushed aside eventually
                        false
                    }
                } else {
                    // an unfixed via can be pushed aside eventually
                    !current_item.is_routable()
                };
                if is_obstacle {
                    match found_obstacle {
                        None => {
                            found_obstacle = Some(current_key);
                            found_obstacle_bounding_box = current_item.bounding_box(board);
                        }
                        Some(found) if found != current_key => {
                            // check, if 1 obstacle is contained in the other obstacle and take the
                            // bigger obstacle in this case (fixed vias inside of pins).
                            let current_bb = current_item.bounding_box(board);
                            if found_obstacle_bounding_box.intersects_int_box(&current_bb) {
                                if current_bb.contains_regular(&fr_geom::RegularTileShape::IntBox(found_obstacle_bounding_box)) {
                                    found_obstacle = Some(current_key);
                                    found_obstacle_bounding_box = current_bb;
                                } else if !found_obstacle_bounding_box.contains_regular(&fr_geom::RegularTileShape::IntBox(current_bb)) {
                                    return None;
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            if found_obstacle.is_some() {
                break;
            }
        }
        let Some(found_obstacle) = found_obstacle else {
            // no obstacle in the way, nothing to do
            return Some(tpolyline.clone());
        };
        let found = board.item(found_obstacle);
        if recursion_depth <= 0
            || (found.is_board_outline() && board.outline_blocks_nets(net_numbers))
            || (found.is_trace() && !board.is_shove_fixed(found_obstacle))
        {
            board.set_shove_failing_obstacle(Some(found_obstacle));
            return None;
        }
        let mut try_spring_over = true;
        if !over_connected_pins {
            // Check if the obstacle has a trace contact on layer
            for c in board.all_contacts(found_obstacle, Some(layer)).iter() {
                if board.item(c).is_trace() {
                    try_spring_over = false;
                    break;
                }
            }
        }
        let mut obstacle_shape: Option<TileShape> = None;
        if try_spring_over {
            let found = board.item(found_obstacle);
            if found.is_obstacle_area() || found.is_trace() {
                if board.tree_shape_count(DEFAULT_TREE, found_obstacle) == 1 {
                    obstacle_shape = board.tree_shape(DEFAULT_TREE, found_obstacle, 0);
                } else {
                    try_spring_over = false;
                }
            } else if found.is_drill_item() {
                obstacle_shape = board.tree_shape_on_layer(DEFAULT_TREE, found_obstacle, layer);
            }
        }
        if !try_spring_over {
            board.set_shove_failing_obstacle(Some(found_obstacle));
            return None;
        }
        // Java: a null obstacle shape (board outline) throws in enlarge
        let obstacle_shape = obstacle_shape.expect("springOver: obstacle shape is null");
        let mut offset_shape: TileShape = if board.default_tree().is_clearance_compensation_used() {
            let offset = half_width + 1;
            obstacle_shape.enlarge(offset as f64)
        } else {
            // enlarge the shape in 2 steps for symmetry reasons
            let offset = half_width + 1;
            let half_cl_offset = 0.5 * board.clearance_value(board.item(found_obstacle).clearance_class(), clearance_class, layer) as f64;
            let s = obstacle_shape.enlarge(offset as f64 + half_cl_offset);
            s.enlarge(half_cl_offset)
        };
        match board.rules.get_trace_angle_restriction() {
            AngleRestriction::NinetyDegree => offset_shape = TileShape::IntBox(offset_shape.bounding_box()),
            AngleRestriction::FortyfiveDegree => {
                offset_shape = TileShape::IntOctagon(offset_shape.bounding_octagon().expect("springOver: bounding octagon"))
            }
            AngleRestriction::None => {}
        }
        if offset_shape.contains_inside(&polyline.first_corner()) || offset_shape.contains_inside(&polyline.last_corner()) {
            // can happen with clearance compensation off because of asymmetry in calculations
            // with the offset shapes
            board.set_shove_failing_obstacle(Some(found_obstacle));
            return None;
        }
        let entries = offset_shape.entrance_points(polyline);
        if entries.is_empty() {
            return Some(tpolyline.clone()); // no obstacle
        }
        if entries.len() < 2 {
            board.set_shove_failing_obstacle(Some(found_obstacle));
            return None;
        }
        let first_intersection_side_no = entries[0][1];
        let last_intersection_side_no = entries[entries.len() - 1][1];
        let first_intersection_line_no = entries[0][0];
        let last_intersection_line_no = entries[entries.len() - 1][0];
        let border_line_count = offset_shape.border_line_count();
        let mut side_diff = last_intersection_side_no - first_intersection_side_no;
        if side_diff < 0 {
            side_diff += border_line_count;
        } else if side_diff == 0 {
            let compare_corner = offset_shape.corner_approx(first_intersection_side_no);
            let first_intersection =
                polyline.lines[first_intersection_line_no as usize].intersection_approx(&offset_shape.border_line(first_intersection_side_no));
            let second_intersection =
                polyline.lines[last_intersection_line_no as usize].intersection_approx(&offset_shape.border_line(last_intersection_side_no));
            if compare_corner.distance(&second_intersection) < compare_corner.distance(&first_intersection) {
                side_diff += border_line_count;
            }
        }
        let mut border = BorderLines::new(&offset_shape);
        let mut substitute_lines: Vec<TLine> = Vec::with_capacity((side_diff + 3) as usize);
        substitute_lines.push(tpolyline.tline(first_intersection_line_no as usize));
        let mut current_edge_line_no = first_intersection_side_no;
        for _ in 1..=side_diff + 1 {
            substitute_lines.push(border.line(current_edge_line_no));
            if current_edge_line_no == border_line_count - 1 {
                current_edge_line_no = 0;
            } else {
                current_edge_line_no += 1;
            }
        }
        substitute_lines.push(tpolyline.tline(last_intersection_line_no as usize));
        let substitute_polyline = TPolyline::from_lines(&mut substitute_lines);
        // build a circuit around the offset shape in counter clock sense from the first
        // intersection point to the second intersection point
        let pieces = tpolyline.cutout(&offset_shape);
        let mut result = substitute_polyline;
        if !pieces.is_empty() {
            result = pieces[0].combine(&result);
        }
        if pieces.len() > 1 {
            result = result.combine(&pieces[1]);
        }
        Self::spring_over(board, &result, half_width, layer, net_numbers, clearance_class, over_connected_pins, recursion_depth - 1, contact_pins)
    }

    /// Java `springOverObstacles(polyline, halfWidth, layer, netNumbers, clearanceClassIndex,
    /// contactPins)`: wraps the polyline around the obstacles in its way, taking the shorter of
    /// the two senses. `None` if that is not possible; `polyline` if there are no obstacles.
    pub fn spring_over_obstacles(
        board: &mut RoutingBoard,
        polyline: &TPolyline,
        half_width: i32,
        layer: LayerNo,
        net_numbers: &[NetNo],
        clearance_class: ClearanceClassNo,
        contact_pins: Option<&ItemSet>,
    ) -> Option<TPolyline> {
        const MAX_SPRING_OVER_RECURSION_DEPTH: i32 = 20;
        let counter_clock_wise_result = Self::spring_over(
            board,
            polyline,
            half_width,
            layer,
            net_numbers,
            clearance_class,
            true,
            MAX_SPRING_OVER_RECURSION_DEPTH,
            contact_pins,
        );
        if let Some(r) = &counter_clock_wise_result {
            if r.same(polyline) {
                return Some(polyline.clone()); // no obstacle
            }
        }
        let clock_wise_result = Self::spring_over(
            board,
            &polyline.reverse(),
            half_width,
            layer,
            net_numbers,
            clearance_class,
            true,
            MAX_SPRING_OVER_RECURSION_DEPTH,
            contact_pins,
        );
        match (clock_wise_result, counter_clock_wise_result) {
            (Some(cw), Some(ccw)) => {
                if cw.polyline.length_approx() <= ccw.polyline.length_approx() {
                    Some(cw.reverse())
                } else {
                    Some(ccw)
                }
            }
            (Some(cw), None) => Some(cw.reverse()),
            (None, Some(ccw)) => Some(ccw),
            (None, None) => None,
        }
    }
}
