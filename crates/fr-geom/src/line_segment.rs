//! Port of `LineSegment.java`.
//!
//! A LineSegment starts at the intersection of `start` and `middle` and ends at the intersection
//! of `middle` and `end`. The difference to a Line is that a Line is infinite.

use std::sync::OnceLock;

use crate::float_point::{java_max, java_min, FloatPoint};
use crate::int_box::IntBox;
use crate::int_octagon::IntOctagon;
use crate::int_point::IntPoint;
use crate::java_compat::math_round_i32;
use crate::line::Line;
use crate::point::Point;
use crate::polyline::Polyline;
use crate::polyline_shape::PolylineShape;
use crate::side::Side;
use crate::signum::Signum;
use crate::simplex::Simplex;
use crate::tile_shape::TileShape;

#[derive(Clone)]
pub struct LineSegment {
    start: Line,
    middle: Line,
    end: Line,
    precalculated_start_point: OnceLock<Point>,
    precalculated_end_point: OnceLock<Point>,
}

impl std::fmt::Debug for LineSegment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LineSegment")
            .field("start", &self.start)
            .field("middle", &self.middle)
            .field("end", &self.end)
            .finish()
    }
}

impl LineSegment {
    /// Creates a line segment from the 3 input lines. start_line and end_line must not be
    /// parallel to middle_line.
    pub fn new(start_line: Line, middle_line: Line, end_line: Line) -> LineSegment {
        LineSegment {
            start: start_line,
            middle: middle_line,
            end: end_line,
            precalculated_start_point: OnceLock::new(),
            precalculated_end_point: OnceLock::new(),
        }
    }

    /// Creates the no-th line segment of polyline for no between 1 and polyline.lines.len() - 2.
    /// Returns None if no is out of range (Java logs a warning and creates a segment with null
    /// lines, which fails on first use).
    pub fn from_polyline(polyline: &Polyline, no: i32) -> Option<LineSegment> {
        let len = polyline.lines.len() as i32;
        if no <= 0 || no >= len - 1 {
            log::warn!("LineSegment from Polyline: no out of range");
            return None;
        }
        let no = no as usize;
        Some(LineSegment::new(
            polyline.lines[no - 1].clone(),
            polyline.lines[no].clone(),
            polyline.lines[no + 1].clone(),
        ))
    }

    /// Creates the no-th line segment of shape for no between 0 and shape.border_line_count() - 1.
    /// Returns None if no is out of range (see `from_polyline`).
    pub fn from_polyline_shape(shape: &PolylineShape, no: i32) -> Option<LineSegment> {
        let line_count = shape.border_line_count();
        if no < 0 || no >= line_count {
            log::warn!("LineSegment from TileShape: no out of range");
            return None;
        }
        let start = if no == 0 {
            shape.border_line(line_count - 1)
        } else {
            shape.border_line(no - 1)
        };
        let middle = shape.border_line(no);
        let end = if no == line_count - 1 {
            shape.border_line(0)
        } else {
            shape.border_line(no + 1)
        };
        Some(LineSegment::new(start, middle, end))
    }

    /// Returns the intersection of the first 2 lines of this segment.
    pub fn start_point(&self) -> Point {
        self.precalculated_start_point
            .get_or_init(|| self.middle.intersection(&self.start))
            .clone()
    }

    /// Returns the intersection of the last 2 lines of this segment.
    pub fn end_point(&self) -> Point {
        self.precalculated_end_point
            .get_or_init(|| self.middle.intersection(&self.end))
            .clone()
    }

    /// Returns an approximation of the intersection of the first 2 lines of this segment.
    pub fn start_point_approx(&self) -> FloatPoint {
        match self.precalculated_start_point.get() {
            Some(p) => p.to_float(),
            None => self.start.intersection_approx(&self.middle),
        }
    }

