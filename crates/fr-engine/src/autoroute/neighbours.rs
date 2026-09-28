//! Port of `autoroute/expansion/SortedRoomNeighbours.java` (any angle routing): calculates the
//! neighbours of an expansion room sorted counterclockwise around its border, the doors to them
//! and the new incomplete rooms in the free space between them.

use std::cell::OnceCell;
use std::cmp::Ordering;

use fr_geom::{ConvexShape, Direction, IntPoint, Line, Point, Side, Signum, Simplex, TileShape};
use fr_jcompat::JavaTreeSet;

use crate::board::{RoutingBoard, TreeEntry, TreeObject};

use super::engine::{AutorouteEngine, FromRoom, RoomId, RoomKind};
use super::rooms::IncompleteFreeSpaceExpansionRoom;
use super::JResult;

/// Java `SortedRoomNeighbour`.
struct Neighbour {
    object_id: i32,
    neighbour_shape: TileShape,
    touching_side_no_of_room: i32,
    touching_side_no_of_neighbour_room: i32,
    room_touch_is_corner: bool,
    neighbour_room_touch_is_corner: bool,
    first_corner: OnceCell<Point>,
    last_corner: OnceCell<Point>,
}

impl Neighbour {
    /// Java `firstCorner()`.
    fn first_corner(&self, room_shape: &TileShape) -> &Point {
        self.first_corner.get_or_init(|| {
            if self.room_touch_is_corner {
                room_shape.corner(self.touching_side_no_of_room)
            } else if self.neighbour_room_touch_is_corner {
                self.neighbour_shape.corner(self.touching_side_no_of_neighbour_room)
            } else {
                let current_first_corner = self.neighbour_shape.corner(self.neighbour_shape.next_no(self.touching_side_no_of_neighbour_room));
                let prev_line = room_shape.border_line(room_shape.prev_no(self.touching_side_no_of_room));
                if prev_line.side_of(&current_first_corner) == Side::OnTheRight {
                    current_first_corner
                } else {
                    // currentFirstCorner is outside the door shape
                    room_shape.corner(self.touching_side_no_of_room)
                }
            }
        })
    }

    /// Java `lastCorner()`.
    fn last_corner(&self, room_shape: &TileShape) -> &Point {
        self.last_corner.get_or_init(|| {
            if self.room_touch_is_corner {
                room_shape.corner(self.touching_side_no_of_room)
            } else if self.neighbour_room_touch_is_corner {
                self.neighbour_shape.corner(self.touching_side_no_of_neighbour_room)
            } else {
                let current_last_corner = self.neighbour_shape.corner(self.touching_side_no_of_neighbour_room);
                let next_line = room_shape.border_line(room_shape.next_no(self.touching_side_no_of_room));
                if next_line.side_of(&current_last_corner) == Side::OnTheRight {
                    current_last_corner
                } else {
                    // currentLastCorner is outside the door shape
                    room_shape.corner(room_shape.next_no(self.touching_side_no_of_room))
                }
            }
        })
    }
}

const C_DIST_TOLERANCE: f64 = 1.0;

/// Java `SortedRoomNeighbour.compareTo`.
fn compare(a: &Neighbour, b: &Neighbour, room_shape: &TileShape) -> Ordering {
    let compare_value = a.touching_side_no_of_room.wrapping_sub(b.touching_side_no_of_room);
    if compare_value != 0 {
        return compare_value.cmp(&0);
    }
    let compare_corner = room_shape.corner_approx(a.touching_side_no_of_room);
    let this_distance = a.first_corner(room_shape).to_float().distance(&compare_corner);
    let other_distance = b.first_corner(room_shape).to_float().distance(&compare_corner);
    let mut delta_distance = this_distance - other_distance;
    if delta_distance.abs() <= C_DIST_TOLERANCE {
        // check corners for equality
        if a.first_corner(room_shape) == b.first_corner(room_shape) {
            // in this case compare the last corners
            let this_distance2 = a.last_corner(room_shape).to_float().distance(&compare_corner);
            let other_distance2 = b.last_corner(room_shape).to_float().distance(&compare_corner);
            delta_distance = this_distance2 - other_distance2;
            if delta_distance.abs() <= C_DIST_TOLERANCE && a.neighbour_room_touch_is_corner && b.neighbour_room_touch_is_corner {
                // Otherwise there may be a short 1 dim. touch at a link between 2 trace lines.
                let mut compare_line_no = a.touching_side_no_of_room;
                if a.room_touch_is_corner {
                    compare_line_no = room_shape.prev_no(compare_line_no);
                }
                let compare_dir = room_shape.border_line(compare_line_no).direction().opposite();
                let this_compare_line = a.neighbour_shape.border_line(a.touching_side_no_of_neighbour_room);
                let other_compare_line = b.neighbour_shape.border_line(b.touching_side_no_of_neighbour_room);
                delta_distance = compare_dir.compare_from(&this_compare_line.direction(), &other_compare_line.direction()) as f64;
            }
        }
    }
    let mut res = Signum::as_int(delta_distance);
    if res == 0 {
        // Deterministic tie-breaker for identical geometry
        res = a.object_id.wrapping_sub(b.object_id);
    }
    res.cmp(&0)
}

