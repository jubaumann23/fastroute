//! Port of `autoroute/expansion/SortedOrthogonalRoomNeighbours.java`.

use std::cmp::Ordering;

use fr_geom::limits::CRIT_INT;
use fr_geom::{ConvexShape, IntBox, TileShape};
use fr_jcompat::JavaTreeSet;

use crate::board::{RoutingBoard, TreeObject};

use super::engine::{AutorouteEngine, FromRoom, RoomId};
use super::neighbours::{completed_room_of, from_room_layer, from_room_shape, neighbour_room_of, sort_entries};
use super::rooms::IncompleteFreeSpaceExpansionRoom;
use super::JResult;

/// Java `SortedOrthogonalRoomNeighbours.SortedRoomNeighbour`.
struct Neighbour {
    object_id: i32,
    intersection: IntBox,
    first_touching_side: i32,
    last_touching_side: i32,
}

struct Sorted90 {
    completed_room: RoomId,
    is_obstacle_expansion_room: bool,
    room_shape: IntBox,
    edge_interior_touches_obstacle: [bool; 4],
    neighbours: Vec<Neighbour>,
    sorted: JavaTreeSet<usize>,
}

/// Java `SortedRoomNeighbour.compareTo`.
fn compare(a: &Neighbour, b: &Neighbour) -> Ordering {
    if a.first_touching_side > b.first_touching_side {
        return Ordering::Greater;
    }
    if a.first_touching_side < b.first_touching_side {
        return Ordering::Less;
    }
    let is1 = &a.intersection;
    let is2 = &b.intersection;
    let mut cmp_value = match a.first_touching_side {
        0 => is1.ll.x.wrapping_sub(is2.ll.x),
        1 => is1.ll.y.wrapping_sub(is2.ll.y),
        2 => is2.ur.x.wrapping_sub(is1.ur.x),
        3 => is2.ur.y.wrapping_sub(is1.ur.y),
        _ => {
            log::warn!("SortedRoomNeighbour.compareTo: firstTouchingSide out of range ");
            return Ordering::Equal;
        }
    };
    if cmp_value == 0 {
        let this_diff = (a.last_touching_side - a.first_touching_side + 4) % 4;
        let other_diff = (b.last_touching_side - b.first_touching_side + 4) % 4;
        if this_diff > other_diff {
            return Ordering::Greater;
        }
        if this_diff < other_diff {
            return Ordering::Less;
        }
        cmp_value = match a.last_touching_side {
            0 => is1.ur.x.wrapping_sub(is2.ur.x),
            1 => is1.ur.y.wrapping_sub(is2.ur.y),
            2 => is2.ll.x.wrapping_sub(is1.ll.x),
            3 => is2.ll.y.wrapping_sub(is1.ll.y),
            _ => {
                log::warn!("SortedRoomNeighbour.compareTo: firstTouchingSide out of range ");
                return Ordering::Equal;
            }
        };
    }
    if cmp_value == 0 {
        // Deterministic tie-breaker for identical geometry
        cmp_value = a.object_id.wrapping_sub(b.object_id);
    }
    cmp_value.cmp(&0)
}