    /// Returns an approximation of the intersection of the last 2 lines of this segment.
    pub fn end_point_approx(&self) -> FloatPoint {
        match self.precalculated_end_point.get() {
            Some(p) => p.to_float(),
            None => self.end.intersection_approx(&self.middle),
        }
    }

    /// Returns the (infinite) line of this segment.
    pub fn get_line(&self) -> &Line {
        &self.middle
    }

    /// Returns the start closing line of this segment.
    pub fn get_start_closing_line(&self) -> &Line {
        &self.start
    }

    /// Returns the end closing line of this segment.
    pub fn get_end_closing_line(&self) -> &Line {
        &self.end
    }

    /// Returns the line segment with the opposite direction.
    pub fn opposite(&self) -> LineSegment {
        LineSegment::new(
            self.end.opposite(),
            self.middle.opposite(),
            self.start.opposite(),
        )
    }

    /// Transforms this LineSegment into a polyline of length 3.
    pub fn to_polyline(&self) -> Polyline {
        Polyline::from_lines(vec![
            self.start.clone(),
            self.middle.clone(),
            self.end.clone(),
        ])
    }

    /// Creates a 1 dimensional simplex from this line segment with the same shape.
    pub fn to_simplex(&self) -> Simplex {
        let mut lines = Vec::with_capacity(4);
        if self.end_point().side_of_line(&self.start) == Side::OnTheRight {
            lines.push(self.start.opposite());
        } else {
            lines.push(self.start.clone());
        }
        lines.push(self.middle.clone());
        lines.push(self.middle.opposite());
        if self.start_point().side_of_line(&self.end) == Side::OnTheRight {
            lines.push(self.end.opposite());
        } else {
            lines.push(self.end.clone());
        }
        Simplex::get_instance(&lines)
    }

    /// Checks if point is contained in this line segment.
    pub fn contains(&self, point: &Point) -> bool {
        if !point.is_int_point() {
            log::warn!("LineSegments.contains currently only implemented for IntPoints");
            return false;
        }
        if self.middle.side_of(point) != Side::Collinear {
            return false;
        }
        // create a perpendicular line at point and check, that the two endpoints of this
        // segment are on different sides of that line.
        let perpendicular_direction = self.middle.direction().turn_45_degree(2);
        let perpendicular_line = Line::from_point_direction(point.clone(), perpendicular_direction);
        let start_point_side = perpendicular_line.side_of(&self.start_point());
        let end_point_side = perpendicular_line.side_of(&self.end_point());
        start_point_side != end_point_side || start_point_side == Side::Collinear
    }

    /// Calculates the smallest surrounding box of this line segment.
    pub fn bounding_box(&self) -> IntBox {
        let start_corner = self.middle.intersection_approx(&self.start);
        let end_corner = self.middle.intersection_approx(&self.end);
        let llx = java_min(start_corner.x, end_corner.x);
        let lly = java_min(start_corner.y, end_corner.y);
        let urx = java_max(start_corner.x, end_corner.x);
        let ury = java_max(start_corner.y, end_corner.y);
        let lower_left = IntPoint::new(llx.floor() as i32, lly.floor() as i32);
        let upper_right = IntPoint::new(urx.ceil() as i32, ury.ceil() as i32);
        IntBox::from_points(lower_left, upper_right)
    }

