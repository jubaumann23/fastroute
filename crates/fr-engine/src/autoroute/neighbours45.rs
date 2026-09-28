//! Port of `autoroute/expansion/Sorted45DegreeRoomNeighbours.java`.

use std::cmp::Ordering;

use fr_geom::limits::CRIT_INT;
use fr_geom::{ConvexShape, IntOctagon, TileShape};
use fr_jcompat::JavaTreeSet;

use crate::board::{RoutingBoard, TreeObject};

use super::engine::{AutorouteEngine, FromRoom, RoomId};
use super::neighbours::{completed_room_of, from_room_layer, from_room_shape, neighbour_room_of, sort_entries};
use super::rooms::IncompleteFreeSpaceExpansionRoom;
use super::JResult;

/// Java `Sorted45DegreeRoomNeighbours.SortedRoomNeighbour`.
struct Neighbour {
    object_id: i32,
    intersection: IntOctagon,
    first_touching_side: i32,
    last_touching_side: i32,
}

struct Sorted45 {
    completed_room: RoomId,
    /// Java `roomShape` (bounding octagon of the completed room shape).
    room_shape: IntOctagon,
    edge_interior_touches_obstacle: [bool; 8],
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
    // now the first touch of this and other is at the same side
    let is1 = &a.intersection;
    let is2 = &b.intersection;
    let mut cmp_value = match a.first_touching_side {
        0 => is1.corner(0).x.wrapping_sub(is2.corner(0).x),
        1 => is1.corner(1).x.wrapping_sub(is2.corner(1).x),
        2 => is1.corner(2).y.wrapping_sub(is2.corner(2).y),
        3 => is1.corner(3).y.wrapping_sub(is2.corner(3).y),
        4 => is2.corner(4).x.wrapping_sub(is1.corner(4).x),
        5 => is2.corner(5).x.wrapping_sub(is1.corner(5).x),
        6 => is2.corner(6).y.wrapping_sub(is1.corner(6).y),
        7 => is2.corner(7).y.wrapping_sub(is1.corner(7).y),
        _ => {
            log::warn!("SortedRoomNeighbour.compareTo: firstTouchingSide out of range ");
            return Ordering::Equal;
        }
    };
    if cmp_value == 0 {
        // The first touching points of this neighbour and other with the room shape are equal.
        // Compare the last touching points.
        let this_diff = (a.last_touching_side - a.first_touching_side + 8) % 8;
        let other_diff = (b.last_touching_side - b.first_touching_side + 8) % 8;
        if this_diff > other_diff {
            return Ordering::Greater;
        }
        if this_diff < other_diff {
            return Ordering::Less;
        }
        // now the last touch of this and other is at the same side
        match a.last_touching_side {
            0 => cmp_value = is1.corner(1).x.wrapping_sub(is2.corner(1).x),
            1 => cmp_value = is1.corner(2).x.wrapping_sub(is2.corner(2).x),
            2 => cmp_value = is1.corner(3).y.wrapping_sub(is2.corner(3).y),
            3 => cmp_value = is1.corner(4).y.wrapping_sub(is2.corner(4).y),
            4 => cmp_value = is2.corner(5).x.wrapping_sub(is1.corner(5).x),
            5 => cmp_value = is2.corner(6).x.wrapping_sub(is1.corner(6).x),
            6 => cmp_value = is2.corner(7).y.wrapping_sub(is1.corner(7).y),
            7 => cmp_value = is2.corner(0).y.wrapping_sub(is1.corner(0).y),
            _ => {}
        }
    }
    if cmp_value == 0 {
        // Deterministic tie-breaker for identical geometry
        cmp_value = a.object_id.wrapping_sub(b.object_id);
    }
    cmp_value.cmp(&0)
}

