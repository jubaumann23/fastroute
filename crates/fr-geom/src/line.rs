//! Port of `Line.java`: directed lines in the plane defined by two points.

use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::OnceLock;

use crate::direction::Direction;
use crate::float_line::FloatLine;
use crate::float_point::FloatPoint;
use crate::int_direction::IntDirection;
use crate::int_point::IntPoint;
use crate::int_vector::IntVector;
use crate::java_compat::{
    big, bigint_double_value, bigint_int_value, bigint_signum, math_round_i32, INT_MAX_F64,
};
use crate::jmath;
use crate::limits::CRIT_INT;
use crate::point::Point;
use crate::rational_point::RationalPoint;
use crate::side::Side;
use crate::signum::Signum;
use crate::tile_shape::TileShape;
use crate::vector::Vector;

use num_bigint::BigInt;
use num_traits::One;

/// A directed line through `a` and `b`.
///
/// The direction is computed lazily and cached (like the transient `dir` field in Java).
/// `PartialEq` is Java `Line.equals`: same set of points and same orientation.
#[derive(Clone)]
pub struct Line {
    pub a: Point,
    pub b: Point,
    dir: DirCache,
}

/// Lazily computed direction of a line. `IntDirection`s (the normal case) are cached in atomics,
/// which are much cheaper to initialize and to clone than a `OnceLock`; other directions in a
/// `OnceLock`.
#[derive(Default)]
struct DirCache {
    /// 1 if `packed` holds the direction.
    int_set: AtomicU8,
    /// `IntDirection` x (high 32 bits) and y (low 32 bits).
    packed: AtomicU64,
    other: OnceLock<Direction>,
}

impl DirCache {
    #[inline]
    fn with(dir: Direction) -> DirCache {
        let cache = DirCache::default();
        cache.store(dir);
        cache
    }

    #[inline]
    fn store(&self, dir: Direction) {
        match dir {
            Direction::Int(d) => {
                self.packed.store(((d.x as u32 as u64) << 32) | d.y as u32 as u64, Ordering::Relaxed);
                self.int_set.store(1, Ordering::Release);
            }
            other => {
                let _ = self.other.set(other);
            }
        }
    }

    #[inline]
    fn get(&self) -> Option<Direction> {
        if self.int_set.load(Ordering::Acquire) == 1 {
            let p = self.packed.load(Ordering::Relaxed);
            return Some(Direction::Int(IntDirection::new((p >> 32) as u32 as i32, p as u32 as i32)));
        }
        self.other.get().cloned()
    }
}

impl Clone for DirCache {
    #[inline]
    fn clone(&self) -> DirCache {
        DirCache {
            int_set: AtomicU8::new(self.int_set.load(Ordering::Acquire)),
            packed: AtomicU64::new(self.packed.load(Ordering::Relaxed)),
            other: self.other.clone(),
        }
    }
}

impl std::fmt::Debug for Line {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Line")
            .field("a", &self.a)
            .field("b", &self.b)
            .finish()
    }
}

impl Line {
    /// Creates a directed Line from two points.
    pub fn new(a: Point, b: Point) -> Line {
        if !(a.is_int_point() && b.is_int_point()) {
            log::warn!("Line(a, b) only implemented for IntPoints till now");
        }
        Line {
            a,
            b,
            dir: DirCache::default(),
        }
    }

    /// Creates a directed Line from two IntPoints.
    #[inline]
    pub fn from_int_points(a: IntPoint, b: IntPoint) -> Line {
        Line {
            a: Point::Int(a),
            b: Point::Int(b),
            dir: DirCache::default(),
        }
    }

    /// Creates a directed Line from four integer coordinates.
    #[inline]
    pub fn new_ints(ax: i32, ay: i32, bx: i32, by: i32) -> Line {
        Line::from_int_points(IntPoint::new(ax, ay), IntPoint::new(bx, by))
    }