/// Java `SortedRoomNeighbours` (the per room state).
struct SortedRoomNeighbours {
    completed_room: RoomId,
    /// Java `roomShape` (the shape of the completed room when the neighbours were calculated).
    room_shape: TileShape,
    neighbours: Vec<Neighbour>,
    sorted: JavaTreeSet<usize>,
    own_net_objects: Vec<TreeEntry>,
}

impl SortedRoomNeighbours {
    fn add_sorted_neighbour(&mut self, n: Neighbour) {
        let index = self.neighbours.len();
        self.neighbours.push(n);
        let neighbours = &self.neighbours;
        let room_shape = &self.room_shape;
        self.sorted.add_by(index, |a, b| compare(&neighbours[*a], &neighbours[*b], room_shape));
    }

    fn sorted_list(&self) -> Vec<usize> {
        self.sorted.iter().copied().collect()
    }
}

/// Java `SortedRoomNeighbours.calculate(room, autorouteEngine)`.
pub(crate) fn calculate(eng: &mut AutorouteEngine, board: &mut RoutingBoard, mut room: FromRoom<'_>) -> JResult<Option<RoomId>> {
    let net_number = eng.net_number();
    loop {
        let room_id_no = eng.generate_room_id_no();
        let rn = calculate_neighbours(eng, board, &room, net_number, room_id_no);
        // Check, that each side of the room shape has at least one touching neighbour.
        // Otherwise, improve the room shape by enlarging.
        let edge_removed = try_remove_edge(&rn, &mut room, board, eng.tree, net_number);
        let result = rn.completed_room;
        if edge_removed {
            eng.remove_all_doors(result)?;
            continue;
        }
        // Now calculate the new incomplete rooms together with the doors between this room and
        // the sorted neighbours.
        if rn.sorted.is_empty() {
            if let FromRoom::Obstacle(obstacle_room) = room {
                calculate_incomplete_rooms_with_empty_neighbours(eng, board, obstacle_room);
            }
        } else {
            calculate_new_incomplete_rooms(&rn, eng, &room);
            if eng.room(result).shape().dimension() < 2 {
                log::trace!("AutorouteEngine.calculate_new_incomplete_rooms_with_more_than_1_neighbour: unexpected dimension for smoothened_shape");
            }
        }
        if eng.room(result).is_complete_free() {
            calculate_target_doors(eng, board, result, &rn.own_net_objects);
        }
        return Ok(Some(result));
    }
}

fn calculate_incomplete_rooms_with_empty_neighbours(eng: &mut AutorouteEngine, board: &RoutingBoard, room: RoomId) {
    let room_shape = eng.room(room).shape().clone();
    let layer = eng.room(room).layer;
    for i in 0..room_shape.border_line_count() {
        let current_line = room_shape.border_line(i);
        if eng.insert_door_ok_obstacle(board, room, Some(&current_line)) {
            let new_room_shape = TileShape::Simplex(Simplex::new(vec![current_line.opposite()]));
            let new_contained_shape = room_shape.intersection(&new_room_shape);
            let new_room = eng.add_incomplete_expansion_room(Some(new_room_shape), layer, Some(new_contained_shape));
            let new_door = eng.new_door(room, new_room, 1);
            eng.add_door(room, new_door);
            eng.add_door(new_room, new_door);
        }
    }
}