impl Sorted45 {
    /// Java `addSortedNeighbour` including the `SortedRoomNeighbour` constructor (which marks the
    /// touched edges of the room).
    fn add_sorted_neighbour(&mut self, object: TreeObject, intersection: IntOctagon) {
        let room = self.room_shape;
        let first_touching_side = if intersection.bottom_y == room.bottom_y && intersection.lower_left_diagonal_x > room.lower_left_diagonal_x {
            0
        } else if intersection.lower_right_diagonal_x == room.lower_right_diagonal_x && intersection.bottom_y > room.bottom_y {
            1
        } else if intersection.right_x == room.right_x && intersection.lower_right_diagonal_x < room.lower_right_diagonal_x {
            2
        } else if intersection.upper_right_diagonal_x == room.upper_right_diagonal_x && intersection.right_x < room.right_x {
            3
        } else if intersection.top_y == room.top_y && intersection.upper_right_diagonal_x < room.upper_right_diagonal_x {
            4
        } else if intersection.upper_left_diagonal_x == room.upper_left_diagonal_x && intersection.top_y < room.top_y {
            5
        } else if intersection.left_x == room.left_x && intersection.upper_left_diagonal_x > room.upper_left_diagonal_x {
            6
        } else if intersection.lower_left_diagonal_x == room.lower_left_diagonal_x && intersection.left_x > room.left_x {
            7
        } else {
            // the roomShape may be contained in the neighbourShape
            return;
        };
        let last_touching_side = if intersection.lower_left_diagonal_x == room.lower_left_diagonal_x && intersection.bottom_y > room.bottom_y {
            7
        } else if intersection.left_x == room.left_x && intersection.lower_left_diagonal_x > room.lower_left_diagonal_x {
            6
        } else if intersection.upper_left_diagonal_x == room.upper_left_diagonal_x && intersection.left_x > room.left_x {
            5
        } else if intersection.top_y == room.top_y && intersection.upper_left_diagonal_x > room.upper_left_diagonal_x {
            4
        } else if intersection.upper_right_diagonal_x == room.upper_right_diagonal_x && intersection.top_y < room.top_y {
            3
        } else if intersection.right_x == room.right_x && intersection.upper_right_diagonal_x < room.upper_right_diagonal_x {
            2
        } else if intersection.lower_right_diagonal_x == room.lower_right_diagonal_x && intersection.right_x < room.right_x {
            1
        } else if intersection.bottom_y == room.bottom_y && intersection.lower_right_diagonal_x < room.lower_right_diagonal_x {
            0
        } else {
            // the roomShape may be contained in the neighbourShape
            return;
        };
        let mut next_side_no = first_touching_side;
        loop {
            let current_side_index = next_side_no;
            next_side_no = (next_side_no + 1) % 8;
            if !self.edge_interior_touches_obstacle[current_side_index as usize] {
                let mut touch_only_at_corner = false;
                if current_side_index == first_touching_side && intersection.corner(current_side_index) == room.corner(next_side_no) {
                    touch_only_at_corner = true;
                }
                if current_side_index == last_touching_side && intersection.corner(next_side_no) == room.corner(current_side_index) {
                    touch_only_at_corner = true;
                }
                if !touch_only_at_corner {
                    self.edge_interior_touches_obstacle[current_side_index as usize] = true;
                }
            }
            if current_side_index == last_touching_side {
                break;
            }
        }
        let index = self.neighbours.len();
        self.neighbours.push(Neighbour { object_id: object.id(), intersection, first_touching_side, last_touching_side });
        let neighbours = &self.neighbours;
        self.sorted.add_by(index, |a, b| compare(&neighbours[*a], &neighbours[*b]));
    }
}

/// Java `Sorted45DegreeRoomNeighbours.calculate(room, autorouteEngine)`.
pub(crate) fn calculate(eng: &mut AutorouteEngine, board: &mut RoutingBoard, mut room: FromRoom<'_>) -> JResult<Option<RoomId>> {
    let net_number = eng.net_number();
    loop {
        let room_id_no = eng.generate_room_id_no();
        let rn = calculate_neighbours(eng, board, &room, net_number, room_id_no);
        // Check, that each side of the room shape has at least one touching neighbour.
        // Otherwise, improve the room shape by enlarging.
        let edge_removed = try_remove_edge_line(&rn, eng, &mut room, board, net_number);
        let result = rn.completed_room;
        if edge_removed {
            eng.remove_all_doors(result)?;
            continue;
        }
        // Now calculate the new incomplete rooms together with the doors between this room and
        // the sorted neighbours.
        if rn.sorted.is_empty() {
            if eng.room(result).is_obstacle() {
                calculate_edge_incomplete_rooms_of_obstacle_expansion_room(&rn, eng, board, &room, 0, 7);
            }
        } else {
            calculate_new_incomplete_rooms(&rn, eng, board, &room);
        }
        return Ok(Some(result));
    }
}

