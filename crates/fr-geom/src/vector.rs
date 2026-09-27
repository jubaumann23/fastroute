//! Port of `Vector.java`: vectors are used for translating Points in the plane.

use num_bigint::BigInt;
use num_traits::{One, Signed};

use crate::direction::Direction;
use crate::float_point::FloatPoint;
use crate::int_point::IntPoint;
use crate::int_vector::IntVector;
use crate::java_compat::{bigint_int_value, bigint_signum};
use crate::jmath;
use crate::limits::{crit_int_big, CRIT_INT};
use crate::point::Point;
use crate::rational_point::RationalPoint;
use crate::rational_vector::RationalVector;
use crate::side::Side;
use crate::signum::Signum;

/// Closed hierarchy `Vector = IntVector | RationalVector`.
///
/// `PartialEq` follows Java `equals`: an IntVector never equals a RationalVector.
#[derive(Clone, Debug, PartialEq)]
pub enum Vector {
    Int(IntVector),
    Rational(Box<RationalVector>),
}

impl Vector {
    /// Standard implementation of the zero vector.
    pub const ZERO: Vector = Vector::Int(IntVector::ZERO);

    /// Creates a Vector (x, y) in the plane (a RationalVector if a coordinate exceeds CRIT_INT).
    pub fn get_instance(x: i32, y: i32) -> Vector {
        let result = IntVector::new(x, y);
        if x.wrapping_abs() > CRIT_INT || y.wrapping_abs() > CRIT_INT {
            return Vector::Rational(Box::new(RationalVector::from_int(&result)));
        }
        Vector::Int(result)
    }

    /// Creates a 2-dimensional Vector from the 3 input values `(x / z, y / z)`.
    pub fn get_instance_big(x: BigInt, y: BigInt, z: BigInt) -> Vector {
        let (mut x, mut y, mut z) = (x, y, z);
        if bigint_signum(&z) < 0 {
            x = -x;
            y = -y;
            z = -z;
        }
        if bigint_signum(&(&x % &z)) == 0 {
            // x and y can be divided by z (sic: only x is checked, like in Java)
            x = &x / &z;
            y = &y / &z;
            z = BigInt::one();
        }
        if z.is_one() {
            let crit = crit_int_big();
            if x.abs() <= crit && y.abs() <= crit {
                return Vector::Int(IntVector::new(bigint_int_value(&x), bigint_int_value(&y)));
            }
        }
        Vector::Rational(Box::new(RationalVector::new(x, y, z)))
    }

    pub fn is_zero(&self) -> bool {
        match self {
            Vector::Int(v) => v.is_zero(),
            Vector::Rational(v) => v.is_zero(),
        }
    }

    pub fn negate(&self) -> Vector {
        match self {
            Vector::Int(v) => Vector::Int(v.negate()),
            Vector::Rational(v) => Vector::Rational(Box::new(v.negate())),
        }
    }

    /// Adds other to this vector.
    pub fn add(&self, other: &Vector) -> Vector {
        // this.add(other) -> other.add(<this type>)
        match (self, other) {
            (Vector::Int(a), Vector::Int(b)) => Vector::Int(b.add_int(a)),
            (Vector::Int(a), Vector::Rational(b)) => Vector::Rational(Box::new(b.add_int(a))),
            (Vector::Rational(a), Vector::Int(b)) => {
                // IntVector.add(RationalVector other) -> other.add(this)
                Vector::Rational(Box::new(a.add_int(b)))
            }
            (Vector::Rational(a), Vector::Rational(b)) => {
                Vector::Rational(Box::new(b.add_rational(a)))
            }
        }
    }

    /// Let L be the line from the Zero Vector to other. Returns ON_THE_LEFT, if this Vector is on
    /// the left of L, ON_THE_RIGHT if on the right, and COLLINEAR otherwise.
    pub fn side_of(&self, other: &Vector) -> Side {
        // this.sideOf(other) == other.sideOf(<this type>).negate()
        match (self, other) {
            (Vector::Int(a), Vector::Int(b)) => b.side_of_int(a).negate(),
            (Vector::Int(a), Vector::Rational(b)) => b.side_of_int(a).negate(),
            (Vector::Rational(a), Vector::Int(b)) => b.side_of_rational(a).negate(),
            (Vector::Rational(a), Vector::Rational(b)) => b.side_of_rational(a).negate(),
        }
    }

    pub fn is_orthogonal(&self) -> bool {
        match self {
            Vector::Int(v) => v.is_orthogonal(),
            Vector::Rational(v) => v.is_orthogonal(),
        }
    }

    pub fn is_diagonal(&self) -> bool {
        match self {
            Vector::Int(v) => v.is_diagonal(),
            Vector::Rational(v) => v.is_diagonal(),
        }
    }