    /// Creates a directed Line from a point and a direction (Java `new Line(Point, Direction)`,
    /// which stores `dir` as the cached direction).
    pub fn from_point_direction(a: Point, dir: Direction) -> Line {
        let b = a.translate_by(&dir.get_vector());
        if !(a.is_int_point() && b.is_int_point()) {
            log::warn!("Line(a, dir) only implemented for IntPoints till now");
        }
        Line { a, b, dir: DirCache::with(dir) }
    }

    /// Creates a directed line from a point and a direction (Java `Line.getInstance`; the
    /// direction is not cached).
    pub fn get_instance(a: Point, dir: &Direction) -> Line {
        let b = a.translate_by(&dir.get_vector());
        Line::new(a, b)
    }

    /// Returns a unique ID for deterministic tie-breaking.
    pub fn get_id(&self) -> i32 {
        self.a
            .get_id()
            .wrapping_mul(31)
            .wrapping_add(self.b.get_id())
    }

    /// Returns true, if this and other define the same line. Only works for lines consisting of
    /// IntPoints (panics otherwise, like the Java casts).
    pub fn fast_equals(&self, other: &Line) -> bool {
        let this_a = self.a.as_int();
        let this_b = self.b.as_int();
        let other_a = other.a.as_int();
        let dx1 = other_a.x.wrapping_sub(this_a.x) as f64;
        let dy1 = other_a.y.wrapping_sub(this_a.y) as f64;
        let dx2 = this_b.x.wrapping_sub(this_a.x) as f64;
        let dy2 = this_b.y.wrapping_sub(this_a.y) as f64;
        let det = dx1 * dy2 - dx2 * dy1;
        if det != 0.0 {
            return false;
        }
        self.direction() == other.direction()
    }

    /// Gets the direction of this directed line.
    #[inline]
    pub fn direction(&self) -> Direction {
        if let Some(d) = self.dir.get() {
            return d;
        }
        let d = Direction::get_instance(&self.b.difference_by(&self.a));
        self.dir.store(d.clone());
        d
    }

    /// The direction as IntDirection (Java cast `(IntDirection) line.direction()`).
    #[inline]
    pub fn int_direction(&self) -> IntDirection {
        self.direction().as_int()
    }

    /// Returns ON_THE_LEFT, if this Line is on the left of point, ON_THE_RIGHT, if this Line is
    /// on the right of point, and COLLINEAR, if this Line contains point.
    #[inline]
    pub fn side_of(&self, point: &Point) -> Side {
        if let (Point::Int(p), Point::Int(a), Point::Int(b)) = (point, &self.a, &self.b) {
            // Fast path, identical to point.sideOf(this).negate() for IntPoints:
            // v1 = p - a, v2 = b - a, result = Side.of(v1.x * v2.y - v1.y * v2.x)
            let v1x = a.x.wrapping_sub(p.x).wrapping_neg();
            let v1y = a.y.wrapping_sub(p.y).wrapping_neg();
            let v2x = a.x.wrapping_sub(b.x).wrapping_neg();
            let v2y = a.y.wrapping_sub(b.y).wrapping_neg();
            let det = v1x as f64 * v2y as f64 - v1y as f64 * v2x as f64;
            return Side::of(det);
        }
        point.side_of_line(self).negate()
    }

    /// Returns COLLINEAR, if point is on the line with the given tolerance. Otherwise
    /// ON_THE_LEFT, if this line is on the left of point, or ON_THE_RIGHT. Only implemented for
    /// IntPoint lines.
    #[inline]
    pub fn side_of_float_tol(&self, point: &FloatPoint, tolerance: f64) -> Side {
        let this_a = self.a.as_int();
        let this_b = self.b.as_int();
        let det = this_b.y.wrapping_sub(this_a.y) as f64 * (point.x - this_a.x as f64)
            - this_b.x.wrapping_sub(this_a.x) as f64 * (point.y - this_a.y as f64);
        if det - tolerance > 0.0 {
            Side::OnTheLeft
        } else if det + tolerance < 0.0 {
            Side::OnTheRight
        } else {
            Side::Collinear
        }
    }