fn calculate_neighbours(eng: &mut AutorouteEngine, board: &mut RoutingBoard, room: &FromRoom<'_>, net_number: i32, room_id_no: i32) -> Sorted45 {
    let room_shape = from_room_shape(eng, room);
    let layer = from_room_layer(eng, room);
    let completed_room = completed_room_of(eng, room, &room_shape, room_id_no);
    let room_oct = room_shape.bounding_octagon().expect("bounding octagon");
    let mut result = Sorted45 {
        completed_room,
        room_shape: eng.room(completed_room).shape().bounding_octagon().expect("bounding octagon"),
        edge_interior_touches_obstacle: [false; 8],
        neighbours: Vec::new(),
        sorted: JavaTreeSet::new(fr_jcompat::treemap::NaturalOrder),
    };
    let completed_is_free = eng.room(completed_room).is_complete_free();
    let completed_is_obstacle = eng.room(completed_room).is_obstacle();
    let mut overlapping_objects = board.overlapping_tree_entries_list(eng.tree, &ConvexShape::Tile(room_shape.clone()), layer, &[]);
    sort_entries(&mut overlapping_objects);
    // Calculate the touching neighbour objects and sort them in counterclock sense around the
    // border of the room shape.
    for current_entry in overlapping_objects {
        let object = current_entry.object;
        if completed_is_free && !board.object_is_trace_obstacle(object, net_number) {
            eng.set_net_dependent(completed_room);
            eng.add_target_door_if_connected(board, completed_room, object, current_entry.shape_index, net_number);
            continue;
        }
        let current_shape = board.object_tree_shape(eng.tree, object, current_entry.shape_index).expect("tree shape is null");
        let current_oct = current_shape.bounding_octagon().expect("bounding octagon");
        let intersection = room_oct.intersection_int_octagon(&current_oct);
        let dimension = intersection.dimension();
        if dimension > 1 && completed_is_obstacle {
            if let TreeObject::Item { key, .. } = object {
                // only Obstacle expansion room may have a 2-dim overlap
                if board.item(key).is_routable() {
                    if let Some(overlap_room) = eng.get_expansion_room(board, key, current_entry.shape_index) {
                        eng.create_overlap_door(board, completed_room, overlap_room);
                    }
                }
            }
            continue;
        }
        if dimension < 0 {
            // may happen at a corner from 2 diagonal lines with non integer coordinates.
            continue;
        }
        result.add_sorted_neighbour(object, intersection);
        if dimension > 0 {
            // make sure, that there is a door to the neighbour room.
            if let Some(neighbour_room) = neighbour_room_of(eng, board, object, current_entry.shape_index) {
                if eng.insert_door_ok(board, completed_room, neighbour_room, &TileShape::IntOctagon(intersection)) {
                    let new_door = eng.new_door_auto(completed_room, neighbour_room);
                    eng.add_door(neighbour_room, new_door);
                    eng.add_door(completed_room, new_door);
                }
            }
        }
    }
    result
}

fn remove_not_touching_border_lines(room_oct: &IntOctagon, t: &[bool; 8]) -> IntOctagon {
    let crit = CRIT_INT;
    let left_x = if t[6] { room_oct.left_x } else { -crit };
    let bottom_y = if t[0] { room_oct.bottom_y } else { -crit };
    let right_x = if t[2] { room_oct.right_x } else { crit };
    let top_y = if t[4] { room_oct.top_y } else { crit };
    let upper_left_diagonal_x = if t[5] { room_oct.upper_left_diagonal_x } else { -crit };
    let lower_right_diagonal_x = if t[1] { room_oct.lower_right_diagonal_x } else { crit };
    let lower_left_diagonal_x = if t[7] { room_oct.lower_left_diagonal_x } else { -crit };
    let upper_right_diagonal_x = if t[3] { room_oct.upper_right_diagonal_x } else { crit };
    IntOctagon::new(left_x, bottom_y, right_x, top_y, upper_left_diagonal_x, lower_right_diagonal_x, lower_left_diagonal_x, upper_right_diagonal_x)
        .normalize()
}