    pub fn is_multiple_of_45_degree(&self) -> bool {
        self.is_orthogonal() || self.is_diagonal()
    }

    /// Signum of the scalar product of this vector and other.
    pub fn projection(&self, other: &Vector) -> Signum {
        // this.projection(other) == other.projection(<this type>)
        match (self, other) {
            (Vector::Int(a), Vector::Int(b)) => b.projection_int(a),
            (Vector::Int(a), Vector::Rational(b)) => b.projection_int(a),
            (Vector::Rational(a), Vector::Int(b)) => {
                // IntVector.projection(RationalVector other) -> other.projection(this)
                a.projection_int(b)
            }
            (Vector::Rational(a), Vector::Rational(b)) => b.projection_rational(a),
        }
    }

    /// Returns an approximation of the scalar product of this vector with other by a double.
    pub fn scalar_product(&self, other: &Vector) -> f64 {
        match (self, other) {
            (Vector::Int(a), Vector::Int(b)) => b.scalar_product_int(a),
            (Vector::Int(a), Vector::Rational(b)) => b.scalar_product_int(a),
            (Vector::Rational(a), Vector::Int(b)) => a.scalar_product_int(b),
            (Vector::Rational(a), Vector::Rational(b)) => b.scalar_product_rational(a),
        }
    }

    pub fn to_float(&self) -> FloatPoint {
        match self {
            Vector::Int(v) => v.to_float(),
            Vector::Rational(v) => v.to_float(),
        }
    }

    pub fn turn_90_degree(&self, factor: i32) -> Vector {
        match self {
            Vector::Int(v) => Vector::Int(v.turn_90_degree(factor)),
            Vector::Rational(v) => Vector::Rational(Box::new(v.turn_90_degree(factor))),
        }
    }

    pub fn mirror_at_x_axis(&self) -> Vector {
        match self {
            Vector::Int(v) => Vector::Int(v.mirror_at_x_axis()),
            Vector::Rational(v) => Vector::Rational(Box::new(v.mirror_at_x_axis())),
        }
    }

    pub fn mirror_at_y_axis(&self) -> Vector {
        match self {
            Vector::Int(v) => Vector::Int(v.mirror_at_y_axis()),
            Vector::Rational(v) => Vector::Rational(Box::new(v.mirror_at_y_axis())),
        }
    }

    /// Returns an approximation of the Euclidean length of this vector.
    pub fn length_approx(&self) -> f64 {
        self.to_float().size()
    }

    /// Returns an approximation of the cosinus of the angle between this vector and other.
    pub fn cos_angle(&self, other: &Vector) -> f64 {
        let mut result = self.scalar_product(other);
        result /= self.to_float().size() * other.to_float().size();
        result
    }

    /// Returns an approximation of the signed angle between this vector and other
    /// (Java `angleApprox(Vector)`).
    pub fn angle_approx_to(&self, other: &Vector) -> f64 {
        let mut result = jmath::acos(self.cos_angle(other));
        if self.side_of(other) == Side::OnTheLeft {
            result = -result;
        }
        result
    }

    /// Returns an approximation of the signed angle between this vector and the x axis.
    pub fn angle_approx(&self) -> f64 {
        let other = Vector::Int(IntVector::new(1, 0));
        other.angle_approx_to(self)
    }

    /// Returns an approximation vector of this vector with the same direction and length length.
    pub fn change_length_approx(&self, length: f64) -> Vector {
        match self {
            Vector::Int(v) => v.change_length_approx(length),
            Vector::Rational(v) => v.change_length_approx(length),
        }
    }

    pub fn to_normalized_direction(&self) -> Direction {
        match self {
            Vector::Int(v) => Direction::Int(v.to_normalized_direction()),
            Vector::Rational(v) => v.to_normalized_direction(),
        }
    }

    /// Package private `addTo(IntPoint)`.
    pub fn add_to_int_point(&self, point: &IntPoint) -> Point {
        match self {
            Vector::Int(v) => v.add_to_int_point(point),
            Vector::Rational(v) => v.add_to_int_point(point),
        }
    }

    /// Package private `addTo(RationalPoint)`.
    pub fn add_to_rational_point(&self, point: &RationalPoint) -> Point {
        match self {
            Vector::Int(v) => v.add_to_rational_point(point),
            Vector::Rational(v) => v.add_to_rational_point(point),
        }
    }

    /// Returns the IntVector, panicking like the Java cast `(IntVector) v` otherwise.
    #[inline]
    pub fn as_int(&self) -> IntVector {
        match self {
            Vector::Int(v) => *v,
            Vector::Rational(_) => panic!("ClassCastException: RationalVector is not an IntVector"),
        }
    }
}

impl From<IntVector> for Vector {
    fn from(v: IntVector) -> Self {
        Vector::Int(v)
    }
}
