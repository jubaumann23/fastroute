//! Port of `TileShape.java`: convex shapes whose borders consist of straight lines
//! (`IntBox | IntOctagon | Simplex`).
//!
//! Java's abstract class is split into the trait [`TileShapeImpl`] (abstract methods and the
//! inherited default implementations, overridable per leaf type) and the closed enum
//! [`TileShape`], the value type used by callers. Java double dispatch
//! (`a.intersection(b)` -> `b.intersection(<type of a>)`) is reproduced by matching on the
//! variant pairs in exactly the Java call order.

use crate::circle::Circle;
use crate::convex_shape::ConvexShape;
use crate::direction::Direction;
use crate::float_line::FloatLine;
use crate::float_point::FloatPoint;
use crate::int_box::IntBox;
use crate::int_octagon::IntOctagon;
use crate::int_point::IntPoint;
use crate::line::Line;
use crate::line_segment::LineSegment;
use crate::point::Point;
use crate::polygon::Polygon;
use crate::polyline::Polyline;
use crate::polyline_shape::{PolylineShape, PolylineShapeImpl};
use crate::regular_tile_shape::RegularTileShape;
use crate::shape::Shape;
use crate::shape_bounding_directions::ShapeBoundingDirections;
use crate::side::Side;
use crate::simplex::Simplex;
use crate::vector::Vector;

/// Abstract part and default methods of Java `TileShape` (see module docs).
///
/// Methods with the suffix `_tile` correspond to Java methods with covariant return types; the
/// leaf types additionally have inherent methods without the suffix returning the exact type.
pub trait TileShapeImpl: PolylineShapeImpl {
    /// Upcast of this leaf into the enum (cheap: Simplex clones share their data).
    fn to_tile_shape(&self) -> TileShape;
    /// Converts the physical instance of this shape to a simpler physical instance, if possible.
    fn simplify(&self) -> TileShape;
    /// Returns a unique ID for this shape for deterministic tie-breaking.
    fn get_id(&self) -> i32;
    fn is_int_box(&self) -> bool;
    fn is_int_octagon(&self) -> bool;
    /// Java overload `intersection(IntBox)` of this class.
    fn intersection_int_box_tile(&self, other: &IntBox) -> TileShape;
    /// Java overload `intersection(IntOctagon)` of this class.
    fn intersection_int_octagon_tile(&self, other: &IntOctagon) -> TileShape;
    /// Java overload `intersection(Simplex)` of this class.
    fn intersection_simplex_tile(&self, other: &Simplex) -> TileShape;
    /// Returns the edge number if line is a border line of this shape; otherwise -1.
    fn border_line_index(&self, line: &Line) -> i32;
    /// Converts the internal representation of this TileShape to a Simplex.
    fn to_simplex(&self) -> Simplex;
    /// ConvexShape `offset`.
    fn offset_tile(&self, distance: f64) -> TileShape;
    fn max_width(&self) -> f64;
    fn min_width(&self) -> f64;
    fn translate_by_tile(&self, vector: &Vector) -> TileShape;
    /// Cuts shape out of this shape and divides the result into convex pieces.
    fn cutout_tile(&self, shape: &TileShape) -> Option<Vec<TileShape>>;
    /// Java `cutoutFrom(IntBox)`: `shape` minus this.
    fn cutout_from_int_box_tile(&self, shape: &IntBox) -> Option<Vec<TileShape>>;
    /// Java `cutoutFrom(IntOctagon)`.
    fn cutout_from_int_octagon_tile(&self, shape: &IntOctagon) -> Option<Vec<TileShape>>;
    /// Java `cutoutFrom(Simplex)`.
    fn cutout_from_simplex_tile(&self, shape: &Simplex) -> Option<Vec<TileShape>>;
    /// Java `boundingOctagon()` (None only for unbounded simplices).
    fn bounding_octagon_opt(&self) -> Option<IntOctagon>;
    /// Java `enlarge(double)`.
    fn enlarge_tile(&self, offset: f64) -> TileShape;
    fn intersects_int_box(&self, other: &IntBox) -> bool;
    fn intersects_int_octagon(&self, other: &IntOctagon) -> bool;
    fn intersects_simplex(&self, other: &Simplex) -> bool;
    fn intersects_circle(&self, other: &Circle) -> bool;
    /// Java `boundingShape(ShapeBoundingDirections)`.
    fn bounding_shape_tile(&self, dirs: &ShapeBoundingDirections) -> Option<RegularTileShape>;

    /// Content of the area of the shape; Double.MAX_VALUE if unbounded.
    fn area(&self) -> f64 {
        if !self.is_bounded() {
            return f64::MAX;
        }
        if self.dimension() < 2 {
            return 0.0;
        }
        // half of the absolute value of x0 (y1 - yn-1) + x1 (y2 - y0) + ... + xn-1 (y0 - yn-2)
        let mut result = 0.0;
        let corner_count = self.border_line_count();
        let mut prev_corner = self.corner_approx(corner_count - 2);
        let mut current_corner = self.corner_approx(corner_count - 1);
        for i in 0..corner_count {
            let next_corner = self.corner_approx(i);
            result += current_corner.x * (next_corner.y - prev_corner.y);
            prev_corner = current_corner;
            current_corner = next_corner;
        }
        0.5 * result.abs()
    }

    /// Returns true, if point is not contained in the inside or the edge of the shape.
    fn is_outside(&self, point: &Point) -> bool {
        let line_count = self.border_line_count();
        if line_count == 0 {
            return true;
        }
        for i in 0..line_count {
            if self.border_line(i).side_of(point) == Side::OnTheLeft {
                return true;
            }
        }
        false
    }

    /// Java `contains(Point)`.
    fn contains(&self, point: &Point) -> bool {
        !self.is_outside(point)
    }

    /// Java `contains(FloatPoint)`.
    fn contains_float(&self, point: &FloatPoint) -> bool {
        self.contains_float_tol(point, 0.0)
    }

    /// Java `contains(FloatPoint, double)`: tolerance is used in the determinant calculation,
    /// not as distance.
    fn contains_float_tol(&self, point: &FloatPoint, tolerance: f64) -> bool {
        let line_count = self.border_line_count();
        if line_count == 0 {
            return false;
        }
        for i in 0..line_count {
            if self.border_line(i).side_of_float_tol(point, tolerance) != Side::OnTheRight {
                return false;
            }
        }
        true
    }

    /// Java `contains(TileShape)`: returns true, if this shape contains other completely.
    fn contains_tile_shape(&self, other: &TileShape) -> bool {
        for i in 0..other.border_line_count() {
            if !self.contains(&other.corner(i)) {
                return false;
            }
        }
        true
    }

    /// Returns true, if point is contained in this shape, but not on an edge line.
    fn contains_inside(&self, point: &Point) -> bool {
        let line_count = self.border_line_count();
        if line_count == 0 {
            return false;
        }
        for i in 0..line_count {
            if self.border_line(i).side_of(point) != Side::OnTheRight {
                return false;
            }
        }
        true
    }

