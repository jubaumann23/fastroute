//! Port of `board/facade/RoutingBoard.java`, `RoutingBoardOperations.java`,
//! `RoutingBoardSearchFacade.java` and `RoutingBoardUndoFacade.java` (the headless parts), and of
//! `autoroute/RoutingFailureLog.java`.
//!
//! [`RoutingBoard`] wraps a [`BasicBoard`] (it dereferences to it) and adds the routing state of
//! the Java subclass. The changed area (`RoutingBoard.changedArea`) and the autoroute database
//! hook (`additionalUpdateAfterChange`) already live in [`BasicBoard`]
//! ([`BasicBoard::changed_area`], [`BasicBoard::autoroute_maintenance`]).
//!
//! Not ported (GUI only, not reachable from the headless autorouter / optimizer):
//! `removeItemsAndPullTight`, `moveDrillItem`, `pickNearestRoutingItem`, `checkMoveItem`,
//! `checkChangeNet`, `redo`, `changeConductionIsObstacle` (deprecated alias). `forcedVia` is only
//! called by the GUI but ported because it is a thin wrapper of
//! [`forced_via_inserter::insert`](super::actions::forced_via_inserter::insert).
//!
//! The autoroute entry points (`initAutoroute`, `autoroute`, `fanout`, `autorouteConnection`) are
//! implemented in [`crate::autoroute::router`] (porting unit U8).
//!
//! Time limits: every limit Java creates with `new TimeLimit(ms)` is created through
//! [`RoutingBoard::time_limits`] ([`TimeLimitPolicy`]), so a deterministic run can disable them.

use std::collections::BTreeMap;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use fr_geom::{
    ConvexShape, FloatPoint, IntBox, IntOctagon, IntPoint, LineSegment, Point, Polyline, PolylineShape, TileShape,
};
use fr_settings::ExpansionCostFactor;

use crate::datastructures::{StopToken, TimeLimit};
use crate::ids::{AngleRestriction, ClearanceClassNo, FixedState, LayerNo, NetNo};
use crate::library::BoardLibrary;
use crate::rules::{BoardRules, ViaInfo};
use crate::structure::{ChangedArea, Communication, Components, LayerStructure, ShapeEntrySide};

use super::basic_board::BasicBoard;
use super::item::{ItemKey, StopConnectionOption};
use super::item_list::ItemSet;
use super::optimize::trace_shover::TraceShover;
use super::optimize::trace_tightener::TraceTightener;
use super::optimize::tracked::TPolyline;
use super::search_tree::{SearchTreeManager, DEFAULT_TREE};
use super::selection_filter::{ItemSelectionFilter, SelectableChoices};

/// Java `AutorouteAttemptState` (used by the failure log; the autorouter, U8, produces it).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AutorouteAttemptState {
    Unknown,
    Skipped,
    NoUnconnectedNets,
    ConnectedToPlane,
    AlreadyConnected,
    NoConnections,
    Routed,
    Failed,
    InsertError,
}

/// Java `RoutingFailureLog.ItemFailureInfo`.
#[derive(Clone, Debug)]
pub struct ItemFailureInfo {
    pub item: ItemKey,
    pub net_number: NetNo,
    pub failure_count: i32,
    pub last_failure_state: Option<AutorouteAttemptState>,
    pub last_failure_reason: String,
    pub last_attempt_pass: i64,
}

/// Java `RoutingFailureLog`: routing failures per item id. Not undone by
/// [`BasicBoard::undo`]; copied by [`RoutingBoard::deep_copy`] (Java serializes it).
#[derive(Clone, Debug, Default)]
pub struct RoutingFailureLog {
    failures: BTreeMap<i32, ItemFailureInfo>,
}

impl RoutingFailureLog {
    /// Java `FAILURE_THRESHOLD`.
    pub const FAILURE_THRESHOLD: i32 = 50;

    /// Java `recordFailure(item, passNo, state, reason)`.
    pub fn record_failure(&mut self, board: &BasicBoard, item: ItemKey, pass_no: i32, state: AutorouteAttemptState, reason: &str) {
        let it = board.item(item);
        let info = self.failures.entry(it.id().0).or_insert_with(|| ItemFailureInfo {
            item,
            net_number: if it.net_count() > 0 { it.net_number(0) } else { -1 },
            failure_count: 0,
            last_failure_state: None,
            last_failure_reason: String::new(),
            last_attempt_pass: 0,
        });
        info.failure_count += 1;
        info.last_attempt_pass = pass_no as i64;
        info.last_failure_state = Some(state);
        info.last_failure_reason = reason.to_string();
    }

    /// Java `shouldSkip(item)`.
    pub fn should_skip(&self, board: &BasicBoard, item: ItemKey) -> bool {
        self.failures
            .get(&board.item(item).id().0)
            .map(|i| i.failure_count >= Self::FAILURE_THRESHOLD)
            .unwrap_or(false)
    }

    /// Java `getFailureCount(item)`.
    pub fn get_failure_count(&self, board: &BasicBoard, item: ItemKey) -> i32 {
        self.failures.get(&board.item(item).id().0).map(|i| i.failure_count).unwrap_or(0)
    }

    /// Java `getUnroutableItems()` (ordered by item id; Java iterates a `ConcurrentHashMap`).
    pub fn get_unroutable_items(&self) -> Vec<&ItemFailureInfo> {
        self.failures.values().filter(|i| i.failure_count >= Self::FAILURE_THRESHOLD).collect()
    }

    /// Java `hasUnroutableItems()`.
    pub fn has_unroutable_items(&self) -> bool {
        self.failures.values().any(|i| i.failure_count >= Self::FAILURE_THRESHOLD)
    }

