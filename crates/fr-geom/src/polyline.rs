//! Port of `Polyline.java`.
//!
//! A Polyline is a sequence of lines, where no 2 consecutive lines may be parallel. A Polyline of
//! n lines defines a polygon of n-1 intersection points of consecutive lines. Polylines with
//! integer line points are used instead of polygons with rational corners for performance.

use std::sync::{Arc, OnceLock};

use crate::direction::Direction;
use crate::float_point::{java_max, java_min, FloatPoint};
use crate::int_box::IntBox;
use crate::int_octagon::IntOctagon;
use crate::int_point::IntPoint;
use crate::java_compat::{INT_MAX_F64, INT_MIN_F64};
use crate::line::Line;
use crate::line_segment::LineSegment;
use crate::point::Point;
use crate::polygon::Polygon;
use crate::side::Side;
use crate::tile_shape::TileShape;
use crate::vector::Vector;

const USE_BOUNDING_OCTAGON_FOR_OFFSET_SHAPES: bool = true;

#[derive(Default, Debug)]
struct PolylineCache {
    float_corners: OnceLock<Box<[OnceLock<FloatPoint>]>>,
    corners: OnceLock<Box<[OnceLock<Point>]>>,
    bounding_box: OnceLock<IntBox>,
}

/// Cloning is cheap and shares the lazily computed corner caches (like a Java reference).
#[derive(Clone)]
pub struct Polyline {
    /// Stores the array of lines of this polyline.
    pub lines: Arc<[Line]>,
    cache: Arc<PolylineCache>,
}

impl std::fmt::Debug for Polyline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Polyline")
            .field("lines", &self.lines)
            .finish()
    }
}

impl Polyline {
    fn with_lines(lines: Vec<Line>) -> Polyline {
        Polyline {
            lines: lines.into(),
            cache: Arc::new(PolylineCache::default()),
        }
    }

    fn empty() -> Polyline {
        Polyline::with_lines(Vec::new())
    }

    /// Creates a polyline of length polygon.corner_count + 1 from polygon, so that the i-th
    /// corner of polygon is the intersection of the i-th and the i+1-th lines. polygon must have
    /// at least 2 corners.
    pub fn from_polygon(polygon: &Polygon) -> Polyline {
        let points = polygon.corners();
        if points.len() < 2 {
            log::warn!("Polyline: must contain at least 2 different points");
            return Polyline::empty();
        }
        let n = points.len();
        let mut lines: Vec<Line> = Vec::with_capacity(n + 1);
        // construct perpendicular lines at the start and at the end to represent the first and
        // the last point of points as intersection of lines.
        let dir =
            Direction::get_instance_from_points(&points[0], &points[1]).expect("points differ");
        lines.push(Line::get_instance(
            points[0].clone(),
            &dir.turn_45_degree(2),
        ));
        for i in 1..n {
            lines.push(Line::new(points[i - 1].clone(), points[i].clone()));
        }
        let dir = Direction::get_instance_from_points(&points[n - 1], &points[n - 2])
            .expect("points differ");
        lines.push(Line::get_instance(
            points[n - 1].clone(),
            &dir.turn_45_degree(2),
        ));
        Polyline::with_lines(lines)
    }

    /// Creates a polyline from an array of points.
    pub fn from_points(points: &[Point]) -> Polyline {
        Polyline::from_polygon(&Polygon::new(points))
    }

    /// Creates a polyline from an array of IntPoints.
    pub fn from_int_points(points: &[IntPoint]) -> Polyline {
        let pts: Vec<Point> = points.iter().map(|p| Point::Int(*p)).collect();
        Polyline::from_points(&pts)
    }

    /// Creates a polyline consisting of three lines.
    pub fn from_two_points(from_corner: &Point, to_corner: &Point) -> Polyline {
        if from_corner == to_corner {
            return Polyline::empty();
        }
        let dir =
            Direction::get_instance_from_points(from_corner, to_corner).expect("points differ");
        let l0 = Line::get_instance(from_corner.clone(), &dir.turn_45_degree(2));
        let l1 = Line::new(from_corner.clone(), to_corner.clone());
        let l2 = Line::get_instance(to_corner.clone(), &dir.turn_45_degree(2));
        Polyline::with_lines(vec![l0, l1, l2])
    }

