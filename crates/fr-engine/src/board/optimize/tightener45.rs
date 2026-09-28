//! Port of `board/optimize/TraceTightener45.java`.

use fr_geom::limits::SQRT2;
use fr_geom::{Direction, FloatPoint, IntPoint, Line, Point, Polyline, Side, Signum};

use super::super::item::ItemKey;
use super::super::routing_board::{jmax, jmin, RoutingBoard};
use super::trace_tightener::TraceTightener;
use super::tracked::{TLine, TPolyline};

impl TraceTightener {
    /// Java `TraceTightener45.pullTight(polyline)`.
    pub(crate) fn pull_tight_45(&mut self, board: &mut RoutingBoard, polyline: &TPolyline) -> TPolyline {
        let mut new_result = self.avoid_acid_traps(polyline);
        let mut prev_result: Option<TPolyline> = None;
        while prev_result.as_ref().map(|p| !new_result.same(p)).unwrap_or(true) && !self.is_stop_requested() {
            prev_result = Some(new_result.clone());
            let tmp1 = self.reduce_corners_45(board, &new_result);
            let tmp2 = self.smoothen_corners_45(board, &tmp1);
            new_result = self.reposition_lines(board, &tmp2);
        }
        new_result
    }

    /// Java `reduceCorners(polyline)`: tries to reduce the number of corners.
    fn reduce_corners_45(&mut self, board: &mut RoutingBoard, polyline: &TPolyline) -> TPolyline {
        let p = &polyline.polyline;
        if p.lines.len() <= 4 {
            return polyline.clone();
        }
        let mut current_corner: [Point; 4] = [p.corner(0), p.corner(1), p.corner(2), p.corner(3)];
        for c in &current_corner {
            if !c.is_int_point() {
                return polyline.clone();
            }
        }
        let mut in_clip: [bool; 4] = [true; 4];
        for i in 0..4 {
            in_clip[i] = !self.clip_is_outside(&current_corner[i]);
        }
        let mut polyline_changed = false;
        let mut new_corners: Vec<Point> = Vec::with_capacity(p.lines.len() - 3);
        new_corners.push(current_corner[0].clone());
        let mut new_corner: Option<Point> = None;
        let len = p.lines.len() as i32;
        let mut corner_index = 3;
        while corner_index < len - 1 {
            current_corner[3] = p.corner(corner_index);
            if !current_corner[3].is_int_point() {
                return polyline.clone();
            }
            if current_corner[1] == current_corner[2]
                || (corner_index < len - 2 && current_corner[3].side_of(&current_corner[1], &current_corner[2]) == Side::Collinear)
            {
                // corners in the middle af a line can be skipped
                corner_index += 1;
                current_corner[2] = current_corner[3].clone();
                in_clip[2] = in_clip[3];
                if corner_index < len - 1 {
                    current_corner[3] = p.corner(corner_index);
                    if !current_corner[3].is_int_point() {
                        return polyline.clone();
                    }
                }
                polyline_changed = true;
            }
            in_clip[3] = !self.clip_is_outside(&current_corner[3]);
            let mut corner_removed = false;
            if in_clip[1] && in_clip[2] && in_clip[3] {
                // translate the line from current_corner[2] to current_corner[1] to current_corner[3]
                let delta = current_corner[3].difference_by(&current_corner[2]);
                let nc = current_corner[1].translate_by(&delta);
                new_corner = Some(nc.clone());
                if current_corner[3] == current_corner[2] {
                    // just remove multiple corner
                    corner_removed = true;
                } else if nc.side_of(&current_corner[0], &current_corner[1]) == Side::Collinear {
                    let check_polyline = Polyline::from_points(&[nc.clone(), current_corner[1].clone()]);
                    if check_polyline.lines.len() == 3 {
                        if self.check_offset_shape(board, &check_polyline, 0) {
                            if nc == current_corner[3] {
                                corner_removed = true;
                            } else {
                                let check_polyline = Polyline::from_points(&[nc.clone(), current_corner[3].clone()]);
                                if check_polyline.lines.len() == 3 {
                                    corner_removed = self.check_offset_shape(board, &check_polyline, 0);
                                } else {
                                    corner_removed = true;
                                }
                            }
                        }
                    } else {
                        corner_removed = true;
                    }
                }
            }
            if !corner_removed && in_clip[0] && in_clip[1] && in_clip[2] {
                // the first try has failed. Try to translate the line from corner2 to corner1
                // to corner0
                let delta = current_corner[0].difference_by(&current_corner[1]);
                let nc = current_corner[2].translate_by(&delta);
                new_corner = Some(nc.clone());
                if current_corner[0] == current_corner[1] {
                    // just remove multiple corner
                    corner_removed = true;
                } else if nc.side_of(&current_corner[2], &current_corner[3]) == Side::Collinear {
                    let check_polyline = Polyline::from_points(&[nc.clone(), current_corner[0].clone()]);
                    if check_polyline.lines.len() == 3 {
                        if self.check_offset_shape(board, &check_polyline, 0) {
                            let check_polyline = Polyline::from_points(&[nc.clone(), current_corner[2].clone()]);
                            if check_polyline.lines.len() == 3 {
                                corner_removed = self.check_offset_shape(board, &check_polyline, 0);
                            } else {
                                corner_removed = true;
                            }
                        }
                    } else {
                        corner_removed = true;
                    }
                }
            }
            if corner_removed {
                polyline_changed = true;
                let nc = new_corner.clone().expect("TraceTightener45.reduce_corners: new corner");
                current_corner[1] = nc.clone();
                in_clip[1] = !self.clip_is_outside(&current_corner[1]);
                if board.changed_area.is_some() {
                    self.join(board, &nc.to_float());
                    self.join(board, &current_corner[1].to_float());
                    self.join(board, &current_corner[2].to_float());
                }
            } else {
                new_corners.push(current_corner[1].clone());
                current_corner[0] = current_corner[1].clone();
                current_corner[1] = current_corner[2].clone();
                in_clip[0] = in_clip[1];
                in_clip[1] = in_clip[2];
            }
            current_corner[2] = current_corner[3].clone();
            in_clip[2] = in_clip[3];
            corner_index += 1;
        }
        if !polyline_changed {
            return polyline.clone();
        }
        let mut adjusted = new_corners;
        adjusted.push(current_corner[1].clone());
        adjusted.push(current_corner[2].clone());
        TPolyline::from_points(&adjusted)
    }