    /// Java `clear()`.
    pub fn clear(&mut self) {
        self.failures.clear();
    }
}

/// How the Java `new TimeLimit(ms)` calls of the board algorithms are created.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeLimitMode {
    /// Java wall clock limits.
    WallClock,
    /// Limits never fire.
    Disabled,
    /// Deterministic call budget of the Java parity build (`limit_ms * factor` calls, see
    /// [`TimeLimit::Count`]).
    Count { factor: i64 },
}

/// How the Java `new TimeLimit(ms)` calls of the board algorithms are created.
#[derive(Clone, Debug)]
pub struct TimeLimitPolicy {
    pub mode: TimeLimitMode,
    /// Set when a limit fired (with wall clock limits the run is then time dependent).
    pub fired: Option<Arc<AtomicBool>>,
}

impl Default for TimeLimitPolicy {
    fn default() -> Self {
        TimeLimitPolicy { mode: TimeLimitMode::WallClock, fired: None }
    }
}

impl TimeLimitPolicy {
    /// A policy creating only disabled limits.
    pub fn disabled() -> Self {
        TimeLimitPolicy { mode: TimeLimitMode::Disabled, fired: None }
    }

    /// The deterministic count mode of the Java parity build (default factor 10).
    pub fn count() -> Self {
        TimeLimitPolicy { mode: TimeLimitMode::Count { factor: crate::datastructures::time_limit::DEFAULT_COUNT_FACTOR }, fired: None }
    }

    /// Java `new TimeLimit(milliSeconds)`.
    pub fn make(&self, milli_seconds: i32) -> TimeLimit {
        let t = match self.mode {
            TimeLimitMode::WallClock => TimeLimit::wall_clock(milli_seconds),
            TimeLimitMode::Disabled => TimeLimit::disabled(milli_seconds),
            TimeLimitMode::Count { factor } => TimeLimit::count(milli_seconds, factor),
        };
        match &self.fired {
            Some(f) => t.with_fired_flag(f.clone()),
            None => t,
        }
    }
}

/// The point returned by Java `insertForcedTraceSegment` / `insertForcedTracePolyline`, with
/// the object identity the Java callers compare with `==` (`okPoint == toCorner`).
#[derive(Clone, Debug, PartialEq)]
pub enum ForcedTraceEnd {
    /// Java `null`: an error occurred while inserting, the database may be damaged (undo).
    Failed,
    /// The `fromCorner` object: nothing (or nothing more) could be inserted.
    From(Point),
    /// The `toCorner` object: the whole segment was inserted.
    To(Point),
    /// Another point up to which the trace was inserted.
    Other(Point),
}

impl ForcedTraceEnd {
    /// The point (`None` for [`ForcedTraceEnd::Failed`]).
    pub fn point(&self) -> Option<&Point> {
        match self {
            ForcedTraceEnd::Failed => None,
            ForcedTraceEnd::From(p) | ForcedTraceEnd::To(p) | ForcedTraceEnd::Other(p) => Some(p),
        }
    }

    /// Java `result == toCorner`.
    pub fn is_to(&self) -> bool {
        matches!(self, ForcedTraceEnd::To(_))
    }
}

/// Java `RoutingBoard`.
#[derive(Clone, Debug)]
pub struct RoutingBoard {
    pub basic: BasicBoard,
    /// Java `failureLog`.
    pub failure_log: RoutingFailureLog,
    shove_failing_obstacle: Option<ItemKey>,
    shove_failing_layer: LayerNo,
    /// Creation of the Java wall clock time limits.
    pub time_limits: TimeLimitPolicy,
    // pcbkit hook H5: order seed for the pipeline's autorouter (None = upstream behaviour).
    pub order_seed: Option<i64>,
    // pcbkit R2: the most rounds one pull-tight pass may run (0 = unlimited, upstream behaviour).
    pub tighten_round_cap: u32,
    // pcbkit hook H6: when Some, only nets with `mask[net_no] == true` are routed or fanned out.
    pub route_nets: Option<Vec<bool>>,
    /// Java `autorouteEngine` (transient: not copied by `clone`, see
    /// [`crate::autoroute::router`]).
    pub autoroute_engine: crate::autoroute::router::EngineSlot,
}

impl Deref for RoutingBoard {
    type Target = BasicBoard;
    fn deref(&self) -> &BasicBoard {
        &self.basic
    }
}

impl DerefMut for RoutingBoard {
    fn deref_mut(&mut self) -> &mut BasicBoard {
        &mut self.basic
    }
}

