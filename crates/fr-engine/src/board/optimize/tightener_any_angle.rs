//! Port of `board/optimize/TraceTightenerAnyAngle.java`.

use fr_geom::limits::SQRT2;
use fr_geom::{Direction, FloatPoint, Line, Point, Polyline, Side, Signum};

use super::super::item::ItemKey;
use super::super::routing_board::{jmax, jmin, RoutingBoard};
use super::tightener45::ContactFound;
use super::trace_tightener::{TraceTightener, C_MAX_COS_ANGLE};
use super::tracked::{TLine, TPolyline};

/// Java `SKIP_LENGTH`.
const SKIP_LENGTH: f64 = 10.0;

impl TraceTightener {
    /// Java `TraceTightenerAnyAngle.pullTight(polyline)`.
    pub(crate) fn pull_tight_any_angle(&mut self, board: &mut RoutingBoard, polyline: &TPolyline) -> TPolyline {
        let mut new_result = self.avoid_acid_traps(polyline);
        let mut prev_result: Option<TPolyline> = None;
        while prev_result.as_ref().map(|p| !new_result.same(p)).unwrap_or(true) && !self.is_stop_requested() {
            prev_result = Some(new_result.clone());
            let tmp = self.skip_segments_of_length0(board, &new_result);
            let tmp0 = self.reduce_lines_any_angle(board, &tmp);
            let tmp1 = self.skip_lines_any_angle(board, &tmp0);
            let tmp2 = self.reduce_corners_any_angle(board, &tmp1);
            let tmp3 = self.reposition_lines(board, &tmp2);
            new_result = self.smoothen_corners_any_angle(board, &tmp3);
        }
        new_result
    }