    /// Calculates the smallest surrounding octagon of this line segment.
    pub fn bounding_octagon(&self) -> IntOctagon {
        let start_corner = self.middle.intersection_approx(&self.start);
        let end_corner = self.middle.intersection_approx(&self.end);
        let lx = java_min(start_corner.x, end_corner.x).floor();
        let ly = java_min(start_corner.y, end_corner.y).floor();
        let rx = java_max(start_corner.x, end_corner.x).ceil();
        let uy = java_max(start_corner.y, end_corner.y).ceil();
        let start_x_minus_y = start_corner.x - start_corner.y;
        let end_x_minus_y = end_corner.x - end_corner.y;
        let ulx = java_min(start_x_minus_y, end_x_minus_y).floor();
        let lrx = java_max(start_x_minus_y, end_x_minus_y).ceil();
        let start_x_plus_y = start_corner.x + start_corner.y;
        let end_x_plus_y = end_corner.x + end_corner.y;
        let llx = java_min(start_x_plus_y, end_x_plus_y).floor();
        let urx = java_max(start_x_plus_y, end_x_plus_y).ceil();
        let result = IntOctagon::new(
            lx as i32, ly as i32, rx as i32, uy as i32, ulx as i32, lrx as i32, llx as i32,
            urx as i32,
        );
        result.normalize()
    }

    /// Creates a new line segment with the same start and middle line and an end line, so that
    /// the length of the new line segment is about new_length.
    pub fn change_length_approx(&self, new_length: f64) -> LineSegment {
        let new_end_point = self
            .start_point_approx()
            .change_length(&self.end_point_approx(), new_length);
        let perpendicular_direction = self.middle.direction().turn_45_degree(2);
        let new_end_line =
            Line::from_point_direction(Point::Int(new_end_point.round()), perpendicular_direction);
        LineSegment::new(self.start.clone(), self.middle.clone(), new_end_line)
    }

    /// Looks up the intersections of this line segment with other. The result may have length
    /// 0, 1 or 2 (see Java doc). Intersecting lines, not points, are returned.
    pub fn intersection(&self, other: &LineSegment) -> Vec<Line> {
        if !self
            .bounding_box()
            .intersects_int_box(&other.bounding_box())
        {
            return Vec::new();
        }
        let start_point_side = self.start_point().side_of_line(&other.middle);
        let end_point_side = self.end_point().side_of_line(&other.middle);
        if start_point_side == Side::Collinear && end_point_side == Side::Collinear {
            // there may be an overlap
            let this_sorted = self.sort_endpoints_in_xy();
            let other_sorted = other.sort_endpoints_in_xy();
            let (left_line, right_line) = if this_sorted
                .start_point()
                .compare_xy(&other_sorted.start_point())
                <= 0
            {
                (this_sorted, other_sorted)
            } else {
                (other_sorted, this_sorted)
            };
            let cmp = left_line.end_point().compare_xy(&right_line.start_point());
            if cmp < 0 {
                // end point of the left line is to the left of the start point of the right line
                return Vec::new();
            }
            if cmp == 0 {
                // end point of the left line is equal to the start point of the right line
                return vec![left_line.end.clone()];
            }
            // now there is a real overlap
            let second = if right_line.end_point().compare_xy(&left_line.end_point()) >= 0 {
                left_line.end.clone()
            } else {
                right_line.end.clone()
            };
            return vec![right_line.start.clone(), second];
        }
        if start_point_side == end_point_side
            || other.start_point().side_of_line(&self.middle)
                == other.end_point().side_of_line(&self.middle)
        {
            return Vec::new(); // no intersection possible
        }
        // now both start points and both end points are on different sides of the middle line
        // of the other segment.
        vec![other.middle.clone()]
    }

    /// Checks if this LineSegment and other contain a common point.
    pub fn intersects(&self, other: &LineSegment) -> bool {
        !self.intersection(other).is_empty()
    }

    /// Checks if this LineSegment and other contain a common LineSegment, which is not reduced to
    /// a point.
    pub fn overlaps(&self, other: &LineSegment) -> bool {
        self.intersection(other).len() > 1
    }

