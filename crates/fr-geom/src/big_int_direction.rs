//! Port of `BigIntDirection.java`: a direction as a tuple of infinite precision integers.

use num_bigint::BigInt;
use num_traits::{One, Signed};

use crate::direction::Direction;
use crate::int_direction::IntDirection;
use crate::java_compat::bigint_signum;
use crate::rational_vector::RationalVector;
use crate::vector::Vector;

#[derive(Clone, Debug)]
pub struct BigIntDirection {
    pub x: BigInt,
    pub y: BigInt,
}

impl BigIntDirection {
    pub fn new(x: BigInt, y: BigInt) -> Self {
        BigIntDirection { x, y }
    }

    /// Creates a BigIntDirection from an IntDirection.
    pub fn from_int(dir: &IntDirection) -> Self {
        BigIntDirection {
            x: BigInt::from(dir.x),
            y: BigInt::from(dir.y),
        }
    }

    pub fn is_orthogonal(&self) -> bool {
        bigint_signum(&self.x) == 0 || bigint_signum(&self.y) == 0
    }

    pub fn is_diagonal(&self) -> bool {
        self.x.abs() == self.y.abs()
    }

    pub fn get_vector(&self) -> Vector {
        Vector::Rational(Box::new(RationalVector::new(
            self.x.clone(),
            self.y.clone(),
            BigInt::one(),
        )))
    }

    pub fn turn_45_degree(&self, _factor: i32) -> Direction {
        log::warn!("BigIntDirection: turn_45_degree not yet implemented");
        Direction::BigInt(Box::new(self.clone()))
    }

    pub fn opposite(&self) -> BigIntDirection {
        BigIntDirection::new(-&self.x, -&self.y)
    }

    /// Package private `compareTo(IntDirection)`.
    pub fn compare_to_int(&self, other: &IntDirection) -> i32 {
        self.compare_to_big(&BigIntDirection::from_int(other))
    }

    /// Package private `compareTo(BigIntDirection)`.
    pub fn compare_to_big(&self, other: &BigIntDirection) -> i32 {
        let x1 = bigint_signum(&self.x);
        let y1 = bigint_signum(&self.y);
        let x2 = bigint_signum(&other.x);
        let y2 = bigint_signum(&other.y);
        if y1 > 0 {
            if y2 < 0 {
                return -1;
            }
            if y2 == 0 {
                if x2 > 0 {
                    return 1;
                }
                return -1;
            }
        } else if y1 < 0 {
            if y2 >= 0 {
                return 1;
            }
        } else {
            // y1 == 0
            if x1 > 0 {
                if y2 != 0 || x2 < 0 {
                    return -1;
                }
                return 0;
            }
            // x1 < 0
            if y2 > 0 || y2 == 0 && x2 > 0 {
                return 1;
            }
            if y2 < 0 {
                return -1;
            }
            return 0;
        }
        // now this direction and other are located in the same open horizontal half plane
        let determinant = &self.y * &other.x - &self.x * &other.y;
        bigint_signum(&determinant)
    }

    /// Implements Comparable (see IntDirection::compare_to).
    pub fn compare_to(&self, other: &Direction) -> i32 {
        match other {
            // IntDirection.compareTo(BigIntDirection) == -this.compareTo(IntDirection)
            Direction::Int(o) => -o.compare_to_big(self),
            Direction::BigInt(o) => -o.compare_to_big(self),
        }
    }
}
