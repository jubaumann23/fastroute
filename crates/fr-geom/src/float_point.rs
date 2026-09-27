//! Port of `FloatPoint.java`: a point in the plane as a tuple of doubles.
//!
//! Because arithmetic with doubles is in general not exact, FloatPoint is not a `Point`.

use crate::direction::Direction;
use crate::float_line::FloatLine;
use crate::int_box::IntBox;
use crate::int_octagon::IntOctagon;
use crate::int_point::IntPoint;
use crate::java_compat::{math_rint, math_round_i32, INT_MAX_F64, INT_MIN_F64};
use crate::jmath;
use crate::line::Line;
use crate::side::Side;

/// `PartialEq` compares coordinates (Java FloatPoint has identity equality; all Java uses
/// in the geometry package are coordinate based).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct FloatPoint {
    pub x: f64,
    pub y: f64,
}

impl FloatPoint {
    pub const ZERO: FloatPoint = FloatPoint { x: 0.0, y: 0.0 };

    #[inline]
    pub const fn new(x: f64, y: f64) -> Self {
        FloatPoint { x, y }
    }

    /// Creates a FloatPoint from an IntPoint.
    #[inline]
    pub fn from_int_point(p: &IntPoint) -> Self {
        FloatPoint {
            x: p.x as f64,
            y: p.y as f64,
        }
    }

    /// Calculates the smallest IntOctagon containing all the input points.
    pub fn bounding_octagon(points: &[FloatPoint]) -> IntOctagon {
        let mut min_x = INT_MAX_F64;
        let mut min_y = INT_MAX_F64;
        let mut max_x = INT_MIN_F64;
        let mut max_y = INT_MIN_F64;
        let mut min_ulx = INT_MAX_F64;
        let mut max_lrx = INT_MIN_F64;
        let mut min_llx = INT_MAX_F64;
        let mut max_urx = INT_MIN_F64;
        for current in points {
            min_x = java_min(min_x, current.x);
            min_y = java_min(min_y, current.y);
            max_x = java_max(max_x, current.x);
            max_y = java_max(max_y, current.y);
            let tmp = current.x - current.y;
            min_ulx = java_min(min_ulx, tmp);
            max_lrx = java_max(max_lrx, tmp);
            let tmp = current.x + current.y;
            min_llx = java_min(min_llx, tmp);
            max_urx = java_max(max_urx, tmp);
        }
        IntOctagon::new(
            min_x.floor() as i32,
            min_y.floor() as i32,
            max_x.ceil() as i32,
            max_y.ceil() as i32,
            min_ulx.floor() as i32,
            max_lrx.ceil() as i32,
            min_llx.floor() as i32,
            max_urx.ceil() as i32,
        )
    }

    /// Returns the square of the distance from this point to the zero point.
    #[inline]
    pub fn size_square(&self) -> f64 {
        self.x * self.x + self.y * self.y
    }

    /// Returns the distance from this point to the zero point.
    #[inline]
    pub fn size(&self) -> f64 {
        jmath::sqrt(self.size_square())
    }

    /// Returns the square of the distance from this Point to the Point other.
    #[inline]
    pub fn distance_square(&self, other: &FloatPoint) -> f64 {
        let dx = other.x - self.x;
        let dy = other.y - self.y;
        dx * dx + dy * dy
    }

    /// Returns the distance from this point to the point other.
    #[inline]
    pub fn distance(&self, other: &FloatPoint) -> f64 {
        jmath::sqrt(self.distance_square(other))
    }

    /// Computes the weighted distance to other.
    pub fn weighted_distance(
        &self,
        other: &FloatPoint,
        horizontal_weight: f64,
        vertical_weight: f64,
    ) -> f64 {
        let mut delta_x = self.x - other.x;
        let mut delta_y = self.y - other.y;
        delta_x *= horizontal_weight;
        delta_y *= vertical_weight;
        jmath::sqrt(delta_x * delta_x + delta_y * delta_y)
    }

    /// Rounds the coordinates to an IntPoint (Java `(int) Math.round`).
    #[inline]
    pub fn round(&self) -> IntPoint {
        IntPoint::new(math_round_i32(self.x), math_round_i32(self.y))
    }

