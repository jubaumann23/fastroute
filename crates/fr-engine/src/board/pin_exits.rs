//! The pin exit restriction parts of `Pin` (`getTraceExitRestrictions`,
//! `calcNearestExitRestrictionDirection`, `nearestTraceExitCorner`, widths, swappable pins)
//! and of `PolylineTrace` (`checkConnectionToPin`, `correctConnectionToPin`,
//! `swapConnectionToPin`).

use fr_geom::{Direction, FloatPoint, IntBox, Line, Point, Polyline, Shape, Signum, TileShape};

use crate::ids::{AngleRestriction, FixedState, LayerNo};

use super::basic_board::BasicBoard;
use super::item::{ItemKey, ItemKind};
use super::optimize::tracked::{BorderLines, TLine, TPolyline};
use super::item_list::ItemSet;

/// Java `Pin.TraceExitRestriction`.
#[derive(Clone, Debug, PartialEq)]
pub struct TraceExitRestriction {
    pub direction: Direction,
    pub min_length: f64,
}

impl BasicBoard {
    /// Java `Pin.getTraceExitRestrictions(layer)`: the allowed trace exit directions of the pad
    /// on `layer` with the minimal line lengths into them.
    pub fn pin_trace_exit_restrictions(&self, pin: ItemKey, layer: LayerNo) -> Vec<TraceExitRestriction> {
        let mut result = Vec::new();
        let item = self.item(pin);
        let ItemKind::Pin(p) = &item.kind else {
            return result;
        };
        let first_layer = item.first_layer(self);
        let padstack_layer = item.pin_padstack_layer(self, layer - first_layer);
        let mut pad_xy_factor = 1.5;
        let component = if item.component_no() > 0 && item.component_no() <= self.components.count() {
            Some(self.components.get(item.component_no()))
        } else {
            None
        };
        if let Some(c) = component {
            if self.library.packages.get(c.get_package()).pin_count() <= 3 {
                pad_xy_factor *= 2.0; // allow connection to the longer side also for shorter pads.
            }
        }
        let Some(padstack) = item.padstack(self) else {
            return result;
        };
        let exit_directions = padstack.get_trace_exit_directions(padstack_layer, pad_xy_factor);
        if exit_directions.is_empty() {
            return result;
        }
        let Some(component) = component else {
            return result;
        };
        let Some(Shape::Tile(pad_shape)) = item.drill_shape(self, layer - first_layer) else {
            return result;
        };
        let component_rotation = component.get_rotation_in_degree();
        let pin_center = item.center(self);
        let center_approx = pin_center.to_float();
        for padstack_exit_direction in &exit_directions {
            let package = self.library.packages.get(component.get_package());
            let Some(package_pin) = package.get_pin(p.pin_index) else {
                continue;
            };
            let rotation = component_rotation + package_pin.rotation_in_degree;
            let exit_direction = if rotation % 45.0 == 0.0 {
                padstack_exit_direction.turn_45_degree((rotation as i32) / 45)
            } else {
                let angle = rotation.to_radians() + padstack_exit_direction.angle_approx();
                Direction::get_instance_approx(angle)
            };
            // calculate the minimum line length from the pin center into the exit direction
            let border_line_no = pad_shape.intersecting_border_line_no(&pin_center, &exit_direction);
            if border_line_no < 0 {
                log::warn!("Pin.get_trace_exit_restrictions: border line not found");
                continue;
            }
            let exit_line = Line::from_point_direction(pin_center.clone(), exit_direction.clone());
            let nearest_border_point = exit_line.intersection_approx(&pad_shape.border_line(border_line_no));
            result.push(TraceExitRestriction { direction: exit_direction, min_length: center_approx.distance(&nearest_border_point) });
        }
        result
    }

    /// Java `Pin.hasTraceExitRestrictions()`.
    pub fn pin_has_trace_exit_restrictions(&self, pin: ItemKey) -> bool {
        let item = self.item(pin);
        (item.first_layer(self)..=item.last_layer(self)).any(|l| !self.pin_trace_exit_restrictions(pin, l).is_empty())
    }

