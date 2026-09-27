//! Port of `RationalVector.java`: analog RationalPoint, but implementing Vector functionality.

use num_bigint::BigInt;
use num_integer::Integer;
use num_traits::{One, Signed};

use crate::big_int_aux::{add_rational_coordinates, determinant};
use crate::big_int_direction::BigIntDirection;
use crate::direction::Direction;
use crate::float_point::FloatPoint;
use crate::int_direction::IntDirection;
use crate::int_point::IntPoint;
use crate::int_vector::IntVector;
use crate::java_compat::{bigint_double_value, bigint_int_value, bigint_signum};
use crate::limits::crit_int_big;
use crate::point::Point;
use crate::rational_point::RationalPoint;
use crate::side::Side;
use crate::signum::Signum;
use crate::vector::Vector;

/// The 2-dimensional vector with rational coordinates `(x / z, y / z)`, `z >= 0`.
#[derive(Clone, Debug)]
pub struct RationalVector {
    pub x: BigInt,
    pub y: BigInt,
    pub z: BigInt,
}

impl RationalVector {
    /// Creates a RationalVector from 3 BigIntegers; the denominator is made non-negative.
    pub fn new(x: BigInt, y: BigInt, z: BigInt) -> Self {
        if bigint_signum(&z) >= 0 {
            RationalVector { x, y, z }
        } else {
            RationalVector {
                x: -x,
                y: -y,
                z: -z,
            }
        }
    }

    /// Creates a RationalVector from an IntVector.
    pub fn from_int(v: &IntVector) -> Self {
        RationalVector {
            x: BigInt::from(v.x),
            y: BigInt::from(v.y),
            z: BigInt::one(),
        }
    }

    pub fn is_zero(&self) -> bool {
        bigint_signum(&self.x) == 0 && bigint_signum(&self.y) == 0
    }

    pub fn negate(&self) -> RationalVector {
        RationalVector::new(-&self.x, -&self.y, self.z.clone())
    }

    /// Package private `add(RationalVector)`.
    pub fn add_rational(&self, other: &RationalVector) -> RationalVector {
        let [x, y, z] =
            add_rational_coordinates([&self.x, &self.y, &self.z], [&other.x, &other.y, &other.z]);
        RationalVector::new(x, y, z)
    }

    /// Package private `add(IntVector)`.
    pub fn add_int(&self, other: &IntVector) -> RationalVector {
        self.add_rational(&RationalVector::from_int(other))
    }

    /// Package private `sideOf(IntVector)`.
    pub fn side_of_int(&self, other: &IntVector) -> Side {
        self.side_of_rational(&RationalVector::from_int(other))
    }

    /// Package private `sideOf(RationalVector)`.
    pub fn side_of_rational(&self, other: &RationalVector) -> Side {
        let det = &self.y * &other.x - &self.x * &other.y;
        Side::of(bigint_signum(&det) as f64)
    }

    pub fn is_orthogonal(&self) -> bool {
        bigint_signum(&self.x) == 0 || bigint_signum(&self.y) == 0
    }

    pub fn is_diagonal(&self) -> bool {
        self.x.abs() == self.y.abs()
    }

    /// Package private `projection(IntVector)`.
    pub fn projection_int(&self, other: &IntVector) -> Signum {
        // Vector vector = new RationalVector(other); return vector.projection(this);
        RationalVector::from_int(other).projection_rational(self)
    }

    /// Package private `projection(RationalVector)`.
    pub fn projection_rational(&self, other: &RationalVector) -> Signum {
        let tmp = &self.x * &other.x + &self.y * &other.y;
        Signum::of(bigint_signum(&tmp) as f64)
    }

    /// Package private `scalarProduct(IntVector)`.
    pub fn scalar_product_int(&self, other: &IntVector) -> f64 {
        RationalVector::from_int(other).scalar_product_rational(self)
    }

    /// Package private `scalarProduct(RationalVector)`.
    pub fn scalar_product_rational(&self, other: &RationalVector) -> f64 {
        let v1 = self.to_float();
        let v2 = other.to_float();
        v1.x * v2.x + v1.y * v2.y
    }

    pub fn to_float(&self) -> FloatPoint {
        let xd = bigint_double_value(&self.x);
        let yd = bigint_double_value(&self.y);
        let zd = bigint_double_value(&self.z);
        FloatPoint::new(xd / zd, yd / zd)
    }

    pub fn change_length_approx(&self, _length: f64) -> Vector {
        log::warn!("RationalVector: change_length_approx not yet implemented");
        Vector::Rational(Box::new(self.clone()))
    }

    pub fn turn_90_degree(&self, factor: i32) -> RationalVector {
        let mut n = factor;
        while n < 0 {
            n += 4;
        }
        while n >= 4 {
            n -= 4;
        }
        match n {
            1 => RationalVector::new(-&self.y, self.x.clone(), self.z.clone()),
            2 => RationalVector::new(-&self.x, -&self.y, self.z.clone()),
            3 => RationalVector::new(self.y.clone(), -&self.x, self.z.clone()),
            _ => self.clone(),
        }
    }

    pub fn mirror_at_y_axis(&self) -> RationalVector {
        RationalVector::new(-&self.x, self.y.clone(), self.z.clone())
    }

    pub fn mirror_at_x_axis(&self) -> RationalVector {
        RationalVector::new(self.x.clone(), -&self.y, self.z.clone())
    }

    /// Panics with division by zero if x and y are both 0 (Java: ArithmeticException).
    pub fn to_normalized_direction(&self) -> Direction {
        let gcd = self.x.gcd(&self.y);
        let dx = &self.x / &gcd;
        let dy = &self.y / &gcd;
        let crit = crit_int_big();
        if dx.abs() <= crit && dy.abs() <= crit {
            return Direction::Int(IntDirection::new(
                bigint_int_value(&dx),
                bigint_int_value(&dy),
            ));
        }
        Direction::BigInt(Box::new(BigIntDirection::new(dx, dy)))
    }

    pub fn add_to_int_point(&self, point: &IntPoint) -> Point {
        let new_x = &self.z * BigInt::from(point.x) + &self.x;
        let new_y = &self.z * BigInt::from(point.y) + &self.y;
        Point::Rational(Box::new(RationalPoint::new(new_x, new_y, self.z.clone())))
    }

    pub fn add_to_rational_point(&self, point: &RationalPoint) -> Point {
        let [x, y, z] =
            add_rational_coordinates([&self.x, &self.y, &self.z], [&point.x, &point.y, &point.z]);
        Point::Rational(Box::new(RationalPoint::new(x, y, z)))
    }
}

impl PartialEq for RationalVector {
    fn eq(&self, other: &Self) -> bool {
        let det = determinant(&self.x, &other.x, &self.z, &other.z);
        if bigint_signum(&det) != 0 {
            return false;
        }
        let det = determinant(&self.y, &other.y, &self.z, &other.z);
        bigint_signum(&det) == 0
    }
}
