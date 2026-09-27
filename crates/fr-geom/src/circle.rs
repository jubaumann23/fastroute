//! Port of `Circle.java`: circle shapes in the plane.

use std::f64::consts::PI;

use crate::direction::Direction;
use crate::float_point::{java_max, FloatPoint};
use crate::int_box::IntBox;
use crate::int_octagon::IntOctagon;
use crate::int_point::IntPoint;
use crate::int_vector::IntVector;
use crate::java_compat::math_round_i32;
use crate::line::Line;
use crate::point::Point;
use crate::polyline::Polyline;
use crate::regular_tile_shape::RegularTileShape;
use crate::shape::Shape;
use crate::shape_bounding_directions::ShapeBoundingDirections;
use crate::simplex::Simplex;
use crate::tile_shape::{TileShape, TileShapeImpl};
use crate::vector::Vector;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Circle {
    pub center: IntPoint,
    pub radius: i32,
}

impl Circle {
    /// Creates a new instance of Circle (a negative radius is negated with a warning).
    pub fn new(center: IntPoint, radius: i32) -> Circle {
        let radius = if radius < 0 {
            log::warn!("Circle: unexpected negative radius");
            -radius
        } else {
            radius
        };
        Circle { center, radius }
    }

    pub fn is_empty(&self) -> bool {
        false
    }

    pub fn is_bounded(&self) -> bool {
        true
    }

    pub fn dimension(&self) -> i32 {
        if self.radius == 0 {
            // circle is reduced to a point
            return 0;
        }
        2
    }

    pub fn circumference(&self) -> f64 {
        2.0 * PI * self.radius as f64
    }

    pub fn area(&self) -> f64 {
        (PI * self.radius as f64) * self.radius as f64
    }

    pub fn centre_of_gravity(&self) -> FloatPoint {
        self.center.to_float()
    }

    #[inline]
    fn radius_square(&self) -> f64 {
        self.radius as f64 * self.radius as f64
    }

    pub fn is_outside(&self, point: &Point) -> bool {
        let fp = point.to_float();
        fp.distance_square(&self.center.to_float()) > self.radius_square()
    }

    pub fn contains(&self, point: &Point) -> bool {
        !self.is_outside(point)
    }

    pub fn contains_float(&self, point: &FloatPoint) -> bool {
        point.distance_square(&self.center.to_float()) <= self.radius_square()
    }

    pub fn contains_inside(&self, point: &Point) -> bool {
        let fp = point.to_float();
        fp.distance_square(&self.center.to_float()) < self.radius_square()
    }

    pub fn contains_on_border(&self, point: &Point) -> bool {
        let fp = point.to_float();
        fp.distance_square(&self.center.to_float()) == self.radius_square()
    }

    pub fn distance(&self, point: &FloatPoint) -> f64 {
        let d = point.distance(&self.center.to_float()) - self.radius as f64;
        java_max(d, 0.0)
    }

    pub fn smallest_radius(&self) -> f64 {
        self.radius as f64
    }

    pub fn bounding_box(&self) -> IntBox {
        IntBox::new(
            self.center.x - self.radius,
            self.center.y - self.radius,
            self.center.x + self.radius,
            self.center.y + self.radius,
        )
    }

    pub fn bounding_octagon(&self) -> IntOctagon {
        let left_x = self.center.x - self.radius;
        let right_x = self.center.x + self.radius;
        let bottom_y = self.center.y - self.radius;
        let top_y = self.center.y + self.radius;

        let sqrt2_minus1 = 2f64.sqrt() - 1.0;
        let ceil_corner_value = (sqrt2_minus1 * self.radius as f64).ceil() as i32;
        let floor_corner_value = (sqrt2_minus1 * self.radius as f64).floor() as i32;

        let upper_left_diagonal_x = left_x - (self.center.y + floor_corner_value);
        let lower_right_diagonal_x = right_x - (self.center.y - ceil_corner_value);
        let lower_left_diagonal_x = left_x + (self.center.y - floor_corner_value);
        let upper_right_diagonal_x = right_x + (self.center.y + ceil_corner_value);
        IntOctagon::new(
            left_x,
            bottom_y,
            right_x,
            top_y,
            upper_left_diagonal_x,
            lower_right_diagonal_x,
            lower_left_diagonal_x,
            upper_right_diagonal_x,
        )
    }

    /// Java `boundingTile()`: the bounding octagon.
    pub fn bounding_tile(&self) -> TileShape {
        TileShape::IntOctagon(self.bounding_octagon())
    }

    /// Creates a bounding tile shape around this circle, so that the length of the line segments
    /// of the tile is at most max_segment_length (Java `boundingTile(int)`).
    pub fn bounding_tile_max(&self, max_segment_length: i32) -> TileShape {
        let quadrant_division_count = self.radius / max_segment_length + 1;
        if quadrant_division_count <= 2 {
            return TileShape::IntOctagon(self.bounding_octagon());
        }
        let q = quadrant_division_count as usize;
        let mut tangent_line_arr: Vec<Option<Line>> = vec![None; q * 4];
        let center = Point::Int(self.center);
        for i in 0..q {
            // calculate the tangential points in the first quadrant
            let border_delta = if i == 0 {
                IntVector::new(self.radius, 0)
            } else {
                let current_angle = i as f64 * PI / (2.0 * quadrant_division_count as f64);
                let current_x = (current_angle.sin() * self.radius as f64).ceil() as i32;
                let current_y = (current_angle.cos() * self.radius as f64).ceil() as i32;
                IntVector::new(current_x, current_y)
            };
            let current_a = center.translate_by(&Vector::Int(border_delta));
            let current_b = current_a.turn_90_degree(1, &center);
            let current_direction = Direction::get_instance(&current_b.difference_by(&center));
            let current_tangent = Line::from_point_direction(current_a, current_direction);
            tangent_line_arr[2 * q + i] = Some(current_tangent.turn_90_degree(1, &self.center));
            tangent_line_arr[3 * q + i] = Some(current_tangent.turn_90_degree(2, &self.center));
            tangent_line_arr[i] = Some(current_tangent.turn_90_degree(3, &self.center));
            tangent_line_arr[q + i] = Some(current_tangent);
        }
        let lines: Vec<Line> = tangent_line_arr
            .into_iter()
            .map(|l| l.expect("filled"))
            .collect();
        TileShape::get_instance_lines(&lines)
    }