    fn pin_padstack_box(&self, pin: ItemKey, layer: LayerNo) -> Option<IntBox> {
        let item = self.item(pin);
        let padstack_layer = item.pin_padstack_layer(self, layer - item.first_layer(self));
        match item.padstack(self)?.get_shape(padstack_layer) {
            None => {
                log::warn!("Pin.get_min_width: padstackShape is null");
                None
            }
            Some(s) => Some(s.bounding_box()),
        }
    }

    /// Java `Pin.getMinWidth(layer)`.
    pub fn pin_min_width(&self, pin: ItemKey, layer: LayerNo) -> f64 {
        self.pin_padstack_box(pin, layer).map(|b| b.min_width()).unwrap_or(0.0)
    }

    /// Java `Pin.getMaxWidth(layer)`.
    pub fn pin_max_width(&self, pin: ItemKey, layer: LayerNo) -> f64 {
        self.pin_padstack_box(pin, layer).map(|b| b.max_width()).unwrap_or(0.0)
    }

    /// Java `Pin.getTraceNeckdownHalfwidth(layer)`.
    pub fn pin_trace_neckdown_half_width(&self, pin: ItemKey, layer: LayerNo) -> i32 {
        (0.5 * self.pin_min_width(pin, layer) - 1.0).max(1.0) as i32
    }

    /// Java `Pin.getSwappablePins()`.
    pub fn pin_swappable_pins(&self, pin: ItemKey) -> ItemSet {
        let mut result = ItemSet::new();
        let item = self.item(pin);
        let Some(p) = item.as_pin() else { return result };
        if item.component_no() <= 0 || item.component_no() > self.components.count() {
            return result;
        }
        let component = self.components.get(item.component_no());
        let Some(lp_no) = component.get_logical_part() else { return result };
        let logical_part = self.library.logical_parts.get(lp_no);
        let Some(this_part_pin) = logical_part.get_pin(p.pin_index) else { return result };
        if this_part_pin.gate_pin_swap_code <= 0 {
            return result;
        }
        // look up all part pins with the same gate name and the same gate pin swap code
        for i in 0..logical_part.pin_count() {
            if i == p.pin_index {
                continue;
            }
            if let Some(current) = logical_part.get_pin(i) {
                if current.gate_pin_swap_code == this_part_pin.gate_pin_swap_code && current.gate_name == this_part_pin.gate_name {
                    match self.get_pin(item.component_no(), current.pin_index) {
                        Some(k) => {
                            result.insert(self.item(k).id(), k);
                        }
                        None => log::warn!("Pin.get_swappable_pins: swappable pin not found"),
                    }
                }
            }
        }
        result
    }

    /// Java `Pin.calcNearestExitRestrictionDirection(tracePolyline, traceHalfWidth, layer)`.
    pub fn pin_nearest_exit_restriction_direction(&self, pin: ItemKey, trace_polyline: &Polyline, trace_half_width: i32, layer: LayerNo) -> Option<Direction> {
        let restrictions = self.pin_trace_exit_restrictions(pin, layer);
        if restrictions.is_empty() {
            return None;
        }
        let item = self.item(pin);
        let Some(Shape::Tile(pin_shape)) = item.drill_shape(self, layer - item.first_layer(self)) else {
            return None;
        };
        let edge_to_turn_dist = self.rules.get_pin_edge_to_turn_dist();
        if edge_to_turn_dist < 0.0 {
            return None;
        }
        let offset_pin_shape = pin_shape.offset(edge_to_turn_dist + trace_half_width as f64);
        let entries = offset_pin_shape.entrance_points(trace_polyline);
        let latest = *entries.last()?;
        let trace_entry_location = trace_polyline.lines[latest[0] as usize].intersection_approx(&offset_pin_shape.border_line(latest[1]));
        let pin_center = item.center(self);
        nearest_exit(&offset_pin_shape, &pin_center, &restrictions, &trace_entry_location, trace_polyline).map(|n| n.direction)
    }

