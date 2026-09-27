//! Port of `IntVector.java`: implementation of Vector via a tuple of integers.

use crate::big_int_aux::binary_gcd;
use crate::direction::Direction;
use crate::float_point::FloatPoint;
use crate::int_direction::IntDirection;
use crate::int_point::IntPoint;
use crate::point::Point;
use crate::rational_point::RationalPoint;
use crate::rational_vector::RationalVector;
use crate::side::Side;
use crate::signum::Signum;
use crate::vector::Vector;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct IntVector {
    pub x: i32,
    pub y: i32,
}

impl IntVector {
    pub const ZERO: IntVector = IntVector { x: 0, y: 0 };

    /// Creates an IntVector from two integer coordinates (range check omitted like in Java).
    #[inline]
    pub const fn new(x: i32, y: i32) -> Self {
        IntVector { x, y }
    }

    #[inline]
    pub fn is_zero(&self) -> bool {
        self.x == 0 && self.y == 0
    }

    #[inline]
    pub fn negate(&self) -> IntVector {
        IntVector::new(self.x.wrapping_neg(), self.y.wrapping_neg())
    }

    #[inline]
    pub fn is_orthogonal(&self) -> bool {
        self.x == 0 || self.y == 0
    }

    #[inline]
    pub fn is_diagonal(&self) -> bool {
        self.x.wrapping_abs() == self.y.wrapping_abs()
    }

    #[inline]
    pub fn is_multiple_of_45_degree(&self) -> bool {
        self.is_orthogonal() || self.is_diagonal()
    }

    /// Calculates the determinant of the matrix consisting of this Vector and other.
    #[inline]
    pub fn determinant(&self, other: &IntVector) -> i64 {
        (self.x as i64 * other.y as i64).wrapping_sub(self.y as i64 * other.x as i64)
    }

    pub fn turn_90_degree(&self, factor: i32) -> IntVector {
        let mut n = factor;
        while n < 0 {
            n += 4;
        }
        while n >= 4 {
            n -= 4;
        }
        match n {
            0 => IntVector::new(self.x, self.y),
            1 => IntVector::new(self.y.wrapping_neg(), self.x),
            2 => IntVector::new(self.x.wrapping_neg(), self.y.wrapping_neg()),
            3 => IntVector::new(self.y, self.x.wrapping_neg()),
            _ => IntVector::ZERO,
        }
    }

    pub fn mirror_at_y_axis(&self) -> IntVector {
        IntVector::new(self.x.wrapping_neg(), self.y)
    }

    pub fn mirror_at_x_axis(&self) -> IntVector {
        IntVector::new(self.x, self.y.wrapping_neg())
    }

    /// Package private `add(IntVector)`.
    #[inline]
    pub fn add_int(&self, other: &IntVector) -> IntVector {
        IntVector::new(self.x.wrapping_add(other.x), self.y.wrapping_add(other.y))
    }

    /// Returns the Point, which results from adding this vector to point.
    #[inline]
    pub fn add_to_int_point(&self, point: &IntPoint) -> Point {
        Point::Int(IntPoint::new(
            point.x.wrapping_add(self.x),
            point.y.wrapping_add(self.y),
        ))
    }

    pub fn add_to_rational_point(&self, point: &RationalPoint) -> Point {
        point.translate_by_int_vector(self)
    }

    /// Package private `sideOf(IntVector other)` of this vector.
    #[inline]
    pub fn side_of_int(&self, other: &IntVector) -> Side {
        let determinant = other.x as f64 * self.y as f64 - other.y as f64 * self.x as f64;
        Side::of(determinant)
    }

    /// Package private `sideOf(RationalVector other)`.
    pub fn side_of_rational(&self, other: &RationalVector) -> Side {
        other.side_of_int(self).negate()
    }

    /// Package private `projection(IntVector other)`.
    #[inline]
    pub fn projection_int(&self, other: &IntVector) -> Signum {
        let tmp = self.x as f64 * other.x as f64 + self.y as f64 * other.y as f64;
        Signum::of(tmp)
    }

    /// Package private `scalarProduct(IntVector other)`.
    #[inline]
    pub fn scalar_product_int(&self, other: &IntVector) -> f64 {
        self.x as f64 * other.x as f64 + self.y as f64 * other.y as f64
    }

    #[inline]
    pub fn to_float(&self) -> FloatPoint {
        FloatPoint::new(self.x as f64, self.y as f64)
    }

    pub fn change_length_approx(&self, length: f64) -> Vector {
        let new_point = self.to_float().change_size(length);
        Point::Int(new_point.round()).difference_by(&Point::ZERO)
    }

    pub fn to_normalized_direction(&self) -> IntDirection {
        let mut dx = self.x;
        let mut dy = self.y;
        let gcd = binary_gcd(dx.wrapping_abs(), dy.wrapping_abs());
        if gcd > 1 {
            dx /= gcd;
            dy /= gcd;
        }
        IntDirection::new(dx, dy)
    }

    #[inline]
    pub fn to_vector(self) -> Vector {
        Vector::Int(self)
    }

    /// The direction of this vector (normalized).
    pub fn to_direction(&self) -> Direction {
        Direction::Int(self.to_normalized_direction())
    }
}