    pub fn is_contained_in(&self, b: &IntBox) -> bool {
        if b.ll.x > self.center.x - self.radius {
            return false;
        }
        if b.ll.y > self.center.y - self.radius {
            return false;
        }
        if b.ur.x < self.center.x + self.radius {
            return false;
        }
        b.ur.y >= self.center.y + self.radius
    }

    pub fn turn_90_degree(&self, factor: i32, pole: &IntPoint) -> Circle {
        let new_center = Point::Int(self.center)
            .turn_90_degree(factor, &Point::Int(*pole))
            .as_int();
        Circle::new(new_center, self.radius)
    }

    pub fn rotate_approx(&self, angle: f64, pole: &FloatPoint) -> Circle {
        let new_center = self.center.to_float().rotate(angle, pole).round();
        Circle::new(new_center, self.radius)
    }

    pub fn mirror_vertical(&self, pole: &IntPoint) -> Circle {
        let new_center = Point::Int(self.center)
            .mirror_vertical(&Point::Int(*pole))
            .as_int();
        Circle::new(new_center, self.radius)
    }

    pub fn mirror_horizontal(&self, pole: &IntPoint) -> Circle {
        let new_center = Point::Int(self.center)
            .mirror_horizontal(&Point::Int(*pole))
            .as_int();
        Circle::new(new_center, self.radius)
    }

    pub fn max_width(&self) -> f64 {
        (2 * self.radius) as f64
    }

    pub fn min_width(&self) -> f64 {
        (2 * self.radius) as f64
    }

    pub fn bounding_shape(&self, dirs: &ShapeBoundingDirections) -> RegularTileShape {
        dirs.bounds_circle(self)
    }

    pub fn offset(&self, offset: f64) -> Circle {
        let new_radius = self.radius as f64 + offset;
        Circle::new(self.center, math_round_i32(new_radius))
    }

    pub fn shrink(&self, offset: f64) -> Circle {
        let new_radius = self.radius as f64 - offset;
        let r = math_round_i32(new_radius).max(1);
        Circle::new(self.center, r)
    }

    pub fn translate_by(&self, vector: &Vector) -> Circle {
        if *vector == Vector::ZERO {
            return *self;
        }
        if !matches!(vector, Vector::Int(_)) {
            log::warn!("Circle.translate_by only implemented for IntVectors till now");
            return *self;
        }
        let new_center = Point::Int(self.center).translate_by(vector).as_int();
        Circle::new(new_center, self.radius)
    }

    /// Not implemented in Java (returns null).
    pub fn nearest_point_approx(&self, _point: &FloatPoint) -> Option<FloatPoint> {
        log::warn!("Circle.nearest_point_approx not yet implemented");
        None
    }

    pub fn border_distance(&self, point: &FloatPoint) -> f64 {
        let d = point.distance(&self.center.to_float()) - self.radius as f64;
        d.abs()
    }

    pub fn enlarge(&self, offset: f64) -> Circle {
        if offset == 0.0 {
            return *self;
        }
        let new_radius = self.radius + math_round_i32(offset);
        Circle::new(self.center, new_radius)
    }

    /// Checks, if this shape and other have a nonempty intersection.
    pub fn intersects(&self, other: &Shape) -> bool {
        other.intersects_circle(self)
    }

    pub fn intersects_circle(&self, other: &Circle) -> bool {
        let mut radius_sum_square = (self.radius + other.radius) as f64;
        radius_sum_square *= radius_sum_square;
        self.center.distance_square(&other.center) <= radius_sum_square
    }

    pub fn intersects_int_box(&self, b: &IntBox) -> bool {
        b.distance(&self.center.to_float()) <= self.radius as f64
    }

    pub fn intersects_int_octagon(&self, oct: &IntOctagon) -> bool {
        TileShapeImpl::distance(oct, &self.center.to_float()) <= self.radius as f64
    }

    pub fn intersects_simplex(&self, simplex: &Simplex) -> bool {
        TileShapeImpl::distance(simplex, &self.center.to_float()) <= self.radius as f64
    }

    /// Not implemented in Java (returns null).
    pub fn cutout_polyline(&self, _polyline: &Polyline) -> Option<Vec<Polyline>> {
        log::warn!("Circle.cutout not yet implemented");
        None
    }

    pub fn split_to_convex(&self) -> Vec<TileShape> {
        vec![self.bounding_tile()]
    }

    pub fn get_border(&self) -> Circle {
        *self
    }

    pub fn get_holes(&self) -> Vec<Shape> {
        Vec::new()
    }

    pub fn corner_approx_arr(&self) -> Vec<FloatPoint> {
        Vec::new()
    }
}