    /// COLLINEAR if point is on the border (with tolerance), ON_THE_LEFT if outside,
    /// ON_THE_RIGHT if inside.
    fn side_of_border(&self, point: &FloatPoint, tolerance: f64) -> Side {
        let line_count = self.border_line_count();
        if line_count == 0 {
            return Side::Collinear;
        }
        let mut result = Side::OnTheRight; // point is inside
        for i in 0..line_count {
            let current_side = self.border_line(i).side_of_float_tol(point, tolerance);
            if current_side == Side::OnTheLeft {
                return Side::OnTheLeft; // point is outside
            } else if current_side == Side::Collinear {
                result = current_side;
            }
        }
        result
    }

    /// Number of the edge line segment containing point, or -1.
    fn contains_on_border_line_no(&self, point: &Point) -> i32 {
        let line_count = self.border_line_count();
        if line_count == 0 {
            return -1;
        }
        let mut containing_line_no = -1;
        for i in 0..line_count {
            let side_of = self.border_line(i).side_of(point);
            if side_of == Side::OnTheLeft {
                // point outside the convex shape
                return -1;
            }
            if side_of == Side::Collinear {
                containing_line_no = i;
            }
        }
        containing_line_no
    }

    /// Returns true, if point lies exact on the boundary of the shape.
    fn contains_on_border(&self, point: &Point) -> bool {
        self.contains_on_border_line_no(point) >= 0
    }

    /// Returns true, if this shape contains other completely (approximately).
    fn contains_approx(&self, other: &TileShape) -> bool {
        other
            .corner_approx_arr()
            .iter()
            .all(|c| self.contains_float(c))
    }

    /// Distance between point and its nearest point on the shape; 0 if contained.
    fn distance(&self, point: &FloatPoint) -> f64 {
        let nearest_point = self
            .nearest_point_approx(point)
            .expect("NullPointerException: no nearest point");
        nearest_point.distance(point)
    }

    /// Distance between point and its nearest point on the edge of the shape.
    fn border_distance(&self, point: &FloatPoint) -> f64 {
        let nearest_point = self
            .nearest_border_point_approx(point)
            .expect("NullPointerException: no border point");
        nearest_point.distance(point)
    }

    fn smallest_radius(&self) -> f64 {
        self.border_distance(&self.centre_of_gravity())
    }

    /// Point in this shape with the smallest distance to from_point (from_point itself if
    /// contained). None for an empty shape.
    fn nearest_point(&self, from_point: &Point) -> Option<Point> {
        if !self.is_outside(from_point) {
            return Some(from_point.clone());
        }
        self.nearest_border_point(from_point)
    }

    /// Approximation of the nearest point of the shape to from_point. None for an empty shape.
    fn nearest_point_approx(&self, from_point: &FloatPoint) -> Option<FloatPoint> {
        if self.contains_float(from_point) {
            return Some(*from_point);
        }
        self.nearest_border_point_approx(from_point)
    }

    /// Returns the nearest point to from_point on the edge of the shape (None if empty).
    fn nearest_border_point(&self, from_point: &Point) -> Option<Point> {
        let line_count = self.border_line_count();
        if line_count == 0 {
            return None;
        }
        let from_point_f = from_point.to_float();
        if line_count == 1 {
            return Some(self.border_line(0).perpendicular_projection(from_point));
        }
        let mut min_dist = f64::MAX;
        let mut min_dist_ind = 0;
        // calculate the distance to the nearest corner first
        for i in 0..line_count {
            let current_corner_f = self.corner_approx(i);
            let current_distance = current_corner_f.distance_square(&from_point_f);
            if current_distance < min_dist {
                min_dist = current_distance;
                min_dist_ind = i;
            }
        }
        let mut nearest_point = self.corner(min_dist_ind);
        let mut prev_ind = line_count - 2;
        let mut current_ind = line_count - 1;
        for next_ind in 0..line_count {
            let projection = self
                .border_line(current_ind)
                .perpendicular_projection(from_point);
            if (!self.corner_is_bounded(current_ind)
                || self.border_line(prev_ind).side_of(&projection) == Side::OnTheRight)
                && (!self.corner_is_bounded(next_ind)
                    || self.border_line(next_ind).side_of(&projection) == Side::OnTheRight)
            {
                let projection_f = projection.to_float();
                let current_distance = projection_f.distance_square(&from_point_f);
                if current_distance < min_dist {
                    min_dist = current_distance;
                    nearest_point = projection;
                }
            }
            prev_ind = current_ind;
            current_ind = next_ind;
        }
        Some(nearest_point)
    }

    /// Approximation of the nearest point to from_point on the border of this shape.
    fn nearest_border_point_approx(&self, from_point: &FloatPoint) -> Option<FloatPoint> {
        self.nearest_border_points_approx(from_point, 1)
            .into_iter()
            .next()
            .flatten()
    }

    /// Approximation of the count nearest points to from_point on the border of this shape,
    /// on different border lines, sorted ascending by distance. Like the Java array, the result
    /// has length `min(count, lineCount)` and its tail may contain `None` (Java null) for
    /// unbounded shapes.
    fn nearest_border_points_approx(
        &self,
        from_point: &FloatPoint,
        count: i32,
    ) -> Vec<Option<FloatPoint>> {
        if count <= 0 {
            return Vec::new();
        }
        let line_count = self.border_line_count();
        if line_count == 0 {
            return Vec::new();
        }
        if line_count == 1 {
            return vec![Some(from_point.projection_approx(&self.border_line(0)))];
        }
        if self.dimension() == 0 {
            return vec![Some(self.corner_approx(0))];
        }
        let result_count = count.min(line_count) as usize;
        let mut nearest_points: Vec<Option<FloatPoint>> = vec![None; result_count];
        let mut min_dists = vec![f64::MAX; result_count];

        fn insert(
            nearest_points: &mut [Option<FloatPoint>],
            min_dists: &mut [f64],
            current_distance: f64,
            p: FloatPoint,
        ) {
            let result_count = min_dists.len();
            for j in 0..result_count {
                if current_distance < min_dists[j] {
                    for k in j + 1..result_count {
                        min_dists[k] = min_dists[k - 1];
                        nearest_points[k] = nearest_points[k - 1];
                    }
                    min_dists[j] = current_distance;
                    nearest_points[j] = Some(p);
                    break;
                }
            }
        }

        // calculate the distances to the nearest corners first
        for i in 0..line_count {
            if self.corner_is_bounded(i) {
                let current_corner = self.corner_approx(i);
                let current_distance = current_corner.distance_square(from_point);
                insert(
                    &mut nearest_points,
                    &mut min_dists,
                    current_distance,
                    current_corner,
                );
            }
        }
        let mut prev_ind = line_count - 2;
        let mut current_ind = line_count - 1;
        for next_ind in 0..line_count {
            let projection = from_point.projection_approx(&self.border_line(current_ind));
            if (!self.corner_is_bounded(current_ind)
                || self.border_line(prev_ind).side_of_float(&projection) == Side::OnTheRight)
                && (!self.corner_is_bounded(next_ind)
                    || self.border_line(next_ind).side_of_float(&projection) == Side::OnTheRight)
            {
                let current_distance = projection.distance_square(from_point);
                insert(
                    &mut nearest_points,
                    &mut min_dists,
                    current_distance,
                    projection,
                );
            }
            prev_ind = current_ind;
            current_ind = next_ind;
        }
        nearest_points
    }