    /// Java `reduceCorners(polyline)`: replaces two consecutive lines by a line through
    /// IntPoints near the previous and the next corner.
    fn reduce_corners_any_angle(&mut self, board: &mut RoutingBoard, polyline: &TPolyline) -> TPolyline {
        let len = polyline.len();
        if len < 4 {
            return polyline.clone();
        }
        let p = polyline.polyline.clone();
        let last_index = len as i32 - 4;
        let mut new_lines: Vec<Option<TLine>> = vec![None; len];
        new_lines[0] = Some(polyline.tline(0));
        new_lines[1] = Some(polyline.tline(1));
        let mut new_line_index: usize = 1;
        let mut polyline_changed = false;
        let mut current_lines: Vec<TLine> = vec![polyline.tline(0), polyline.tline(0), polyline.tline(0)];
        let nl = |v: &Vec<Option<TLine>>, k: usize| -> TLine { v[k].clone().unwrap() };
        for i in 0..=last_index {
            let iu = i as usize;
            let mut skip_line = false;
            let new_a = nl(&new_lines, new_line_index - 1).line.intersection_approx(&nl(&new_lines, new_line_index).line);
            let new_b = p.corner_approx(i + 2);
            let in_clip_shape = self.current_clip_shape.is_none()
                || (self.clip_contains(&new_a) && self.clip_contains(&new_b) && self.clip_contains(&p.corner_approx(new_line_index as i32)));
            if in_clip_shape {
                current_lines[1] = self.ids.line(Line::new(Point::Int(new_a.round()), Point::Int(new_b.round())));
                let mut ok = true;
                if new_line_index == 1 {
                    if !p.first_corner().is_int_point() {
                        // first corner must not be changed
                        ok = false;
                    } else {
                        let dir = current_lines[1].line.direction();
                        current_lines[0] = self.ids.line(Line::get_instance(p.first_corner(), &dir.turn_45_degree(2)));
                    }
                } else {
                    current_lines[0] = nl(&new_lines, new_line_index - 1);
                }
                if i == last_index {
                    if !p.last_corner().is_int_point() {
                        // last corner must not be changed
                        ok = false;
                    } else {
                        let dir = current_lines[1].line.direction();
                        current_lines[2] = self.ids.line(Line::get_instance(p.last_corner(), &dir.turn_45_degree(2)));
                    }
                } else {
                    current_lines[2] = polyline.tline(iu + 3);
                }
                // check, if the intersection of current_lines[0] and current_lines[1] is near
                // new_a and the intersection of current_lines[1] and current_lines[2] is near
                // new_b (numerical stability problems with near parallel lines).
                const CHECK_DIST: f64 = 100.0;
                if ok {
                    let check_is = current_lines[0].line.intersection_approx(&current_lines[1].line);
                    if check_is.distance_square(&new_a) > CHECK_DIST {
                        ok = false;
                    }
                }
                if ok {
                    let check_is = current_lines[1].line.intersection_approx(&current_lines[2].line);
                    if check_is.distance_square(&new_b) > CHECK_DIST {
                        ok = false;
                    }
                }
                if ok && i == 1 && !p.first_corner().is_int_point() {
                    // There may be a connection to a trace. Make sure that the second corner of
                    // the new polyline is on the same side of the trace as the third corner.
                    let new_corner = current_lines[0].line.intersection(&current_lines[1].line);
                    let l0 = nl(&new_lines, 0).line;
                    if new_corner.side_of_line(&l0) != p.corner(1).side_of_line(&l0) {
                        ok = false;
                    }
                }
                if ok && i == last_index - 1 && !p.last_corner().is_int_point() {
                    // There may be a connection to a trace. Make sure that the second last corner
                    // of the new polyline is on the same side of the trace as the third last.
                    let new_corner = current_lines[1].line.intersection(&current_lines[2].line);
                    let l0 = nl(&new_lines, 0).line;
                    if new_corner.side_of_line(&l0) != p.corner(p.corner_count() - 2).side_of_line(&l0) {
                        ok = false;
                    }
                }
                let mut current_polyline: Option<TPolyline> = None;
                if ok {
                    let skip_corner = nl(&new_lines, new_line_index).line.intersection_approx(&p.lines[iu + 2]);
                    let cp = TPolyline::from_lines(&mut current_lines);
                    if cp.len() != 3 {
                        ok = false;
                    }
                    let length_before = skip_corner.distance(&new_a) + skip_corner.distance(&new_b);
                    // 1.5 added because of possible inaccuracy SQRT_2 by twice rounding.
                    let length_after = cp.polyline.length_approx() + 1.5;
                    if length_after >= length_before {
                        // May happen from rounding to integer. Prevent infinite loop.
                        ok = false;
                    }
                    current_polyline = Some(cp);
                }
                if ok {
                    skip_line = self.check_offset_shape(board, &current_polyline.as_ref().unwrap().polyline, 0);
                }
            }
            if skip_line {
                polyline_changed = true;
                new_lines[new_line_index] = Some(current_lines[1].clone());
                if new_line_index == 1 {
                    // make the first line perpendicular to the current line
                    new_lines[0] = Some(current_lines[0].clone());
                }
                if i == last_index {
                    // make the last line perpendicular to the current line
                    new_line_index += 1;
                    new_lines[new_line_index] = Some(current_lines[2].clone());
                }
                if board.changed_area.is_some() {
                    self.join(board, &new_a);
                    self.join(board, &new_b);
                }
            } else {
                new_line_index += 1;
                new_lines[new_line_index] = Some(polyline.tline(iu + 2));
                if i == last_index {
                    new_line_index += 1;
                    new_lines[new_line_index] = Some(polyline.tline(iu + 3));
                }
            }
            if nl(&new_lines, new_line_index).line.is_parallel(&nl(&new_lines, new_line_index - 1).line) {
                // skip line, if it is parallel to the previous one
                new_line_index -= 1;
            }
        }
        if !polyline_changed {
            return polyline.clone();
        }
        let mut cleaned: Vec<TLine> = new_lines[..=new_line_index].iter().map(|l| l.clone().unwrap()).collect();
        TPolyline::from_lines(&mut cleaned)
    }

    /// Java `smoothenCorners(polyline)`: cuts off corners where possible.
    fn smoothen_corners_any_angle(&mut self, board: &mut RoutingBoard, polyline: &TPolyline) -> TPolyline {
        if polyline.len() < 4 {
            return polyline.clone();
        }
        let mut polyline_changed = false;
        let mut lines = polyline.tlines();
        let mut i = 0usize;
        while i + 3 < lines.len() {
            if let Some(new_line) = self.smoothen_corner_any_angle(board, &lines, i) {
                polyline_changed = true;
                // add the new line into the line array
                lines.insert(i + 2, new_line);
                i += 1;
            }
            i += 1;
        }
        if !polyline_changed {
            return polyline.clone();
        }
        TPolyline::from_lines(&mut lines)
    }

    /// Java `TraceTightenerAnyAngle.repositionLines(polyline)`.
    pub(crate) fn reposition_lines_any_angle(&mut self, board: &mut RoutingBoard, polyline: &TPolyline) -> TPolyline {
        if polyline.len() < 5 {
            return polyline.clone();
        }
        let mut polyline_changed = false;
        let mut lines = polyline.tlines();
        let mut i = 0usize;
        while i + 4 < lines.len() {
            if let Some(new_line) = self.reposition_line_any_angle(board, &lines, i as i32) {
                polyline_changed = true;
                lines[i + 2] = new_line;
                if lines[i + 2].line.is_parallel(&lines[i + 1].line) || lines[i + 2].line.is_parallel(&lines[i + 3].line) {
                    // calculation of corners not possible before skipping parallel lines
                    break;
                }
            }
            i += 1;
        }
        if !polyline_changed {
            return polyline.clone();
        }
        TPolyline::from_lines(&mut lines)
    }

