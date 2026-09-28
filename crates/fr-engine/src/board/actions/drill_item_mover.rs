//! Port of `board/actions/DrillItemMover.java` (shoving vias and pins) and of
//! `DrillItem.moveBy(vector)` (moving a drill item and connecting its traces).

use std::collections::BTreeMap;

use fr_geom::{FloatPoint, IntOctagon, IntPoint, Point, TileShape, Vector};

use crate::datastructures::TimeLimit;
use crate::ids::{AngleRestriction, ClearanceClassNo, FixedState, LayerNo, NetNo};
use crate::structure::ShapeEntrySide;

use super::super::item::ItemKey;
use super::super::optimize::trace_shover::shove_obstacles;
use super::super::routing_board::RoutingBoard;
use super::super::search_tree::DEFAULT_TREE;
use super::super::shape_trace_entries::ShapeTraceEntries;
use super::forced_pad_router::{self, CheckDrillResult};

/// The shape of a drill item translated by `vector` as used by `check` and `insert`
/// (bounding box in 90 degree mode, else bounding octagon).
fn translated_tile(board: &RoutingBoard, shape: &TileShape, vector: &Vector) -> TileShape {
    let new_shape = shape.translate_by(vector);
    if board.rules.get_trace_angle_restriction() == AngleRestriction::NinetyDegree {
        TileShape::IntBox(new_shape.bounding_box())
    } else {
        TileShape::IntOctagon(new_shape.bounding_octagon().expect("DrillItemMover: bounding octagon"))
    }
}

/// Java `DrillItemMover.check(drillItem, vector, maxRecursionDepth, maxViaRecursionDepth,
/// ignoreItems, board, timeLimit)`: checks if the drill item can be translated by `vector`,
/// shoving obstacle traces and vias aside. `ignore_items`: Java `null` (`None`) or a list the
/// drill item is added to.
#[allow(clippy::too_many_arguments)]
pub fn check(
    board: &mut RoutingBoard,
    drill_item: ItemKey,
    vector: &Vector,
    max_recursion_depth: i32,
    max_via_recursion_depth: i32,
    ignore_items: Option<Vec<ItemKey>>,
    time_limit: Option<&TimeLimit>,
) -> bool {
    if let Some(t) = time_limit {
        if t.limit_exceeded() {
            return false;
        }
    }
    if board.item(drill_item).is_shove_fixed_state() {
        return false;
    }
    // Check, that the drill item is only connected to traces.
    for c in board.normal_contacts(drill_item).iter() {
        let ci = board.item(c);
        if !(ci.is_trace() || ci.is_conduction_area()) {
            return false;
        }
    }
    let mut effective_ignore_items = ignore_items.unwrap_or_default();
    effective_ignore_items.push(drill_item);
    let item = board.item(drill_item);
    let attach_allowed = item.as_via().map(|v| v.attach_allowed).unwrap_or(false);
    let (first_layer, last_layer) = (item.first_layer(board), item.last_layer(board));
    let nets = item.net_numbers().to_vec();
    let cl = item.clearance_class();
    let center = item.center(board);
    for current_layer in first_layer..=last_layer {
        let current_ind = current_layer - first_layer;
        let Some(current_shape) = board.tree_shape(DEFAULT_TREE, drill_item, current_ind) else {
            continue;
        };
        let current_tile_shape = translated_tile(board, &current_shape, vector);
        let from_side = ShapeEntrySide::from_point(&center, &current_tile_shape);
        if forced_pad_router::check_forced_pad(
            board,
            &current_tile_shape,
            Some(&from_side),
            current_layer,
            &nets,
            cl,
            attach_allowed,
            Some(&effective_ignore_items),
            max_recursion_depth,
            max_via_recursion_depth,
            true,
            time_limit,
        ) == CheckDrillResult::NotDrillable
        {
            return false;
        }
    }
    true
}

