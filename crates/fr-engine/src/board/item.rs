//! Port of `board/model/items/*` (`Item`, `DrillItem`, `Pin`, `Via`, `Trace`, `ObstacleArea`,
//! `ViaObstacleArea`, `ComponentObstacleArea`, `ConductionArea`, `ComponentOutline`,
//! `BoardItemType`, `Connectable`), `board/trace/PolylineTrace` (data part) and
//! `board/model/structure/BoardOutline`.
//!
//! # Representation
//!
//! * [`Item`] is the common header (Java `Item` fields) plus an [`ItemKind`] enum of the concrete
//!   classes. The board owns all items in an arena addressed by [`ItemKey`]; nothing points back
//!   to the board, methods needing board data take `&BasicBoard`.
//! * Java object identity (`==` on items) is [`ItemKey`] equality.
//! * Transient Java caches (`Pin.precalculatedShapes`, `Pin` center, `Via.precalculatedShapes`,
//!   `ObstacleArea.precalculatedAbsoluteArea`, `BoardOutline.keepoutArea`) are `OnceLock`s,
//!   reset by [`Item::clear_derived_data`]. The per-tree shapes and leaves live in the search
//!   trees' side tables (Java `ItemSearchTreesInfo`).
//! * `DrillItem.precalculatedFirstLayer/LastLayer` are recomputed on demand (they only change
//!   together with `clearDerivedData`).
//! * GUI (`printInfo`, hover info, rendering caches of `ConductionArea`), serialization and
//!   the autoroute info (`getAutorouteInfo`, owned by the autorouter in the port) are dropped.

use std::sync::{Arc, OnceLock};

use fr_geom::{
    Area, FloatPoint, IntBox, IntPoint, Point, Polyline, PolylineArea, PolylineShape, Shape,
    TileShape, Vector,
};

use crate::ids::{ClearanceClassNo, ComponentNo, FixedState, ItemId, LayerNo, NetNo, PadstackNo};
use crate::library::Padstack;
use crate::rules::Nets;

use super::basic_board::BasicBoard;

/// Handle of an item in the board's item arena. Stays valid (and the item readable, as a
/// tombstone with `on_board == false`) after the item was removed, until
/// [`BasicBoard::compact`] frees removed items.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ItemKey {
    pub(crate) index: u32,
    pub(crate) generation: u32,
}

impl std::fmt::Debug for ItemKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "key{}v{}", self.index, self.generation)
    }
}

impl ItemKey {
    /// Arena slot of the item.
    pub fn index(&self) -> u32 {
        self.index
    }
}

/// Java `BoardItemType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BoardItemType {
    Trace,
    Pin,
    Via,
    ObstacleArea,
    ViaObstacleArea,
    ConductionArea,
    ComponentObstacleArea,
    BoardOutline,
    ComponentOutline,
    Other,
}

/// Java `Item.StopConnectionOption`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopConnectionOption {
    None,
    FanoutVia,
    Via,
}

/// The three classes `ObstacleArea`, `ViaObstacleArea` and `ComponentObstacleArea`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ObstacleKind {
    /// Java `ObstacleArea` (keepout).
    Keepout,
    /// Java `ViaObstacleArea`.
    ViaKeepout,
    /// Java `ComponentObstacleArea`.
    ComponentKeepout,
}

/// Java `Pin` specific data.
#[derive(Clone, Debug)]
pub struct Pin {
    /// The index of this pin in its component (starting with 0).
    pub pin_index: i32,
    /// Java `changedTo` (pin swap): the id of the pin this pin was changed to; `None` = itself.
    pub changed_to: Option<ItemId>,
    pub(crate) center: OnceLock<Point>,
    pub(crate) shapes: OnceLock<Arc<[Option<Shape>]>>,
}

/// Java `Via` specific data.
#[derive(Clone, Debug)]
pub struct Via {
    pub(crate) padstack: PadstackNo,
    pub(crate) center: Point,
    /// True, if coppersharing of this via with smd pins of the same net is allowed.
    pub attach_allowed: bool,
    /// Escape via inserted by the escalation fanout phase.
    pub is_escape_via: bool,
    /// The SMD layer of an escape via, -1 otherwise.
    pub escape_via_smd_layer: LayerNo,
    pub(crate) shapes: OnceLock<Arc<[Option<Shape>]>>,
}

/// Java `PolylineTrace` data (with the `Trace` fields).
#[derive(Clone, Debug)]
pub struct PolylineTrace {
    pub(crate) polyline: Polyline,
    /// The Java object identities of the lines of the polyline (see
    /// [`super::optimize::tracked`]).
    pub(crate) line_ids: Arc<[u64]>,
    pub(crate) layer: LayerNo,
    pub(crate) half_width: i32,
}

/// Java `ObstacleArea` data (also the base of `ConductionArea`).
#[derive(Clone, Debug)]
pub struct ObstacleArea {
    pub kind: ObstacleKind,
    /// Null (`None`) if the area does not belong to a component.
    pub name: Option<String>,
    pub(crate) relative_area: Arc<Area>,
    pub(crate) layer: LayerNo,
    pub(crate) translation: Vector,
    pub(crate) rotation_in_degree: f64,
    pub(crate) side_changed: bool,
    pub(crate) absolute_area: OnceLock<Arc<Area>>,
}

/// Java `ConductionArea`.
#[derive(Clone, Debug)]
pub struct ConductionArea {
    pub(crate) area: ObstacleArea,
    pub(crate) is_obstacle: bool,
    pub(crate) is_filled: bool,
}

/// Java `ComponentOutline`.
#[derive(Clone, Debug)]
pub struct ComponentOutline {
    pub(crate) relative_area: Arc<Area>,
    pub(crate) translation: Vector,
    pub(crate) rotation_in_degree: f64,
    pub(crate) is_front: bool,
    pub is_courtyard: bool,
    pub is_fabrication: bool,
    pub is_closed: bool,
    pub(crate) absolute_area: OnceLock<Arc<Area>>,
}

/// Java `BoardOutline` (an item in `board/model/structure`).
#[derive(Clone, Debug)]
pub struct BoardOutline {
    pub(crate) shapes: Arc<[PolylineShape]>,
    pub(crate) keepout_area: OnceLock<Arc<Area>>,
    pub(crate) keepout_outside_outline: bool,
}