/// Java `calculateEdgeIncompleteRoomsOfObstacleExpansionRoom(fromSideIndex, toSideIndex)`.
fn calculate_edge_incomplete_rooms_of_obstacle_expansion_room(
    rn: &Sorted45,
    eng: &mut AutorouteEngine,
    board: &RoutingBoard,
    from_room: &FromRoom<'_>,
    from_side_index: i32,
    to_side_index: i32,
) {
    if !matches!(from_room, FromRoom::Obstacle(_)) {
        log::warn!("Sorted45DegreeRoomNeighbours.calculate_side_incomplete_rooms_of_obstacle_expansion_room: ObstacleExpansionRoom expected for this.fromRoom");
        return;
    }
    let board_oct = board.bounding_box.bounding_octagon();
    let mut current_corner = rn.room_shape.corner(from_side_index);
    let mut current_side_index = from_side_index;
    loop {
        let next_side_no = (current_side_index + 1) % 8;
        let next_corner = rn.room_shape.corner(next_side_no);
        if current_corner != next_corner {
            let mut o = board_oct;
            match current_side_index {
                0 => o.top_y = rn.room_shape.bottom_y,
                1 => o.upper_left_diagonal_x = rn.room_shape.lower_right_diagonal_x,
                2 => o.left_x = rn.room_shape.right_x,
                3 => o.lower_left_diagonal_x = rn.room_shape.upper_right_diagonal_x,
                4 => o.bottom_y = rn.room_shape.top_y,
                5 => o.lower_right_diagonal_x = rn.room_shape.upper_left_diagonal_x,
                6 => o.right_x = rn.room_shape.left_x,
                7 => o.upper_right_diagonal_x = rn.room_shape.lower_left_diagonal_x,
                _ => {
                    log::warn!("SortedOrthoganelRoomNeighbours.calculate_edge_incomplete_rooms_of_obstacle_expansion_room: currentSideIndex illegal");
                    return;
                }
            }
            insert_incomplete_room(rn, eng, from_room, o);
        }
        // (Java does not advance currentCorner)
        let _ = &mut current_corner;
        if current_side_index == to_side_index {
            break;
        }
        current_side_index = next_side_no;
    }
}

/// Java `tryRemoveEdgeLine(netNumber, autorouteSearchTree)`.
fn try_remove_edge_line(rn: &Sorted45, eng: &AutorouteEngine, room: &mut FromRoom<'_>, board: &RoutingBoard, net_number: i32) -> bool {
    let FromRoom::Incomplete(current_incomplete_room) = room else {
        return false;
    };
    let Some(TileShape::IntOctagon(room_oct)) = current_incomplete_room.shape.clone() else {
        log::warn!("Sorted45DegreeRoomNeighbours.tryRemoveEdgeLine: IntOctagon expected for roomShape type");
        return false;
    };
    let room_area = room_oct.area();
    let mut try_remove_edge_lines = false;
    for i in 0..8 {
        if !rn.edge_interior_touches_obstacle[i as usize] {
            let prev_corner = TileShape::IntOctagon(rn.room_shape).corner_approx(i);
            let next_corner = TileShape::IntOctagon(rn.room_shape).corner_approx(TileShape::IntOctagon(rn.room_shape).next_no(i));
            if prev_corner.distance_square(&next_corner) > 1.0 {
                try_remove_edge_lines = true;
                break;
            }
        }
    }
    if !try_remove_edge_lines {
        return false;
    }
    // Touching neighbour missing at the edge side. Remove the edge line and restart.
    let enlarged_oct = remove_not_touching_border_lines(&room_oct, &rn.edge_interior_touches_obstacle);
    let mut ignore_shape: Option<TileShape> = None;
    let mut ignore_object: Option<TreeObject> = None;
    let mut max_door_area = 0.0;
    for &d in &eng.room(rn.completed_room).doors {
        // insert the overlapping doors with CompleteFreeSpaceExpansionRooms for the information
        // in complete_shape about the objects to ignore.
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
        Some(TileShape::IntOctagon(enlarged_oct)),
        current_incomplete_room.layer,
        current_incomplete_room.contained_shape.clone(),
    );
    let new_rooms = board.complete_shape(eng.tree, &enlarged_room, net_number, ignore_object, ignore_shape.as_ref());
    if new_rooms.len() == 1 {
        // Check, that the area increases to prevent endless loop.
        let new_room = new_rooms.into_iter().next().unwrap();
        if new_room.shape.as_ref().unwrap().area() > room_area {
            current_incomplete_room.shape = new_room.shape;
            current_incomplete_room.contained_shape = new_room.contained_shape;
            return true;
        }
    }
    false
}

