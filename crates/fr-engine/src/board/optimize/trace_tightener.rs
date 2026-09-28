//! Port of `board/optimize/TraceTightener.java` (the abstract base class and the factory) and of
//! `PolylineTrace.pullTight(TraceTightener)`, `PolylineTrace.pullTight(ownNetOnly, accuracy,
//! stoppable)` and `PolylineTrace.smoothenEndCornersFork`.
//!
//! The three Java subclasses are one struct with a [`TightenerKind`]; their methods are in
//! [`super::tightener45`], [`super::tightener90`] and [`super::tightener_any_angle`]. The board is
//! passed to every method (Java keeps a reference). Line object identity is tracked with
//! [`TPolyline`] so that `PolylineTrace.change` reuses exactly the search tree entries Java
//! reuses.

use fr_geom::{ConvexShape, FloatPoint, IntOctagon, Line, Point, Polyline, Side, Signum, TileShape};
use fr_settings::ExpansionCostFactor;

use crate::datastructures::{StopToken, TimeLimit};
use crate::ids::{AngleRestriction, ClearanceClassNo, LayerNo, NetNo};

use super::super::item::ItemKey;
use super::super::item_list::ItemSet;
use super::super::routing_board::RoutingBoard;
use super::super::search_tree::{TreeObject, DEFAULT_TREE};
use super::super::selection_filter::{ItemSelectionFilter, SelectableChoices};
use super::tracked::{LineIds, TLine, TPolyline};
use super::via_optimizer;

/// Java `c_max_cos_angle`: with angles too close to 180 degree the algorithm becomes
/// numerically unstable.
pub(crate) const C_MAX_COS_ANGLE: f64 = 0.999;
/// Java `c_min_corner_dist_square`.
pub(crate) const C_MIN_CORNER_DIST_SQUARE: f64 = 0.9;

/// The Java subclass of a [`TraceTightener`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TightenerKind {
    /// `TraceTightener90`.
    Ninety,
    /// `TraceTightener45`.
    FortyFive,
    /// `TraceTightenerAnyAngle`.
    AnyAngle,
}

/// Java `TraceTightener`: optimizes traces and vias.
#[derive(Clone, Debug)]
pub struct TraceTightener {
    pub kind: TightenerKind,
    /// If not empty, only traces with exactly these nets are optimized.
    pub only_net_no_arr: Vec<NetNo>,
    stop: Option<StopToken>,
    time_limit: Option<TimeLimit>,
    /// Traces containing the keep point must also contain it after optimizing.
    keep_point: Option<Point>,
    keep_point_layer: LayerNo,
    pub(crate) current_layer: LayerNo,
    pub(crate) current_half_width: i32,
    pub(crate) current_net_numbers: Vec<NetNo>,
    pub(crate) current_clearance_class: ClearanceClassNo,
    pub current_clip_shape: Option<IntOctagon>,
    pub(crate) contact_pins: Option<ItemSet>,
    pub min_translate_dist: i32,
    pub(crate) ids: LineIds,
}

impl TraceTightener {
    /// Java `TraceTightener.getInstance(board, onlyNetNoArr, clipShape, minTranslateDist,
    /// stoppableThread, timeLimit, keepPoint, keepPointLayer)`. A wall clock limit is created
    /// through [`RoutingBoard::time_limits`] if `time_limit_ms > 0`.
    #[allow(clippy::too_many_arguments)]
    pub fn get_instance(
        board: &RoutingBoard,
        only_net_no_arr: &[NetNo],
        clip_shape: Option<IntOctagon>,
        min_translate_dist: i32,
        stop: Option<StopToken>,
        time_limit_ms: i32,
        keep_point: Option<Point>,
        keep_point_layer: LayerNo,
    ) -> TraceTightener {
        let kind = match board.rules.get_trace_angle_restriction() {
            AngleRestriction::NinetyDegree => TightenerKind::Ninety,
            AngleRestriction::FortyfiveDegree => TightenerKind::FortyFive,
            AngleRestriction::None => TightenerKind::AnyAngle,
        };
        let time_limit = if time_limit_ms > 0 { Some(board.time_limits.make(time_limit_ms)) } else { None };
        TraceTightener {
            kind,
            only_net_no_arr: only_net_no_arr.to_vec(),
            stop,
            time_limit,
            keep_point,
            keep_point_layer,
            current_layer: 0,
            current_half_width: 0,
            current_net_numbers: Vec::new(),
            current_clearance_class: 0,
            current_clip_shape: clip_shape,
            contact_pins: None,
            min_translate_dist: min_translate_dist.max(100),
            ids: LineIds,
        }
    }