impl BoardOutline {
    /// Java `HALF_WIDTH`.
    pub const HALF_WIDTH: i32 = 100;
}

/// The concrete item classes.
#[derive(Clone, Debug)]
pub enum ItemKind {
    Pin(Pin),
    Via(Via),
    Trace(PolylineTrace),
    ObstacleArea(ObstacleArea),
    ConductionArea(ConductionArea),
    ComponentOutline(ComponentOutline),
    BoardOutline(BoardOutline),
}

/// Java `Item`: the common header of all board items.
#[derive(Clone, Debug)]
pub struct Item {
    pub(crate) id: ItemId,
    pub(crate) net_numbers: Vec<NetNo>,
    pub(crate) clearance_class: ClearanceClassNo,
    pub(crate) component_no: ComponentNo,
    pub(crate) fixed_state: FixedState,
    pub(crate) on_board: bool,
    /// Java `smallestClearance` (written by the clearance violation calculation).
    pub smallest_clearance: f64,
    pub kind: ItemKind,
}

impl Item {
    pub(crate) fn new_header(
        id: ItemId,
        net_numbers: &[NetNo],
        clearance_class: ClearanceClassNo,
        component_no: ComponentNo,
        fixed_state: FixedState,
        kind: ItemKind,
    ) -> Item {
        Item {
            id,
            net_numbers: net_numbers.to_vec(),
            clearance_class,
            component_no,
            fixed_state,
            on_board: false,
            smallest_clearance: -1.0,
            kind,
        }
    }

    /// Creates a (not inserted) polyline trace. Java `new PolylineTrace(...)`; the layer is
    /// clamped to `0 ..= layer_count - 1` like the `Trace` constructor does.
    #[allow(clippy::too_many_arguments)]
    pub fn new_trace(
        id: ItemId,
        polyline: Polyline,
        layer: LayerNo,
        half_width: i32,
        net_numbers: &[NetNo],
        clearance_class: ClearanceClassNo,
        component_no: ComponentNo,
        fixed_state: FixedState,
        layer_count: i32,
    ) -> Item {
        if polyline.lines.len() < 3 {
            log::warn!("PolylineTrace: polyline.lines.length >= 3 expected");
        }
        let layer = layer.max(0).min(layer_count - 1);
        let line_ids = super::optimize::tracked::fresh_line_ids(polyline.lines.len());
        Item::new_header(
            id,
            net_numbers,
            clearance_class,
            component_no,
            fixed_state,
            ItemKind::Trace(PolylineTrace { polyline, line_ids, layer, half_width }),
        )
    }

    /// [`Item::new_trace`] with the identities of the lines of the polyline (lines shared with
    /// other traces).
    #[allow(clippy::too_many_arguments)]
    pub fn new_trace_tracked(
        id: ItemId,
        polyline: super::optimize::tracked::TPolyline,
        layer: LayerNo,
        half_width: i32,
        net_numbers: &[NetNo],
        clearance_class: ClearanceClassNo,
        component_no: ComponentNo,
        fixed_state: FixedState,
        layer_count: i32,
    ) -> Item {
        let mut item = Item::new_trace(id, polyline.polyline, layer, half_width, net_numbers, clearance_class, component_no, fixed_state, layer_count);
        item.trace_mut().line_ids = polyline.ids;
        item
    }

    // ------------------------------------------------------------------------------------------
    // header accessors

    /// Java `getId()`.
    #[inline]
    pub fn id(&self) -> ItemId {
        self.id
    }

    /// Java `netNumbers`.
    #[inline]
    pub fn net_numbers(&self) -> &[NetNo] {
        &self.net_numbers
    }

    /// Java `clearanceClassIndex()`.
    #[inline]
    pub fn clearance_class(&self) -> ClearanceClassNo {
        self.clearance_class
    }

    /// Java `getComponentId()`.
    #[inline]
    pub fn component_no(&self) -> ComponentNo {
        self.component_no
    }

    /// Java `getFixedState()`.
    #[inline]
    pub fn fixed_state(&self) -> FixedState {
        self.fixed_state
    }

    /// Java `setFixedState` (no board index depends on it).
    pub fn set_fixed_state(&mut self, fixed_state: FixedState) {
        self.fixed_state = fixed_state;
    }

    /// Java `unfix()`.
    pub fn unfix(&mut self) {
        if self.fixed_state != FixedState::SystemFixed {
            self.fixed_state = FixedState::Unfixed;
        }
    }

    /// Java `isOnTheBoard()`.
    #[inline]
    pub fn is_on_board(&self) -> bool {
        self.on_board
    }

    /// Java `getBoardItemType()`.
    pub fn board_item_type(&self) -> BoardItemType {
        match &self.kind {
            ItemKind::Pin(_) => BoardItemType::Pin,
            ItemKind::Via(_) => BoardItemType::Via,
            ItemKind::Trace(_) => BoardItemType::Trace,
            ItemKind::ConductionArea(_) => BoardItemType::ConductionArea,
            ItemKind::ObstacleArea(a) => match a.kind {
                ObstacleKind::Keepout => BoardItemType::ObstacleArea,
                ObstacleKind::ViaKeepout => BoardItemType::ViaObstacleArea,
                ObstacleKind::ComponentKeepout => BoardItemType::ComponentObstacleArea,
            },
            ItemKind::ComponentOutline(_) => BoardItemType::ComponentOutline,
            ItemKind::BoardOutline(_) => BoardItemType::BoardOutline,
        }
    }