    /// Rounds this point, so that if this point is on the right side of any directed line with
    /// direction dir, the result point will also be on the right side.
    pub fn round_to_the_right(&self, dir: &Direction) -> IntPoint {
        let dv = dir.get_vector().to_float();
        let rounded_x = if dv.y > 0.0 {
            self.x.ceil() as i32
        } else if dv.y < 0.0 {
            self.x.floor() as i32
        } else {
            math_round_i32(self.x)
        };
        let rounded_y = if dv.x > 0.0 {
            self.y.floor() as i32
        } else if dv.x < 0.0 {
            self.y.ceil() as i32
        } else {
            math_round_i32(self.y)
        };
        IntPoint::new(rounded_x, rounded_y)
    }

    /// Round this Point so that x is a multiple of horizontal_grid and y of vertical_grid.
    pub fn round_to_grid(&self, horizontal_grid: i32, vertical_grid: i32) -> IntPoint {
        let rounded_x = if horizontal_grid > 0 {
            math_rint(self.x / horizontal_grid as f64) * horizontal_grid as f64
        } else {
            self.x
        };
        let rounded_y = if vertical_grid > 0 {
            math_rint(self.y / vertical_grid as f64) * vertical_grid as f64
        } else {
            self.y
        };
        IntPoint::new(rounded_x as i32, rounded_y as i32)
    }

    /// Rounds this point, so that if this point is on the left side of any directed line with
    /// direction dir, the result point will also be on the left side.
    pub fn round_to_the_left(&self, dir: &Direction) -> IntPoint {
        let dv = dir.get_vector().to_float();
        let rounded_x = if dv.y > 0.0 {
            self.x.floor() as i32
        } else if dv.y < 0.0 {
            self.x.ceil() as i32
        } else {
            math_round_i32(self.x)
        };
        let rounded_y = if dv.x > 0.0 {
            self.y.ceil() as i32
        } else if dv.x < 0.0 {
            self.y.floor() as i32
        } else {
            math_round_i32(self.y)
        };
        IntPoint::new(rounded_x, rounded_y)
    }

    /// Adds the coordinates of this FloatPoint and other.
    #[inline]
    pub fn add(&self, other: &FloatPoint) -> FloatPoint {
        FloatPoint::new(self.x + other.x, self.y + other.y)
    }

    /// Subtracts the coordinates of other from this FloatPoint.
    #[inline]
    pub fn subtract(&self, other: &FloatPoint) -> FloatPoint {
        FloatPoint::new(self.x - other.x, self.y - other.y)
    }

    /// Returns an approximation of the perpendicular projection of this point onto line.
    pub fn projection_approx(&self, line: &Line) -> FloatPoint {
        let float_line = FloatLine::new(line.a.to_float(), line.b.to_float());
        float_line.perpendicular_projection(self)
    }

    /// Calculates the scalar product of (p1 - this) with (p2 - this).
    #[inline]
    pub fn scalar_product(&self, p1: &FloatPoint, p2: &FloatPoint) -> f64 {
        let dx1 = p1.x - self.x;
        let dx2 = p2.x - self.x;
        let dy1 = p1.y - self.y;
        let dy2 = p2.y - self.y;
        dx1 * dx2 + dy1 * dy2
    }

    /// Approximates a FloatPoint on the line from zero to this point with distance new_size
    /// from zero.
    pub fn change_size(&self, new_size: f64) -> FloatPoint {
        if self.x == 0.0 && self.y == 0.0 {
            // the size of the zero point cannot be changed
            return *self;
        }
        let length = jmath::sqrt(self.x * self.x + self.y * self.y);
        let new_x = (self.x * new_size) / length;
        let new_y = (self.y * new_size) / length;
        FloatPoint::new(new_x, new_y)
    }

    /// Approximates a FloatPoint on the line from this point to to_point with distance
    /// new_length from this point.
    pub fn change_length(&self, to_point: &FloatPoint, new_length: f64) -> FloatPoint {
        let dx = to_point.x - self.x;
        let dy = to_point.y - self.y;
        if dx == 0.0 && dy == 0.0 {
            log::warn!("IntPoint.change_length: Points are equal");
            return *to_point;
        }
        let length = jmath::sqrt(dx * dx + dy * dy);
        let new_x = self.x + (dx * new_length) / length;
        let new_y = self.y + (dy * new_length) / length;
        FloatPoint::new(new_x, new_y)
    }