    /// Java `smoothenCorners(polyline)`: smoothens the 90 degree corners to 45 degree.
    fn smoothen_corners_45(&mut self, board: &mut RoutingBoard, polyline: &TPolyline) -> TPolyline {
        let mut result = polyline.clone();
        let mut polyline_changed = true;
        while polyline_changed {
            if result.len() < 4 {
                return result;
            }
            polyline_changed = false;
            let mut lines = result.tlines();
            let mut i = 1usize;
            while i + 2 < lines.len() {
                let d1 = lines[i].line.direction();
                let d2 = lines[i + 1].line.direction();
                if d1.is_multiple_of_45_degree() && d2.is_multiple_of_45_degree() && d1.projection(&d2) != Signum::Positive {
                    // there is a 90 degree or sharper angle
                    let mut new_line = self.smoothen_corner_45(board, &lines, i);
                    if new_line.is_none() {
                        // the greedy smoothening couldn't change the polyline
                        new_line = self.smoothen_sharp_corner_45(board, &lines, i);
                    }
                    if let Some(nl) = new_line {
                        polyline_changed = true;
                        lines.insert(i + 1, nl);
                        i += 1;
                    }
                }
                i += 1;
            }
            if polyline_changed {
                result = TPolyline::from_lines(&mut lines);
            }
        }
        result
    }

