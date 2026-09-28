//! Port of `autoroute/path/{FoundConnectionLocator, FoundConnectionLocator45Degree,
//! FoundConnectionLocatorAnyAngle}.java`: calculates from the backtrack list of the maze search the
//! corners of the traces and the locations of the vias of the found connection.

use std::collections::HashMap;

use fr_geom::{FloatLine, FloatPoint, IntPoint, Point, Side, Signum, TileShape};

use crate::board::{ItemKey, ItemSet, RoutingBoard};
use crate::ids::{AngleRestriction, LayerNo};

use super::control::AutorouteControl;
use super::engine::{AutorouteEngine, Expandable, RoomId};
use super::maze::MazeSearchResult;
use super::{JResult, JavaException, TRACE_WIDTH_TOLERANCE};

/// Java `FoundConnectionLocator.ResultItem`: the corners of a new trace and its layer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResultItem {
    pub corners: Vec<IntPoint>,
    pub layer: LayerNo,
}

/// Java `FoundConnectionLocator.BacktrackElement`.
#[derive(Clone, Copy, Debug)]
pub struct BacktrackElement {
    pub door: Expandable,
    pub section_no_of_door: i32,
    /// The common room of this door and the next door in the backtrack list.
    pub next_room: Option<RoomId>,
}

/// A corner returned by `calculateNextTraceCorners`: Java compares the returned objects with
/// `currentFromPoint` by identity.
#[derive(Clone, Copy, Debug)]
enum Corner {
    /// The `currentFromPoint` object itself.
    Current,
    New(FloatPoint),
}

/// Java `FoundConnectionLocator` (the result of the construction).
#[derive(Clone, Debug)]
pub struct FoundConnectionLocator {
    /// The new items implementing the found connection.
    pub connection_items: Vec<ResultItem>,
    /// The start item of the new routed connection.
    pub start_item: Option<ItemKey>,
    /// The layer of the connection to the start item.
    pub start_layer: LayerNo,
    /// The destination item of the new routed connection.
    pub target_item: Option<ItemKey>,
    /// The layer of the connection to the target item.
    pub target_layer: LayerNo,
    /// The backtrack doors from the destination to the start.
    pub backtrack_array: Vec<BacktrackElement>,
}

struct Locator<'a> {
    eng: &'a mut AutorouteEngine,
    ctrl: &'a AutorouteControl,
    angle_restriction: AngleRestriction,
    /// True for `FoundConnectionLocator45Degree` (45 and 90 degree), false for any angle.
    fortyfive: bool,
    backtrack_array: Vec<BacktrackElement>,
    current_from_point: FloatPoint,
    previous_from_point: FloatPoint,
    current_trace_layer: LayerNo,
    current_from_door_index: i32,
    current_to_door_index: i32,
    current_target_door_index: i32,
    current_target_shape: Option<TileShape>,
}

