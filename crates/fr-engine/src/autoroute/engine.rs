//! Port of `autoroute/maze/AutorouteEngine.java`, the expansion room / door classes of
//! `autoroute/expansion` (`ExpansionRoom`, `FreeSpaceExpansionRoom`,
//! `CompleteFreeSpaceExpansionRoom`, `ObstacleExpansionRoom`, `ExpansionDoor`,
//! `TargetItemExpansionDoor`, `ExpandableObject`), of `autoroute/ItemAutorouteInfo.java`,
//! `maze/MazeSearchElement.java` and of `autoroute/drill/{DrillPage, DrillPageArray,
//! ExpansionDrill}.java`.
//!
//! All objects live in arenas of the engine; see the module docs of [`crate::autoroute`].

use std::collections::HashMap;

use fr_geom::{ConvexShape, FloatLine, IntBox, Line, Point, PolylineArea, PolylineShape, Simplex, TileShape};

use crate::board::{ItemKey, ItemKind, RoutingBoard, TreeKind, TreeObject};
use crate::datastructures::{StopToken, TimeLimit};
use crate::ids::{LayerNo, NetNo};

use super::connection::Connection;
use super::rooms::{IncompleteFreeSpaceExpansionRoom, RoomKey};
use super::{JResult, JavaException, TRACE_WIDTH_TOLERANCE};

/// Handle of an expansion room (incomplete free space, complete free space or obstacle room).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RoomId(pub u32);

/// Handle of an [`ExpansionDoor`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DoorId(pub u32);

/// Handle of a [`TargetItemExpansionDoor`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TargetDoorId(pub u32);

/// Handle of an [`ExpansionDrill`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DrillId(pub u32);

/// Handle of a [`DrillPage`] (row major index into the page array).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PageId(pub u32);

/// Java `ExpandableObject`: a door, a target door, a drill or a drill page.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Expandable {
    Door(DoorId),
    Target(TargetDoorId),
    Drill(DrillId),
    Page(PageId),
}

/// Java `MazeSearchElement.Adjustment`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Adjustment {
    #[default]
    None,
    Right,
    Left,
}

/// Java `MazeSearchElement`: the maze search state of a section of an expandable object.
#[derive(Clone, Debug, Default)]
pub struct MazeSearchElement {
    pub is_occupied: bool,
    pub backtrack_door: Option<Expandable>,
    pub section_no_of_backtrack_door: i32,
    pub room_ripped: bool,
    pub adjustment: Adjustment,
    pub ripup_cost: i32,
}

impl MazeSearchElement {
    /// Java `reset()`.
    pub fn reset(&mut self) {
        *self = MazeSearchElement::default();
    }
}

/// The class specific data of an expansion room.
#[derive(Clone, Debug)]
pub enum RoomKind {
    /// Java `IncompleteFreeSpaceExpansionRoom` (`shape == None`: the whole plane).
    Incomplete { shape: Option<TileShape>, contained_shape: Option<TileShape> },
    /// Java `CompleteFreeSpaceExpansionRoom`.
    Complete { shape: TileShape, id: i32, target_doors: Vec<TargetDoorId>, net_dependent: bool },
    /// Java `ObstacleExpansionRoom`.
    Obstacle { item: ItemKey, item_id: i32, index_in_item: i32, shape: TileShape, doors_calculated: bool },
}

/// Java `ExpansionRoom` (the arena entry).
#[derive(Clone, Debug)]
pub struct Room {
    pub layer: LayerNo,
    /// The doors to neighbour rooms (Java `getDoors()`, an `ArrayList`).
    pub doors: Vec<DoorId>,
    pub kind: RoomKind,
}

impl Room {
    pub fn is_incomplete(&self) -> bool {
        matches!(self.kind, RoomKind::Incomplete { .. })
    }
    /// `instanceof CompleteFreeSpaceExpansionRoom`.
    pub fn is_complete_free(&self) -> bool {
        matches!(self.kind, RoomKind::Complete { .. })
    }
    /// `instanceof ObstacleExpansionRoom`.
    pub fn is_obstacle(&self) -> bool {
        matches!(self.kind, RoomKind::Obstacle { .. })
    }
    /// `instanceof CompleteExpansionRoom` (complete free space or obstacle room).
    pub fn is_complete_expansion_room(&self) -> bool {
        !self.is_incomplete()
    }
    /// The item of an obstacle room.
    pub fn obstacle_item(&self) -> Option<ItemKey> {
        match &self.kind {
            RoomKind::Obstacle { item, .. } => Some(*item),
            _ => None,
        }
    }
    /// Java `getShape()` (`None` for an incomplete room covering the whole plane).
    pub fn shape_opt(&self) -> Option<&TileShape> {
        match &self.kind {
            RoomKind::Incomplete { shape, .. } => shape.as_ref(),
            RoomKind::Complete { shape, .. } | RoomKind::Obstacle { shape, .. } => Some(shape),
        }
    }
    /// Java `getShape()` of a room with a shape (panics like Java's NullPointerException).
    pub fn shape(&self) -> &TileShape {
        self.shape_opt().expect("ExpansionRoom.getShape: shape is null")
    }
    /// Java `getTargetDoors()` (empty for obstacle and incomplete rooms).
    pub fn target_doors(&self) -> &[TargetDoorId] {
        match &self.kind {
            RoomKind::Complete { target_doors, .. } => target_doors,
            _ => &[],
        }
    }
}

/// Java `ExpansionDoor`: the common edge between two rooms.
#[derive(Clone, Debug)]
pub struct ExpansionDoor {
    pub first_room: RoomId,
    pub second_room: RoomId,
    /// The dimension of the door (1 or 2).
    pub dimension: i32,
    /// Java `sectionArr` (`None` until `getSectionSegments` allocates it).
    pub sections: Option<Vec<MazeSearchElement>>,
}

/// Java `TargetItemExpansionDoor`: a door to a start or destination item.
#[derive(Clone, Debug)]
pub struct TargetItemExpansionDoor {
    pub item: ItemKey,
    pub item_id: i32,
    pub tree_entry_no: i32,
    pub room: RoomId,
    pub shape: TileShape,
    pub element: MazeSearchElement,
}

/// Java `ExpansionDrill`: a layer change expansion object.
#[derive(Clone, Debug)]
pub struct ExpansionDrill {
    pub shape: TileShape,
    /// The location, where the drill is checked.
    pub location: Point,
    pub first_layer: LayerNo,
    pub last_layer: LayerNo,
    /// The expansion room of each layer (Java `roomArr`).
    pub room_arr: Vec<Option<RoomId>>,
    pub elements: Vec<MazeSearchElement>,
}

impl ExpansionDrill {
    fn new(shape: TileShape, location: Point, first_layer: LayerNo, last_layer: LayerNo) -> Self {
        let n = (last_layer - first_layer + 1).max(0) as usize;
        ExpansionDrill { shape, location, first_layer, last_layer, room_arr: vec![None; n], elements: vec![MazeSearchElement::default(); n] }
    }

    /// Java `getId()`.
    pub fn java_id(&self) -> i32 {
        31i32.wrapping_mul(31i32.wrapping_mul(self.location.get_id()).wrapping_add(self.first_layer)).wrapping_add(self.last_layer)
    }
}

/// Java `DrillPage`: a rectangular page of expansion drills.
#[derive(Clone, Debug)]
pub struct DrillPage {
    pub shape: IntBox,
    pub elements: Vec<MazeSearchElement>,
    /// The drills of this page (`None` if not yet calculated).
    pub drills: Option<Vec<DrillId>>,
    /// The net for which the drills are calculated.
    pub net_number: NetNo,
}