    /// Java `reduceLines(polyline)`: moves lines parallel beyond the intersection of the next or
    /// previous lines to reduce the number of lines.
    fn reduce_lines_any_angle(&mut self, board: &mut RoutingBoard, polyline: &TPolyline) -> TPolyline {
        if polyline.len() < 6 {
            return polyline.clone();
        }
        let mut polyline_changed = false;
        let mut lines: Vec<TLine> = polyline.tlines();
        let mut i: i64 = 2;
        while i < lines.len() as i64 - 2 {
            let iu = i as usize;
            let prev_corner = lines[iu - 2].line.intersection_approx(&lines[iu - 1].line);
            let next_corner = lines[iu + 1].line.intersection_approx(&lines[iu + 2].line);
            let in_clip_shape = self.clip_contains(&prev_corner) && self.clip_contains(&next_corner);
            if !in_clip_shape {
                i += 1;
                continue;
            }
            let translate_line = lines[iu].line.clone();
            let prev_dist = translate_line.signed_distance(&prev_corner);
            let next_dist = translate_line.signed_distance(&next_corner);
            if Signum::of(prev_dist) != Signum::of(next_dist) {
                // the 2 corners are on different sides of the translate_line
                i += 1;
                continue;
            }
            let mut translate_dist = if prev_dist.abs() < next_dist.abs() { prev_dist } else { next_dist };
            if translate_dist == 0.0 {
                // line segment may have length 0
                i += 1;
                continue;
            }
            let line_side = translate_line.side_of_float(&prev_corner);
            let mut new_line = translate_line.translate(-translate_dist);
            // make sure, we have crossed the nearest corner;
            let sign = Signum::as_int(translate_dist);
            let mut new_line_side_of_prev_corner = new_line.side_of_float(&prev_corner);
            let mut new_line_side_of_next_corner = new_line.side_of_float(&next_corner);
            while new_line_side_of_prev_corner == line_side && new_line_side_of_next_corner == line_side {
                translate_dist += sign as f64 * 0.5;
                new_line = translate_line.translate(-translate_dist);
                new_line_side_of_prev_corner = new_line.side_of_float(&prev_corner);
                new_line_side_of_next_corner = new_line.side_of_float(&next_corner);
            }
            let mut crossed_corners_before_count = 0usize;
            let mut crossed_corners_after_count = 0usize;
            if new_line_side_of_prev_corner != line_side {
                crossed_corners_before_count += 1;
            }
            if new_line_side_of_next_corner != line_side {
                crossed_corners_after_count += 1;
            }
            // check, that we haven't crossed both corners
            if crossed_corners_before_count > 1 || crossed_corners_after_count > 1 {
                i += 1;
                continue;
            }
            // check, that next_nearest_corner and nearest_corner are on different sides of
            // new_line;
            if crossed_corners_before_count > 0 {
                if i < 3 {
                    i += 1;
                    continue;
                }
                let prev_prev_corner = lines[iu - 3].line.intersection_approx(&lines[iu - 2].line);
                if new_line.side_of_float(&prev_prev_corner) != line_side {
                    i += 1;
                    continue;
                }
            }
            if crossed_corners_after_count > 0 {
                if i >= lines.len() as i64 - 3 {
                    i += 1;
                    continue;
                }
                let next_next_corner = lines[iu + 2].line.intersection_approx(&lines[iu + 3].line);
                if new_line.side_of_float(&next_next_corner) != line_side {
                    i += 1;
                    continue;
                }
            }
            let new_len = lines.len() - crossed_corners_before_count - crossed_corners_after_count;
            let keep_before_ind = iu - crossed_corners_before_count;
            let mut current_lines: Vec<TLine> = Vec::with_capacity(new_len);
            current_lines.extend(lines[..keep_before_ind].iter().cloned());
            current_lines.push(self.ids.line(new_line));
            let rest_from = iu + 1 + crossed_corners_after_count;
            current_lines.extend(lines[rest_from..rest_from + (new_len - (keep_before_ind + 1))].iter().cloned());
            let tmp = TPolyline::from_lines(&mut current_lines);
            let mut check_ok = false;
            if tmp.len() == current_lines.len() {
                check_ok = self.check_offset_shape(board, &tmp.polyline, keep_before_ind as i32 - 1);
            }
            if check_ok {
                if board.changed_area.is_some() {
                    self.join(board, &prev_corner);
                    self.join(board, &next_corner);
                }
                polyline_changed = true;
                lines = current_lines;
                i -= 1;
            }
            i += 1;
        }
        if !polyline_changed {
            return polyline.clone();
        }
        TPolyline::from_lines(&mut lines)
    }

