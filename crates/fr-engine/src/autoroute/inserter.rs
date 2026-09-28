//! Port of `autoroute/path/FoundConnectionInserter.java`: inserts the traces and vias of the
//! connection found by the autorouter.

use fr_geom::{IntPoint, Point, Polyline};

use crate::board::actions::forced_via_inserter;
use crate::board::{ForcedTraceEnd, ItemKey, ItemSelectionFilter, RoutingBoard, SelectableChoices};
use crate::ids::{LayerNo, NetNo};

use super::control::AutorouteControl;
use super::locator::{calculate_additional_corner, FoundConnectionLocator, ResultItem};

/// Java `FoundConnectionInserter`.
#[derive(Clone, Debug, Default)]
pub struct FoundConnectionInserter {
    pub last_corner: Option<IntPoint>,
    pub first_corner: Option<IntPoint>,
}

/// Result of `tryNeckDown` (the Java callers compare the returned point by identity).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NeckDownResult {
    Null,
    /// The `fromCorner` argument object.
    From,
    /// The `toCorner` argument object.
    To,
    Other,
}

impl FoundConnectionInserter {
    /// Java `FoundConnectionInserter.getInstance(connection, board, ctrl)`: inserts the connection.
    /// `None` if the insertion did not succeed.
    pub fn get_instance(connection: &FoundConnectionLocator, board: &mut RoutingBoard, ctrl: &AutorouteControl) -> Option<FoundConnectionInserter> {
        let mut current_layer = connection.target_layer;
        let mut new_instance = FoundConnectionInserter::default();
        for current_new_item in &connection.connection_items {
            let first = current_new_item.corners.first().map(|p| Point::Int(*p));
            if !new_instance.insert_via(board, ctrl, first.as_ref(), current_layer, current_new_item.layer) {
                return None;
            }
            current_layer = current_new_item.layer;
            if !new_instance.insert_trace(board, ctrl, current_new_item) {
                return None;
            }
        }
        let last = new_instance.last_corner.map(Point::Int);
        if !new_instance.insert_via(board, ctrl, last.as_ref(), current_layer, connection.start_layer) {
            return None;
        }
        if let Some(target) = connection.target_item {
            if board.item(target).is_trace() {
                match new_instance.first_corner {
                    Some(fc) => {
                        board.connect_to_trace(&fc, target, ctrl.trace_half_width[connection.start_layer as usize], ctrl.trace_clearance_class);
                    }
                    None => log::warn!("FoundConnectionInserter: firstCorner is null for net #{}, skipping connect_to_trace for target item.", ctrl.net_number),
                }
            }
        }
        if let Some(start) = connection.start_item {
            if board.item(start).is_trace() {
                match new_instance.last_corner {
                    Some(lc) => {
                        board.connect_to_trace(&lc, start, ctrl.trace_half_width[connection.target_layer as usize], ctrl.trace_clearance_class);
                    }
                    None => log::warn!("FoundConnectionInserter: lastCorner is null for net #{}, skipping connect_to_trace for start item.", ctrl.net_number),
                }
            }
        }
        board.normalize_traces(ctrl.net_number);
        Some(new_instance)
    }