impl Sorted90 {
    fn add_sorted_neighbour(&mut self, object: TreeObject, intersection: IntBox) {
        let r = self.room_shape;
        let is = intersection;
        if is.ll.y == r.ll.y && is.ur.x > r.ll.x && is.ll.x < r.ur.x {
            self.edge_interior_touches_obstacle[0] = true;
        }
        if is.ur.x == r.ur.x && is.ur.y > r.ll.y && is.ll.y < r.ur.y {
            self.edge_interior_touches_obstacle[1] = true;
        }
        if is.ur.y == r.ur.y && is.ur.x > r.ll.x && is.ll.x < r.ur.x {
            self.edge_interior_touches_obstacle[2] = true;
        }
        if is.ll.x == r.ll.x && is.ur.y > r.ll.y && is.ll.y < r.ur.y {
            self.edge_interior_touches_obstacle[3] = true;
        }
        let first_touching_side = if is.ll.y == r.ll.y && is.ll.x > r.ll.x {
            0
        } else if is.ur.x == r.ur.x && is.ll.y > r.ll.y {
            1
        } else if is.ur.y == r.ur.y {
            2
        } else if is.ll.x == r.ll.x {
            3
        } else {
            log::warn!("SortedRoomNeighbour: case not expected");
            -1
        };
        let last_touching_side = if is.ll.x == r.ll.x && is.ll.y > r.ll.y {
            3
        } else if is.ur.y == r.ur.y && is.ll.x > r.ll.x {
            2
        } else if is.ur.x == r.ur.x {
            1
        } else if is.ll.y == r.ll.y {
            0
        } else {
            log::warn!("SortedRoomNeighbour: case not expected");
            -1
        };
        let index = self.neighbours.len();
        self.neighbours.push(Neighbour { object_id: object.id(), intersection, first_touching_side, last_touching_side });
        let neighbours = &self.neighbours;
        self.sorted.add_by(index, |a, b| compare(&neighbours[*a], &neighbours[*b]));
    }
}

/// Java `SortedOrthogonalRoomNeighbours.calculate(room, autorouteEngine)`.
pub(crate) fn calculate(eng: &mut AutorouteEngine, board: &mut RoutingBoard, mut room: FromRoom<'_>) -> JResult<Option<RoomId>> {
    let net_number = eng.net_number();
    loop {
        let room_id_no = eng.generate_room_id_no();
        let Some(rn) = calculate_neighbours(eng, board, &room, net_number, room_id_no) else {
            return Ok(None);
        };
        let edge_removed = try_remove_edge(&rn, eng, &mut room, board, net_number);
        let result = rn.completed_room;
        if edge_removed {
            eng.remove_all_doors(result)?;
            continue;
        }
        if rn.sorted.is_empty() {
            if let FromRoom::Obstacle(obstacle_room) = room {
                calculate_incomplete_rooms_with_empty_neighbours(eng, board, obstacle_room);
            }
        } else {
            calculate_new_incomplete_rooms(&rn, eng, board, &room);
        }
        return Ok(Some(result));
    }
}

fn calculate_incomplete_rooms_with_empty_neighbours(eng: &mut AutorouteEngine, board: &RoutingBoard, room: RoomId) {
    let TileShape::IntBox(room_box) = eng.room(room).shape().clone() else {
        log::warn!("SortedOrthoganelRoomNeighbours.calculate_incomplete_rooms_with_empty_neighbours: IntBox expected for roomShape");
        return;
    };
    let layer = eng.room(room).layer;
    let bb = board.bounding_box;
    for i in 0..4 {
        let new_room_box = match i {
            0 => IntBox::new(bb.ll.x, bb.ll.y, bb.ur.x, room_box.ll.y),
            1 => IntBox::new(room_box.ur.x, bb.ll.y, bb.ur.x, bb.ur.y),
            2 => IntBox::new(bb.ll.x, room_box.ur.y, bb.ur.x, bb.ur.y),
            _ => IntBox::new(bb.ll.x, bb.ll.y, room_box.ll.x, bb.ur.y),
        };
        let new_contained_box = room_box.intersection_int_box(&new_room_box);
        let new_room = eng.add_incomplete_expansion_room(Some(TileShape::IntBox(new_room_box)), layer, Some(TileShape::IntBox(new_contained_box)));
        let new_door = eng.new_door(room, new_room, 1);
        eng.add_door(room, new_door);
        eng.add_door(new_room, new_door);
    }
}