impl DrillPage {
    /// Java `getId()` (depends on the net number of the last drill calculation).
    pub fn java_id(&self) -> i32 {
        31i32.wrapping_mul(self.shape.get_id()).wrapping_add(self.net_number)
    }
}

/// Java `DrillPageArray`.
#[derive(Clone, Debug)]
pub struct DrillPageArray {
    bounds: IntBox,
    column_count: i32,
    row_count: i32,
    page_width: i32,
    page_height: i32,
    /// Row major (`pages[j * column_count + i]`).
    pub pages: Vec<DrillPage>,
}

impl DrillPageArray {
    /// Java `new DrillPageArray(board, maxPageWidth)`.
    pub fn new(bounds: IntBox, layer_count: i32, max_page_width: i32) -> Self {
        let length = bounds.ur.x.wrapping_sub(bounds.ll.x) as f64;
        let height = bounds.ur.y.wrapping_sub(bounds.ll.y) as f64;
        let column_count = (length / max_page_width as f64).ceil() as i32;
        let row_count = (height / max_page_width as f64).ceil() as i32;
        let page_width = (length / column_count as f64).ceil() as i32;
        let page_height = (height / row_count as f64).ceil() as i32;
        let mut pages = Vec::with_capacity((row_count.max(0) * column_count.max(0)) as usize);
        for j in 0..row_count {
            for i in 0..column_count {
                let ll_x = bounds.ll.x + i * page_width;
                let ur_x = if i == column_count - 1 { bounds.ur.x } else { ll_x + page_width };
                let ll_y = bounds.ll.y + j * page_height;
                let ur_y = if j == row_count - 1 { bounds.ur.y } else { ll_y + page_height };
                pages.push(DrillPage {
                    shape: IntBox::new(ll_x, ll_y, ur_x, ur_y),
                    elements: vec![MazeSearchElement::default(); layer_count.max(0) as usize],
                    drills: None,
                    net_number: -1,
                });
            }
        }
        DrillPageArray { bounds, column_count, row_count, page_width, page_height, pages }
    }

    /// Java `overlappingPages(shape)`: the pages with a 2-dimensional overlap with `shape`.
    pub fn overlapping_pages(&self, shape: &TileShape) -> Vec<PageId> {
        let mut result = Vec::new();
        let shape_box = shape.bounding_box().intersection_int_box(&self.bounds);
        let min_j = ((shape_box.ll.y.wrapping_sub(self.bounds.ll.y)) as f64 / self.page_height as f64).floor() as i32;
        let max_j = (shape_box.ur.y.wrapping_sub(self.bounds.ll.y)) as f64 / self.page_height as f64;
        let min_i = ((shape_box.ll.x.wrapping_sub(self.bounds.ll.x)) as f64 / self.page_width as f64).floor() as i32;
        let max_i = (shape_box.ur.x.wrapping_sub(self.bounds.ll.x)) as f64 / self.page_width as f64;
        let mut j = min_j;
        while (j as f64) < max_j {
            let mut i = min_i;
            while (i as f64) < max_i {
                let index = j * self.column_count + i;
                let page = &self.pages[index as usize];
                let intersection = shape.intersection(&TileShape::IntBox(page.shape));
                if intersection.dimension() > 1 {
                    result.push(PageId(index as u32));
                }
                i += 1;
            }
            j += 1;
        }
        result
    }

    /// Java `invalidate(shape)`.
    pub fn invalidate(&mut self, shape: &TileShape) {
        for p in self.overlapping_pages(shape) {
            self.pages[p.0 as usize].drills = None;
        }
    }

    /// The number of rows (Java `rowCount`).
    pub fn row_count(&self) -> i32 {
        self.row_count
    }
}

/// Java `ItemAutorouteInfo` (plus `Via.autorouteDrillInfo`).
#[derive(Clone, Debug, Default)]
pub struct ItemAutorouteInfo {
    /// True if the item belongs to the start set of the maze search.
    pub start_info: bool,
    /// Index into [`AutorouteEngine::connections`].
    pub precalculated_connection: Option<usize>,
    /// The obstacle rooms for each tree shape (Java `expansionRoomArr`).
    pub expansion_rooms: Option<Vec<Option<RoomId>>>,
    /// Java `Via.autorouteDrillInfo`.
    pub via_drill: Option<DrillId>,
}

/// The room whose doors are calculated (Java `calculateDoors(ExpansionRoom room)`): an
/// incomplete room value (its shape may be changed by the edge removal) or an obstacle room.
pub enum FromRoom<'a> {
    Incomplete(&'a mut IncompleteFreeSpaceExpansionRoom),
    Obstacle(RoomId),
}

/// The room passed to [`AutorouteEngine::complete_expansion_room`]: a room of the incomplete room
/// list or a new `IncompleteFreeSpaceExpansionRoom` not in the list (`ExpansionDrill`).
pub enum IncompleteRef {
    Arena(RoomId),
    Temp(IncompleteFreeSpaceExpansionRoom),
}

/// pcbkit F2: consecutive `complete_expansion_room` panics after which the search is abandoned.
const MAX_ROOM_FAULTS: u32 = 8;

/// pcbkit F2: set when a search gave up on a repeatedly failing room completion. The server clears it
/// before a request and answers `internal` if it is set afterwards (see [`take_engine_fault`]).
static ENGINE_FAULT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// pcbkit F2: returns whether an engine fault was recorded since the last call, and clears it.
pub fn take_engine_fault() -> bool {
    ENGINE_FAULT.swap(false, std::sync::atomic::Ordering::SeqCst)
}

/// Java `AutorouteEngine`.
#[derive(Clone, Debug)]
pub struct AutorouteEngine {
    /// The index of the autoroute search tree in the board's search tree manager.
    pub tree: usize,
    /// The compensated clearance class of the autoroute search tree.
    pub tree_clearance_class: i32,
    /// If true, the database is retained and maintained between connections.
    pub maintain_database: bool,
    pub(crate) drill_page_array: DrillPageArray,
    pub(crate) stop: Option<StopToken>,
    /// pcbkit F2: consecutive caught panics of `complete_expansion_room` (reset by a success).
    room_faults: u32,
    net_number: NetNo,
    time_limit: Option<TimeLimit>,
    /// Java `incompleteExpansionRooms != null`.
    incomplete_list_exists: bool,
    /// Java `completeExpansionRooms` (in insertion order).
    pub(crate) complete_rooms: Option<Vec<RoomId>>,
    expansion_room_instance_count: i32,
    pub(crate) rooms: Vec<Room>,
    pub(crate) doors: Vec<ExpansionDoor>,
    pub(crate) target_doors: Vec<TargetItemExpansionDoor>,
    pub(crate) drills: Vec<ExpansionDrill>,
    pub(crate) item_infos: HashMap<ItemKey, ItemAutorouteInfo>,
    pub(crate) connections: Vec<Connection>,
    /// pcbkit H3: blocker items of the last connection (see `AutorouteControl::collect_blockers`).
    pub blockers: Vec<ItemKey>,
}