    /// Java `Pin.nearestTraceExitCorner(fromPoint, traceHalfWidth, layer)`.
    pub fn pin_nearest_trace_exit_corner(&self, pin: ItemKey, from_point: &FloatPoint, trace_half_width: i32, layer: LayerNo) -> Option<FloatPoint> {
        let restrictions = self.pin_trace_exit_restrictions(pin, layer);
        if restrictions.is_empty() {
            return None;
        }
        let item = self.item(pin);
        let Some(Shape::Tile(pin_shape)) = item.drill_shape(self, layer - item.first_layer(self)) else {
            return None;
        };
        let pin_center = item.center(self);
        let edge_to_turn_dist = self.rules.get_pin_edge_to_turn_dist();
        if edge_to_turn_dist < 0.0 {
            return None;
        }
        let offset_pin_shape = pin_shape.offset(edge_to_turn_dist + trace_half_width as f64);
        let mut min_distance = f64::MAX;
        let mut nearest = None;
        for r in &restrictions {
            let border_line_no = offset_pin_shape.intersecting_border_line_no(&pin_center, &r.direction);
            let exit_ray = Line::from_point_direction(pin_center.clone(), r.direction.clone());
            let exit_corner = exit_ray.intersection_approx(&offset_pin_shape.border_line(border_line_no));
            let d = exit_corner.distance_square(from_point);
            if d < min_distance {
                min_distance = d;
                nearest = Some(exit_corner);
            }
        }
        nearest
    }

    /// The first pin (in contact set order) among the start or end contacts of a trace.
    fn trace_end_pin(&self, trace: ItemKey, at_start: bool) -> Option<ItemKey> {
        let contacts = if at_start { self.trace_start_contacts(trace) } else { self.trace_end_contacts(trace) };
        let result = contacts.iter().find(|k| self.item(*k).is_pin());
        result
    }

    /// Java `PolylineTrace.checkConnectionToPin(atStart)`: false if a pin is at that end and
    /// the connection violates its exit restrictions.
    pub fn check_connection_to_pin(&self, trace: ItemKey, at_start: bool) -> bool {
        let item = self.item(trace);
        let t = item.trace();
        if t.corner_count() < 2 {
            return true;
        }
        let Some(contact_pin) = self.trace_end_pin(trace, at_start) else {
            return true;
        };
        let restrictions = self.pin_trace_exit_restrictions(contact_pin, t.layer());
        if restrictions.is_empty() {
            return true;
        }
        let (end_corner, prev_end_corner) = if at_start {
            (item.first_corner(), t.polyline().corner(1))
        } else {
            (item.last_corner(), t.polyline().corner(t.polyline().corner_count() - 2))
        };
        let Some(trace_end_direction) = Direction::get_instance_from_points(&end_corner, &prev_end_corner) else {
            return true;
        };
        let Some(matching) = restrictions.iter().find(|r| r.direction == trace_end_direction) else {
            return false;
        };
        let edge_to_turn_dist = self.rules.get_pin_edge_to_turn_dist();
        if edge_to_turn_dist < 0.0 {
            return false;
        }
        let end_line_length = end_corner.to_float().distance(&prev_end_corner.to_float());
        let current_clearance = self.clearance_value(item.clearance_class(), self.item(contact_pin).clearance_class(), t.layer()) as f64;
        let add_width = edge_to_turn_dist.max(current_clearance + 1.0);
        let preserve_length = matching.min_length + t.half_width() as f64 + add_width;
        preserve_length <= end_line_length
    }