impl FoundConnectionLocator {
    /// Java `FoundConnectionLocator.getInstance(mazeSearchResult, ctrl, searchTree,
    /// angleRestriction, rippedItemList, ripupCosts)`. `Err` is a Java exception (caught by the
    /// caller).
    pub fn get_instance(
        maze_search_result: &MazeSearchResult,
        ctrl: &AutorouteControl,
        eng: &mut AutorouteEngine,
        board: &RoutingBoard,
        angle_restriction: AngleRestriction,
        ripped_item_list: &mut ItemSet,
        ripup_costs: Option<&mut HashMap<ItemKey, i32>>,
    ) -> JResult<FoundConnectionLocator> {
        let fortyfive = matches!(angle_restriction, AngleRestriction::NinetyDegree | AngleRestriction::FortyfiveDegree);
        let backtrack_array = backtrack(eng, board, maze_search_result, ripped_item_list, ripup_costs)?;
        let mut result = FoundConnectionLocator {
            connection_items: Vec::new(),
            start_item: None,
            start_layer: 0,
            target_item: None,
            target_layer: 0,
            backtrack_array: backtrack_array.clone(),
        };
        let start_info = *backtrack_array.last().ok_or(JavaException)?;
        let Expandable::Target(start_door) = start_info.door else {
            log::warn!("FoundConnectionLocator: ItemExpansionDoor expected for startInfo.door");
            return Ok(result);
        };
        let (start_item, start_door_room, start_tree_entry_no) = {
            let td = eng.target_door(start_door);
            (td.item, td.room, td.tree_entry_no)
        };
        result.start_item = Some(start_item);
        result.start_layer = eng.room(start_door_room).layer;
        let mut loc = Locator {
            eng,
            ctrl,
            angle_restriction,
            fortyfive,
            backtrack_array,
            current_from_point: FloatPoint::ZERO,
            previous_from_point: FloatPoint::ZERO,
            current_trace_layer: 0,
            current_from_door_index: 0,
            current_to_door_index: 0,
            current_target_door_index: 0,
            current_target_shape: None,
        };
        let mut at_fanout_end = false;
        match maze_search_result.destination_door {
            Expandable::Target(d) => {
                let (item, room, tree_entry_no) = {
                    let td = loc.eng.target_door(d);
                    (td.item, td.room, td.tree_entry_no)
                };
                result.target_item = Some(item);
                result.target_layer = loc.eng.room(room).layer;
                loc.current_from_point = calculate_starting_point(loc.eng, board, item, tree_entry_no, room)?;
            }
            Expandable::Drill(d) => {
                // may happen only in case of fanout
                result.target_item = None;
                let drill = loc.eng.drill(d);
                loc.current_from_point = drill.location.to_float();
                result.target_layer = drill.first_layer + maze_search_result.section_no_of_door;
                at_fanout_end = true;
            }
            _ => {
                log::warn!("FoundConnectionLocator: unexpected type of destinationDoor");
                result.target_item = None;
                result.target_layer = 0;
                return Ok(result);
            }
        }
        loc.current_trace_layer = result.target_layer;
        loc.previous_from_point = loc.current_from_point;
        let backtrack_len = loc.backtrack_array.len() as i32;
        let mut connection_done = false;
        while !connection_done {
            let mut layer_changed = false;
            if at_fanout_end {
                // do not increase this.currentTargetDoorIndex
                layer_changed = true;
            } else {
                loc.current_target_door_index = loc.current_from_door_index + 1;
                while loc.current_target_door_index < backtrack_len && !layer_changed {
                    if let Expandable::Drill(_) = loc.backtrack(loc.current_target_door_index)?.door {
                        layer_changed = true;
                    } else {
                        loc.current_target_door_index += 1;
                    }
                }
            }
            if layer_changed {
                // the next trace leads to a via
                let Expandable::Drill(d) = loc.backtrack(loc.current_target_door_index)?.door else {
                    return Err(JavaException);
                };
                let location = loc.eng.drill(d).location.clone();
                loc.current_target_shape = Some(TileShape::IntBox(TileShape::get_instance_point(&location)));
            } else {
                // the next trace leads to the final target
                connection_done = true;
                loc.current_target_door_index = backtrack_len - 1;
                let target_shape = board.trace_connection_shape(loc.eng.tree, start_item, start_tree_entry_no).ok_or(JavaException)?;
                let mut target = target_shape.intersection(loc.eng.room(start_door_room).shape());
                if target.dimension() >= 2 {
                    // the target is a conduction area, make a save connection by shrinking the
                    // shape by the trace halfwidth.
                    let trace_half_width = ctrl.compensated_trace_half_width[loc.eng.room(start_door_room).layer as usize] as f64;
                    let shrinked_shape = target.offset(-trace_half_width);
                    if !shrinked_shape.is_empty() {
                        target = shrinked_shape;
                    }
                }
                loc.current_target_shape = Some(target);
            }
            loc.current_to_door_index = loc.current_from_door_index + 1;
            let next_trace = loc.calculate_next_trace(layer_changed, at_fanout_end)?;
            at_fanout_end = false;
            result.connection_items.push(next_trace);
        }
        Ok(result)
    }
}

/// Java `calculateStartingPoint(fromDoor, searchTree)`.
fn calculate_starting_point(eng: &AutorouteEngine, board: &RoutingBoard, item: ItemKey, tree_entry_no: i32, room: RoomId) -> JResult<FloatPoint> {
    let connection_shape = board.trace_connection_shape(eng.tree, item, tree_entry_no).ok_or(JavaException)?;
    let connection_shape = connection_shape.intersection(eng.room(room).shape());
    Ok(connection_shape.centre_of_gravity().round().to_float())
}

/// Java `FoundConnectionLocator.backtrack(...)`: the doors from the destination back to the start.
/// The ripped obstacle items are added to `ripped_item_list`.
fn backtrack(
    eng: &AutorouteEngine,
    board: &RoutingBoard,
    maze_search_result: &MazeSearchResult,
    ripped_item_list: &mut ItemSet,
    mut ripup_costs: Option<&mut HashMap<ItemKey, i32>>,
) -> JResult<Vec<BacktrackElement>> {
    let mut result = Vec::new();
    let mut current_backtrack_door = maze_search_result.destination_door;
    let mut current_element = eng.element(current_backtrack_door, maze_search_result.section_no_of_door).clone();
    let mut add_ripped = |room: Option<RoomId>, cost: i32, list: &mut ItemSet| {
        if let Some(r) = room {
            if let Some(item) = eng.room(r).obstacle_item() {
                list.insert(board.item(item).id(), item);
                if let Some(m) = ripup_costs.as_deref_mut() {
                    m.insert(item, cost);
                }
            }
        }
    };
    let mut current_next_room: Option<RoomId> = match current_backtrack_door {
        Expandable::Target(t) => Some(eng.target_door(t).room),
        Expandable::Drill(d) => {
            let drill = eng.drill(d);
            let index = drill.first_layer + maze_search_result.section_no_of_door;
            let room = *drill.room_arr.get(index as usize).ok_or(JavaException)?;
            if current_element.room_ripped {
                for tmp_room in drill.room_arr.clone() {
                    add_ripped(tmp_room, current_element.ripup_cost, ripped_item_list);
                }
            }
            room
        }
        _ => None,
    };
    let mut current = BacktrackElement { door: current_backtrack_door, section_no_of_door: maze_search_result.section_no_of_door, next_room: current_next_room };
    loop {
        result.push(current);
        let Some(bt) = current_element.backtrack_door else {
            break;
        };
        current_backtrack_door = bt;
        let mut current_section_no = current_element.section_no_of_backtrack_door;
        let count = eng.expandable_element_count(current_backtrack_door);
        if current_section_no >= count {
            log::warn!("FoundConnectionLocator: currentSectionNo to big");
            current_section_no = count - 1;
        }
        if let Expandable::Drill(d) = current_backtrack_door {
            current_next_room = *eng.drill(d).room_arr.get(current_section_no as usize).ok_or(JavaException)?;
        } else {
            current_next_room = eng.expandable_other_room(current_backtrack_door, current_next_room);
        }
        current_element = eng.element(current_backtrack_door, current_section_no).clone();
        current = BacktrackElement { door: current_backtrack_door, section_no_of_door: current_section_no, next_room: current_next_room };
        if current_element.room_ripped {
            add_ripped(current_next_room, current_element.ripup_cost, ripped_item_list);
        }
    }
    Ok(result)
}