    /// Java simple class name (for dumps and logging).
    pub fn class_name(&self) -> &'static str {
        match &self.kind {
            ItemKind::Pin(_) => "Pin",
            ItemKind::Via(_) => "Via",
            ItemKind::Trace(_) => "PolylineTrace",
            ItemKind::ConductionArea(_) => "ConductionArea",
            ItemKind::ObstacleArea(a) => match a.kind {
                ObstacleKind::Keepout => "ObstacleArea",
                ObstacleKind::ViaKeepout => "ViaObstacleArea",
                ObstacleKind::ComponentKeepout => "ComponentObstacleArea",
            },
            ItemKind::ComponentOutline(_) => "ComponentOutline",
            ItemKind::BoardOutline(_) => "BoardOutline",
        }
    }

    // ------------------------------------------------------------------------------------------
    // class tests (Java instanceof)

    #[inline]
    pub fn is_pin(&self) -> bool {
        matches!(self.kind, ItemKind::Pin(_))
    }
    #[inline]
    pub fn is_via(&self) -> bool {
        matches!(self.kind, ItemKind::Via(_))
    }
    /// `instanceof DrillItem`.
    #[inline]
    pub fn is_drill_item(&self) -> bool {
        matches!(self.kind, ItemKind::Pin(_) | ItemKind::Via(_))
    }
    /// `instanceof Trace` (all traces are polyline traces).
    #[inline]
    pub fn is_trace(&self) -> bool {
        matches!(self.kind, ItemKind::Trace(_))
    }
    #[inline]
    pub fn is_conduction_area(&self) -> bool {
        matches!(self.kind, ItemKind::ConductionArea(_))
    }
    /// `instanceof ObstacleArea` (includes the subclasses and `ConductionArea`).
    #[inline]
    pub fn is_obstacle_area(&self) -> bool {
        matches!(self.kind, ItemKind::ObstacleArea(_) | ItemKind::ConductionArea(_))
    }
    #[inline]
    pub fn is_via_obstacle_area(&self) -> bool {
        matches!(&self.kind, ItemKind::ObstacleArea(a) if a.kind == ObstacleKind::ViaKeepout)
    }
    #[inline]
    pub fn is_component_obstacle_area(&self) -> bool {
        matches!(&self.kind, ItemKind::ObstacleArea(a) if a.kind == ObstacleKind::ComponentKeepout)
    }
    #[inline]
    pub fn is_board_outline(&self) -> bool {
        matches!(self.kind, ItemKind::BoardOutline(_))
    }
    #[inline]
    pub fn is_component_outline(&self) -> bool {
        matches!(self.kind, ItemKind::ComponentOutline(_))
    }
    /// `instanceof Connectable` (DrillItem, Trace, ConductionArea).
    #[inline]
    pub fn is_connectable_class(&self) -> bool {
        matches!(
            self.kind,
            ItemKind::Pin(_) | ItemKind::Via(_) | ItemKind::Trace(_) | ItemKind::ConductionArea(_)
        )
    }

    pub fn as_trace(&self) -> Option<&PolylineTrace> {
        match &self.kind {
            ItemKind::Trace(t) => Some(t),
            _ => None,
        }
    }
    pub fn as_pin(&self) -> Option<&Pin> {
        match &self.kind {
            ItemKind::Pin(p) => Some(p),
            _ => None,
        }
    }
    pub fn as_via(&self) -> Option<&Via> {
        match &self.kind {
            ItemKind::Via(v) => Some(v),
            _ => None,
        }
    }
    /// The `ObstacleArea` part of obstacle and conduction areas.
    pub fn as_obstacle_area(&self) -> Option<&ObstacleArea> {
        match &self.kind {
            ItemKind::ObstacleArea(a) => Some(a),
            ItemKind::ConductionArea(c) => Some(&c.area),
            _ => None,
        }
    }
    pub fn as_conduction_area(&self) -> Option<&ConductionArea> {
        match &self.kind {
            ItemKind::ConductionArea(c) => Some(c),
            _ => None,
        }
    }
    pub fn as_board_outline(&self) -> Option<&BoardOutline> {
        match &self.kind {
            ItemKind::BoardOutline(o) => Some(o),
            _ => None,
        }
    }
    pub fn as_component_outline(&self) -> Option<&ComponentOutline> {
        match &self.kind {
            ItemKind::ComponentOutline(o) => Some(o),
            _ => None,
        }
    }

    /// The trace data; panics if this is not a trace (Java cast).
    pub fn trace(&self) -> &PolylineTrace {
        self.as_trace().expect("item is not a trace")
    }

    pub(crate) fn trace_mut(&mut self) -> &mut PolylineTrace {
        match &mut self.kind {
            ItemKind::Trace(t) => t,
            _ => panic!("item is not a trace"),
        }
    }

    // ------------------------------------------------------------------------------------------
    // nets

    /// Java `containsNet`.
    pub fn contains_net(&self, net_number: NetNo) -> bool {
        if net_number <= 0 {
            return false;
        }
        self.net_numbers.contains(&net_number)
    }

    /// Java `sharesNet`.
    pub fn shares_net(&self, other: &Item) -> bool {
        self.shares_net_no(&other.net_numbers)
    }

    /// Java `sharesNetNo`.
    pub fn shares_net_no(&self, net_numbers: &[NetNo]) -> bool {
        self.net_numbers.iter().any(|n| net_numbers.contains(n))
    }

    /// Java `netCount()`.
    pub fn net_count(&self) -> i32 {
        self.net_numbers.len() as i32
    }

    /// Java `getNetNumber(no)`.
    pub fn net_number(&self, no: i32) -> NetNo {
        self.net_numbers[no as usize]
    }

    /// Java `netsNormal()`.
    pub fn nets_normal(&self) -> bool {
        self.net_numbers.iter().all(|&n| Nets::is_normal_net_number(n))
    }

    /// Java `netsEqual(Item)`.
    pub fn nets_equal(&self, other: &Item) -> bool {
        self.nets_equal_nos(&other.net_numbers)
    }

    /// Java `netsEqual(int[])`.
    pub fn nets_equal_nos(&self, net_numbers: &[NetNo]) -> bool {
        if self.net_numbers.len() != net_numbers.len() {
            return false;
        }
        net_numbers.iter().all(|&n| self.contains_net(n))
    }

    /// Java `SearchTreeObject.isObstacle(int netNumber)` for items without override
    /// (all item classes use the default `!containsNet`).
    pub fn is_obstacle_for_net(&self, net_number: NetNo) -> bool {
        !self.contains_net(net_number)
    }

    /// Java `isDrillable(netNumber)`.
    pub fn is_drillable(&self, net_number: NetNo) -> bool {
        match &self.kind {
            ItemKind::Trace(_) => self.contains_net(net_number),
            ItemKind::ConductionArea(c) => !c.is_obstacle || self.contains_net(net_number),
            _ => false,
        }
    }

    // ------------------------------------------------------------------------------------------
    // fixed state

    /// Java `isUserFixed()`.
    pub fn is_user_fixed(&self) -> bool {
        self.fixed_state >= FixedState::UserFixed
    }

    /// Java `Item.isShoveFixed()` (without the net class check of `Trace.isShoveFixed`, see
    /// [`BasicBoard::is_shove_fixed`]).
    pub fn is_shove_fixed_state(&self) -> bool {
        self.fixed_state >= FixedState::ShoveFixed
    }

    /// Java `isRoutable()`: an unfixed trace or via with a net.
    pub fn is_routable(&self) -> bool {
        match self.kind {
            ItemKind::Trace(_) | ItemKind::Via(_) => !self.is_user_fixed() && !self.net_numbers.is_empty(),
            _ => false,
        }
    }

    /// Java `isConnectable()`.
    pub fn is_connectable(&self) -> bool {
        self.is_connectable_class() && !self.net_numbers.is_empty()
    }

    // ------------------------------------------------------------------------------------------
    // obstacle relation (Java `isObstacle(Item other)` and overrides)

    /// Java `isObstacle(Item other)`. `self_key`/`other_key` implement the `other == this`
    /// identity checks; `board` is needed for the board outline's edge pin nets and pin names.
    pub fn is_obstacle(&self, self_key: ItemKey, other: &Item, other_key: ItemKey, board: &BasicBoard) -> bool {
        let same = self_key == other_key;
        match &self.kind {
            ItemKind::Pin(p) => {
                if same || other.is_obstacle_area() {
                    return false;
                }
                let source = board.java_variant == super::basic_board::JavaVariant::Source;
                if !other.shares_net(self) {
                    if let ItemKind::Pin(op) = &other.kind {
                        if source
                            && self.component_no > 0
                            && self.component_no == other.component_no
                            && self.net_numbers.is_empty()
                            && other.net_numbers.is_empty()
                            && board.is_same_logical_pad(self.component_no, p.pin_index, op.pin_index)
                        {
                            return false;
                        }
                    }
                    return true;
                }
                if other.is_trace() || (source && other.is_pin()) {
                    return false;
                }
                !self.drill_allowed(board) || !other.is_via()
            }
            ItemKind::Via(v) => {
                if same || other.is_component_obstacle_area() {
                    return false;
                }
                if let ItemKind::ConductionArea(c) = &other.kind {
                    if !c.is_obstacle {
                        return false;
                    }
                }
                if !other.shares_net(self) {
                    return true;
                }
                if other.is_trace() {
                    return false;
                }
                !v.attach_allowed || !other.is_pin() || !other.drill_allowed(board)
            }
            ItemKind::Trace(_) => {
                if same || other.is_via_obstacle_area() || other.is_component_obstacle_area() {
                    return false;
                }
                if let ItemKind::ConductionArea(c) = &other.kind {
                    if !c.is_obstacle {
                        return false;
                    }
                }
                !other.shares_net(self)
            }
            ItemKind::ObstacleArea(a) => match a.kind {
                ObstacleKind::Keepout => {
                    if other.shares_net(self) {
                        return false;
                    }
                    other.is_trace() || other.is_via()
                }
                ObstacleKind::ViaKeepout => {
                    if other.shares_net(self) {
                        return false;
                    }
                    other.is_via()
                }
                ObstacleKind::ComponentKeepout => {
                    !same && other.is_component_obstacle_area() && other.component_no != self.component_no
                }
            },
            ItemKind::ConductionArea(c) => {
                if c.is_obstacle {
                    if other.shares_net(self) {
                        return false;
                    }
                    other.is_trace() || other.is_via()
                } else {
                    false
                }
            }
            ItemKind::ComponentOutline(_) => false,
            ItemKind::BoardOutline(_) => {
                if other.is_board_outline() || other.is_obstacle_area() {
                    return false;
                }
                if other.is_trace() && !board.outline_blocks_nets(&other.net_numbers) {
                    return false;
                }
                true
            }
        }
    }

    /// Java `Pin.drillAllowed()`: vias through the pad are allowed for SMD pins.
    pub fn drill_allowed(&self, board: &BasicBoard) -> bool {
        self.first_layer(board) == self.last_layer(board)
    }

    // ------------------------------------------------------------------------------------------
    // layers

    /// Java `firstLayer()`.
    pub fn first_layer(&self, board: &BasicBoard) -> LayerNo {
        match &self.kind {
            ItemKind::Pin(_) | ItemKind::Via(_) => {
                let padstack = self.padstack(board).expect("DrillItem: padstack not found");
                if self.is_placed_on_front(board) || padstack.placed_absolute {
                    padstack.from_layer()
                } else {
                    padstack.board_layer_count() - padstack.to_layer() - 1
                }
            }
            ItemKind::Trace(t) => t.layer,
            ItemKind::ObstacleArea(a) => a.layer,
            ItemKind::ConductionArea(c) => c.area.layer,
            ItemKind::ComponentOutline(o) => o.layer(board),
            ItemKind::BoardOutline(_) => 0,
        }
    }

    /// Java `lastLayer()`.
    pub fn last_layer(&self, board: &BasicBoard) -> LayerNo {
        match &self.kind {
            ItemKind::Pin(_) | ItemKind::Via(_) => {
                let padstack = self.padstack(board).expect("DrillItem: padstack not found");
                if self.is_placed_on_front(board) || padstack.placed_absolute {
                    padstack.to_layer()
                } else {
                    padstack.board_layer_count() - padstack.from_layer() - 1
                }
            }
            ItemKind::Trace(t) => t.layer,
            ItemKind::ObstacleArea(a) => a.layer,
            ItemKind::ConductionArea(c) => c.area.layer,
            ItemKind::ComponentOutline(o) => o.layer(board),
            ItemKind::BoardOutline(_) => board.layer_count() - 1,
        }
    }

    /// Java `isOnLayer(layer)`.
    pub fn is_on_layer(&self, board: &BasicBoard, layer: LayerNo) -> bool {
        match &self.kind {
            ItemKind::BoardOutline(_) => true,
            _ => layer >= self.first_layer(board) && layer <= self.last_layer(board),
        }
    }

    /// Java `sharesLayer`.
    pub fn shares_layer(&self, other: &Item, board: &BasicBoard) -> bool {
        let max_first = self.first_layer(board).max(other.first_layer(board));
        let min_last = self.last_layer(board).min(other.last_layer(board));
        max_first <= min_last
    }

    /// Java `firstCommonLayer`.
    pub fn first_common_layer(&self, other: &Item, board: &BasicBoard) -> LayerNo {
        let max_first = self.first_layer(board).max(other.first_layer(board));
        let min_last = self.last_layer(board).min(other.last_layer(board));
        if max_first > min_last {
            -1
        } else {
            max_first
        }
    }

    /// Java `lastCommonLayer`.
    pub fn last_common_layer(&self, other: &Item, board: &BasicBoard) -> LayerNo {
        let max_first = self.first_layer(board).max(other.first_layer(board));
        let min_last = self.last_layer(board).min(other.last_layer(board));
        if max_first > min_last {
            -1
        } else {
            min_last
        }
    }

    /// Java `shapeLayer(index)`. For the board outline this needs the tile shape count of the
    /// default tree, see [`BasicBoard::shape_layer`].
    pub(crate) fn shape_layer_with_count(&self, board: &BasicBoard, index: i32, outline_shape_count: impl FnOnce() -> i32) -> LayerNo {
        match &self.kind {
            ItemKind::Pin(_) | ItemKind::Via(_) => {
                let index = index.max(0);
                let from_layer = self.first_layer(board);
                let to_layer = self.last_layer(board);
                let index = index.min(to_layer - from_layer);
                from_layer + index
            }
            ItemKind::Trace(t) => t.layer,
            ItemKind::ObstacleArea(a) => a.layer,
            ItemKind::ConductionArea(c) => c.area.layer,
            ItemKind::ComponentOutline(o) => o.layer(board),
            ItemKind::BoardOutline(_) => {
                let shape_count = outline_shape_count();
                let layer_count = board.layer_count();
                let result = if shape_count > 0 { index.wrapping_mul(layer_count) / shape_count } else { 0 };
                if result < 0 || result >= layer_count {
                    log::warn!("BoardOutline.shapeLayer: index out of range");
                }
                result
            }
        }
    }

    // ------------------------------------------------------------------------------------------
    // drill items

    /// Java `DrillItem.getPadstack()`.
    pub fn padstack<'b>(&self, board: &'b BasicBoard) -> Option<&'b Padstack> {
        match &self.kind {
            ItemKind::Via(v) => board.library.padstacks.get(v.padstack),
            ItemKind::Pin(p) => {
                let component = board.components.get(self.component_no);
                let package = board.library.packages.get(component.get_package());
                let padstack_id = package.get_pin(p.pin_index)?.padstack_id;
                board.library.padstacks.get(padstack_id)
            }
            _ => None,
        }
    }

    /// Java `DrillItem.isPlacedOnFront()`.
    pub fn is_placed_on_front(&self, board: &BasicBoard) -> bool {
        match &self.kind {
            ItemKind::Pin(_) => {
                if self.component_no > 0 && self.component_no <= board.components.count() {
                    board.components.get(self.component_no).placed_on_front()
                } else {
                    true
                }
            }
            _ => true,
        }
    }

    /// Java `DrillItem.getCenter()`. Panics for other items.
    pub fn center(&self, board: &BasicBoard) -> Point {
        match &self.kind {
            ItemKind::Via(v) => v.center.clone(),
            ItemKind::Pin(p) => p.center.get_or_init(|| self.calculate_pin_center(p, board)).clone(),
            _ => panic!("Item.center: not a drill item"),
        }
    }

    fn calculate_pin_center(&self, _p: &Pin, board: &BasicBoard) -> Point {
        let component = board.components.get(self.component_no);
        let location = component.get_location().expect("Pin.getCenter: component not placed");
        let mut pin_center = location.translate_by(&self.pin_relative_location(board));
        // check that the pin center is inside the pin shape and correct it eventually
        let padstack = self.padstack(board).expect("Pin.getCenter: padstack not found");
        let count = padstack.to_layer() - padstack.from_layer() + 1;
        let mut current_shape = None;
        for i in 0..count {
            current_shape = self.drill_shape(board, i);
            if current_shape.is_some() {
                break;
            }
        }
        match current_shape {
            None => log::warn!("Pin: At least 1 shape != null expected"),
            Some(shape) => {
                if !shape.contains_inside(&pin_center) {
                    pin_center = Point::Int(shape.centre_of_gravity().round());
                }
            }
        }
        pin_center
    }

    /// Java `Pin.relativeLocation()`.
    pub fn pin_relative_location(&self, board: &BasicBoard) -> Vector {
        let p = self.as_pin().expect("not a pin");
        let component = board.components.get(self.component_no);
        let package = board.library.packages.get(component.get_package());
        let package_pin = package.get_pin(p.pin_index).expect("Pin: package pin not found");
        let mut rel_location = package_pin.relative_location.clone();
        let component_rotation = component.get_rotation_in_degree();
        let flip_rotate_first = board.components.get_flip_style_rotate_first();
        if !component.placed_on_front() && !flip_rotate_first {
            rel_location = package_pin.relative_location.mirror_at_y_axis();
        }
        if component_rotation % 90.0 == 0.0 {
            let factor = (component_rotation as i32) / 90;
            if factor != 0 {
                rel_location = rel_location.turn_90_degree(factor);
            }
        } else {
            // rotation may be not exact
            let location_approx = rel_location
                .to_float()
                .rotate(component_rotation.to_radians(), &FloatPoint::ZERO);
            rel_location = Point::Int(location_approx.round()).difference_by(&Point::ZERO);
        }
        if !component.placed_on_front() && flip_rotate_first {
            rel_location = rel_location.mirror_at_y_axis();
        }
        rel_location
    }

    /// Java `Pin.getPadstackLayer(index)`.
    pub fn pin_padstack_layer(&self, board: &BasicBoard, index: i32) -> LayerNo {
        let padstack = self.padstack(board).expect("Pin: padstack not found");
        let component = board.components.get(self.component_no);
        if component.placed_on_front() || padstack.placed_absolute {
            index + self.first_layer(board)
        } else {
            padstack.board_layer_count() - index - self.first_layer(board) - 1
        }
    }

    /// Java `DrillItem.getShape(index)` (the pad shape on the `index`-th layer of the item).
    pub fn drill_shape(&self, board: &BasicBoard, index: i32) -> Option<Shape> {
        let shapes = self.drill_shapes(board)?;
        shapes.get(index as usize).cloned().flatten()
    }

    /// All pad shapes of a drill item (cached).
    pub fn drill_shapes(&self, board: &BasicBoard) -> Option<Arc<[Option<Shape>]>> {
        match &self.kind {
            ItemKind::Via(v) => Some(v.shapes.get_or_init(|| self.calculate_via_shapes(v, board)).clone()),
            ItemKind::Pin(p) => Some(p.shapes.get_or_init(|| self.calculate_pin_shapes(p, board)).clone()),
            _ => None,
        }
    }

    fn calculate_via_shapes(&self, v: &Via, board: &BasicBoard) -> Arc<[Option<Shape>]> {
        let Some(padstack) = board.library.padstacks.get(v.padstack) else {
            log::warn!("Via.get_shape: padstack is null");
            return Arc::from(Vec::new());
        };
        let count = padstack.to_layer() - padstack.from_layer() + 1;
        let first_layer = self.first_layer(board);
        let translate_vector = v.center.difference_by(&Point::ZERO);
        let mut result = Vec::with_capacity(count.max(0) as usize);
        for i in 0..count {
            let padstack_layer = i + first_layer;
            result.push(padstack.get_shape(padstack_layer).map(|s| s.to_shape().translate_by(&translate_vector)));
        }
        Arc::from(result)
    }

    fn calculate_pin_shapes(&self, p: &Pin, board: &BasicBoard) -> Arc<[Option<Shape>]> {
        let padstack = self.padstack(board).expect("Pin.get_shape: padstack not found");
        let count = (padstack.to_layer() - padstack.from_layer() + 1).max(0);
        let mut result: Vec<Option<Shape>> = vec![None; count as usize];
        let component = board.components.get(self.component_no);
        let package = board.library.packages.get(component.get_package());
        let Some(package_pin) = package.get_pin(p.pin_index) else {
            log::warn!("Pin.get_shape: pinNo out of range");
            return Arc::from(result);
        };
        let mut rel_location = package_pin.relative_location.clone();
        let component_rotation = component.get_rotation_in_degree();
        let flip_rotate_first = board.components.get_flip_style_rotate_first();
        let mirror_on_y_axis = !component.placed_on_front() && !flip_rotate_first;
        if mirror_on_y_axis {
            rel_location = package_pin.relative_location.mirror_at_y_axis();
        }
        let component_translation = component
            .get_location()
            .expect("Pin.get_shape: component not placed")
            .difference_by(&Point::ZERO);
        let zero = IntPoint::new(0, 0);
        for (shape_index, slot) in result.iter_mut().enumerate() {
            let padstack_layer = self.pin_padstack_layer(board, shape_index as i32);
            let Some(pad_shape) = padstack.get_shape(padstack_layer) else {
                continue;
            };
            let mut current_shape = pad_shape.to_shape();
            let pin_rotation = package_pin.rotation_in_degree;
            if pin_rotation % 90.0 == 0.0 {
                let factor = (pin_rotation as i32) / 90;
                if factor != 0 {
                    current_shape = current_shape.turn_90_degree(factor, &zero);
                }
            } else {
                current_shape = current_shape.rotate_approx(pin_rotation.to_radians(), &FloatPoint::ZERO);
            }
            if mirror_on_y_axis {
                current_shape = current_shape.mirror_vertical(&zero);
            }
            // translate the shape first relative to the component
            let mut translated = current_shape.translate_by(&rel_location);
            if component_rotation % 90.0 == 0.0 {
                let factor = (component_rotation as i32) / 90;
                if factor != 0 {
                    translated = translated.turn_90_degree(factor, &zero);
                }
            } else {
                translated = translated.rotate_approx(component_rotation.to_radians(), &FloatPoint::ZERO);
            }
            if !component.placed_on_front() && flip_rotate_first {
                translated = translated.mirror_vertical(&zero);
            }
            *slot = Some(translated.translate_by(&component_translation));
        }
        Arc::from(result)
    }

    /// Java `DrillItem.getShapeOnLayer(layer)`.
    pub fn drill_shape_on_layer(&self, board: &BasicBoard, layer: LayerNo) -> Option<Shape> {
        let from = self.first_layer(board);
        let to = self.last_layer(board);
        if layer < from || layer > to {
            log::warn!("DrillItem.get_shape_on_layer: layer out of range");
            return None;
        }
        self.drill_shape(board, layer - from)
    }

    /// Java `DrillItem.smallestRadius()`.
    pub fn drill_smallest_radius(&self, board: &BasicBoard) -> f64 {
        let mut result = f64::MAX;
        let c = self.center(board).to_float();
        for i in 0..self.drill_tile_shape_count(board) {
            if let Some(shape) = self.drill_shape(board, i) {
                result = result.min(shape.border_distance(&c));
            }
        }
        result
    }

    /// Java `DrillItem.tileShapeCount()`: `padstack.toLayer() - padstack.fromLayer() + 1`.
    pub fn drill_tile_shape_count(&self, board: &BasicBoard) -> i32 {
        match self.padstack(board) {
            Some(p) => p.to_layer() - p.from_layer() + 1,
            None => 0,
        }
    }

    /// Java `DrillItem.minWidth()` (not cached).
    pub fn drill_min_width(&self, board: &BasicBoard) -> f64 {
        let mut min_width = i32::MAX as f64;
        for layer in self.first_layer(board)..=self.last_layer(board) {
            if !board.layer_structure.layers[layer as usize].is_signal {
                continue;
            }
            if let Some(shape) = self.drill_shape_on_layer(board, layer) {
                let b = shape.bounding_box();
                min_width = min_width.min(b.width() as f64);
                min_width = min_width.min(b.height() as f64);
            }
        }
        min_width
    }

    // ------------------------------------------------------------------------------------------
    // traces

    /// Java `Trace.firstCorner()`.
    pub fn first_corner(&self) -> Point {
        self.trace().polyline.corner(0)
    }

    /// Java `Trace.lastCorner()`.
    pub fn last_corner(&self) -> Point {
        let pl = &self.trace().polyline;
        pl.corner(pl.lines.len() as i32 - 2)
    }

    // ------------------------------------------------------------------------------------------
    // bounding box

    /// Java `boundingBox()`.
    pub fn bounding_box(&self, board: &BasicBoard) -> IntBox {
        match &self.kind {
            ItemKind::Pin(_) | ItemKind::Via(_) => {
                let mut result = IntBox::EMPTY;
                for i in 0..self.drill_tile_shape_count(board) {
                    if let Some(shape) = self.drill_shape(board, i) {
                        result = result.union_int_box(&shape.bounding_box());
                    }
                }
                result
            }
            ItemKind::Trace(t) => t.polyline.bounding_box().offset(t.half_width as f64),
            ItemKind::ObstacleArea(a) => a.get_area(board).bounding_box(),
            ItemKind::ConductionArea(c) => c.area.get_area(board).bounding_box(),
            ItemKind::ComponentOutline(o) => o.get_area(board).bounding_box(),
            ItemKind::BoardOutline(o) => {
                let mut result = IntBox::EMPTY;
                for s in o.shapes.iter() {
                    result = result.union_int_box(&s.bounding_box());
                }
                result
            }
        }
    }

    // ------------------------------------------------------------------------------------------
    // derived data

    /// Java `clearDerivedData()` for the data cached on the item itself (the precalculated
    /// tree shapes are cleared by the board, see [`BasicBoard::clear_derived_data`]).
    pub fn clear_derived_data(&mut self) {
        match &mut self.kind {
            ItemKind::Pin(p) => {
                p.shapes = OnceLock::new();
            }
            ItemKind::Via(v) => {
                v.shapes = OnceLock::new();
            }
            ItemKind::ObstacleArea(a) => a.absolute_area = OnceLock::new(),
            ItemKind::ConductionArea(c) => c.area.absolute_area = OnceLock::new(),
            ItemKind::ComponentOutline(o) => o.absolute_area = OnceLock::new(),
            ItemKind::Trace(_) | ItemKind::BoardOutline(_) => {}
        }
    }

    /// Java `Pin.setCenter(null)` + `clearDerivedData` (used when the component was turned,
    /// rotated or flipped).
    pub fn reset_pin_center(&mut self) {
        if let ItemKind::Pin(p) = &mut self.kind {
            p.center = OnceLock::new();
        }
        self.clear_derived_data();
    }

    /// Java `copy(id)`: a copy of this item with another id (not on the board, without caches
    /// that depend on the id).
    pub fn copy_with_id(&self, id: ItemId) -> Item {
        let mut result = self.clone();
        result.id = id;
        result.on_board = false;
        result.smallest_clearance = -1.0;
        if let ItemKind::Pin(p) = &mut result.kind {
            p.changed_to = None;
        }
        result
    }
}