    /// Java `TraceTightenerAnyAngle.smoothenCorner(lines, startNo)`.
    fn smoothen_corner_any_angle(&mut self, board: &mut RoutingBoard, lines: &[TLine], start_no: usize) -> Option<TLine> {
        if lines.len() < start_no + 4 {
            return None;
        }
        let current_corner = lines[start_no + 1].line.intersection_approx(&lines[start_no + 2].line);
        if !self.clip_contains(&current_corner) {
            return None;
        }
        let cosinus_angle = lines[start_no + 1].line.cos_angle(&lines[start_no + 2].line);
        if cosinus_angle > C_MAX_COS_ANGLE {
            // lines are already nearly parallel, don't divide angle any further because of
            // problems with numerical stability
            return None;
        }
        let prev_corner = lines[start_no].line.intersection_approx(&lines[start_no + 1].line);
        let next_corner = lines[start_no + 2].line.intersection_approx(&lines[start_no + 3].line);
        // create a line approximately through current_corner, whose direction is about the
        // middle of the directions of the previous and the next line. Translations of this line
        // are used to cut off the corner.
        let prev_dir = lines[start_no + 1].line.direction();
        let next_dir = lines[start_no + 2].line.direction();
        let middle_dir = prev_dir.middle_approx(&next_dir);
        let translate_line = Line::get_instance(Point::Int(current_corner.round()), &middle_dir);
        let prev_dist = translate_line.signed_distance(&prev_corner);
        let next_dist = translate_line.signed_distance(&next_corner);
        let (nearest_point, mut max_translate_dist) =
            if prev_dist.abs() < next_dist.abs() { (prev_corner, prev_dist) } else { (next_corner, next_dist) };
        if max_translate_dist.abs() < 1.0 {
            return None;
        }
        let mut current_lines: Vec<TLine> = Vec::with_capacity(lines.len() + 1);
        current_lines.extend(lines[..start_no + 2].iter().cloned());
        current_lines.push(lines[start_no + 2].clone()); // placeholder, replaced below
        current_lines.extend(lines[start_no + 2..].iter().cloned());
        let mut translate_dist = max_translate_dist;
        let mut delta_dist = max_translate_dist;
        let side_of_nearest_point = translate_line.side_of_float(&nearest_point);
        let sign = Signum::as_int(max_translate_dist);
        let mut result: Option<TLine> = None;
        while delta_dist.abs() > self.min_translate_dist as f64 {
            let mut check_ok = false;
            let new_line = translate_line.translate(-translate_dist);
            let new_line_side = new_line.side_of_float(&nearest_point);
            if new_line_side == side_of_nearest_point || new_line_side == Side::Collinear {
                current_lines[start_no + 2] = self.ids.line(new_line);
                let expected = current_lines.len();
                let tmp = TPolyline::from_lines(&mut current_lines);
                if tmp.len() == expected {
                    check_ok = self.check_offset_shape(board, &tmp.polyline, start_no as i32 + 1);
                }
                delta_dist /= 2.0;
                if check_ok {
                    result = Some(current_lines[start_no + 2].clone());
                    if translate_dist == max_translate_dist {
                        // biggest possible change
                        break;
                    }
                    translate_dist += delta_dist;
                } else {
                    translate_dist -= delta_dist;
                }
            } else {
                // moved a little bit too far at the first time because of numerical inaccuracy
                let shorten_value = sign as f64 * 0.5;
                max_translate_dist -= shorten_value;
                translate_dist -= shorten_value;
                delta_dist -= shorten_value;
            }
        }
        let result = result?;
        if board.changed_area.is_some() {
            let new_prev_corner = current_lines[start_no].line.intersection_approx(&current_lines[start_no + 1].line);
            let new_next_corner = current_lines[start_no + 3].line.intersection_approx(&current_lines[start_no + 4].line);
            self.join(board, &new_prev_corner);
            self.join(board, &new_next_corner);
        }
        Some(result)
    }

