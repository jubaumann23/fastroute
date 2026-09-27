//! Port of `IntPoint.java`: implementation of the abstract class Point as a tuple of integers.

use num_bigint::BigInt;
use num_traits::One;

use crate::float_point::FloatPoint;
use crate::int_box::IntBox;
use crate::int_octagon::IntOctagon;
use crate::int_vector::IntVector;
use crate::java_compat::{big, bigint_int_value, bigint_signum};
use crate::jmath;
use crate::line::Line;
use crate::point::Point;
use crate::rational_point::RationalPoint;
use crate::rational_vector::RationalVector;
use crate::side::Side;
use crate::vector::Vector;

/// Equality and hashing are by value, like Java `IntPoint.equals/hashCode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct IntPoint {
    pub x: i32,
    pub y: i32,
}

impl IntPoint {
    /// Creates an IntPoint from two integer coordinates. (Java logs a debug message if a
    /// coordinate exceeds CRIT_INT; that message is omitted here.)
    #[inline]
    pub const fn new(x: i32, y: i32) -> Self {
        IntPoint { x, y }
    }

    /// Java `hashCode()`: `31 * x + y` with int overflow.
    #[inline]
    pub fn hash_code(&self) -> i32 {
        self.x.wrapping_mul(31).wrapping_add(self.y)
    }

    /// Returns a unique ID for this point for deterministic tie-breaking.
    #[inline]
    pub fn get_id(&self) -> i32 {
        self.x.wrapping_mul(31).wrapping_add(self.y)
    }

    #[inline]
    pub fn is_infinite(&self) -> bool {
        false
    }

    #[inline]
    pub fn surrounding_box(&self) -> IntBox {
        IntBox::from_points(*self, *self)
    }

    pub fn surrounding_octagon(&self) -> IntOctagon {
        let tmp1 = self.x.wrapping_sub(self.y);
        let tmp2 = self.x.wrapping_add(self.y);
        IntOctagon::new(self.x, self.y, self.x, self.y, tmp1, tmp1, tmp2, tmp2)
    }

    #[inline]
    pub fn is_contained_in(&self, b: &IntBox) -> bool {
        self.x >= b.ll.x && self.y >= b.ll.y && self.x <= b.ur.x && self.y <= b.ur.y
    }

    /// Returns the translation of this point by vector.
    pub fn translate_by(&self, vector: &Vector) -> Point {
        if *vector == Vector::ZERO {
            return Point::Int(*self);
        }
        vector.add_to_int_point(self)
    }

    /// Package private `translateBy(IntVector)`.
    #[inline]
    pub fn translate_by_int_vector(&self, v: &IntVector) -> IntPoint {
        IntPoint::new(self.x.wrapping_add(v.x), self.y.wrapping_add(v.y))
    }

    /// Package private `translateBy(RationalVector)`.
    pub fn translate_by_rational_vector(&self, v: &RationalVector) -> Point {
        v.add_to_int_point(self)
    }

    /// Returns the difference vector of this point and other.
    pub fn difference_by(&self, other: &Point) -> Vector {
        match other {
            // other.differenceBy(this).negate()
            Point::Int(o) => Vector::Int(o.difference_by_int(self).negate()),
            Point::Rational(o) => o.difference_by_int(self).negate(),
        }
    }

    /// `differenceBy(IntPoint)`, which returns an IntVector in Java.
    #[inline]
    pub fn difference_by_int(&self, other: &IntPoint) -> IntVector {
        IntVector::new(self.x.wrapping_sub(other.x), self.y.wrapping_sub(other.y))
    }

    /// Package private `differenceBy(RationalPoint)`.
    pub fn difference_by_rational(&self, other: &RationalPoint) -> Vector {
        other.difference_by_int(self).negate()
    }

    pub fn side_of_line(&self, line: &Line) -> Side {
        let v1 = self.difference_by(&line.a);
        let v2 = line.b.difference_by(&line.a);
        v1.side_of(&v2)
    }

    #[inline]
    pub fn to_float(&self) -> FloatPoint {
        FloatPoint::new(self.x as f64, self.y as f64)
    }

    /// Returns the determinant of the vectors (x, y) and (other.x, other.y).
    #[inline]
    pub fn determinant(&self, other: &IntPoint) -> i64 {
        (self.x as i64 * other.y as i64).wrapping_sub(self.y as i64 * other.x as i64)
    }