impl PolylineTrace {
    /// Java `polyline()`.
    pub fn polyline(&self) -> &Polyline {
        &self.polyline
    }
    /// The polyline with the identities of its lines.
    pub fn tpolyline(&self) -> super::optimize::tracked::TPolyline {
        super::optimize::tracked::TPolyline::new(self.polyline.clone(), self.line_ids.clone())
    }
    /// Sets the polyline and the identities of its lines (Java `lines = newPolyline`).
    pub(crate) fn set_tpolyline(&mut self, polyline: super::optimize::tracked::TPolyline) {
        debug_assert_eq!(polyline.polyline.lines.len(), polyline.ids.len());
        self.polyline = polyline.polyline;
        self.line_ids = polyline.ids;
    }
    /// Java `getLayer()`.
    pub fn layer(&self) -> LayerNo {
        self.layer
    }
    /// Java `getHalfWidth()`.
    pub fn half_width(&self) -> i32 {
        self.half_width
    }
    /// Java `cornerCount()`.
    pub fn corner_count(&self) -> i32 {
        self.polyline.lines.len() as i32 - 1
    }
    /// Java `getLength()`.
    pub fn length(&self) -> f64 {
        self.polyline.length_approx()
    }
    /// Java `tileShapeCount()`.
    pub fn tile_shape_count(&self) -> i32 {
        (self.polyline.lines.len() as i32 - 2).max(0)
    }
    /// Java `firstCorner()`.
    pub fn first_corner(&self) -> Point {
        self.polyline.corner(0)
    }
    /// Java `lastCorner()`.
    pub fn last_corner(&self) -> Point {
        self.polyline.corner(self.polyline.lines.len() as i32 - 2)
    }
}