    /// Java `sideOf(FloatPoint)`.
    #[inline]
    pub fn side_of_float(&self, point: &FloatPoint) -> Side {
        self.side_of_float_tol(point, 0.0)
    }

    /// Returns ON_THE_LEFT, if this line is on the left of the intersection of p1 and p2,
    /// ON_THE_RIGHT if on the right, and COLLINEAR, if all 3 lines intersect in exactly 1 point.
    pub fn side_of_intersection(&self, p1: &Line, p2: &Line) -> Side {
        let intersection_approx = p1.intersection_approx(p2);
        let result = self.side_of_float_tol(&intersection_approx, 1.0);
        if result == Side::Collinear {
            // Previous calculation was with FloatPoints and a tolerance for performance reasons.
            // Make an exact check for collinearity now.
            let intersection = p1.intersection(p2);
            return self.side_of(&intersection);
        }
        result
    }

    /// Looks, if all interior points of tile are on the right side of this line.
    pub fn is_on_the_left(&self, tile: &TileShape) -> bool {
        for i in 0..tile.border_line_count() {
            if self.side_of(&tile.corner(i)) == Side::OnTheRight {
                return false;
            }
        }
        true
    }

    /// Looks, if all interior points of tile are on the left side of this line.
    pub fn is_on_the_right(&self, tile: &TileShape) -> bool {
        for i in 0..tile.border_line_count() {
            if self.side_of(&tile.corner(i)) == Side::OnTheLeft {
                return false;
            }
        }
        true
    }

    /// Returns the signed distance of this line from point. Positive, if the line is on the left
    /// of point, else negative. Only implemented for IntPoint lines.
    pub fn signed_distance(&self, point: &FloatPoint) -> f64 {
        let this_a = self.a.as_int();
        let this_b = self.b.as_int();
        let dx = this_b.x.wrapping_sub(this_a.x) as f64;
        let dy = this_b.y.wrapping_sub(this_a.y) as f64;
        let det = dy * (point.x - this_a.x as f64) - dx * (point.y - this_a.y as f64);
        // area of the parallelogramm spanned by the 3 points
        let length = jmath::sqrt(dx * dx + dy * dy);
        det / length
    }

    /// Returns true if the two lines define the same set of points, but may have opposite
    /// directions.
    #[inline]
    pub fn overlaps(&self, other: &Line) -> bool {
        self.side_of(&other.a) == Side::Collinear && self.side_of(&other.b) == Side::Collinear
    }

    /// Returns the line defining the same set of points, but with opposite direction.
    #[inline]
    pub fn opposite(&self) -> Line {
        Line::new(self.b.clone(), self.a.clone())
    }