fn calculate_target_doors(eng: &mut AutorouteEngine, board: &RoutingBoard, room: RoomId, own_net_objects: &[TreeEntry]) {
    if !own_net_objects.is_empty() {
        eng.set_net_dependent(room);
    }
    let net_number = eng.net_number();
    for entry in own_net_objects {
        eng.add_target_door_if_connected(board, room, entry.object, entry.shape_index, net_number);
    }
}

/// Sorts the overlapping tree entries by object id and shape index (Java `List.sort`, stable).
pub(crate) fn sort_entries(entries: &mut [TreeEntry]) {
    entries.sort_by(|e1, e2| {
        let id_diff = e1.object.id().wrapping_sub(e2.object.id());
        if id_diff != 0 {
            return id_diff.cmp(&0);
        }
        e1.shape_index.wrapping_sub(e2.shape_index).cmp(&0)
    });
}

/// The shape of the room whose neighbours are calculated (Java `room.getShape()`).
pub(crate) fn from_room_shape(eng: &AutorouteEngine, room: &FromRoom<'_>) -> TileShape {
    match room {
        FromRoom::Incomplete(r) => r.shape.clone().expect("calculate_neighbours: room shape is null"),
        FromRoom::Obstacle(r) => eng.room(*r).shape().clone(),
    }
}

/// The layer of the room whose neighbours are calculated.
pub(crate) fn from_room_layer(eng: &AutorouteEngine, room: &FromRoom<'_>) -> i32 {
    match room {
        FromRoom::Incomplete(r) => r.layer,
        FromRoom::Obstacle(r) => eng.room(*r).layer,
    }
}

/// Java `completedRoom` creation of `calculateNeighbours`.
pub(crate) fn completed_room_of(eng: &mut AutorouteEngine, room: &FromRoom<'_>, room_shape: &TileShape, room_id_no: i32) -> RoomId {
    match room {
        FromRoom::Incomplete(r) => eng.new_complete_room(room_shape.clone(), r.layer, room_id_no),
        FromRoom::Obstacle(r) => *r,
    }
}