impl Via {
    /// The via padstack.
    pub fn padstack_no(&self) -> PadstackNo {
        self.padstack
    }
}

impl ObstacleArea {
    pub(crate) fn new(
        kind: ObstacleKind,
        area: Area,
        layer: LayerNo,
        translation: Vector,
        rotation_in_degree: f64,
        side_changed: bool,
        name: Option<String>,
    ) -> ObstacleArea {
        ObstacleArea {
            kind,
            name,
            relative_area: Arc::new(area),
            layer,
            translation,
            rotation_in_degree,
            side_changed,
            absolute_area: OnceLock::new(),
        }
    }

    /// Java `getLayer()`.
    pub fn layer(&self) -> LayerNo {
        self.layer
    }
    /// Java `getRelativeArea()`.
    pub fn relative_area(&self) -> &Area {
        &self.relative_area
    }
    /// Java `getTranslation()`.
    pub fn translation(&self) -> &Vector {
        &self.translation
    }
    /// Java `getRotationInDegree()`.
    pub fn rotation_in_degree(&self) -> f64 {
        self.rotation_in_degree
    }
    /// Java `getSideChanged()`.
    pub fn side_changed(&self) -> bool {
        self.side_changed
    }

    /// Java `getArea()`: the absolute area (cached).
    pub fn get_area(&self, board: &BasicBoard) -> Arc<Area> {
        self.absolute_area
            .get_or_init(|| {
                Arc::new(transform_area(
                    &self.relative_area,
                    self.side_changed,
                    self.rotation_in_degree,
                    &self.translation,
                    board.components.get_flip_style_rotate_first(),
                ))
            })
            .clone()
    }