    /// Java `TraceTightenerAnyAngle.repositionLine(lines, startNo)`.
    pub(crate) fn reposition_line_any_angle(&mut self, board: &mut RoutingBoard, lines: &[TLine], start_no: i32) -> Option<TLine> {
        if lines.len() as i32 - start_no < 5 {
            return None;
        }
        let s = start_no as usize;
        if self.current_clip_shape.is_some() {
            // check, that the corners of the line to translate are inside the clip shape
            for i in 1..3 {
                let current_corner = lines[s + i].line.intersection_approx(&lines[s + i + 1].line);
                if !self.clip_contains(&current_corner) {
                    return None;
                }
            }
        }
        let translate_line = lines[s + 2].line.clone();
        let mut prev_corner = lines[s].line.intersection_approx(&lines[s + 1].line);
        let mut next_corner = lines[s + 3].line.intersection_approx(&lines[s + 4].line);
        let mut prev_dist = translate_line.signed_distance(&prev_corner);
        let mut corners_skipped_before = 0i32;
        let mut corners_skipped_after = 0i32;
        const EPSILON: f64 = 0.001;
        while prev_dist.abs() < EPSILON {
            // move also all lines through the start corner of the line to translate
            corners_skipped_before += 1;
            let current_no = start_no - corners_skipped_before;
            if current_no < 0 {
                // the first corner is on the line to translate
                return None;
            }
            prev_corner = lines[current_no as usize].line.intersection_approx(&lines[current_no as usize + 1].line);
            prev_dist = translate_line.signed_distance(&prev_corner);
        }
        let mut next_dist = translate_line.signed_distance(&next_corner);
        while next_dist.abs() < EPSILON {
            // move also all lines through the end corner of the line to translate
            corners_skipped_after += 1;
            let current_no = start_no + 3 + corners_skipped_after;
            if current_no >= lines.len() as i32 - 2 {
                // the last corner is on the line to translate
                return None;
            }
            next_corner = lines[current_no as usize].line.intersection_approx(&lines[current_no as usize + 1].line);
            next_dist = translate_line.signed_distance(&next_corner);
        }
        if Signum::of(prev_dist) != Signum::of(next_dist) {
            // the 2 corners are at different sides of translate_line
            return None;
        }
        let (nearest_point, mut max_translate_dist) =
            if prev_dist.abs() < next_dist.abs() { (prev_corner, prev_dist) } else { (next_corner, next_dist) };
        let mut current_lines: Vec<TLine> = lines.to_vec();
        let mut translate_dist = max_translate_dist;
        let mut delta_dist = max_translate_dist;
        let side_of_nearest_point = translate_line.side_of_float(&nearest_point);
        let sign = Signum::as_int(max_translate_dist);
        let mut result: Option<TLine> = None;
        let mut first_time = true;
        while first_time || delta_dist.abs() > self.min_translate_dist as f64 {
            let mut check_ok = false;
            let mut new_line = translate_line.translate(-translate_dist);
            if first_time && translate_dist.abs() < 1.0 {
                if new_line == translate_line {
                    // try the parallel line through the nearest point
                    let rounded_nearest_point = nearest_point.round();
                    if nearest_point.distance(&rounded_nearest_point.to_float()) < translate_dist.abs() {
                        new_line = Line::get_instance(Point::Int(rounded_nearest_point), &translate_line.direction());
                    }
                    first_time = false;
                }
                if new_line == translate_line {
                    return None;
                }
            }
            let new_line_side = new_line.side_of_float(&nearest_point);
            if new_line_side == side_of_nearest_point || new_line_side == Side::Collinear {
                first_time = false;
                current_lines[s + 2] = self.ids.line(new_line.clone());
                // corners_skipped_before > 0 or corners_skipped_after > 0 happens very rarely.
                // But this handling seems to be important because there are situations which no
                // other tightening function can solve. For example when 3 or more consecutive
                // corners are equal.
                let mut prev_translated_line = new_line.clone();
                for i in 0..corners_skipped_before {
                    // Translate the previous lines onto or past the intersection of new_line with
                    // the first untranslated line.
                    let prev_line_no = (start_no + 1 - corners_skipped_before) as usize;
                    let current_prev_corner = prev_translated_line.intersection_approx(&current_lines[prev_line_no].line);
                    let current_translate_line = &lines[(start_no + 1 - i) as usize].line;
                    let current_translate_dist = current_translate_line.signed_distance(&current_prev_corner);
                    prev_translated_line = current_translate_line.translate(-current_translate_dist);
                    current_lines[(start_no + 1 - i) as usize] = self.ids.line(prev_translated_line.clone());
                }
                prev_translated_line = new_line.clone();
                for i in 0..corners_skipped_after {
                    // Translate the next lines onto or past the intersection of new_line with the
                    // first untranslated line.
                    let next_line_no = (start_no + 3 + corners_skipped_after) as usize;
                    let current_next_corner = prev_translated_line.intersection_approx(&current_lines[next_line_no].line);
                    let current_translate_line = &lines[(start_no + 3 + i) as usize].line;
                    let current_translate_dist = current_translate_line.signed_distance(&current_next_corner);
                    prev_translated_line = current_translate_line.translate(-current_translate_dist);
                    current_lines[(start_no + 3 + i) as usize] = self.ids.line(prev_translated_line.clone());
                }
                let expected = current_lines.len();
                let tmp = TPolyline::from_lines(&mut current_lines);
                if tmp.len() == expected {
                    check_ok = self.check_offset_shape(board, &tmp.polyline, start_no + 1);
                }
                delta_dist /= 2.0;
                if check_ok {
                    result = Some(current_lines[s + 2].clone());
                    if translate_dist == max_translate_dist {
                        // biggest possible change
                        break;
                    }
                    translate_dist += delta_dist;
                } else {
                    translate_dist -= delta_dist;
                }
            } else {
                // moved a little bit too far at the first time because of numerical inaccuracy
                let shorten_value = sign as f64 * 0.5;
                max_translate_dist -= shorten_value;
                translate_dist -= shorten_value;
                delta_dist -= shorten_value;
            }
        }
        let result = result?;
        if board.changed_area.is_some() {
            let new_prev_corner = current_lines[s].line.intersection_approx(&current_lines[s + 1].line);
            let new_next_corner = current_lines[s + 3].line.intersection_approx(&current_lines[s + 4].line);
            self.join(board, &new_prev_corner);
            self.join(board, &new_next_corner);
        }
        Some(result)
    }