    /// Java `PolylineTrace.correctConnectionToPin(atStart, angleRestriction)`: tries to
    /// correct a violated pin exit restriction. Returns true if the trace was changed.
    pub fn correct_connection_to_pin(&mut self, trace: ItemKey, at_start: bool, angle_restriction: AngleRestriction) -> bool {
        if self.check_connection_to_pin(trace, at_start) {
            return false;
        }
        let item = self.item(trace);
        let t = item.trace();
        let layer = t.layer();
        let half_width = t.half_width();
        let trace_tpolyline = if at_start { t.tpolyline() } else { t.tpolyline().reverse() };
        let trace_polyline = trace_tpolyline.polyline.clone();
        let Some(contact_pin) = self.trace_end_pin(trace, at_start) else {
            return false;
        };
        let restrictions = self.pin_trace_exit_restrictions(contact_pin, layer);
        if restrictions.is_empty() {
            return false;
        }
        let pin_item = self.item(contact_pin);
        let Some(Shape::Tile(pin_shape)) = pin_item.drill_shape(self, layer - pin_item.first_layer(self)) else {
            return false;
        };
        let edge_to_turn_dist = self.rules.get_pin_edge_to_turn_dist();
        if edge_to_turn_dist < 0.0 {
            return false;
        }
        let current_clearance = self.clearance_value(item.clearance_class(), pin_item.clearance_class(), layer) as f64;
        let add_width = edge_to_turn_dist.max(current_clearance + 1.0);
        let mut offset_pin_shape = pin_shape.offset(half_width as f64 + add_width);
        if angle_restriction == AngleRestriction::NinetyDegree || offset_pin_shape.is_int_box() {
            offset_pin_shape = TileShape::IntBox(offset_pin_shape.bounding_box());
        } else if angle_restriction == AngleRestriction::FortyfiveDegree {
            offset_pin_shape = TileShape::IntOctagon(offset_pin_shape.bounding_octagon().expect("bounding octagon"));
        }
        let entries = offset_pin_shape.entrance_points(&trace_polyline);
        let Some(&latest) = entries.last() else {
            return false;
        };
        let trace_entry_location = trace_polyline.lines[latest[0] as usize].intersection_approx(&offset_pin_shape.border_line(latest[1]));
        let pin_center = pin_item.center(self);
        let Some(nearest) = nearest_exit(&offset_pin_shape, &pin_center, &restrictions, &trace_entry_location, &trace_polyline) else {
            // Java would dereference null here
            panic!("correctConnectionToPin: no exit restriction selected");
        };
        // the Java line objects (the exit ray is shared with the exit stub, the border lines of a
        // simplex are the same objects on every call)
        let exit_ray = TLine::fresh(nearest.exit_ray.clone());
        let mut border = BorderLines::new(&offset_pin_shape);
        // append the polygon piece around the border of the pin shape.
        let corner_count = offset_pin_shape.border_line_count();
        let clock_wise_side_diff = (nearest.border_line_no - latest[1] + corner_count) % corner_count;
        let counter_clock_wise_side_diff = (latest[1] - nearest.border_line_no + corner_count) % corner_count;
        let mut current_border_line_no = nearest.border_line_no;
        let mut middle: Vec<TLine> = Vec::new();
        if counter_clock_wise_side_diff <= clock_wise_side_diff {
            for _ in 0..=counter_clock_wise_side_diff {
                middle.push(border.line(current_border_line_no));
                current_border_line_no = (current_border_line_no + 1) % corner_count;
            }
        } else {
            for _ in 0..=clock_wise_side_diff {
                middle.push(border.line(current_border_line_no));
                current_border_line_no = (current_border_line_no - 1 + corner_count) % corner_count;
            }
        }
        let mut current_lines: Vec<TLine> = Vec::with_capacity(middle.len() + 2);
        current_lines.push(exit_ray.clone());
        current_lines.extend(middle);
        current_lines.push(trace_tpolyline.tline(latest[0] as usize));
        // (Java normalizes the directions in current_lines if nothing is filtered)
        let border_polyline = TPolyline::from_lines(&mut current_lines);
        let nets = item.net_numbers().to_vec();
        let cl = item.clearance_class();
        if !self.check_polyline_trace(&border_polyline.polyline, layer, half_width, &nets, cl) {
            return false;
        }
        let mut cut_lines: Vec<TLine> = Vec::with_capacity(trace_polyline.lines.len() - latest[0] as usize + 1);
        cut_lines.push(current_lines[current_lines.len() - 2].clone());
        cut_lines.extend(trace_tpolyline.tlines_range(latest[0] as usize..trace_tpolyline.len()));
        let cut_polyline = TPolyline::from_lines(&mut cut_lines);
        let mut changed_polyline = if cut_polyline.polyline.first_corner() == cut_polyline.polyline.last_corner() {
            border_polyline
        } else {
            border_polyline.combine(&cut_polyline)
        };
        if !at_start {
            changed_polyline = changed_polyline.reverse();
        }
        self.change_trace_tracked(trace, changed_polyline);
        // create a shove fixed exit line.
        let mut exit_lines = vec![
            TLine::fresh(Line::from_point_direction(pin_center.clone(), nearest.direction.turn_45_degree(2))),
            exit_ray,
            border.line(nearest.border_line_no),
        ];
        let exit_line_segment = TPolyline::from_lines(&mut exit_lines);
        self.insert_trace_tracked(exit_line_segment, layer, half_width, &nets, cl, FixedState::ShoveFixed);
        true
    }