    /// Returns the middle point between this point and to_point.
    pub fn middle_point(&self, to_point: &FloatPoint) -> FloatPoint {
        let middle_x = 0.5 * (self.x + to_point.x);
        let middle_y = 0.5 * (self.y + to_point.y);
        FloatPoint::new(middle_x, middle_y)
    }

    /// Returns ON_THE_LEFT, if this Point is on the left of the line from p1 to p2, and
    /// ON_THE_RIGHT, if on the right. (Collinearity is not reliable with doubles.)
    #[inline]
    pub fn side_of(&self, p1: &FloatPoint, p2: &FloatPoint) -> Side {
        let d21x = p2.x - p1.x;
        let d21y = p2.y - p1.y;
        let d01x = self.x - p1.x;
        let d01y = self.y - p1.y;
        let determinant = d21x * d01y - d21y * d01x;
        Side::of(determinant)
    }

    /// Rotates this FloatPoint by angle (in radians) around the pole.
    pub fn rotate(&self, angle: f64, pole: &FloatPoint) -> FloatPoint {
        if angle == 0.0 {
            return *self;
        }
        let dx = self.x - pole.x;
        let dy = self.y - pole.y;
        let sin_angle = jmath::sin(angle);
        let cos_angle = jmath::cos(angle);
        let new_dx = dx * cos_angle - dy * sin_angle;
        let new_dy = dx * sin_angle + dy * cos_angle;
        FloatPoint::new(pole.x + new_dx, pole.y + new_dy)
    }

    /// Turns this FloatPoint by factor times 90 degrees around ZERO.
    pub fn turn_90_degree(&self, factor: i32) -> FloatPoint {
        let mut n = factor;
        while n < 0 {
            n += 4;
        }
        while n >= 4 {
            n -= 4;
        }
        match n {
            0 => FloatPoint::new(self.x, self.y),
            1 => FloatPoint::new(-self.y, self.x),
            2 => FloatPoint::new(-self.x, -self.y),
            3 => FloatPoint::new(self.y, -self.x),
            _ => FloatPoint::ZERO,
        }
    }

    /// Turns this FloatPoint by factor times 90 degrees around pole.
    pub fn turn_90_degree_around(&self, factor: i32, pole: &FloatPoint) -> FloatPoint {
        let v = self.subtract(pole).turn_90_degree(factor);
        pole.add(&v)
    }

    /// Checks, if this point is contained in the box spanned by p1 and p2 with the input
    /// tolerance.
    pub fn is_contained_in_box(&self, p1: &FloatPoint, p2: &FloatPoint, tolerance: f64) -> bool {
        let (min_x, max_x) = if p1.x < p2.x {
            (p1.x, p2.x)
        } else {
            (p2.x, p1.x)
        };
        if self.x < min_x - tolerance || self.x > max_x + tolerance {
            return false;
        }
        let (min_y, max_y) = if p1.y < p2.y {
            (p1.y, p2.y)
        } else {
            (p2.y, p1.y)
        };
        self.y >= min_y - tolerance && self.y <= max_y + tolerance
    }

    /// Creates the smallest IntBox containing this point.
    pub fn bounding_box(&self) -> IntBox {
        let lower_left = IntPoint::new(self.x.floor() as i32, self.y.floor() as i32);
        let upper_right = IntPoint::new(self.x.ceil() as i32, self.y.ceil() as i32);
        IntBox::from_points(lower_left, upper_right)
    }