    /// Java `optChangedArea(traceCosts)`: optimizes the route in the marked area of the board.
    pub fn opt_changed_area(&mut self, board: &mut RoutingBoard, trace_costs: Option<&[ExpansionCostFactor]>) {
        if board.changed_area.is_none() {
            return;
        }
        let mut something_changed = true;
        while something_changed {
            something_changed = false;
            for i in 0..board.layer_count() {
                let Some(changed_area) = board.changed_area.as_mut() else { return };
                let changed_region = changed_area.get_area(i);
                if changed_region.is_empty() {
                    continue;
                }
                changed_area.set_empty(i);
                let changed_area_offset = 1.5
                    * (board.rules.clearance_matrix.max_value_on_layer(i) + 2 * board.rules.get_max_trace_half_width()) as f64;
                let changed_region = changed_region.enlarge(changed_area_offset);
                // search in the search tree for all overlapping traces with the region on layer i
                let items = board.overlapping_objects(&ConvexShape::Tile(TileShape::IntOctagon(changed_region)), i);
                for object in items {
                    if self.is_stop_requested() {
                        return;
                    }
                    let TreeObject::Item { key, .. } = object else { continue };
                    let item = board.item(key);
                    if item.is_trace() {
                        if self.pull_tight_trace(board, key) {
                            something_changed = true;
                            if self.split_traces_at_keep_point(board) {
                                break;
                            }
                        } else if self.smoothen_end_corners_at_trace(board, key) {
                            something_changed = true;
                            break; // because items may be removed
                        }
                    } else if item.is_via() {
                        if let Some(costs) = trace_costs {
                            if via_optimizer::opt_via_location(board, key, Some(costs), self.min_translate_dist, 10) {
                                something_changed = true;
                            }
                        }
                    }
                }
            }
        }
    }

    /// Java `pullTight(polyline, layer, halfWidth, netNumbers, clearanceClassIndex,
    /// contactPins)`: optimizes a single trace polyline. `contact_pins` are the pins at the end
    /// corners; other pins are obstacles, even of the own net.
    #[allow(clippy::too_many_arguments)]
    pub fn pull_tight_polyline(
        &mut self,
        board: &mut RoutingBoard,
        polyline: &TPolyline,
        layer: LayerNo,
        half_width: i32,
        net_numbers: &[NetNo],
        clearance_class: ClearanceClassNo,
        contact_pins: Option<ItemSet>,
    ) -> TPolyline {
        self.current_layer = layer;
        self.current_half_width = half_width + board.clearance_compensation_value(DEFAULT_TREE, clearance_class, layer);
        self.current_net_numbers = net_numbers.to_vec();
        self.current_clearance_class = clearance_class;
        self.contact_pins = contact_pins;
        match self.kind {
            TightenerKind::Ninety => self.pull_tight_90(board, polyline),
            TightenerKind::FortyFive => self.pull_tight_45(board, polyline),
            TightenerKind::AnyAngle => self.pull_tight_any_angle(board, polyline),
        }
    }

    /// Java `PolylineTrace.pullTight(TraceTightener)`: tries to shorten the trace without creating
    /// clearance violations. Returns true if the trace was changed.
    pub fn pull_tight_trace(&mut self, board: &mut RoutingBoard, trace: ItemKey) -> bool {
        let item = board.item(trace);
        if !item.is_on_board() {
            // This trace may have been deleted in a trace split for example
            return false;
        }
        if board.is_shove_fixed(trace) {
            return false;
        }
        if !item.nets_normal() {
            return false;
        }
        if !self.only_net_no_arr.is_empty() && !item.nets_equal_nos(&self.only_net_no_arr) {
            return false;
        }
        if let Some(&n) = item.net_numbers().first() {
            if !board.rules.net_class_of(n).get_pull_tight() {
                return false;
            }
        }
        let t = item.trace();
        let (layer, half_width, nets, cl) = (t.layer(), t.half_width(), item.net_numbers().to_vec(), item.clearance_class());
        let old = t.tpolyline();
        let contact_pins = board.touching_pins_at_end_corners(trace);
        let new_lines = self.pull_tight_polyline(board, &old, layer, half_width, &nets, cl, Some(contact_pins));
        if !new_lines.same(&old) {
            board.change_trace_tracked(trace, new_lines);
            return true;
        }
        let angle_restriction = board.rules.get_trace_angle_restriction();
        if angle_restriction != AngleRestriction::NinetyDegree && board.rules.get_pin_edge_to_turn_dist() > 0.0 {
            if board.swap_connection_to_pin(trace, true) {
                self.pull_tight_trace(board, trace);
                return true;
            }
            if board.swap_connection_to_pin(trace, false) {
                self.pull_tight_trace(board, trace);
                return true;
            }
            // optimize algorithm could not improve the trace, try to remove acid traps
            if board.correct_connection_to_pin(trace, true, angle_restriction) {
                self.pull_tight_trace(board, trace);
                return true;
            }
            if board.correct_connection_to_pin(trace, false, angle_restriction) {
                self.pull_tight_trace(board, trace);
                return true;
            }
        }
        false
    }