fn ninety_degree_corner(from_point: &FloatPoint, to_point: &FloatPoint, horizontal_first: bool) -> FloatPoint {
    if horizontal_first {
        FloatPoint::new(to_point.x, from_point.y)
    } else {
        FloatPoint::new(from_point.x, to_point.y)
    }
}

fn fortyfive_degree_corner(from_point: &FloatPoint, to_point: &FloatPoint, horizontal_first: bool) -> FloatPoint {
    let abs_dx = (to_point.x - from_point.x).abs();
    let abs_dy = (to_point.y - from_point.y).abs();
    let x;
    let y;
    if abs_dx <= abs_dy {
        if horizontal_first {
            x = to_point.x;
            y = if to_point.y >= from_point.y { from_point.y + abs_dx } else { from_point.y - abs_dx };
        } else {
            x = from_point.x;
            y = if to_point.y > from_point.y { to_point.y - abs_dx } else { to_point.y + abs_dx };
        }
    } else if horizontal_first {
        y = from_point.y;
        x = if to_point.x > from_point.x { to_point.x - abs_dy } else { to_point.x + abs_dy };
    } else {
        y = to_point.y;
        x = if to_point.x > from_point.x { from_point.x + abs_dy } else { from_point.x - abs_dy };
    }
    FloatPoint::new(x, y)
}

/// Java `FoundConnectionLocator.calculateAdditionalCorner(fromPoint, toPoint, horizontalFirst,
/// angleRestriction)`.
pub fn calculate_additional_corner(from_point: &FloatPoint, to_point: &FloatPoint, horizontal_first: bool, angle_restriction: AngleRestriction) -> FloatPoint {
    match angle_restriction {
        AngleRestriction::NinetyDegree => ninety_degree_corner(from_point, to_point, horizontal_first),
        AngleRestriction::FortyfiveDegree => fortyfive_degree_corner(from_point, to_point, horizontal_first),
        AngleRestriction::None => *to_point,
    }
}

fn round_to_integer(p: &FloatPoint) -> FloatPoint {
    p.round().to_float()
}