    /// Creates a polyline from an array of lines. Lines, which are parallel to the previous line,
    /// are skipped. The directed lines are normalized, so that they intersect the previous line
    /// before the next line.
    ///
    /// Deviation: Java normalizes the directions inside the caller's array when nothing was
    /// filtered; here the input is consumed, so the caller never observes that mutation.
    pub fn from_lines(input_lines: Vec<Line>) -> Polyline {
        let filtered = remove_consecutive_parallel_lines(input_lines);
        let mut filtered = remove_overlaps(filtered);
        if filtered.len() < 3 {
            return Polyline::empty();
        }
        // turn evtl the direction of the lines that they point always from the previous corner
        // to the next corner
        for i in 1..filtered.len() - 1 {
            let corner = filtered[i].intersection_approx(&filtered[i + 1]);
            let side_of_line = filtered[i - 1].side_of_float(&corner);
            if side_of_line != Side::Collinear {
                let d0 = filtered[i - 1].direction();
                let d1 = filtered[i].direction();
                let side1 = d0.side_of(&d1);
                if side1 != side_of_line {
                    filtered[i] = filtered[i].opposite();
                }
            }
        }
        Polyline::with_lines(filtered)
    }

    /// Returns the number of lines minus 1.
    #[inline]
    pub fn corner_count(&self) -> i32 {
        self.lines.len() as i32 - 1
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.lines.len() < 3
    }

    /// Checks, if this polyline is empty or if all corner points are equal.
    pub fn is_point(&self) -> bool {
        if self.lines.len() < 3 {
            return true;
        }
        let first_corner = self.corner(0);
        for i in 1..self.lines.len() as i32 - 1 {
            if self.corner(i) != first_corner {
                return false;
            }
        }
        true
    }

    /// Checks if all lines of this polyline are orthogonal.
    pub fn is_orthogonal(&self) -> bool {
        self.lines.iter().all(|l| l.is_orthogonal())
    }

    /// Checks if all lines of this polyline are multiples of 45 degrees.
    pub fn is_multiple_of_45_degree(&self) -> bool {
        self.lines.iter().all(|l| l.is_multiple_of_45_degree())
    }

    /// Returns the intersection of the first line with the second line.
    pub fn first_corner(&self) -> Point {
        self.corner(0)
    }

    /// Returns the intersection of the last line with the line before the last line.
    pub fn last_corner(&self) -> Point {
        self.corner(self.lines.len() as i32 - 2)
    }

    /// Returns the array of the intersections of two consecutive lines.
    pub fn corners(&self) -> Vec<Point> {
        if self.lines.len() < 2 {
            return Vec::new();
        }
        (0..self.lines.len() as i32 - 1)
            .map(|i| self.corner(i))
            .collect()
    }

    /// Returns the array of intersections of consecutive lines, approximated by FloatPoints.
    pub fn corner_approx_arr(&self) -> Vec<FloatPoint> {
        if self.lines.len() < 2 {
            return Vec::new();
        }
        (0..self.lines.len() as i32 - 1)
            .map(|i| self.corner_approx(i))
            .collect()
    }

    /// Approximation of the intersection of the no-th with the (no + 1)-th line.
    pub fn corner_approx(&self, corner_index: i32) -> FloatPoint {
        let len = self.lines.len() as i32;
        let no = if corner_index < 0 {
            log::warn!("Polyline.corner_approx: no is < 0");
            0
        } else if corner_index >= len - 1 {
            log::warn!("Polyline.corner_approx: no must be less than lines.length - 1");
            len - 2
        } else {
            corner_index
        };
        let no = no as usize; // panics like Java's array access for too short polylines
        let arr = self.cache.float_corners.get_or_init(|| {
            (0..self.lines.len().saturating_sub(1))
                .map(|_| OnceLock::new())
                .collect()
        });
        *arr[no].get_or_init(|| self.lines[no].intersection_approx(&self.lines[no + 1]))
    }