    /// Java `skipLines(polyline)`.
    fn skip_lines_any_angle(&mut self, board: &mut RoutingBoard, polyline: &TPolyline) -> TPolyline {
        let p = polyline.polyline.clone();
        let len = p.lines.len() as i32;
        let mut i = 1;
        while i < len - 3 {
            for j in 0..=1 {
                let (mut current_line, corner1, corner2) = if j == 0 {
                    // try to skip the line before the i+2-th line
                    (p.lines[(i + 2) as usize].clone(), p.corner_approx(i), p.corner_approx(i - 1))
                } else {
                    // try to skip the line after i-th line
                    (p.lines[i as usize].clone(), p.corner_approx(i + 1), p.corner_approx(i + 2))
                };
                let in_clip_shape = self.clip_contains(&corner1) && self.clip_contains(&corner2);
                if !in_clip_shape {
                    continue;
                }
                let mut side1 = current_line.side_of_float(&corner1);
                let side2 = current_line.side_of_float(&corner2);
                if side1 != side2 {
                    // the two corners are on different sides of the line
                    let reduced = polyline.skip_lines(i + 1, i + 1);
                    if reduced.len() as i32 == len - 1 {
                        let mut shape_index = i - 1;
                        if j == 0 {
                            shape_index += 1;
                        }
                        if self.check_offset_shape(board, &reduced.polyline, shape_index) {
                            if board.changed_area.is_some() {
                                self.join(board, &corner1);
                                self.join(board, &corner2);
                            }
                            return reduced;
                        }
                    }
                }
                // now try skipping 2 lines
                if i >= len - 4 {
                    break;
                }
                let corner3 = if j == 1 { p.corner_approx(i + 3) } else { p.corner_approx(i + 1) };
                if !self.clip_contains(&corner3) {
                    continue;
                }
                let side2 = if j == 0 {
                    // current_line is 1 line later than in the case skipping 1 line when coming
                    // from behind
                    current_line = p.lines[(i + 3) as usize].clone();
                    side1 = current_line.side_of_float(&corner1);
                    current_line.side_of_float(&corner2)
                } else {
                    side1 = current_line.side_of_float(&corner3);
                    side2
                };
                if side1 != side2 {
                    // the two corners are on different sides of the line
                    let reduced = polyline.skip_lines(i + 1, i + 2);
                    if reduced.len() as i32 == len - 2 {
                        let mut shape_index = i - 1;
                        if j == 0 {
                            shape_index += 1;
                        }
                        if self.check_offset_shape(board, &reduced.polyline, shape_index) {
                            if board.changed_area.is_some() {
                                self.join(board, &corner1);
                                self.join(board, &corner2);
                                self.join(board, &corner3);
                            }
                            return reduced;
                        }
                    }
                }
            }
            i += 1;
        }
        polyline.clone()
    }