    /// Java `isStopRequested()`.
    pub(crate) fn is_stop_requested(&self) -> bool {
        if let Some(s) = &self.stop {
            if s.is_stop_requested() {
                return true;
            }
        }
        match &self.time_limit {
            None => false,
            Some(t) => {
                let exceeded = t.limit_exceeded();
                if exceeded {
                    log::debug!("TraceTightener.is_stop_requested: time limit exceeded");
                }
                exceeded
            }
        }
    }

    /// `board.checkTraceShape(shape, currentLayer, currentNetNumbers, currentClearanceClassIndex,
    /// contactPins)`.
    #[inline]
    pub(crate) fn check_shape(&self, board: &RoutingBoard, shape: &TileShape) -> bool {
        board.check_trace_shape(shape, self.current_layer, &self.current_net_numbers, self.current_clearance_class, self.contact_pins.as_ref())
    }

    /// `polyline.offsetShape(currentHalfWidth, no)` checked with [`Self::check_shape`].
    #[inline]
    pub(crate) fn check_offset_shape(&self, board: &RoutingBoard, polyline: &Polyline, no: i32) -> bool {
        let shape = polyline.offset_shape(self.current_half_width, no).expect("TraceTightener: offset shape is null");
        self.check_shape(board, &shape)
    }

    /// `board.changedArea.join(point, currentLayer)` if the changed area exists.
    #[inline]
    pub(crate) fn join(&self, board: &mut RoutingBoard, point: &FloatPoint) {
        if let Some(c) = board.changed_area.as_mut() {
            c.join(point, self.current_layer);
        }
    }

    /// The clip shape tests of Java (`currentClipShape.isOutside(Point)`).
    #[inline]
    pub(crate) fn clip_is_outside(&self, point: &Point) -> bool {
        match &self.current_clip_shape {
            None => false,
            Some(c) => TileShape::IntOctagon(*c).is_outside(point),
        }
    }

    /// `currentClipShape.contains(FloatPoint)` (true without clip shape).
    #[inline]
    pub(crate) fn clip_contains(&self, point: &FloatPoint) -> bool {
        match &self.current_clip_shape {
            None => true,
            Some(c) => c.contains_float(point),
        }
    }

    /// Java `repositionLines(polyline)` (overridden by the any angle tightener).
    pub(crate) fn reposition_lines(&mut self, board: &mut RoutingBoard, polyline: &TPolyline) -> TPolyline {
        if self.kind == TightenerKind::AnyAngle {
            return self.reposition_lines_any_angle(board, polyline);
        }
        if polyline.len() < 5 {
            return polyline.clone();
        }
        let lines = polyline.tlines();
        for i in 2..polyline.len() - 2 {
            if let Some(new_line) = self.reposition_line(board, &lines, i as i32) {
                let mut new_lines = lines.clone();
                new_lines[i] = new_line;
                let result = TPolyline::from_lines(&mut new_lines);
                return self.skip_segments_of_length0(board, &result);
            }
        }
        polyline.clone()
    }

    /// Java `repositionLine(lines, no)` (virtual: overridden by the any angle tightener).
    pub(crate) fn reposition_line(&mut self, board: &mut RoutingBoard, lines: &[TLine], no: i32) -> Option<TLine> {
        if self.kind == TightenerKind::AnyAngle {
            return self.reposition_line_any_angle(board, lines, no);
        }
        self.reposition_line_base(board, lines, no)
    }