impl AutorouteEngine {
    /// Java `new AutorouteEngine(board, traceClearanceClassIndex, maintainDatabase)`.
    pub fn new(board: &mut RoutingBoard, trace_clearance_class: i32, maintain_database: bool) -> Self {
        let tree = board.get_autoroute_tree(trace_clearance_class);
        let mut max_drill_page_width = (5.0 * board.rules.get_default_via_diameter(&board.library.padstacks)) as i32;
        max_drill_page_width = max_drill_page_width.max(10000);
        let drill_page_array = DrillPageArray::new(board.bounding_box, board.layer_count(), max_drill_page_width);
        AutorouteEngine {
            tree,
            tree_clearance_class: board.search_tree(tree).compensated_clearance_class_no,
            maintain_database,
            drill_page_array,
            stop: None,
            room_faults: 0,
            net_number: -1,
            time_limit: None,
            incomplete_list_exists: false,
            complete_rooms: None,
            expansion_room_instance_count: 0,
            rooms: Vec::new(),
            doors: Vec::new(),
            target_doors: Vec::new(),
            drills: Vec::new(),
            item_infos: HashMap::new(),
            connections: Vec::new(),
            blockers: Vec::new(),
        }
    }

    /// Java `new AutorouteEngine(...)` replacing the engine `old` of the board without clearing
    /// it (Java `initAutoroute`): the expansion objects of `old` that are still referenced (rooms
    /// left in a search tree, item autoroute infos) stay alive, so the arenas are carried over.
    pub fn new_carry(board: &mut RoutingBoard, trace_clearance_class: i32, maintain_database: bool, old: Option<Box<AutorouteEngine>>) -> Self {
        let mut e = Self::new(board, trace_clearance_class, maintain_database);
        if let Some(old) = old {
            let rooms_in_trees = board.search_trees().trees().iter().any(|t| !t.room_keys().is_empty());
            if rooms_in_trees || !old.item_infos.is_empty() {
                let old = *old;
                e.rooms = old.rooms;
                e.doors = old.doors;
                e.target_doors = old.target_doors;
                e.drills = old.drills;
                e.item_infos = old.item_infos;
                e.connections = old.connections;
            }
        }
        e
    }

    /// Java `initConnection(netNumber, stoppableThread, timeLimit)`.
    pub fn init_connection(&mut self, board: &mut RoutingBoard, net_number: NetNo, stop: Option<StopToken>, time_limit: Option<TimeLimit>) {
        self.process_board_changes(board);
        if self.maintain_database && net_number != self.net_number {
            if let Some(list) = &self.complete_rooms {
                // invalidate the net dependent complete free space expansion rooms.
                let rooms_to_remove: Vec<RoomId> = list
                    .iter()
                    .copied()
                    .filter(|r| matches!(self.rooms[r.0 as usize].kind, RoomKind::Complete { net_dependent: true, .. }))
                    .collect();
                for r in rooms_to_remove {
                    self.remove_complete_expansion_room(board, r);
                }
            }
            // invalidate the neighbour rooms of the items of netNumber
            for key in board.get_items() {
                if board.item(key).contains_net(net_number) {
                    board.additional_update_after_change(key);
                    self.process_board_changes(board);
                }
            }
        }
        self.net_number = net_number;
        self.stop = stop;
        self.time_limit = time_limit;
    }

    /// Does the work of Java `RoutingBoard.additionalUpdateAfterChange` that the board could not
    /// do (the board already removed the rooms from the tree, see
    /// [`AutorouteMaintenance`](crate::board::AutorouteMaintenance)).
    pub fn process_board_changes(&mut self, board: &mut RoutingBoard) {
        let Some(m) = board.basic.autoroute_maintenance.as_mut() else {
            return;
        };
        let removed = std::mem::take(&mut m.removed_rooms);
        let shapes = std::mem::take(&mut m.invalidated_drill_shapes);
        let cleared = std::mem::take(&mut m.cleared_autoroute_info);
        for s in &shapes {
            self.drill_page_array.invalidate(s);
        }
        for r in removed {
            // (Java calls removeCompleteExpansionRoom for every room found in the tree)
            let id = RoomId(r.0);
            if (id.0 as usize) < self.rooms.len() && self.room(id).is_complete_free() {
                self.remove_complete_expansion_room(board, id);
            }
        }
        for key in cleared {
            self.item_infos.remove(&key);
        }
    }

    /// Java `getNetNumber()`.
    pub fn net_number(&self) -> NetNo {
        self.net_number
    }

    /// Java `isStopRequested()`.
    pub fn is_stop_requested(&self) -> bool {
        if let Some(t) = &self.time_limit {
            if t.limit_exceeded() {
                return true;
            }
        }
        match &self.stop {
            None => false,
            Some(s) => s.is_stop_requested(),
        }
    }

    /// Java `clear()`: removes the complete rooms from the search tree (in list order) and drops
    /// all temporary data, including the item autoroute infos
    /// (`board.clearAllItemTemporaryAutorouteData()`).
    pub fn clear(&mut self, board: &mut RoutingBoard) {
        if let Some(list) = self.complete_rooms.take() {
            let tree = board.search_trees_mut().tree_mut(self.tree);
            for r in list {
                tree.remove_room(RoomKey(r.0));
            }
        }
        self.incomplete_list_exists = false;
        self.expansion_room_instance_count = 0;
        self.item_infos.clear();
    }

    /// Java `board.clearAllItemTemporaryAutorouteData()`: drops all item autoroute infos.
    pub fn clear_item_infos(&mut self) {
        self.item_infos.clear();
    }

    /// Java `resetAllDoors()` (only used if the database is maintained).
    pub fn reset_all_doors(&mut self, board: &RoutingBoard) {
        let list: Vec<RoomId> = self.complete_room_list().to_vec();
        for r in list {
            self.reset_room_doors(r);
        }
        for key in board.get_items() {
            if let Some(info) = self.item_infos.get(&key) {
                let rooms: Vec<RoomId> = info.expansion_rooms.iter().flatten().flatten().copied().collect();
                for r in rooms {
                    self.reset_room_doors(r);
                }
                self.item_infos.get_mut(&key).unwrap().precalculated_connection = None;
            }
        }
        for page in 0..self.drill_page_array.pages.len() {
            self.reset_page(PageId(page as u32));
        }
    }

    fn reset_room_doors(&mut self, r: RoomId) {
        let doors = self.rooms[r.0 as usize].doors.clone();
        for d in doors {
            self.reset_expandable(Expandable::Door(d));
        }
        let targets = self.rooms[r.0 as usize].target_doors().to_vec();
        for t in targets {
            self.reset_expandable(Expandable::Target(t));
        }
    }

    fn reset_page(&mut self, page: PageId) {
        let drills = self.drill_page_array.pages[page.0 as usize].drills.clone();
        if let Some(drills) = drills {
            for d in drills {
                self.reset_expandable(Expandable::Drill(d));
            }
        }
        for e in self.drill_page_array.pages[page.0 as usize].elements.iter_mut() {
            e.reset();
        }
    }

    /// Java `ExpandableObject.reset()`.
    pub fn reset_expandable(&mut self, e: Expandable) {
        match e {
            Expandable::Door(d) => {
                if let Some(s) = &mut self.doors[d.0 as usize].sections {
                    for x in s.iter_mut() {
                        x.reset();
                    }
                }
            }
            Expandable::Target(t) => self.target_doors[t.0 as usize].element.reset(),
            Expandable::Drill(d) => {
                for x in self.drills[d.0 as usize].elements.iter_mut() {
                    x.reset();
                }
            }
            Expandable::Page(p) => self.reset_page(p),
        }
    }

    /// Java `generateRoomIdNo()`.
    pub fn generate_room_id_no(&mut self) -> i32 {
        self.expansion_room_instance_count = self.expansion_room_instance_count.wrapping_add(1);
        self.expansion_room_instance_count
    }

    // ------------------------------------------------------------------------------------------
    // room accessors

    #[inline]
    pub fn room(&self, r: RoomId) -> &Room {
        &self.rooms[r.0 as usize]
    }

    #[inline]
    pub fn room_mut(&mut self, r: RoomId) -> &mut Room {
        &mut self.rooms[r.0 as usize]
    }