/// Java `insertIncompleteRoom(autorouteEngine, lx, ly, rx, uy, ulx, lrx, llx, urx)`.
fn insert_incomplete_room(rn: &Sorted45, eng: &mut AutorouteEngine, from_room: &FromRoom<'_>, o: IntOctagon) {
    let new_incomplete_room_shape = IntOctagon::new(
        o.left_x,
        o.bottom_y,
        o.right_x,
        o.top_y,
        o.upper_left_diagonal_x,
        o.lower_right_diagonal_x,
        o.lower_left_diagonal_x,
        o.upper_right_diagonal_x,
    )
    .normalize();
    if new_incomplete_room_shape.dimension() == 2 {
        let new_contained_shape = rn.room_shape.intersection_int_octagon(&new_incomplete_room_shape);
        if !new_contained_shape.is_empty() {
            let door_dimension = new_contained_shape.dimension();
            if door_dimension > 0 {
                let layer = from_room_layer(eng, from_room);
                let new_room = eng.add_incomplete_expansion_room(
                    Some(TileShape::IntOctagon(new_incomplete_room_shape)),
                    layer,
                    Some(TileShape::IntOctagon(new_contained_shape)),
                );
                let new_door = eng.new_door(rn.completed_room, new_room, door_dimension);
                eng.add_door(rn.completed_room, new_door);
                eng.add_door(new_room, new_door);
            }
        }
    }
}

/// Java `calculateNewIncompleteRoomsForObstacleExpansionRoom(prevNeighbour, nextNeighbour)`.
fn calculate_new_incomplete_rooms_for_obstacle_expansion_room(
    rn: &Sorted45,
    eng: &mut AutorouteEngine,
    board: &RoutingBoard,
    from_room: &FromRoom<'_>,
    prev: usize,
    next: usize,
) {
    let prev_n = &rn.neighbours[prev];
    let next_n = &rn.neighbours[next];
    let from_side_index = prev_n.last_touching_side;
    let to_side_index = next_n.first_touching_side;
    if from_side_index == to_side_index && prev != next {
        // no return in case of only 1 neighbour.
        return;
    }
    let board_oct = board.bounding_box.bounding_octagon();
    let room = rn.room_shape;
    // insert the new incomplete room from prevNeighbour to the next corner of the room shape.
    let mut o = board_oct;
    match from_side_index {
        0 => {
            o.top_y = room.bottom_y;
            o.upper_left_diagonal_x = prev_n.intersection.lower_right_diagonal_x;
        }
        1 => {
            o.upper_left_diagonal_x = room.lower_right_diagonal_x;
            o.left_x = prev_n.intersection.right_x;
        }
        2 => {
            o.left_x = room.right_x;
            o.lower_left_diagonal_x = prev_n.intersection.upper_right_diagonal_x;
        }
        3 => {
            o.lower_left_diagonal_x = room.upper_right_diagonal_x;
            o.bottom_y = prev_n.intersection.top_y;
        }
        4 => {
            o.bottom_y = room.top_y;
            o.lower_right_diagonal_x = prev_n.intersection.upper_left_diagonal_x;
        }
        5 => {
            o.lower_right_diagonal_x = room.upper_left_diagonal_x;
            o.right_x = prev_n.intersection.left_x;
        }
        6 => {
            o.right_x = room.left_x;
            o.upper_right_diagonal_x = prev_n.intersection.lower_left_diagonal_x;
        }
        7 => {
            o.upper_right_diagonal_x = room.lower_left_diagonal_x;
            o.top_y = prev_n.intersection.bottom_y;
        }
        _ => {}
    }
    insert_incomplete_room(rn, eng, from_room, o);
    // insert the new incomplete room from the previous corner of the room shape to nextNeighbour.
    let mut o = board_oct;
    match to_side_index {
        0 => {
            o.top_y = room.bottom_y;
            o.upper_right_diagonal_x = next_n.intersection.lower_left_diagonal_x;
        }
        1 => {
            o.upper_left_diagonal_x = room.lower_right_diagonal_x;
            o.top_y = next_n.intersection.bottom_y;
        }
        2 => {
            o.left_x = room.right_x;
            o.upper_left_diagonal_x = next_n.intersection.lower_right_diagonal_x;
        }
        3 => {
            o.lower_left_diagonal_x = room.upper_right_diagonal_x;
            o.left_x = next_n.intersection.right_x;
        }
        4 => {
            o.bottom_y = room.top_y;
            o.lower_left_diagonal_x = next_n.intersection.upper_right_diagonal_x;
        }
        5 => {
            o.lower_right_diagonal_x = room.upper_left_diagonal_x;
            o.bottom_y = next_n.intersection.top_y;
        }
        6 => {
            o.right_x = room.left_x;
            o.lower_right_diagonal_x = next_n.intersection.upper_left_diagonal_x;
        }
        7 => {
            o.upper_right_diagonal_x = room.lower_left_diagonal_x;
            o.right_x = next_n.intersection.left_x;
        }
        _ => {}
    }
    insert_incomplete_room(rn, eng, from_room, o);
    // Insert the new incomplete rooms on the intermediate free sides of the obstacle expansion
    // room.
    let current_from_side_no = (from_side_index + 1) % 8;
    if current_from_side_no == to_side_index {
        return;
    }
    let current_to_side_no = (to_side_index + 7) % 8;
    calculate_edge_incomplete_rooms_of_obstacle_expansion_room(rn, eng, board, from_room, current_from_side_no, current_to_side_no);
}