    /// Returns the intersection of the no-th with the (no + 1)-th line. Panics if the polyline
    /// has less than 2 lines (Java returns null).
    pub fn corner(&self, corner_index: i32) -> Point {
        let len = self.lines.len() as i32;
        if len < 2 {
            panic!("Polyline.corner: lines.length is < 2");
        }
        let no = if corner_index < 0 {
            log::warn!("Polyline.corner: no is < 0");
            0
        } else if corner_index >= len - 1 {
            log::warn!("Polyline.corner: no must be less than lines.length - 1");
            len - 2
        } else {
            corner_index
        } as usize;
        let arr = self
            .cache
            .corners
            .get_or_init(|| (0..self.lines.len() - 1).map(|_| OnceLock::new()).collect());
        arr[no]
            .get_or_init(|| self.lines[no].intersection(&self.lines[no + 1]))
            .clone()
    }

    /// Returns the polyline with the reversed order of lines.
    pub fn reverse(&self) -> Polyline {
        let reversed: Vec<Line> = self.lines.iter().rev().map(|l| l.opposite()).collect();
        Polyline::from_lines(reversed)
    }

    /// Calculates the length of this polyline from from_corner to to_corner.
    pub fn length_approx_range(&self, requested_from_corner: i32, requested_to_corner: i32) -> f64 {
        let from_corner = requested_from_corner.max(0);
        let to_corner = requested_to_corner.min(self.lines.len() as i32 - 2);
        let mut result = 0.0;
        for i in from_corner..to_corner {
            result += self.corner_approx(i + 1).distance(&self.corner_approx(i));
        }
        result
    }

    /// Calculates the cumulative distance between consecutive corners of this polyline.
    pub fn length_approx(&self) -> f64 {
        self.length_approx_range(0, self.lines.len() as i32 - 2)
    }

    /// Calculates for each line a shape around this line where the right and left edge lines
    /// have the distance half_width from the center line. Returns lines.len() - 2 shapes.
    pub fn offset_shapes(&self, half_width: i32) -> Vec<TileShape> {
        self.offset_shapes_range(half_width, 0, self.lines.len() as i32 - 1)
    }