    #[inline]
    pub fn door(&self, d: DoorId) -> &ExpansionDoor {
        &self.doors[d.0 as usize]
    }

    #[inline]
    pub fn target_door(&self, t: TargetDoorId) -> &TargetItemExpansionDoor {
        &self.target_doors[t.0 as usize]
    }

    #[inline]
    pub fn drill(&self, d: DrillId) -> &ExpansionDrill {
        &self.drills[d.0 as usize]
    }

    #[inline]
    pub fn page(&self, p: PageId) -> &DrillPage {
        &self.drill_page_array.pages[p.0 as usize]
    }

    /// The complete free space rooms in list order.
    pub fn complete_room_list(&self) -> &[RoomId] {
        self.complete_rooms.as_deref().unwrap_or(&[])
    }

    /// The tree object of a complete free space room.
    pub fn room_tree_object(&self, r: RoomId) -> TreeObject {
        TreeObject::Room { key: RoomKey(r.0), id: self.room_java_id(r) }
    }

    /// Java `ExpansionRoom.getId()`.
    pub fn room_java_id(&self, r: RoomId) -> i32 {
        let room = self.room(r);
        match &room.kind {
            RoomKind::Incomplete { shape, .. } => {
                let shape = shape.as_ref().expect("IncompleteFreeSpaceExpansionRoom.getId: shape is null");
                31i32.wrapping_mul(shape.get_id()).wrapping_add(room.layer)
            }
            RoomKind::Complete { id, .. } => *id,
            RoomKind::Obstacle { item_id, index_in_item, .. } => (*item_id << 10) | *index_in_item,
        }
    }

    /// Java `ExpandableObject.getId()`.
    pub fn expandable_java_id(&self, e: Expandable) -> i32 {
        match e {
            Expandable::Door(d) => {
                let door = self.door(d);
                let id1 = self.room_java_id(door.first_room);
                let id2 = self.room_java_id(door.second_room);
                id1.min(id2).wrapping_mul(31).wrapping_add(id1.max(id2))
            }
            Expandable::Target(t) => {
                let td = self.target_door(t);
                31i32.wrapping_mul(td.item_id).wrapping_add(self.room_java_id(td.room))
            }
            Expandable::Drill(d) => self.drill(d).java_id(),
            Expandable::Page(p) => self.page(p).java_id(),
        }
    }

    /// Java `ExpandableObject.getShape()`.
    pub fn expandable_shape(&self, e: Expandable) -> TileShape {
        match e {
            Expandable::Door(d) => self.door_shape(d),
            Expandable::Target(t) => self.target_door(t).shape.clone(),
            Expandable::Drill(d) => self.drill(d).shape.clone(),
            Expandable::Page(p) => TileShape::IntBox(self.page(p).shape),
        }
    }

    /// Java `ExpandableObject.getDimension()`.
    pub fn expandable_dimension(&self, e: Expandable) -> i32 {
        match e {
            Expandable::Door(d) => self.door(d).dimension,
            _ => 2,
        }
    }

    /// Java `ExpandableObject.otherRoom(CompleteExpansionRoom room)`.
    pub fn expandable_other_room(&self, e: Expandable, room: Option<RoomId>) -> Option<RoomId> {
        match e {
            Expandable::Door(d) => room.and_then(|r| self.door_other_complete_room(d, r)),
            _ => None,
        }
    }

    /// Java `ExpandableObject.mazeSearchElementCount()` (panics for a door without sections like
    /// Java's NullPointerException).
    pub fn expandable_element_count(&self, e: Expandable) -> i32 {
        match e {
            Expandable::Door(d) => self.door(d).sections.as_ref().expect("ExpansionDoor.sectionArr is null").len() as i32,
            Expandable::Target(_) => 1,
            Expandable::Drill(d) => self.drill(d).elements.len() as i32,
            Expandable::Page(p) => self.page(p).elements.len() as i32,
        }
    }

    /// Java `ExpandableObject.getMazeSearchElement(index)`.
    pub fn element(&self, e: Expandable, index: i32) -> &MazeSearchElement {
        match e {
            Expandable::Door(d) => &self.door(d).sections.as_ref().expect("ExpansionDoor.sectionArr is null")[index as usize],
            Expandable::Target(t) => &self.target_door(t).element,
            Expandable::Drill(d) => &self.drill(d).elements[index as usize],
            Expandable::Page(p) => &self.page(p).elements[index as usize],
        }
    }

    /// Mutable [`Self::element`].
    pub fn element_mut(&mut self, e: Expandable, index: i32) -> &mut MazeSearchElement {
        match e {
            Expandable::Door(d) => &mut self.doors[d.0 as usize].sections.as_mut().expect("ExpansionDoor.sectionArr is null")[index as usize],
            Expandable::Target(t) => &mut self.target_doors[t.0 as usize].element,
            Expandable::Drill(d) => &mut self.drills[d.0 as usize].elements[index as usize],
            Expandable::Page(p) => &mut self.drill_page_array.pages[p.0 as usize].elements[index as usize],
        }
    }

    // ------------------------------------------------------------------------------------------
    // doors

    /// Java `ExpansionDoor.getShape()`: the intersection of the shapes of the two rooms.
    pub fn door_shape(&self, d: DoorId) -> TileShape {
        let door = self.door(d);
        self.room(door.first_room).shape().intersection(self.room(door.second_room).shape())
    }

    /// Java `ExpansionDoor.otherRoom(ExpansionRoom room)`.
    pub fn door_other_room(&self, d: DoorId, room: RoomId) -> Option<RoomId> {
        let door = self.door(d);
        if room == door.first_room {
            Some(door.second_room)
        } else if room == door.second_room {
            Some(door.first_room)
        } else {
            None
        }
    }

    /// Java `ExpansionDoor.otherRoom(CompleteExpansionRoom room)`: `None` if the other room is not
    /// complete.
    pub fn door_other_complete_room(&self, d: DoorId, room: RoomId) -> Option<RoomId> {
        let other = self.door_other_room(d, room)?;
        if self.room(other).is_complete_expansion_room() {
            Some(other)
        } else {
            None
        }
    }

    /// `new ExpansionDoor(first, second, dimension)` (not yet added to the rooms).
    pub(crate) fn new_door(&mut self, first: RoomId, second: RoomId, dimension: i32) -> DoorId {
        let id = DoorId(self.doors.len() as u32);
        self.doors.push(ExpansionDoor { first_room: first, second_room: second, dimension, sections: None });
        id
    }

    /// `new ExpansionDoor(first, second)`: the dimension is that of the shape intersection.
    pub(crate) fn new_door_auto(&mut self, first: RoomId, second: RoomId) -> DoorId {
        let dimension = self.room(first).shape().intersection(self.room(second).shape()).dimension();
        self.new_door(first, second, dimension)
    }

    /// Java `ExpansionRoom.addDoor(door)`.
    pub(crate) fn add_door(&mut self, room: RoomId, door: DoorId) {
        self.rooms[room.0 as usize].doors.push(door);
    }

    /// Java `ExpansionRoom.removeDoor(door)` (for an `ExpansionDoor`).
    pub(crate) fn remove_door(&mut self, room: RoomId, door: DoorId) -> bool {
        let doors = &mut self.rooms[room.0 as usize].doors;
        match doors.iter().position(|&d| d == door) {
            Some(i) => {
                doors.remove(i);
                true
            }
            None => false,
        }
    }

    /// Java `ExpansionRoom.doorExists(other)`.
    pub fn door_exists(&self, room: RoomId, other: RoomId) -> bool {
        self.room(room).doors.iter().any(|&d| {
            let door = self.door(d);
            door.first_room == other || door.second_room == other
        })
    }