    /// Java `splitToConvex()`.
    pub fn split_to_convex(&self, board: &BasicBoard) -> Option<Vec<TileShape>> {
        self.get_area(board).split_to_convex()
    }
}

/// The transformation of `ObstacleArea.getArea` / `ComponentOutline.getArea`.
fn transform_area(relative: &Area, mirrored: bool, rotation: f64, translation: &Vector, flip_style_rotate_first: bool) -> Area {
    let zero = IntPoint::new(0, 0);
    let mut turned = relative.clone();
    if mirrored && !flip_style_rotate_first {
        turned = turned.mirror_vertical(&zero);
    }
    if rotation != 0.0 {
        if rotation % 90.0 == 0.0 {
            turned = turned.turn_90_degree((rotation as i32) / 90, &zero);
        } else {
            turned = turned.rotate_approx(rotation.to_radians(), &FloatPoint::ZERO);
        }
    }
    if mirrored && flip_style_rotate_first {
        turned = turned.mirror_vertical(&zero);
    }
    turned.translate_by(translation)
}

impl ConductionArea {
    /// The `ObstacleArea` part.
    pub fn area(&self) -> &ObstacleArea {
        &self.area
    }
    /// Java `getIsObstacle()`.
    pub fn is_obstacle(&self) -> bool {
        self.is_obstacle
    }
    /// Java `setIsObstacle(value)` (plain field update; callers reinsert the tree items).
    pub fn set_is_obstacle(&mut self, value: bool) {
        self.is_obstacle = value;
    }
    /// Java `getIsFilled()`.
    pub fn is_filled(&self) -> bool {
        self.is_filled
    }
}