    /// Calculates for each line between from_no and to_no a shape around this line, where the
    /// right and left edge lines have the distance half_width from the center line.
    pub fn offset_shapes_range(
        &self,
        half_width: i32,
        requested_from_no: i32,
        requested_to_no: i32,
    ) -> Vec<TileShape> {
        let from_no = requested_from_no.max(0);
        let to_no = requested_to_no.min(self.lines.len() as i32 - 1);
        let shape_count = (to_no - from_no - 1).max(0);
        let mut shapes: Vec<TileShape> = Vec::with_capacity(shape_count as usize);
        if shape_count == 0 {
            return shapes;
        }
        let lines = &self.lines;
        let hw = -(half_width as f64);
        let mut prev_dir = lines[from_no as usize].direction().get_vector();
        let mut current_direction = lines[from_no as usize + 1].direction().get_vector();
        for i in (from_no + 1)..to_no {
            let iu = i as usize;
            let next_dir = lines[iu + 1].direction().get_vector();

            let mut offset_lines: Vec<Line> = Vec::with_capacity(4);
            // current center line translated to the right
            offset_lines.push(lines[iu].translate(hw));

            // create the front line of the offset shape
            let next_dir_from_curr_dir = next_dir.side_of(&current_direction);
            if next_dir_from_curr_dir == Side::OnTheLeft {
                // left turn from current line to next line: next right line
                offset_lines.push(lines[iu + 1].translate(hw));
            } else {
                // next left line in opposite direction
                offset_lines.push(lines[iu + 1].opposite().translate(hw));
            }
            // current left line in opposite direction
            offset_lines.push(lines[iu].opposite().translate(hw));

            // create the back line of the offset shape
            let current_dir_from_prev_dir = current_direction.side_of(&prev_dir);
            if current_dir_from_prev_dir == Side::OnTheLeft {
                // previous line translated to the right
                offset_lines.push(lines[iu - 1].translate(hw));
            } else {
                // previous left line in opposite direction
                offset_lines.push(lines[iu - 1].opposite().translate(hw));
            }

            // cut off outstanding corners with following shapes
            let mut corner_to_check: Option<FloatPoint> = None;
            let mut current_line = offset_lines[1].clone();
            let mut check_line = if next_dir_from_curr_dir == Side::OnTheLeft {
                offset_lines[2].clone()
            } else {
                offset_lines[0].clone()
            };
            let mut check_distance_corner = self.corner_approx(i);
            let check_dist_square = 2.0 * half_width as f64 * half_width as f64;
            let mut cut_dog_ear_lines: Vec<Line> = Vec::new();
            let mut tmp_curr_dir = next_dir.clone();
            let mut direction_changed = false;
            for j in (i + 2)..(lines.len() as i32 - 1) {
                if self
                    .corner_approx(j - 1)
                    .distance_square(&check_distance_corner)
                    > check_dist_square
                {
                    break;
                }
                if !direction_changed {
                    corner_to_check = Some(current_line.intersection_approx(&check_line));
                }
                let tmp_next_dir = lines[j as usize].direction().get_vector();
                let tmp_next_dir_from_tmp_curr_dir = tmp_next_dir.side_of(&tmp_curr_dir);
                direction_changed = tmp_next_dir_from_tmp_curr_dir != next_dir_from_curr_dir;
                if !direction_changed {
                    let next_border_line = if tmp_next_dir_from_tmp_curr_dir == Side::OnTheLeft {
                        lines[j as usize].translate(hw)
                    } else {
                        lines[j as usize].opposite().translate(hw)
                    };
                    let ctc = corner_to_check.expect("corner_to_check set");
                    if next_border_line.side_of_float(&ctc) == Side::OnTheLeft
                        && next_border_line.side_of(&self.corner(i)) == Side::OnTheRight
                        && next_border_line.side_of(&self.corner(i - 1)) == Side::OnTheRight
                    {
                        // an outstanding corner
                        cut_dog_ear_lines.push(next_border_line.clone());
                    }
                    tmp_curr_dir = tmp_next_dir;
                    current_line = next_border_line;
                }
            }
            // cut off outstanding corners with previous shapes
            check_distance_corner = self.corner_approx(i - 1);
            check_line = if current_dir_from_prev_dir == Side::OnTheLeft {
                offset_lines[2].clone()
            } else {
                offset_lines[0].clone()
            };
            current_line = offset_lines[3].clone();
            tmp_curr_dir = prev_dir.clone();
            direction_changed = false;
            let mut j = i - 2;
            while j >= 1 {
                if self
                    .corner_approx(j)
                    .distance_square(&check_distance_corner)
                    > check_dist_square
                {
                    break;
                }
                if !direction_changed {
                    corner_to_check = Some(current_line.intersection_approx(&check_line));
                }
                let tmp_prev_dir = lines[j as usize].direction().get_vector();
                let tmp_curr_dir_from_tmp_prev_dir = tmp_curr_dir.side_of(&tmp_prev_dir);
                direction_changed = tmp_curr_dir_from_tmp_prev_dir != current_dir_from_prev_dir;
                if !direction_changed {
                    let prev_border_line = if tmp_curr_dir.side_of(&tmp_prev_dir) == Side::OnTheLeft
                    {
                        lines[j as usize].translate(hw)
                    } else {
                        lines[j as usize].opposite().translate(hw)
                    };
                    let ctc = corner_to_check.expect("corner_to_check set");
                    if prev_border_line.side_of_float(&ctc) == Side::OnTheLeft
                        && prev_border_line.side_of(&self.corner(i)) == Side::OnTheRight
                        && prev_border_line.side_of(&self.corner(i - 1)) == Side::OnTheRight
                    {
                        // an outstanding corner
                        cut_dog_ear_lines.push(prev_border_line.clone());
                    }
                    tmp_curr_dir = tmp_prev_dir;
                    current_line = prev_border_line;
                }
                j -= 1;
            }
            let mut s1 = TileShape::get_instance_lines(&offset_lines);
            if !cut_dog_ear_lines.is_empty() {
                s1 = s1.intersection(&TileShape::get_instance_lines(&cut_dog_ear_lines));
            }
            let bounding_shape: TileShape = if USE_BOUNDING_OCTAGON_FOR_OFFSET_SHAPES {
                // intersect with the bounding octagon
                let surr_oct = self.bounding_octagon_range(i - 1, i);
                TileShape::IntOctagon(surr_oct.offset(half_width as f64))
            } else {
                // intersect with the bounding box
                let surr_box = self.bounding_box_range(i - 1, i);
                let offset_box = surr_box.offset(half_width as f64);
                TileShape::Simplex(offset_box.to_simplex())
            };
            let shape = bounding_shape.intersection_with_simplify(&s1);
            if shape.is_empty() {
                log::warn!("offset_shapes: shape is empty");
            }
            shapes.push(shape);

            prev_dir = current_direction;
            current_direction = next_dir;
        }
        shapes
    }