    /// Returns the number of the nearest corner of the shape to from_point. (Like Java, the
    /// initial minimum is Double.MIN_VALUE, so the result is always 0.)
    fn index_of_nearest_corner(&self, from_point: &Point) -> i32 {
        let from_point_f = from_point.to_float();
        let mut result = 0;
        let mut min_dist = f64::from_bits(1); // Double.MIN_VALUE
        for i in 0..self.border_line_count() {
            let current_distance = self.corner_approx(i).distance(&from_point_f);
            if current_distance < min_dist {
                min_dist = current_distance;
                result = i;
            }
        }
        result
    }

    /// Line segment of the approximated corners with index 0 and corner_count / 2.
    fn diagonal_corner_segment(&self) -> Option<FloatLine> {
        if self.is_empty() {
            return None;
        }
        let first_corner = self.corner_approx(0);
        let last_corner = self.corner_approx(self.border_line_count() / 2);
        Some(FloatLine::new(first_corner, last_corner))
    }

    /// Approximation of the count nearest relative outside locations of shape in the direction
    /// of different border lines of this shape, sorted ascending.
    fn nearest_relative_outside_locations(&self, shape: &TileShape, count: i32) -> Vec<FloatPoint> {
        let line_count = self.border_line_count();
        if count <= 0
            || line_count < 3
            || !self.to_tile_shape().intersects(&Shape::Tile(shape.clone()))
        {
            return Vec::new();
        }
        let result_count = count.min(line_count) as usize;
        let mut translate_coors: Vec<Option<FloatPoint>> = vec![None; result_count];
        let mut min_dists = vec![f64::MAX; result_count];
        let mut current_ind = line_count - 1;
        let other_line_count = shape.border_line_count();
        for next_ind in 0..line_count {
            let mut current_max_dist = 0.0;
            let mut current_translate_coor = FloatPoint::ZERO;
            let border_line = self.border_line(current_ind);
            for corner_index in 0..other_line_count {
                let current_corner = shape.corner_approx(corner_index);
                if border_line.side_of_float(&current_corner) == Side::OnTheRight {
                    let projection = current_corner.projection_approx(&border_line);
                    let current_distance = projection.distance_square(&current_corner);
                    if current_distance > current_max_dist {
                        current_max_dist = current_distance;
                        current_translate_coor = projection.subtract(&current_corner);
                    }
                }
            }
            for j in 0..result_count {
                if current_max_dist < min_dists[j] {
                    for k in j + 1..result_count {
                        min_dists[k] = min_dists[k - 1];
                        translate_coors[k] = translate_coors[k - 1];
                    }
                    min_dists[j] = current_max_dist;
                    translate_coors[j] = Some(current_translate_coor);
                    break;
                }
            }
            current_ind = next_ind;
        }
        // every iteration inserts (current_max_dist < f64::MAX), so all entries are set
        translate_coors
            .into_iter()
            .map(|p| p.expect("filled"))
            .collect()
    }

    /// Shrinks the shape by offset; the result will not be empty.
    fn shrink(&self, offset: f64) -> TileShape {
        let result = self.offset_tile(-offset);
        if result.is_empty() {
            let centre_box = self.centre_of_gravity().bounding_box();
            return self.intersection_int_box_tile(&centre_box);
        }
        result
    }

    /// Maximum of the edge widths of the shape (only defined for bounded shapes).
    fn length(&self) -> f64 {
        if !self.is_bounded() {
            return i32::MAX as f64;
        }
        let dimension = self.dimension();
        if dimension <= 0 {
            return 0.0;
        }
        if dimension == 1 {
            return self.circumference() / 2.0;
        }
        // now the shape is 2-dimensional
        let mut max_distance = -1.0;
        let mut max_distance2 = -1.0;
        let gravity_point = self.centre_of_gravity();
        for i in 0..self.border_line_count() {
            let current_distance = self.border_line(i).signed_distance(&gravity_point).abs();
            if current_distance > max_distance {
                max_distance2 = max_distance;
                max_distance = current_distance;
            } else if current_distance > max_distance2 {
                max_distance2 = current_distance;
            }
        }
        max_distance + max_distance2
    }

    /// If this shape and other have a common border piece, returns the indices in this shape and
    /// other of the touching edge lines; otherwise an empty vector.
    fn touching_sides(&self, other: &TileShape) -> Vec<i32> {
        // search the first edge line of other with reverse direction >= right
        let mut side_no2 = -1;
        let mut dir2: Option<Direction> = None;
        for i in 0..other.border_line_count() {
            let current_direction = other.border_line(i).direction();
            if current_direction.compare_to(&Direction::LEFT) >= 0 {
                side_no2 = i;
                dir2 = Some(current_direction.opposite());
                break;
            }
        }
        let mut dir2 = match dir2 {
            Some(d) => d,
            None => {
                log::warn!("touching_side : dir2 not found");
                return Vec::new();
            }
        };
        let mut side_no1 = 0;
        let mut dir1 = self.border_line(0).direction();
        let max_ind = self.border_line_count() + other.border_line_count();
        for _ in 0..max_ind {
            let compare = dir2.compare_to(&dir1);
            if compare == 0
                && self
                    .border_line(side_no1)
                    .is_equal_or_opposite(&other.border_line(side_no2))
            {
                return vec![side_no1, side_no2];
            }
            if compare >= 0 {
                // dir2 is bigger than dir1
                side_no1 = (side_no1 + 1) % self.border_line_count();
                dir1 = self.border_line(side_no1).direction();
            } else {
                // dir1 is bigger than dir2
                side_no2 = (side_no2 + 1) % other.border_line_count();
                dir2 = other.border_line(side_no2).direction().opposite();
            }
        }
        Vec::new()
    }

    /// Minimal distance of line to this shape, assuming line is on the left of this shape.
    /// Returns -1, if line is on the right of this shape or intersects its interior.
    fn distance_to_the_left(&self, line: &Line) -> f64 {
        let mut result = i32::MAX as f64;
        for i in 0..self.border_line_count() {
            let current_corner = self.corner_approx(i);
            let mut line_side = line.side_of_float_tol(&current_corner, 1.0);
            if line_side == Side::Collinear {
                line_side = line.side_of(&self.corner(i));
            }
            if line_side == Side::OnTheRight {
                // current point would be outside the result shape
                result = -1.0;
                break;
            }
            result = crate::float_point::java_min(result, line.signed_distance(&current_corner));
        }
        result
    }