    /// Constructs an approximation of this line segment by orthogonal stairs with integer
    /// coordinates. Panics with division by zero where Java throws ArithmeticException.
    pub fn stair_approximation(&self, width: f64, to_the_right: bool) -> Vec<IntPoint> {
        let start_point = self.start_point().to_float().round();
        let end_point = self.end_point().to_float().round();
        if start_point == end_point {
            return Vec::new();
        }
        if start_point.x == end_point.x || start_point.y == end_point.y {
            return vec![start_point, end_point];
        }

        let dx = end_point.x - start_point.x;
        let dy = end_point.y - start_point.y;
        let abs_dx = dx.wrapping_abs();
        let abs_dy = dy.wrapping_abs();
        let function_of_x = abs_dx >= abs_dy;
        // use otherwise function of y for better numerical stability

        let mut stair_width;
        let stair_count;
        if function_of_x {
            stair_width = math_round_i32((width * abs_dx as f64) / abs_dy as f64);
            stair_count = (abs_dx - 1) / stair_width + 1;
            if end_point.x < start_point.x {
                stair_width = -stair_width;
            }
        } else {
            stair_width = math_round_i32((width * abs_dy as f64) / abs_dx as f64);
            stair_count = (abs_dy - 1) / stair_width + 1;
            if end_point.y < start_point.y {
                stair_width = -stair_width;
            }
        }
        let mut result = vec![IntPoint::default(); (2 * stair_count + 1) as usize];
        result[0] = start_point;
        let det = dx as f64 * dy as f64;
        let change_x_first = to_the_right && det > 0.0 || !to_the_right && det < 0.0;
        let mut current_index = 0usize;

        let mut prev_line_point_x = start_point.x;
        let mut prev_line_point_y = start_point.y;
        for i in 1..stair_count {
            let current_line_point_x;
            let current_line_point_y;
            if function_of_x {
                current_line_point_x = start_point.x + i * stair_width;
                current_line_point_y = math_round_i32(
                    self.get_line()
                        .function_value_approx(current_line_point_x as f64),
                );
            } else {
                current_line_point_y = start_point.y + i * stair_width;
                current_line_point_x = math_round_i32(
                    self.get_line()
                        .function_in_y_value_approx(current_line_point_y as f64),
                );
            }
            current_index += 1;
            if change_x_first {
                result[current_index] = IntPoint::new(current_line_point_x, prev_line_point_y);
            } else {
                result[current_index] = IntPoint::new(prev_line_point_x, current_line_point_y);
            }
            current_index += 1;
            result[current_index] = IntPoint::new(current_line_point_x, current_line_point_y);
            prev_line_point_x = current_line_point_x;
            prev_line_point_y = current_line_point_y;
        }
        current_index += 1;
        if change_x_first {
            result[current_index] = IntPoint::new(end_point.x, prev_line_point_y);
        } else {
            result[current_index] = IntPoint::new(prev_line_point_x, end_point.y);
        }
        current_index += 1;
        result[current_index] = end_point;
        result
    }