fn calculate_neighbours(eng: &mut AutorouteEngine, board: &mut RoutingBoard, room: &FromRoom<'_>, net_number: i32, room_id_no: i32) -> Option<Sorted90> {
    let room_shape = from_room_shape(eng, room);
    let TileShape::IntBox(room_box) = room_shape else {
        log::warn!("SortedOrthogonalRoomNeighbours.calculate: IntBox expected for roomShape");
        return None;
    };
    let layer = from_room_layer(eng, room);
    let completed_room = completed_room_of(eng, room, &room_shape, room_id_no);
    let TileShape::IntBox(completed_box) = eng.room(completed_room).shape().clone() else {
        panic!("SortedOrthogonalRoomNeighbours: IntBox expected (ClassCastException)");
    };
    let mut result = Sorted90 {
        completed_room,
        is_obstacle_expansion_room: matches!(room, FromRoom::Obstacle(_)),
        room_shape: completed_box,
        edge_interior_touches_obstacle: [false; 4],
        neighbours: Vec::new(),
        sorted: JavaTreeSet::new(fr_jcompat::treemap::NaturalOrder),
    };
    let completed_is_free = eng.room(completed_room).is_complete_free();
    let completed_is_obstacle = eng.room(completed_room).is_obstacle();
    let mut overlapping_objects = board.overlapping_tree_entries_list(eng.tree, &ConvexShape::Tile(room_shape.clone()), layer, &[]);
    sort_entries(&mut overlapping_objects);
    for current_entry in overlapping_objects {
        let object = current_entry.object;
        if completed_is_free && !board.object_is_trace_obstacle(object, net_number) {
            eng.set_net_dependent(completed_room);
            eng.add_target_door_if_connected(board, completed_room, object, current_entry.shape_index, net_number);
            continue;
        }
        let current_shape = board.object_tree_shape(eng.tree, object, current_entry.shape_index).expect("tree shape is null");
        let TileShape::IntBox(current_box) = current_shape else {
            log::warn!("OrthogonalAutorouteEngine:calculate_sorted_neighbours: IntBox expected for currentShape");
            return None;
        };
        let intersection = room_box.intersection_int_box(&current_box);
        let dimension = intersection.dimension();
        if dimension > 1 && completed_is_obstacle {
            if let TreeObject::Item { key, .. } = object {
                if board.item(key).is_routable() {
                    if let Some(overlap_room) = eng.get_expansion_room(board, key, current_entry.shape_index) {
                        eng.create_overlap_door(board, completed_room, overlap_room);
                    }
                }
            }
            continue;
        }
        if dimension < 0 {
            log::warn!("AutorouteEngine.calculate_doors: dimension >= 0 expected");
            continue;
        }
        result.add_sorted_neighbour(object, intersection);
        if dimension > 0 {
            if let Some(neighbour_room) = neighbour_room_of(eng, board, object, current_entry.shape_index) {
                if eng.insert_door_ok(board, completed_room, neighbour_room, &TileShape::IntBox(intersection)) {
                    let new_door = eng.new_door_auto(completed_room, neighbour_room);
                    eng.add_door(neighbour_room, new_door);
                    eng.add_door(completed_room, new_door);
                }
            }
        }
    }
    Some(result)
}

fn remove_border_line(room_box: &IntBox, remove_edge_no: i32) -> Option<IntBox> {
    let crit = CRIT_INT;
    match remove_edge_no {
        0 => Some(IntBox::new(room_box.ll.x, -crit, room_box.ur.x, room_box.ur.y)),
        1 => Some(IntBox::new(room_box.ll.x, room_box.ll.y, crit, room_box.ur.y)),
        2 => Some(IntBox::new(room_box.ll.x, room_box.ll.y, room_box.ur.x, crit)),
        3 => Some(IntBox::new(-crit, room_box.ll.y, room_box.ur.x, room_box.ur.y)),
        _ => {
            log::warn!("SortedOrthogonalRoomNeighbours.removeBorderLine: illegal removeEdgeNo");
            None
        }
    }
}

