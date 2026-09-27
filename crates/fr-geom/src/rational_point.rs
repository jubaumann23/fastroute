//! Port of `RationalPoint.java`.
//!
//! Points in the projective plane represented by 3 infinite precision integer coordinates
//! x, y, z. The affine point is (x/z, y/z); points with z = 0 lie on the line at infinity.

use num_bigint::BigInt;
use num_integer::Integer;
use num_traits::{One, Signed};

use crate::big_int_aux::{add_rational_coordinates, determinant};
use crate::float_point::FloatPoint;
use crate::int_box::IntBox;
use crate::int_octagon::IntOctagon;
use crate::int_point::IntPoint;
use crate::int_vector::IntVector;
use crate::java_compat::{
    big, bigint_double_value, bigint_hash_code, bigint_int_value, bigint_signum,
};
use crate::limits::crit_int_big;
use crate::line::Line;
use crate::point::Point;
use crate::rational_vector::RationalVector;
use crate::side::Side;
use crate::vector::Vector;

#[derive(Clone, Debug)]
pub struct RationalPoint {
    pub x: BigInt,
    pub y: BigInt,
    pub z: BigInt,
}

impl RationalPoint {
    /// Creates a RationalPoint representing (x / z, y / z). Panics if z < 0 (Java throws
    /// IllegalArgumentException).
    pub fn new(x: BigInt, y: BigInt, z: BigInt) -> Self {
        if bigint_signum(&z) < 0 {
            panic!("RationalPoint: z is expected to be >= 0");
        }
        RationalPoint { x, y, z }
    }

    /// Creates a RationalPoint from an IntPoint.
    pub fn from_int(p: &IntPoint) -> Self {
        RationalPoint {
            x: BigInt::from(p.x),
            y: BigInt::from(p.y),
            z: BigInt::one(),
        }
    }

    pub fn to_float(&self) -> FloatPoint {
        let mut xd = bigint_double_value(&self.x);
        let mut yd = bigint_double_value(&self.y);
        let zd = bigint_double_value(&self.z);
        if zd == 0.0 {
            xd = f32::MAX as f64;
            yd = f32::MAX as f64;
        } else {
            xd /= zd;
            yd /= zd;
        }
        FloatPoint::new(xd, yd)
    }

    pub fn get_id(&self) -> i32 {
        let mut result = bigint_hash_code(&self.x);
        result = result
            .wrapping_mul(31)
            .wrapping_add(bigint_hash_code(&self.y));
        result
            .wrapping_mul(31)
            .wrapping_add(bigint_hash_code(&self.z))
    }

    /// Java `hashCode()`: hash of the gcd-reduced coordinates (0 for infinite points).
    pub fn hash_code(&self) -> i32 {
        if bigint_signum(&self.z) == 0 {
            return 0;
        }
        let gcd = self.x.abs().gcd(&self.y.abs()).gcd(&self.z);
        let (rx, ry, rz) = if gcd > BigInt::one() {
            (&self.x / &gcd, &self.y / &gcd, &self.z / &gcd)
        } else {
            (self.x.clone(), self.y.clone(), self.z.clone())
        };
        let mut result = bigint_hash_code(&rx);
        result = result.wrapping_mul(31).wrapping_add(bigint_hash_code(&ry));
        result.wrapping_mul(31).wrapping_add(bigint_hash_code(&rz))
    }

    pub fn is_infinite(&self) -> bool {
        bigint_signum(&self.z) == 0
    }

    pub fn surrounding_box(&self) -> IntBox {
        let fp = self.to_float();
        let llx = fp.x.floor() as i32;
        let lly = fp.y.floor() as i32;
        let urx = fp.x.ceil() as i32;
        let ury = fp.y.ceil() as i32;
        IntBox::new(llx, lly, urx, ury)
    }

    pub fn surrounding_octagon(&self) -> IntOctagon {
        let fp = self.to_float();
        let lx = fp.x.floor() as i32;
        let ly = fp.y.floor() as i32;
        let rx = fp.x.ceil() as i32;
        let uy = fp.y.ceil() as i32;

        let tmp = fp.x - fp.y;
        let ulx = tmp.floor() as i32;
        let lrx = tmp.ceil() as i32;

        let tmp = fp.x + fp.y;
        let llx = tmp.floor() as i32;
        let urx = tmp.ceil() as i32;
        IntOctagon::new(lx, ly, rx, uy, ulx, lrx, llx, urx)
    }

    pub fn is_contained_in(&self, b: &IntBox) -> bool {
        let tmp = big(b.ll.x as i64) * &self.z;
        if self.x < tmp {
            return false;
        }
        let tmp = big(b.ll.y as i64) * &self.z;
        if self.y < tmp {
            return false;
        }
        let tmp = big(b.ur.x as i64) * &self.z;
        if self.x > tmp {
            return false;
        }
        let tmp = big(b.ur.y as i64) * &self.z;
        self.y <= tmp
    }