    /// Calculates the touching points of the tangents from this point to a circle around
    /// to_point with radius distance. Returns an empty vector if this point is inside the circle.
    pub fn tangential_points(&self, to_point: &FloatPoint, distance: f64) -> Vec<FloatPoint> {
        // turn the situation 90 degree if the x difference is smaller than the y difference
        // for better numerical stability
        let dx = (self.x - to_point.x).abs();
        let dy = (self.y - to_point.y).abs();
        let situation_turned = dy > dx;
        let (pole, circle_center) = if situation_turned {
            (
                FloatPoint::new(-self.y, self.x),
                FloatPoint::new(-to_point.y, to_point.x),
            )
        } else {
            (*self, *to_point)
        };

        let dx = pole.x - circle_center.x;
        let dy = pole.y - circle_center.y;
        let dx_square = dx * dx;
        let dy_square = dy * dy;
        let dist_square = dx_square + dy_square;
        let radius_square = distance * distance;
        let discriminant = radius_square * dy_square - (radius_square - dx_square) * dist_square;

        if discriminant <= 0.0 {
            // pole is inside the circle.
            return Vec::new();
        }
        let square_root = jmath::sqrt(discriminant);

        let a1 = radius_square * dy;
        let dy1 = (a1 + distance * square_root) / dist_square;
        let dy2 = (a1 - distance * square_root) / dist_square;

        let first_point_y = dy1 + circle_center.y;
        let first_point_x = (radius_square - dy * dy1) / dx + circle_center.x;
        let second_point_y = dy2 + circle_center.y;
        let second_point_x = (radius_square - dy * dy2) / dx + circle_center.x;

        if situation_turned {
            // turn the result by 270 degree
            vec![
                FloatPoint::new(first_point_y, -first_point_x),
                FloatPoint::new(second_point_y, -second_point_x),
            ]
        } else {
            vec![
                FloatPoint::new(first_point_x, first_point_y),
                FloatPoint::new(second_point_x, second_point_y),
            ]
        }
    }

    /// Left tangential point of the line from this point to a circle around to_point with
    /// radius distance. Returns None, if this point is inside this circle.
    pub fn left_tangential_point(
        &self,
        to_point: &FloatPoint,
        distance: f64,
    ) -> Option<FloatPoint> {
        let tangent_points = self.tangential_points(to_point, distance);
        if tangent_points.len() < 2 {
            return None;
        }
        if to_point.side_of(self, &tangent_points[0]) == Side::OnTheRight {
            Some(tangent_points[0])
        } else {
            Some(tangent_points[1])
        }
    }

    /// Right tangential point of the line from this point to a circle around to_point with
    /// radius distance. Returns None, if this point is inside this circle.
    pub fn right_tangential_point(
        &self,
        to_point: &FloatPoint,
        distance: f64,
    ) -> Option<FloatPoint> {
        let tangent_points = self.tangential_points(to_point, distance);
        if tangent_points.len() < 2 {
            return None;
        }
        if to_point.side_of(self, &tangent_points[0]) == Side::OnTheLeft {
            Some(tangent_points[0])
        } else {
            Some(tangent_points[1])
        }
    }

    /// Calculates the center of the circle through this point, p1 and p2.
    pub fn circle_center(&self, p1: &FloatPoint, p2: &FloatPoint) -> FloatPoint {
        let slope1 = (p1.y - self.y) / (p1.x - self.x);
        let slope2 = (p2.y - p1.y) / (p2.x - p1.x);
        let center_x = (slope1 * slope2 * (self.y - p2.y) + slope2 * (self.x + p1.x)
            - slope1 * (p1.x + p2.x))
            / (2.0 * (slope2 - slope1));
        let center_y = (0.5 * (self.x + p1.x) - center_x) / slope1 + 0.5 * (self.y + p1.y);
        FloatPoint::new(center_x, center_y)
    }

    /// Returns true, if this point is contained in the circle through p1, p2 and p3.
    pub fn inside_circle(&self, p1: &FloatPoint, p2: &FloatPoint, p3: &FloatPoint) -> bool {
        let center = p1.circle_center(p2, p3);
        let radius_square = center.distance_square(p1);
        self.distance_square(&center) < radius_square - 1.0 // - 1 is a tolerance
    }
}

/// Java `Math.min(double, double)` (NaN propagating, -0.0 < 0.0).
#[inline]
pub(crate) fn java_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        return f64::NAN;
    }
    if a == 0.0 && b == 0.0 {
        return if a.is_sign_negative() { a } else { b };
    }
    if a <= b {
        a
    } else {
        b
    }
}

/// Java `Math.max(double, double)` (NaN propagating, 0.0 > -0.0).
#[inline]
pub(crate) fn java_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        return f64::NAN;
    }
    if a == 0.0 && b == 0.0 {
        return if a.is_sign_positive() { a } else { b };
    }
    if a >= b {
        a
    } else {
        b
    }
}

impl std::fmt::Display for FloatPoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "({} , {})", self.x, self.y)
    }
}