    /// Java `smoothenSharpCorner(lines, no)`: a line smoothening the 90 degree corner, so close
    /// to the corner that no clearance check is necessary.
    fn smoothen_sharp_corner_45(&mut self, board: &mut RoutingBoard, lines: &[TLine], no: usize) -> Option<TLine> {
        let current_corner = lines[no].line.intersection_approx(&lines[no + 1].line);
        if current_corner.x != (current_corner.x as i32) as f64 {
            // intersection of 2 diagonal lines is not integer
            if let Some(r) = self.smoothen_non_integer_corner_45(lines, no) {
                return Some(r);
            }
        }
        let prev_corner = lines[no].line.intersection_approx(&lines[no - 1].line);
        let next_corner = lines[no + 1].line.intersection_approx(&lines[no + 2].line);
        let prev_dir = lines[no].line.direction();
        let next_dir = lines[no + 1].line.direction();
        let new_line_dir = Direction::get_instance(&prev_dir.get_vector().add(&next_dir.get_vector()));
        let translate_line = Line::get_instance(Point::Int(current_corner.round()), &new_line_dir);
        let mut translate_dist = (SQRT2 - 1.0) * self.current_half_width as f64;
        let prev_dist = translate_line.signed_distance(&prev_corner).abs();
        let next_dist = translate_line.signed_distance(&next_corner).abs();
        translate_dist = jmin(translate_dist, prev_dist);
        translate_dist = jmin(translate_dist, next_dist);
        if translate_dist < 0.99 {
            return None;
        }
        translate_dist = jmax(translate_dist - 1.0, 1.0);
        if translate_line.side_of_float(&next_corner) == Side::OnTheLeft {
            translate_dist = -translate_dist;
        }
        let result = translate_line.translate(translate_dist);
        if board.changed_area.is_some() {
            self.join(board, &current_corner);
        }
        Some(self.ids.line(result))
    }

    /// Java `smoothenNonIntegerCorner(lines, no)`: a short axis parallel line removing a non
    /// integer corner of two intersecting diagonal lines.
    fn smoothen_non_integer_corner_45(&mut self, lines: &[TLine], no: usize) -> Option<TLine> {
        let prev_line = &lines[no].line;
        let next_line = &lines[no + 1].line;
        if prev_line.is_equal_or_opposite(next_line) {
            return None;
        }
        if !(prev_line.is_diagonal() && next_line.is_diagonal()) {
            return None;
        }
        let current_corner = prev_line.intersection_approx(next_line);
        let prev_corner = prev_line.intersection_approx(&lines[no - 1].line);
        let next_corner = next_line.intersection_approx(&lines[no + 2].line);
        let mut new_x = 0;
        let mut new_y = 0;
        let mut new_line_is_vertical = false;
        let mut new_line_is_horizontal = false;
        if prev_corner.x > current_corner.x && next_corner.x > current_corner.x {
            new_x = current_corner.x.ceil() as i32;
            new_y = current_corner.y.ceil() as i32;
            new_line_is_vertical = true;
        } else if prev_corner.x < current_corner.x && next_corner.x < current_corner.x {
            new_x = current_corner.x.floor() as i32;
            new_y = current_corner.y.floor() as i32;
            new_line_is_vertical = true;
        } else if prev_corner.y > current_corner.y && next_corner.y > current_corner.y {
            new_x = current_corner.x.ceil() as i32;
            new_y = current_corner.y.ceil() as i32;
            new_line_is_horizontal = true;
        } else if prev_corner.y < current_corner.y && next_corner.y < current_corner.y {
            new_x = current_corner.x.floor() as i32;
            new_y = current_corner.y.floor() as i32;
            new_line_is_horizontal = true;
        }
        let new_line_dir = if new_line_is_vertical {
            if prev_corner.y < next_corner.y {
                Direction::UP
            } else {
                Direction::DOWN
            }
        } else if new_line_is_horizontal {
            if prev_corner.x < next_corner.x {
                Direction::RIGHT
            } else {
                Direction::LEFT
            }
        } else {
            return None;
        };
        let line_a = Point::Int(IntPoint::new(new_x, new_y));
        Some(self.ids.line(Line::from_point_direction(line_a, new_line_dir)))
    }