fn calculate_neighbours(eng: &mut AutorouteEngine, board: &mut RoutingBoard, room: &FromRoom<'_>, net_number: i32, room_id_no: i32) -> SortedRoomNeighbours {
    let room_shape = from_room_shape(eng, room);
    let layer = from_room_layer(eng, room);
    let completed_room = completed_room_of(eng, room, &room_shape, room_id_no);
    let is_incomplete = matches!(room, FromRoom::Incomplete(_));
    let mut result = SortedRoomNeighbours {
        completed_room,
        room_shape: eng.room(completed_room).shape().clone(),
        neighbours: Vec::new(),
        sorted: JavaTreeSet::new(fr_jcompat::treemap::NaturalOrder),
        own_net_objects: Vec::new(),
    };
    let mut overlapping_objects = board.overlapping_tree_entries_list(eng.tree, &ConvexShape::Tile(room_shape.clone()), layer, &[]);
    sort_entries(&mut overlapping_objects);
    // Calculate the touching neighbour objects and sort them in counterclock sense around the
    // border of the room shape.
    for current_entry in overlapping_objects {
        let object = current_entry.object;
        if is_incomplete && !board.object_is_trace_obstacle(object, net_number) {
            // delay processing the target doors until the room shape will not change anymore
            result.own_net_objects.push(current_entry);
            continue;
        }
        let current_shape = board.object_tree_shape(eng.tree, object, current_entry.shape_index).expect("tree shape is null");
        let intersection = room_shape.intersection(&current_shape);
        let dimension = intersection.dimension();
        if dimension > 1 {
            if eng.room(completed_room).is_obstacle() {
                if let TreeObject::Item { key, .. } = object {
                    // only Obstacle expansion room may have a 2-dim overlap
                    if board.item(key).is_routable() {
                        if let Some(overlap_room) = eng.get_expansion_room(board, key, current_entry.shape_index) {
                            eng.create_overlap_door(board, completed_room, overlap_room);
                        }
                    }
                }
            } else {
                log::trace!("SortedRoomNeighbours.calculate: unexpected area overlap of free space expansion room");
            }
            continue;
        }
        if dimension < 0 {
            log::debug!("SortedRoomNeighbours.calculate: dimension >= 0 expected");
            continue;
        }
        if dimension == 1 {
            let touching_sides = room_shape.touching_sides(&current_shape);
            if touching_sides.len() != 2 {
                log::debug!("SortedRoomNeighbours.calculate: touchingSides length 2 expected");
                continue;
            }
            result.add_sorted_neighbour(Neighbour {
                object_id: object.id(),
                neighbour_shape: current_shape.clone(),
                touching_side_no_of_room: touching_sides[0],
                touching_side_no_of_neighbour_room: touching_sides[1],
                room_touch_is_corner: false,
                neighbour_room_touch_is_corner: false,
                first_corner: OnceCell::new(),
                last_corner: OnceCell::new(),
            });
            // make sure, that there is a door to the neighbour room.
            let neighbour_room = neighbour_room_of(eng, board, object, current_entry.shape_index);
            if let Some(neighbour_room) = neighbour_room {
                if eng.insert_door_ok(board, completed_room, neighbour_room, &intersection) {
                    let new_door = eng.new_door(completed_room, neighbour_room, 1);
                    eng.add_door(neighbour_room, new_door);
                    eng.add_door(completed_room, new_door);
                }
            }
        } else {
            // dimension = 0
            let touching_point = intersection.corner(0);
            let room_corner_no = room_shape.equals_corner(&touching_point);
            let (room_touch_is_corner, touching_side_no_of_room) = if room_corner_no >= 0 {
                (true, room_corner_no)
            } else {
                let no = room_shape.contains_on_border_line_no(&touching_point);
                if no < 0 {
                    log::debug!("SortedRoomNeighbours.calculate: touchingSideNoOfRoom >= 0 expected");
                }
                (false, no)
            };
            let neighbour_room_corner_no = current_shape.equals_corner(&touching_point);
            let (neighbour_room_touch_is_corner, touching_side_no_of_neighbour_room) = if neighbour_room_corner_no >= 0 {
                // The previous border line is preferred to make the shape of the incomplete room
                // as big as possible
                (true, current_shape.prev_no(neighbour_room_corner_no))
            } else {
                let no = current_shape.contains_on_border_line_no(&touching_point);
                if no < 0 {
                    log::debug!("SortedRoomNeighbours.calculate: touchingSideNoOfNeighbourRoom >= 0 expected");
                }
                (false, no)
            };
            result.add_sorted_neighbour(Neighbour {
                object_id: object.id(),
                neighbour_shape: current_shape.clone(),
                touching_side_no_of_room,
                touching_side_no_of_neighbour_room,
                room_touch_is_corner,
                neighbour_room_touch_is_corner,
                first_corner: OnceCell::new(),
                last_corner: OnceCell::new(),
            });
        }
    }
    result
}

/// The expansion room of a neighbour tree object (Java: the object itself if it is a room, the
/// obstacle room of a routable item, else `null`).
pub(crate) fn neighbour_room_of(eng: &mut AutorouteEngine, board: &RoutingBoard, object: TreeObject, shape_index: i32) -> Option<RoomId> {
    match object {
        TreeObject::Room { key, .. } => Some(RoomId(key.0)),
        TreeObject::Item { key, .. } => {
            if board.item(key).is_routable() {
                // expand the item for ripup and pushing purposes
                eng.get_expansion_room(board, key, shape_index)
            } else {
                None
            }
        }
    }
}