    /// Returns the intersection point of the 2 lines. If the lines are parallel
    /// `result.is_infinite()` will be true. Only implemented for lines of IntPoints.
    pub fn intersection(&self, other: &Line) -> Point {
        let this_a = self.a.as_int();
        let this_b = self.b.as_int();
        let other_a = other.a.as_int();
        let other_b = other.b.as_int();
        // b.differenceBy(a) (as computed by IntPoint.differenceBy(Point))
        let delta1 = this_a.difference_by_int(&this_b).negate();
        let delta2 = other_a.difference_by_int(&other_b).negate();
        // Separate handling for orthogonal and 45 degree lines for better performance
        if delta1.x == 0 {
            // this line is vertical
            if delta2.y == 0 {
                // other line is horizontal
                return Point::Int(IntPoint::new(this_a.x, other_a.y));
            }
            if delta2.x == delta2.y {
                // other line is right diagonal
                let this_x = this_a.x;
                return Point::Int(IntPoint::new(
                    this_x,
                    other_a.y.wrapping_add(this_x).wrapping_sub(other_a.x),
                ));
            }
            if delta2.x == -delta2.y {
                // other line is left diagonal
                let this_x = this_a.x;
                return Point::Int(IntPoint::new(
                    this_x,
                    other_a.y.wrapping_add(other_a.x).wrapping_sub(this_x),
                ));
            }
        } else if delta1.y == 0 {
            // this line is horizontal
            if delta2.x == 0 {
                // other line is vertical
                return Point::Int(IntPoint::new(other_a.x, this_a.y));
            }
            if delta2.x == delta2.y {
                // other line is right diagonal
                let this_y = this_a.y;
                return Point::Int(IntPoint::new(
                    other_a.x.wrapping_add(this_y).wrapping_sub(other_a.y),
                    this_y,
                ));
            }
            if delta2.x == -delta2.y {
                // other line is left diagonal
                let this_y = this_a.y;
                return Point::Int(IntPoint::new(
                    other_a.x.wrapping_add(other_a.y).wrapping_sub(this_y),
                    this_y,
                ));
            }
        } else if delta1.x == delta1.y {
            // this line is right diagonal
            if delta2.x == 0 {
                // other line is vertical
                let other_x = other_a.x;
                return Point::Int(IntPoint::new(
                    other_x,
                    this_a.y.wrapping_add(other_x).wrapping_sub(this_a.x),
                ));
            }
            if delta2.y == 0 {
                // other line is horizontal
                let other_y = other_a.y;
                return Point::Int(IntPoint::new(
                    this_a.x.wrapping_add(other_y).wrapping_sub(this_a.y),
                    other_y,
                ));
            }
        } else if delta1.x == -delta1.y {
            // this line is left diagonal
            if delta2.x == 0 {
                // other line is vertical
                let other_x = other_a.x;
                return Point::Int(IntPoint::new(
                    other_x,
                    this_a.y.wrapping_add(this_a.x).wrapping_sub(other_x),
                ));
            }
            if delta2.y == 0 {
                // other line is horizontal
                let other_y = other_a.y;
                return Point::Int(IntPoint::new(
                    this_a.x.wrapping_add(this_a.y).wrapping_sub(other_y),
                    other_y,
                ));
            }
        }

        // General case. All products fit into i128 exactly (|coordinates| < 2^31, so
        // |det1|, |det2| < 2^63 and |is_x|, |is_y| < 2^96); Java uses BigInteger here.
        let det1 = this_a.determinant(&this_b) as i128;
        let det2 = other_a.determinant(&other_b) as i128;
        let mut det = delta2.determinant(&delta1) as i128;
        let mut is_x = det1 * delta2.x as i128 - det2 * delta1.x as i128;
        let mut is_y = det1 * delta2.y as i128 - det2 * delta1.y as i128;
        let signum = det.signum();
        if signum != 0 {
            if signum < 0 {
                det = -det;
                is_x = -is_x;
                is_y = -is_y;
            }
            if is_x % det == 0 && is_y % det == 0 {
                is_x /= det;
                is_y /= det;
                // Java: Math.abs(isX.doubleValue()) <= CRIT_INT; exact for these magnitudes
                if (is_x as f64).abs() <= CRIT_INT as f64 && (is_y as f64).abs() <= CRIT_INT as f64
                {
                    return Point::Int(IntPoint::new(is_x as i32, is_y as i32));
                }
                det = 1;
            }
        }
        Point::Rational(Box::new(RationalPoint::new(
            BigInt::from(is_x),
            BigInt::from(is_y),
            BigInt::from(det),
        )))
    }