/// Java `DrillItemMover.insert(drillItem, vector, maxRecursionDepth, maxViaRecursionDepth,
/// tidyRegion, board)`: translates the drill item by `vector`, shoving obstacle traces and vias
/// aside. (The Java `tidyRegion` parameter is only updated locally and has no effect.)
pub fn insert(board: &mut RoutingBoard, drill_item: ItemKey, vector: &Vector, max_recursion_depth: i32, max_via_recursion_depth: i32) -> bool {
    if board.item(drill_item).is_shove_fixed_state() {
        return false;
    }
    let item = board.item(drill_item);
    let attach_allowed = item.as_via().map(|v| v.attach_allowed).unwrap_or(false);
    let ignore_items = vec![drill_item];
    let (first_layer, last_layer) = (item.first_layer(board), item.last_layer(board));
    let nets = item.net_numbers().to_vec();
    let cl = item.clearance_class();
    let center = item.center(board);
    for current_layer in first_layer..=last_layer {
        let current_ind = current_layer - first_layer;
        let Some(current_shape) = board.tree_shape(DEFAULT_TREE, drill_item, current_ind) else {
            continue;
        };
        let current_tile_shape = translated_tile(board, &current_shape, vector);
        let from_side = ShapeEntrySide::from_point(&center, &current_tile_shape);
        if !forced_pad_router::forced_pad(
            board,
            &current_tile_shape,
            Some(&from_side),
            current_layer,
            &nets,
            cl,
            attach_allowed,
            Some(&ignore_items),
            max_recursion_depth,
            max_via_recursion_depth,
        ) {
            return false;
        }
        let current_bounding_box = TileShape::IntBox(current_shape.bounding_box());
        for j in 0..4 {
            board.join_changed_area(&current_bounding_box.corner_approx(j), current_layer);
        }
    }
    board.move_drill_item_by(drill_item, vector);
    true
}

/// Java `DrillItemMover.shoveVias(obstacleShape, fromSide, layer, netNumbers,
/// clearanceClassIndex, ignoreItems, maxRecursionDepth, maxViaRecursionDepth,
/// copperSharingAllowed, board)`: shoves vias out of the shape. Returns false if the database
/// is damaged (an undo is necessary).
#[allow(clippy::too_many_arguments)]
pub fn shove_vias(
    board: &mut RoutingBoard,
    obstacle_shape: &TileShape,
    from_side: Option<&ShapeEntrySide>,
    layer: LayerNo,
    net_numbers: &[NetNo],
    clearance_class: ClearanceClassNo,
    ignore_items: Option<&[ItemKey]>,
    max_recursion_depth: i32,
    max_via_recursion_depth: i32,
    copper_sharing_allowed: bool,
) -> bool {
    let mut shape_entries = ShapeTraceEntries::new(obstacle_shape.clone(), layer, net_numbers, clearance_class, from_side.cloned());
    let obstacles: Vec<ItemKey> = shove_obstacles(board, obstacle_shape, layer, clearance_class).iter().collect();
    if !shape_entries.store_items(board, &obstacles, false, copper_sharing_allowed) {
        return true;
    }
    if let Some(ignore) = ignore_items {
        shape_entries.shove_via_list.retain(|v| !ignore.contains(v));
    }
    if shape_entries.shove_via_list.is_empty() {
        return true;
    }
    let shape_radius = 0.5 * obstacle_shape.bounding_box().min_width();
    for current_via in shape_entries.shove_via_list.clone() {
        if board.item(current_via).shares_net_no(net_numbers) {
            continue;
        }
        if max_via_recursion_depth <= 0 {
            return true;
        }
        let try_via_centers = try_shove_via_points(board, obstacle_shape, layer, current_via, clearance_class, true);
        let mut new_via_center: Option<IntPoint> = None;
        let max_dist =
            0.5 * board.item(current_via).drill_shape_on_layer(board, layer).expect("via shape").bounding_box().max_width() + shape_radius;
        let max_dist_square = max_dist * max_dist;
        let current_via_center = board.item(current_via).center(board).as_int();
        let check_via_center = current_via_center.to_float();
        let mut rel_coor: Option<Vector> = None;
        for (i, c) in try_via_centers.iter().enumerate() {
            if i == 0 || check_via_center.distance_square(&c.to_float()) <= max_dist_square {
                let local_ignore_items: Vec<ItemKey> = ignore_items.map(|x| x.to_vec()).unwrap_or_default();
                let delta = Vector::Int(c.difference_by_int(&current_via_center));
                // No time limit here because the item database is already changed.
                let shove_ok = check(board, current_via, &delta, max_recursion_depth, max_via_recursion_depth - 1, Some(local_ignore_items), None);
                rel_coor = Some(delta);
                if shove_ok {
                    new_via_center = Some(*c);
                    break;
                }
            }
        }
        if new_via_center.is_none() {
            continue;
        }
        if !insert(board, current_via, rel_coor.as_ref().unwrap(), max_recursion_depth, max_via_recursion_depth - 1) {
            return false;
        }
    }
    true
}