    /// COLLINEAR if line intersects the interior of this shape, ON_THE_LEFT if this shape is
    /// completely on the left of line, ON_THE_RIGHT if completely on the right (Java
    /// `sideOf(Line)`).
    fn side_of(&self, line: &Line) -> Side {
        let mut on_the_left = false;
        let mut on_the_right = false;
        for i in 0..self.border_line_count() {
            let current_side = line.side_of(&self.corner(i));
            if current_side == Side::OnTheLeft {
                on_the_right = true;
            } else if current_side == Side::OnTheRight {
                on_the_left = true;
            }
            if on_the_left && on_the_right {
                return Side::Collinear;
            }
        }
        if on_the_left {
            Side::OnTheLeft
        } else {
            Side::OnTheRight
        }
    }

    fn turn_90_degree_tile(&self, factor: i32, pole: &IntPoint) -> TileShape {
        let new_lines: Vec<Line> = (0..self.border_line_count())
            .map(|i| self.border_line(i).turn_90_degree(factor, pole))
            .collect();
        TileShape::get_instance_lines(&new_lines)
    }

    fn rotate_approx_tile(&self, angle: f64, pole: &FloatPoint) -> TileShape {
        if angle == 0.0 {
            return self.to_tile_shape();
        }
        let new_corners: Vec<Point> = (0..self.border_line_count())
            .map(|i| Point::Int(self.corner_approx(i).rotate(angle, pole).round()))
            .collect();
        let corner_polygon = Polygon::new(&new_corners);
        let polygon_corners = corner_polygon.corners();
        if polygon_corners.len() >= 3 {
            TileShape::get_instance_points(polygon_corners)
        } else if polygon_corners.len() == 2 {
            let current_polyline = Polyline::from_points(polygon_corners);
            // Java: new LineSegment(polyline, 0) logs "no out of range" and the following
            // toSimplex() throws a NullPointerException.
            let current_segment = LineSegment::from_polyline(&current_polyline, 0)
                .expect("NullPointerException: LineSegment(polyline, 0) is out of range");
            TileShape::Simplex(current_segment.to_simplex())
        } else if polygon_corners.len() == 1 {
            TileShape::IntBox(polygon_corners[0].surrounding_box())
        } else {
            TileShape::Simplex(Simplex::empty())
        }
    }

    fn mirror_vertical_tile(&self, pole: &IntPoint) -> TileShape {
        let new_lines: Vec<Line> = (0..self.border_line_count())
            .map(|i| self.border_line(i).mirror_vertical(pole))
            .collect();
        TileShape::get_instance_lines(&new_lines)
    }

    fn mirror_horizontal_tile(&self, pole: &IntPoint) -> TileShape {
        let new_lines: Vec<Line> = (0..self.border_line_count())
            .map(|i| self.border_line(i).mirror_horizontal(pole))
            .collect();
        TileShape::get_instance_lines(&new_lines)
    }

    /// Border line of this shape intersecting the ray from point into direction. point is
    /// assumed to be inside this shape, otherwise -1 is returned.
    fn intersecting_border_line_no(&self, point: &Point, direction: &Direction) -> i32 {
        if !self.contains(point) {
            return -1;
        }
        let from_point = point.to_float();
        let intersection_line = Line::from_point_direction(point.clone(), direction.clone());
        let second_line_point = intersection_line.b.to_float();
        let mut result = -1;
        let mut min_distance = f32::MAX as f64;
        for i in 0..self.border_line_count() {
            let current_border_line = self.border_line(i);
            let current_intersection = current_border_line.intersection_approx(&intersection_line);
            if current_intersection.x >= i32::MAX as f64 {
                continue; // lines are parallel
            }
            let current_distance = current_intersection.distance_square(&from_point);
            if current_distance < min_distance {
                let direction_ok = current_border_line.side_of_float(&second_line_point)
                    == Side::OnTheLeft
                    || second_line_point.distance_square(&current_intersection) < current_distance;
                if direction_ok {
                    result = i;
                    min_distance = current_distance;
                }
            }
        }
        result
    }

    /// Cuts out the parts of polyline in the interior of this shape and returns the remaining
    /// pieces. Pieces completely contained in the border of this shape are not returned.
    fn cutout_polyline(&self, polyline: &Polyline) -> Vec<Polyline> {
        let intersection_no = self.entrance_points(polyline);
        let first_corner = polyline.first_corner();
        let first_corner_is_inside = self.contains_inside(&first_corner);
        if intersection_no.is_empty() {
            // no intersections
            if first_corner_is_inside {
                // polyline is contained completely in this shape
                return Vec::new();
            }
            // polyline is completely outside
            return vec![polyline.clone()];
        }
        let mut pieces: Vec<Polyline> = Vec::new();
        let mut current_intersection_no = 0usize;
        let mut current_intersection_tuple = intersection_no[current_intersection_no];
        let first_intersection = polyline.lines[current_intersection_tuple[0] as usize]
            .intersection(&self.border_line(current_intersection_tuple[1]));
        if !first_corner_is_inside {
            // calculate outside piece at start
            if first_corner != first_intersection {
                // otherwise skip 1 point outside polyline at the start
                let n = current_intersection_tuple[0] as usize;
                let mut current_lines: Vec<Line> = polyline.lines[..n + 1].to_vec();
                // close the polyline piece with the intersected edge line.
                current_lines.push(self.border_line(current_intersection_tuple[1]));
                let current_piece = Polyline::from_lines(current_lines);
                if !current_piece.is_empty() {
                    pieces.push(current_piece);
                }
            }
            current_intersection_no += 1;
        }
        while current_intersection_no + 1 < intersection_no.len() {
            // calculate the next outside polyline piece
            current_intersection_tuple = intersection_no[current_intersection_no];
            let next_intersection_tuple = intersection_no[current_intersection_no + 1];
            let curr_no = current_intersection_tuple[0];
            let next_no = next_intersection_tuple[0];
            // check that at least 1 corner of polyline between the intersections is not
            // contained in this shape. Otherwise, the part of polyline between these
            // intersections is completely contained in the border and can be ignored
            let mut insert_piece = false;
            for i in curr_no + 1..next_no {
                if self.is_outside(&polyline.corner(i)) {
                    insert_piece = true;
                    break;
                }
            }
            if insert_piece {
                let len = (next_no - curr_no + 3) as usize;
                let mut current_lines: Vec<Line> = Vec::with_capacity(len);
                current_lines.push(self.border_line(current_intersection_tuple[1]));
                current_lines.extend(
                    polyline.lines[curr_no as usize..curr_no as usize + len - 2]
                        .iter()
                        .cloned(),
                );
                current_lines.push(self.border_line(next_intersection_tuple[1]));
                let current_piece = Polyline::from_lines(current_lines);
                if !current_piece.is_empty() {
                    pieces.push(current_piece);
                }
            }
            current_intersection_no += 2;
        }
        if current_intersection_no < intersection_no.len() {
            // calculate outside piece at end
            current_intersection_tuple = intersection_no[current_intersection_no];
            let n = current_intersection_tuple[0] as usize;
            let mut current_lines: Vec<Line> = Vec::with_capacity(polyline.lines.len() - n + 1);
            current_lines.push(self.border_line(current_intersection_tuple[1]));
            current_lines.extend(polyline.lines[n..].iter().cloned());
            let current_piece = Polyline::from_lines(current_lines);
            if !current_piece.is_empty() {
                pieces.push(current_piece);
            }
        }
        pieces
    }