    /// Only implemented for lines consisting of IntPoints (Java casts and would throw otherwise).
    pub fn perpendicular_projection(&self, line: &Line) -> Point {
        let v = line.b.difference_by(&line.a).as_int();
        let vxvx = big(v.x as i64 * v.x as i64);
        let vyvy = big(v.y as i64 * v.y as i64);
        let vxvy = big(v.x as i64 * v.y as i64);
        let mut denominator = &vxvx + &vyvy;
        let det = big(line.a.as_int().determinant(&line.b.as_int()));
        let point_x = big(self.x as i64);
        let point_y = big(self.y as i64);

        let mut proj_x = &vxvx * &point_x + &vxvy * &point_y + &det * big(v.y as i64);
        let mut proj_y = &vxvy * &point_x + &vyvy * &point_y - &det * big(v.x as i64);

        let signum = bigint_signum(&denominator);
        if signum != 0 {
            if signum < 0 {
                denominator = -denominator;
                proj_x = -proj_x;
                proj_y = -proj_y;
            }
            if bigint_signum(&(&proj_x % &denominator)) == 0
                && bigint_signum(&(&proj_y % &denominator)) == 0
            {
                proj_x = &proj_x / &denominator;
                proj_y = &proj_y / &denominator;
                return Point::Int(IntPoint::new(
                    bigint_int_value(&proj_x),
                    bigint_int_value(&proj_y),
                ));
            }
        }
        Point::Rational(Box::new(RationalPoint::new(proj_x, proj_y, denominator)))
    }

    /// Returns the signed area of the parallelogramm spanned by the vectors p2 - p1 and this - p1.
    pub fn signed_area(&self, p1: &IntPoint, p2: &IntPoint) -> f64 {
        let d21 = p2.difference_by_int(p1);
        let d01 = self.difference_by_int(p1);
        d21.determinant(&d01) as f64
    }

    /// Calculates the square of the distance between this point and to_point.
    #[inline]
    pub fn distance_square(&self, to_point: &IntPoint) -> f64 {
        // Java: double dx = toPoint.x - this.x; (int subtraction, then widened)
        let dx = to_point.x.wrapping_sub(self.x) as f64;
        let dy = to_point.y.wrapping_sub(self.y) as f64;
        dx * dx + dy * dy
    }

    #[inline]
    pub fn distance(&self, to_point: &IntPoint) -> f64 {
        jmath::sqrt(self.distance_square(to_point))
    }

    /// Snaps this point onto the horizontal or vertical line through other.
    pub fn orthogonal_projection(&self, other: &IntPoint) -> IntPoint {
        let horizontal_distance = self.x.wrapping_sub(other.x).wrapping_abs();
        let vertical_distance = self.y.wrapping_sub(other.y).wrapping_abs();
        if horizontal_distance <= vertical_distance {
            // projection onto the vertical line through other
            IntPoint::new(other.x, self.y)
        } else {
            // projection onto the horizontal line through other
            IntPoint::new(self.x, other.y)
        }
    }

    /// Snaps this point onto an orthogonal or diagonal line through other.
    pub fn fortyfive_degree_projection(&self, other: &IntPoint) -> IntPoint {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        let mut dist_arr = [0.0f64; 4];
        dist_arr[0] = dx.wrapping_abs() as f64;
        dist_arr[1] = dy.wrapping_abs() as f64;
        let diagonal1 = (dy as f64 - dx as f64) / 2.0;
        let diagonal2 = (dy as f64 + dx as f64) / 2.0;
        dist_arr[2] = diagonal1.abs();
        dist_arr[3] = diagonal2.abs();
        let mut min_dist = dist_arr[0];
        for d in dist_arr.iter().skip(1) {
            if *d < min_dist {
                min_dist = *d;
            }
        }
        if min_dist == dist_arr[0] {
            // projection onto the vertical line through other
            IntPoint::new(other.x, self.y)
        } else if min_dist == dist_arr[1] {
            // projection onto the horizontal line through other
            IntPoint::new(self.x, other.y)
        } else if min_dist == dist_arr[2] {
            // projection onto the right diagonal line through other
            let diagonal_value = diagonal2 as i32;
            IntPoint::new(other.x + diagonal_value, other.y + diagonal_value)
        } else {
            // projection onto the left diagonal line through other
            let diagonal_value = diagonal1 as i32;
            IntPoint::new(other.x - diagonal_value, other.y + diagonal_value)
        }
    }