impl Locator<'_> {
    fn backtrack(&self, index: i32) -> JResult<BacktrackElement> {
        if index < 0 {
            return Err(JavaException);
        }
        self.backtrack_array.get(index as usize).copied().ok_or(JavaException)
    }

    /// Java `calculateNextTrace(layerChanged, atFanoutEnd)`.
    fn calculate_next_trace(&mut self, layer_changed: bool, at_fanout_end: bool) -> JResult<ResultItem> {
        let mut corner_list: Vec<FloatPoint> = vec![self.current_from_point];
        if !at_fanout_end {
            if let Some(adjusted_start_corner) = self.adjust_start_corner()? {
                let add_corner = calculate_additional_corner(&self.current_from_point, &adjusted_start_corner, true, self.angle_restriction);
                corner_list.push(add_corner);
                corner_list.push(adjusted_start_corner);
                self.previous_from_point = self.current_from_point;
                self.current_from_point = adjusted_start_corner;
            }
        }
        loop {
            let next_corners = if self.fortyfive { self.calculate_next_trace_corners_45()? } else { self.calculate_next_trace_corners_any_angle()? };
            if next_corners.is_empty() {
                break;
            }
            for c in next_corners {
                // (Java: `if (currentNextCorner != prevCorner)`; the only corner identical to
                // prevCorner is currentFromPoint itself)
                if let Corner::New(p) = c {
                    corner_list.push(p);
                    self.previous_from_point = self.current_from_point;
                    self.current_from_point = p;
                }
            }
        }
        let mut next_layer = self.current_trace_layer;
        if layer_changed {
            self.current_from_door_index = self.current_target_door_index + 1;
            if let Some(next_room) = self.backtrack(self.current_from_door_index)?.next_room {
                next_layer = self.eng.room(next_room).layer;
            }
        }
        // Round the new trace corners to Integer.
        let mut corners: Vec<IntPoint> = Vec::new();
        let mut prev_point: Option<IntPoint> = None;
        for corner in corner_list {
            let current_point = corner.round();
            if Some(current_point) != prev_point {
                corners.push(current_point);
                prev_point = Some(current_point);
            }
        }
        let result = ResultItem { corners, layer: self.current_trace_layer };
        self.current_trace_layer = next_layer;
        Ok(result)
    }

    /// Java `adjustStartCorner()`: `None` if `currentFromPoint` is kept.
    fn adjust_start_corner(&self) -> JResult<Option<FloatPoint>> {
        if self.current_from_door_index < 0 {
            return Ok(None);
        }
        let current_from_info = self.backtrack(self.current_from_door_index)?;
        let Some(next_room) = current_from_info.next_room else {
            return Ok(None);
        };
        let trace_half_width = self.ctrl.compensated_trace_half_width[self.current_trace_layer as usize] as f64;
        let shrinked_room_shape = self.eng.room(next_room).shape().offset(-trace_half_width);
        if shrinked_room_shape.is_empty() || shrinked_room_shape.contains_float(&self.current_from_point) {
            return Ok(None);
        }
        let p = shrinked_room_shape.nearest_point_approx(&self.current_from_point).ok_or(JavaException)?;
        Ok(Some(p.round().to_float()))
    }

    // ------------------------------------------------------------------------------------------
    // FoundConnectionLocator45Degree

    fn calculate_next_trace_corners_45(&mut self) -> JResult<Vec<Corner>> {
        let mut result = Vec::new();
        if self.current_to_door_index > self.current_target_door_index {
            return Ok(result);
        }
        let current_from_info = self.backtrack(self.current_to_door_index - 1)?;
        let Some(from_room) = current_from_info.next_room else {
            log::warn!("FoundConnectionLocator45Degree.calculate_next_trace_corners: nextRoom is null");
            return Ok(result);
        };
        let room_shape = self.eng.room(from_room).shape().clone();
        let trace_halfwidth = self.ctrl.compensated_trace_half_width[self.current_trace_layer as usize];
        let trace_halfwidth_add = trace_halfwidth + TRACE_WIDTH_TOLERANCE;
        let shrink_offset = if self.eng.room(from_room).is_obstacle() { trace_halfwidth } else { trace_halfwidth_add };
        let mut shrinked_room_shape = room_shape.offset(-(shrink_offset as f64));
        if !shrinked_room_shape.is_empty() {
            // enter the shrunk room shape by a 45-degree angle first
            let nearest_room_point = shrinked_room_shape.nearest_point_approx(&self.current_from_point).ok_or(JavaException)?;
            let horizontal_first = self.calc_horizontal_first_from_door(current_from_info.door, &self.current_from_point, &nearest_room_point);
            let nearest_room_point = round_to_integer(&nearest_room_point);
            result.push(Corner::New(calculate_additional_corner(&self.current_from_point, &nearest_room_point, horizontal_first, self.angle_restriction)));
            result.push(Corner::New(nearest_room_point));
            self.current_from_point = nearest_room_point;
        } else {
            shrinked_room_shape = room_shape;
        }
        if self.current_to_door_index == self.current_target_door_index {
            let target = self.current_target_shape.as_ref().ok_or(JavaException)?;
            let nearest_point = round_to_integer(&target.nearest_point_approx(&self.current_from_point).ok_or(JavaException)?);
            let mut add_corner = calculate_additional_corner(&self.current_from_point, &nearest_point, true, self.angle_restriction);
            if !shrinked_room_shape.contains_float(&add_corner) {
                add_corner = calculate_additional_corner(&self.current_from_point, &nearest_point, false, self.angle_restriction);
            }
            result.push(Corner::New(add_corner));
            result.push(Corner::New(nearest_point));
            self.current_to_door_index += 1;
            return Ok(result);
        }
        let current_to_info = self.backtrack(self.current_to_door_index)?;
        let Expandable::Door(current_to_door) = current_to_info.door else {
            log::warn!("FoundConnectionLocator45Degree.calculate_next_trace_corners: ExpansionDoor expected");
            return Ok(result);
        };
        let mut nearest_to_door_point: FloatPoint;
        if self.eng.door(current_to_door).dimension == 2 {
            // May not happen in free angle routing mode because then corners are cut off.
            let to_door_shape = self.eng.door_shape(current_to_door);
            let shrinked_to_door_shape = to_door_shape.shrink(shrink_offset as f64);
            nearest_to_door_point = round_to_integer(&shrinked_to_door_shape.nearest_point_approx(&self.current_from_point).ok_or(JavaException)?);
        } else {
            let line_sections = self.eng.door_section_segments(current_to_door, trace_halfwidth as f64);
            if current_to_info.section_no_of_door >= line_sections.len() as i32 {
                log::warn!("FoundConnectionLocator45Degree.calculate_next_trace_corners: lineSections inconsistent");
                return Ok(result);
            }
            let current_line_section = line_sections[current_to_info.section_no_of_door as usize];
            let section_len = current_line_section.b.distance(&current_line_section.a);
            if section_len <= 2.5 * trace_halfwidth_add as f64 {
                nearest_to_door_point = current_line_section.a.middle_point(&current_line_section.b);
            } else {
                let safe_line_section = current_line_section.shrink_segment(trace_halfwidth_add as f64);
                nearest_to_door_point = safe_line_section.nearest_segment_point(&self.current_from_point);
            }
            let mut nearest_to_door_point_ok = true;
            if let Some(next_room) = current_to_info.next_room {
                let next_room_shape = TileShape::Simplex(self.eng.room(next_room).shape().to_simplex());
                // with IntBox or IntOctagon the next calculation will not work, because they have
                // border lines of length 0.
                let nearest_points = next_room_shape.nearest_border_points_approx(&nearest_to_door_point, 2);
                if nearest_points.len() >= 2 {
                    nearest_to_door_point_ok = nearest_points[1].ok_or(JavaException)?.distance(&nearest_to_door_point) >= trace_halfwidth_add as f64;
                }
            }
            if nearest_to_door_point_ok {
                let prev_room_shape = TileShape::Simplex(self.eng.room(from_room).shape().to_simplex());
                let prev_nearest_points = prev_room_shape.nearest_border_points_approx(&nearest_to_door_point, 2);
                if prev_nearest_points.len() >= 2 {
                    nearest_to_door_point_ok = prev_nearest_points[1].ok_or(JavaException)?.distance(&nearest_to_door_point) >= trace_halfwidth_add as f64;
                }
            }
            if !nearest_to_door_point_ok {
                // may be the room has an acute (45 degree) angle at a corner of the door
                nearest_to_door_point = current_line_section.a.middle_point(&current_line_section.b);
            }
        }
        nearest_to_door_point = round_to_integer(&nearest_to_door_point);
        let horizontal_first = self.calc_horizontal_first_to_door(current_to_info.door, &self.current_from_point, &nearest_to_door_point);
        result.push(Corner::New(calculate_additional_corner(&self.current_from_point, &nearest_to_door_point, horizontal_first, self.angle_restriction)));
        result.push(Corner::New(nearest_to_door_point));
        self.current_to_door_index += 1;
        Ok(result)
    }

    /// Java `calcHorizontalFirstFromDoor(fromDoor, fromPoint, toPoint)`.
    fn calc_horizontal_first_from_door(&self, from_door: Expandable, from_point: &FloatPoint, to_point: &FloatPoint) -> bool {
        self.calc_horizontal_first(from_door, from_point, to_point, true)
    }

    /// Java `calcHorizontalFirstToDoor(toDoor, fromPoint, toPoint)`.
    fn calc_horizontal_first_to_door(&self, to_door: Expandable, from_point: &FloatPoint, to_point: &FloatPoint) -> bool {
        self.calc_horizontal_first(to_door, from_point, to_point, false)
    }

    fn calc_horizontal_first(&self, door: Expandable, from_point: &FloatPoint, to_point: &FloatPoint, from_door: bool) -> bool {
        let door_shape = self.eng.expandable_shape(door);
        let door_box = door_shape.bounding_box();
        if self.eng.expandable_dimension(door) != 1 {
            return if from_door { door_box.height() >= door_box.width() } else { door_box.height() <= door_box.width() };
        }
        let door_line_segment = door_shape.diagonal_corner_segment().expect("diagonal corner segment");
        let (left_corner, right_corner) = if door_line_segment.a.x < door_line_segment.b.x
            || door_line_segment.a.x == door_line_segment.b.x && door_line_segment.a.y <= door_line_segment.b.y
        {
            (door_line_segment.a, door_line_segment.b)
        } else {
            (door_line_segment.b, door_line_segment.a)
        };
        let door_dx = right_corner.x - left_corner.x;
        let door_dy = right_corner.y - left_corner.y;
        let abs_door_dy = door_dy.abs();
        let door_max_width = super::control::jmax(door_dx, abs_door_dy);
        let door_half_max_width = 0.5 * door_max_width;
        if door_box.width() as f64 <= door_half_max_width {
            // door is about vertical
            from_door
        } else if door_box.height() as f64 <= door_half_max_width {
            // door is about horizontal
            !from_door
        } else {
            let dx = to_point.x - from_point.x;
            let dy = to_point.y - from_point.y;
            let same_sign = Signum::of(dx) == Signum::of(dy);
            let right_diagonal = left_corner.y < right_corner.y;
            // from door: right diagonal: same sign -> |dx| > |dy|; to door: the opposite
            let dx_bigger = dx.abs() > dy.abs();
            let dx_smaller = dx.abs() < dy.abs();
            match (from_door, right_diagonal, same_sign) {
                (true, true, true) => dx_bigger,
                (true, true, false) => dx_smaller,
                (true, false, true) => dx_smaller,
                (true, false, false) => dx_bigger,
                (false, true, true) => dx_smaller,
                (false, true, false) => dx_bigger,
                (false, false, true) => dx_bigger,
                (false, false, false) => dx_smaller,
            }
        }
    }

    // ------------------------------------------------------------------------------------------
    // FoundConnectionLocatorAnyAngle

    /// Java `calcDoorLeftCorner(toInfo)` / `calcDoorRightCorner(toInfo)`.
    fn calc_door_corner(&self, to_info: &BacktrackElement, left: bool) -> JResult<FloatPoint> {
        let from_room = self.eng.expandable_other_room(to_info.door, to_info.next_room).ok_or(JavaException)?;
        let pole = self.eng.room(from_room).shape().centre_of_gravity();
        let current_to_door_shape = self.eng.expandable_shape(to_info.door);
        let no = if left { current_to_door_shape.index_of_left_most_corner(&pole) } else { current_to_door_shape.index_of_right_most_corner(&pole) };
        Ok(current_to_door_shape.corner_approx(no))
    }

    fn calculate_next_trace_corners_any_angle(&mut self) -> JResult<Vec<Corner>> {
        let mut result = Vec::new();
        let cur = self.current_from_point;
        let target_shape = self.current_target_shape.clone().ok_or(JavaException)?;
        if self.current_to_door_index >= self.current_target_door_index {
            if self.current_to_door_index == self.current_target_door_index {
                let nearest_point = target_shape.nearest_point(&Point::Int(cur.round())).ok_or(JavaException)?.to_float();
                self.current_to_door_index += 1;
                result.push(Corner::New(nearest_point));
            }
            return Ok(result);
        }
        let trace_halfwidth_exact = self.ctrl.compensated_trace_half_width[self.current_trace_layer as usize] as f64;
        let trace_halfwidth_max = trace_halfwidth_exact + TRACE_WIDTH_TOLERANCE as f64;
        let trace_halfwidth_middle = trace_halfwidth_exact + C_TOLERANCE;
        let current_to_info = self.backtrack(self.current_to_door_index)?;
        let mut door_left_corner = Some(self.calc_door_corner(&current_to_info, true)?);
        let mut door_right_corner = Some(self.calc_door_corner(&current_to_info, false)?);
        if cur.side_of(door_left_corner.as_ref().unwrap(), door_right_corner.as_ref().unwrap()) != Side::OnTheRight {
            // the door is already crossed at this.fromPoint
            if cur.scalar_product(&self.previous_from_point, door_left_corner.as_ref().unwrap()) >= 0.0 {
                // Also the left corner of the door is passed.
                door_left_corner = None;
            }
            if cur.scalar_product(&self.previous_from_point, door_right_corner.as_ref().unwrap()) >= 0.0 {
                // Also the right corner of the door is passed.
                door_right_corner = None;
            }
            if door_left_corner.is_none() && door_right_corner.is_none() {
                // The door is completely passed.
                self.current_to_door_index += 1;
                result.push(Corner::Current);
                return Ok(result);
            }
        }
        // Calculate the visibility range for a trace line from currentFromPoint through the
        // interval from left_most_visible_point to right_most_visible_point, by advancing the door
        // index as far as possible, so that still something is visible.
        let mut end_of_trace = false;
        let mut left_tangent_point: Option<FloatPoint>;
        let mut right_tangent_point: Option<FloatPoint>;
        let mut new_door_ind = self.current_to_door_index;
        let mut left_ind = new_door_ind;
        let mut right_ind = new_door_ind;
        let mut current_door_ind = self.current_to_door_index + 1;
        // (point, is the currentFromPoint object)
        let mut result_corner: Option<(FloatPoint, bool)> = None;
        // construct a maximum length straight line through the doors
        loop {
            left_tangent_point = right_tangential_point(&cur, door_left_corner.as_ref(), trace_halfwidth_max);
            if door_left_corner.is_some() && left_tangent_point.is_none() {
                left_tangent_point = door_left_corner;
            }
            right_tangent_point = left_tangential_point(&cur, door_right_corner.as_ref(), trace_halfwidth_max);
            if door_right_corner.is_some() && right_tangent_point.is_none() {
                right_tangent_point = door_right_corner;
            }
            if let (Some(ltp), Some(rtp)) = (&left_tangent_point, &right_tangent_point) {
                if rtp.side_of(&cur, ltp) != Side::OnTheRight {
                    // The gap between left_most_visible_point and right_most_visible_point is too
                    // small for a trace with the current half width.
                    let dlc = door_left_corner.ok_or(JavaException)?;
                    let drc = door_right_corner.ok_or(JavaException)?;
                    let left_corner_distance = dlc.distance(&cur);
                    let right_corner_distance = drc.distance(&cur);
                    if left_corner_distance <= right_corner_distance {
                        new_door_ind = left_ind;
                        result_corner = left_turn_next_corner(&cur, trace_halfwidth_max, &dlc, &drc);
                    } else {
                        new_door_ind = right_ind;
                        result_corner = right_turn_next_corner(&cur, trace_halfwidth_max, &drc, &dlc);
                    }
                    break;
                }
            }
            if current_door_ind >= self.current_target_door_index {
                end_of_trace = true;
                break;
            }
            let next_to_info = self.backtrack(current_door_ind)?;
            let mut next_left_corner = Some(self.calc_door_corner(&next_to_info, true)?);
            let mut next_right_corner = Some(self.calc_door_corner(&next_to_info, false)?);
            if cur.side_of(next_left_corner.as_ref().unwrap(), next_right_corner.as_ref().unwrap()) != Side::OnTheRight {
                // the door may be already crossed at this.fromPoint
                if door_left_corner.is_none() && cur.scalar_product(&self.previous_from_point, next_left_corner.as_ref().unwrap()) >= 0.0 {
                    next_left_corner = None;
                }
                if door_right_corner.is_none() && cur.scalar_product(&self.previous_from_point, next_right_corner.as_ref().unwrap()) >= 0.0 {
                    next_right_corner = None;
                }
                if next_left_corner.is_none() && next_right_corner.is_none() {
                    // The door is completely passed.
                    self.current_to_door_index += 1;
                    result.push(Corner::Current);
                    return Ok(result);
                }
            }
            if let (Some(dlc), Some(drc)) = (door_left_corner, door_right_corner) {
                // otherwise the following sideOf conditions may not be correct even if all
                // parameter points are defined
                let nlc = next_left_corner.ok_or(JavaException)?;
                if nlc.side_of(&cur, &drc) == Side::OnTheRight {
                    // bend to the right
                    new_door_ind = right_ind + 1;
                    result_corner = right_turn_next_corner(&cur, trace_halfwidth_max, &drc, &nlc);
                    break;
                }
                let nrc = next_right_corner.ok_or(JavaException)?;
                if nrc.side_of(&cur, &dlc) == Side::OnTheLeft {
                    // bend to the left
                    new_door_ind = left_ind + 1;
                    result_corner = left_turn_next_corner(&cur, trace_halfwidth_max, &dlc, &nrc);
                    break;
                }
            }
            let mut smaller_on_the_right = door_right_corner.is_none();
            if let Some(drc) = door_right_corner {
                let nrc = next_right_corner.ok_or(JavaException)?;
                if nrc.side_of(&cur, &drc) != Side::OnTheRight {
                    if let Some(tp) = cur.left_tangential_point(&nrc, trace_halfwidth_max) {
                        let check_line = FloatLine::new(cur, tp);
                        if check_line.segment_distance(&drc) >= trace_halfwidth_max {
                            smaller_on_the_right = true;
                        }
                    }
                }
            }
            if smaller_on_the_right {
                // The visibility range gets smaller on the right side.
                door_right_corner = next_right_corner;
                right_ind = current_door_ind;
            }
            let mut smaller_on_the_left = door_left_corner.is_none();
            if let Some(dlc) = door_left_corner {
                let nlc = next_left_corner.ok_or(JavaException)?;
                if nlc.side_of(&cur, &dlc) != Side::OnTheLeft {
                    if let Some(tp) = cur.right_tangential_point(&nlc, trace_halfwidth_max) {
                        let check_line = FloatLine::new(cur, tp);
                        if check_line.segment_distance(&dlc) >= trace_halfwidth_max {
                            smaller_on_the_left = true;
                        }
                    }
                }
            }
            if smaller_on_the_left {
                // The visibility range gets smaller on the left side.
                door_left_corner = next_left_corner;
                left_ind = current_door_ind;
            }
            current_door_ind += 1;
        }
        if end_of_trace {
            let nearest_point = target_shape.nearest_point(&Point::Int(cur.round())).ok_or(JavaException)?.to_float();
            result_corner = Some((nearest_point, false));
            if left_tangent_point.is_some() && nearest_point.side_of(&cur, left_tangent_point.as_ref().unwrap()) == Side::OnTheLeft {
                // The nearest target point is to the left of the visible range, add another corner
                new_door_ind = left_ind + 1;
                let target_right_corner = target_shape.corner_approx(target_shape.index_of_right_most_corner(&cur));
                if let Some(c) = right_left_tangential_point(&cur, &target_right_corner, door_left_corner.as_ref(), trace_halfwidth_max) {
                    result_corner = Some((c, false));
                    end_of_trace = false;
                }
            } else if right_tangent_point.is_some() && nearest_point.side_of(&cur, right_tangent_point.as_ref().unwrap()) == Side::OnTheRight {
                // The nearest target point is to the right of the visible range, add another corner
                let target_left_corner = target_shape.corner_approx(target_shape.index_of_left_most_corner(&cur));
                new_door_ind = right_ind + 1;
                if let Some(c) = left_right_tangential_point(&cur, &target_left_corner, door_right_corner.as_ref(), trace_halfwidth_max) {
                    result_corner = Some((c, false));
                    end_of_trace = false;
                }
            }
        }
        if end_of_trace {
            new_door_ind = self.current_target_door_index;
        }
        // Check clearance violation with the previous door shapes and correct them in this case.
        let check_line = result_corner.map(|(p, _)| FloatLine::new(cur, p));
        let check_from_door_index = (self.current_to_door_index - 5).max(self.current_from_door_index + 1);
        let mut corrected_result: Option<FloatPoint> = None;
        let mut corrected_door_ind = 0;
        for i in check_from_door_index..new_door_ind {
            let check_line = check_line.ok_or(JavaException)?;
            let info = self.backtrack(i)?;
            let current_left_corner = self.calc_door_corner(&info, true)?;
            let current_distance = check_line.segment_distance(&current_left_corner);
            if current_distance.abs() < trace_halfwidth_middle {
                if let Some(c) = right_left_tangential_point(&check_line.a, &check_line.b, Some(&current_left_corner), trace_halfwidth_max) {
                    if corrected_result.is_none() || c.side_of(&cur, corrected_result.as_ref().unwrap()) == Side::OnTheRight {
                        corrected_door_ind = i;
                        corrected_result = Some(c);
                    }
                }
            }
            let current_right_corner = self.calc_door_corner(&info, false)?;
            let current_distance = check_line.segment_distance(&current_right_corner);
            if current_distance.abs() < trace_halfwidth_middle {
                if let Some(c) = left_right_tangential_point(&check_line.a, &check_line.b, Some(&current_right_corner), trace_halfwidth_max) {
                    if corrected_result.is_none() || c.side_of(&cur, corrected_result.as_ref().unwrap()) == Side::OnTheLeft {
                        corrected_door_ind = i;
                        corrected_result = Some(c);
                    }
                }
            }
        }
        if let Some(c) = corrected_result {
            result_corner = Some((c, false));
            new_door_ind = corrected_door_ind.max(self.current_to_door_index);
        }
        self.current_to_door_index = new_door_ind;
        if let Some((p, is_current)) = result_corner {
            if !is_current {
                result.push(Corner::New(p));
            }
        }
        Ok(result)
    }
}