    /// Java `TraceTightener.repositionLine(lines, no)`: tries to reposition the line with index
    /// `no` to make the polyline shorter.
    fn reposition_line_base(&mut self, board: &mut RoutingBoard, lines: &[TLine], no: i32) -> Option<TLine> {
        if lines.len() as i32 - no < 3 {
            return None;
        }
        let l = |k: i32| &lines[k as usize].line;
        if self.current_clip_shape.is_some() {
            // check, that the corners of the line to translate are inside the clip shape
            for i in -1..1 {
                let current_corner = l(no + i).intersection(l(no + i + 1));
                if self.clip_is_outside(&current_corner) {
                    return None;
                }
            }
        }
        let translate_line = l(no).clone();
        let prev_corner = l(no - 2).intersection(l(no - 1));
        let next_corner = l(no + 1).intersection(l(no + 2));
        let prev_dist = translate_line.signed_distance(&prev_corner.to_float());
        let next_dist = translate_line.signed_distance(&next_corner.to_float());
        if Signum::of(prev_dist) != Signum::of(next_dist) {
            // the 2 corners are at different sides of translate_line
            return None;
        }
        let (nearest_point, mut max_translate_dist) =
            if prev_dist.abs() < next_dist.abs() { (prev_corner, prev_dist) } else { (next_corner, next_dist) };
        let _ = max_translate_dist;
        let mut translate_dist = max_translate_dist;
        let mut delta_dist = max_translate_dist;
        let side_of_nearest_point = translate_line.side_of(&nearest_point);
        let sign = Signum::as_int(max_translate_dist);
        let mut new_line: Option<TLine> = None;
        let mut check_lines: Vec<TLine> = vec![lines[(no - 1) as usize].clone(), lines[no as usize].clone(), lines[(no + 1) as usize].clone()];
        let mut first_time = true;
        while first_time || delta_dist.abs() > self.min_translate_dist as f64 {
            let candidate = if first_time && nearest_point.is_int_point() {
                Line::get_instance(nearest_point.clone(), &translate_line.direction())
            } else {
                translate_line.translate(-translate_dist)
            };
            check_lines[1] = self.ids.line(candidate);
            if check_lines[1].line == translate_line {
                // may happen at first time if nearest_point is not an IntPoint
                return None;
            }
            let new_line_side_of_nearest_point = check_lines[1].line.side_of(&nearest_point);
            if new_line_side_of_nearest_point != side_of_nearest_point && new_line_side_of_nearest_point != Side::Collinear {
                // moved a little bit to far at the first time because of numerical inaccuracy;
                // may happen if nearest_point is not an IntPoint
                let shorten_value = sign as f64 * 0.5;
                max_translate_dist -= shorten_value;
                translate_dist -= shorten_value;
                delta_dist -= shorten_value;
                continue;
            }
            let tmp = TPolyline::from_lines(&mut check_lines);
            let mut check_ok = false;
            if tmp.len() == 3 {
                check_ok = self.check_offset_shape(board, &tmp.polyline, 0);
            }
            delta_dist /= 2.0;
            if check_ok {
                new_line = Some(check_lines[1].clone());
                if first_time {
                    // biggest possible change
                    break;
                }
                translate_dist += delta_dist;
            } else {
                translate_dist -= delta_dist;
            }
            first_time = false;
        }
        if let Some(nl) = &new_line {
            if board.changed_area.is_some() {
                // mark the changed area
                let p1 = check_lines[0].line.intersection_approx(&nl.line);
                let p2 = check_lines[2].line.intersection_approx(&nl.line);
                let p3 = l(no - 1).intersection_approx(l(no));
                let p4 = l(no).intersection_approx(l(no + 1));
                self.join(board, &p1);
                self.join(board, &p2);
                self.join(board, &p3);
                self.join(board, &p4);
            }
        }
        new_line
    }