    /// Java `smoothenCorner(lines, no)`: a line smoothening the 90 degree corner as far as
    /// possible (with clearance checks).
    fn smoothen_corner_45(&mut self, board: &mut RoutingBoard, lines: &[TLine], no: usize) -> Option<TLine> {
        let prev_corner = lines[no].line.intersection_approx(&lines[no - 1].line);
        let current_corner = lines[no].line.intersection_approx(&lines[no + 1].line);
        let next_corner = lines[no + 1].line.intersection_approx(&lines[no + 2].line);
        let prev_dir = lines[no].line.direction();
        let next_dir = lines[no + 1].line.direction();
        let new_line_dir = Direction::get_instance(&prev_dir.get_vector().add(&next_dir.get_vector()));
        let translate_line = Line::get_instance(Point::Int(current_corner.round()), &new_line_dir);
        let prev_dist = translate_line.signed_distance(&prev_corner).abs();
        let next_dist = translate_line.signed_distance(&next_corner).abs();
        if prev_dist == 0.0 || next_dist == 0.0 {
            return None;
        }
        let (mut max_translate_dist, nearest_corner) = if prev_dist <= next_dist { (prev_dist, prev_corner) } else { (next_dist, next_corner) };
        if max_translate_dist < 1.0 {
            return None;
        }
        max_translate_dist = jmax(max_translate_dist - 1.0, 1.0);
        if translate_line.side_of_float(&next_corner) == Side::OnTheLeft {
            max_translate_dist = -max_translate_dist;
        }
        let mut check_lines: Vec<TLine> = vec![lines[no].clone(), lines[no].clone(), lines[no + 1].clone()];
        let mut translate_dist = max_translate_dist;
        let mut delta_dist = max_translate_dist;
        let side_of_nearest_corner = translate_line.side_of_float(&nearest_corner);
        let sign = Signum::as_int(max_translate_dist);
        let mut result: Option<TLine> = None;
        while delta_dist.abs() > self.min_translate_dist as f64 {
            let mut check_ok = false;
            let new_line = translate_line.translate(translate_dist);
            let new_line_side = new_line.side_of_float(&nearest_corner);
            if new_line_side == side_of_nearest_corner || new_line_side == Side::Collinear {
                check_lines[1] = self.ids.line(new_line);
                let tmp = TPolyline::from_lines(&mut check_lines);
                if tmp.len() == 3 {
                    check_ok = self.check_offset_shape(board, &tmp.polyline, 0);
                }
                delta_dist /= 2.0;
                if check_ok {
                    result = Some(check_lines[1].clone());
                    if translate_dist == max_translate_dist {
                        // biggest possible change
                        break;
                    }
                    translate_dist += delta_dist;
                } else {
                    translate_dist -= delta_dist;
                }
            } else {
                // moved a little bit to far at the first time because of numerical inaccuracy
                let shorten_value = sign as f64 * 0.5;
                max_translate_dist -= shorten_value;
                translate_dist -= shorten_value;
                delta_dist -= shorten_value;
            }
        }
        if let Some(r) = &result {
            if board.changed_area.is_some() {
                let new_prev_corner = check_lines[0].line.intersection_approx(&r.line);
                let new_next_corner = check_lines[2].line.intersection_approx(&r.line);
                self.join(board, &new_prev_corner);
                self.join(board, &new_next_corner);
                self.join(board, &current_corner);
            }
        }
        result
    }