const C_TOLERANCE: f64 = 1.0;

/// Java `FloatPoint.rightTangentialPoint(toPoint, distance)` (`null` for a `null` point).
fn right_tangential_point(from: &FloatPoint, to: Option<&FloatPoint>, distance: f64) -> Option<FloatPoint> {
    from.right_tangential_point(to?, distance)
}

/// Java `FloatPoint.leftTangentialPoint(toPoint, distance)` (`null` for a `null` point).
fn left_tangential_point(from: &FloatPoint, to: Option<&FloatPoint>, distance: f64) -> Option<FloatPoint> {
    from.left_tangential_point(to?, distance)
}

/// Java `rightTurnNextCorner(fromCorner, dist, toCorner, nextCorner)`: `(point, is fromCorner)`.
fn right_turn_next_corner(from_corner: &FloatPoint, dist: f64, to_corner: &FloatPoint, next_corner: &FloatPoint) -> Option<(FloatPoint, bool)> {
    let Some(tp) = from_corner.left_tangential_point(to_corner, dist) else {
        log::trace!("FoundConnectionLocator.right_turn_next_corner: left tangential point is null");
        return Some((*from_corner, true));
    };
    let first_line = FloatLine::new(*from_corner, tp);
    let Some(tp) = to_corner.right_tangential_point(next_corner, 2.0 * dist + C_TOLERANCE) else {
        log::trace!("FoundConnectionLocator.right_turn_next_corner: right tangential point is null");
        return Some((*from_corner, true));
    };
    let second_line = FloatLine::new(*to_corner, tp).translate(dist);
    first_line.intersection(&second_line).map(|p| (p, false))
}