    /// Calculates for the no-th line segment a shape around this line where the right and left
    /// edge lines have the distance half_width from the center line.
    /// 0 <= no <= lines.len() - 3; returns None otherwise.
    pub fn offset_shape(&self, half_width: i32, no: i32) -> Option<TileShape> {
        if no < 0 || no > self.lines.len() as i32 - 3 {
            log::warn!("Polyline.offsetShape: no out of range");
            return None;
        }
        self.offset_shapes_range(half_width, no, no + 2)
            .into_iter()
            .next()
    }

    /// Calculates for the no-th line segment a box shape around this line where the border
    /// lines have the distance half_width from the center line.
    pub fn offset_box(&self, half_width: i32, no: i32) -> IntBox {
        let current_line_segment =
            LineSegment::from_polyline(self, no + 1).expect("LineSegment: no out of range");
        current_line_segment
            .bounding_box()
            .offset(half_width as f64)
    }

    /// Returns the polyline translated by vector.
    pub fn translate_by(&self, vector: &Vector) -> Polyline {
        if *vector == Vector::ZERO {
            return self.clone();
        }
        Polyline::from_lines(self.lines.iter().map(|l| l.translate_by(vector)).collect())
    }

    /// Returns the polyline turned by factor times 90 degrees around pole.
    pub fn turn_90_degree(&self, factor: i32, pole: &IntPoint) -> Polyline {
        Polyline::from_lines(
            self.lines
                .iter()
                .map(|l| l.turn_90_degree(factor, pole))
                .collect(),
        )
    }

    /// Returns an approximation of this polyline rotated around pole.
    pub fn rotate_approx(&self, angle: f64, pole: &FloatPoint) -> Polyline {
        if angle == 0.0 {
            return self.clone();
        }
        let new_corners: Vec<IntPoint> = (0..self.corner_count())
            .map(|i| self.corner_approx(i).rotate(angle, pole).round())
            .collect();
        Polyline::from_int_points(&new_corners)
    }

    /// Mirrors this polyline at the vertical line through pole.
    pub fn mirror_vertical(&self, pole: &IntPoint) -> Polyline {
        Polyline::from_lines(self.lines.iter().map(|l| l.mirror_vertical(pole)).collect())
    }

    /// Mirrors this polyline at the horizontal line through pole.
    pub fn mirror_horizontal(&self, pole: &IntPoint) -> Polyline {
        Polyline::from_lines(
            self.lines
                .iter()
                .map(|l| l.mirror_horizontal(pole))
                .collect(),
        )
    }

    /// Returns the smallest box containing the corners from index from_corner_no to
    /// to_corner_no.
    pub fn bounding_box_range(
        &self,
        requested_from_corner_no: i32,
        requested_to_corner_no: i32,
    ) -> IntBox {
        let from_corner_no = requested_from_corner_no.max(0);
        let to_corner_no = requested_to_corner_no.min(self.lines.len() as i32 - 2);
        let mut llx = INT_MAX_F64;
        let mut lly = llx;
        let mut urx = INT_MIN_F64;
        let mut ury = urx;
        for i in from_corner_no..=to_corner_no {
            let c = self.corner_approx(i);
            llx = java_min(llx, c.x);
            lly = java_min(lly, c.y);
            urx = java_max(urx, c.x);
            ury = java_max(ury, c.y);
        }
        let lower_left = IntPoint::new(llx.floor() as i32, lly.floor() as i32);
        let upper_right = IntPoint::new(urx.ceil() as i32, ury.ceil() as i32);
        IntBox::from_points(lower_left, upper_right)
    }

    /// Returns the smallest box containing the intersection points of the lines of this
    /// polyline.
    pub fn bounding_box(&self) -> IntBox {
        *self
            .cache
            .bounding_box
            .get_or_init(|| self.bounding_box_range(0, self.corner_count() - 1))
    }