    /// Calculates a corner point p so that the lines this->p and p->to_point are multiples of
    /// 45 degree and the angle at p is 45 degree. Returns None, if the line from this point to
    /// to_point is already a multiple of 45 degree.
    pub fn fortyfive_degree_corner(
        &self,
        to_point: &IntPoint,
        left_turn: bool,
    ) -> Option<IntPoint> {
        let dx = to_point.x - self.x;
        let dy = to_point.y - self.y;
        // handle the 8 sections between the 45 degree lines
        let result = if dy > 0 && dy < dx {
            if left_turn {
                IntPoint::new(to_point.x - dy, self.y)
            } else {
                IntPoint::new(self.x + dy, to_point.y)
            }
        } else if dx > 0 && dy > dx {
            if left_turn {
                IntPoint::new(to_point.x, self.y + dx)
            } else {
                IntPoint::new(self.x, to_point.y - dx)
            }
        } else if dx < 0 && dy > -dx {
            if left_turn {
                IntPoint::new(self.x, to_point.y + dx)
            } else {
                IntPoint::new(to_point.x, self.y - dx)
            }
        } else if dy > 0 && dy < -dx {
            if left_turn {
                IntPoint::new(self.x - dy, to_point.y)
            } else {
                IntPoint::new(to_point.x + dy, self.y)
            }
        } else if dy < 0 && dy > dx {
            if left_turn {
                IntPoint::new(to_point.x - dy, self.y)
            } else {
                IntPoint::new(self.x + dy, to_point.y)
            }
        } else if dx < 0 && dy < dx {
            if left_turn {
                IntPoint::new(to_point.x, self.y + dx)
            } else {
                IntPoint::new(self.x, to_point.y - dx)
            }
        } else if dx > 0 && dy < -dx {
            if left_turn {
                IntPoint::new(self.x, to_point.y + dx)
            } else {
                IntPoint::new(to_point.x, self.y - dx)
            }
        } else if dy < 0 && dy > -dx {
            if left_turn {
                IntPoint::new(self.x - dy, to_point.y)
            } else {
                IntPoint::new(to_point.x + dy, self.y)
            }
        } else {
            // the line from this point to to_point is already a multiple of 45 degree
            return None;
        };
        Some(result)
    }

    /// Calculates a corner point p so that the lines this->p and p->to_point are orthogonal.
    /// Returns None, if the line from this point to to_point is already orthogonal.
    pub fn ninety_degree_corner(&self, to_point: &IntPoint, left_turn: bool) -> Option<IntPoint> {
        let dx = to_point.x - self.x;
        let dy = to_point.y - self.y;
        // handle the 4 quadrants
        if dx > 0 && dy > 0 || dx < 0 && dy < 0 {
            if left_turn {
                Some(IntPoint::new(to_point.x, self.y))
            } else {
                Some(IntPoint::new(self.x, to_point.y))
            }
        } else if dx < 0 && dy > 0 || dx > 0 && dy < 0 {
            if left_turn {
                Some(IntPoint::new(self.x, to_point.y))
            } else {
                Some(IntPoint::new(to_point.x, self.y))
            }
        } else {
            None
        }
    }

    /// Package private `compareX(IntPoint)`.
    #[inline]
    pub fn compare_x_int(&self, other: &IntPoint) -> i32 {
        if self.x > other.x {
            1
        } else if self.x == other.x {
            0
        } else {
            -1
        }
    }

    /// Package private `compareY(IntPoint)`.
    #[inline]
    pub fn compare_y_int(&self, other: &IntPoint) -> i32 {
        if self.y > other.y {
            1
        } else if self.y == other.y {
            0
        } else {
            -1
        }
    }

    pub fn compare_x(&self, other: &Point) -> i32 {
        match other {
            Point::Int(o) => -o.compare_x_int(self),
            Point::Rational(o) => -o.compare_x_int(self),
        }
    }

    pub fn compare_y(&self, other: &Point) -> i32 {
        match other {
            Point::Int(o) => -o.compare_y_int(self),
            Point::Rational(o) => -o.compare_y_int(self),
        }
    }

    #[inline]
    pub fn to_point(self) -> Point {
        Point::Int(self)
    }

    /// RationalPoint with the same coordinates.
    pub fn to_rational(&self) -> RationalPoint {
        RationalPoint::new(BigInt::from(self.x), BigInt::from(self.y), BigInt::one())
    }
}

impl std::fmt::Display for IntPoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "({},{})", self.x, self.y)
    }
}