/// Java `tryRemoveEdge(netNumber, autorouteSearchTree)`.
fn try_remove_edge(rn: &Sorted90, eng: &AutorouteEngine, room: &mut FromRoom<'_>, board: &RoutingBoard, net_number: i32) -> bool {
    let FromRoom::Incomplete(current_incomplete_room) = room else {
        return false;
    };
    let Some(TileShape::IntBox(room_box)) = current_incomplete_room.shape.clone() else {
        log::warn!("SortedOrthogonalRoomNeighbours.try_remove_edge: IntBox expected for roomShape type");
        return false;
    };
    let room_area = room_box.area();
    let mut remove_edge_no = -1;
    for i in 0..4 {
        if !rn.edge_interior_touches_obstacle[i as usize] {
            remove_edge_no = i;
            break;
        }
    }
    if remove_edge_no < 0 {
        return false;
    }
    let enlarged_box = remove_border_line(&room_box, remove_edge_no).expect("remove_border_line");
    let mut ignore_shape: Option<TileShape> = None;
    let mut ignore_object: Option<TreeObject> = None;
    let mut max_door_area = 0.0;
    for &d in &eng.room(rn.completed_room).doors {
        if eng.door(d).dimension == 2 {
            if let Some(other_room) = eng.door_other_complete_room(d, rn.completed_room) {
                if eng.room(other_room).is_complete_free() {
                    let current_door_shape = eng.door_shape(d);
                    let current_door_area = current_door_shape.area();
                    if current_door_area > max_door_area {
                        max_door_area = current_door_area;
                        ignore_shape = Some(current_door_shape);
                        ignore_object = Some(eng.room_tree_object(other_room));
                    }
                }
            }
        }
    }
    let enlarged_room = IncompleteFreeSpaceExpansionRoom::new(
        Some(TileShape::IntBox(enlarged_box)),
        current_incomplete_room.layer,
        current_incomplete_room.contained_shape.clone(),
    );
    let new_rooms = board.complete_shape(eng.tree, &enlarged_room, net_number, ignore_object, ignore_shape.as_ref());
    if new_rooms.len() == 1 {
        let new_room = new_rooms.into_iter().next().unwrap();
        if new_room.shape.as_ref().unwrap().area() > room_area {
            current_incomplete_room.shape = new_room.shape;
            current_incomplete_room.contained_shape = new_room.contained_shape;
            return true;
        }
    }
    false
}

/// Java `insertIncompleteRoom(autorouteEngine, llX, llY, urX, urY)`.
fn insert_incomplete_room(rn: &Sorted90, eng: &mut AutorouteEngine, from_room: &FromRoom<'_>, ll_x: i32, ll_y: i32, ur_x: i32, ur_y: i32) {
    let new_shape = IntBox::new(ll_x, ll_y, ur_x, ur_y);
    if new_shape.dimension() == 2 {
        let new_contained_shape = rn.room_shape.intersection_int_box(&new_shape);
        if !new_contained_shape.is_empty() {
            let door_dimension = new_shape.intersection_int_box(&rn.room_shape).dimension();
            if door_dimension > 0 {
                let layer = from_room_layer(eng, from_room);
                let new_room = eng.add_incomplete_expansion_room(Some(TileShape::IntBox(new_shape)), layer, Some(TileShape::IntBox(new_contained_shape)));
                let new_door = eng.new_door(rn.completed_room, new_room, door_dimension);
                eng.add_door(rn.completed_room, new_door);
                eng.add_door(new_room, new_door);
            }
        }
    }
}

