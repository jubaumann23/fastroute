//! Port of `Direction.java`.
//!
//! A Direction is an equivalence class of vectors. Two vectors define the same Direction, if they
//! point into the same direction. Directions are preferred to angles, because arithmetic with
//! angles is in general not exact.

use crate::big_int_direction::BigIntDirection;
use crate::float_point::FloatPoint;
use crate::int_direction::IntDirection;
use crate::int_vector::IntVector;
use crate::java_compat::math_round_i32;
use crate::jmath;
use crate::point::Point;
use crate::side::Side;
use crate::signum::Signum;
use crate::vector::Vector;

/// Closed hierarchy `Direction = IntDirection | BigIntDirection`.
///
/// `PartialEq` follows Java `Direction.equals`: variants must match (Java `getClass()` check),
/// the vectors must be collinear and point into the same direction. Two zero directions compare
/// equal (Java only does so for the identical `Direction.NULL` instance).
#[derive(Clone, Debug)]
pub enum Direction {
    Int(IntDirection),
    BigInt(Box<BigIntDirection>),
}

impl Direction {
    pub const NULL: Direction = Direction::Int(IntDirection::NULL);
    /// The direction to the east.
    pub const RIGHT: Direction = Direction::Int(IntDirection::RIGHT);
    /// The direction to the northeast.
    pub const RIGHT45: Direction = Direction::Int(IntDirection::RIGHT45);
    /// The direction to the north.
    pub const UP: Direction = Direction::Int(IntDirection::UP);
    /// The direction to the northwest.
    pub const UP45: Direction = Direction::Int(IntDirection::UP45);
    /// The direction to the west.
    pub const LEFT: Direction = Direction::Int(IntDirection::LEFT);
    /// The direction to the southwest.
    pub const LEFT45: Direction = Direction::Int(IntDirection::LEFT45);
    /// The direction to the south.
    pub const DOWN: Direction = Direction::Int(IntDirection::DOWN);
    /// The direction to the southeast.
    pub const DOWN45: Direction = Direction::Int(IntDirection::DOWN45);

    /// Creates a Direction from the input Vector.
    #[inline]
    pub fn get_instance(vector: &Vector) -> Direction {
        vector.to_normalized_direction()
    }

    /// Calculates the direction from `from` to `to`. If from and to are equal, None is returned.
    pub fn get_instance_from_points(from: &Point, to: &Point) -> Option<Direction> {
        if from == to {
            return None;
        }
        Some(Self::get_instance(&to.difference_by(from)))
    }

    /// Creates a Direction whose angle with the x-axis is nearly equal to angle.
    pub fn get_instance_approx(angle: f64) -> Direction {
        let scale_factor = 10000.0;
        let x = math_round_i32(jmath::cos(angle) * scale_factor);
        let y = math_round_i32(jmath::sin(angle) * scale_factor);
        Self::get_instance(&Vector::Int(IntVector::new(x, y)))
    }

    /// Returns any Vector pointing into this direction.
    pub fn get_vector(&self) -> Vector {
        match self {
            Direction::Int(d) => d.get_vector(),
            Direction::BigInt(d) => d.get_vector(),
        }
    }

    /// Returns true, if the direction is horizontal or vertical.
    pub fn is_orthogonal(&self) -> bool {
        match self {
            Direction::Int(d) => d.is_orthogonal(),
            Direction::BigInt(d) => d.is_orthogonal(),
        }
    }

    /// Returns true, if the direction is diagonal.
    pub fn is_diagonal(&self) -> bool {
        match self {
            Direction::Int(d) => d.is_diagonal(),
            Direction::BigInt(d) => d.is_diagonal(),
        }
    }

    /// Returns true, if the direction is orthogonal or diagonal.
    pub fn is_multiple_of_45_degree(&self) -> bool {
        self.is_orthogonal() || self.is_diagonal()
    }

    /// Turns the direction by factor times 45 degree.
    pub fn turn_45_degree(&self, factor: i32) -> Direction {
        match self {
            Direction::Int(d) => Direction::Int(d.turn_45_degree(factor)),
            Direction::BigInt(d) => d.turn_45_degree(factor),
        }
    }

    /// Returns the opposite direction of this direction.
    pub fn opposite(&self) -> Direction {
        match self {
            Direction::Int(d) => Direction::Int(d.opposite()),
            Direction::BigInt(d) => Direction::BigInt(Box::new(d.opposite())),
        }
    }

    /// Returns the IntDirection, panicking like the Java cast `(IntDirection) dir` if this is a
    /// BigIntDirection.
    #[inline]
    pub fn as_int(&self) -> IntDirection {
        match self {
            Direction::Int(d) => *d,
            Direction::BigInt(_) => {
                panic!("ClassCastException: BigIntDirection is not an IntDirection")
            }
        }
    }

    /// Let L be the line from the Zero Vector to other.get_vector(). Returns ON_THE_LEFT, if
    /// this.get_vector() is on the left of L, ON_THE_RIGHT if on the right, COLLINEAR otherwise.
    pub fn side_of(&self, other: &Direction) -> Side {
        self.get_vector().side_of(&other.get_vector())
    }

    /// Signum of the scalar product of vectors representing this direction and other.
    pub fn projection(&self, other: &Direction) -> Signum {
        self.get_vector().projection(&other.get_vector())
    }

    /// Calculates an approximation of the direction in the middle of this direction and other.
    pub fn middle_approx(&self, other: &Direction) -> Direction {
        let v1 = self.get_vector().to_float();
        let v2 = other.get_vector().to_float();
        let length1 = v1.size();
        let length2 = v2.size();
        let x = v1.x / length1 + v2.x / length2;
        let y = v1.y / length1 + v2.y / length2;
        let scale_factor = 1000.0;
        let vm = Vector::Int(IntVector::new(
            math_round_i32(x * scale_factor),
            math_round_i32(y * scale_factor),
        ));
        Direction::get_instance(&vm)
    }

    /// Returns 1, if the angle between p1 and this direction is bigger than the angle between p2
    /// and this direction, 0 if p1 is equal to p2, and -1 otherwise.
    pub fn compare_from(&self, p1: &Direction, p2: &Direction) -> i32 {
        if p1.compare_to(self) >= 0 {
            if p2.compare_to(self) >= 0 {
                p1.compare_to(p2)
            } else {
                -1
            }
        } else if p2.compare_to(self) >= 0 {
            1
        } else {
            p1.compare_to(p2)
        }
    }

    /// Returns an approximation of the signed angle corresponding to this direction.
    pub fn angle_approx(&self) -> f64 {
        self.get_vector().angle_approx()
    }

    /// Implements Java `Comparable<Direction>.compareTo`: 1 if this direction has a strictly
    /// bigger angle with the positive x-axis than other, 0 if equal, -1 otherwise.
    pub fn compare_to(&self, other: &Direction) -> i32 {
        match self {
            Direction::Int(d) => d.compare_to(other),
            Direction::BigInt(d) => d.compare_to(other),
        }
    }

    /// Approximation of this direction's vector as FloatPoint.
    pub fn to_float(&self) -> FloatPoint {
        self.get_vector().to_float()
    }
}

impl From<IntDirection> for Direction {
    fn from(d: IntDirection) -> Self {
        Direction::Int(d)
    }
}

impl PartialEq for Direction {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Direction::Int(a), Direction::Int(b)) => a == b,
            (Direction::BigInt(_), Direction::BigInt(_)) => {
                if self.side_of(other) != Side::Collinear {
                    return false;
                }
                self.get_vector().projection(&other.get_vector()) == Signum::Positive
            }
            _ => false,
        }
    }
}