    /// Reference implementation of the general case of [`Line::intersection`] with BigInt
    /// arithmetic exactly as in Java (used to cross check the i128 fast path in tests).
    #[doc(hidden)]
    pub fn intersection_bigint_reference(&self, other: &Line) -> Point {
        let this_a = self.a.as_int();
        let this_b = self.b.as_int();
        let other_a = other.a.as_int();
        let other_b = other.b.as_int();
        let delta1 = this_a.difference_by_int(&this_b).negate();
        let delta2 = other_a.difference_by_int(&other_b).negate();
        let det1 = big(this_a.determinant(&this_b));
        let det2 = big(other_a.determinant(&other_b));
        let mut det = big(delta2.determinant(&delta1));
        let mut is_x = &det1 * big(delta2.x as i64) - &det2 * big(delta1.x as i64);
        let mut is_y = &det1 * big(delta2.y as i64) - &det2 * big(delta1.y as i64);
        let signum = bigint_signum(&det);
        if signum != 0 {
            if signum < 0 {
                det = -det;
                is_x = -is_x;
                is_y = -is_y;
            }
            if bigint_signum(&(&is_x % &det)) == 0 && bigint_signum(&(&is_y % &det)) == 0 {
                is_x = &is_x / &det;
                is_y = &is_y / &det;
                if bigint_double_value(&is_x).abs() <= CRIT_INT as f64
                    && bigint_double_value(&is_y).abs() <= CRIT_INT as f64
                {
                    return Point::Int(IntPoint::new(
                        bigint_int_value(&is_x),
                        bigint_int_value(&is_y),
                    ));
                }
                det = BigInt::one();
            }
        }
        Point::Rational(Box::new(RationalPoint::new(is_x, is_y, det)))
    }

    /// Returns an approximation of the intersection of the 2 lines by a FloatPoint. If the lines
    /// are parallel the result coordinates will be Integer.MAX_VALUE. Only implemented for lines
    /// consisting of IntPoints.
    #[inline]
    pub fn intersection_approx(&self, other: &Line) -> FloatPoint {
        let this_a = self.a.as_int();
        let this_b = self.b.as_int();
        let other_a = other.a.as_int();
        let other_b = other.b.as_int();
        let d1x = this_b.x.wrapping_sub(this_a.x) as f64;
        let d1y = this_b.y.wrapping_sub(this_a.y) as f64;
        let d2x = other_b.x.wrapping_sub(other_a.x) as f64;
        let d2y = other_b.y.wrapping_sub(other_a.y) as f64;
        let det1 = this_a.x as f64 * this_b.y as f64 - this_a.y as f64 * this_b.x as f64;
        let det2 = other_a.x as f64 * other_b.y as f64 - other_a.y as f64 * other_b.x as f64;
        let det = d2x * d1y - d2y * d1x;
        if det == 0.0 {
            FloatPoint::new(INT_MAX_F64, INT_MAX_F64)
        } else {
            FloatPoint::new(
                (d2x * det1 - d1x * det2) / det,
                (d2y * det1 - d1y * det2) / det,
            )
        }
    }

    /// Returns the perpendicular projection of point onto this line.
    pub fn perpendicular_projection(&self, point: &Point) -> Point {
        point.perpendicular_projection(self)
    }

    /// Translates the line perpendicular by dist. If dist > 0, the line is translated to the
    /// left, otherwise to the right. Only implemented for IntPoint lines.
    pub fn translate(&self, dist: f64) -> Line {
        let ai = self.a.as_int();
        let dir = self.direction();
        let v = dir.get_vector().as_int();
        let vxvx = v.x as f64 * v.x as f64;
        let vyvy = v.y as f64 * v.y as f64;
        let length = jmath::sqrt(vxvx + vyvy);
        let new_a = if vxvx <= vyvy {
            // translate along the x axis
            let rel_x = math_round_i32((dist * length) / v.y as f64);
            IntPoint::new(ai.x.wrapping_sub(rel_x), ai.y)
        } else {
            // translate along the y axis
            let rel_y = math_round_i32((dist * length) / v.x as f64);
            IntPoint::new(ai.x, ai.y.wrapping_add(rel_y))
        };
        Line::get_instance(Point::Int(new_a), &dir)
    }

    /// Translates the line by vector.
    pub fn translate_by(&self, vector: &Vector) -> Line {
        if *vector == Vector::ZERO {
            return self.clone();
        }
        let new_a = self.a.translate_by(vector);
        let new_b = self.b.translate_by(vector);
        Line::new(new_a, new_b)
    }