    /// Constructs an approximation of this line segment by 45 degree stairs with integer
    /// coordinates. (Like Java, the function-of-y branch uses `function_value_approx`.)
    pub fn stair_approximation_45(&self, width: f64, to_the_right: bool) -> Vec<IntPoint> {
        let start_point = self.start_point().to_float().round();
        let end_point = self.end_point().to_float().round();
        if start_point == end_point {
            return Vec::new();
        }
        let delta = end_point.difference_by_int(&start_point);
        if delta.is_multiple_of_45_degree() {
            return vec![start_point, end_point];
        }
        let abs_delta_x = delta.x.wrapping_abs();
        let abs_delta_y = delta.y.wrapping_abs();
        let function_of_x = abs_delta_x >= abs_delta_y;
        let det = delta.x as f64 * delta.y as f64;
        let mut stair_width;
        let stair_count;
        if function_of_x {
            stair_width = math_round_i32((width * abs_delta_x as f64) / abs_delta_y as f64);
            stair_count = (abs_delta_x - 1) / stair_width + 1;
            if end_point.x < start_point.x {
                stair_width = -stair_width;
            }
        } else {
            stair_width = math_round_i32((width * abs_delta_y as f64) / abs_delta_x as f64);
            stair_count = (abs_delta_y - 1) / stair_width + 1;
            if end_point.y < start_point.y {
                stair_width = -stair_width;
            }
        }
        let mut result = vec![IntPoint::default(); (2 * stair_count + 1) as usize];
        result[0] = start_point;
        let mut prev_line_point = start_point;
        let mut current_index = 0usize;
        let sgn = Signum::as_int(stair_width as f64);
        for i in 1..=stair_count {
            let current_line_point = if i == stair_count {
                end_point
            } else if function_of_x {
                let current_x = start_point.x + i * stair_width;
                let current_y =
                    math_round_i32(self.get_line().function_value_approx(current_x as f64));
                IntPoint::new(current_x, current_y)
            } else {
                let current_y = start_point.y + i * stair_width;
                let current_x =
                    math_round_i32(self.get_line().function_value_approx(current_y as f64));
                IntPoint::new(current_x, current_y)
            };
            let current_x;
            let current_y;
            if function_of_x {
                let diagonal_first = to_the_right && det < 0.0 || !to_the_right && det > 0.0;
                if diagonal_first {
                    current_x = prev_line_point.x
                        + sgn * (current_line_point.y - prev_line_point.y).wrapping_abs();
                    current_y = current_line_point.y;
                } else {
                    // horizontal first
                    current_x = current_line_point.x
                        - sgn * (current_line_point.y - prev_line_point.y).wrapping_abs();
                    current_y = prev_line_point.y;
                }
            } else {
                // function of y
                let diagonal_first = to_the_right && det > 0.0 || !to_the_right && det < 0.0;
                if diagonal_first {
                    current_x = current_line_point.x;
                    current_y = prev_line_point.y
                        + sgn * (current_line_point.x - prev_line_point.x).wrapping_abs();
                } else {
                    current_x = prev_line_point.x;
                    current_y = current_line_point.y
                        - sgn * (current_line_point.x - prev_line_point.x).wrapping_abs();
                }
            }
            current_index += 1;
            result[current_index] = IntPoint::new(current_x, current_y);
            current_index += 1;
            result[current_index] = current_line_point;
            prev_line_point = current_line_point;
        }
        result
    }