    /// Java `ExpansionDoor.getSectionSegments(offset)` (allocates the sections).
    pub fn door_section_segments(&mut self, d: DoorId, offset_param: f64) -> Vec<FloatLine> {
        let offset = offset_param + TRACE_WIDTH_TOLERANCE as f64;
        let door_shape = self.door_shape(d);
        if door_shape.is_empty() {
            return Vec::new();
        }
        let door = self.door(d);
        let dimension = door.dimension;
        let (door_line_segment, shrinked_line_segment) = if dimension == 1 {
            let seg = door_shape.diagonal_corner_segment().expect("door shape is empty");
            (seg, seg.shrink_segment(offset))
        } else if dimension == 2 && self.room(door.first_room).is_complete_free() && self.room(door.second_room).is_complete_free() {
            // Overlapping doors at a corner possible in case of 90- or 45-degree routing.
            let Some(seg) = self.calc_door_line_segment(d, &door_shape) else {
                // CompleteFreeSpaceExpansionRoom inside other room
                return Vec::new();
            };
            if seg.b.distance_square(&seg.a) < 4.0 * offset * offset {
                // door is small, 2 dimensional small doors are not yet expanded.
                return Vec::new();
            }
            (seg, seg.shrink_segment(offset))
        } else {
            let gravity_point = door_shape.centre_of_gravity();
            let seg = FloatLine::new(gravity_point, gravity_point);
            (seg, seg)
        };
        let max_door_section_width = 10.0 * offset;
        let section_count = (door_line_segment.b.distance(&door_line_segment.a) / max_door_section_width) as i32 + 1;
        self.allocate_sections(d, section_count);
        shrinked_line_segment.divide_segment_into_sections(section_count)
    }

    /// Java `ExpansionDoor.allocateSections(sectionCount)`.
    fn allocate_sections(&mut self, d: DoorId, section_count: i32) {
        let door = &mut self.doors[d.0 as usize];
        if let Some(s) = &door.sections {
            if s.len() as i32 == section_count {
                return;
            }
        }
        door.sections = Some(vec![MazeSearchElement::default(); section_count.max(0) as usize]);
    }

    /// Java `ExpansionDoor.calcDoorLineSegment(doorShape)`.
    fn calc_door_line_segment(&self, d: DoorId, door_shape: &TileShape) -> Option<FloatLine> {
        let door = self.door(d);
        let first_room_shape = self.room(door.first_room).shape();
        let second_room_shape = self.room(door.second_room).shape();
        let mut first_corner: Option<Point> = None;
        let mut second_corner: Option<Point> = None;
        let corner_count = door_shape.border_line_count();
        for i in 0..corner_count {
            let current_corner = door_shape.corner(i);
            if !first_room_shape.contains_inside(&current_corner) && !second_room_shape.contains_inside(&current_corner) {
                // currentCorner is on the border of both room shapes.
                match &first_corner {
                    None => first_corner = Some(current_corner),
                    Some(f) => {
                        if *f != current_corner {
                            second_corner = Some(current_corner);
                            break;
                        }
                    }
                }
            }
        }
        match (first_corner, second_corner) {
            (Some(a), Some(b)) => Some(FloatLine::new(a.to_float(), b.to_float())),
            _ => None,
        }
    }

    // ------------------------------------------------------------------------------------------
    // rooms

    /// Java `addIncompleteExpansionRoom(shape, layer, containedShape)`.
    pub fn add_incomplete_expansion_room(&mut self, shape: Option<TileShape>, layer: LayerNo, contained_shape: Option<TileShape>) -> RoomId {
        let id = RoomId(self.rooms.len() as u32);
        self.rooms.push(Room { layer, doors: Vec::new(), kind: RoomKind::Incomplete { shape, contained_shape } });
        self.incomplete_list_exists = true;
        id
    }

    /// Java `removeIncompleteExpansionRoom(room)`. Fails (Java NullPointerException) if no
    /// incomplete room list exists.
    pub fn remove_incomplete_expansion_room(&mut self, room: RoomId) -> JResult<()> {
        self.remove_all_doors(room)?;
        if !self.incomplete_list_exists {
            return Err(JavaException);
        }
        Ok(())
    }

    /// Java `removeAllDoors(room)`.
    pub fn remove_all_doors(&mut self, room: RoomId) -> JResult<()> {
        let doors = self.room(room).doors.clone();
        for d in doors {
            let Some(other) = self.door_other_room(d, room) else {
                continue;
            };
            self.remove_door(other, d);
            if self.room(other).is_incomplete() {
                self.remove_incomplete_expansion_room(other)?;
            }
        }
        self.rooms[room.0 as usize].doors = Vec::new();
        if let RoomKind::Complete { target_doors, .. } = &mut self.rooms[room.0 as usize].kind {
            *target_doors = Vec::new();
        }
        Ok(())
    }

    /// Java `removeCompleteExpansionRoom(room)`: removes a complete room from the database and
    /// creates new incomplete rooms for the neighbours.
    pub fn remove_complete_expansion_room(&mut self, board: &mut RoutingBoard, room: RoomId) {
        let room_shape = self.room(room).shape().clone();
        let room_layer = self.room(room).layer;
        let room_doors = self.room(room).doors.clone();
        for d in room_doors {
            // (Java calls the CompleteExpansionRoom overload of otherRoom here)
            let Some(neighbour) = self.door_other_complete_room(d, room) else {
                continue;
            };
            self.remove_door(neighbour, d);
            let neighbour_shape = self.room(neighbour).shape().clone();
            let intersection = room_shape.intersection(&neighbour_shape);
            if intersection.dimension() == 1 {
                // add a new incomplete room to currentNeighbour.
                let touching_sides = room_shape.touching_sides(&neighbour_shape);
                let lines = vec![neighbour_shape.border_line(touching_sides[1]).opposite()];
                let new_shape = TileShape::Simplex(Simplex::get_instance(&lines));
                let new_room = self.add_incomplete_expansion_room(Some(new_shape), room_layer, Some(intersection));
                let new_door = self.new_door(neighbour, new_room, 1);
                self.add_door(neighbour, new_door);
                self.add_door(new_room, new_door);
            }
        }
        if self.remove_all_doors(room).is_err() {
            log::warn!("AutorouteEngine.remove_complete_expansion_room: exception in remove_all_doors");
        }
        board.search_trees_mut().tree_mut(self.tree).remove_room(RoomKey(room.0));
        match &mut self.complete_rooms {
            Some(list) => {
                if let Some(i) = list.iter().position(|&r| r == room) {
                    list.remove(i);
                }
            }
            None => log::warn!("AutorouteEngine.remove_complete_expansion_room: this.completeExpansionRooms is null"),
        }
        self.drill_page_array.invalidate(&room_shape);
    }