    /// Java `PolylineTrace.swapConnectionToPin(atStart)`: if another pin exit restriction fits
    /// better, combines this trace with the shove fixed exit trace. Returns true if changed.
    pub fn swap_connection_to_pin(&mut self, trace: ItemKey, at_start: bool) -> bool {
        let item = self.item(trace);
        let t = item.trace();
        let (trace_polyline, contact_list) = if at_start {
            (t.polyline().clone(), self.trace_start_contacts(trace))
        } else {
            (t.polyline().reverse(), self.trace_end_contacts(trace))
        };
        if contact_list.len() != 1 {
            return false;
        }
        let contact = contact_list.first().unwrap();
        let contact_item = self.item(contact);
        let Some(contact_trace) = contact_item.as_trace() else {
            return false;
        };
        if contact_item.fixed_state() != FixedState::ShoveFixed {
            return false;
        }
        let contact_polyline = contact_trace.polyline().clone();
        let contact_last_line = &contact_polyline.lines[contact_polyline.lines.len() - 2];
        // look, if this trace has a sharp angle with the contact trace.
        let first_line = &trace_polyline.lines[1];
        let mut check_swap = contact_last_line.direction().projection(&first_line.direction()) == Signum::Negative;
        if !check_swap {
            let half_width = t.half_width() as f64;
            if trace_polyline.lines.len() > 3 && trace_polyline.corner_approx(0).distance_square(&trace_polyline.corner_approx(1)) <= half_width * half_width {
                // check also for sharp angle with the second line
                check_swap = contact_last_line.direction().projection(&trace_polyline.lines[2].direction()) == Signum::Negative;
            }
        }
        if !check_swap {
            return false;
        }
        let Some(contact_pin) = self.trace_start_contacts(contact).iter().find(|k| self.item(*k).is_pin()) else {
            return false;
        };
        let combined = contact_polyline.combine(Some(&trace_polyline));
        let nearest = self.pin_nearest_exit_restriction_direction(contact_pin, &combined, t.half_width(), t.layer());
        match nearest {
            None => return false,
            Some(d) if d == contact_polyline.lines[1].direction() => return false, // direction would not be changed
            Some(_) => {}
        }
        let fixed = item.fixed_state();
        self.item_mut(contact).set_fixed_state(fixed);
        self.combine_trace(trace);
        true
    }
}

struct NearestExit {
    direction: Direction,
    exit_ray: Line,
    border_line_no: i32,
}

/// The nearest legal pin exit to the trace entry location (the common loop of
/// `calcNearestExitRestrictionDirection` and `correctConnectionToPin`).
fn nearest_exit(
    offset_pin_shape: &TileShape,
    pin_center: &Point,
    restrictions: &[TraceExitRestriction],
    trace_entry_location: &FloatPoint,
    trace_polyline: &Polyline,
) -> Option<NearestExit> {
    const TOLERANCE: f64 = 1.0;
    let mut min_exit_corner_distance = f64::MAX;
    let mut nearest_exit_corner: Option<FloatPoint> = None;
    let mut result: Option<NearestExit> = None;
    for r in restrictions {
        let border_line_no = offset_pin_shape.intersecting_border_line_no(pin_center, &r.direction);
        let exit_ray = Line::from_point_direction(pin_center.clone(), r.direction.clone());
        let exit_corner = exit_ray.intersection_approx(&offset_pin_shape.border_line(border_line_no));
        let d = exit_corner.distance_square(trace_entry_location);
        let mut new_nearest = false;
        if d + TOLERANCE < min_exit_corner_distance {
            new_nearest = true;
        } else if d < min_exit_corner_distance + TOLERANCE {
            // the distances are near equal, compare to the previous corners of the polyline
            let nearest_corner = nearest_exit_corner.expect("nearest exit corner");
            for i in 1..trace_polyline.corner_count() {
                let corner = trace_polyline.corner_approx(i);
                let current_distance = corner.distance_square(&exit_corner);
                let old_distance = corner.distance_square(&nearest_corner);
                if current_distance + TOLERANCE < old_distance {
                    new_nearest = true;
                    break;
                } else if current_distance > old_distance + TOLERANCE {
                    break;
                }
            }
        }
        if new_nearest {
            min_exit_corner_distance = d;
            nearest_exit_corner = Some(exit_corner);
            result = Some(NearestExit { direction: r.direction.clone(), exit_ray, border_line_no });
        }
    }
    result
}