    /// Java `skipSegmentsOfLength0(polyline)`: tries to skip line segments of length 0 (a check
    /// is necessary because new dog ears may occur).
    pub(crate) fn skip_segments_of_length0(&mut self, board: &mut RoutingBoard, polyline: &TPolyline) -> TPolyline {
        let mut polyline_changed = false;
        let mut current = polyline.clone();
        let mut i: i64 = 1;
        while i < current.len() as i64 - 1 {
            let iu = i as i32;
            let try_skip = if i == 1 || i == current.len() as i64 - 2 {
                // the position of the first corner and the last corner must be retained exactly
                current.polyline.corner(iu) == current.polyline.corner(iu - 1)
            } else {
                let prev_corner = current.polyline.corner_approx(iu - 1);
                let current_corner = current.polyline.corner_approx(iu);
                current_corner.distance_square(&prev_corner) < C_MIN_CORNER_DIST_SQUARE
            };
            if try_skip {
                // check, if skipping the line of length 0 does not result in a clearance violation
                let mut current_lines: Vec<TLine> = Vec::with_capacity(current.len() - 1);
                for k in 0..current.len() {
                    if k as i64 != i {
                        current_lines.push(current.tline(k));
                    }
                }
                let expected_len = current_lines.len();
                let tmp = TPolyline::from_lines(&mut current_lines);
                let mut check_ok = tmp.len() == expected_len;
                if check_ok && !current.polyline.lines[i as usize].is_multiple_of_45_degree() {
                    // no check necessary for skipping 45 degree lines, because the check is
                    // performance critical and the line shapes are intersected with the
                    // bounding octagon anyway.
                    if i > 1 {
                        check_ok = self.check_offset_shape(board, &tmp.polyline, iu - 2);
                    }
                    if check_ok && i < current.len() as i64 - 2 {
                        check_ok = self.check_offset_shape(board, &tmp.polyline, iu - 1);
                    }
                }
                if check_ok {
                    polyline_changed = true;
                    current = tmp;
                    i -= 1;
                }
            }
            i += 1;
        }
        if !polyline_changed {
            return polyline.clone();
        }
        current
    }

    /// Java `smoothenEndCornersAtTrace(trace)`: smoothens acute angles with contact traces.
    /// Returns true if something was changed.
    pub fn smoothen_end_corners_at_trace(&mut self, board: &mut RoutingBoard, trace: ItemKey) -> bool {
        let item = board.item(trace);
        if !self.only_net_no_arr.is_empty() && !item.nets_equal_nos(&self.only_net_no_arr) {
            return false;
        }
        let t = item.trace();
        self.current_layer = t.layer();
        self.current_half_width = t.half_width();
        self.current_net_numbers = item.net_numbers().to_vec();
        self.current_clearance_class = item.clearance_class();
        self.smoothen_end_corners_at_trace1(board, trace)
    }

    fn smoothen_end_corners_at_trace1(&mut self, board: &mut RoutingBoard, trace: ItemKey) -> bool {
        // try to improve the connection to other traces
        if board.is_shove_fixed(trace) {
            return false;
        }
        // to allow the trace to slide to the end point of a contact trace, if the contact trace
        // ends at a pin.
        let saved_contact_pins = self.contact_pins.take();
        let mut result = false;
        let mut connection_to_trace_improved = true;
        let mut current_trace = trace;
        while connection_to_trace_improved {
            connection_to_trace_improved = false;
            let adjusted = self.smoothen_end_corners_at_trace2(board, current_trace);
            if let Some(adjusted_polyline) = adjusted {
                let (trace_layer, current_cl_class, current_fixed_state, nets) = {
                    let it = board.item(current_trace);
                    (it.trace().layer(), it.clearance_class(), it.fixed_state(), it.net_numbers().to_vec())
                };
                board.remove_item(current_trace);
                let adj_ins_trace = board.insert_trace_without_cleaning_tracked(
                    adjusted_polyline.clone(),
                    trace_layer,
                    self.current_half_width,
                    &nets,
                    current_cl_class,
                    current_fixed_state,
                );
                if let Some(adj) = adj_ins_trace {
                    result = true;
                    connection_to_trace_improved = true;
                    board.remove_item(current_trace);
                    current_trace = adj;
                    let current_nets = board.item(current_trace).net_numbers().to_vec();
                    for net in current_nets {
                        board.split_traces(&adjusted_polyline.polyline.first_corner(), trace_layer, net);
                        board.split_traces(&adjusted_polyline.polyline.last_corner(), trace_layer, net);
                        board.normalize_traces(net);
                        if self.split_traces_at_keep_point(board) {
                            return true;
                        }
                    }
                }
            }
        }
        self.contact_pins = saved_contact_pins;
        result
    }

    /// Java `splitTracesAtKeepPoint()`: splits the traces containing the keep point. Returns
    /// true if something was split.
    pub fn split_traces_at_keep_point(&mut self, board: &mut RoutingBoard) -> bool {
        let Some(keep_point) = self.keep_point.clone() else {
            return false;
        };
        let filter = ItemSelectionFilter::single(SelectableChoices::Traces);
        let picked = board.pick_items(&keep_point, self.keep_point_layer, Some(&filter));
        for key in picked.iter() {
            if board.split_trace_at_point(key, &keep_point).is_some() {
                return true;
            }
        }
        false
    }