    /// Tuples (line segment number of polyline, edge line number of this shape) of the points
    /// where polyline enters or leaves the interior of this shape.
    fn entrance_points(&self, polyline: &Polyline) -> Vec<[i32; 2]> {
        let mut result: Vec<[i32; 2]> = Vec::new();
        let mut prev_intersection_line_no = -1;
        let mut prev_intersection_edge_no = -1;
        let tile = self.to_tile_shape();
        for line_index in 1..polyline.lines.len() as i32 - 1 {
            let current_line_seg =
                LineSegment::from_polyline(polyline, line_index).expect("index in range");
            let current_intersections = current_line_seg.border_intersections(&tile);
            for edge_index in current_intersections {
                if line_index != prev_intersection_line_no
                    || edge_index != prev_intersection_edge_no
                {
                    result.push([line_index, edge_index]);
                    prev_intersection_line_no = line_index;
                    prev_intersection_edge_no = edge_index;
                }
            }
        }
        result
    }

    /// Returns a division of this shape into convex pieces (this shape itself).
    fn split_to_convex(&self) -> Vec<TileShape> {
        vec![self.to_tile_shape()]
    }

    /// Divides this shape into sections with width and height at most max_section_width of about
    /// equal size.
    fn divide_into_sections_tile(&self, max_section_width: f64) -> Vec<TileShape> {
        if self.is_empty() {
            return vec![self.to_tile_shape()];
        }
        let section_boxes = self.bounding_box().divide_into_sections(max_section_width);
        let mut section_list = Vec::new();
        for b in section_boxes {
            // Java: this.intersectionWithSimplify(TileShape) -> this.intersection(TileShape)
            let current_section = self.intersection_with_simplify(&TileShape::IntBox(b));
            if current_section.dimension() == 2 {
                section_list.push(current_section);
            }
        }
        section_list
    }

    /// Checks, if line_segment has a common point with the interior of this shape.
    fn is_intersected_interior_by(&self, line_segment: &LineSegment) -> bool {
        self.is_intersected_interior_by_points(
            &line_segment.start_point(),
            &line_segment.end_point(),
            line_segment.get_line(),
        )
    }

    /// Checks if the line segment defined by start_point, end_point and line has a common point
    /// with the interior of this shape.
    fn is_intersected_interior_by_points(
        &self,
        start_point: &Point,
        end_point: &Point,
        line: &Line,
    ) -> bool {
        let float_start_point = start_point.to_float();
        let float_end_point = end_point.to_float();
        let n = self.border_line_count().max(0) as usize;
        let mut start_sides: Vec<Side> = Vec::with_capacity(n);
        let mut end_sides: Vec<Side> = Vec::with_capacity(n);
        for i in 0..n as i32 {
            let current_border_line = self.border_line(i);
            let mut s = current_border_line.side_of_float_tol(&float_start_point, 1.0);
            if s == Side::Collinear {
                s = current_border_line.side_of(start_point);
            }
            let mut e = current_border_line.side_of_float_tol(&float_end_point, 1.0);
            if e == Side::Collinear {
                e = current_border_line.side_of(end_point);
            }
            if s != Side::OnTheRight && e != Side::OnTheRight {
                // both endpoints are outside the border line, no intersection possible
                return false;
            }
            start_sides.push(s);
            end_sides.push(e);
        }
        if start_sides.iter().all(|s| *s == Side::OnTheRight) {
            return true;
        }
        if end_sides.iter().all(|s| *s == Side::OnTheRight) {
            return true;
        }
        let segment_line = line;
        // Check, if this line segment intersects a border line of shape.
        for i in 0..n {
            let s = start_sides[i];
            let e = end_sides[i];
            if s != e {
                if s == Side::Collinear && e == Side::OnTheLeft
                    || e == Side::Collinear && s == Side::OnTheLeft
                {
                    // the interior of shape is not intersected.
                    continue;
                }
                let mut prev_corner_side =
                    segment_line.side_of_float_tol(&self.corner_approx(i as i32), 1.0);
                if prev_corner_side == Side::Collinear {
                    prev_corner_side = segment_line.side_of(&self.corner(i as i32));
                }
                let next_corner_index = if i == n - 1 { 0 } else { i + 1 } as i32;
                let mut next_corner_side =
                    segment_line.side_of_float_tol(&self.corner_approx(next_corner_index), 1.0);
                if next_corner_side == Side::Collinear {
                    next_corner_side = segment_line.side_of(&self.corner(next_corner_index));
                }
                if prev_corner_side == Side::OnTheLeft && next_corner_side == Side::OnTheRight
                    || prev_corner_side == Side::OnTheRight && next_corner_side == Side::OnTheLeft
                {
                    // this line segment crosses a border line of shape
                    return true;
                }
            }
        }
        false
    }

    /// Intersection with simplification of the result.
    fn intersection_with_simplify(&self, other: &TileShape) -> TileShape {
        self.to_tile_shape().intersection(other).simplify()
    }
}

/// Closed hierarchy `TileShape = IntBox | IntOctagon | Simplex` (`RegularTileShape` =
/// IntBox | IntOctagon).
#[derive(Clone, Debug)]
pub enum TileShape {
    IntBox(IntBox),
    IntOctagon(IntOctagon),
    Simplex(Simplex),
}

macro_rules! tile_dispatch {
    ($self:expr, $s:ident => $e:expr) => {
        match $self {
            TileShape::IntBox($s) => $e,
            TileShape::IntOctagon($s) => $e,
            TileShape::Simplex($s) => $e,
        }
    };
}

impl TileShape {
    // ----- static factory methods (Java TileShape.getInstance overloads) -----

    /// Creates a Simplex as intersection of the half-planes defined by directed lines, and
    /// simplifies it.
    pub fn get_instance_lines(lines: &[Line]) -> TileShape {
        Simplex::get_instance(lines).simplify()
    }

    /// Creates a TileShape from the corners of a convex polygon. May work only for IntPoints.
    pub fn get_instance_points(convex_polygon: &[Point]) -> TileShape {
        let n = convex_polygon.len();
        assert!(
            n > 0,
            "ArrayIndexOutOfBoundsException: empty convex polygon"
        );
        let mut lines: Vec<Line> = Vec::with_capacity(n);
        for j in 0..n - 1 {
            lines.push(Line::new(
                convex_polygon[j].clone(),
                convex_polygon[j + 1].clone(),
            ));
        }
        lines.push(Line::new(
            convex_polygon[n - 1].clone(),
            convex_polygon[0].clone(),
        ));
        Self::get_instance_lines(&lines)
    }

    /// Creates a half-plane from a directed line (not simplified).
    pub fn get_instance_line(line: &Line) -> TileShape {
        TileShape::Simplex(Simplex::get_instance(std::slice::from_ref(line)))
    }

    /// Creates a normalized IntOctagon from the input values.
    #[allow(clippy::too_many_arguments)]
    pub fn get_instance_octagon(
        lx: i32,
        ly: i32,
        rx: i32,
        uy: i32,
        ulx: i32,
        lrx: i32,
        llx: i32,
        urx: i32,
    ) -> IntOctagon {
        IntOctagon::new(lx, ly, rx, uy, ulx, lrx, llx, urx).normalize()
    }