    /// Java `TraceTightener45.smoothenStartCornerAtTrace(trace)`.
    pub(crate) fn smoothen_start_corner_at_trace_45(&mut self, board: &mut RoutingBoard, trace: ItemKey) -> Option<TPolyline> {
        let trace_tp = board.item(trace).trace().tpolyline();
        let trace_polyline = trace_tp.polyline.clone();
        let current_end_corner = trace_polyline.corner(0);
        if self.clip_is_outside(&current_end_corner) {
            return None;
        }
        let current_prev_end_corner = trace_polyline.corner(1);
        let line_direction = trace_polyline.lines[1].direction();
        let prev_line_direction = trace_polyline.lines[2].direction();
        let contacts = board.trace_start_contacts(trace);
        let found = self.find_contact_trace_45(board, &contacts, &trace_polyline, &current_end_corner, &current_prev_end_corner, &line_direction, &prev_line_direction)?;
        self.build_smoothened_start(board, &trace_tp, &current_end_corner, &current_prev_end_corner, found, true)
    }

    /// Java `TraceTightener45.smoothenEndCornerAtTrace(trace)`.
    pub(crate) fn smoothen_end_corner_at_trace_45(&mut self, board: &mut RoutingBoard, trace: ItemKey) -> Option<TPolyline> {
        let trace_tp = board.item(trace).trace().tpolyline();
        let trace_polyline = trace_tp.polyline.clone();
        let current_end_corner = trace_polyline.last_corner();
        if self.clip_is_outside(&current_end_corner) {
            return None;
        }
        let current_prev_end_corner = trace_polyline.corner(trace_polyline.corner_count() - 2);
        let n = trace_polyline.lines.len();
        let line_direction = trace_polyline.lines[n - 2].direction().opposite();
        let prev_line_direction = trace_polyline.lines[n - 3].direction().opposite();
        let contacts = board.trace_end_contacts(trace);
        let found = self.find_contact_trace_45(board, &contacts, &trace_polyline, &current_end_corner, &current_prev_end_corner, &line_direction, &prev_line_direction)?;
        self.build_smoothened_start(board, &trace_tp, &current_end_corner, &current_prev_end_corner, found, false)
    }

    /// The contact loop of the 45 degree `smoothenStart/EndCornerAtTrace`. `None`: Java returns
    /// null (a contact is not an unfixed polyline trace).
    #[allow(clippy::too_many_arguments)]
    fn find_contact_trace_45(
        &mut self,
        board: &RoutingBoard,
        contacts: &super::super::item_list::ItemSet,
        trace_polyline: &Polyline,
        current_end_corner: &Point,
        current_prev_end_corner: &Point,
        line_direction: &Direction,
        prev_line_direction: &Direction,
    ) -> Option<ContactFound> {
        let mut found = ContactFound::default();
        for contact in contacts.iter() {
            let item = board.item(contact);
            if item.is_trace() && !board.is_shove_fixed(contact) {
                let contact_tp = item.trace().tpolyline();
                let (corner_approx, other_line, other_prev_line) = self.contact_trace_lines(&contact_tp, current_end_corner);
                let current_prev_corner_side = current_prev_end_corner.side_of_line(&other_line.line);
                let current_projection = line_direction.projection(&other_line.line.direction());
                let mut other_trace_found = false;
                if current_projection == Signum::Positive && current_prev_corner_side != Side::Collinear {
                    if other_line.line.direction().is_orthogonal() {
                        found.acute_angle = true;
                        other_trace_found = true;
                    }
                } else if current_projection == Signum::Zero
                    && trace_polyline.corner_count() > 2
                    && prev_line_direction.projection(&other_line.line.direction()) == Signum::Positive
                {
                    found.bend = true;
                    other_trace_found = true;
                }
                if other_trace_found {
                    found.other_trace_corner_approx = Some(corner_approx);
                    found.other_trace_line = Some(other_line);
                    found.prev_corner_side = Some(current_prev_corner_side);
                    found.other_prev_trace_line = Some(other_prev_line);
                }
            } else {
                return None;
            }
        }
        Some(found)
    }

