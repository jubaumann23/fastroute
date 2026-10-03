//! Port of `board/facade/BasicBoard.java`, `BoardItemRepository.java` and
//! `BoardConnectivityQueries.java` (the non-GUI parts).
//!
//! Differences to Java:
//! * No observers, no graphics update box, no serialization. Snapshots / undo are clones of the
//!   board (`Clone` is cheap: flat arenas, `Arc` for rules, library, components and shapes).
//!   `itemList.saveForUndo` calls are dropped.
//! * `RoutingBoard.additionalUpdateAfterChange` (maintaining the autoroute database) is the
//!   optional [`AutorouteMaintenance`] record: when set, the rooms of the autoroute tree that
//!   overlap a changed item are removed from the tree immediately (preserving the Java tree
//!   structure) and queued for the autorouter.
//! * `RoutingBoard.changedArea` is [`BasicBoard::changed_area`] (None for a plain board).

use std::collections::{BTreeSet, HashSet};
use std::sync::{Arc, OnceLock};

use fr_geom::prelude::*;
use fr_geom::{
    Area, ConvexShape, IntBox, IntOctagon, Point, Polyline, PolylineShape, Shape, TileShape, Vector,
};

use crate::autoroute::rooms::RoomKey;
use crate::datastructures::IdGenerator;
use crate::ids::{ClearanceClassNo, ComponentNo, FixedState, ItemId, LayerNo, NetNo, PadstackNo};
use crate::library::BoardLibrary;
use crate::rules::{BoardRules, Nets};
use crate::structure::{ChangedArea, Communication, Components, LayerStructure};

use super::item::{
    base_pin_name, BoardOutline, ComponentOutline, ConductionArea, Item, ItemKey, ItemKind, ObstacleArea, ObstacleKind, Pin, Via,
};
use super::item_list::{ItemListCursor, ItemRepository, ItemSet};
use super::search_tree::{SearchTreeManager, TreeEntry, TreeObject, DEFAULT_TREE};
use super::selection_filter::{ItemSelectionFilter, SelectableChoices};

/// Java `BasicBoard.MAX_NORMALIZE_ITERATIONS`.
pub const MAX_NORMALIZE_ITERATIONS: i32 = 2000;

/// State of `RoutingBoard.additionalUpdateAfterChange` while the autoroute engine maintains its
/// database (`autorouteEngine.maintainDatabase`). Filled by the board, drained by the
/// autorouter.
#[derive(Clone, Debug, Default)]
pub struct AutorouteMaintenance {
    /// Compensated clearance class of the autoroute search tree.
    pub tree_clearance_class: ClearanceClassNo,
    /// Rooms removed from the autoroute tree (in Java removal order); the autorouter must do
    /// the remaining `removeCompleteExpansionRoom` work (doors, room lists) for them.
    pub removed_rooms: Vec<RoomKey>,
    /// Shapes passed to `invalidateDrillPages`, in call order.
    pub invalidated_drill_shapes: Vec<TileShape>,
    /// Items whose autoroute info must be cleared (`item.clearAutorouteInfo()` of the hook,
    /// `Item.clearDerivedData()`, restored items of an undo), recorded while the board holds an
    /// autoroute engine.
    pub cleared_autoroute_info: Vec<ItemKey>,
    /// Java `autorouteEngine.maintainDatabase`: the hook is only active if true (the record
    /// exists whenever the board holds an autoroute engine).
    pub maintain_database: bool,
}

/// Java `BasicBoard` (items, search trees and elementary operations).
#[derive(Clone, Debug)]
pub struct BasicBoard {
    pub layer_structure: Arc<LayerStructure>,
    pub rules: Arc<BoardRules>,
    pub library: Arc<BoardLibrary>,
    pub components: Arc<Components>,
    pub communication: Communication,
    /// Bounding orthogonal rectangle of this board.
    pub bounding_box: IntBox,
    pub(crate) items: ItemRepository,
    pub(crate) search_trees: SearchTreeManager,
    pub(crate) normalize_suppressed_net_nos: BTreeSet<NetNo>,
    pub(crate) revision: i32,
    max_trace_half_width: i32,
    min_trace_half_width: i32,
    pub pre_existing_clearance_violations_count: i32,
    pub unfixable_clearance_violations_count: i32,
    /// Cache of `BoardOutline.getEdgePinNets()`.
    edge_pin_nets: OnceLock<Arc<HashSet<NetNo>>>,
    /// `RoutingBoard.changedArea` (None for a basic board).
    pub changed_area: Option<ChangedArea>,
    /// `RoutingBoard.additionalUpdateAfterChange` state (None = no autoroute database).
    pub autoroute_maintenance: Option<AutorouteMaintenance>,
    /// Which Java version to reproduce where the reference source and the 2.4.1 jar differ.
    pub java_variant: JavaVariant,
    /// fastroute: pins and vias of a net whose copper overlaps on a common layer are in contact
    /// (Freerouting requires equal centers). See [`Self::set_overlap_contacts`].
    pub(crate) overlap_contacts: bool,
    /// fastroute: the fanout's fallback to the board's default vias only takes vias of the
    /// net's own via clearance class (see `RoutingBoard::fanout`).
    pub fallback_vias_own_class: bool,
    /// Undo bookkeeping of the item list (Java `UndoableObjects` levels), see [`super::undo`].
    pub(crate) undo: super::undo::UndoJournal,
}

/// The reference source (`reference/freerouting/src`) is newer than the 2.4.1 jar used for the
/// Java baseline runs. Within the board model they differ in (found by comparing the bytecode
/// of the source compiled against the jar with the jar's classes):
/// * `BoardOutline`: the source exempts nets of pins at/over the board edge ("edge pin nets")
///   in `isTraceObstacle`, `isObstacle(trace)` and `blocksNets` (used by
///   `ShapeTraceEntries.storeItems`); the jar treats the outline as an obstacle for all traces.
/// * `Pin.isObstacle(other)`: the source never regards same-net pins (and netless sub-pads of
///   the same logical pad) as obstacles; the jar treats a same-net pin like any non-trace,
///   non-via item (`!drillAllowed() || !(other instanceof Via)`).
/// * `ShapeSearchTree.overlappingObjectsWithClearance(.., Set)` /
///   `overlappingItemsWithClearance`: the jar uses the compensated query when the tree is
///   clearance compensated (only the GUI enables compensation), the source always the raw one.
/// * Not modelled here (other units): `Item.clearanceViolations` (tolerance, board outline
///   handling; U6) and `BasicBoard.expandBoundingBoxToIncludeAllItems` (only in the source;
///   U11 loader).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum JavaVariant {
    /// `reference/freerouting/src` (the default).
    #[default]
    Source,
    /// `reference/bin/freerouting-2.4.1.jar`.
    Jar241,
}

impl BasicBoard {
    /// fastroute: counts overlapping copper of a net's pins and vias as contact (see
    /// `overlap_contacts`); renews the content epochs, so no cached contact or unrouted count
    /// of the other rule is used afterwards.
    /// Freerouting reads a `wire_keepout` as a plain keepout (also blocking vias): turns the
    /// trace-only keepouts into keepouts (`--parity`, `--no-enhancements`).
    pub fn wire_keepouts_as_keepouts(&mut self) {
        let keys: Vec<ItemKey> = self.items.iter().collect();
        for k in keys {
            if let super::item::ItemKind::ObstacleArea(a) = &mut self.items.get_mut(k).kind {
                if a.kind == super::item::ObstacleKind::WireKeepout {
                    a.kind = super::item::ObstacleKind::Keepout;
                }
            }
        }
        self.items.touch_global();
    }