    /// Creates a box-like convex shape (as IntOctagon, not normalized).
    pub fn get_instance_box(
        lower_left_x: i32,
        lower_left_y: i32,
        upper_right_x: i32,
        upper_right_y: i32,
    ) -> IntOctagon {
        IntBox::new(lower_left_x, lower_left_y, upper_right_x, upper_right_y).to_int_octagon()
    }

    /// Creates the smallest IntBox containing point.
    pub fn get_instance_point(point: &Point) -> IntBox {
        point.surrounding_box()
    }

    // ----- downcasts -----

    pub fn as_int_box(&self) -> Option<&IntBox> {
        match self {
            TileShape::IntBox(b) => Some(b),
            _ => None,
        }
    }

    pub fn as_int_octagon(&self) -> Option<&IntOctagon> {
        match self {
            TileShape::IntOctagon(o) => Some(o),
            _ => None,
        }
    }

    pub fn as_simplex(&self) -> Option<&Simplex> {
        match self {
            TileShape::Simplex(s) => Some(s),
            _ => None,
        }
    }

    /// Java `instanceof RegularTileShape`.
    pub fn as_regular_tile_shape(&self) -> Option<RegularTileShape> {
        match self {
            TileShape::IntBox(b) => Some(RegularTileShape::IntBox(*b)),
            TileShape::IntOctagon(o) => Some(RegularTileShape::IntOctagon(*o)),
            TileShape::Simplex(_) => None,
        }
    }

    pub fn to_shape(&self) -> Shape {
        Shape::Tile(self.clone())
    }

    pub fn to_convex_shape(&self) -> ConvexShape {
        ConvexShape::Tile(self.clone())
    }

    pub fn to_polyline_shape(&self) -> PolylineShape {
        PolylineShape::Tile(self.clone())
    }

    // ----- double dispatch -----

    /// Returns the intersection of this shape with other (Java double dispatch order preserved).
    pub fn intersection(&self, other: &TileShape) -> TileShape {
        // this.intersection(other) -> other.intersection(<type of this> this)
        match (self, other) {
            (TileShape::IntBox(a), TileShape::IntBox(b)) => {
                TileShape::IntBox(b.intersection_int_box(a))
            }
            (TileShape::IntBox(a), TileShape::IntOctagon(b)) => {
                TileShape::IntOctagon(b.intersection_int_box(a))
            }
            (TileShape::IntBox(a), TileShape::Simplex(b)) => {
                TileShape::Simplex(b.intersection_int_box(a))
            }
            (TileShape::IntOctagon(a), TileShape::IntBox(b)) => {
                TileShape::IntOctagon(b.intersection_int_octagon(a))
            }
            (TileShape::IntOctagon(a), TileShape::IntOctagon(b)) => {
                TileShape::IntOctagon(b.intersection_int_octagon(a))
            }
            (TileShape::IntOctagon(a), TileShape::Simplex(b)) => {
                TileShape::Simplex(b.intersection_int_octagon(a))
            }
            (TileShape::Simplex(a), TileShape::IntBox(b)) => {
                TileShape::Simplex(b.intersection_simplex(a))
            }
            (TileShape::Simplex(a), TileShape::IntOctagon(b)) => {
                TileShape::Simplex(b.intersection_simplex(a))
            }
            (TileShape::Simplex(a), TileShape::Simplex(b)) => {
                TileShape::Simplex(b.intersection_simplex(a))
            }
        }
    }

    /// Java overload `intersection(IntBox)` of this shape's class.
    pub fn intersection_int_box(&self, other: &IntBox) -> TileShape {
        tile_dispatch!(self, s => TileShapeImpl::intersection_int_box_tile(s, other))
    }

    /// Java overload `intersection(IntOctagon)` of this shape's class.
    pub fn intersection_int_octagon(&self, other: &IntOctagon) -> TileShape {
        tile_dispatch!(self, s => TileShapeImpl::intersection_int_octagon_tile(s, other))
    }

    /// Java overload `intersection(Simplex)` of this shape's class.
    pub fn intersection_simplex(&self, other: &Simplex) -> TileShape {
        tile_dispatch!(self, s => TileShapeImpl::intersection_simplex_tile(s, other))
    }

    /// Checks, if this shape and other have a nonempty intersection.
    pub fn intersects(&self, other: &Shape) -> bool {
        // this.intersects(other) -> other.intersects(<type of this> this)
        match self {
            TileShape::IntBox(b) => other.intersects_int_box(b),
            TileShape::IntOctagon(o) => other.intersects_int_octagon(o),
            TileShape::Simplex(s) => other.intersects_simplex(s),
        }
    }

    /// Java overload `intersects(TileShape)` resolves to `intersects(Shape)`.
    pub fn intersects_tile(&self, other: &TileShape) -> bool {
        self.intersects(&Shape::Tile(other.clone()))
    }

    pub fn intersects_int_box(&self, other: &IntBox) -> bool {
        tile_dispatch!(self, s => TileShapeImpl::intersects_int_box(s, other))
    }

    pub fn intersects_int_octagon(&self, other: &IntOctagon) -> bool {
        tile_dispatch!(self, s => TileShapeImpl::intersects_int_octagon(s, other))
    }

    pub fn intersects_simplex(&self, other: &Simplex) -> bool {
        tile_dispatch!(self, s => TileShapeImpl::intersects_simplex(s, other))
    }

    pub fn intersects_circle(&self, other: &Circle) -> bool {
        tile_dispatch!(self, s => TileShapeImpl::intersects_circle(s, other))
    }

    /// Cuts shape out of this shape and divides the result into convex pieces (None where Java
    /// returns null).
    pub fn cutout(&self, shape: &TileShape) -> Option<Vec<TileShape>> {
        tile_dispatch!(self, s => TileShapeImpl::cutout_tile(s, shape))
    }

    /// Java `cutoutFrom(IntBox)`.
    pub fn cutout_from_int_box(&self, shape: &IntBox) -> Option<Vec<TileShape>> {
        tile_dispatch!(self, s => TileShapeImpl::cutout_from_int_box_tile(s, shape))
    }

    /// Java `cutoutFrom(IntOctagon)`.
    pub fn cutout_from_int_octagon(&self, shape: &IntOctagon) -> Option<Vec<TileShape>> {
        tile_dispatch!(self, s => TileShapeImpl::cutout_from_int_octagon_tile(s, shape))
    }

    /// Java `cutoutFrom(Simplex)`.
    pub fn cutout_from_simplex(&self, shape: &Simplex) -> Option<Vec<TileShape>> {
        tile_dispatch!(self, s => TileShapeImpl::cutout_from_simplex_tile(s, shape))
    }

    // ----- forwarding of the PolylineShape / TileShape methods -----