/// Java `leftTurnNextCorner(fromCorner, dist, toCorner, nextCorner)`: `(point, is fromCorner)`.
fn left_turn_next_corner(from_corner: &FloatPoint, dist: f64, to_corner: &FloatPoint, next_corner: &FloatPoint) -> Option<(FloatPoint, bool)> {
    let Some(tp) = from_corner.right_tangential_point(to_corner, dist) else {
        log::trace!("FoundConnectionLocator.left_turn_next_corner: right tangential point is null");
        return Some((*from_corner, true));
    };
    let first_line = FloatLine::new(*from_corner, tp);
    let Some(tp) = to_corner.left_tangential_point(next_corner, 2.0 * dist + C_TOLERANCE) else {
        log::trace!("FoundConnectionLocator.left_turn_next_corner: left tangential point is null");
        return Some((*from_corner, true));
    };
    let second_line = FloatLine::new(*to_corner, tp).translate(-dist);
    first_line.intersection(&second_line).map(|p| (p, false))
}

/// Java `rightLeftTangentialPoint(fromPoint, toPoint, center, dist)`.
fn right_left_tangential_point(from_point: &FloatPoint, to_point: &FloatPoint, center: Option<&FloatPoint>, dist: f64) -> Option<FloatPoint> {
    let tp = right_tangential_point(from_point, center, dist)?;
    let first_line = FloatLine::new(*from_point, tp);
    let tp = left_tangential_point(to_point, center, dist)?;
    let second_line = FloatLine::new(*to_point, tp);
    first_line.intersection(&second_line)
}

/// Java `leftRightTangentialPoint(fromPoint, toPoint, center, dist)`.
fn left_right_tangential_point(from_point: &FloatPoint, to_point: &FloatPoint, center: Option<&FloatPoint>, dist: f64) -> Option<FloatPoint> {
    let tp = left_tangential_point(from_point, center, dist)?;
    let first_line = FloatLine::new(*from_point, tp);
    let tp = right_tangential_point(to_point, center, dist)?;
    let second_line = FloatLine::new(*to_point, tp);
    first_line.intersection(&second_line)
}