/// Java `calculateNewIncompleteRooms(autorouteEngine)`.
fn calculate_new_incomplete_rooms(rn: &Sorted90, eng: &mut AutorouteEngine, board: &RoutingBoard, from_room: &FromRoom<'_>) {
    let bb = board.bounding_box;
    let sorted: Vec<usize> = rn.sorted.iter().copied().collect();
    let mut prev = *sorted.last().unwrap();
    let room = rn.room_shape;
    for &next in &sorted {
        let prev_n = &rn.neighbours[prev];
        let next_n = &rn.neighbours[next];
        let p = prev_n.intersection;
        let n = next_n.intersection;
        if !n.intersects_int_box(&p) {
            // create a door to a new incomplete expansion room between the last corner of the
            // previous neighbour and the first corner of the current neighbour.
            match next_n.first_touching_side {
                0 => {
                    if prev_n.last_touching_side == 0 {
                        if p.ur.x < n.ll.x {
                            insert_incomplete_room(rn, eng, from_room, p.ur.x, bb.ll.y, n.ll.x, room.ll.y);
                        }
                    } else if p.ll.y > room.ll.y || n.ll.x > room.ll.x {
                        if rn.is_obstacle_expansion_room {
                            // no 2-dim doors between obstacle_expansion_rooms and free space rooms.
                            if prev_n.last_touching_side == 3 {
                                insert_incomplete_room(rn, eng, from_room, bb.ll.x, room.ll.y, room.ll.x, p.ll.y);
                            }
                            insert_incomplete_room(rn, eng, from_room, room.ll.x, bb.ll.y, n.ll.x, room.ll.y);
                        } else {
                            insert_incomplete_room(rn, eng, from_room, bb.ll.x, bb.ll.y, n.ll.x, p.ll.y);
                        }
                    }
                }
                1 => {
                    if prev_n.last_touching_side == 1 {
                        if p.ur.y < n.ll.y {
                            insert_incomplete_room(rn, eng, from_room, room.ur.x, p.ur.y, bb.ur.x, n.ll.y);
                        }
                    } else if p.ur.x < room.ur.x || n.ll.y > room.ll.y {
                        if rn.is_obstacle_expansion_room {
                            if prev_n.last_touching_side == 0 {
                                insert_incomplete_room(rn, eng, from_room, p.ur.x, bb.ll.y, room.ur.x, room.ll.y);
                            }
                            insert_incomplete_room(rn, eng, from_room, room.ur.x, room.ll.y, room.ur.x, n.ll.y);
                        } else {
                            insert_incomplete_room(rn, eng, from_room, p.ur.x, bb.ll.y, bb.ur.x, n.ll.y);
                        }
                    }
                }
                2 => {
                    if prev_n.last_touching_side == 2 {
                        if p.ll.x > n.ur.x {
                            insert_incomplete_room(rn, eng, from_room, n.ur.x, room.ur.y, p.ll.x, bb.ur.y);
                        }
                    } else if p.ur.y < room.ur.y || n.ur.x < room.ur.x {
                        if rn.is_obstacle_expansion_room {
                            if prev_n.last_touching_side == 1 {
                                insert_incomplete_room(rn, eng, from_room, room.ur.x, p.ur.y, bb.ur.x, room.ur.y);
                            }
                            insert_incomplete_room(rn, eng, from_room, n.ur.x, room.ur.y, room.ur.x, bb.ur.y);
                        } else {
                            insert_incomplete_room(rn, eng, from_room, n.ur.x, p.ur.y, bb.ur.x, bb.ur.y);
                        }
                    }
                }
                3 => {
                    if prev_n.last_touching_side == 3 {
                        if p.ll.y > n.ur.y {
                            insert_incomplete_room(rn, eng, from_room, bb.ll.x, n.ur.y, room.ll.x, p.ll.y);
                        }
                    } else if n.ur.y < room.ur.y || p.ll.x > room.ll.x {
                        if rn.is_obstacle_expansion_room {
                            if prev_n.last_touching_side == 2 {
                                insert_incomplete_room(rn, eng, from_room, room.ll.x, room.ur.y, p.ll.x, bb.ur.y);
                            }
                            insert_incomplete_room(rn, eng, from_room, bb.ll.x, n.ur.y, room.ll.x, room.ur.y);
                        } else {
                            insert_incomplete_room(rn, eng, from_room, bb.ll.x, n.ur.y, p.ll.x, bb.ur.y);
                        }
                    }
                }
                _ => log::warn!("SortedOrthogonalRoomNeighbour.calculate_new_incomplete: illegal touching side"),
            }
        }
        prev = next;
    }
}