    /// Returns the smallest octagon containing the corners from index from_corner_no to
    /// to_corner_no.
    pub fn bounding_octagon_range(
        &self,
        requested_from_corner_no: i32,
        requested_to_corner_no: i32,
    ) -> IntOctagon {
        let from_corner_no = requested_from_corner_no.max(0);
        let to_corner_no = requested_to_corner_no.min(self.lines.len() as i32 - 2);
        let mut lx = INT_MAX_F64;
        let mut ly = INT_MAX_F64;
        let mut rx = INT_MIN_F64;
        let mut uy = INT_MIN_F64;
        let mut ulx = INT_MAX_F64;
        let mut lrx = INT_MIN_F64;
        let mut llx = INT_MAX_F64;
        let mut urx = INT_MIN_F64;
        for i in from_corner_no..=to_corner_no {
            let c = self.corner_approx(i);
            lx = java_min(lx, c.x);
            ly = java_min(ly, c.y);
            rx = java_max(rx, c.x);
            uy = java_max(uy, c.y);
            let tmp = c.x - c.y;
            ulx = java_min(ulx, tmp);
            lrx = java_max(lrx, tmp);
            let tmp = c.x + c.y;
            llx = java_min(llx, tmp);
            urx = java_max(urx, tmp);
        }
        IntOctagon::new(
            lx.floor() as i32,
            ly.floor() as i32,
            rx.ceil() as i32,
            uy.ceil() as i32,
            ulx.floor() as i32,
            lrx.ceil() as i32,
            llx.floor() as i32,
            urx.ceil() as i32,
        )
    }

    /// Calculates an approximation of the nearest point on this polyline to from_point.
    pub fn nearest_point_approx(&self, from_point: &FloatPoint) -> Option<FloatPoint> {
        let mut min_distance = f64::MAX;
        let mut nearest_point = None;
        // calculate the nearest corner point
        let corners = self.corner_approx_arr();
        for c in &corners {
            let current_distance = c.distance(from_point);
            if current_distance < min_distance {
                min_distance = current_distance;
                nearest_point = Some(*c);
            }
        }
        let c_tolerance = 1.0;
        for i in 1..self.lines.len().saturating_sub(1) {
            let projection = from_point.projection_approx(&self.lines[i]);
            let current_distance = projection.distance(from_point);
            if current_distance < min_distance {
                // look, if the projection is inside the segment
                let segment_length = corners[i].distance(&corners[i - 1]);
                if projection.distance(&corners[i]) + projection.distance(&corners[i - 1])
                    < segment_length + c_tolerance
                {
                    min_distance = current_distance;
                    nearest_point = Some(projection);
                }
            }
        }
        nearest_point
    }

    /// Calculates the distance of from_point to the nearest point on this polyline.
    pub fn distance(&self, from_point: &FloatPoint) -> f64 {
        from_point.distance(
            &self
                .nearest_point_approx(from_point)
                .expect("NullPointerException: no nearest point"),
        )
    }

    /// Combines the two polylines, if they have a common end corner. The order of lines in this
    /// polyline is preserved. Returns the combined polyline or this polyline, if there is no
    /// common end corner.
    pub fn combine(&self, other: Option<&Polyline>) -> Polyline {
        let other = match other {
            Some(o) if self.lines.len() >= 3 && o.lines.len() >= 3 => o,
            _ => return self.clone(),
        };
        let (combine_at_start, combine_other_at_start) =
            if self.first_corner() == other.first_corner() {
                (true, true)
            } else if self.first_corner() == other.last_corner() {
                (true, false)
            } else if self.last_corner() == other.first_corner() {
                (false, true)
            } else if self.last_corner() == other.last_corner() {
                (false, false)
            } else {
                return self.clone(); // no common endpoint
            };
        let tl = &self.lines;
        let ol = &other.lines;
        let mut new_lines: Vec<Line> = Vec::with_capacity(tl.len() + ol.len() - 2);
        if combine_at_start {
            // insert the lines of other in front
            if combine_other_at_start {
                // insert in reverse order, skip the first line of other
                for i in 0..ol.len() - 1 {
                    new_lines.push(ol[ol.len() - i - 1].opposite());
                }
            } else {
                // skip the last line of other
                new_lines.extend(ol[..ol.len() - 1].iter().cloned());
            }
            // append the lines of this polyline, skip the first line
            new_lines.extend(tl[1..].iter().cloned());
        } else {
            // insert the lines of this polyline in front, skip the last line
            new_lines.extend(tl[..tl.len() - 1].iter().cloned());
            if combine_other_at_start {
                // skip the first line of other
                new_lines.extend(ol[1..].iter().cloned());
            } else {
                // insert in reverse order, skip the last line of other
                for i in 1..ol.len() {
                    new_lines.push(ol[ol.len() - i - 1].opposite());
                }
            }
        }
        Polyline::from_lines(new_lines)
    }