    /// Returns true if the line is axis-parallel.
    pub fn is_orthogonal(&self) -> bool {
        self.direction().is_orthogonal()
    }

    /// Returns true if this line is diagonal.
    pub fn is_diagonal(&self) -> bool {
        self.direction().is_diagonal()
    }

    /// Returns true if the direction of this line is a multiple of 45 degrees.
    pub fn is_multiple_of_45_degree(&self) -> bool {
        self.direction().is_multiple_of_45_degree()
    }

    /// Checks if this line and other are parallel.
    pub fn is_parallel(&self, other: &Line) -> bool {
        self.direction().side_of(&other.direction()) == Side::Collinear
    }

    /// Checks if this line and other are perpendicular.
    pub fn is_perpendicular(&self, other: &Line) -> bool {
        let v1 = self.direction().get_vector();
        let v2 = other.direction().get_vector();
        v1.projection(&v2) == Signum::Zero
    }

    /// Returns true if this and other define the same line (possibly with opposite direction).
    pub fn is_equal_or_opposite(&self, other: &Line) -> bool {
        self.side_of(&other.a) == Side::Collinear && self.side_of(&other.b) == Side::Collinear
    }

    /// Calculates the cosine of the angle between this line and other.
    pub fn cos_angle(&self, other: &Line) -> f64 {
        let v1 = self.b.difference_by(&self.a);
        let v2 = other.b.difference_by(&other.a);
        v1.cos_angle(&v2)
    }

    /// A line l1 is bigger than a line l2, if the direction of l1 is bigger than the direction of
    /// l2 (Java `Comparable<Line>`). Only for lines consisting of IntPoints.
    #[inline]
    pub fn compare_to(&self, other: &Line) -> i32 {
        let this_a = self.a.as_int();
        let this_b = self.b.as_int();
        let other_a = other.a.as_int();
        let other_b = other.b.as_int();
        let dx1 = this_b.x.wrapping_sub(this_a.x);
        let dy1 = this_b.y.wrapping_sub(this_a.y);
        let dx2 = other_b.x.wrapping_sub(other_a.x);
        let dy2 = other_b.y.wrapping_sub(other_a.y);
        if dy1 > 0 {
            if dy2 < 0 {
                return -1;
            }
            if dy2 == 0 {
                if dx2 > 0 {
                    return 1;
                }
                return -1;
            }
        } else if dy1 < 0 {
            if dy2 >= 0 {
                return 1;
            }
        } else {
            // dy1 == 0
            if dx1 > 0 {
                if dy2 != 0 || dx2 < 0 {
                    return -1;
                }
                return 0;
            }
            // dx1 < 0
            if dy2 > 0 || dy2 == 0 && dx2 > 0 {
                return 1;
            }
            if dy2 < 0 {
                return -1;
            }
            return 0;
        }
        // now this direction and other are located in the same open horizontal half plane
        let determinant = dx2 as f64 * dy1 as f64 - dy2 as f64 * dx1 as f64;
        Signum::as_int(determinant)
    }

    /// `compare_to` as `Ordering`. Note: to reproduce Java `Arrays.sort(Line[])` exactly (the
    /// comparator is inconsistent for degenerate lines) use [`crate::java_sort::sort_by`] with
    /// [`Line::compare_to`] instead of a Rust sort with this ordering.
    #[inline]
    pub fn cmp_java(&self, other: &Line) -> std::cmp::Ordering {
        self.compare_to(other).cmp(&0)
    }

    /// Approximation of the function value of this line at x, if the line is not vertical.
    pub fn function_value_approx(&self, x: f64) -> f64 {
        let p1 = self.a.to_float();
        let p2 = self.b.to_float();
        let dx = p2.x - p1.x;
        if dx == 0.0 {
            log::warn!("function_value_approx: line is vertical");
            return 0.0;
        }
        let dy = p2.y - p1.y;
        let det = p1.x * p2.y - p2.x * p1.y;
        (dy * x - det) / dx
    }