    pub fn set_overlap_contacts(&mut self, on: bool) {
        self.overlap_contacts = on;
        self.items.touch_global();
    }

    /// Java `new BasicBoard(boundingBox, layerStructure, outlineShapes, outlineClClassNo, rules,
    /// communication)`. The library and components are passed in here (Java creates empty ones
    /// that the loader fills). Inserts the outline.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        bounding_box: IntBox,
        layer_structure: LayerStructure,
        outline_shapes: Vec<PolylineShape>,
        outline_cl_class_no: ClearanceClassNo,
        rules: BoardRules,
        library: BoardLibrary,
        components: Components,
        communication: Communication,
    ) -> BasicBoard {
        let mut board = BasicBoard {
            layer_structure: Arc::new(layer_structure),
            rules: Arc::new(rules),
            library: Arc::new(library),
            components: Arc::new(components),
            communication,
            bounding_box,
            items: ItemRepository::new(),
            search_trees: SearchTreeManager::default(),
            normalize_suppressed_net_nos: BTreeSet::new(),
            revision: 0,
            max_trace_half_width: 1000,
            min_trace_half_width: 10000,
            pre_existing_clearance_violations_count: 0,
            unfixable_clearance_violations_count: 0,
            edge_pin_nets: OnceLock::new(),
            changed_area: None,
            autoroute_maintenance: None,
            java_variant: JavaVariant::Source,
            overlap_contacts: false,
            fallback_vias_own_class: false,
            undo: super::undo::UndoJournal::default(),
        };
        board.insert_outline(outline_shapes, outline_cl_class_no);
        board
    }

    // ------------------------------------------------------------------------------------------
    // basic accessors

    /// Read access to an item (also removed ones until [`Self::compact`]).
    #[inline]
    pub fn item(&self, key: ItemKey) -> &Item {
        self.items.get(key)
    }

    /// Read access that returns `None` for stale keys.
    #[inline]
    pub fn try_item(&self, key: ItemKey) -> Option<&Item> {
        self.items.try_get(key)
    }

    /// Mutable access to an item. Must not be used to change ids, nets, component numbers or
    /// geometry of items on the board (use the dedicated board methods).
    #[inline]
    pub fn item_mut(&mut self, key: ItemKey) -> &mut Item {
        self.items.get_mut(key)
    }

    /// The item repository (list, indexes).
    pub fn item_repository(&self) -> &ItemRepository {
        &self.items
    }

    /// Java `getRevision()`.
    pub fn revision(&self) -> i32 {
        self.revision
    }

    /// Java `incrementRevision()`.
    pub fn increment_revision(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    /// Java `getLayerCount()`.
    #[inline]
    pub fn layer_count(&self) -> i32 {
        self.layer_structure.layers.len() as i32
    }

    /// Java `getMaxTraceHalfWidth()`.
    pub fn max_trace_half_width(&self) -> i32 {
        self.max_trace_half_width
    }

    /// Java `getMinTraceHalfWidth()`.
    pub fn min_trace_half_width(&self) -> i32 {
        self.min_trace_half_width
    }

    /// Java `getBoundingBox()`.
    pub fn bounding_box(&self) -> IntBox {
        self.bounding_box
    }

    /// Java `getBoundingBox(Collection<Item>)`.
    pub fn bounding_box_of(&self, keys: impl IntoIterator<Item = ItemKey>) -> IntBox {
        let mut result = IntBox::EMPTY;
        for k in keys {
            result = result.union_int_box(&self.item(k).bounding_box(self));
        }
        result
    }

    /// Java `contains(Point)`.
    pub fn contains(&self, point: &Point) -> bool {
        point.is_contained_in(&self.bounding_box)
    }

    /// Java `clearanceValue(class1, class2, layer)` (with safety margin).
    pub fn clearance_value(&self, class1: ClearanceClassNo, class2: ClearanceClassNo, layer: LayerNo) -> i32 {
        self.rules.clearance_matrix.get_value(class1, class2, layer, true)
    }

    /// Mutable access to the rules (copy on write if shared with a snapshot).
    pub fn rules_mut(&mut self) -> &mut BoardRules {
        self.items.touch_global();
        Arc::make_mut(&mut self.rules)
    }

    /// Mutable access to the components (copy on write if shared with a snapshot).
    pub fn components_mut(&mut self) -> &mut Components {
        self.items.touch_global();
        Arc::make_mut(&mut self.components)
    }

    /// Mutable access to the library (copy on write if shared with a snapshot).
    pub fn library_mut(&mut self) -> &mut BoardLibrary {
        self.items.touch_global();
        Arc::make_mut(&mut self.library)
    }

    fn new_item_id(&mut self) -> ItemId {
        ItemId(self.communication.id_generator.new_id())
    }

    /// Frees removed items (tombstones) that are neither listed nor on the board; their keys
    /// become stale. Returns the freed keys.
    pub fn compact(&mut self) -> Vec<ItemKey> {
        let freed = self.items.compact();
        for key in &freed {
            self.clear_search_tree_entries(*key);
        }
        freed
    }

    // ------------------------------------------------------------------------------------------
    // item list access

    /// Java `itemList.startReadObject()`.
    pub fn item_cursor(&self) -> ItemListCursor {
        self.items.cursor()
    }

    /// Java `itemList.readObject(it)`.
    pub fn cursor_next(&self, cursor: &mut ItemListCursor) -> Option<ItemKey> {
        self.items.cursor_next(cursor)
    }

    /// Java `getItems()`: all items in list order (descending id).
    pub fn get_items(&self) -> Vec<ItemKey> {
        self.items.keys()
    }

    /// Java `getItem(id)`.
    pub fn get_item(&self, id: ItemId) -> Option<ItemKey> {
        self.items.by_id(id)
    }

    /// Java `getConnectableItems(netNumber)`.
    pub fn get_connectable_items(&self, net_number: NetNo) -> Vec<ItemKey> {
        self.items.net_items(net_number).filter(|k| self.item(*k).is_connectable_class()).collect()
    }

    /// Java `connectableItemCount(netNumber)`.
    pub fn connectable_item_count(&self, net_number: NetNo) -> i32 {
        self.items.net_items(net_number).filter(|k| self.item(*k).is_connectable_class()).count() as i32
    }

    /// Java `getComponentItems(componentId)`.
    pub fn get_component_items(&self, component_no: ComponentNo) -> Vec<ItemKey> {
        self.items.component_items(component_no).collect()
    }

    /// Java `getComponentPins(componentId)`.
    pub fn get_component_pins(&self, component_no: ComponentNo) -> Vec<ItemKey> {
        self.items.component_items(component_no).filter(|k| self.item(*k).is_pin()).collect()
    }

    /// Java `getPin(componentId, pinIndex)`.
    pub fn get_pin(&self, component_no: ComponentNo, pin_index: i32) -> Option<ItemKey> {
        self.items
            .component_items(component_no)
            .find(|k| matches!(&self.item(*k).kind, ItemKind::Pin(p) if p.pin_index == pin_index))
    }

    /// Java `getConductionAreas()`.
    pub fn get_conduction_areas(&self) -> Vec<ItemKey> {
        self.items.iter().filter(|k| self.item(*k).is_conduction_area()).collect()
    }

    /// Java `getPins()`.
    pub fn get_pins(&self) -> Vec<ItemKey> {
        self.items.iter().filter(|k| self.item(*k).is_pin()).collect()
    }

    /// Java `getSmdPins()`.
    pub fn get_smd_pins(&self) -> Vec<ItemKey> {
        self.items
            .iter()
            .filter(|k| {
                let item = self.item(*k);
                item.is_pin() && item.first_layer(self) == item.last_layer(self)
            })
            .collect()
    }

    /// Java `getVias()`.
    pub fn get_vias(&self) -> Vec<ItemKey> {
        self.items.iter().filter(|k| self.item(*k).is_via()).collect()
    }

    /// Java `getTraces()`.
    pub fn get_traces(&self) -> Vec<ItemKey> {
        self.items.iter().filter(|k| self.item(*k).is_trace()).collect()
    }

    /// Java `cumulativeTraceLength()` (summed in list order).
    pub fn cumulative_trace_length(&self) -> f64 {
        let mut result = 0.0;
        for k in self.items.iter() {
            if let Some(t) = self.item(k).as_trace() {
                result += t.length();
            }
        }
        result
    }

    /// Java `getNon45DegreeTraceCount()`.
    pub fn non_45_degree_trace_count(&self) -> i32 {
        self.items
            .iter()
            .filter(|k| matches!(self.item(*k).as_trace(), Some(t) if !t.polyline.is_multiple_of_45_degree()))
            .count() as i32
    }

    /// Java `getOutline()`: the first board outline in list order.
    pub fn get_outline(&self) -> Option<ItemKey> {
        self.items.iter().find(|k| self.item(*k).is_board_outline())
    }

    // ------------------------------------------------------------------------------------------
    // insertion

    /// Java `insertItem(item)` (`BoardItemRepository.insertItem`): inserts a new item into the
    /// arena, the list and the search trees. Returns its key.
    pub fn insert_item(&mut self, mut item: Item) -> ItemKey {
        let class_count = self.rules.clearance_matrix.get_class_count();
        if item.clearance_class < 0 || item.clearance_class >= class_count {
            log::warn!("LayeredBoard.insert_item: clearanceClass no out of range");
            item.clearance_class = 0;
        }
        let is_pin_or_outline = item.is_pin() || item.is_board_outline();
        let key = self.items.alloc(item);
        self.items.list_insert(key);
        self.journal_insert(key);
        self.tree_insert(key);
        self.additional_update_after_change(key);
        self.increment_revision();
        if is_pin_or_outline {
            self.invalidate_edge_pin_net_cache();
        }
        key
    }

    /// Re-inserts an existing arena item (e.g. a removed one) into the list and trees.
    pub fn reinsert_item(&mut self, key: ItemKey) {
        let is_pin_or_outline = self.item(key).is_pin() || self.item(key).is_board_outline();
        self.items.list_insert(key);
        self.journal_insert(key);
        self.tree_insert(key);
        self.additional_update_after_change(key);
        self.increment_revision();
        if is_pin_or_outline {
            self.invalidate_edge_pin_net_cache();
        }
    }

    /// Java `insertTraceWithoutCleaning(polyline, layer, halfWidth, netNumbers, clearanceClass,
    /// fixedState)`. Returns the new trace, or `None` if nothing was inserted. The lines of
    /// `polyline` are new objects (see [`Self::insert_trace_without_cleaning_tracked`]).
    pub fn insert_trace_without_cleaning(
        &mut self,
        polyline: Polyline,
        layer: LayerNo,
        half_width: i32,
        net_numbers: &[NetNo],
        clearance_class: ClearanceClassNo,
        fixed_state: FixedState,
    ) -> Option<ItemKey> {
        let tp = super::optimize::tracked::TPolyline::fresh(polyline);
        self.insert_trace_without_cleaning_tracked(tp, layer, half_width, net_numbers, clearance_class, fixed_state)
    }

    /// [`Self::insert_trace_without_cleaning`] with the Java identities of the lines.
    pub fn insert_trace_without_cleaning_tracked(
        &mut self,
        polyline: super::optimize::tracked::TPolyline,
        layer: LayerNo,
        half_width: i32,
        net_numbers: &[NetNo],
        clearance_class: ClearanceClassNo,
        fixed_state: FixedState,
    ) -> Option<ItemKey> {
        if polyline.polyline.corner_count() < 2 {
            return None;
        }
        let id = self.new_item_id();
        let item =
            Item::new_trace_tracked(id, polyline, layer, half_width, net_numbers, clearance_class, 0, fixed_state, self.layer_count());
        if item.first_corner() == item.last_corner() && fixed_state < FixedState::UserFixed {
            return None;
        }
        let nets_normal = item.nets_normal();
        let key = self.insert_item(item);
        if nets_normal {
            self.max_trace_half_width = self.max_trace_half_width.max(half_width);
            self.min_trace_half_width = self.min_trace_half_width.min(half_width);
        }
        Some(key)
    }

    /// Java `insertTrace(Polyline, ...)`: inserts and normalizes the new trace.
    pub fn insert_trace(
        &mut self,
        polyline: Polyline,
        layer: LayerNo,
        half_width: i32,
        net_numbers: &[NetNo],
        clearance_class: ClearanceClassNo,
        fixed_state: FixedState,
    ) {
        let tp = super::optimize::tracked::TPolyline::fresh(polyline);
        self.insert_trace_tracked(tp, layer, half_width, net_numbers, clearance_class, fixed_state);
    }

    /// [`Self::insert_trace`] with the Java identities of the lines.
    pub fn insert_trace_tracked(
        &mut self,
        polyline: super::optimize::tracked::TPolyline,
        layer: LayerNo,
        half_width: i32,
        net_numbers: &[NetNo],
        clearance_class: ClearanceClassNo,
        fixed_state: FixedState,
    ) {
        let Some(new_trace) = self.insert_trace_without_cleaning_tracked(polyline, layer, half_width, net_numbers, clearance_class, fixed_state)
        else {
            return;
        };
        let clip_shape = self.changed_area.as_ref().map(|c| c.get_area(layer));
        // Java catches exceptions of the normalization here and logs a warning.
        self.normalize_trace(new_trace, clip_shape.as_ref());
    }

    /// Java `insertTrace(Point[], ...)`.
    pub fn insert_trace_points(
        &mut self,
        points: &[Point],
        layer: LayerNo,
        half_width: i32,
        net_numbers: &[NetNo],
        clearance_class: ClearanceClassNo,
        fixed_state: FixedState,
    ) {
        for p in points {
            if !self.bounding_box.contains(p) {
                log::warn!("LayeredBoard.insert_trace: input point out of range");
            }
        }
        let poly = Polyline::from_points(points);
        self.insert_trace(poly, layer, half_width, net_numbers, clearance_class, fixed_state);
    }

    /// Java `insertVia(padstack, center, netNumbers, clearanceClassIndex, fixedState,
    /// attachAllowed)`: inserts the via and splits the traces of its nets at the center.
    pub fn insert_via(
        &mut self,
        padstack: PadstackNo,
        center: Point,
        net_numbers: &[NetNo],
        clearance_class: ClearanceClassNo,
        fixed_state: FixedState,
        attach_allowed: bool,
    ) -> ItemKey {
        let key = self.insert_via_item(padstack, center.clone(), net_numbers, clearance_class, fixed_state, attach_allowed, false, -1);
        let (from_layer, to_layer) = {
            let p = self.library.padstacks.get(padstack).expect("insertVia: padstack not found");
            (p.from_layer(), p.to_layer())
        };
        for i in from_layer..to_layer {
            for &net in net_numbers {
                self.split_traces(&center, i, net);
            }
        }
        key
    }

    /// Java `insertEscapeVia(...)`.
    pub fn insert_escape_via(
        &mut self,
        padstack: PadstackNo,
        center: Point,
        net_numbers: &[NetNo],
        clearance_class: ClearanceClassNo,
        fixed_state: FixedState,
        smd_layer: LayerNo,
    ) -> ItemKey {
        let key = self.insert_via_item(padstack, center.clone(), net_numbers, clearance_class, fixed_state, true, true, smd_layer);
        let (from_layer, to_layer) = {
            let p = self.library.padstacks.get(padstack).expect("insertEscapeVia: padstack not found");
            (p.from_layer(), p.to_layer())
        };
        for i in from_layer..=to_layer {
            for &net in net_numbers {
                self.split_traces(&center, i, net);
            }
        }
        key
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_via_item(
        &mut self,
        padstack: PadstackNo,
        center: Point,
        net_numbers: &[NetNo],
        clearance_class: ClearanceClassNo,
        fixed_state: FixedState,
        attach_allowed: bool,
        is_escape_via: bool,
        escape_via_smd_layer: LayerNo,
    ) -> ItemKey {
        let id = self.new_item_id();
        let via = Via {
            padstack,
            center,
            attach_allowed,
            is_escape_via,
            escape_via_smd_layer,
            shapes: OnceLock::new(),
        };
        let item = Item::new_header(id, net_numbers, clearance_class, 0, fixed_state, ItemKind::Via(via));
        self.insert_item(item)
    }

    /// Java `insertPin(componentId, pinIndex, netNumbers, clearanceClassIndex, fixedState)`.
    pub fn insert_pin(
        &mut self,
        component_no: ComponentNo,
        pin_index: i32,
        net_numbers: &[NetNo],
        clearance_class: ClearanceClassNo,
        fixed_state: FixedState,
    ) -> ItemKey {
        let id = self.new_item_id();
        let pin = Pin { pin_index, changed_to: None, center: OnceLock::new(), shapes: OnceLock::new() };
        let item = Item::new_header(id, net_numbers, clearance_class, component_no, fixed_state, ItemKind::Pin(pin));
        self.insert_item(item)
    }

    /// Java `insertObstacle`, `insertViaObstacle` and `insertComponentObstacle` (both
    /// overloads; the short ones use translation zero, rotation 0, no side change, component 0
    /// and no name).
    #[allow(clippy::too_many_arguments)]
    pub fn insert_obstacle_area(
        &mut self,
        kind: ObstacleKind,
        area: Area,
        layer: LayerNo,
        translation: Vector,
        rotation_in_degree: f64,
        side_changed: bool,
        clearance_class: ClearanceClassNo,
        component_no: ComponentNo,
        name: Option<String>,
        fixed_state: FixedState,
    ) -> ItemKey {
        let id = self.new_item_id();
        let a = ObstacleArea::new(kind, area, layer, translation, rotation_in_degree, side_changed, name);
        let item = Item::new_header(id, &[], clearance_class, component_no, fixed_state, ItemKind::ObstacleArea(a));
        self.insert_item(item)
    }

    /// Java `insertObstacle(area, layer, clearanceClassIndex, fixedState)`.
    pub fn insert_obstacle(&mut self, area: Area, layer: LayerNo, clearance_class: ClearanceClassNo, fixed_state: FixedState) -> ItemKey {
        self.insert_obstacle_area(ObstacleKind::Keepout, area, layer, Vector::ZERO, 0.0, false, clearance_class, 0, None, fixed_state)
    }

    /// Java `insertComponentOutline(...)`. Returns `None` for an unbounded area.
    #[allow(clippy::too_many_arguments)]
    pub fn insert_component_outline(
        &mut self,
        area: Area,
        is_front: bool,
        translation: Vector,
        rotation_in_degree: f64,
        component_no: ComponentNo,
        is_courtyard: bool,
        is_fabrication: bool,
        is_closed: bool,
        fixed_state: FixedState,
    ) -> Option<ItemKey> {
        if !area.is_bounded() {
            log::warn!("BasicBoard.insert_component_outline: area is not bounded");
            return None;
        }
        let id = self.new_item_id();
        let o = ComponentOutline {
            relative_area: Arc::new(area),
            translation,
            rotation_in_degree,
            is_front,
            is_courtyard,
            is_fabrication,
            is_closed,
            absolute_area: OnceLock::new(),
        };
        let item = Item::new_header(id, &[], 0, component_no, fixed_state, ItemKind::ComponentOutline(o));
        Some(self.insert_item(item))
    }

    /// Java `insertConductionArea(area, layer, netNumbers, clearanceClassIndex, isObstacle,
    /// fixedState)` generalized with the placement parameters of the `ConductionArea`
    /// constructor (used for component conduction areas by the loader).
    #[allow(clippy::too_many_arguments)]
    pub fn insert_conduction_area_placed(
        &mut self,
        area: Area,
        layer: LayerNo,
        translation: Vector,
        rotation_in_degree: f64,
        side_changed: bool,
        net_numbers: &[NetNo],
        clearance_class: ClearanceClassNo,
        component_no: ComponentNo,
        name: Option<String>,
        is_obstacle: bool,
        fixed_state: FixedState,
    ) -> ItemKey {
        let id = self.new_item_id();
        let c = ConductionArea {
            area: ObstacleArea::new(ObstacleKind::Keepout, area, layer, translation, rotation_in_degree, side_changed, name),
            is_obstacle,
            is_filled: true,
        };
        let item = Item::new_header(id, net_numbers, clearance_class, component_no, fixed_state, ItemKind::ConductionArea(c));
        self.insert_item(item)
    }

    /// Java `insertConductionArea(area, layer, netNumbers, clearanceClassIndex, isObstacle,
    /// fixedState)`.
    pub fn insert_conduction_area(
        &mut self,
        area: Area,
        layer: LayerNo,
        net_numbers: &[NetNo],
        clearance_class: ClearanceClassNo,
        is_obstacle: bool,
        fixed_state: FixedState,
    ) -> ItemKey {
        self.insert_conduction_area_placed(area, layer, Vector::ZERO, 0.0, false, net_numbers, clearance_class, 0, None, is_obstacle, fixed_state)
    }

    /// Java `insertOutline(outlineShapes, clearanceClassIndex)`.
    pub fn insert_outline(&mut self, outline_shapes: Vec<PolylineShape>, clearance_class: ClearanceClassNo) -> ItemKey {
        let id = self.new_item_id();
        let o = BoardOutline { shapes: Arc::from(outline_shapes), keepout_area: OnceLock::new(), keepout_outside_outline: false };
        let item = Item::new_header(id, &[], clearance_class, 0, FixedState::SystemFixed, ItemKind::BoardOutline(o));
        self.insert_item(item)
    }

    /// Java `BoardOutline.generateKeepoutOutside(value)`: reinserts the outline if the value
    /// changes.
    pub fn generate_keepout_outside(&mut self, outline: ItemKey, value: bool) {
        let ItemKind::BoardOutline(o) = &mut self.items.get_mut(outline).kind else {
            panic!("generate_keepout_outside: not an outline");
        };
        if o.keepout_outside_outline == value {
            return;
        }
        o.keepout_outside_outline = value;
        self.tree_remove(outline);
        self.tree_insert(outline);
    }

    // ------------------------------------------------------------------------------------------
    // removal

    /// Java `removeItem(item)` (`BoardItemRepository.removeItem`): items whose deletion is
    /// forbidden are not removed. The item stays readable (tombstone).
    pub fn remove_item(&mut self, key: ItemKey) {
        if self.is_deletion_forbidden(key) {
            return;
        }
        self.additional_update_after_change(key);
        self.tree_remove(key);
        self.journal_delete(key);
        self.items.list_remove(key);
        self.increment_revision();
        let item = self.item(key);
        if item.is_pin() || item.is_board_outline() {
            self.invalidate_edge_pin_net_cache();
        }
    }

    /// Java `removeItems(itemList)`: returns false if some items could not be removed.
    pub fn remove_items(&mut self, keys: impl IntoIterator<Item = ItemKey>) -> bool {
        let mut result = true;
        for key in keys {
            if self.is_deletion_forbidden(key) || self.item(key).is_user_fixed() {
                result = false;
            } else {
                self.remove_item(key);
            }
        }
        result
    }

    /// Java `deleteAllTracksAndVias()`: deletes all traces and vias from the item list. Like
    /// Java, the items are *not* removed from the search trees and stay "on the board".
    pub fn delete_all_tracks_and_vias(&mut self) {
        let mut cursor = self.items.cursor();
        while let Some(key) = self.items.cursor_next(&mut cursor) {
            let item = self.item(key);
            if item.is_trace() || item.is_via() {
                self.journal_delete(key);
                self.items.list_remove(key);
            }
        }
    }

    /// Java `unfillConductionAreas()`.
    pub fn unfill_conduction_areas(&mut self) {
        self.rules_mut().set_ignore_conduction(true);
        let mut cursor = self.items.cursor();
        while let Some(key) = self.items.cursor_next(&mut cursor) {
            if let ItemKind::ConductionArea(c) = &mut self.items.get_mut(key).kind {
                c.is_filled = false;
                c.is_obstacle = false;
            }
            if self.item(key).is_conduction_area() {
                self.clear_derived_data(key);
            }
        }
        self.reinsert_tree_items();
    }

    /// Java `makeConductive(area, netNumber)`: replaces an obstacle area by a conduction area.
    pub fn make_conductive(&mut self, area: ItemKey, net_number: NetNo) -> ItemKey {
        let item = self.item(area);
        let a = item.as_obstacle_area().expect("makeConductive: not an obstacle area").clone();
        let new_item = Item::new_header(
            ItemId(0),
            &[net_number],
            item.clearance_class,
            item.component_no,
            item.fixed_state,
            ItemKind::ConductionArea(ConductionArea {
                area: ObstacleArea::new(
                    ObstacleKind::Keepout,
                    (*a.relative_area).clone(),
                    a.layer,
                    a.translation.clone(),
                    a.rotation_in_degree,
                    a.side_changed,
                    a.name.clone(),
                ),
                is_obstacle: true,
                is_filled: true,
            }),
        );
        let mut new_item = new_item;
        new_item.id = self.new_item_id();
        self.remove_item(area);
        self.insert_item(new_item)
    }

    // ------------------------------------------------------------------------------------------
    // item property changes

    /// Java `Item.isDeletionForbidden()`.
    pub fn is_deletion_forbidden(&self, key: ItemKey) -> bool {
        let item = self.item(key);
        if item.component_no > 0 || item.is_user_fixed() {
            return true;
        }
        if let ItemKind::ConductionArea(c) = &item.kind {
            return !self.layer_structure.layers[c.area.layer as usize].is_signal;
        }
        false
    }

    /// Java `Trace.isShoveFixed()` / `Item.isShoveFixed()`.
    pub fn is_shove_fixed(&self, key: ItemKey) -> bool {
        let item = self.item(key);
        if item.is_shove_fixed_state() {
            return true;
        }
        if item.is_trace() {
            for &n in &item.net_numbers {
                if Nets::is_normal_net_number(n) && self.rules.net_class_of(n).is_shove_fixed() {
                    return true;
                }
            }
        }
        false
    }

    /// Java `Item.assignNetNo(netNumber)`.
    pub fn assign_net_no(&mut self, key: ItemKey, net_number: NetNo) {
        if !Nets::is_normal_net_number(net_number) {
            return;
        }
        if net_number > self.rules.nets.max_net_number() {
            log::warn!("Item.assign_net_no: netNumber to big");
            return;
        }
        self.save_for_undo(key);
        let mut nets = self.item(key).net_numbers.clone();
        if net_number <= 0 {
            nets.clear();
        } else {
            if nets.is_empty() {
                nets.push(0);
            } else if nets.len() > 1 {
                log::warn!("Item.assign_net_no: unexpected netCount > 1");
            }
            nets[0] = net_number;
        }
        self.items.set_nets(key, nets);
        if self.item(key).is_pin() {
            self.invalidate_edge_pin_net_cache();
        }
    }

    /// Java `Item.removeFromNet(netNumber)`.
    pub fn remove_from_net(&mut self, key: ItemKey, net_number: NetNo) -> bool {
        let nets = &self.item(key).net_numbers;
        let Some(found) = nets.iter().rposition(|&n| n == net_number) else {
            return false;
        };
        let mut new_nets = nets.clone();
        new_nets.remove(found);
        self.items.set_nets(key, new_nets);
        true
    }

    /// Java `Item.setClearanceClassIndex(index)`.
    pub fn set_clearance_class_index(&mut self, key: ItemKey, index: ClearanceClassNo) {
        if index < 0 || index >= self.rules.clearance_matrix.get_class_count() {
            log::warn!("Item.set_clearance_class_no: index out of range");
            return;
        }
        self.items.get_mut(key).clearance_class = index;
    }

    /// Java `Item.changeClearanceClassIndex(index)`.
    pub fn change_clearance_class_index(&mut self, key: ItemKey, index: ClearanceClassNo) {
        if index < 0 || index >= self.rules.clearance_matrix.get_class_count() {
            log::warn!("Item.set_clearance_class_no: index out of range");
            return;
        }
        self.items.get_mut(key).clearance_class = index;
        self.clear_derived_data(key);
        if self.is_clearance_compensation_used() {
            self.tree_remove(key);
            self.tree_insert(key);
        }
    }

    /// Java `Item.assignComponentId(id)`.
    pub fn assign_component_no(&mut self, key: ItemKey, component_no: ComponentNo) {
        self.items.set_component(key, component_no);
    }

    /// Java `ConductionArea.setIsObstacle`.
    pub fn set_conduction_area_is_obstacle(&mut self, key: ItemKey, value: bool) {
        if let ItemKind::ConductionArea(c) = &mut self.items.get_mut(key).kind {
            c.is_obstacle = value;
        }
    }

    /// Java `Via.setPadstack` (the caller reinserts the via if it is on the board).
    pub fn set_via_padstack(&mut self, key: ItemKey, padstack: PadstackNo) {
        if let ItemKind::Via(v) = &mut self.items.get_mut(key).kind {
            v.padstack = padstack;
        }
    }

    /// Java `Pin.swap(other)`.
    pub fn swap_pins(&mut self, a: ItemKey, b: ItemKey) -> bool {
        if self.item(a).net_count() > 1 || self.item(b).net_count() > 1 {
            log::warn!("Pin.swap not yet implemented for pins belonging to more than 1 net ");
            return false;
        }
        let a_net = if self.item(a).net_count() > 0 { self.item(a).net_number(0) } else { 0 };
        let b_net = if self.item(b).net_count() > 0 { self.item(b).net_number(0) } else { 0 };
        self.assign_net_no(a, b_net);
        self.assign_net_no(b, a_net);
        let (a_id, b_id) = (self.item(a).id(), self.item(b).id());
        let a_changed = self.item(a).as_pin().unwrap().changed_to.unwrap_or(a_id);
        let b_changed = self.item(b).as_pin().unwrap().changed_to.unwrap_or(b_id);
        if let ItemKind::Pin(p) = &mut self.items.get_mut(a).kind {
            p.changed_to = if b_changed == a_id { None } else { Some(b_changed) };
        }
        if let ItemKind::Pin(p) = &mut self.items.get_mut(b).kind {
            p.changed_to = if a_changed == b_id { None } else { Some(a_changed) };
        }
        true
    }

    /// Java `Item.moveBy(vector)` for traces, vias and areas (translates the item in the
    /// board; the extra connection traces of `DrillItem.moveBy` are not inserted).
    pub fn translate_item(&mut self, key: ItemKey, vector: &Vector) {
        self.save_for_undo(key);
        self.tree_remove(key);
        {
            let item = self.items.get_mut(key);
            match &mut item.kind {
                ItemKind::Trace(t) => {
                    // (Java keeps the line objects only for a zero vector)
                    if *vector != Vector::ZERO {
                        t.polyline = t.polyline.translate_by(vector);
                        t.line_ids = super::optimize::tracked::fresh_line_ids(t.polyline.lines.len());
                    }
                }
                ItemKind::Via(v) => v.center = v.center.translate_by(vector),
                ItemKind::Pin(p) => {
                    if let Some(c) = p.center.get().cloned() {
                        p.center = OnceLock::from(c.translate_by(vector));
                    }
                }
                ItemKind::ObstacleArea(a) => a.translation = a.translation.add(vector),
                ItemKind::ConductionArea(c) => c.area.translation = c.area.translation.add(vector),
                ItemKind::ComponentOutline(o) => o.translation = o.translation.add(vector),
                ItemKind::BoardOutline(o) => {
                    // Java assigns the translated shapes to the loop variable only (no effect).
                    if let Some(a) = o.keepout_area.get().cloned() {
                        o.keepout_area = OnceLock::from(Arc::new(a.translate_by(vector)));
                    }
                }
            }
            item.clear_derived_data();
        }
        self.clear_derived_data(key);
        self.tree_insert(key);
        if self.item(key).is_pin() {
            self.invalidate_edge_pin_net_cache();
        }
    }

    // ------------------------------------------------------------------------------------------
    // board outline helpers

    /// Java `invalidateEdgePinNetCache()`.
    pub fn invalidate_edge_pin_net_cache(&mut self) {
        self.edge_pin_nets = OnceLock::new();
    }

    /// Java `BoardOutline.getEdgePinNets()` of the board outline.
    pub fn edge_pin_nets(&self) -> Arc<HashSet<NetNo>> {
        self.edge_pin_nets
            .get_or_init(|| {
                let mut set = HashSet::new();
                let Some(outline_key) = self.get_outline() else {
                    return Arc::new(set);
                };
                let outline = self.item(outline_key).as_board_outline().unwrap();
                for pin_key in self.get_pins() {
                    let pin = self.item(pin_key);
                    let center = pin.center(self);
                    let mut is_edge_or_outside = false;
                    if !outline.contains(&center) {
                        is_edge_or_outside = true;
                    } else {
                        let first = pin.first_layer(self);
                        for layer in first..=pin.last_layer(self) {
                            if let Some(Shape::Tile(tile)) = pin.drill_shape(self, layer - first) {
                                for c in 0..tile.border_line_count() {
                                    if !outline.contains(&tile.corner(c)) {
                                        is_edge_or_outside = true;
                                        break;
                                    }
                                }
                            }
                            if is_edge_or_outside {
                                break;
                            }
                        }
                    }
                    if is_edge_or_outside {
                        for &n in &pin.net_numbers {
                            set.insert(n);
                        }
                    }
                }
                Arc::new(set)
            })
            .clone()
    }

    /// Java `BoardOutline.blocksNets(netNumbers)` (always true for [`JavaVariant::Jar241`],
    /// which has no edge pin exemption).
    pub fn outline_blocks_nets(&self, net_numbers: &[NetNo]) -> bool {
        if net_numbers.is_empty() || self.java_variant == JavaVariant::Jar241 {
            return true;
        }
        let edge = self.edge_pin_nets();
        net_numbers.iter().any(|&n| !(n > 0 && edge.contains(&n)))
    }

    /// Java `Pin.isSameLogicalPad(pin1, pin2)` for two pins of the same component.
    pub(crate) fn is_same_logical_pad(&self, component_no: ComponentNo, pin_index_1: i32, pin_index_2: i32) -> bool {
        if component_no <= 0 || component_no > self.components.count() {
            return false;
        }
        let component = self.components.get(component_no);
        let package = self.library.packages.get(component.get_package());
        if pin_index_1 >= package.pin_count() || pin_index_2 >= package.pin_count() {
            return false;
        }
        let name1 = &package.get_pin(pin_index_1).unwrap().name;
        let name2 = &package.get_pin(pin_index_2).unwrap().name;
        let base1 = base_pin_name(name1);
        let base2 = base_pin_name(name2);
        !base1.is_empty() && base1 == base2
    }

    /// Java `expandBoundingBoxToIncludeAllItems()`.
    pub fn expand_bounding_box_to_include_all_items(&mut self) {
        let mut bounds = self.bounding_box;
        let mut changed = false;
        for key in self.get_items() {
            let item_box = self.item(key).bounding_box(self);
            if !item_box.is_empty() && !bounds.contains_regular(&fr_geom::RegularTileShape::IntBox(item_box)) {
                bounds = bounds.union_int_box(&item_box);
                changed = true;
            }
        }
        if changed {
            self.bounding_box = bounds.offset(1000.0);
            if self.get_outline().is_some() {
                self.invalidate_edge_pin_net_cache();
            }
        }
    }

    // ------------------------------------------------------------------------------------------
    // RoutingBoard hook

    /// Java `additionalUpdateAfterChange(item)` (see [`AutorouteMaintenance`]).
    pub fn additional_update_after_change(&mut self, key: ItemKey) {
        let Some(m) = &self.autoroute_maintenance else {
            return;
        };
        if !m.maintain_database {
            return;
        }
        let Some(t) = self.search_trees.tree_index(m.tree_clearance_class) else {
            return;
        };
        let shape_count = self.tree_shape_count(t, key);
        let mut removed = Vec::new();
        let mut drill_shapes = Vec::new();
        for i in 0..shape_count {
            let Some(shape) = self.tree_shape(t, key, i) else { continue };
            drill_shapes.push(shape.clone());
            let layer = self.shape_layer(key, i);
            let overlaps = self.overlapping_objects_in(t, &ConvexShape::Tile(shape), layer, &[]);
            for object in overlaps {
                if let TreeObject::Room { key: room, .. } = object {
                    // Java removes the room at once (AutorouteEngine.removeCompleteExpansionRoom)
                    self.search_trees.trees[t].remove_room(room);
                    removed.push(room);
                }
            }
        }
        let m = self.autoroute_maintenance.as_mut().unwrap();
        // interleave like Java: per shape the drill pages are invalidated before the rooms are
        // removed; both lists keep their own call order.
        m.invalidated_drill_shapes.extend(drill_shapes);
        m.removed_rooms.extend(removed);
        m.cleared_autoroute_info.push(key);
    }

    /// Records that Java would null the autoroute info of the item here (`clearDerivedData`,
    /// `clearAutorouteInfo`); the autoroute engine drops it (see [`AutorouteMaintenance`]).
    pub(crate) fn note_autoroute_info_cleared(&mut self, key: ItemKey) {
        if let Some(m) = &mut self.autoroute_maintenance {
            m.cleared_autoroute_info.push(key);
        }
    }

    // ------------------------------------------------------------------------------------------
    // geometric queries

    /// Java `overlappingObjects(shape, layer)` on the default tree.
    pub fn overlapping_objects(&self, shape: &ConvexShape, layer: LayerNo) -> Vec<TreeObject> {
        self.overlapping_objects_in(DEFAULT_TREE, shape, layer, &[])
    }

    /// Java `overlappingObjects(shape, layer)` restricted to items (the rooms are never in the
    /// default tree).
    pub fn overlapping_items_of(&self, shape: &ConvexShape, layer: LayerNo) -> ItemSet {
        let mut result = ItemSet::new();
        for o in self.overlapping_objects(shape, layer) {
            if let TreeObject::Item { key, id } = o {
                result.insert(ItemId(id), key);
            }
        }
        result
    }

    /// Java `overlappingItemsWithClearance(shape, layer, ignoreNetNos, clearanceClass)`.
    pub fn overlapping_items_with_clearance(&self, shape: &ConvexShape, layer: LayerNo, ignore_net_nos: &[NetNo], clearance_class: ClearanceClassNo) -> ItemSet {
        self.overlapping_items_with_clearance_in(DEFAULT_TREE, shape, layer, ignore_net_nos, clearance_class)
    }

    /// Java `overlappingItems(area, layer)`.
    pub fn overlapping_items(&self, area: &Area, layer: LayerNo) -> ItemSet {
        let mut result = ItemSet::new();
        for tile in area.split_to_convex().unwrap_or_default() {
            for o in self.overlapping_objects(&ConvexShape::Tile(tile), layer) {
                if let TreeObject::Item { key, id } = o {
                    result.insert(ItemId(id), key);
                }
            }
        }
        result
    }

    /// Java `pickItems(location, layer, filter)`.
    pub fn pick_items(&self, location: &Point, layer: LayerNo, filter: Option<&ItemSelectionFilter>) -> ItemSet {
        let point_shape = ConvexShape::Tile(TileShape::IntBox(TileShape::get_instance_point(location)));
        let mut result = ItemSet::new();
        for o in self.overlapping_objects(&point_shape, layer) {
            if let TreeObject::Item { key, id } = o {
                result.insert(ItemId(id), key);
            }
        }
        if let Some(filter) = filter {
            result = filter.filter(self, &result);
        }
        result
    }

    /// Java `checkShape(area, layer, netNumbers, clearanceClassIndex)`.
    pub fn check_shape(&self, shape: &Area, layer: LayerNo, net_numbers: &[NetNo], clearance_class: ClearanceClassNo) -> bool {
        let tiles = shape.split_to_convex().unwrap_or_default();
        for tile in tiles {
            if !tile.is_contained_in(&self.bounding_box) {
                return false;
            }
            let obstacles = self.overlapping_objects_with_clearance_in(DEFAULT_TREE, &ConvexShape::Tile(tile), layer, net_numbers, clearance_class);
            for object in obstacles {
                let mut is_obstacle = true;
                for &n in net_numbers {
                    if !self.object_is_obstacle(object, n) {
                        is_obstacle = false;
                    }
                }
                if is_obstacle {
                    return false;
                }
            }
        }
        true
    }

    /// Java `checkTraceShape(shape, layer, netNumbers, clearanceClassIndex, contactPins)`.
    pub fn check_trace_shape(
        &self,
        shape: &TileShape,
        layer: LayerNo,
        net_numbers: &[NetNo],
        clearance_class: ClearanceClassNo,
        contact_pins: Option<&ItemSet>,
    ) -> bool {
        if !shape.is_contained_in(&self.bounding_box) {
            return false;
        }
        let query = ConvexShape::Tile(shape.clone());
        let mut entries: Vec<TreeEntry> = Vec::new();
        if self.default_tree().is_clearance_compensation_used() {
            self.overlapping_tree_entries(DEFAULT_TREE, &query, layer, &[], &mut entries);
        } else {
            self.overlapping_tree_entries_with_clearance_raw(DEFAULT_TREE, &query, layer, &[], clearance_class, &mut entries);
        }
        for entry in entries {
            let TreeObject::Item { key, .. } = entry.object else { continue };
            let current_item = self.item(key);
            if let Some(pins) = contact_pins {
                if pins.contains(current_item.id()) {
                    continue;
                }
                if current_item.is_pin() {
                    // Other pins are handled as obstacles to avoid acid traps.
                    return false;
                }
            }
            let mut is_obstacle = true;
            for &n in net_numbers {
                if !self.is_trace_obstacle(current_item, n) {
                    is_obstacle = false;
                }
            }
            if is_obstacle && current_item.is_trace() {
                if let Some(pins) = contact_pins {
                    // check for traces of foreign nets at tie pins, which will be ignored inside
                    // the pin shape
                    let mut intersection: Option<TileShape> = None;
                    for pin_key in pins.iter() {
                        let pin = self.item(pin_key);
                        if pin.net_count() <= 1 || !pin.shares_net(current_item) {
                            continue;
                        }
                        if intersection.is_none() {
                            let obstacle_trace_shape = self.tile_shape(key, entry.shape_index).expect("tile shape");
                            intersection = Some(shape.intersection(&obstacle_trace_shape));
                        }
                        let pin_shape = self.tile_shape_on_layer(pin_key, layer).expect("pin shape on layer");
                        if pin_shape.contains_approx(intersection.as_ref().unwrap()) {
                            is_obstacle = false;
                            break;
                        }
                    }
                }
            }
            if is_obstacle {
                return false;
            }
        }
        true
    }

    /// Java `checkPolylineTrace(polyline, layer, penHalfWidth, netNumbers,
    /// clearanceClassIndex)`. Like Java, the temporary trace consumes an item id (hence `&mut`).
    pub fn check_polyline_trace(&mut self, polyline: &Polyline, layer: LayerNo, pen_half_width: i32, net_numbers: &[NetNo], clearance_class: ClearanceClassNo) -> bool {
        let tmp_id = self.new_item_id();
        let tmp = Item::new_trace(
            tmp_id,
            polyline.clone(),
            layer,
            pen_half_width,
            net_numbers,
            clearance_class,
            0,
            FixedState::Unfixed,
            self.layer_count(),
        );
        let contact_pins = self.touching_pins_at_end_corners_of(&tmp);
        let shapes = self.item_tile_shapes(&tmp);
        for s in shapes.iter() {
            let s = s.clone().expect("checkPolylineTrace: tile shape is null");
            if !self.check_trace_shape(&s, layer, net_numbers, clearance_class, Some(&contact_pins)) {
                return false;
            }
        }
        true
    }

    /// Java `DrillItem.getTileShapeOnLayer(layer)`.
    pub fn tile_shape_on_layer(&self, key: ItemKey, layer: LayerNo) -> Option<TileShape> {
        let item = self.item(key);
        let from = item.first_layer(self);
        let to = item.last_layer(self);
        if layer < from || layer > to {
            log::warn!("DrillItem.get_tile_shape_on_layer: layer out of range");
            return None;
        }
        self.tile_shape(key, layer - from)
    }

    /// Java `DrillItem.getTreeShapeOnLayer(tree, layer)`.
    pub fn tree_shape_on_layer(&self, t: usize, key: ItemKey, layer: LayerNo) -> Option<TileShape> {
        let item = self.item(key);
        let from = item.first_layer(self);
        let to = item.last_layer(self);
        if layer < from || layer > to {
            log::warn!("DrillItem.get_tree_shape_on_layer: layer out of range");
            return None;
        }
        self.tree_shape(t, key, layer - from)
    }

    /// Java `Trace.touchingPinsAtEndCorners()` for a trace that is not necessarily on the board.
    pub fn touching_pins_at_end_corners_of(&self, trace: &Item) -> ItemSet {
        let mut result = ItemSet::new();
        let t = trace.trace();
        let mut end_point = trace.first_corner();
        for i in 0..2 {
            let oct: IntOctagon = end_point.surrounding_octagon().enlarge(t.half_width as f64);
            let overlaps = self.overlapping_items_with_clearance(&ConvexShape::Tile(TileShape::IntOctagon(oct)), t.layer, &[], trace.clearance_class);
            for (id, key) in overlaps.entries() {
                let item = self.item(key);
                if item.is_pin() && item.shares_net(trace) {
                    result.insert(id, key);
                }
            }
            if i == 0 {
                end_point = trace.last_corner();
            }
        }
        result
    }

    /// Java `Trace.touchingPinsAtEndCorners()`.
    pub fn touching_pins_at_end_corners(&self, key: ItemKey) -> ItemSet {
        self.touching_pins_at_end_corners_of(self.item(key))
    }

    // ------------------------------------------------------------------------------------------
    // traces

    /// Java `combineTraces(netNumber)`.
    pub fn combine_traces(&mut self, net_number: NetNo) -> bool {
        let mut result = false;
        let mut something_changed = true;
        while something_changed {
            something_changed = false;
            let mut cursor = self.items.cursor();
            while let Some(key) = self.items.cursor_next(&mut cursor) {
                let item = self.item(key);
                if (net_number < 0 || item.contains_net(net_number)) && item.is_trace() && item.is_on_board() && self.combine_trace(key) {
                    something_changed = true;
                    result = true;
                    break;
                }
            }
        }
        result
    }

    /// Java `normalizeTraces(netNumber)`.
    pub fn normalize_traces(&mut self, net_number: NetNo) -> bool {
        if self.normalize_suppressed_net_nos.contains(&net_number) {
            log::debug!("BasicBoard.normalizeTraces: skipping net {net_number} because normalization already hit the oscillation cap");
            return false;
        }
        let mut result = false;
        let mut something_changed = true;
        let mut iteration_count = 0;
        while something_changed {
            iteration_count += 1;
            if iteration_count > MAX_NORMALIZE_ITERATIONS {
                log::warn!("BasicBoard.normalizeTraces: reached {MAX_NORMALIZE_ITERATIONS} iterations for net {net_number}, stopping");
                self.normalize_suppressed_net_nos.insert(net_number);
                break;
            }
            something_changed = false;
            let net_traces = self.net_polyline_traces(net_number);
            for trace in net_traces {
                if self.item(trace).is_on_board() {
                    // Java: `if (normalize(null)) {..} else if (!isUserFixed() && removeIfCycle(..)) {..}`
                    if self.normalize_trace(trace, None) || (!self.item(trace).is_user_fixed() && self.remove_if_cycle(trace)) {
                        something_changed = true;
                        result = true;
                    }
                }
            }
        }
        result
    }

    /// The on-board polyline traces of a net in item list order.
    fn net_polyline_traces(&self, net_number: NetNo) -> Vec<ItemKey> {
        self.items
            .net_items(net_number)
            .filter(|k| {
                let item = self.item(*k);
                item.is_trace() && item.is_on_board()
            })
            .collect()
    }

    /// Java `normalizeAllTraces()`: groups the traces by net in a Java `HashMap<Integer,..>`
    /// (iteration order reproduced) and normalizes each net.
    pub fn normalize_all_traces(&mut self) -> bool {
        let mut result = false;
        let mut traces_by_net: fr_jcompat::hashmap::JavaIntHashMap<Vec<ItemKey>> = fr_jcompat::hashmap::JavaIntHashMap::new();
        let mut cursor = self.items.cursor();
        while let Some(key) = self.items.cursor_next(&mut cursor) {
            let item = self.item(key);
            if item.is_trace() && item.is_on_board() {
                for &n in &item.net_numbers {
                    traces_by_net.compute_if_absent(n, Vec::new).push(key);
                }
            }
        }
        let entries: Vec<(i32, Vec<ItemKey>)> = traces_by_net.iter().map(|(k, v)| (k, v.clone())).collect();
        for (net_number, mut net_traces) in entries {
            let mut something_changed = true;
            let mut iteration_count = 0;
            while something_changed {
                iteration_count += 1;
                if iteration_count > MAX_NORMALIZE_ITERATIONS {
                    log::warn!("BasicBoard.normalize_all_traces: reached {MAX_NORMALIZE_ITERATIONS} iterations for net {net_number}, stopping");
                    break;
                }
                something_changed = false;
                for &trace in &net_traces {
                    if self.item(trace).is_on_board() {
                        // Java: `if (normalize(null)) {..} else if (!isUserFixed() && removeIfCycle(..)) {..}`
                        if self.normalize_trace(trace, None) || (!self.item(trace).is_user_fixed() && self.remove_if_cycle(trace)) {
                            something_changed = true;
                            result = true;
                        }
                    }
                }
                if something_changed {
                    net_traces = self.net_polyline_traces(net_number);
                }
            }
        }
        result
    }

    /// Java `splitTraces(location, layer, netNumber)`.
    pub fn split_traces(&mut self, location: &Point, layer: LayerNo, net_number: NetNo) -> bool {
        let filter = ItemSelectionFilter::single(SelectableChoices::Traces);
        let picked = self.pick_items(location, layer, Some(&filter));
        let location_shape = TileShape::get_instance_point(location).bounding_octagon();
        let mut trace_split = false;
        for key in picked.iter() {
            if self.item(key).contains_net(net_number) {
                let pieces = self.split_trace(key, Some(&location_shape));
                if pieces.len() != 1 {
                    trace_split = true;
                }
            }
        }
        trace_split
    }

    /// Java `getTraceTail(location, layer, netNumbers)`.
    pub fn get_trace_tail(&self, location: &Point, layer: LayerNo, net_numbers: &[NetNo]) -> Option<ItemKey> {
        let point_shape = ConvexShape::Tile(TileShape::IntBox(TileShape::get_instance_point(location)));
        for o in self.overlapping_objects(&point_shape, layer) {
            let TreeObject::Item { key, .. } = o else { continue };
            let item = self.item(key);
            if !item.is_trace() || !item.nets_equal_nos(net_numbers) {
                continue;
            }
            if item.first_corner() == *location && self.trace_start_contacts(key).is_empty() {
                return Some(key);
            }
            if item.last_corner() == *location && self.trace_end_contacts(key).is_empty() {
                return Some(key);
            }
        }
        None
    }

    /// Java `removeIfCycle(trace)`.
    pub fn remove_if_cycle(&mut self, trace: ItemKey) -> bool {
        if !self.item(trace).is_on_board() {
            return false;
        }
        if !self.is_cycle(trace) {
            return false;
        }
        // Remove tails at the endpoints after removing the cycle, if there was no tail before.
        let item = self.item(trace);
        let current_layer = item.trace().layer;
        let current_nets = item.net_numbers.clone();
        let end_corners = [item.first_corner(), item.last_corner()];
        let mut tail_before = [false; 2];
        for i in 0..2 {
            tail_before[i] = self.get_trace_tail(&end_corners[i], current_layer, &current_nets).is_some();
        }
        let connection_items = self.get_connection_items(trace, super::item::StopConnectionOption::None);
        self.remove_items(connection_items.iter().collect::<Vec<_>>());
        for i in 0..2 {
            if !tail_before[i] {
                if let Some(tail) = self.get_trace_tail(&end_corners[i], current_layer, &current_nets) {
                    let items = self.get_connection_items(tail, super::item::StopConnectionOption::None);
                    self.remove_items(items.iter().collect::<Vec<_>>());
                }
            }
        }
        true
    }

}