impl RoutingBoard {
    /// Java `new RoutingBoard(boundingBox, layerStructure, outlineShapes, outlineClClassNo, rules,
    /// communication)` (library and components passed in, see [`BasicBoard::new`]).
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
    ) -> RoutingBoard {
        let basic =
            BasicBoard::new(bounding_box, layer_structure, outline_shapes, outline_cl_class_no, rules, library, components, communication);
        RoutingBoard::from_basic(basic)
    }

    /// Wraps a loaded board.
    pub fn from_basic(basic: BasicBoard) -> RoutingBoard {
        RoutingBoard {
            basic,
            failure_log: RoutingFailureLog::default(),
            shove_failing_obstacle: None,
            shove_failing_layer: -1,
            time_limits: TimeLimitPolicy::default(),
            order_seed: None,
            tighten_round_cap: 0,
            route_nets: None,
            autoroute_engine: Default::default(),
        }
    }

    /// The wrapped board.
    pub fn into_basic(self) -> BasicBoard {
        self.basic
    }

    // ------------------------------------------------------------------------------------------
    // shove failure diagnostics

    /// Java `getShoveFailingObstacle()`.
    pub fn shove_failing_obstacle(&self) -> Option<ItemKey> {
        self.shove_failing_obstacle
    }

    /// Java `setShoveFailingObstacle(item)`.
    pub fn set_shove_failing_obstacle(&mut self, item: Option<ItemKey>) {
        self.shove_failing_obstacle = item;
    }

    /// Java `getShoveFailingLayer()`.
    pub fn shove_failing_layer(&self) -> LayerNo {
        self.shove_failing_layer
    }

    /// Java `setShoveFailingLayer(layer)`.
    pub fn set_shove_failing_layer(&mut self, layer: LayerNo) {
        self.shove_failing_layer = layer;
    }

    /// Java `clearShoveFailingObstacle()`.
    pub fn clear_shove_failing_obstacle(&mut self) {
        self.shove_failing_obstacle = None;
        self.shove_failing_layer = -1;
    }

    // ------------------------------------------------------------------------------------------
    // changed area (RoutingBoardOperations)

    /// Java `startMarkingChangedArea()`.
    pub fn start_marking_changed_area(&mut self) {
        if self.basic.changed_area.is_none() {
            let lc = self.layer_count();
            self.basic.changed_area = Some(ChangedArea::new(lc));
        }
    }

    /// Java `joinChangedArea(point, layer)`.
    pub fn join_changed_area(&mut self, point: &FloatPoint, layer: LayerNo) {
        if let Some(c) = &mut self.basic.changed_area {
            c.join(point, layer);
        }
    }

    /// Java `markAllChangedArea()`.
    pub fn mark_all_changed_area(&mut self) {
        self.start_marking_changed_area();
        let b = self.basic.bounding_box;
        let corners = [
            b.ll.to_float(),
            FloatPoint::new(b.ur.x as f64, b.ll.y as f64),
            b.ur.to_float(),
            FloatPoint::new(b.ll.x as f64, b.ur.y as f64),
        ];
        for layer in 0..self.layer_count() {
            for c in &corners {
                self.join_changed_area(c, layer);
            }
        }
    }

    /// Java `optChangedArea(onlyNetNoArr, clipShape, accuracy, traceCosts, stoppableThread,
    /// timeLimit)`: optimizes the route in the marked area and ends the marking.
    pub fn opt_changed_area(
        &mut self,
        only_net_no_arr: &[NetNo],
        clip_shape: Option<IntOctagon>,
        accuracy: i32,
        trace_costs: Option<&[ExpansionCostFactor]>,
        stop: Option<&StopToken>,
        time_limit_ms: i32,
    ) {
        self.opt_changed_area_keep(only_net_no_arr, clip_shape, accuracy, trace_costs, stop, time_limit_ms, None, 0);
    }

    /// Java `optChangedArea(..., keepPoint, keepPointLayer)`: traces on `keep_point_layer`
    /// containing `keep_point` still contain it after optimizing.
    #[allow(clippy::too_many_arguments)]
    pub fn opt_changed_area_keep(
        &mut self,
        only_net_no_arr: &[NetNo],
        clip_shape: Option<IntOctagon>,
        accuracy: i32,
        trace_costs: Option<&[ExpansionCostFactor]>,
        stop: Option<&StopToken>,
        time_limit_ms: i32,
        keep_point: Option<Point>,
        keep_point_layer: LayerNo,
    ) {
        if self.basic.changed_area.is_none() {
            return;
        }
        // (Java skips the optimization if clipShape is the IntOctagon.EMPTY object; only the GUI
        // passes it)
        let mut algo = TraceTightener::get_instance(
            self,
            only_net_no_arr,
            clip_shape,
            accuracy,
            stop.cloned(),
            time_limit_ms,
            keep_point,
            keep_point_layer,
        );
        algo.opt_changed_area(self, trace_costs);
        self.basic.changed_area = None;
    }

    // ------------------------------------------------------------------------------------------
    // RoutingBoardSearchFacade

    /// Java `checkTraceSegment(fromPoint, toPoint, layer, netNumbers, traceHalfWidth, clClassNo,
    /// onlyNotShovableObstacles)`: the maximal length of the segment from `from_point` that can be
    /// inserted without conflict (`i32::MAX` if no conflict).
    #[allow(clippy::too_many_arguments)]
    pub fn check_trace_segment(
        &self,
        from_point: &Point,
        to_point: &Point,
        layer: LayerNo,
        net_numbers: &[NetNo],
        trace_half_width: i32,
        cl_class_no: ClearanceClassNo,
        only_not_shovable_obstacles: bool,
    ) -> f64 {
        if from_point == to_point {
            return 0.0;
        }
        let polyline = Polyline::from_two_points(from_point, to_point);
        let segment = LineSegment::from_polyline(&polyline, 1).expect("checkTraceSegment: line segment");
        self.check_trace_segment_ls(&segment, layer, net_numbers, trace_half_width, cl_class_no, only_not_shovable_obstacles)
    }

    /// Java `checkTraceSegment(LineSegment, layer, netNumbers, traceHalfWidth, clClassNo,
    /// onlyNotShovableObstacles)`.
    pub fn check_trace_segment_ls(
        &self,
        line_segment: &LineSegment,
        layer: LayerNo,
        net_numbers: &[NetNo],
        trace_half_width: i32,
        cl_class_no: ClearanceClassNo,
        only_not_shovable_obstacles: bool,
    ) -> f64 {
        let check_polyline = line_segment.to_polyline();
        if check_polyline.lines.len() != 3 {
            return 0.0;
        }
        let shape_to_check = check_polyline.offset_shape(trace_half_width, 0).expect("checkTraceSegment: offset shape");
        let from_point = line_segment.start_point_approx();
        let to_point = line_segment.end_point_approx();
        let line_length = to_point.distance(&from_point);
        let mut ok_length = i32::MAX as f64;
        let compensated = self.default_tree().is_clearance_compensation_used();
        let entries = self.overlapping_tree_entries_with_clearance(
            DEFAULT_TREE,
            &ConvexShape::Tile(shape_to_check.clone()),
            layer,
            net_numbers,
            cl_class_no,
        );
        for entry in entries {
            let Some(obstacle) = entry.object.item() else { continue };
            let item = self.item(obstacle);
            if only_not_shovable_obstacles && item.is_routable() && !self.is_shove_fixed(obstacle) {
                continue;
            }
            let obstacle_shape = self.tree_shape(DEFAULT_TREE, obstacle, entry.shape_index).expect("checkTraceSegment: tree shape");
            let (offset_shape, shorten_value) = if compensated {
                let v = trace_half_width as f64
                    + self.rules.clearance_matrix.clearance_compensation_value(item.clearance_class(), layer) as f64;
                (shape_to_check.clone(), v)
            } else {
                let clearance_value = self.clearance_value(item.clearance_class(), cl_class_no, layer);
                (shape_to_check.offset(clearance_value as f64), (trace_half_width + clearance_value) as f64)
            };
            let intersection = obstacle_shape.intersection(&offset_shape);
            if intersection.is_empty() {
                continue;
            }
            let nearest = intersection.nearest_point_approx(&from_point).expect("checkTraceSegment: nearest point");
            let mut projection = from_point.scalar_product(&to_point, &nearest) / line_length;
            projection = jmax(0.0, projection - shorten_value - 1.0);
            if projection < ok_length {
                ok_length = projection;
                if ok_length <= 0.0 {
                    return 0.0;
                }
            }
        }
        ok_length
    }

    // ------------------------------------------------------------------------------------------
    // forced vias and traces

    /// Java `forcedVia(viaInfo, location, netNumbers, traceClearanceClassIndex,
    /// tracePenHalfwidthArr, maxRecursionDepth, maxViaRecursionDepth, tidyWidth,
    /// pullTightAccuracy, pullTightTimeLimit)`.
    #[allow(clippy::too_many_arguments)]
    pub fn forced_via(
        &mut self,
        via_info: &ViaInfo,
        location: &Point,
        net_numbers: &[NetNo],
        trace_clearance_class: ClearanceClassNo,
        trace_pen_half_widths: &[i32],
        max_recursion_depth: i32,
        max_via_recursion_depth: i32,
        tidy_width: i32,
        pull_tight_accuracy: i32,
        pull_tight_time_limit: i32,
    ) -> bool {
        self.clear_shove_failing_obstacle();
        self.start_marking_changed_area();
        let result = super::actions::forced_via_inserter::insert(
            self,
            via_info,
            location,
            net_numbers,
            trace_clearance_class,
            trace_pen_half_widths,
            max_recursion_depth,
            max_via_recursion_depth,
        );
        if result {
            let tidy_clip_shape = if tidy_width < i32::MAX { Some(location.surrounding_octagon().enlarge(tidy_width as f64)) } else { None };
            let opt_nets: Vec<NetNo> = if max_recursion_depth <= 0 { net_numbers.to_vec() } else { Vec::new() };
            self.opt_changed_area(&opt_nets, tidy_clip_shape, pull_tight_accuracy, None, None, pull_tight_time_limit);
        }
        result
    }

    /// Java `insertForcedTraceSegment(fromCorner, toCorner, halfWidth, layer, netNumbers,
    /// clearanceClassIndex, maxRecursionDepth, maxViaRecursionDepth, maxSpringOverRecursionDepth,
    /// tidyWidth, pullTightAccuracy, withCheck, timeLimit)`.
    #[allow(clippy::too_many_arguments)]
    pub fn insert_forced_trace_segment(
        &mut self,
        from_corner: &Point,
        to_corner: &Point,
        half_width: i32,
        layer: LayerNo,
        net_numbers: &[NetNo],
        clearance_class: ClearanceClassNo,
        max_recursion_depth: i32,
        max_via_recursion_depth: i32,
        max_spring_over_recursion_depth: i32,
        tidy_width: i32,
        pull_tight_accuracy: i32,
        with_check: bool,
        time_limit: Option<&TimeLimit>,
    ) -> ForcedTraceEnd {
        if from_corner == to_corner {
            return ForcedTraceEnd::To(to_corner.clone());
        }
        let insert_polyline = Polyline::from_two_points(from_corner, to_corner);
        let ok_point = self.insert_forced_trace_polyline(
            &insert_polyline,
            half_width,
            layer,
            net_numbers,
            clearance_class,
            max_recursion_depth,
            max_via_recursion_depth,
            max_spring_over_recursion_depth,
            tidy_width,
            pull_tight_accuracy,
            with_check,
            time_limit,
        );
        // the corners of insertPolyline are identified with the argument objects
        match ok_point {
            ForcedTraceEnd::From(_) => ForcedTraceEnd::From(from_corner.clone()),
            ForcedTraceEnd::To(_) => ForcedTraceEnd::To(to_corner.clone()),
            other => other,
        }
    }

    /// Java `checkForcedTracePolyline(polyline, halfWidth, layer, netNumbers,
    /// clearanceClassIndex, maxRecursionDepth, maxViaRecursionDepth,
    /// maxSpringOverRecursionDepth)`.
    #[allow(clippy::too_many_arguments)]
    pub fn check_forced_trace_polyline(
        &mut self,
        polyline: &Polyline,
        half_width: i32,
        layer: LayerNo,
        net_numbers: &[NetNo],
        clearance_class: ClearanceClassNo,
        max_recursion_depth: i32,
        max_via_recursion_depth: i32,
        max_spring_over_recursion_depth: i32,
    ) -> bool {
        let compensated_half_width = half_width + self.clearance_compensation_value(DEFAULT_TREE, clearance_class, layer);
        let trace_shapes = polyline.offset_shapes_range(compensated_half_width, 0, polyline.lines.len() as i32 - 1);
        let orthogonal_mode = self.rules.get_trace_angle_restriction() == AngleRestriction::NinetyDegree;
        for (i, shape) in trace_shapes.into_iter().enumerate() {
            let current_trace_shape = if orthogonal_mode { TileShape::IntBox(shape.bounding_box()) } else { shape };
            let from_side = ShapeEntrySide::from_polyline(polyline, i as i32 + 1, &current_trace_shape);
            if !TraceShover::check(
                self,
                &current_trace_shape,
                Some(&from_side),
                None,
                layer,
                net_numbers,
                clearance_class,
                max_recursion_depth,
                max_via_recursion_depth,
                max_spring_over_recursion_depth,
                None,
            ) {
                return false;
            }
        }
        true
    }

    /// Java `insertForcedTracePolyline(polyline, halfWidth, layer, netNumbers,
    /// clearanceClassIndex, maxRecursionDepth, maxViaRecursionDepth, maxSpringOverRecursionDepth,
    /// tidyWidth, pullTightAccuracy, withCheck, timeLimit)`: inserts the polyline while shoving
    /// obstacles aside. Returns the last corner up to which the shove succeeded
    /// ([`ForcedTraceEnd::From`] / [`ForcedTraceEnd::To`] are `polyline`'s first / last corner).
    #[allow(clippy::too_many_arguments)]
    pub fn insert_forced_trace_polyline(
        &mut self,
        polyline: &Polyline,
        half_width: i32,
        layer: LayerNo,
        net_numbers: &[NetNo],
        clearance_class: ClearanceClassNo,
        max_recursion_depth: i32,
        max_via_recursion_depth: i32,
        max_spring_over_recursion_depth: i32,
        tidy_width: i32,
        pull_tight_accuracy: i32,
        with_check: bool,
        time_limit: Option<&TimeLimit>,
    ) -> ForcedTraceEnd {
        self.clear_shove_failing_obstacle();
        if polyline.lines.len() < 3 {
            // Java: corner(i) of a degenerate polyline is null
            return ForcedTraceEnd::Failed;
        }
        let from_corner = polyline.first_corner();
        let to_corner = polyline.last_corner();
        if from_corner == to_corner {
            return ForcedTraceEnd::To(to_corner);
        }
        if !(from_corner.is_int_point() && to_corner.is_int_point()) {
            log::warn!("RoutingBoard.insert_forced_trace_segment: only implemented for IntPoints");
            return ForcedTraceEnd::From(from_corner);
        }
        self.start_marking_changed_area();
        // Check, if there ends an item of the same net at fromCorner. If so, its geometry will
        // be used to cut off dog ears of the check shape.
        let mut picked_trace: Option<ItemKey> = None;
        let filter = ItemSelectionFilter::single(SelectableChoices::Traces);
        let picked_items = self.pick_items(&from_corner, layer, Some(&filter));
        if picked_items.len() == 1 {
            let k = picked_items.first().unwrap();
            let t = self.item(k);
            if t.nets_equal_nos(net_numbers) && t.trace().half_width() == half_width && t.clearance_class() == clearance_class {
                picked_trace = Some(k);
            }
        }
        let compensated_half_width = half_width + self.clearance_compensation_value(DEFAULT_TREE, clearance_class, layer);
        // (the lines of the polyline of the caller are new objects)
        let input = TPolyline::fresh(polyline.clone());
        let Some(mut new_polyline) =
            TraceShover::spring_over_obstacles(self, &input, compensated_half_width, layer, net_numbers, clearance_class, None)
        else {
            return ForcedTraceEnd::From(from_corner);
        };
        let mut combined_polyline = match picked_trace {
            None => new_polyline.polyline.clone(),
            Some(t) => new_polyline.polyline.combine(Some(self.item(t).trace().polyline())),
        };
        if combined_polyline.lines.len() < 3 {
            return ForcedTraceEnd::From(from_corner);
        }
        // (Java int arithmetic: may be negative if the combination shortened the polyline)
        let start_shape_no = combined_polyline.lines.len() as i32 - new_polyline.len() as i32;
        // calculate the last shapes of combinedPolyline for checking
        let trace_shapes =
            combined_polyline.offset_shapes_range(compensated_half_width, start_shape_no, combined_polyline.lines.len() as i32 - 1);
        let trace_shape_count = trace_shapes.len() as i32;
        let mut last_shape_no = trace_shape_count;
        let orthogonal_mode = self.rules.get_trace_angle_restriction() == AngleRestriction::NinetyDegree;
        for (i, shape) in trace_shapes.iter().enumerate() {
            let i = i as i32;
            let current_trace_shape = if orthogonal_mode { TileShape::IntBox(shape.bounding_box()) } else { shape.clone() };
            let from_side = ShapeEntrySide::from_polyline(
                &combined_polyline,
                combined_polyline.corner_count() - trace_shape_count - 1 + i,
                &current_trace_shape,
            );
            if with_check
                && !TraceShover::check(
                    self,
                    &current_trace_shape,
                    Some(&from_side),
                    None,
                    layer,
                    net_numbers,
                    clearance_class,
                    max_recursion_depth,
                    max_via_recursion_depth,
                    max_spring_over_recursion_depth,
                    time_limit,
                )
            {
                last_shape_no = i;
                break;
            }
            let insert_ok = TraceShover::insert(
                self,
                &current_trace_shape,
                Some(&from_side),
                layer,
                net_numbers,
                clearance_class,
                None,
                max_recursion_depth,
                max_via_recursion_depth,
                max_spring_over_recursion_depth,
            );
            if !insert_ok {
                return ForcedTraceEnd::Failed;
            }
        }
        // newCorner starts as the toCorner object
        let mut new_corner = ForcedTraceEnd::To(to_corner.clone());
        if last_shape_no < trace_shape_count {
            // the shove with index lastShapeNo failed: sample the shove line to a shorter shove
            // distance and try again.
            let mut last_trace_shape = trace_shapes[last_shape_no as usize].clone();
            if orthogonal_mode {
                last_trace_shape = TileShape::IntBox(last_trace_shape.bounding_box());
            }
            let sample_width = 2 * self.min_trace_half_width();
            let last_corner = new_polyline.polyline.corner_approx(last_shape_no + 1);
            let prev_last_corner = new_polyline.polyline.corner_approx(last_shape_no);
            let last_segment_length = last_corner.distance(&prev_last_corner);
            if last_segment_length > 100.0 * sample_width as f64 {
                // too many cycles to sample
                return ForcedTraceEnd::From(from_corner);
            }
            let mut shape_index = combined_polyline.corner_count() - trace_shape_count - 1 + last_shape_no;
            if last_segment_length > sample_width as f64 {
                new_polyline = new_polyline.shorten(new_polyline.len() as i32 - (trace_shape_count - last_shape_no - 1), sample_width as f64);
                let current_last_corner = new_polyline.polyline.last_corner();
                if !current_last_corner.is_int_point() {
                    return ForcedTraceEnd::From(from_corner);
                }
                new_corner = ForcedTraceEnd::Other(current_last_corner);
                combined_polyline = match picked_trace {
                    None => new_polyline.polyline.clone(),
                    Some(t) => new_polyline.polyline.combine(Some(self.item(t).trace().polyline())),
                };
                if combined_polyline.lines.len() < 3 {
                    return new_corner;
                }
                shape_index = combined_polyline.lines.len() as i32 - 3;
                last_trace_shape = combined_polyline.offset_shape(compensated_half_width, shape_index).expect("offset shape");
                if orthogonal_mode {
                    last_trace_shape = TileShape::IntBox(last_trace_shape.bounding_box());
                }
            }
            let from_side = ShapeEntrySide::from_polyline(&combined_polyline, shape_index, &last_trace_shape);
            if !TraceShover::check(
                self,
                &last_trace_shape,
                Some(&from_side),
                None,
                layer,
                net_numbers,
                clearance_class,
                max_recursion_depth,
                max_via_recursion_depth,
                max_spring_over_recursion_depth,
                time_limit,
            ) {
                return ForcedTraceEnd::From(from_corner);
            }
            if !TraceShover::insert(
                self,
                &last_trace_shape,
                Some(&from_side),
                layer,
                net_numbers,
                clearance_class,
                None,
                max_recursion_depth,
                max_via_recursion_depth,
                max_spring_over_recursion_depth,
            ) {
                log::trace!("RoutingBoard.insert_forced_trace_polyline: shove trace failed");
                return ForcedTraceEnd::Failed;
            }
        }
        // insert the new trace segment
        for i in 0..new_polyline.polyline.corner_count() {
            let c = new_polyline.polyline.corner_approx(i);
            self.join_changed_area(&c, layer);
        }
        let new_trace = self.insert_trace_without_cleaning_tracked(
            new_polyline.clone(),
            layer,
            half_width,
            net_numbers,
            clearance_class,
            FixedState::Unfixed,
        );
        let Some(mut new_trace) = new_trace else {
            // Java: NullPointerException in newTrace.combine()
            panic!("RoutingBoard.insertForcedTracePolyline: new trace not inserted");
        };
        self.combine_trace(new_trace);
        let new_corner_point = new_corner.point().unwrap().clone();
        let tidy_region = if tidy_width < i32::MAX {
            Some(new_corner_point.surrounding_octagon().enlarge(tidy_width as f64))
        } else {
            None
        };
        let opt_nets: Vec<NetNo> = if max_recursion_depth <= 0 { net_numbers.to_vec() } else { Vec::new() };
        let mut pull_tight_algo = TraceTightener::get_instance(
            self,
            &opt_nets,
            tidy_region,
            pull_tight_accuracy,
            None,
            -1,
            Some(new_corner_point.clone()),
            layer,
        );
        let mut new_trace_opt = Some(new_trace);
        // Remove evtl. generated cycles because otherwise pullTight may not work correctly.
        let clip = self.basic.changed_area.as_ref().expect("changed area").get_area(layer);
        if self.normalize_trace(new_trace, Some(&clip)) {
            pull_tight_algo.split_traces_at_keep_point(self);
            // otherwise the new corner may no more be contained in the new trace after optimizing
            let item_filter = ItemSelectionFilter::single(SelectableChoices::Traces);
            let current_picked_items = self.pick_items(&new_corner_point, layer, Some(&item_filter));
            new_trace_opt = None;
            if let Some(found) = current_picked_items.first() {
                if self.item(found).is_trace() {
                    new_trace = found;
                    new_trace_opt = Some(new_trace);
                }
            }
        }
        // To avoid, that a separate handling for moving backwards in the own trace line becomes
        // necessary, pull tight is called here.
        if tidy_width > 0 {
            if let Some(t) = new_trace_opt {
                pull_tight_algo.pull_tight_trace(self, t);
            }
        }
        new_corner
    }

    // ------------------------------------------------------------------------------------------
    // autoroute entry points: `initAutoroute`, `autoroute`, `fanout`, `autorouteConnection` are
    // implemented in `crate::autoroute::router` (porting unit U8).

    /// Java `finishAutoroute()`: clears the autoroute database (`autorouteEngine.clear()`) and
    /// drops the engine.
    pub fn finish_autoroute(&mut self) {
        if let Some(mut engine) = self.autoroute_engine.take() {
            engine.clear(self);
        }
        self.basic.autoroute_maintenance = None;
    }

    /// pcbkit F2: a copy for a parallel worker. A board clone drops the autoroute engine but keeps the
    /// expansion rooms the engine left in the search trees (a retained database); a worker's fresh
    /// engine knows none of them and panics on the first neighbour room it finds there. The copy
    /// therefore holds no rooms (they are only a cache of the engine that owned them).
    pub fn clone_for_worker(&self) -> RoutingBoard {
        let mut b = self.clone();
        let n = b.search_trees().trees().len();
        for t in 0..n {
            for key in b.search_trees().trees()[t].room_keys() {
                b.search_trees_mut().tree_mut(t).remove_room(key);
            }
        }
        b
    }

    /// Java `clearTransientAutorouteState()`.
    pub fn clear_transient_autoroute_state(&mut self) {
        self.finish_autoroute();
        self.clear_all_item_temporary_autoroute_data();
        self.basic.changed_area = None;
        self.shove_failing_obstacle = None;
        self.shove_failing_layer = -1;
    }

    /// Java `clearAllItemTemporaryAutorouteData()`: the item autoroute infos are owned by the
    /// autoroute engine in the port (cleared there if the board holds an engine).
    pub fn clear_all_item_temporary_autoroute_data(&mut self) {
        if let Some(engine) = self.autoroute_engine.0.as_mut() {
            engine.clear_item_infos();
        }
    }

    /// Java `isMaintainingAutorouteDatabase()`.
    pub fn is_maintaining_autoroute_database(&self) -> bool {
        self.basic.autoroute_maintenance.as_ref().map(|m| m.maintain_database).unwrap_or(false)
    }

    // ------------------------------------------------------------------------------------------
    // misc RoutingBoard functions

    /// Java `connectToTrace(fromPoint, toTrace, penHalfWidth, clearanceClassIndex)`.
    pub fn connect_to_trace(&mut self, from_point: &IntPoint, to_trace: ItemKey, pen_half_width: i32, clearance_class: ClearanceClassNo) -> bool {
        let from = Point::Int(*from_point);
        let (trace_polyline, trace_layer, net_numbers) = {
            let t = self.item(to_trace);
            (t.trace().polyline().clone(), t.trace().layer(), t.net_numbers().to_vec())
        };
        if trace_polyline.contains(&from) {
            // no connection line necessary
            return true;
        }
        let Some(projection_line) = trace_polyline.projection_line(&from) else {
            return false;
        };
        let connection_line = projection_line.to_polyline();
        if connection_line.lines.len() != 3 {
            return false;
        }
        if !self.check_polyline_trace(&connection_line, trace_layer, pen_half_width, &net_numbers, clearance_class) {
            return false;
        }
        if self.basic.changed_area.is_some() {
            for i in 0..connection_line.corner_count() {
                let c = connection_line.corner_approx(i);
                self.join_changed_area(&c, trace_layer);
            }
        }
        self.insert_trace(connection_line, trace_layer, pen_half_width, &net_numbers, clearance_class, FixedState::Unfixed);
        let first_corner = self.item(to_trace).first_corner();
        let last_corner = self.item(to_trace).last_corner();
        if from != first_corner {
            if let Some(tail) = self.get_trace_tail(&first_corner, trace_layer, &net_numbers) {
                if !self.item(tail).is_user_fixed() {
                    self.remove_item(tail);
                }
            }
        }
        if from != last_corner {
            if let Some(tail) = self.get_trace_tail(&last_corner, trace_layer, &net_numbers) {
                if !self.item(tail).is_user_fixed() {
                    self.remove_item(tail);
                }
            }
        }
        true
    }

    /// Java `containsTraceTails(items, exceptNetNoArr)`.
    pub fn contains_trace_tails(&self, items: impl IntoIterator<Item = ItemKey>, except_net_no_arr: &[NetNo]) -> bool {
        for key in items {
            let item = self.item(key);
            if item.is_trace() && !item.nets_equal_nos(except_net_no_arr) && self.is_tail(key) {
                return true;
            }
        }
        false
    }

    /// Java `removeTraceTails(netNumber, stopConnectionOption)`: removes all trace tails of the
    /// net (all nets if `net_number <= 0`). Returns true if something was removed.
    pub fn remove_trace_tails(&mut self, net_number: NetNo, stop_connection_option: StopConnectionOption) -> bool {
        let mut stub_set = ItemSet::new();
        for key in self.get_items() {
            let item = self.item(key);
            if !item.is_routable() || item.net_count() != 1 {
                continue;
            }
            if net_number > 0 && item.net_number(0) != net_number {
                continue;
            }
            if self.is_tail(key) {
                if item.is_via() {
                    if stop_connection_option == StopConnectionOption::Via {
                        continue;
                    }
                    if stop_connection_option == StopConnectionOption::FanoutVia && self.is_fanout_via(key, None) {
                        continue;
                    }
                }
                stub_set.insert(item.id(), key);
            }
        }
        let mut stub_connections = ItemSet::new();
        for key in stub_set.iter() {
            let item_contact_count = self.normal_contacts(key).len();
            if item_contact_count == 1 {
                stub_connections.extend_from(&self.get_connection_items(key, stop_connection_option));
            } else {
                // the connected items are no stubs for example if a via is only connected on 1
                // layer, but to several traces.
                stub_connections.insert(self.item(key).id(), key);
            }
        }
        if stub_connections.is_empty() {
            return false;
        }
        self.remove_items(stub_connections.iter().collect::<Vec<_>>());
        self.combine_traces(net_number);
        true
    }

    /// Java `changePlaneAsObstacle(value)`: sets whether all conduction areas on signal layers
    /// are obstacles for foreign nets.
    pub fn change_plane_as_obstacle(&mut self, value: bool) {
        let target_ignore = !value;
        let mut something_changed = false;
        let mut cursor = self.item_cursor();
        while let Some(key) = self.cursor_next(&mut cursor) {
            let item = self.item(key);
            if let Some(c) = item.as_conduction_area() {
                let layer = c.area().layer();
                if self.layer_structure.layers[layer as usize].is_signal && c.is_obstacle() != value {
                    self.set_conduction_area_is_obstacle(key, value);
                    something_changed = true;
                }
            }
        }
        if self.rules.get_ignore_conduction() != target_ignore {
            self.rules_mut().set_ignore_conduction(target_ignore);
            something_changed = true;
        }
        if something_changed {
            self.reinsert_tree_items();
        }
    }

    /// Java `reduceNetsOfRouteItems()`: reduces the nets of traces and vias with more than one
    /// net at tie pins. Returns the Java result (always false, Java never sets it).
    pub fn reduce_nets_of_route_items(&mut self) -> bool {
        let result = false;
        let mut something_changed = true;
        while something_changed {
            something_changed = false;
            let mut cursor = self.item_cursor();
            while let Some(key) = self.cursor_next(&mut cursor) {
                let item = self.item(key);
                if item.net_numbers().len() <= 1 || item.fixed_state() == FixedState::SystemFixed {
                    continue;
                }
                if item.is_via() {
                    let contacts = self.normal_contacts(key);
                    let nets = item.net_numbers().to_vec();
                    for net in nets {
                        for c in contacts.iter() {
                            if !self.item(c).contains_net(net) {
                                self.remove_from_net(key, net);
                                something_changed = true;
                                break;
                            }
                        }
                        if something_changed {
                            break;
                        }
                    }
                } else if item.is_trace() {
                    let mut contacts = self.trace_start_contacts(key);
                    for i in 0..2 {
                        // Java iterates the (changing) net array by index
                        let mut n = 0;
                        while n < self.item(key).net_numbers().len() {
                            let net = self.item(key).net_numbers()[n];
                            n += 1;
                            let mut pin_found = false;
                            for c in contacts.iter() {
                                if self.item(c).is_pin() {
                                    pin_found = true;
                                    if !self.item(c).contains_net(net) {
                                        self.remove_from_net(key, net);
                                        something_changed = true;
                                        break;
                                    }
                                }
                            }
                            if !pin_found {
                                // at tie pins traces may have different nets
                                for c in contacts.iter() {
                                    if !self.item(c).is_pin() && !self.item(c).contains_net(net) {
                                        self.remove_from_net(key, net);
                                        something_changed = true;
                                        break;
                                    }
                                }
                            }
                        }
                        if something_changed {
                            break;
                        }
                        if i == 0 {
                            contacts = self.trace_end_contacts(key);
                        }
                    }
                    if something_changed {
                        break;
                    }
                }
                if something_changed {
                    break;
                }
            }
        }
        result
    }

    // ------------------------------------------------------------------------------------------
    // undo / copies (RoutingBoardUndoFacade)

    /// Java `deepCopy()` (serialization round trip): a copy with rebuilt search trees (a fresh
    /// default tree without clearance compensation, items inserted in list order) and the
    /// transient state reset (revision, `normalizeSuppressedNetNos`, changed area, autoroute
    /// database, shove failure). The failure log and the undo levels are copied.
    pub fn deep_copy(&self) -> RoutingBoard {
        let mut copy = self.clone();
        copy.basic.reset_transient_state_for_copy();
        copy.clear_all_item_temporary_autoroute_data();
        copy.finish_autoroute();
        copy.basic.changed_area = None;
        copy.shove_failing_obstacle = None;
        copy.shove_failing_layer = -1;
        copy
    }

}

impl BasicBoard {
    /// The transient state of a deserialized Java board (see [`RoutingBoard::deep_copy`]).
    pub(crate) fn reset_transient_state_for_copy(&mut self) {
        for key in self.items.keys() {
            self.items.get_mut(key).on_board = false;
        }
        self.search_trees = SearchTreeManager::default();
        self.normalize_suppressed_net_nos.clear();
        self.revision = 0;
        self.unfixable_clearance_violations_count = 0;
        for key in self.items.keys() {
            self.tree_insert(key);
        }
        // items outside the list are not serialized
        self.compact();
    }
}

/// Java `Math.max(double, double)`.
#[inline]
pub(crate) fn jmax(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a == 0.0 && b == 0.0 {
        if a.is_sign_negative() && b.is_sign_negative() {
            -0.0
        } else {
            0.0
        }
    } else if a > b {
        a
    } else {
        b
    }
}

/// Java `Math.min(double, double)`.
#[inline]
pub(crate) fn jmin(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a == 0.0 && b == 0.0 {
        if a.is_sign_negative() || b.is_sign_negative() {
            -0.0
        } else {
            0.0
        }
    } else if a < b {
        a
    } else {
        b
    }
}