    /// Java `insertTrace(trace)`: inserts the trace by shoving aside obstacle traces and vias.
    /// Returns false if that was not possible for the whole trace.
    fn insert_trace(&mut self, board: &mut RoutingBoard, ctrl: &AutorouteControl, trace: &ResultItem) -> bool {
        if trace.corners.len() == 1 {
            // Single-point trace: the start and end are the same location.
            if self.first_corner.is_none() {
                self.first_corner = Some(trace.corners[0]);
            }
            self.last_corner = Some(trace.corners[0]);
            return true;
        }
        // switch off correcting connection to pin because it may get wrong in inserting the
        // polygon line for line.
        let saved_edge_to_turn_dist = board.rules.get_pin_edge_to_turn_dist();
        board.rules_mut().set_pin_edge_to_turn_dist(-1.0);
        // Look for pins at the start and the end of trace in case that neckdown is necessary.
        let mut start_pin: Option<ItemKey> = None;
        let mut end_pin: Option<ItemKey> = None;
        if ctrl.with_neckdown {
            let item_filter = ItemSelectionFilter::single(SelectableChoices::Pins);
            let mut current_end_corner = Point::Int(trace.corners[0]);
            for i in 0..2 {
                let picked_items = board.pick_items(&current_end_corner, trace.layer, Some(&item_filter));
                for current_pin in picked_items.iter() {
                    let p = board.item(current_pin);
                    if p.contains_net(ctrl.net_number) && p.center(board) == current_end_corner {
                        if i == 0 {
                            start_pin = Some(current_pin);
                        } else {
                            end_pin = Some(current_pin);
                        }
                    }
                }
                current_end_corner = Point::Int(*trace.corners.last().unwrap());
            }
        }
        let net_numbers = [ctrl.net_number];
        let lu = trace.layer as usize;
        let mut from_corner_no = 0usize;
        let mut result = true;
        for i in 1..trace.corners.len() {
            let current_corner_arr: Vec<Point> = trace.corners[from_corner_no..=i].iter().map(|p| Point::Int(*p)).collect();
            let insert_polyline = Polyline::from_points(&current_corner_arr);
            let ok_point = board.insert_forced_trace_polyline(
                &insert_polyline,
                ctrl.trace_half_width[lu],
                trace.layer,
                &net_numbers,
                ctrl.trace_clearance_class,
                ctrl.max_shove_trace_recursion_depth,
                ctrl.max_shove_via_recursion_depth,
                ctrl.max_spring_over_recursion_depth,
                i32::MAX,
                ctrl.pull_tight_accuracy,
                true,
                None,
            );
            let mut neckdown_inserted = false;
            let mut micro_neckdown_inserted = false;
            let ok_is_last = ok_point.is_to();
            if ok_point != ForcedTraceEnd::Failed && !ok_is_last && ctrl.with_neckdown && current_corner_arr.len() == 2 {
                neckdown_inserted =
                    self.insert_neckdown(board, ctrl, ok_point.point().unwrap(), &current_corner_arr[1], trace.layer, start_pin, end_pin);
            }
            if !neckdown_inserted && !ok_is_last && (ctrl.is_fanout || ctrl.with_neckdown) && current_corner_arr.len() == 2 {
                micro_neckdown_inserted =
                    self.insert_fanout_micro_neckdown(board, ctrl, ok_point.point(), &current_corner_arr[1], trace.layer, &net_numbers, start_pin, end_pin);
            }
            if ok_is_last || neckdown_inserted || micro_neckdown_inserted {
                from_corner_no = i;
            } else if matches!(ok_point, ForcedTraceEnd::From(_)) && i != trace.corners.len() - 1 {
                // if okPoint == insertPolyline.firstCorner() the spring over may have failed.
                // Repeating the insertion with more distant corners may allow the spring over to
                // correct the situation.
                if from_corner_no > 0 && current_corner_arr.len() < 3 {
                    // first correction
                    from_corner_no -= 1;
                }
                log::trace!("FoundConnectionInserter: violation corrected");
            } else {
                log::debug!(
                    "FoundConnectionInserter: insert trace failed for net #{} at corner {}/{} on layer {}",
                    ctrl.net_number,
                    i,
                    trace.corners.len() - 1,
                    trace.layer
                );
                result = false;
                break;
            }
        }
        for i in 0..trace.corners.len() - 1 {
            if let Some(trace_stub) = board.get_trace_tail(&Point::Int(trace.corners[i]), trace.layer, &net_numbers) {
                board.remove_item(trace_stub);
            }
        }
        board.rules_mut().set_pin_edge_to_turn_dist(saved_edge_to_turn_dist);
        if self.first_corner.is_none() {
            self.first_corner = Some(trace.corners[0]);
        }
        self.last_corner = Some(*trace.corners.last().unwrap());
        result
    }

    /// Java `insertFanoutMicroNeckdown(okPoint, targetPoint, layer, netNumbers, startPin, endPin)`.
    #[allow(clippy::too_many_arguments)]
    fn insert_fanout_micro_neckdown(
        &mut self,
        board: &mut RoutingBoard,
        ctrl: &AutorouteControl,
        ok_point: Option<&Point>,
        target_point: &Point,
        layer: LayerNo,
        net_numbers: &[NetNo],
        start_pin: Option<ItemKey>,
        end_pin: Option<ItemKey>,
    ) -> bool {
        let from_point = ok_point.unwrap_or(target_point);
        if from_point == target_point {
            return false;
        }
        let base_half_width = ctrl.trace_half_width[layer as usize];
        // LinkedHashSet<Integer>: insertion order without duplicates
        let mut candidate_half_widths: Vec<i32> = Vec::new();
        let add = |v: i32, list: &mut Vec<i32>| {
            if !list.contains(&v) {
                list.push(v);
            }
        };
        if let Some(p) = start_pin {
            if board.item(p).is_on_layer(board, layer) {
                add(board.pin_trace_neckdown_half_width(p, layer), &mut candidate_half_widths);
            }
        }
        if let Some(p) = end_pin {
            if board.item(p).is_on_layer(board, layer) {
                add(board.pin_trace_neckdown_half_width(p, layer), &mut candidate_half_widths);
            }
        }
        add(1.max(base_half_width.wrapping_mul(3) / 4), &mut candidate_half_widths);
        add(1.max(base_half_width.wrapping_mul(3) / 5), &mut candidate_half_widths);
        add(1.max(base_half_width / 2), &mut candidate_half_widths);
        let from_point = from_point.clone();
        for candidate_half_width in candidate_half_widths {
            if candidate_half_width <= 0 || candidate_half_width >= base_half_width {
                continue;
            }
            // fastroute extension: never below the board minimum track width
            let candidate_half_width = ctrl.clamp_neckdown_half_width(candidate_half_width);
            if candidate_half_width >= base_half_width {
                continue;
            }
            let candidate_ok_point = board.insert_forced_trace_segment(
                &from_point,
                target_point,
                candidate_half_width,
                layer,
                net_numbers,
                ctrl.trace_clearance_class,
                ctrl.max_shove_trace_recursion_depth,
                ctrl.max_shove_via_recursion_depth,
                ctrl.max_spring_over_recursion_depth,
                i32::MAX,
                ctrl.pull_tight_accuracy,
                true,
                None,
            );
            if candidate_ok_point.is_to() {
                return true;
            }
        }
        false
    }