    /// Returns the border line numbers of shape which are intersected by this line segment
    /// (length 0, 1 or 2; with 2 intersections the one nearest to the start point comes first).
    pub fn border_intersections(&self, shape: &TileShape) -> Vec<i32> {
        if !self
            .bounding_box()
            .intersects_int_box(&shape.bounding_box())
        {
            return Vec::new();
        }
        let edge_count = shape.border_line_count();
        let mut prev_line = shape.border_line(edge_count - 1);
        let mut current_line = shape.border_line(0);
        let mut result = [0i32; 2];
        let mut intersection: [Option<Point>; 2] = [None, None];
        let mut intersection_count = 0usize;
        let line_start = self.start_point();
        let line_end = self.end_point();

        for edge_line_no in 0..edge_count {
            let next_line = if edge_line_no == edge_count - 1 {
                shape.border_line(0)
            } else {
                shape.border_line(edge_line_no + 1)
            };

            let start_point_side = current_line.side_of(&line_start);
            let end_point_side = current_line.side_of(&line_end);
            if start_point_side == Side::OnTheLeft && end_point_side == Side::OnTheLeft {
                // both endpoints are outside the border line, no intersection possible
                return Vec::new();
            }
            if start_point_side == Side::Collinear && end_point_side != Side::OnTheRight {
                // the start is on current_line; touches count only if the interior is entered
                return Vec::new();
            }
            if end_point_side == Side::Collinear && start_point_side != Side::OnTheRight {
                return Vec::new();
            }

            if start_point_side != Side::OnTheRight || end_point_side != Side::OnTheRight {
                // not both points are inside the halfplane defined by current_line
                let is = self.middle.intersection(&current_line);
                let prev_line_side_of_is = prev_line.side_of(&is);
                let next_line_side_of_is = next_line.side_of(&is);
                if prev_line_side_of_is != Side::OnTheLeft
                    && next_line_side_of_is != Side::OnTheLeft
                {
                    // this line segment intersects current_line between the previous and the
                    // next corner of the shape
                    if prev_line_side_of_is == Side::Collinear {
                        // goes through the previous corner; check that it is not merely a touch
                        let prev_prev_corner = if edge_line_no == 0 {
                            shape.corner(edge_count - 1)
                        } else {
                            shape.corner(edge_line_no - 1)
                        };
                        let next_corner = if edge_line_no == edge_count - 1 {
                            shape.corner(0)
                        } else {
                            shape.corner(edge_line_no + 1)
                        };
                        let prev_prev_corner_side = self.middle.side_of(&prev_prev_corner);
                        let next_corner_side = self.middle.side_of(&next_corner);
                        if prev_prev_corner_side == Side::Collinear
                            || next_corner_side == Side::Collinear
                            || prev_prev_corner_side == next_corner_side
                        {
                            return Vec::new();
                        }
                    }
                    if next_line_side_of_is == Side::Collinear {
                        // goes through the next corner; check that it is not merely a touch
                        let prev_corner = shape.corner(edge_line_no);
                        let next_next_corner = if edge_line_no == edge_count - 2 {
                            shape.corner(0)
                        } else if edge_line_no == edge_count - 1 {
                            shape.corner(1)
                        } else {
                            shape.corner(edge_line_no + 2)
                        };
                        let prev_corner_side = self.middle.side_of(&prev_corner);
                        let next_next_corner_side = self.middle.side_of(&next_next_corner);
                        if prev_corner_side == Side::Collinear
                            || next_next_corner_side == Side::Collinear
                            || prev_corner_side == next_next_corner_side
                        {
                            return Vec::new();
                        }
                    }
                    let mut intersection_already_handled = false;
                    for item in intersection.iter().take(intersection_count) {
                        if item.as_ref() == Some(&is) {
                            intersection_already_handled = true;
                            break;
                        }
                    }
                    if !intersection_already_handled {
                        if intersection_count < result.len() {
                            // a new intersection is found
                            result[intersection_count] = edge_line_no;
                            intersection[intersection_count] = Some(is);
                            intersection_count += 1;
                        } else {
                            log::warn!(
                                "border_intersections: intersection_count ({}) is too big!",
                                intersection_count
                            );
                        }
                    }
                }
            }
            prev_line = current_line;
            current_line = next_line;
        }

        if intersection_count == 0 {
            return Vec::new();
        }
        if intersection_count == 2 {
            // assure the correct order
            let is0 = intersection[0]
                .as_ref()
                .map(|p| p.to_float())
                .unwrap_or_default();
            let is1 = intersection[1]
                .as_ref()
                .map(|p| p.to_float())
                .unwrap_or_default();
            let current_start = line_start.to_float();
            if current_start.distance_square(&is1) < current_start.distance_square(&is0) {
                result.swap(0, 1);
            }
            return result.to_vec();
        }
        vec![result[0]]
    }

    /// Inverts the direction of this.middle, if start_point() has a bigger x coordinate than
    /// end_point(), or an equal x coordinate and a bigger y coordinate.
    pub fn sort_endpoints_in_xy(&self) -> LineSegment {
        let swap_endlines = self.start_point().compare_xy(&self.end_point()) > 0;
        if swap_endlines {
            let result =
                LineSegment::new(self.end.clone(), self.middle.clone(), self.start.clone());
            if let Some(p) = self.precalculated_end_point.get() {
                let _ = result.precalculated_start_point.set(p.clone());
            }
            if let Some(p) = self.precalculated_start_point.get() {
                let _ = result.precalculated_end_point.set(p.clone());
            }
            result
        } else {
            self.clone()
        }
    }
}