    /// Approximation of the function value in y of this line at y, if not horizontal.
    pub fn function_in_y_value_approx(&self, y: f64) -> f64 {
        let p1 = self.a.to_float();
        let p2 = self.b.to_float();
        let dy = p2.y - p1.y;
        if dy == 0.0 {
            log::warn!("function_in_y_value_approx: line is horizontal");
            return 0.0;
        }
        let dx = p2.x - p1.x;
        let det = p1.x * p2.y - p2.x * p1.y;
        (dx * y + det) / dy
    }

    /// Calculates the direction from from_point to the nearest point on this line. Returns None,
    /// if from_point is contained in this line.
    pub fn perpendicular_direction(&self, from_point: &Point) -> Option<Direction> {
        let line_side = self.side_of(from_point);
        if line_side == Side::Collinear {
            return None;
        }
        let dir1 = self.direction().turn_45_degree(2);
        let dir2 = self.direction().turn_45_degree(6);

        let check_point1 = from_point.translate_by(&dir1.get_vector());
        if self.side_of(&check_point1) != line_side {
            return Some(dir1);
        }
        let check_point2 = from_point.translate_by(&dir2.get_vector());
        if self.side_of(&check_point2) != line_side {
            return Some(dir2);
        }
        let nearest_line_point = from_point.to_float().projection_approx(self);
        if nearest_line_point.distance_square(&check_point1.to_float())
            <= nearest_line_point.distance_square(&check_point2.to_float())
        {
            Some(dir1)
        } else {
            Some(dir2)
        }
    }

    /// Turns this line by factor times 90 degree around pole.
    pub fn turn_90_degree(&self, factor: i32, pole: &IntPoint) -> Line {
        let pole = Point::Int(*pole);
        let new_a = self.a.turn_90_degree(factor, &pole);
        let new_b = self.b.turn_90_degree(factor, &pole);
        Line::new(new_a, new_b)
    }

    /// Mirrors this line at the vertical line through pole.
    pub fn mirror_vertical(&self, pole: &IntPoint) -> Line {
        let pole = Point::Int(*pole);
        let new_a = self.b.mirror_vertical(&pole);
        let new_b = self.a.mirror_vertical(&pole);
        Line::new(new_a, new_b)
    }

    /// Mirrors this line at the horizontal line through pole.
    pub fn mirror_horizontal(&self, pole: &IntPoint) -> Line {
        let pole = Point::Int(*pole);
        let new_a = self.b.mirror_horizontal(&pole);
        let new_b = self.a.mirror_horizontal(&pole);
        Line::new(new_a, new_b)
    }

    /// Returns the Euclidean length of this line (Java float; the squares are computed with int
    /// overflow like in Java).
    pub fn length(&self) -> f32 {
        let ipa = self.a.as_int();
        let ipb = self.b.as_int();
        let dx = ipb.x.wrapping_sub(ipa.x);
        let dy = ipb.y.wrapping_sub(ipa.y);
        let sq = dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy));
        jmath::sqrt(sq as f64) as f32
    }

    /// FloatLine through the approximated points.
    pub fn to_float_line(&self) -> FloatLine {
        FloatLine::new(self.a.to_float(), self.b.to_float())
    }

    /// Direction vector `b - a` for IntPoint lines.
    #[inline]
    pub fn int_delta(&self) -> IntVector {
        let a = self.a.as_int();
        let b = self.b.as_int();
        IntVector::new(b.x.wrapping_sub(a.x), b.y.wrapping_sub(a.y))
    }
}

impl PartialEq for Line {
    /// Java `Line.equals`.
    fn eq(&self, other: &Line) -> bool {
        if self.side_of(&other.a) != Side::Collinear {
            return false;
        }
        if self.side_of(&other.b) != Side::Collinear {
            return false;
        }
        let dir1 = self.b.difference_by(&self.a);
        let dir2 = other.b.difference_by(&other.a);
        dir1.projection(&dir2) == Signum::Positive
    }
}