    pub fn border_line_count(&self) -> i32 {
        tile_dispatch!(self, s => PolylineShapeImpl::border_line_count(s))
    }
    pub fn corner(&self, no: i32) -> Point {
        tile_dispatch!(self, s => PolylineShapeImpl::corner(s, no))
    }
    pub fn border_line(&self, no: i32) -> Line {
        tile_dispatch!(self, s => PolylineShapeImpl::border_line(s, no))
    }
    pub fn corner_is_bounded(&self, no: i32) -> bool {
        tile_dispatch!(self, s => PolylineShapeImpl::corner_is_bounded(s, no))
    }
    pub fn is_empty(&self) -> bool {
        tile_dispatch!(self, s => PolylineShapeImpl::is_empty(s))
    }
    pub fn is_bounded(&self) -> bool {
        tile_dispatch!(self, s => PolylineShapeImpl::is_bounded(s))
    }
    pub fn dimension(&self) -> i32 {
        tile_dispatch!(self, s => PolylineShapeImpl::dimension(s))
    }
    pub fn bounding_box(&self) -> IntBox {
        tile_dispatch!(self, s => PolylineShapeImpl::bounding_box(s))
    }
    pub fn corner_approx(&self, no: i32) -> FloatPoint {
        tile_dispatch!(self, s => PolylineShapeImpl::corner_approx(s, no))
    }
    pub fn corner_approx_arr(&self) -> Vec<FloatPoint> {
        tile_dispatch!(self, s => PolylineShapeImpl::corner_approx_arr(s))
    }
    pub fn bounded_corners(&self) -> Vec<Point> {
        tile_dispatch!(self, s => PolylineShapeImpl::bounded_corners(s))
    }
    pub fn equals_corner(&self, point: &Point) -> i32 {
        tile_dispatch!(self, s => PolylineShapeImpl::equals_corner(s, point))
    }
    pub fn circumference(&self) -> f64 {
        tile_dispatch!(self, s => PolylineShapeImpl::circumference(s))
    }
    pub fn centre_of_gravity(&self) -> FloatPoint {
        tile_dispatch!(self, s => PolylineShapeImpl::centre_of_gravity(s))
    }
    pub fn is_contained_in(&self, b: &IntBox) -> bool {
        tile_dispatch!(self, s => PolylineShapeImpl::is_contained_in(s, b))
    }
    pub fn index_of_left_most_corner(&self, from_point: &FloatPoint) -> i32 {
        tile_dispatch!(self, s => PolylineShapeImpl::index_of_left_most_corner(s, from_point))
    }
    pub fn index_of_right_most_corner(&self, from_point: &FloatPoint) -> i32 {
        tile_dispatch!(self, s => PolylineShapeImpl::index_of_right_most_corner(s, from_point))
    }
    pub fn polar_line_segment(&self, from_point: &FloatPoint) -> Option<FloatLine> {
        tile_dispatch!(self, s => PolylineShapeImpl::polar_line_segment(s, from_point))
    }
    pub fn prev_no(&self, no: i32) -> i32 {
        tile_dispatch!(self, s => PolylineShapeImpl::prev_no(s, no))
    }
    pub fn next_no(&self, no: i32) -> i32 {
        tile_dispatch!(self, s => PolylineShapeImpl::next_no(s, no))
    }
    /// Java `intersects(Line)`.
    pub fn intersects_line(&self, line: &Line) -> bool {
        tile_dispatch!(self, s => PolylineShapeImpl::intersects_line(s, line))
    }
    pub fn left_most_corner(&self, from_point: &Point) -> Point {
        tile_dispatch!(self, s => PolylineShapeImpl::left_most_corner(s, from_point))
    }
    pub fn right_most_corner(&self, from_point: &Point) -> Point {
        tile_dispatch!(self, s => PolylineShapeImpl::right_most_corner(s, from_point))
    }