/// Java `calculateNewIncompleteRooms(autorouteEngine)`.
fn calculate_new_incomplete_rooms(rn: &Sorted45, eng: &mut AutorouteEngine, board: &RoutingBoard, from_room: &FromRoom<'_>) {
    let board_oct = board.bounding_box.bounding_octagon();
    let sorted: Vec<usize> = rn.sorted.iter().copied().collect();
    let mut prev = *sorted.last().unwrap();
    let from_is_obstacle = matches!(from_room, FromRoom::Obstacle(_));
    if from_is_obstacle && sorted.len() == 1 {
        // ObstacleExpansionRoom has only 1 neighbour
        calculate_new_incomplete_rooms_for_obstacle_expansion_room(rn, eng, board, from_room, prev, prev);
        return;
    }
    for &next in &sorted {
        let prev_n = &rn.neighbours[prev];
        let next_n = &rn.neighbours[next];
        let intersection = TileShape::IntOctagon(next_n.intersection).intersection(&TileShape::IntOctagon(prev_n.intersection));
        let insert = if intersection.is_empty() {
            true
        } else if intersection.dimension() >= 1 {
            false
        } else if prev_n.last_touching_side == next_n.first_touching_side {
            // Point contact (dimension == 0): touch at a corner of the room shape
            false
        } else {
            prev_n.last_touching_side != (next_n.first_touching_side + 1) % 8
        };
        if insert {
            // create a door to a new incomplete expansion room between the last corner of the
            // previous neighbour and the first corner of the current neighbour
            if from_is_obstacle && next_n.first_touching_side != prev_n.last_touching_side {
                calculate_new_incomplete_rooms_for_obstacle_expansion_room(rn, eng, board, from_room, prev, next);
            } else {
                let mut o = board_oct;
                let p = &prev_n.intersection;
                let n = &next_n.intersection;
                match next_n.first_touching_side {
                    0 => {
                        if p.lower_left_diagonal_x < n.lower_left_diagonal_x {
                            o.upper_right_diagonal_x = n.lower_left_diagonal_x;
                            o.top_y = p.bottom_y;
                            if prev_n.last_touching_side == 0 {
                                o.upper_left_diagonal_x = p.lower_right_diagonal_x;
                            }
                        } else if p.lower_left_diagonal_x > n.lower_left_diagonal_x {
                            o.right_x = n.left_x;
                            o.upper_right_diagonal_x = p.lower_left_diagonal_x;
                        } else {
                            o.upper_right_diagonal_x = n.lower_left_diagonal_x;
                        }
                    }
                    1 => {
                        if p.bottom_y < n.bottom_y {
                            o.top_y = n.bottom_y;
                            o.upper_left_diagonal_x = p.lower_right_diagonal_x;
                            if prev_n.last_touching_side == 1 {
                                o.left_x = p.right_x;
                            }
                        } else if p.bottom_y > n.bottom_y {
                            o.top_y = p.bottom_y;
                            o.upper_right_diagonal_x = n.lower_left_diagonal_x;
                        } else {
                            o.top_y = n.bottom_y;
                        }
                    }
                    2 => {
                        if p.lower_right_diagonal_x > n.lower_right_diagonal_x {
                            o.upper_left_diagonal_x = n.lower_right_diagonal_x;
                            o.left_x = p.right_x;
                            if prev_n.last_touching_side == 2 {
                                o.lower_left_diagonal_x = p.upper_right_diagonal_x;
                            }
                        } else if p.lower_right_diagonal_x < n.lower_right_diagonal_x {
                            o.top_y = n.bottom_y;
                            o.upper_left_diagonal_x = p.lower_right_diagonal_x;
                        } else {
                            o.upper_left_diagonal_x = n.lower_right_diagonal_x;
                        }
                    }
                    3 => {
                        if p.right_x > n.right_x {
                            o.left_x = n.right_x;
                            o.lower_left_diagonal_x = p.upper_right_diagonal_x;
                            if prev_n.last_touching_side == 3 {
                                o.bottom_y = p.top_y;
                            }
                        } else if p.right_x < n.right_x {
                            o.left_x = p.right_x;
                            o.upper_left_diagonal_x = n.lower_right_diagonal_x;
                        } else {
                            o.left_x = n.right_x;
                        }
                    }
                    4 => {
                        if p.upper_right_diagonal_x > n.upper_right_diagonal_x {
                            o.lower_left_diagonal_x = n.upper_right_diagonal_x;
                            o.bottom_y = p.top_y;
                            if prev_n.last_touching_side == 4 {
                                o.lower_right_diagonal_x = p.upper_left_diagonal_x;
                            }
                        } else if p.upper_right_diagonal_x < n.upper_right_diagonal_x {
                            o.left_x = n.right_x;
                            o.lower_left_diagonal_x = p.upper_right_diagonal_x;
                        } else {
                            o.lower_left_diagonal_x = n.upper_right_diagonal_x;
                        }
                    }
                    5 => {
                        if p.top_y > n.top_y {
                            o.bottom_y = n.top_y;
                            o.lower_right_diagonal_x = p.upper_left_diagonal_x;
                            if prev_n.last_touching_side == 5 {
                                o.right_x = p.left_x;
                            }
                        } else if p.top_y < n.top_y {
                            o.bottom_y = p.top_y;
                            o.lower_left_diagonal_x = n.upper_right_diagonal_x;
                        } else {
                            o.bottom_y = n.top_y;
                        }
                    }
                    6 => {
                        if p.upper_left_diagonal_x < n.upper_left_diagonal_x {
                            o.lower_right_diagonal_x = n.upper_left_diagonal_x;
                            o.right_x = p.left_x;
                            if prev_n.last_touching_side == 6 {
                                o.upper_right_diagonal_x = p.lower_left_diagonal_x;
                            }
                        } else if p.upper_left_diagonal_x > n.upper_left_diagonal_x {
                            o.bottom_y = n.top_y;
                            o.lower_right_diagonal_x = p.upper_left_diagonal_x;
                        } else {
                            o.lower_right_diagonal_x = n.upper_left_diagonal_x;
                        }
                    }
                    7 => {
                        if p.left_x < n.left_x {
                            o.right_x = n.left_x;
                            o.upper_right_diagonal_x = p.lower_left_diagonal_x;
                            if prev_n.last_touching_side == 7 {
                                o.top_y = p.bottom_y;
                            }
                        } else if p.left_x > n.left_x {
                            o.right_x = p.left_x;
                            o.lower_right_diagonal_x = n.upper_left_diagonal_x;
                        } else {
                            o.right_x = n.left_x;
                        }
                    }
                    _ => log::warn!("Sorted45DegreeRoomNeighbour.calculate_new_incomplete: illegal touching side"),
                }
                insert_incomplete_room(rn, eng, from_room, o);
            }
        }
        prev = next;
    }
}