/// Java `tryRemoveEdge(netNumber, autorouteSearchTree)`.
fn try_remove_edge(rn: &SortedRoomNeighbours, room: &mut FromRoom<'_>, board: &RoutingBoard, tree: usize, net_number: i32) -> bool {
    let FromRoom::Incomplete(current_incomplete_room) = room else {
        return false;
    };
    let mut remove_edge_no = -1;
    let room_simplex = current_incomplete_room.shape.as_ref().unwrap().to_simplex();
    let room_shape_area = TileShape::Simplex(room_simplex.clone()).area();
    let mut prev_edge_no = -1;
    let mut current_edge_no = 0;
    for &n in rn.sorted.iter() {
        let next_neighbour = &rn.neighbours[n];
        if next_neighbour.touching_side_no_of_room == prev_edge_no {
            continue;
        }
        if next_neighbour.touching_side_no_of_room == current_edge_no {
            prev_edge_no = current_edge_no;
            current_edge_no += 1;
        } else {
            // On the edge side with index currentEdgeNo is no touching neighbour.
            remove_edge_no = current_edge_no;
            break;
        }
    }
    let simplex_tile = TileShape::Simplex(room_simplex.clone());
    if remove_edge_no < 0 && current_edge_no < simplex_tile.border_line_count() {
        // missing touching neighbour at the last edge side.
        remove_edge_no = current_edge_no;
    }
    if remove_edge_no >= 0 {
        // Touching neighbour missing at the edge side with index removeEdgeNo.
        // Remove the edge line and restart the algorithm.
        let enlarged_shape = room_simplex.remove_border_line(remove_edge_no);
        let enlarged_room = IncompleteFreeSpaceExpansionRoom::new(
            Some(TileShape::Simplex(enlarged_shape)),
            current_incomplete_room.layer,
            current_incomplete_room.contained_shape.clone(),
        );
        let new_rooms = board.complete_shape(tree, &enlarged_room, net_number, None, None);
        if new_rooms.len() != 1 {
            log::trace!("AutorouteEngine.calculate_doors: 1 completed shape expected");
            return false;
        }
        // Check, that the area increases to prevent endless loop.
        let new_room = new_rooms.into_iter().next().unwrap();
        if new_room.shape.as_ref().unwrap().area() > room_shape_area {
            current_incomplete_room.shape = new_room.shape;
            current_incomplete_room.contained_shape = new_room.contained_shape;
            return true;
        }
    }
    false
}

