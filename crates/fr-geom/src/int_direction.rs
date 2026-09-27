//! Port of `IntDirection.java`: Direction as an equivalence class of IntVectors.

use crate::big_int_direction::BigIntDirection;
use crate::direction::Direction;
use crate::int_vector::IntVector;
use crate::signum::Signum;
use crate::vector::Vector;

/// Implements the abstract class Direction as an equivalence class of IntVector's.
///
/// Equality (`PartialEq`) follows Java `Direction.equals`: two directions are equal if they
/// point into the same direction, even if the coordinates are not normalized.
#[derive(Clone, Copy, Debug)]
pub struct IntDirection {
    pub x: i32,
    pub y: i32,
}

impl IntDirection {
    pub const NULL: IntDirection = IntDirection { x: 0, y: 0 };
    pub const RIGHT: IntDirection = IntDirection { x: 1, y: 0 };
    pub const RIGHT45: IntDirection = IntDirection { x: 1, y: 1 };
    pub const UP: IntDirection = IntDirection { x: 0, y: 1 };
    pub const UP45: IntDirection = IntDirection { x: -1, y: 1 };
    pub const LEFT: IntDirection = IntDirection { x: -1, y: 0 };
    pub const LEFT45: IntDirection = IntDirection { x: -1, y: -1 };
    pub const DOWN: IntDirection = IntDirection { x: 0, y: -1 };
    pub const DOWN45: IntDirection = IntDirection { x: 1, y: -1 };

    #[inline]
    pub const fn new(x: i32, y: i32) -> Self {
        IntDirection { x, y }
    }

    #[inline]
    pub fn from_vector(v: &IntVector) -> Self {
        IntDirection { x: v.x, y: v.y }
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

    #[inline]
    pub fn get_vector(&self) -> Vector {
        Vector::Int(IntVector::new(self.x, self.y))
    }

    #[inline]
    pub fn get_int_vector(&self) -> IntVector {
        IntVector::new(self.x, self.y)
    }

    /// Package private `compareTo(IntDirection)`.
    pub fn compare_to_int(&self, other: &IntDirection) -> i32 {
        let (x, y) = (self.x, self.y);
        if y > 0 {
            if other.y < 0 {
                return -1;
            }
            if other.y == 0 {
                if other.x > 0 {
                    return 1;
                }
                return -1;
            }
        } else if y < 0 {
            if other.y >= 0 {
                return 1;
            }
        } else {
            // y == 0
            if x > 0 {
                if other.y != 0 || other.x < 0 {
                    return -1;
                }
                return 0;
            }
            // x < 0
            if other.y > 0 || other.y == 0 && other.x > 0 {
                return 1;
            }
            if other.y < 0 {
                return -1;
            }
            return 0;
        }
        // now this direction and other are located in the same open horizontal half plane
        let determinant = other.x as f64 * y as f64 - other.y as f64 * x as f64;
        Signum::as_int(determinant)
    }

    /// Package private `compareTo(BigIntDirection)`.
    pub fn compare_to_big(&self, other: &BigIntDirection) -> i32 {
        -other.compare_to_int(self)
    }

    /// Implements Comparable: 1 if this direction has a strictly bigger angle with the positive
    /// x-axis than other, 0 if equal, -1 otherwise.
    pub fn compare_to(&self, other: &Direction) -> i32 {
        match other {
            Direction::Int(o) => -o.compare_to_int(self),
            Direction::BigInt(o) => -o.compare_to_int(self),
        }
    }

    #[inline]
    pub fn opposite(&self) -> IntDirection {
        IntDirection::new(self.x.wrapping_neg(), self.y.wrapping_neg())
    }

    /// Turns the direction by factor times 45 degree. Note: like Java, `factor % 8` keeps the
    /// sign, so negative factors (other than multiples of 8) yield the NULL direction.
    /// (int arithmetic wraps like in Java, which matters for huge BigInt-free directions)
    pub fn turn_45_degree(&self, factor: i32) -> IntDirection {
        let (x, y) = (self.x, self.y);
        match factor % 8 {
            0 => IntDirection::new(x, y),
            1 => IntDirection::new(x.wrapping_sub(y), x.wrapping_add(y)),
            2 => IntDirection::new(y.wrapping_neg(), x),
            3 => IntDirection::new(x.wrapping_neg().wrapping_sub(y), x.wrapping_sub(y)),
            4 => IntDirection::new(x.wrapping_neg(), y.wrapping_neg()),
            5 => IntDirection::new(y.wrapping_sub(x), x.wrapping_neg().wrapping_sub(y)),
            6 => IntDirection::new(y, x.wrapping_neg()),
            7 => IntDirection::new(x.wrapping_add(y), y.wrapping_sub(x)),
            _ => IntDirection::new(0, 0),
        }
    }

    #[inline]
    pub fn determinant(&self, other: &IntDirection) -> f64 {
        self.x as f64 * other.y as f64 - self.y as f64 * other.x as f64
    }

    #[inline]
    pub fn to_direction(self) -> Direction {
        Direction::Int(self)
    }
}

impl PartialEq for IntDirection {
    /// Java `Direction.equals` (see [`Direction`]'s `PartialEq`).
    fn eq(&self, other: &Self) -> bool {
        if self.x == other.x && self.y == other.y {
            // covers the Java identity shortcut, in particular Direction.NULL == Direction.NULL
            return true;
        }
        let v1 = self.get_int_vector();
        let v2 = other.get_int_vector();
        if v1.side_of_int(&v2).negate() != crate::side::Side::Collinear {
            return false;
        }
        v1.projection_int(&v2) == Signum::Positive
    }
}
