//! Port of `Point.java`: abstract class for points in the plane (`IntPoint | RationalPoint`).

use num_bigint::BigInt;
use num_traits::{One, Signed};

use crate::direction::Direction;
use crate::float_point::FloatPoint;
use crate::int_box::IntBox;
use crate::int_octagon::IntOctagon;
use crate::int_point::IntPoint;
use crate::java_compat::{bigint_int_value, bigint_signum};
use crate::limits::{crit_int_big, CRIT_INT};
use crate::line::Line;
use crate::rational_point::RationalPoint;
use crate::side::Side;
use crate::vector::Vector;

/// Closed hierarchy `Point = IntPoint | RationalPoint`.
///
/// `PartialEq`/`Hash` follow Java `equals`/`hashCode`: an IntPoint never equals a RationalPoint
/// (Java compares `getClass()`), rational points compare by cross multiplication.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Point {
    Int(IntPoint),
    Rational(Box<RationalPoint>),
}

impl Point {
    /// Standard implementation of the zero point.
    pub const ZERO: Point = Point::Int(IntPoint::new(0, 0));

    /// Creates an IntPoint from x and y; a RationalPoint if x or y is too big for an IntPoint.
    pub fn get_instance(x: i32, y: i32) -> Point {
        let result = IntPoint::new(x, y);
        if x.wrapping_abs() > CRIT_INT || y.wrapping_abs() > CRIT_INT {
            return Point::Rational(Box::new(RationalPoint::from_int(&result)));
        }
        Point::Int(result)
    }