impl ComponentOutline {
    /// Java `getLayer()`.
    pub fn layer(&self, board: &BasicBoard) -> LayerNo {
        if self.is_front {
            0
        } else {
            board.layer_count() - 1
        }
    }
    /// Java `isFront()`.
    pub fn is_front(&self) -> bool {
        self.is_front
    }
    /// Java `getArea()` (cached).
    pub fn get_area(&self, board: &BasicBoard) -> Arc<Area> {
        self.absolute_area
            .get_or_init(|| {
                Arc::new(transform_area(
                    &self.relative_area,
                    !self.is_front,
                    self.rotation_in_degree,
                    &self.translation,
                    board.components.get_flip_style_rotate_first(),
                ))
            })
            .clone()
    }
}

impl BoardOutline {
    /// Java `shapeCount()`.
    pub fn shape_count(&self) -> i32 {
        self.shapes.len() as i32
    }
    /// Java `getShape(index)`.
    pub fn shape(&self, index: i32) -> Option<&PolylineShape> {
        self.shapes.get(index as usize)
    }
    /// All outline shapes.
    pub fn shapes(&self) -> &[PolylineShape] {
        &self.shapes
    }
    /// Java `lineCount()`.
    pub fn line_count(&self) -> i32 {
        self.shapes.iter().map(|s| s.border_line_count()).sum()
    }
    /// Java `keepoutOutsideOutlineGenerated()`.
    pub fn keepout_outside_outline_generated(&self) -> bool {
        self.keepout_outside_outline
    }
    /// Java `getKeepoutArea()`: the board box with the outline shapes as holes (cached at the
    /// first call, with the board bounding box of that time, like Java).
    pub fn keepout_area(&self, board_box: &IntBox) -> Arc<Area> {
        self.keepout_area
            .get_or_init(|| {
                let border = PolylineShape::Tile(TileShape::IntBox(*board_box));
                Arc::new(Area::PolylineArea(PolylineArea::new(border, self.shapes.to_vec())))
            })
            .clone()
    }
    /// Java `contains(Point)`.
    pub fn contains(&self, point: &Point) -> bool {
        self.shapes.iter().any(|s| s.contains(point))
    }
    /// Java `contains(FloatPoint)`.
    pub fn contains_float(&self, point: &FloatPoint) -> bool {
        self.shapes.iter().any(|s| s.contains_float(point))
    }
}

