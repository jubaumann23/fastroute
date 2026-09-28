//! Port of `board/optimize/TraceTightener90.java` (the smoothen functions return null in Java,
//! see [`TraceTightener::smoothen_end_corners_at_trace`]).

use super::super::routing_board::RoutingBoard;
use super::trace_tightener::TraceTightener;
use super::tracked::{TLine, TPolyline};

impl TraceTightener {
    /// Java `TraceTightener90.pullTight(polyline)`.
    pub(crate) fn pull_tight_90(&mut self, board: &mut RoutingBoard, polyline: &TPolyline) -> TPolyline {
        let mut new_result = self.avoid_acid_traps(polyline);
        let mut prev_result: Option<TPolyline> = None;
        while prev_result.as_ref().map(|p| !new_result.same(p)).unwrap_or(true) && !self.is_stop_requested() {
            prev_result = Some(new_result.clone());
            let tmp1 = self.try_skip_second_corner(board, &new_result);
            let tmp2 = self.try_skip_corners(board, &tmp1);
            new_result = self.reposition_lines(board, &tmp2);
        }
        new_result
    }

    /// Java `trySkipSecondCorner(polyline)`.
    fn try_skip_second_corner(&mut self, board: &mut RoutingBoard, polyline: &TPolyline) -> TPolyline {
        if polyline.len() < 5 {
            return polyline.clone();
        }
        let mut check_lines: Vec<TLine> = vec![polyline.tline(1), polyline.tline(0), polyline.tline(3), polyline.tline(4)];
        let check_polyline = TPolyline::from_lines(&mut check_lines);
        if check_polyline.len() != 4 || !self.clip_contains(&check_polyline.polyline.corner_approx(1)) {
            return polyline.clone();
        }
        for i in 0..2 {
            if !self.check_offset_shape(board, &check_polyline.polyline, i) {
                return polyline.clone();
            }
        }
        // now the second corner can be skipped.
        let mut new_lines: Vec<TLine> = Vec::with_capacity(polyline.len() - 1);
        new_lines.push(polyline.tline(1));
        new_lines.push(polyline.tline(0));
        for k in 3..polyline.len() {
            new_lines.push(polyline.tline(k));
        }
        TPolyline::from_lines(&mut new_lines)
    }

    /// Java `trySkipCorners(polyline)`: tries to reduce the number of corners.
    fn try_skip_corners(&mut self, board: &mut RoutingBoard, polyline: &TPolyline) -> TPolyline {
        let len = polyline.len();
        let mut new_lines: Vec<Option<TLine>> = vec![None; len.max(2)];
        new_lines[0] = Some(polyline.tline(0));
        new_lines[1] = Some(polyline.tline(1));
        let mut new_line_index: usize = 1;
        let mut polyline_changed = false;
        let mut check_lines: Vec<TLine> = Vec::new();
        let mut second_last_corner_skipped = false;
        let mut i = 5usize;
        while i <= len {
            let mut skip_lines = false;
            let in_clip_shape = self.clip_contains(&polyline.polyline.corner_approx(i as i32 - 3));
            if in_clip_shape {
                let l3 = if i < len {
                    polyline.tline(i)
                } else {
                    // use as concluding line the second last line
                    polyline.tline(i - 2)
                };
                check_lines = vec![
                    new_lines[new_line_index - 1].clone().unwrap(),
                    new_lines[new_line_index].clone().unwrap(),
                    polyline.tline(i - 1),
                    l3,
                ];
                let check_polyline = TPolyline::from_lines(&mut check_lines);
                skip_lines = check_polyline.len() == 4 && self.clip_contains(&check_polyline.polyline.corner_approx(1));
                if skip_lines {
                    skip_lines = self.check_offset_shape(board, &check_polyline.polyline, 0);
                }
                if skip_lines {
                    skip_lines = self.check_offset_shape(board, &check_polyline.polyline, 1);
                }
            }
            if skip_lines {
                if i == len {
                    second_last_corner_skipped = true;
                }
                if board.changed_area.is_some() {
                    let new_corner = check_lines[1].line.intersection_approx(&check_lines[2].line);
                    self.join(board, &new_corner);
                    let skipped_corner = polyline.polyline.lines[i - 2].intersection_approx(&polyline.polyline.lines[i - 3]);
                    self.join(board, &skipped_corner);
                }
                polyline_changed = true;
                i += 1;
            } else {
                new_line_index += 1;
                new_lines[new_line_index] = Some(polyline.tline(i - 3));
            }
            i += 1;
        }
        if !polyline_changed {
            return polyline.clone();
        }
        if second_last_corner_skipped {
            // The second last corner of polyline was skipped
            new_line_index += 1;
            new_lines[new_line_index] = Some(polyline.tline(len - 1));
            new_line_index += 1;
            new_lines[new_line_index] = Some(polyline.tline(len - 2));
        } else {
            for k in (1..=3).rev() {
                new_line_index += 1;
                new_lines[new_line_index] = Some(polyline.tline(len - k));
            }
        }
        let mut cleaned: Vec<TLine> = new_lines[..=new_line_index].iter().map(|l| l.clone().unwrap()).collect();
        TPolyline::from_lines(&mut cleaned)
    }
}