    /// Returns the translation of this point by vector.
    pub fn translate_by(&self, vector: &Vector) -> Point {
        if *vector == Vector::ZERO {
            return Point::Rational(Box::new(self.clone()));
        }
        vector.add_to_rational_point(self)
    }

    /// Package private `translateBy(IntVector)`.
    pub fn translate_by_int_vector(&self, v: &IntVector) -> Point {
        self.translate_by_rational_vector(&RationalVector::from_int(v))
    }

    /// Package private `translateBy(RationalVector)`.
    pub fn translate_by_rational_vector(&self, v: &RationalVector) -> Point {
        let [x, y, z] = add_rational_coordinates([&self.x, &self.y, &self.z], [&v.x, &v.y, &v.z]);
        Point::Rational(Box::new(RationalPoint::new(x, y, z)))
    }

    /// Returns the difference vector of this point and other.
    pub fn difference_by(&self, other: &Point) -> Vector {
        match other {
            // other.differenceBy(this).negate()
            Point::Int(o) => o.difference_by_rational(self).negate(),
            Point::Rational(o) => o.difference_by_rational(self).negate(),
        }
    }

    /// Package private `differenceBy(IntPoint)`.
    pub fn difference_by_int(&self, other: &IntPoint) -> Vector {
        self.difference_by_rational(&RationalPoint::from_int(other))
    }

    /// Package private `differenceBy(RationalPoint)`.
    pub fn difference_by_rational(&self, other: &RationalPoint) -> Vector {
        let ox = -&other.x;
        let oy = -&other.y;
        let [x, y, z] = add_rational_coordinates([&self.x, &self.y, &self.z], [&ox, &oy, &other.z]);
        Vector::Rational(Box::new(RationalVector::new(x, y, z)))
    }

    pub fn side_of_line(&self, line: &Line) -> Side {
        let me = Point::Rational(Box::new(self.clone()));
        me.side_of(&line.a, &line.b)
    }

    /// Only implemented for lines consisting of IntPoints. Note: the sign of the `det * v.x`
    /// term differs from IntPoint.perpendicular_projection; this reproduces the Java code as is.
    pub fn perpendicular_projection(&self, line: &Line) -> Point {
        let v = line.b.difference_by(&line.a).as_int();
        let vxvx = big(v.x as i64 * v.x as i64);
        let vyvy = big(v.y as i64 * v.y as i64);
        let vxvy = big(v.x as i64 * v.y as i64);
        let mut denominator = &vxvx + &vyvy;
        let det = big(line.a.as_int().determinant(&line.b.as_int()));

        let mut proj_x = &vxvx * &self.x + &vxvy * &self.y + &det * big(v.y as i64) * &self.z;
        let mut proj_y = &vxvy * &self.x + &vyvy * &self.y + &det * big(v.x as i64) * &self.z;

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
                let crit = crit_int_big();
                if proj_x.abs() <= crit && proj_y.abs() <= crit {
                    return Point::Int(IntPoint::new(
                        bigint_int_value(&proj_x),
                        bigint_int_value(&proj_y),
                    ));
                }
                denominator = BigInt::one();
            }
        }
        Point::Rational(Box::new(RationalPoint::new(proj_x, proj_y, denominator)))
    }

    /// Package private `compareX(RationalPoint)`.
    pub fn compare_x_rational(&self, other: &RationalPoint) -> i32 {
        let tmp1 = &self.x * &other.z;
        let tmp2 = &other.x * &self.z;
        cmp_to_i32(tmp1.cmp(&tmp2))
    }

    /// Package private `compareX(IntPoint)`.
    pub fn compare_x_int(&self, other: &IntPoint) -> i32 {
        let tmp1 = &self.z * big(other.x as i64);
        cmp_to_i32(self.x.cmp(&tmp1))
    }

    /// Package private `compareY(RationalPoint)`.
    pub fn compare_y_rational(&self, other: &RationalPoint) -> i32 {
        let tmp1 = &self.y * &other.z;
        let tmp2 = &other.y * &self.z;
        cmp_to_i32(tmp1.cmp(&tmp2))
    }

    /// Package private `compareY(IntPoint)`.
    pub fn compare_y_int(&self, other: &IntPoint) -> i32 {
        let tmp1 = &self.z * big(other.y as i64);
        cmp_to_i32(self.y.cmp(&tmp1))
    }
}

#[inline]
fn cmp_to_i32(o: std::cmp::Ordering) -> i32 {
    match o {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

impl PartialEq for RationalPoint {
    fn eq(&self, other: &Self) -> bool {
        let det = determinant(&self.x, &other.x, &self.z, &other.z);
        if bigint_signum(&det) != 0 {
            return false;
        }
        let det = determinant(&self.y, &other.y, &self.z, &other.z);
        bigint_signum(&det) == 0
    }
}

impl Eq for RationalPoint {}

impl std::hash::Hash for RationalPoint {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.hash_code().hash(state)
    }
}