    /// The construction part of the 45 degree `smoothenStartCornerAtTrace` (`at_start`) and
    /// `smoothenEndCornerAtTrace`.
    fn build_smoothened_start(
        &mut self,
        board: &mut RoutingBoard,
        trace_tp: &TPolyline,
        current_end_corner: &Point,
        current_prev_end_corner: &Point,
        found: ContactFound,
        at_start: bool,
    ) -> Option<TPolyline> {
        let n = trace_tp.len();
        if found.acute_angle {
            let other_trace_line = found.other_trace_line.clone().unwrap();
            let left = found.prev_corner_side == Some(Side::OnTheLeft);
            let factor = if at_start == left { 2 } else { 6 };
            let new_line_dir = other_trace_line.line.direction().turn_45_degree(factor);
            let translate_line = Line::get_instance(Point::Int(current_end_corner.to_float().round()), &new_line_dir);
            let mut translate_dist = (SQRT2 - 1.0) * self.current_half_width as f64;
            let prev_corner_dist = translate_line.signed_distance(&current_prev_end_corner.to_float()).abs();
            let other_dist = translate_line.signed_distance(found.other_trace_corner_approx.as_ref().unwrap()).abs();
            translate_dist = jmin(translate_dist, prev_corner_dist);
            translate_dist = jmin(translate_dist, other_dist);
            if translate_dist >= 0.99 {
                translate_dist = jmax(translate_dist - 1.0, 1.0);
                if translate_line.side_of(current_prev_end_corner) == Side::OnTheLeft {
                    translate_dist = -translate_dist;
                }
                let add_line = TLine::fresh(translate_line.translate(translate_dist));
                // construct the new trace polyline.
                let mut new_lines: Vec<TLine> = Vec::with_capacity(n + 1);
                if at_start {
                    new_lines.push(other_trace_line);
                    new_lines.push(add_line);
                    new_lines.extend(trace_tp.tlines_range(1..n));
                } else {
                    new_lines.extend(trace_tp.tlines_range(0..n - 1));
                    new_lines.push(add_line);
                    new_lines.push(other_trace_line);
                }
                return Some(TPolyline::from_lines(&mut new_lines));
            }
        } else if found.bend {
            let other_trace_line = found.other_trace_line.clone().unwrap();
            let other_prev_trace_line = found.other_prev_trace_line.clone().unwrap();
            let mut check_line_arr: Vec<TLine> = Vec::with_capacity(n + 1);
            if at_start {
                check_line_arr.push(other_prev_trace_line);
                check_line_arr.push(other_trace_line.clone());
                check_line_arr.extend(trace_tp.tlines_range(1..n));
            } else {
                check_line_arr.extend(trace_tp.tlines_range(0..n - 1));
                check_line_arr.push(other_trace_line.clone());
                check_line_arr.push(other_prev_trace_line);
            }
            let no = if at_start { 2 } else { n as i32 - 2 };
            if let Some(new_line) = self.reposition_line(board, &check_line_arr, no) {
                let mut new_lines: Vec<TLine> = Vec::with_capacity(n);
                if at_start {
                    new_lines.push(other_trace_line);
                    new_lines.push(new_line);
                    new_lines.extend(trace_tp.tlines_range(2..n));
                } else {
                    new_lines.extend(trace_tp.tlines_range(0..n - 2));
                    new_lines.push(new_line);
                    new_lines.push(other_trace_line);
                }
                return Some(TPolyline::from_lines(&mut new_lines));
            }
        }
        None
    }
}

/// The contact trace found by the smoothen functions.
#[derive(Clone, Debug, Default)]
pub(crate) struct ContactFound {
    pub acute_angle: bool,
    pub bend: bool,
    pub other_trace_corner_approx: Option<FloatPoint>,
    pub other_trace_line: Option<TLine>,
    pub other_prev_trace_line: Option<TLine>,
    pub prev_corner_side: Option<Side>,
}