/// Java `calculateNewIncompleteRooms(autorouteEngine)`.
fn calculate_new_incomplete_rooms(rn: &SortedRoomNeighbours, eng: &mut AutorouteEngine, from_room: &FromRoom<'_>) {
    let sorted = rn.sorted_list();
    let last_index = *sorted.last().unwrap();
    let room_shape = rn.room_shape.clone();
    let mut prev = last_index;
    let from_shape = from_room_shape(eng, from_room);
    let room_simplex = TileShape::Simplex(from_shape.to_simplex());
    let from_layer = from_room_layer(eng, from_room);
    let incomplete_contained = match from_room {
        FromRoom::Incomplete(r) => Some(r.contained_shape.clone()),
        FromRoom::Obstacle(_) => None,
    };
    let completed_room = rn.completed_room;
    for &next in &sorted {
        let prev_n = &rn.neighbours[prev];
        let next_n = &rn.neighbours[next];
        let mut first_touching_side_no = prev_n.touching_side_no_of_room;
        let mut last_touching_side_no = next_n.touching_side_no_of_room;
        let current_next_no = room_simplex.next_no(first_touching_side_no);
        let intersection_with_prev_neighbour_ends_at_corner = (first_touching_side_no != last_touching_side_no || prev == last_index)
            && *prev_n.last_corner(&room_shape) == room_simplex.corner(current_next_no);
        let intersection_with_next_neighbour_starts_at_corner = (first_touching_side_no != last_touching_side_no || prev == last_index)
            && *next_n.first_corner(&room_shape) == room_simplex.corner(last_touching_side_no);
        if intersection_with_prev_neighbour_ends_at_corner {
            first_touching_side_no = current_next_no;
        }
        if intersection_with_next_neighbour_starts_at_corner {
            last_touching_side_no = room_simplex.prev_no(last_touching_side_no);
        }
        let mut neighbours_touch = false;
        if rn.sorted.len() > 1 {
            neighbours_touch = prev_n.last_corner(&room_shape) == next_n.first_corner(&room_shape);
        }
        if !neighbours_touch {
            // create a door to a new incomplete expansion room between the last corner of the
            // previous neighbour and the first corner of the current neighbour.
            let mut last_bounding_line_no = prev_n.touching_side_no_of_neighbour_room;
            if !(intersection_with_prev_neighbour_ends_at_corner || prev_n.room_touch_is_corner) {
                last_bounding_line_no = prev_n.neighbour_shape.prev_no(last_bounding_line_no);
            }
            let mut first_bounding_line_no = next_n.touching_side_no_of_neighbour_room;
            if !(intersection_with_next_neighbour_starts_at_corner || next_n.neighbour_room_touch_is_corner) {
                first_bounding_line_no = next_n.neighbour_shape.next_no(first_bounding_line_no);
            }
            let mut start_edge_line: Option<Line> = Some(next_n.neighbour_shape.border_line(first_bounding_line_no).opposite());
            // startEdgeLine is only used for the first new incomplete room.
            let mut middle_edge_line: Option<Line> = None;
            let mut current_touching_side_no = last_touching_side_no;
            let mut first_time = true;
            // The loop goes backwards from the edge line of nextNeighbour to the edge line of
            // prevNeighbour.
            loop {
                let mut corner_cut_off = false;
                if let Some(contained) = &incomplete_contained {
                    if current_touching_side_no == last_touching_side_no && first_touching_side_no != last_touching_side_no {
                        // Create a new line approximately from the last corner of the previous
                        // neighbour to the first corner of the next neighbour to cut off the
                        // outstanding corners of the room shape in the empty space.
                        let cut_line_start: IntPoint = prev_n.last_corner(&room_shape).to_float().round();
                        let cut_line_end: IntPoint = next_n.first_corner(&room_shape).to_float().round();
                        let cut_line = Line::from_int_points(cut_line_start, cut_line_end);
                        let cut_half_plane = TileShape::get_instance_line(&cut_line);
                        let new_shape = eng.room(completed_room).shape().intersection(&cut_half_plane);
                        if let RoomKind::Complete { shape, .. } = &mut eng.room_mut(completed_room).kind {
                            *shape = new_shape;
                        }
                        // Otherwise room.containedShape would no longer be contained in the shape
                        // after cutting of the corner.
                        corner_cut_off = contained.as_ref().expect("contained shape is null").side_of(&cut_line) == Side::OnTheLeft;
                        if corner_cut_off {
                            middle_edge_line = Some(cut_line.opposite());
                        }
                    }
                }
                let next_touching_side_no = room_simplex.prev_no(current_touching_side_no);
                if !corner_cut_off {
                    middle_edge_line = Some(room_simplex.border_line(current_touching_side_no).opposite());
                }
                let middle = middle_edge_line.clone().unwrap();
                let middle_line_dir: Direction = middle.direction();
                let last_time =
                    current_touching_side_no == first_touching_side_no && !(prev == last_index && first_time) || corner_cut_off;
                let end_edge_line: Option<Line> = if last_time {
                    let l = prev_n.neighbour_shape.border_line(last_bounding_line_no).opposite();
                    if l.direction().side_of(&middle_line_dir) != Side::OnTheLeft {
                        // Concave corner between the middle and the last line.
                        None
                    } else {
                        Some(l)
                    }
                } else {
                    None
                };
                if let Some(s) = &start_edge_line {
                    if middle_line_dir.side_of(&s.direction()) != Side::OnTheLeft {
                        // concave corner between the first and the middle line
                        start_edge_line = None;
                    }
                }
                let mut new_edge_lines = Vec::with_capacity(3);
                if let Some(s) = &start_edge_line {
                    new_edge_lines.push(s.clone());
                }
                new_edge_lines.push(middle);
                if let Some(e) = &end_edge_line {
                    new_edge_lines.push(e.clone());
                }
                let new_room_shape = Simplex::get_instance(&new_edge_lines);
                if !new_room_shape.is_empty() {
                    let new_room_tile = TileShape::Simplex(new_room_shape);
                    let new_contained_shape = eng.room(completed_room).shape().intersection(&new_room_tile);
                    if !new_contained_shape.is_empty() {
                        let new_room = eng.add_incomplete_expansion_room(Some(new_room_tile), from_layer, Some(new_contained_shape));
                        let new_door = eng.new_door(completed_room, new_room, 1);
                        eng.add_door(completed_room, new_door);
                        eng.add_door(new_room, new_door);
                    }
                }
                if last_time {
                    break;
                }
                current_touching_side_no = next_touching_side_no;
                start_edge_line = None;
                first_time = false;
            }
        }
        prev = next;
    }
}