/// Java `Pin.getBasePinName`: strips composite sub-pad suffixes (`@1`, `#1`, `_1`, `-1`).
pub fn base_pin_name(pin_name: &str) -> String {
    // Java works on UTF-16 indices; the searched characters are ASCII, so byte indices of the
    // found positions give the same substrings.
    if let Some(at) = pin_name.find('@') {
        return if at > 0 { pin_name[..at].to_string() } else { "@".to_string() };
    }
    if let Some(hash) = pin_name.find('#') {
        return if hash > 0 { pin_name[..hash].to_string() } else { "#".to_string() };
    }
    let len = pin_name.len();
    if let Some(u) = pin_name.rfind('_') {
        if u > 0 && u < len - 1 && is_all_digits(&pin_name[u + 1..]) {
            return pin_name[..u].to_string();
        }
    }
    if let Some(h) = pin_name.rfind('-') {
        if h > 0 && h < len - 1 && is_all_digits(&pin_name[h + 1..]) {
            return pin_name[..h].to_string();
        }
    }
    pin_name.to_string()
}

/// Java `Character.isDigit` over all chars (Unicode decimal digits).
fn is_all_digits(s: &str) -> bool {
    s.chars().all(is_unicode_decimal_digit)
}

fn is_unicode_decimal_digit(c: char) -> bool {
    // Character.isDigit is true for Unicode category Nd. `char::is_numeric` also covers Nl/No,
    // so restrict to characters that have a decimal digit value in common Nd blocks.
    c.is_ascii_digit()
        || ('\u{0660}'..='\u{0669}').contains(&c)
        || ('\u{06F0}'..='\u{06F9}').contains(&c)
        || ('\u{0966}'..='\u{096F}').contains(&c)
        || ('\u{FF10}'..='\u{FF19}').contains(&c)
}