    /// Java `TraceTightenerAnyAngle.smoothenStartCornerAtTrace(trace)`.
    pub(crate) fn smoothen_start_corner_at_trace_any_angle(&mut self, board: &mut RoutingBoard, trace: ItemKey) -> Option<TPolyline> {
        let trace_tp = board.item(trace).trace().tpolyline();
        let trace_polyline = trace_tp.polyline.clone();
        let current_end_corner = trace_polyline.corner(0);
        if self.clip_is_outside(&current_end_corner) {
            return None;
        }
        let mut current_prev_end_corner = trace_polyline.corner(1);
        let skip_short_segment = !current_end_corner.is_int_point()
            && current_end_corner.to_float().distance_square(&current_prev_end_corner.to_float()) < SKIP_LENGTH;
        let mut start_line_no = 1usize;
        if skip_short_segment {
            if trace_polyline.corner_count() < 3 {
                return None;
            }
            current_prev_end_corner = trace_polyline.corner(2);
            start_line_no += 1;
        }
        let line_direction = trace_polyline.lines[start_line_no].direction();
        let prev_line_direction = trace_polyline.lines[start_line_no + 1].direction();
        let contacts = board.trace_start_contacts(trace);
        let found = self.find_contact_trace_any_angle(board, &contacts, &trace_polyline, &current_end_corner, &current_prev_end_corner, &line_direction, &prev_line_direction, false)?;
        let n = trace_polyline.lines.len();
        let mut new_line_count = n + 1;
        let mut diff = 1usize;
        if skip_short_segment {
            new_line_count -= 1;
            diff -= 1;
        }
        if found.acute_angle {
            let other_trace_line = found.other_trace_line.clone().unwrap();
            let factor = if found.prev_corner_side == Some(Side::OnTheLeft) { 2 } else { 6 };
            let add_line = self.smoothen_add_line(&other_trace_line.line, factor, &current_end_corner, &current_prev_end_corner, found.other_trace_corner_approx.as_ref().unwrap())?;
            // construct the new trace polyline.
            let mut new_lines: Vec<TLine> = Vec::with_capacity(new_line_count);
            new_lines.push(other_trace_line.clone());
            new_lines.push(TLine::fresh(add_line));
            new_lines.extend(trace_tp.tlines_range(2 - diff..2 - diff + (new_line_count - 2)));
            return Some(TPolyline::from_lines(&mut new_lines));
        } else if found.bend {
            let other_trace_line = found.other_trace_line.clone().unwrap();
            let mut check_line_arr: Vec<TLine> = Vec::with_capacity(new_line_count);
            check_line_arr.push(found.other_prev_trace_line.clone().unwrap());
            check_line_arr.push(other_trace_line.clone());
            check_line_arr.extend(trace_tp.tlines_range(2 - diff..2 - diff + (new_line_count - 2)));
            if let Some(new_line) = self.reposition_line(board, &check_line_arr, 0) {
                let mut new_lines: Vec<TLine> = Vec::with_capacity(n);
                new_lines.push(other_trace_line.clone());
                new_lines.push(new_line);
                new_lines.extend(trace_tp.tlines_range(2..n));
                return Some(TPolyline::from_lines(&mut new_lines));
            }
        }
        None
    }