    /// Factory method for creating a Point from 3 BigIntegers.
    pub fn get_instance_big(x: BigInt, y: BigInt, z: BigInt) -> Point {
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
                return Point::Int(IntPoint::new(bigint_int_value(&x), bigint_int_value(&y)));
            }
        }
        Point::Rational(Box::new(RationalPoint::new(x, y, z)))
    }

    /// Returns the IntPoint, panicking like the Java cast `(IntPoint) p` otherwise.
    #[inline]
    pub fn as_int(&self) -> IntPoint {
        match self {
            Point::Int(p) => *p,
            Point::Rational(_) => panic!("ClassCastException: RationalPoint is not an IntPoint"),
        }
    }

    /// Java `p instanceof IntPoint`.
    #[inline]
    pub fn is_int_point(&self) -> bool {
        matches!(self, Point::Int(_))
    }

    /// Returns the IntPoint if this is one.
    #[inline]
    pub fn to_int_point(&self) -> Option<IntPoint> {
        match self {
            Point::Int(p) => Some(*p),
            Point::Rational(_) => None,
        }
    }

    /// Returns the translation of this point by vector.
    pub fn translate_by(&self, vector: &Vector) -> Point {
        match self {
            Point::Int(p) => p.translate_by(vector),
            Point::Rational(p) => p.translate_by(vector),
        }
    }

    /// Returns the difference vector of this point and other.
    pub fn difference_by(&self, other: &Point) -> Vector {
        match self {
            Point::Int(p) => p.difference_by(other),
            Point::Rational(p) => p.difference_by(other),
        }
    }

    /// Approximates the coordinates of this point by float coordinates.
    #[inline]
    pub fn to_float(&self) -> FloatPoint {
        match self {
            Point::Int(p) => p.to_float(),
            Point::Rational(p) => p.to_float(),
        }
    }

    /// Returns a unique ID for this point for deterministic tie-breaking.
    pub fn get_id(&self) -> i32 {
        match self {
            Point::Int(p) => p.get_id(),
            Point::Rational(p) => p.get_id(),
        }
    }

    /// Java `hashCode()`.
    pub fn hash_code(&self) -> i32 {
        match self {
            Point::Int(p) => p.hash_code(),
            Point::Rational(p) => p.hash_code(),
        }
    }

    /// Returns true, if this Point is a RationalPoint with denominator z = 0.
    pub fn is_infinite(&self) -> bool {
        match self {
            Point::Int(p) => p.is_infinite(),
            Point::Rational(p) => p.is_infinite(),
        }
    }

    /// Creates the smallest Box with integer coordinates containing this point.
    pub fn surrounding_box(&self) -> IntBox {
        match self {
            Point::Int(p) => p.surrounding_box(),
            Point::Rational(p) => p.surrounding_box(),
        }
    }

    /// Creates the smallest Octagon with integer coordinates containing this point.
    pub fn surrounding_octagon(&self) -> IntOctagon {
        match self {
            Point::Int(p) => p.surrounding_octagon(),
            Point::Rational(p) => p.surrounding_octagon(),
        }
    }

    /// Returns true, if this point lies in the interior or on the border of box.
    pub fn is_contained_in(&self, b: &IntBox) -> bool {
        match self {
            Point::Int(p) => p.is_contained_in(b),
            Point::Rational(p) => p.is_contained_in(b),
        }
    }

    /// Returns the side of a line on which this point lies (Java `sideOf(Line)`).
    pub fn side_of_line(&self, line: &Line) -> Side {
        match self {
            Point::Int(p) => p.side_of_line(line),
            Point::Rational(p) => p.side_of_line(line),
        }
    }

    /// Returns ON_THE_LEFT, if this Point is on the left of the line from p1 to p2;
    /// ON_THE_RIGHT, if on the right; COLLINEAR otherwise.
    pub fn side_of(&self, p1: &Point, p2: &Point) -> Side {
        let v1 = self.difference_by(p1);
        let v2 = p2.difference_by(p1);
        v1.side_of(&v2)
    }

    /// Returns the nearest point to this point on line.
    pub fn perpendicular_projection(&self, line: &Line) -> Point {
        match self {
            Point::Int(p) => p.perpendicular_projection(line),
            Point::Rational(p) => p.perpendicular_projection(line),
        }
    }

    /// Calculates the perpendicular direction from this point to line. Returns Direction::NULL,
    /// if this point lies on line.
    pub fn perpendicular_direction(&self, line: &Line) -> Direction {
        self.perpendicular_direction_opt(line)
            .unwrap_or(Direction::NULL)
    }

    /// Like [`Self::perpendicular_direction`], but returns None where Java returns the
    /// `Direction.NULL` singleton (Java code compares that result by reference).
    pub fn perpendicular_direction_opt(&self, line: &Line) -> Option<Direction> {
        let side = self.side_of_line(line);
        if side == Side::Collinear {
            return None;
        }
        if side == Side::OnTheRight {
            Some(line.direction().turn_45_degree(2))
        } else {
            Some(line.direction().turn_45_degree(6))
        }
    }

    /// Returns 1, if this Point has a strict bigger x coordinate than other, 0 if equal, -1
    /// otherwise.
    pub fn compare_x(&self, other: &Point) -> i32 {
        match (self, other) {
            (Point::Int(a), _) => a.compare_x(other),
            (Point::Rational(a), Point::Int(b)) => -(-a.compare_x_int(b)),
            (Point::Rational(a), Point::Rational(b)) => -b.compare_x_rational(a),
        }
    }

    /// Returns 1, if this Point has a strict bigger y coordinate than other, 0 if equal, -1
    /// otherwise.
    pub fn compare_y(&self, other: &Point) -> i32 {
        match (self, other) {
            (Point::Int(a), _) => a.compare_y(other),
            (Point::Rational(a), Point::Int(b)) => -(-a.compare_y_int(b)),
            (Point::Rational(a), Point::Rational(b)) => -b.compare_y_rational(a),
        }
    }

    /// Returns compare_x(other) if not 0, otherwise compare_y(other).
    pub fn compare_xy(&self, other: &Point) -> i32 {
        let result = self.compare_x(other);
        if result == 0 {
            return self.compare_y(other);
        }
        result
    }

    /// Turns this point by factor times 90 degree around pole.
    pub fn turn_90_degree(&self, factor: i32, pole: &Point) -> Point {
        let v = self.difference_by(pole).turn_90_degree(factor);
        pole.translate_by(&v)
    }

    /// Mirrors this point at the vertical line through pole.
    pub fn mirror_vertical(&self, pole: &Point) -> Point {
        let v = self.difference_by(pole).mirror_at_y_axis();
        pole.translate_by(&v)
    }

    /// Mirrors this point at the horizontal line through pole.
    pub fn mirror_horizontal(&self, pole: &Point) -> Point {
        let v = self.difference_by(pole).mirror_at_x_axis();
        pole.translate_by(&v)
    }
}

impl From<IntPoint> for Point {
    #[inline]
    fn from(p: IntPoint) -> Self {
        Point::Int(p)
    }
}

impl From<RationalPoint> for Point {
    fn from(p: RationalPoint) -> Self {
        Point::Rational(Box::new(p))
    }
}