    /// Splits this polyline at the line with index line_index into two by inserting end_line as
    /// concluding line of the first piece and as start line of the second piece. Returns None,
    /// if nothing was split.
    pub fn split(&self, line_index: i32, end_line: &Line) -> Option<[Polyline; 2]> {
        let len = self.lines.len() as i32;
        if line_index < 1 || line_index > len - 2 {
            log::warn!("Polyline.split: lineIndex out of range");
            return None;
        }
        let li = line_index as usize;
        if self.lines[li].is_parallel(end_line) {
            return None;
        }
        let new_end_corner = self.lines[li].intersection(end_line);
        if line_index == 1 && new_end_corner == self.first_corner()
            || line_index >= len - 2 && new_end_corner == self.last_corner()
        {
            // No split, if end_line does not intersect, but touches only this Polyline at an
            // end point.
            return None;
        }
        let first_piece: Vec<Line> = if self.corner(line_index - 1) == new_end_corner {
            // skip line segment of length 0 at the end of the first piece
            self.lines[..li + 1].to_vec()
        } else {
            let mut v = self.lines[..li + 1].to_vec();
            v.push(end_line.clone());
            v
        };
        let second_piece: Vec<Line> = if self.corner(line_index) == new_end_corner {
            // skip line segment of length 0 at the beginning of the second piece
            self.lines[li..].to_vec()
        } else {
            let mut v = Vec::with_capacity(self.lines.len() - li + 1);
            v.push(end_line.clone());
            v.extend(self.lines[li..].iter().cloned());
            v
        };
        let result = [
            Polyline::from_lines(first_piece),
            Polyline::from_lines(second_piece),
        ];
        if result[0].is_point() || result[1].is_point() {
            return None;
        }
        Some(result)
    }

    /// Creates a new polyline by skipping lines from from_no to to_no.
    pub fn skip_lines(&self, from_no: i32, to_no: i32) -> Polyline {
        if from_no < 0 || to_no > self.lines.len() as i32 - 1 || from_no > to_no {
            return self.clone();
        }
        let mut new_lines: Vec<Line> = self.lines[..from_no as usize].to_vec();
        new_lines.extend(self.lines[to_no as usize + 1..].iter().cloned());
        Polyline::from_lines(new_lines)
    }

    /// Returns whether this polyline contains the given point.
    pub fn contains(&self, point: &Point) -> bool {
        for i in 1..self.lines.len() as i32 - 1 {
            let current_segment = LineSegment::from_polyline(self, i).expect("index in range");
            if current_segment.contains(point) {
                return true;
            }
        }
        false
    }

    /// Creates a perpendicular line segment from point onto the nearest line segment of this
    /// polyline. Returns None, if the perpendicular line does not intersect the nearest line
    /// segment inside its bounds or if point is contained in this polyline.
    pub fn projection_line(&self, point: &Point) -> Option<LineSegment> {
        let from_point = point.to_float();
        let mut min_distance = f64::MAX;
        let mut result_line: Option<Line> = None;
        let mut nearest_line: Option<&Line> = None;
        for i in 1..self.lines.len().saturating_sub(1) {
            let projection = from_point.projection_approx(&self.lines[i]);
            let current_distance = projection.distance(&from_point);
            if current_distance < min_distance {
                let direction_towards_line = match self.lines[i].perpendicular_direction(point) {
                    Some(d) => d,
                    None => continue,
                };
                let current_result_line =
                    Line::from_point_direction(point.clone(), direction_towards_line);
                let prev_corner = self.corner(i as i32 - 1);
                let next_corner = self.corner(i as i32);
                let prev_corner_side = current_result_line.side_of(&prev_corner);
                let next_corner_side = current_result_line.side_of(&next_corner);
                if prev_corner_side == next_corner_side && prev_corner_side != Side::Collinear {
                    // the projection point is outside the line segment
                    continue;
                }
                nearest_line = Some(&self.lines[i]);
                min_distance = current_distance;
                result_line = Some(current_result_line);
            }
        }
        let nearest_line = nearest_line?;
        let start_line = Line::from_point_direction(point.clone(), nearest_line.direction());
        Some(LineSegment::new(
            start_line,
            result_line.expect("set with nearest_line"),
            nearest_line.clone(),
        ))
    }