    /// Java `TraceTightenerAnyAngle.smoothenEndCornerAtTrace(trace)`.
    pub(crate) fn smoothen_end_corner_at_trace_any_angle(&mut self, board: &mut RoutingBoard, trace: ItemKey) -> Option<TPolyline> {
        let trace_tp = board.item(trace).trace().tpolyline();
        let trace_polyline = trace_tp.polyline.clone();
        let current_end_corner = trace_polyline.last_corner();
        if self.clip_is_outside(&current_end_corner) {
            return None;
        }
        let mut current_prev_end_corner = trace_polyline.corner(trace_polyline.corner_count() - 2);
        let skip_short_segment = !current_end_corner.is_int_point()
            && current_end_corner.to_float().distance_square(&current_prev_end_corner.to_float()) < SKIP_LENGTH;
        let n = trace_polyline.lines.len();
        let mut end_line_no = n - 2;
        if skip_short_segment {
            if trace_polyline.corner_count() < 3 {
                return None;
            }
            current_prev_end_corner = trace_polyline.corner(trace_polyline.corner_count() - 3);
            end_line_no -= 1;
        }
        let line_direction = trace_polyline.lines[end_line_no].direction().opposite();
        // (Java uses the same line for the previous line direction)
        let prev_line_direction = trace_polyline.lines[end_line_no].direction().opposite();
        let contacts = board.trace_end_contacts(trace);
        let found = self.find_contact_trace_any_angle(board, &contacts, &trace_polyline, &current_end_corner, &current_prev_end_corner, &line_direction, &prev_line_direction, true)?;
        let mut new_line_count = n + 1;
        let mut diff = 0usize;
        if skip_short_segment {
            new_line_count -= 1;
            diff += 1;
        }
        if found.acute_angle {
            let other_trace_line = found.other_trace_line.clone().unwrap();
            let factor = if found.prev_corner_side == Some(Side::OnTheLeft) { 6 } else { 2 };
            let add_line = self.smoothen_add_line(&other_trace_line.line, factor, &current_end_corner, &current_prev_end_corner, found.other_trace_corner_approx.as_ref().unwrap())?;
            // construct the new trace polyline.
            let mut new_lines: Vec<Option<TLine>> = vec![None; new_line_count];
            for (k, slot) in new_lines.iter_mut().enumerate().take(n - 1) {
                *slot = Some(trace_tp.tline(k));
            }
            new_lines[new_line_count - 2] = Some(TLine::fresh(add_line));
            new_lines[new_line_count - 1] = Some(other_trace_line.clone());
            let mut new_lines: Vec<TLine> = new_lines.into_iter().map(|l| l.expect("line")).collect();
            return Some(TPolyline::from_lines(&mut new_lines));
        } else if found.bend {
            let other_trace_line = found.other_trace_line.clone().unwrap();
            let mut check_line_arr: Vec<TLine> = Vec::with_capacity(new_line_count);
            check_line_arr.extend(trace_tp.tlines_range(diff..diff + (new_line_count - 2)));
            check_line_arr.push(other_trace_line.clone());
            check_line_arr.push(found.other_prev_trace_line.clone().unwrap());
            let no = check_line_arr.len() as i32 - 5;
            if let Some(new_line) = self.reposition_line(board, &check_line_arr, no) {
                let mut new_lines: Vec<TLine> = Vec::with_capacity(n);
                new_lines.extend(trace_tp.tlines_range(0..n - 2));
                new_lines.push(new_line);
                new_lines.push(other_trace_line.clone());
                return Some(TPolyline::from_lines(&mut new_lines));
            }
        }
        None
    }

    /// The acute angle line of the any angle `smoothenStart/EndCornerAtTrace` (`None` if the
    /// translate distance is below 0.99).
    fn smoothen_add_line(&self, other_trace_line: &Line, factor: i32, current_end_corner: &Point, current_prev_end_corner: &Point, other_trace_corner_approx: &FloatPoint) -> Option<Line> {
        let new_line_dir: Direction = other_trace_line.direction().turn_45_degree(factor);
        let translate_line = Line::get_instance(Point::Int(current_end_corner.to_float().round()), &new_line_dir);
        let mut translate_dist = (SQRT2 - 1.0) * self.current_half_width as f64;
        let prev_corner_dist = translate_line.signed_distance(&current_prev_end_corner.to_float()).abs();
        let other_dist = translate_line.signed_distance(other_trace_corner_approx).abs();
        translate_dist = jmin(translate_dist, prev_corner_dist);
        translate_dist = jmin(translate_dist, other_dist);
        if translate_dist < 0.99 {
            return None;
        }
        translate_dist = jmax(translate_dist - 1.0, 1.0);
        if translate_line.side_of(current_prev_end_corner) == Side::OnTheLeft {
            translate_dist = -translate_dist;
        }
        Some(translate_line.translate(translate_dist))
    }

    /// The contact loop of the any angle `smoothenStart/EndCornerAtTrace` (`at_end`: the end
    /// version ignores contact traces with at most 2 corners).
    #[allow(clippy::too_many_arguments)]
    fn find_contact_trace_any_angle(
        &mut self,
        board: &RoutingBoard,
        contacts: &super::super::item_list::ItemSet,
        trace_polyline: &Polyline,
        current_end_corner: &Point,
        current_prev_end_corner: &Point,
        line_direction: &Direction,
        prev_line_direction: &Direction,
        at_end: bool,
    ) -> Option<ContactFound> {
        let mut found = ContactFound::default();
        for contact in contacts.iter() {
            let item = board.item(contact);
            if item.is_trace() && !board.is_shove_fixed(contact) {
                let contact_tp = item.trace().tpolyline();
                if at_end && contact_tp.polyline.corner_count() <= 2 {
                    continue;
                }
                let (corner_approx, other_line, other_prev_line) = self.contact_trace_lines(&contact_tp, current_end_corner);
                let current_prev_corner_side = current_prev_end_corner.side_of_line(&other_line.line);
                let current_projection = line_direction.projection(&other_line.line.direction());
                let mut other_trace_found = false;
                if current_projection == Signum::Positive && current_prev_corner_side != Side::Collinear {
                    found.acute_angle = true;
                    other_trace_found = true;
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
}