    /// Java `smoothenEndCornersAtTrace2(trace)`: the adjusted polyline, or `None` if nothing can
    /// be changed.
    fn smoothen_end_corners_at_trace2(&mut self, board: &mut RoutingBoard, trace: ItemKey) -> Option<TPolyline> {
        if !board.item(trace).is_on_board() {
            return None;
        }
        let mut result = self.smoothen_start_corner_at_trace(board, trace);
        match &result {
            None => {
                result = self.smoothen_end_corner_at_trace(board, trace);
                if let Some(r) = &result {
                    if board.changed_area.is_some() {
                        // mark the changed area
                        let p = r.polyline.corner_approx(r.polyline.corner_count() - 1);
                        self.join(board, &p);
                    }
                }
            }
            Some(r) => {
                if board.changed_area.is_some() {
                    let p = r.polyline.corner_approx(0);
                    self.join(board, &p);
                }
            }
        }
        if let Some(r) = result {
            self.contact_pins = Some(board.touching_pins_at_end_corners(trace));
            return Some(self.skip_segments_of_length0(board, &r));
        }
        None
    }

    fn smoothen_start_corner_at_trace(&mut self, board: &mut RoutingBoard, trace: ItemKey) -> Option<TPolyline> {
        match self.kind {
            TightenerKind::Ninety => None,
            TightenerKind::FortyFive => self.smoothen_start_corner_at_trace_45(board, trace),
            TightenerKind::AnyAngle => self.smoothen_start_corner_at_trace_any_angle(board, trace),
        }
    }

    fn smoothen_end_corner_at_trace(&mut self, board: &mut RoutingBoard, trace: ItemKey) -> Option<TPolyline> {
        match self.kind {
            TightenerKind::Ninety => None,
            TightenerKind::FortyFive => self.smoothen_end_corner_at_trace_45(board, trace),
            TightenerKind::AnyAngle => self.smoothen_end_corner_at_trace_any_angle(board, trace),
        }
    }

    /// Java `avoidAcidTraps(polyline)` (disabled in Java by `if (true) return polyline;`).
    #[inline]
    pub(crate) fn avoid_acid_traps(&self, polyline: &TPolyline) -> TPolyline {
        polyline.clone()
    }

    /// The contact trace data used by `smoothenStartCornerAtTrace` / `smoothenEndCornerAtTrace`
    /// of the 45 degree and any angle tighteners: for a contact trace ending at `end_corner`,
    /// `(otherTraceCornerApprox, otherTraceLine, otherPrevTraceLine)`.
    pub(crate) fn contact_trace_lines(&mut self, contact: &TPolyline, end_corner: &Point) -> (FloatPoint, TLine, TLine) {
        let contact_polyline = &contact.polyline;
        if contact_polyline.first_corner() == *end_corner {
            (contact_polyline.corner_approx(1), contact.tline(1), contact.tline(2))
        } else {
            let current_corner_no = contact_polyline.corner_count() - 2;
            (
                contact_polyline.corner_approx(current_corner_no),
                contact.tline((current_corner_no + 1) as usize).opposite(),
                contact.tline(current_corner_no as usize),
            )
        }
    }
}

impl RoutingBoard {
    /// Java `PolylineTrace.pullTight(ownNetOnly, pullTightAccuracy, stoppableThread)`: pulls the
    /// trace tight without creating clearance violations. Returns true if it was changed.
    pub fn pull_tight_trace(&mut self, trace: ItemKey, own_net_only: bool, pull_tight_accuracy: i32, stop: Option<&StopToken>) -> bool {
        let nets: Vec<NetNo> = if own_net_only { self.item(trace).net_numbers().to_vec() } else { Vec::new() };
        let mut algo = TraceTightener::get_instance(self, &nets, None, pull_tight_accuracy, stop.cloned(), -1, None, -1);
        algo.pull_tight_trace(self, trace)
    }

    /// Java `PolylineTrace.smoothenEndCornersFork(ownNetOnly, pullTightAccuracy,
    /// stoppableThread)`: smoothens the end corners of the trace at forks with other traces.
    pub fn smoothen_end_corners_fork(&mut self, trace: ItemKey, own_net_only: bool, pull_tight_accuracy: i32, stop: Option<&StopToken>) -> bool {
        let nets: Vec<NetNo> = if own_net_only { self.item(trace).net_numbers().to_vec() } else { Vec::new() };
        let mut algo = TraceTightener::get_instance(self, &nets, None, pull_tight_accuracy, stop.cloned(), -1, None, -1);
        algo.smoothen_end_corners_at_trace(self, trace)
    }
}