    /// Shortens this polyline to new_line_count lines. Additionally, the last line segment will
    /// be approximately shortened to last_segment_length. The last corner of the new polyline
    /// will be an IntPoint.
    pub fn shorten(&self, new_line_count: i32, last_segment_length: f64) -> Polyline {
        let last_corner = self.corner_approx(new_line_count - 2);
        let prev_last_corner = self.corner_approx(new_line_count - 3);
        let new_last_corner = prev_last_corner
            .change_length(&last_corner, last_segment_length)
            .round();
        if Point::Int(new_last_corner) == self.corner(self.corner_count() - 2) {
            // skip the last line
            return self.skip_lines(new_line_count - 1, new_line_count - 1);
        }
        let n = new_line_count as usize;
        let mut new_lines: Vec<Line> = self.lines[..n - 2].to_vec();
        // create the last 2 lines of the new polyline
        let mut first_line_point = self.lines[n - 2].a.clone();
        if first_line_point == Point::Int(new_last_corner) {
            first_line_point = self.lines[n - 2].b.clone();
        }
        let new_prev_last_line = Line::new(first_line_point, Point::Int(new_last_corner));
        let last = Line::get_instance(
            Point::Int(new_last_corner),
            &new_prev_last_line.direction().turn_45_degree(6),
        );
        new_lines.push(new_prev_last_line);
        new_lines.push(last);
        Polyline::from_lines(new_lines)
    }
}

fn remove_consecutive_parallel_lines(lines: Vec<Line>) -> Vec<Line> {
    if lines.len() < 3 {
        // polyline must have at least 3 lines
        return lines;
    }
    let original_len = lines.len();
    let mut tmp: Vec<Line> = Vec::with_capacity(original_len);
    let mut iter = lines.into_iter();
    tmp.push(iter.next().expect("non empty"));
    for line in iter {
        // skip multiple lines
        if !tmp.last().expect("non empty").is_parallel(&line) {
            tmp.push(line);
        }
    }
    if tmp.len() == original_len {
        // nothing skipped
        return tmp;
    }
    // at least 1 line is skipped
    if tmp.len() < 3 {
        return Vec::new();
    }
    tmp
}

/// Checks if previous and next lines are equal or opposite and removes the resulting overlap.
fn remove_overlaps(lines: Vec<Line>) -> Vec<Line> {
    let n = lines.len();
    if n < 4 {
        return lines;
    }
    let mut tmp: Vec<Option<Line>> = vec![None; n];
    let mut new_length: usize = 0;
    tmp[0] = Some(lines[0].clone());
    if !lines[0].is_equal_or_opposite(&lines[2]) {
        new_length += 1;
    }
    // else skip the first line
    tmp[new_length] = Some(lines[1].clone());
    new_length += 1;
    for i in 2..n - 2 {
        if tmp[new_length - 1]
            .as_ref()
            .expect("set")
            .is_equal_or_opposite(&lines[i + 1])
        {
            // skip 2 lines
            new_length -= 1;
        } else {
            tmp[new_length] = Some(lines[i].clone());
            new_length += 1;
        }
    }
    tmp[new_length] = Some(lines[n - 2].clone());
    new_length += 1;
    // Guard: new_length must be >= 2 before accessing tmp[new_length - 2].
    if new_length >= 2
        && !lines[n - 1].is_equal_or_opposite(tmp[new_length - 2].as_ref().expect("set"))
    {
        tmp[new_length] = Some(lines[n - 1].clone());
        new_length += 1;
    }
    // else skip the last line
    if new_length == n {
        // nothing skipped
        return lines;
    }
    if new_length < 3 {
        return Vec::new();
    }
    tmp.into_iter()
        .take(new_length)
        .map(|l| l.expect("set"))
        .collect()
}