    /// Java `insertNeckdown(fromCorner, toCorner, layer, startPin, endPin)`.
    #[allow(clippy::too_many_arguments)]
    fn insert_neckdown(
        &mut self,
        board: &mut RoutingBoard,
        ctrl: &AutorouteControl,
        from_corner: &Point,
        to_corner: &Point,
        layer: LayerNo,
        start_pin: Option<ItemKey>,
        end_pin: Option<ItemKey>,
    ) -> bool {
        if let Some(p) = start_pin {
            // okPoint == fromCorner <=> tryNeckDown returned its toCorner argument
            let ok_point = self.try_neck_down(board, ctrl, to_corner, from_corner, layer, p);
            if ok_point == NeckDownResult::To {
                return true;
            }
        }
        if let Some(p) = end_pin {
            let ok_point = self.try_neck_down(board, ctrl, from_corner, to_corner, layer, p);
            return ok_point == NeckDownResult::To;
        }
        false
    }

    /// Java `tryNeckDown(fromCorner, toCorner, layer, pin, atStart)`.
    fn try_neck_down(&mut self, board: &mut RoutingBoard, ctrl: &AutorouteControl, from_corner: &Point, to_corner: &Point, layer: LayerNo, pin: ItemKey) -> NeckDownResult {
        if !board.item(pin).is_on_layer(board, layer) {
            return NeckDownResult::Null;
        }
        let lu = layer as usize;
        let pin_center = board.item(pin).center(board).to_float();
        let current_clearance = board.rules.clearance_matrix.get_value(ctrl.trace_clearance_class, board.item(pin).clearance_class(), layer, true) as f64;
        let pin_neck_down_distance = 2.0 * (0.5 * board.pin_max_width(pin, layer) + current_clearance);
        if pin_center.distance(&to_corner.to_float()) >= pin_neck_down_distance {
            return NeckDownResult::Null;
        }
        // fastroute extension: never below the board minimum track width
        let neck_down_halfwidth = ctrl.clamp_neckdown_half_width(board.pin_trace_neckdown_half_width(pin, layer));
        if neck_down_halfwidth >= ctrl.trace_half_width[lu] {
            return NeckDownResult::Null;
        }
        let float_from_corner = from_corner.to_float();
        let float_to_corner = to_corner.to_float();
        const TOLERANCE: i32 = 2;
        let net_numbers = [ctrl.net_number];
        let mut ok_length = board.check_trace_segment(from_corner, to_corner, layer, &net_numbers, ctrl.trace_half_width[lu], ctrl.trace_clearance_class, true);
        if ok_length >= i32::MAX as f64 {
            return NeckDownResult::From;
        }
        ok_length -= TOLERANCE as f64;
        // (point, identity of neckDownEndPoint: true = the fromCorner object)
        let mut neck_down_end_point: Point;
        let mut end_is_from = false;
        let angle = board.rules.get_trace_angle_restriction();
        if ok_length <= TOLERANCE as f64 {
            neck_down_end_point = from_corner.clone();
            end_is_from = true;
        } else {
            let float_neck_down_end_point = float_from_corner.change_length(&float_to_corner, ok_length);
            neck_down_end_point = Point::Int(float_neck_down_end_point.round());
            // add a corner in case neckDownEndPoint is not exactly on the line from fromCorner to
            // toCorner
            let horizontal_first = (float_from_corner.x - float_neck_down_end_point.x).abs() >= (float_from_corner.y - float_neck_down_end_point.y).abs();
            let add_corner = Point::Int(calculate_additional_corner(&float_from_corner, &float_neck_down_end_point, horizontal_first, angle).round());
            let current_ok_point = self.forced_segment(board, ctrl, from_corner, &add_corner, ctrl.trace_half_width[lu], layer, &net_numbers);
            if !current_ok_point.is_to() {
                return NeckDownResult::From;
            }
            let current_ok_point = self.forced_segment(board, ctrl, &add_corner, &neck_down_end_point, ctrl.trace_half_width[lu], layer, &net_numbers);
            if !current_ok_point.is_to() {
                return NeckDownResult::From;
            }
            let add_corner = Point::Int(calculate_additional_corner(&float_neck_down_end_point, &float_to_corner, !horizontal_first, angle).round());
            if add_corner != *to_corner {
                let current_ok_point = self.forced_segment(board, ctrl, &neck_down_end_point, &add_corner, ctrl.trace_half_width[lu], layer, &net_numbers);
                if !current_ok_point.is_to() {
                    return NeckDownResult::From;
                }
                neck_down_end_point = add_corner;
            }
        }
        let r = self.forced_segment(board, ctrl, &neck_down_end_point, to_corner, neck_down_halfwidth, layer, &net_numbers);
        match r {
            ForcedTraceEnd::Failed => NeckDownResult::Null,
            ForcedTraceEnd::To(_) => NeckDownResult::To,
            ForcedTraceEnd::From(_) => {
                if end_is_from {
                    NeckDownResult::From
                } else {
                    NeckDownResult::Other
                }
            }
            ForcedTraceEnd::Other(_) => NeckDownResult::Other,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn forced_segment(
        &self,
        board: &mut RoutingBoard,
        ctrl: &AutorouteControl,
        from: &Point,
        to: &Point,
        half_width: i32,
        layer: LayerNo,
        net_numbers: &[NetNo],
    ) -> ForcedTraceEnd {
        board.insert_forced_trace_segment(
            from,
            to,
            half_width,
            layer,
            net_numbers,
            ctrl.trace_clearance_class,
            ctrl.max_shove_trace_recursion_depth,
            ctrl.max_shove_via_recursion_depth,
            ctrl.max_spring_over_recursion_depth,
            i32::MAX,
            ctrl.pull_tight_accuracy,
            true,
            None,
        )
    }

    /// Java `insertVia(location, fromLayer, toLayer)`: inserts the cheapest via covering both
    /// layers. Returns false if no suitable via was found or the insertion failed.
    fn insert_via(&mut self, board: &mut RoutingBoard, ctrl: &AutorouteControl, location: Option<&Point>, input_from_layer: LayerNo, input_to_layer: LayerNo) -> bool {
        if input_from_layer == input_to_layer {
            return true; // no via necessary
        }
        let (from_layer, to_layer) = if input_from_layer < input_to_layer { (input_from_layer, input_to_layer) } else { (input_to_layer, input_from_layer) };
        let Some(location) = location else {
            // Java: NullPointerException in ForcedViaInserter.check
            panic!("FoundConnectionInserter.insert_via: location is null");
        };
        let net_numbers = [ctrl.net_number];
        let mut via_info = None;
        let mut found_suitable_span = false;
        for i in 0..ctrl.via_rule.via_count() {
            let current_via_info = board.rules.via_infos[ctrl.via_rule.get_via(i)].clone();
            let (pf, pt) = {
                let ps = board.library.padstacks.get(current_via_info.get_padstack()).expect("via padstack");
                (ps.from_layer(), ps.to_layer())
            };
            if pf > from_layer || pt < to_layer {
                continue;
            }
            found_suitable_span = true;
            if forced_via_inserter::check(
                board,
                &current_via_info,
                location,
                &net_numbers,
                ctrl.max_shove_trace_recursion_depth,
                ctrl.max_shove_via_recursion_depth,
                Some(&ctrl.trace_half_width),
                ctrl.trace_clearance_class,
            ) {
                via_info = Some(current_via_info);
                break;
            }
        }
        let Some(via_info) = via_info else {
            if !found_suitable_span {
                log::debug!("FoundConnectionInserter: via mask not found for net #{} covering layers {from_layer} to {to_layer}", ctrl.net_number);
            } else {
                log::debug!("FoundConnectionInserter: via placement blocked by clearance/shove limits for net #{}", ctrl.net_number);
            }
            return false;
        };
        if !forced_via_inserter::insert(
            board,
            &via_info,
            location,
            &net_numbers,
            ctrl.trace_clearance_class,
            &ctrl.trace_half_width,
            ctrl.max_shove_trace_recursion_depth,
            ctrl.max_shove_via_recursion_depth,
        ) {
            log::debug!("FoundConnectionInserter: forced via failed for net #{}", ctrl.net_number);
            return false;
        }
        true
    }
}