/// Java `DrillItemMover.tryShoveViaPoints(obstacleShape, layer, via, clearanceClassIndex,
/// extendedCheck, board)`: possible new locations of a via shoved outside the shape (several if
/// `extended_check`).
pub fn try_shove_via_points(
    board: &RoutingBoard,
    obstacle_shape: &TileShape,
    layer: LayerNo,
    via: ItemKey,
    clearance_class: ClearanceClassNo,
    extended_check: bool,
) -> Vec<IntPoint> {
    let Some(mut current_via_shape) = board.tree_shape_on_layer(DEFAULT_TREE, via, layer) else {
        return Vec::new();
    };
    let compensated = board.default_tree().is_clearance_compensation_used();
    let is_int_octagon = obstacle_shape.is_int_octagon();
    let clearance_value = board.clearance_value(clearance_class, board.item(via).clearance_class(), layer) as f64;
    let angle_restriction = board.rules.get_trace_angle_restriction();
    let mut shove_distance;
    if angle_restriction == AngleRestriction::NinetyDegree || is_int_octagon {
        shove_distance = 0.5 * current_via_shape.bounding_box().max_width();
        if !compensated {
            shove_distance += clearance_value;
        }
    } else {
        // a different algorithm is used for calculating the new via centers
        shove_distance = 0.0;
        if !compensated {
            // enlarge obstacle_shape and current_via_shape by half of the clearance value to
            // synchronize with the check algorithm in overlapping_tree_entries_with_clearance
            shove_distance += 0.5 * clearance_value;
        }
    }
    // The additional constant 2 is an empirical value for the tolerance in case of diagonal
    // shoving.
    shove_distance += 2.0;
    let current_via_center = board.item(via).center(board).as_int();
    if angle_restriction == AngleRestriction::NinetyDegree {
        let current_offset_box = obstacle_shape.bounding_box().offset(shove_distance);
        let try_count = if extended_check { 2 } else { 1 };
        current_offset_box.nearest_border_projections(&current_via_center, try_count)
    } else if is_int_octagon {
        let octagon: IntOctagon = obstacle_shape.bounding_octagon().expect("tryShoveViaPoints: bounding octagon").enlarge(shove_distance);
        let try_count = if extended_check { 4 } else { 1 };
        octagon.nearest_border_projections(&current_via_center, try_count)
    } else {
        let current_offset_shape = obstacle_shape.enlarge(shove_distance);
        if !compensated {
            current_via_shape = current_via_shape.enlarge(0.5 * clearance_value);
        }
        let try_count = if extended_check { 4 } else { 1 };
        let shove_deltas: Vec<FloatPoint> = current_offset_shape.nearest_relative_outside_locations(&current_via_shape, try_count);
        shove_deltas
            .iter()
            .map(|d| {
                let current_delta = Point::Int(d.round()).difference_by(&Point::ZERO);
                Point::Int(current_via_center).translate_by(&current_delta).as_int()
            })
            .collect()
    }
}

impl RoutingBoard {
    /// Java `DrillItem.moveBy(vector)`: translates the drill item and inserts a trace from the
    /// old to the new center on each layer where it was connected to a trace.
    pub fn move_drill_item_by(&mut self, drill_item: ItemKey, vector: &Vector) {
        let old_center = self.item(drill_item).center(self);
        // remember the contact situation of this drill item to traces on each layer (Java
        // TreeSet<TraceInfo> ordered by descending layer, one entry per layer)
        let mut contact_trace_info: BTreeMap<std::cmp::Reverse<LayerNo>, (i32, ClearanceClassNo)> = BTreeMap::new();
        for c in self.normal_contacts(drill_item).iter() {
            let ci = self.item(c);
            if let Some(t) = ci.as_trace() {
                contact_trace_info.entry(std::cmp::Reverse(t.layer())).or_insert((t.half_width(), ci.clearance_class()));
            }
        }
        // Item.moveBy
        self.translate_item(drill_item, vector);
        // Insert a trace from the old center to the new center, on all layers, where this drill
        // item was connected to a trace.
        let mut connect_points: Vec<Point> = vec![old_center.clone()];
        let new_center = self.item(drill_item).center(self);
        if let (Point::Int(o), Point::Int(n)) = (&old_center, &new_center) {
            // Make sure, that the traces will remain 90- or 45-degree.
            let add_corner = match self.rules.get_trace_angle_restriction() {
                AngleRestriction::NinetyDegree => o.ninety_degree_corner(n, true),
                AngleRestriction::FortyfiveDegree => o.fortyfive_degree_corner(n, true),
                AngleRestriction::None => None,
            };
            if let Some(c) = add_corner {
                connect_points.push(Point::Int(c));
            }
        }
        connect_points.push(new_center);
        let nets = self.item(drill_item).net_numbers().to_vec();
        for (layer, (half_width, cl)) in contact_trace_info {
            self.insert_trace_points(&connect_points, layer.0, half_width, &nets, cl, FixedState::Unfixed);
        }
    }
}