    /// Java `completeExpansionRoom(room)`: completes the shape of an incomplete room and returns
    /// the resulting complete rooms (Java catches all exceptions and returns an empty list).
    pub fn complete_expansion_room(&mut self, board: &mut RoutingBoard, room: IncompleteRef) -> Vec<RoomId> {
        // (a panic stands for a Java exception, which is caught here)
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.complete_expansion_room_impl(board, room))) {
            Ok(r) => {
                self.room_faults = 0;
                r.unwrap_or_default()
            }
            Err(_) => {
                log::error!("AutorouteEngine.complete_expansion_room: exception");
                // pcbkit F2: the search retries a room that failed to complete; a panic that repeats
                // would retry forever. After MAX_ROOM_FAULTS in a row, record the fault (the server
                // answers `internal`) and unwind out of the search, which ends the connection.
                self.room_faults += 1;
                if self.room_faults >= MAX_ROOM_FAULTS {
                    ENGINE_FAULT.store(true, std::sync::atomic::Ordering::SeqCst);
                    if let Some(stop) = &self.stop {
                        stop.request_stop();
                    }
                    std::panic::resume_unwind(Box::new("AutorouteEngine.complete_expansion_room: repeated exception"));
                }
                Vec::new()
            }
        }
    }

    fn complete_expansion_room_impl(&mut self, board: &mut RoutingBoard, room: IncompleteRef) -> JResult<Vec<RoomId>> {
        #[cfg(feature = "test-hooks")]
        {
            // FR_ENGINE_TEST_PANIC_ROOMS=<n>: every completion after the first n panics (a persistent engine fault)
            static CALLS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            if let Some(n) = std::env::var("FR_ENGINE_TEST_PANIC_ROOMS").ok().and_then(|v| v.parse::<u32>().ok()) {
                if CALLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst) >= n {
                    panic!("FR_ENGINE_TEST_PANIC_ROOMS");
                }
            }
        }
        let mut result = Vec::new();
        let mut from_door_shape: Option<TileShape> = None;
        let mut ignore_object: Option<TreeObject> = None;
        let value = match &room {
            IncompleteRef::Arena(r) => {
                for &d in &self.room(*r).doors {
                    let Some(other) = self.door_other_room(d, *r) else { continue };
                    if self.room(other).is_complete_free() && self.door(d).dimension == 2 {
                        from_door_shape = Some(self.door_shape(d));
                        ignore_object = Some(self.room_tree_object(other));
                        break;
                    }
                }
                let rm = self.room(*r);
                match &rm.kind {
                    RoomKind::Incomplete { shape, contained_shape } => {
                        IncompleteFreeSpaceExpansionRoom::new(shape.clone(), rm.layer, contained_shape.clone())
                    }
                    _ => panic!("complete_expansion_room: incomplete room expected"),
                }
            }
            IncompleteRef::Temp(v) => v.clone(),
        };
        let completed_shapes = board.complete_shape(self.tree, &value, self.net_number, ignore_object, from_door_shape.as_ref());
        match &room {
            IncompleteRef::Arena(r) => self.remove_incomplete_expansion_room(*r)?,
            IncompleteRef::Temp(_) => {
                if !self.incomplete_list_exists {
                    return Err(JavaException);
                }
            }
        }
        let mut is_first_completed_room = true;
        for current in completed_shapes {
            if current.shape.as_ref().map(|s| s.dimension()).unwrap_or(-1) != 2 {
                continue;
            }
            if is_first_completed_room {
                is_first_completed_room = false;
                if let Some(r) = self.add_complete_room(board, current)? {
                    result.push(r);
                }
            } else {
                // the shape of the first completed room may have changed and may intersect now
                // with the other shapes. Therefore, the completed shapes have to be recalculated.
                let current_completed = board.complete_shape(self.tree, &current, self.net_number, ignore_object, from_door_shape.as_ref());
                for tmp in current_completed {
                    if let Some(r) = self.add_complete_room(board, tmp)? {
                        result.push(r);
                    }
                }
            }
        }
        Ok(result)
    }

    /// Java `addCompleteRoom(room)`: calculates the doors and adds the completed room to the room
    /// database and the search tree.
    fn add_complete_room(&mut self, board: &mut RoutingBoard, mut room: IncompleteFreeSpaceExpansionRoom) -> JResult<Option<RoomId>> {
        let completed = self.calculate_doors(board, FromRoom::Incomplete(&mut room))?;
        let Some(completed) = completed else {
            return Ok(None);
        };
        let shape = self.room(completed).shape().clone();
        if shape.dimension() != 2 {
            return Ok(None);
        }
        self.complete_rooms.get_or_insert_with(Vec::new).push(completed);
        let layer = self.room(completed).layer;
        let id = self.room_java_id(completed);
        board.search_trees_mut().tree_mut(self.tree).insert_room(RoomKey(completed.0), id, shape, layer);
        Ok(Some(completed))
    }

    /// Java `calculateDoors(room)` = `SortedRoomNeighbours.complete(room, engine)`: dispatches on
    /// the class of the autoroute search tree.
    pub(crate) fn calculate_doors(&mut self, board: &mut RoutingBoard, room: FromRoom<'_>) -> JResult<Option<RoomId>> {
        match board.search_tree(self.tree).kind {
            TreeKind::NinetyDegree => super::neighbours90::calculate(self, board, room),
            TreeKind::FortyfiveDegree => super::neighbours45::calculate(self, board, room),
            TreeKind::Default => super::neighbours::calculate(self, board, room),
        }
    }

    /// Adds a new complete free space room (not yet in the tree) and returns it.
    pub(crate) fn new_complete_room(&mut self, shape: TileShape, layer: LayerNo, id: i32) -> RoomId {
        let r = RoomId(self.rooms.len() as u32);
        self.rooms.push(Room { layer, doors: Vec::new(), kind: RoomKind::Complete { shape, id, target_doors: Vec::new(), net_dependent: false } });
        r
    }

    /// Java `completeNeighbourRooms(room)`: completes the neighbour rooms of `room`, so that its
    /// doors do not change later on.
    pub fn complete_neighbour_rooms(&mut self, board: &mut RoutingBoard, room: RoomId) {
        // Keep v1.9 semantics: completing a neighbour can mutate door topology, so restart
        // iteration on the updated door set.
        let mut i = 0;
        while i < self.room(room).doors.len() {
            let d = self.room(room).doors[i];
            i += 1;
            let Some(neighbour) = self.door_other_room(d, room) else {
                continue;
            };
            match &self.room(neighbour).kind {
                RoomKind::Incomplete { .. } => {
                    self.complete_expansion_room(board, IncompleteRef::Arena(neighbour));
                    i = 0;
                }
                RoomKind::Obstacle { doors_calculated, .. } => {
                    if !*doors_calculated {
                        // (exceptions are not caught here in Java; they end the search)
                        let _ = self.calculate_doors(board, FromRoom::Obstacle(neighbour));
                        if let RoomKind::Obstacle { doors_calculated, .. } = &mut self.rooms[neighbour.0 as usize].kind {
                            *doors_calculated = true;
                        }
                    }
                }
                RoomKind::Complete { .. } => {}
            }
        }
    }

    /// Java `invalidateDrillPages(shape)`.
    pub fn invalidate_drill_pages(&mut self, shape: &TileShape) {
        self.drill_page_array.invalidate(shape);
    }

    /// Java `getRoomsWithTargetItems(items)`: the complete rooms with a target door to one of the
    /// items (Java `TreeSet`, ordered by descending room id).
    pub fn rooms_with_target_items(&self, items: &crate::board::ItemSet) -> Vec<RoomId> {
        let mut result: Vec<RoomId> = Vec::new();
        if let Some(list) = &self.complete_rooms {
            for &r in list {
                for &t in self.room(r).target_doors() {
                    let td = self.target_door(t);
                    if items.contains(crate::ids::ItemId(td.item_id)) && !result.contains(&r) {
                        result.push(r);
                    }
                }
            }
        }
        result.sort_by(|a, b| self.room_java_id(*b).wrapping_sub(self.room_java_id(*a)).cmp(&0));
        result
    }

    // ------------------------------------------------------------------------------------------
    // item autoroute info

    /// Java `item.getAutorouteInfo()` (created if necessary).
    pub fn item_info_mut(&mut self, item: ItemKey) -> &mut ItemAutorouteInfo {
        self.item_infos.entry(item).or_default()
    }

    /// True if the item has an autoroute info (Java `getAutorouteInfoPur() != null`).
    pub fn has_item_info(&self, item: ItemKey) -> bool {
        self.item_infos.contains_key(&item)
    }

    /// Java `isStartInfo()` of the item's autoroute info (false for a new info).
    pub fn is_start_item(&self, item: ItemKey) -> bool {
        self.item_infos.get(&item).map(|i| i.start_info).unwrap_or(false)
    }

    /// Java `TargetItemExpansionDoor.isDestinationDoor()`.
    pub fn is_destination_door(&self, t: TargetDoorId) -> bool {
        !self.is_start_item(self.target_door(t).item)
    }

    /// Java `ItemAutorouteInfo.getExpansionRoom(index, autorouteTree)`: the obstacle room of the
    /// item's tree shape `index`, created if necessary.
    pub fn get_expansion_room(&mut self, board: &RoutingBoard, item: ItemKey, index: i32) -> Option<RoomId> {
        let current_shape_count = board.tree_shape_count(self.tree, item);
        let tree = self.tree;
        let info = self.item_infos.entry(item).or_default();
        match &mut info.expansion_rooms {
            None => info.expansion_rooms = Some(vec![None; current_shape_count.max(0) as usize]),
            Some(v) => {
                if v.len() as i32 != current_shape_count {
                    v.resize(current_shape_count.max(0) as usize, None);
                }
            }
        }
        let len = info.expansion_rooms.as_ref().unwrap().len() as i32;
        if index < 0 || index >= len {
            log::warn!("ItemAutorouteInfo.get_expansion_room: index {index} out of range [0, {len})");
            return None;
        }
        if let Some(r) = info.expansion_rooms.as_ref().unwrap()[index as usize] {
            return Some(r);
        }
        // Java throws here; a degenerate shape (e.g. an imported trace segment whose offset shape
        // is empty) is skipped instead, as the callers already treat a missing room as no room.
        let Some(shape) = board.tree_shape(tree, item, index) else {
            static WARNED: std::sync::Once = std::sync::Once::new();
            WARNED.call_once(|| log::warn!("ObstacleExpansionRoom: tree shape is null, item skipped"));
            return None;
        };
        let layer = board.shape_layer(item, index);
        let item_id = board.item(item).id().0;
        let r = RoomId(self.rooms.len() as u32);
        self.rooms.push(Room {
            layer,
            doors: Vec::new(),
            kind: RoomKind::Obstacle { item, item_id, index_in_item: index, shape, doors_calculated: false },
        });
        self.item_infos.get_mut(&item).unwrap().expansion_rooms.as_mut().unwrap()[index as usize] = Some(r);
        Some(r)
    }

    /// Java `ObstacleExpansionRoom.createOverlapDoor(other)`.
    pub(crate) fn create_overlap_door(&mut self, board: &RoutingBoard, this: RoomId, other: RoomId) -> bool {
        if self.door_exists(this, other) {
            return false;
        }
        let (RoomKind::Obstacle { item: this_item, index_in_item: this_index, .. }, RoomKind::Obstacle { item: other_item, index_in_item: other_index, .. }) =
            (&self.room(this).kind, &self.room(other).kind)
        else {
            return false;
        };
        let (this_item, this_index, other_item, other_index) = (*this_item, *this_index, *other_item, *other_index);
        let a = board.item(this_item);
        let b = board.item(other_item);
        if !(a.is_routable() && b.is_routable()) {
            return false;
        }
        if !a.shares_net(b) {
            return false;
        }
        if this_item == other_item {
            if !a.is_trace() {
                return false;
            }
            // create only doors between consecutive trace segments
            if this_index != other_index + 1 && this_index != other_index - 1 {
                return false;
            }
        }
        let new_door = self.new_door(this, other, 2);
        self.add_door(this, new_door);
        self.add_door(other, new_door);
        true
    }

    /// Java `SortedRoomNeighbours.insertDoorOk(room1, room2, doorShape)` (door shape of
    /// dimension 1 expected).
    pub(crate) fn insert_door_ok(&self, board: &RoutingBoard, room1: RoomId, room2: RoomId, door_shape: &TileShape) -> bool {
        if self.door_exists(room1, room2) {
            return false;
        }
        let r1 = self.room(room1);
        let r2 = self.room(room2);
        if let (Some(i1), Some(i2)) = (r1.obstacle_item(), r2.obstacle_item()) {
            // insert only overlap_doors between items of the same net for performance reasons.
            return board.item(i1).shares_net(board.item(i2));
        }
        if !r1.is_obstacle() && !r2.is_obstacle() {
            return true;
        }
        // Insert 1 dimensional doors of trace rooms only, if they are parallel to the trace line.
        let mut door_line: Option<Line> = None;
        let mut prev_corner = door_shape.corner(0);
        let corner_count = door_shape.border_line_count();
        for i in 1..corner_count {
            let current_corner = door_shape.corner(i);
            if current_corner != prev_corner {
                door_line = Some(door_shape.border_line(i - 1));
                break;
            }
            prev_corner = current_corner;
        }
        if r1.is_obstacle() && !self.insert_door_ok_obstacle(board, room1, door_line.as_ref()) {
            return false;
        }
        if r2.is_obstacle() {
            return self.insert_door_ok_obstacle(board, room2, door_line.as_ref());
        }
        true
    }

    pub(crate) fn insert_door_ok_obstacle(&self, board: &RoutingBoard, room: RoomId, door_line: Option<&Line>) -> bool {
        let Some(door_line) = door_line else {
            log::warn!("SortedRoomNeighbours.insert_door_ok: doorLine is null");
            return false;
        };
        let RoomKind::Obstacle { item, index_in_item, .. } = &self.room(room).kind else {
            return true;
        };
        let it = board.item(*item);
        if let ItemKind::Trace(t) = &it.kind {
            let room_index = *index_in_item;
            if room_index == 0 || room_index == t.tile_shape_count() - 1 {
                let current_trace_line = &t.polyline().lines[(room_index + 1) as usize];
                return current_trace_line.is_parallel(door_line);
            }
        }
        true
    }

    /// Java `CompleteFreeSpaceExpansionRoom.calculateTargetDoors(ownNetObject, netNumber,
    /// autorouteSearchTree)` (without the `setNetDependent` of the caller variants).
    pub(crate) fn add_target_door_if_connected(&mut self, board: &RoutingBoard, room: RoomId, object: TreeObject, shape_index: i32, net_number: NetNo) {
        let Some(key) = object.item() else {
            return;
        };
        let item = board.item(key);
        if !item.is_connectable_class() || !item.contains_net(net_number) {
            return;
        }
        let Some(connection_shape) = board.trace_connection_shape(self.tree, key, shape_index) else {
            return;
        };
        if !self.room(room).shape().intersects_tile(&connection_shape) {
            return;
        }
        let item_shape = board.tree_shape(self.tree, key, shape_index).expect("TargetItemExpansionDoor: tree shape is null");
        let shape = item_shape.intersection(self.room(room).shape());
        let t = TargetDoorId(self.target_doors.len() as u32);
        self.target_doors.push(TargetItemExpansionDoor {
            item: key,
            item_id: item.id().0,
            tree_entry_no: shape_index,
            room,
            shape,
            element: MazeSearchElement::default(),
        });
        if let RoomKind::Complete { target_doors, .. } = &mut self.rooms[room.0 as usize].kind {
            target_doors.push(t);
        }
    }

    /// Java `CompleteFreeSpaceExpansionRoom.setNetDependent()`.
    pub(crate) fn set_net_dependent(&mut self, room: RoomId) {
        if let RoomKind::Complete { net_dependent, .. } = &mut self.rooms[room.0 as usize].kind {
            *net_dependent = true;
        }
    }

    // ------------------------------------------------------------------------------------------
    // drills

    fn new_drill(&mut self, drill: ExpansionDrill) -> DrillId {
        let id = DrillId(self.drills.len() as u32);
        self.drills.push(drill);
        id
    }

    /// Java `Via.getAutorouteDrillInfo(autorouteTree)`.
    pub fn via_drill_info(&mut self, board: &RoutingBoard, via: ItemKey) -> DrillId {
        if let Some(d) = self.item_infos.get(&via).and_then(|i| i.via_drill) {
            return d;
        }
        self.item_info_mut(via);
        let item = board.item(via);
        let center = item.center(board);
        let first = item.first_layer(board);
        let last = item.last_layer(board);
        let shape = TileShape::IntBox(TileShape::get_instance_point(&center));
        let drill = self.new_drill(ExpansionDrill::new(shape, center, first, last));
        for i in 0..(last - first + 1) {
            let r = self.get_expansion_room(board, via, i);
            self.drills[drill.0 as usize].room_arr[i as usize] = r;
        }
        self.item_info_mut(via).via_drill = Some(drill);
        drill
    }

    /// Java `ExpansionDrill.calculateExpansionRooms(autorouteEngine)`: looks for the expansion
    /// room of the drill on each layer, creating complete rooms where none exists. Returns false
    /// if that was not possible.
    fn calculate_drill_expansion_rooms(&mut self, board: &mut RoutingBoard, drill: DrillId) -> bool {
        let (location, first_layer, last_layer) = {
            let d = self.drill(drill);
            (d.location.clone(), d.first_layer, d.last_layer)
        };
        let search_shape = TileShape::IntBox(TileShape::get_instance_point(&location));
        let mut overlaps = board.overlapping_objects_in(self.tree, &ConvexShape::Tile(search_shape.clone()), -1, &[]);
        for i in first_layer..=last_layer {
            let mut found_room: Option<RoomId> = None;
            let mut k = 0;
            while k < overlaps.len() {
                let object = overlaps[k];
                let TreeObject::Room { key, .. } = object else {
                    overlaps.remove(k);
                    continue;
                };
                let r = RoomId(key.0);
                if self.room(r).layer == i {
                    found_room = Some(r);
                    overlaps.remove(k);
                    break;
                }
                k += 1;
            }
            if found_room.is_none() {
                // create a new expansion room on this layer
                let new_incomplete = IncompleteFreeSpaceExpansionRoom::new(None, i, Some(search_shape.clone()));
                let new_rooms = self.complete_expansion_room(board, IncompleteRef::Temp(new_incomplete));
                if new_rooms.len() != 1 {
                    // the size may be 0 because of an obstacle in the compensated tree at location
                    return false;
                }
                found_room = Some(new_rooms[0]);
            }
            self.drills[drill.0 as usize].room_arr[(i - first_layer) as usize] = found_room;
        }
        true
    }

    /// Java `DrillPage.calcPinCenterInDrill(drillShape, layer, board)`.
    fn calc_pin_center_in_drill(board: &RoutingBoard, drill_shape: &TileShape, layer: LayerNo) -> Option<Point> {
        let overlapping = board.overlapping_items(&fr_geom::Area::from(drill_shape.clone()), layer);
        let mut result = None;
        for key in overlapping.iter() {
            let item = board.item(key);
            if item.is_pin() && item.drill_allowed(board) && drill_shape.contains_inside(&item.center(board)) {
                result = Some(item.center(board));
            }
        }
        result
    }

    /// Java `DrillPage.getDrills(autorouteEngine, attachSmd)`.
    pub fn page_drills(&mut self, board: &mut RoutingBoard, page: PageId, attach_smd: bool) -> Vec<DrillId> {
        let pi = page.0 as usize;
        let needs_calc = {
            let p = &self.drill_page_array.pages[pi];
            p.drills.is_none() || self.net_number != p.net_number
        };
        if needs_calc {
            let net_number = self.net_number;
            let page_shape = self.drill_page_array.pages[pi].shape;
            self.drill_page_array.pages[pi].net_number = net_number;
            self.drill_page_array.pages[pi].drills = Some(Vec::new());
            let page_tile = TileShape::IntBox(page_shape);
            let overlaps = board.overlapping_tree_entries_list(self.tree, &ConvexShape::Tile(page_tile.clone()), -1, &[]);
            let mut cutout_shapes: Vec<TileShape> = Vec::new();
            // drills on top of existing vias are used in the ripup algorithm
            let mut prev_obstacle_shape = TileShape::IntBox(IntBox::EMPTY);
            for entry in overlaps {
                let Some(key) = entry.object.item() else { continue };
                let item = board.item(key);
                // (a via in a trace-only keepout only helps if two other layers can take its
                // traces: not on two-layer boards, where it only produced failing attempts)
                if item.is_drillable(net_number) && !(item.is_wire_obstacle_area() && board.layer_count() <= 2) {
                    continue;
                }
                if item.is_pin() && attach_smd && item.drill_allowed(board) && item.contains_net(net_number) {
                    continue;
                }
                let current_obstacle_shape = board.tree_shape(self.tree, key, entry.shape_index).expect("DrillPage: tree shape is null");
                if !prev_obstacle_shape.contains_tile_shape(&current_obstacle_shape) {
                    // Checked to avoid multiple cutout for example for vias with the same shape
                    // on all layers.
                    let current_cutout_shape = current_obstacle_shape.intersection(&page_tile);
                    if current_cutout_shape.dimension() == 2 {
                        cutout_shapes.push(current_cutout_shape);
                    }
                }
                prev_obstacle_shape = current_obstacle_shape;
            }
            let holes: Vec<PolylineShape> = cutout_shapes.into_iter().map(PolylineShape::from).collect();
            let shape_with_holes = PolylineArea::new(PolylineShape::from(page_tile), holes);
            let stop = self.stop.clone();
            let drill_shapes = shape_with_holes.split_to_convex_stoppable(stop.as_ref().map(|s| s as &dyn fr_geom::Stoppable));
            if let Some(drill_shapes) = drill_shapes {
                // Use the center points of these drill shapes to try making a via.
                let drill_first_layer = 0;
                let drill_last_layer = board.layer_count() - 1;
                for current_drill_shape in drill_shapes {
                    let mut current_drill_location: Option<Point> = None;
                    if attach_smd {
                        current_drill_location = Self::calc_pin_center_in_drill(board, &current_drill_shape, drill_first_layer);
                        if current_drill_location.is_none() {
                            current_drill_location = Self::calc_pin_center_in_drill(board, &current_drill_shape, drill_last_layer);
                        }
                    }
                    let location = current_drill_location.unwrap_or_else(|| Point::Int(current_drill_shape.centre_of_gravity().round()));
                    let new_drill = self.new_drill(ExpansionDrill::new(current_drill_shape, location, drill_first_layer, drill_last_layer));
                    if self.calculate_drill_expansion_rooms(board, new_drill) {
                        self.drill_page_array.pages[pi].drills.as_mut().unwrap().push(new_drill);
                    }
                }
            }
        }
        self.drill_page_array.pages[pi].drills.clone().unwrap_or_default()
    }

    /// Java `drillPageArray.overlappingPages(shape)`.
    pub fn overlapping_drill_pages(&self, shape: &TileShape) -> Vec<PageId> {
        self.drill_page_array.overlapping_pages(shape)
    }

}