    pub fn simplify(&self) -> TileShape {
        tile_dispatch!(self, s => TileShapeImpl::simplify(s))
    }
    pub fn get_id(&self) -> i32 {
        tile_dispatch!(self, s => TileShapeImpl::get_id(s))
    }
    pub fn is_int_box(&self) -> bool {
        tile_dispatch!(self, s => TileShapeImpl::is_int_box(s))
    }
    pub fn is_int_octagon(&self) -> bool {
        tile_dispatch!(self, s => TileShapeImpl::is_int_octagon(s))
    }
    pub fn border_line_index(&self, line: &Line) -> i32 {
        tile_dispatch!(self, s => TileShapeImpl::border_line_index(s, line))
    }
    pub fn to_simplex(&self) -> Simplex {
        tile_dispatch!(self, s => TileShapeImpl::to_simplex(s))
    }
    pub fn offset(&self, distance: f64) -> TileShape {
        tile_dispatch!(self, s => TileShapeImpl::offset_tile(s, distance))
    }
    pub fn max_width(&self) -> f64 {
        tile_dispatch!(self, s => TileShapeImpl::max_width(s))
    }
    pub fn min_width(&self) -> f64 {
        tile_dispatch!(self, s => TileShapeImpl::min_width(s))
    }
    pub fn translate_by(&self, vector: &Vector) -> TileShape {
        tile_dispatch!(self, s => TileShapeImpl::translate_by_tile(s, vector))
    }
    /// Java `boundingOctagon()`; None only for unbounded simplices.
    pub fn bounding_octagon(&self) -> Option<IntOctagon> {
        tile_dispatch!(self, s => TileShapeImpl::bounding_octagon_opt(s))
    }
    /// Java `boundingTile()` (this shape).
    pub fn bounding_tile(&self) -> TileShape {
        self.clone()
    }
    pub fn enlarge(&self, offset: f64) -> TileShape {
        tile_dispatch!(self, s => TileShapeImpl::enlarge_tile(s, offset))
    }
    pub fn bounding_shape(&self, dirs: &ShapeBoundingDirections) -> Option<RegularTileShape> {
        tile_dispatch!(self, s => TileShapeImpl::bounding_shape_tile(s, dirs))
    }
    pub fn area(&self) -> f64 {
        tile_dispatch!(self, s => TileShapeImpl::area(s))
    }
    pub fn is_outside(&self, point: &Point) -> bool {
        tile_dispatch!(self, s => TileShapeImpl::is_outside(s, point))
    }
    pub fn contains(&self, point: &Point) -> bool {
        tile_dispatch!(self, s => TileShapeImpl::contains(s, point))
    }
    pub fn contains_float(&self, point: &FloatPoint) -> bool {
        tile_dispatch!(self, s => TileShapeImpl::contains_float(s, point))
    }
    pub fn contains_float_tol(&self, point: &FloatPoint, tolerance: f64) -> bool {
        tile_dispatch!(self, s => TileShapeImpl::contains_float_tol(s, point, tolerance))
    }
    /// Java `contains(TileShape)`. (For RegularTileShape arguments on a RegularTileShape Java
    /// selects `contains(RegularTileShape)`, see [`RegularTileShape::contains`].)
    pub fn contains_tile_shape(&self, other: &TileShape) -> bool {
        tile_dispatch!(self, s => TileShapeImpl::contains_tile_shape(s, other))
    }
    pub fn contains_inside(&self, point: &Point) -> bool {
        tile_dispatch!(self, s => TileShapeImpl::contains_inside(s, point))
    }
    pub fn side_of_border(&self, point: &FloatPoint, tolerance: f64) -> Side {
        tile_dispatch!(self, s => TileShapeImpl::side_of_border(s, point, tolerance))
    }
    pub fn contains_on_border_line_no(&self, point: &Point) -> i32 {
        tile_dispatch!(self, s => TileShapeImpl::contains_on_border_line_no(s, point))
    }
    pub fn contains_on_border(&self, point: &Point) -> bool {
        tile_dispatch!(self, s => TileShapeImpl::contains_on_border(s, point))
    }
    pub fn contains_approx(&self, other: &TileShape) -> bool {
        tile_dispatch!(self, s => TileShapeImpl::contains_approx(s, other))
    }
    pub fn distance(&self, point: &FloatPoint) -> f64 {
        tile_dispatch!(self, s => TileShapeImpl::distance(s, point))
    }
    pub fn border_distance(&self, point: &FloatPoint) -> f64 {
        tile_dispatch!(self, s => TileShapeImpl::border_distance(s, point))
    }
    pub fn smallest_radius(&self) -> f64 {
        tile_dispatch!(self, s => TileShapeImpl::smallest_radius(s))
    }
    pub fn nearest_point(&self, from_point: &Point) -> Option<Point> {
        tile_dispatch!(self, s => TileShapeImpl::nearest_point(s, from_point))
    }
    pub fn nearest_point_approx(&self, from_point: &FloatPoint) -> Option<FloatPoint> {
        tile_dispatch!(self, s => TileShapeImpl::nearest_point_approx(s, from_point))
    }
    pub fn nearest_border_point(&self, from_point: &Point) -> Option<Point> {
        tile_dispatch!(self, s => TileShapeImpl::nearest_border_point(s, from_point))
    }
    pub fn nearest_border_point_approx(&self, from_point: &FloatPoint) -> Option<FloatPoint> {
        tile_dispatch!(self, s => TileShapeImpl::nearest_border_point_approx(s, from_point))
    }
    pub fn nearest_border_points_approx(
        &self,
        from_point: &FloatPoint,
        count: i32,
    ) -> Vec<Option<FloatPoint>> {
        tile_dispatch!(self, s => TileShapeImpl::nearest_border_points_approx(s, from_point, count))
    }
    pub fn index_of_nearest_corner(&self, from_point: &Point) -> i32 {
        tile_dispatch!(self, s => TileShapeImpl::index_of_nearest_corner(s, from_point))
    }
    pub fn diagonal_corner_segment(&self) -> Option<FloatLine> {
        tile_dispatch!(self, s => TileShapeImpl::diagonal_corner_segment(s))
    }
    pub fn nearest_relative_outside_locations(
        &self,
        shape: &TileShape,
        count: i32,
    ) -> Vec<FloatPoint> {
        tile_dispatch!(self, s => TileShapeImpl::nearest_relative_outside_locations(s, shape, count))
    }
    pub fn shrink(&self, offset: f64) -> TileShape {
        tile_dispatch!(self, s => TileShapeImpl::shrink(s, offset))
    }
    pub fn length(&self) -> f64 {
        tile_dispatch!(self, s => TileShapeImpl::length(s))
    }
    pub fn touching_sides(&self, other: &TileShape) -> Vec<i32> {
        tile_dispatch!(self, s => TileShapeImpl::touching_sides(s, other))
    }
    pub fn distance_to_the_left(&self, line: &Line) -> f64 {
        tile_dispatch!(self, s => TileShapeImpl::distance_to_the_left(s, line))
    }
    /// Java `sideOf(Line)`.
    pub fn side_of(&self, line: &Line) -> Side {
        tile_dispatch!(self, s => TileShapeImpl::side_of(s, line))
    }
    pub fn turn_90_degree(&self, factor: i32, pole: &IntPoint) -> TileShape {
        tile_dispatch!(self, s => TileShapeImpl::turn_90_degree_tile(s, factor, pole))
    }
    pub fn rotate_approx(&self, angle: f64, pole: &FloatPoint) -> TileShape {
        tile_dispatch!(self, s => TileShapeImpl::rotate_approx_tile(s, angle, pole))
    }
    pub fn mirror_vertical(&self, pole: &IntPoint) -> TileShape {
        tile_dispatch!(self, s => TileShapeImpl::mirror_vertical_tile(s, pole))
    }
    pub fn mirror_horizontal(&self, pole: &IntPoint) -> TileShape {
        tile_dispatch!(self, s => TileShapeImpl::mirror_horizontal_tile(s, pole))
    }
    pub fn intersecting_border_line_no(&self, point: &Point, direction: &Direction) -> i32 {
        tile_dispatch!(self, s => TileShapeImpl::intersecting_border_line_no(s, point, direction))
    }
    /// Java `cutout(Polyline)`.
    pub fn cutout_polyline(&self, polyline: &Polyline) -> Vec<Polyline> {
        tile_dispatch!(self, s => TileShapeImpl::cutout_polyline(s, polyline))
    }
    pub fn entrance_points(&self, polyline: &Polyline) -> Vec<[i32; 2]> {
        tile_dispatch!(self, s => TileShapeImpl::entrance_points(s, polyline))
    }
    pub fn split_to_convex(&self) -> Vec<TileShape> {
        tile_dispatch!(self, s => TileShapeImpl::split_to_convex(s))
    }
    pub fn divide_into_sections(&self, max_section_width: f64) -> Vec<TileShape> {
        tile_dispatch!(self, s => TileShapeImpl::divide_into_sections_tile(s, max_section_width))
    }
    pub fn is_intersected_interior_by(&self, line_segment: &LineSegment) -> bool {
        tile_dispatch!(self, s => TileShapeImpl::is_intersected_interior_by(s, line_segment))
    }
    pub fn is_intersected_interior_by_points(
        &self,
        start_point: &Point,
        end_point: &Point,
        line: &Line,
    ) -> bool {
        tile_dispatch!(self, s => TileShapeImpl::is_intersected_interior_by_points(s, start_point, end_point, line))
    }
    pub fn intersection_with_simplify(&self, other: &TileShape) -> TileShape {
        self.intersection(other).simplify()
    }
    /// Returns this shape as border (Java `getBorder()`).
    pub fn get_border(&self) -> PolylineShape {
        PolylineShape::Tile(self.clone())
    }
    /// Returns the (empty) holes of this shape.
    pub fn get_holes(&self) -> Vec<Shape> {
        Vec::new()
    }
}

impl From<IntBox> for TileShape {
    fn from(b: IntBox) -> Self {
        TileShape::IntBox(b)
    }
}

impl From<IntOctagon> for TileShape {
    fn from(o: IntOctagon) -> Self {
        TileShape::IntOctagon(o)
    }
}

impl From<Simplex> for TileShape {
    fn from(s: Simplex) -> Self {
        TileShape::Simplex(s)
    }
}

impl From<RegularTileShape> for TileShape {
    fn from(r: RegularTileShape) -> Self {
        r.to_tile_shape()
    }
}
