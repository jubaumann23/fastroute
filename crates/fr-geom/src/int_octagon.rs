//! Port of `IntOctagon.java`: octagons with integer coordinates and 45-degree border lines.
//!
//! Coordinate arithmetic uses wrapping i32 operations to reproduce Java int overflow on the
//! saturated coordinates of unbounded shapes.

use crate::circle::Circle;
use crate::float_point::FloatPoint;
use crate::fortyfive_degree_direction::FortyfiveDegreeDirection;
use crate::int_box::IntBox;
use crate::int_point::IntPoint;
use crate::java_compat::math_round_i32;
use crate::limits::{CRIT_INT, SQRT2};
use crate::line::Line;
use crate::point::Point;
use crate::regular_tile_shape::RegularTileShape;
use crate::shape_bounding_directions::ShapeBoundingDirections;
use crate::side::Side;
use crate::simplex::Simplex;
use crate::tile_shape::{TileShape, TileShapeImpl};
use crate::vector::Vector;

#[inline(always)]
fn add(a: i32, b: i32) -> i32 {
    a.wrapping_add(b)
}

#[inline(always)]
fn sub(a: i32, b: i32) -> i32 {
    a.wrapping_sub(b)
}

/// An octagon given by 8 border values (see field docs).
///
/// Java's `isEmpty()` is the reference comparison `this == EMPTY`; that identity is modelled by
/// a hidden flag which is only set on [`IntOctagon::EMPTY`] (and on every copy of it). An octagon
/// constructed with inverted bounds via [`IntOctagon::new`] is therefore *not* empty, exactly as
/// in Java.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct IntOctagon {
    /// X-coordinate of the left vertical border.
    pub left_x: i32,
    /// Y-coordinate of the bottom horizontal border.
    pub bottom_y: i32,
    /// X-coordinate of the right vertical border.
    pub right_x: i32,
    /// Y-coordinate of the top horizontal border.
    pub top_y: i32,
    /// X-axis intersection of the upper-left diagonal border (-45 degree).
    pub upper_left_diagonal_x: i32,
    /// X-axis intersection of the lower-right diagonal border (-45 degree).
    pub lower_right_diagonal_x: i32,
    /// X-axis intersection of the lower-left diagonal border (+45 degree).
    pub lower_left_diagonal_x: i32,
    /// X-axis intersection of the upper-right diagonal border (+45 degree).
    pub upper_right_diagonal_x: i32,
    is_empty_instance: bool,
}

impl IntOctagon {
    /// Reusable instance of an empty octagon.
    pub const EMPTY: IntOctagon = IntOctagon {
        left_x: CRIT_INT,
        bottom_y: CRIT_INT,
        right_x: -CRIT_INT,
        top_y: -CRIT_INT,
        upper_left_diagonal_x: CRIT_INT,
        lower_right_diagonal_x: -CRIT_INT,
        lower_left_diagonal_x: CRIT_INT,
        upper_right_diagonal_x: -CRIT_INT,
        is_empty_instance: true,
    };

    /// Creates an IntOctagon from 8 integer boundary values (Java constructor order: lx, ly, rx,
    /// uy, ulx, lrx, llx, urx).
    #[allow(clippy::too_many_arguments)]
    #[inline]
    pub const fn new(
        left_x: i32,
        bottom_y: i32,
        right_x: i32,
        top_y: i32,
        upper_left_diagonal_x: i32,
        lower_right_diagonal_x: i32,
        lower_left_diagonal_x: i32,
        upper_right_diagonal_x: i32,
    ) -> IntOctagon {
        IntOctagon {
            left_x,
            bottom_y,
            right_x,
            top_y,
            upper_left_diagonal_x,
            lower_right_diagonal_x,
            lower_left_diagonal_x,
            upper_right_diagonal_x,
            is_empty_instance: false,
        }
    }

    /// Java `this == EMPTY`.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.is_empty_instance
    }

    #[inline]
    pub fn is_int_octagon(&self) -> bool {
        true
    }

    #[inline]
    pub fn is_bounded(&self) -> bool {
        true
    }

    #[inline]
    pub fn corner_is_bounded(&self, _no: i32) -> bool {
        true
    }

    #[inline]
    pub fn bounding_box(&self) -> IntBox {
        IntBox::new(self.left_x, self.bottom_y, self.right_x, self.top_y)
    }

    #[inline]
    pub fn bounding_octagon(&self) -> IntOctagon {
        *self
    }

    #[inline]
    pub fn bounding_tile(&self) -> IntOctagon {
        *self
    }

    pub fn dimension(&self) -> i32 {
        if self.is_empty_instance {
            return -1;
        }
        if self.right_x > self.left_x
            && self.top_y > self.bottom_y
            && self.lower_right_diagonal_x > self.upper_left_diagonal_x
            && self.upper_right_diagonal_x > self.lower_left_diagonal_x
        {
            2
        } else if self.right_x == self.left_x && self.top_y == self.bottom_y {
            0
        } else {
            1
        }
    }

    /// Returns the no-th corner (0..=7). Panics otherwise (IllegalArgumentException).
    pub fn corner(&self, no: i32) -> IntPoint {
        IntPoint::new(self.corner_x(no), self.corner_y(no))
    }

    /// Returns a stable identifier for this octagon.
    pub fn get_id(&self) -> i32 {
        let mut result = self.left_x;
        result = add(result.wrapping_mul(31), self.right_x);
        result = add(result.wrapping_mul(31), self.bottom_y);
        result = add(result.wrapping_mul(31), self.top_y);
        result = add(result.wrapping_mul(31), self.lower_left_diagonal_x);
        result = add(result.wrapping_mul(31), self.upper_right_diagonal_x);
        result = add(result.wrapping_mul(31), self.upper_left_diagonal_x);
        add(result.wrapping_mul(31), self.lower_right_diagonal_x)
    }

    /// y coordinate of corner no, without allocating an IntPoint.
    pub fn corner_y(&self, no: i32) -> i32 {
        match no {
            0 | 1 => self.bottom_y,
            2 => sub(self.right_x, self.lower_right_diagonal_x),
            3 => sub(self.upper_right_diagonal_x, self.right_x),
            4 | 5 => self.top_y,
            6 => sub(self.left_x, self.upper_left_diagonal_x),
            7 => sub(self.lower_left_diagonal_x, self.left_x),
            _ => panic!("IntOctagon.corner: no out of range"),
        }
    }

    /// x coordinate of corner no, without allocating an IntPoint.
    pub fn corner_x(&self, no: i32) -> i32 {
        match no {
            0 => sub(self.lower_left_diagonal_x, self.bottom_y),
            1 => add(self.lower_right_diagonal_x, self.bottom_y),
            2 | 3 => self.right_x,
            4 => sub(self.upper_right_diagonal_x, self.top_y),
            5 => add(self.upper_left_diagonal_x, self.top_y),
            6 | 7 => self.left_x,
            _ => panic!("IntOctagon.corner: no out of range"),
        }
    }

    pub fn area(&self) -> f64 {
        let (lx, ly, rx, uy) = (self.left_x, self.bottom_y, self.right_x, self.top_y);
        let (ulx, lrx, llx, urx) = (
            self.upper_left_diagonal_x,
            self.lower_right_diagonal_x,
            self.lower_left_diagonal_x,
            self.upper_right_diagonal_x,
        );
        // half of |x0 (y1 - y7) + x1 (y2 - y0) + ... + x7 (y0 - y6)|
        let mut result = sub(llx, ly) as f64 * add(sub(ly, llx), lx) as f64;
        result += add(lrx, ly) as f64 * sub(sub(rx, lrx), ly) as f64;
        result += rx as f64 * add(add(sub(sub(urx, 2i32.wrapping_mul(rx)), ly), uy), lrx) as f64;
        result += sub(urx, uy) as f64 * add(sub(uy, urx), rx) as f64;
        result += add(ulx, uy) as f64 * sub(sub(lx, ulx), uy) as f64;
        result += lx as f64 * add(add(sub(sub(llx, 2i32.wrapping_mul(lx)), uy), ly), ulx) as f64;
        0.5 * result.abs()
    }

    #[inline]
    pub fn border_line_count(&self) -> i32 {
        8
    }

    /// Returns the no-th border line (0..=7). Panics otherwise (IllegalArgumentException).
    pub fn border_line(&self, no: i32) -> Line {
        match no {
            0 => Line::new_ints(0, self.bottom_y, 1, self.bottom_y),
            1 => Line::new_ints(
                self.lower_right_diagonal_x,
                0,
                add(self.lower_right_diagonal_x, 1),
                1,
            ),
            2 => Line::new_ints(self.right_x, 0, self.right_x, 1),
            3 => Line::new_ints(
                self.upper_right_diagonal_x,
                0,
                sub(self.upper_right_diagonal_x, 1),
                1,
            ),
            4 => Line::new_ints(0, self.top_y, -1, self.top_y),
            5 => Line::new_ints(
                self.upper_left_diagonal_x,
                0,
                sub(self.upper_left_diagonal_x, 1),
                -1,
            ),
            6 => Line::new_ints(self.left_x, 0, self.left_x, -1),
            7 => Line::new_ints(
                self.lower_left_diagonal_x,
                0,
                add(self.lower_left_diagonal_x, 1),
                -1,
            ),
            _ => panic!("IntOctagon.borderLine: no out of range"),
        }
    }

    /// Only implemented for IntVectors (Java casts to IntVector).
    pub fn translate_by(&self, rel_coor: &Vector) -> IntOctagon {
        if *rel_coor == Vector::ZERO {
            return *self;
        }
        let v = rel_coor.as_int();
        IntOctagon::new(
            add(self.left_x, v.x),
            add(self.bottom_y, v.y),
            add(self.right_x, v.x),
            add(self.top_y, v.y),
            sub(add(self.upper_left_diagonal_x, v.x), v.y),
            sub(add(self.lower_right_diagonal_x, v.x), v.y),
            add(add(self.lower_left_diagonal_x, v.x), v.y),
            add(add(self.upper_right_diagonal_x, v.x), v.y),
        )
    }

    pub fn max_width(&self) -> f64 {
        let width1 = sub(self.right_x, self.left_x).max(sub(self.top_y, self.bottom_y)) as f64;
        let width2 = sub(self.upper_right_diagonal_x, self.lower_left_diagonal_x)
            .max(sub(self.lower_right_diagonal_x, self.upper_left_diagonal_x))
            as f64;
        crate::float_point::java_max(width1, width2 / SQRT2)
    }

    pub fn min_width(&self) -> f64 {
        let width1 = sub(self.right_x, self.left_x).min(sub(self.top_y, self.bottom_y)) as f64;
        let width2 = sub(self.upper_right_diagonal_x, self.lower_left_diagonal_x)
            .min(sub(self.lower_right_diagonal_x, self.upper_left_diagonal_x))
            as f64;
        crate::float_point::java_min(width1, width2 / SQRT2)
    }

    pub fn offset(&self, distance: f64) -> IntOctagon {
        let width = math_round_i32(distance);
        if width == 0 {
            return *self;
        }
        let dia_width = math_round_i32(SQRT2 * distance);
        let result = IntOctagon::new(
            sub(self.left_x, width),
            sub(self.bottom_y, width),
            add(self.right_x, width),
            add(self.top_y, width),
            sub(self.upper_left_diagonal_x, dia_width),
            add(self.lower_right_diagonal_x, dia_width),
            sub(self.lower_left_diagonal_x, dia_width),
            add(self.upper_right_diagonal_x, dia_width),
        );
        result.normalize()
    }

    pub fn enlarge(&self, offset: f64) -> IntOctagon {
        self.offset(offset)
    }

    /// Java `contains(RegularTileShape)`.
    pub fn contains_regular(&self, other: &RegularTileShape) -> bool {
        match other {
            RegularTileShape::IntBox(b) => b.is_contained_in_int_octagon(self),
            RegularTileShape::IntOctagon(o) => o.is_contained_in_int_octagon(self),
        }
    }

    /// Java `contains(IntOctagon)` resolves to `contains(RegularTileShape)`.
    pub fn contains_int_octagon(&self, other: &IntOctagon) -> bool {
        other.is_contained_in_int_octagon(self)
    }

    /// Returns true if point is contained in this octagon (may be inexact near the border).
    pub fn contains_float(&self, point: &FloatPoint) -> bool {
        if self.left_x as f64 > point.x
            || self.bottom_y as f64 > point.y
            || (self.right_x as f64) < point.x
            || (self.top_y as f64) < point.y
        {
            return false;
        }
        let tmp1 = point.x - point.y;
        let tmp2 = point.x + point.y;
        self.upper_left_diagonal_x as f64 <= tmp1
            && self.lower_right_diagonal_x as f64 >= tmp1
            && self.lower_left_diagonal_x as f64 <= tmp2
            && self.upper_right_diagonal_x as f64 >= tmp2
    }

    pub fn union(&self, other: &RegularTileShape) -> RegularTileShape {
        // other.union(this)
        match other {
            RegularTileShape::IntBox(b) => RegularTileShape::IntOctagon(b.union_int_octagon(self)),
            RegularTileShape::IntOctagon(o) => {
                RegularTileShape::IntOctagon(o.union_int_octagon(self))
            }
        }
    }

    pub fn union_int_octagon(&self, other: &IntOctagon) -> IntOctagon {
        IntOctagon::new(
            self.left_x.min(other.left_x),
            self.bottom_y.min(other.bottom_y),
            self.right_x.max(other.right_x),
            self.top_y.max(other.top_y),
            self.upper_left_diagonal_x.min(other.upper_left_diagonal_x),
            self.lower_right_diagonal_x
                .max(other.lower_right_diagonal_x),
            self.lower_left_diagonal_x.min(other.lower_left_diagonal_x),
            self.upper_right_diagonal_x
                .max(other.upper_right_diagonal_x),
        )
    }

    pub fn union_int_box(&self, other: &IntBox) -> IntOctagon {
        self.union_int_octagon(&other.to_int_octagon())
    }

    pub fn intersection(&self, other: &TileShape) -> TileShape {
        TileShape::IntOctagon(*self).intersection(other)
    }

    /// Package private `intersection(Simplex)`.
    pub fn intersection_simplex(&self, other: &Simplex) -> Simplex {
        other.intersection_int_octagon(self)
    }

    pub fn intersection_int_octagon(&self, other: &IntOctagon) -> IntOctagon {
        let result = IntOctagon::new(
            self.left_x.max(other.left_x),
            self.bottom_y.max(other.bottom_y),
            self.right_x.min(other.right_x),
            self.top_y.min(other.top_y),
            self.upper_left_diagonal_x.max(other.upper_left_diagonal_x),
            self.lower_right_diagonal_x
                .min(other.lower_right_diagonal_x),
            self.lower_left_diagonal_x.max(other.lower_left_diagonal_x),
            self.upper_right_diagonal_x
                .min(other.upper_right_diagonal_x),
        );
        result.normalize()
    }

    /// Package private `intersection(IntBox)`.
    pub fn intersection_int_box(&self, other: &IntBox) -> IntOctagon {
        self.intersection_int_octagon(&other.to_int_octagon())
    }

    /// Returns an equivalent octagon with all redundant bounds tightened (EMPTY if empty).
    pub fn normalize(&self) -> IntOctagon {
        if self.left_x > self.right_x
            || self.bottom_y > self.top_y
            || self.lower_left_diagonal_x > self.upper_right_diagonal_x
            || self.upper_left_diagonal_x > self.lower_right_diagonal_x
        {
            return IntOctagon::EMPTY;
        }
        let mut new_lx = self.left_x;
        let mut new_rx = self.right_x;
        let mut new_ly = self.bottom_y;
        let mut new_uy = self.top_y;
        let mut new_llx = self.lower_left_diagonal_x;
        let mut new_ulx = self.upper_left_diagonal_x;
        let mut new_lrx = self.lower_right_diagonal_x;
        let mut new_urx = self.upper_right_diagonal_x;

        if new_lx < sub(new_llx, new_uy) {
            // the point (lx, uy) is below the lower left border line: move lx
            new_lx = sub(new_llx, new_uy);
        }
        if new_lx < add(new_ulx, new_ly) {
            // the point (lx, ly) is above the upper left border line: move lx
            new_lx = add(new_ulx, new_ly);
        }
        if new_rx > sub(new_urx, new_ly) {
            // the point (rx, ly) is above the upper right border line: move rx
            new_rx = sub(new_urx, new_ly);
        }
        if new_rx > add(new_lrx, new_uy) {
            // the point (rx, uy) is below the lower right border line: move rx
            new_rx = add(new_lrx, new_uy);
        }
        if new_ly < sub(new_lx, new_lrx) {
            // the point (lx, ly) is below the lower right border line: move ly
            new_ly = sub(new_lx, new_lrx);
        }
        if new_ly < sub(new_llx, new_rx) {
            // the point (rx, ly) is below the lower left border line: move ly
            new_ly = sub(new_llx, new_rx);
        }
        if new_uy > sub(new_urx, new_lx) {
            // the point (lx, uy) is above the upper right border line: move uy
            new_uy = sub(new_urx, new_lx);
        }
        if new_uy > sub(new_rx, new_ulx) {
            // the point (rx, uy) is above the upper left border line: move uy
            new_uy = sub(new_rx, new_ulx);
        }
        if sub(new_llx, new_lx) < new_ly {
            // the point (lx, ly) is above the lower left border line: move that line
            new_llx = add(new_lx, new_ly);
        }
        if sub(new_rx, new_lrx) < new_ly {
            // the point (rx, ly) is above the lower right border line: move that line
            new_lrx = sub(new_rx, new_ly);
        }
        if sub(new_urx, new_rx) > new_uy {
            // the point (rx, uy) is below the upper right border line: move that line
            new_urx = add(new_uy, new_rx);
        }
        if sub(new_lx, new_ulx) > new_uy {
            // the point (lx, uy) is below the upper left border line: move that line
            new_ulx = sub(new_lx, new_uy);
        }

        let diag_upper_y = (sub(new_urx, new_ulx) as f64 / 2.0).ceil() as i32;
        if new_uy > diag_upper_y {
            // the intersection of the upper right and the upper left border line is below uy
            new_uy = diag_upper_y;
        }
        let diag_lower_y = (sub(new_llx, new_lrx) as f64 / 2.0).floor() as i32;
        if new_ly < diag_lower_y {
            // the intersection of the lower right and the lower left border line is above ly
            new_ly = diag_lower_y;
        }
        let diag_right_x = (add(new_urx, new_lrx) as f64 / 2.0).ceil() as i32;
        if new_rx > diag_right_x {
            // the intersection of the upper right and the lower right border line is left of rx
            new_rx = diag_right_x;
        }
        let diag_left_x = (add(new_llx, new_ulx) as f64 / 2.0).floor() as i32;
        if new_lx < diag_left_x {
            // the intersection of the lower left and the upper left border line is right of lx
            new_lx = diag_left_x;
        }
        if new_lx > new_rx || new_ly > new_uy || new_llx > new_urx || new_ulx > new_lrx {
            return IntOctagon::EMPTY;
        }
        if self.left_x == new_lx
            && self.right_x == new_rx
            && self.bottom_y == new_ly
            && self.top_y == new_uy
            && self.lower_left_diagonal_x == new_llx
            && self.upper_left_diagonal_x == new_ulx
            && self.lower_right_diagonal_x == new_lrx
            && self.upper_right_diagonal_x == new_urx
        {
            return *self;
        }
        IntOctagon::new(
            new_lx, new_ly, new_rx, new_uy, new_ulx, new_lrx, new_llx, new_urx,
        )
    }

    /// Checks, if this IntOctagon is normalized.
    pub fn is_normalized(&self) -> bool {
        let on = self.normalize();
        self.left_x == on.left_x
            && self.bottom_y == on.bottom_y
            && self.right_x == on.right_x
            && self.top_y == on.top_y
            && self.lower_left_diagonal_x == on.lower_left_diagonal_x
            && self.lower_right_diagonal_x == on.lower_right_diagonal_x
            && self.upper_left_diagonal_x == on.upper_left_diagonal_x
            && self.upper_right_diagonal_x == on.upper_right_diagonal_x
    }

    /// Converts to a Simplex. (Java memorizes the result in the octagon; it is recomputed here,
    /// which yields the same value.)
    pub fn to_simplex(&self) -> Simplex {
        if self.is_empty() {
            return Simplex::empty();
        }
        // Small per thread memo (the conversion is pure; sharing the result is like Java's
        // memorized simplex).
        const SLOTS: usize = 1024;
        thread_local! {
            static MEMO: std::cell::RefCell<Vec<Option<(IntOctagon, Simplex)>>> = std::cell::RefCell::new(vec![None; SLOTS]);
        }
        let h = [
            self.left_x,
            self.bottom_y,
            self.right_x,
            self.top_y,
            self.upper_left_diagonal_x,
            self.lower_right_diagonal_x,
            self.lower_left_diagonal_x,
            self.upper_right_diagonal_x,
        ]
        .iter()
        .fold(0u64, |h, &v| (h ^ v as u32 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let slot = (h >> 40) as usize % SLOTS;
        if let Some(hit) = MEMO.with(|m| match &m.borrow()[slot] {
            Some((o, simplex)) if o == self => Some(simplex.clone()),
            _ => None,
        }) {
            return hit;
        }
        // Java: new Simplex(lines).removeRedundantLines()
        let result = Simplex::from_lines_without_redundant(std::array::from_fn::<Line, 8, _>(|i| self.border_line(i as i32)));
        MEMO.with(|m| m.borrow_mut()[slot] = Some((*self, result.clone())));
        result
    }

    pub fn bounding_shape(&self, dirs: &ShapeBoundingDirections) -> RegularTileShape {
        dirs.bounds_int_octagon(self)
    }

    /// Calculates the side of the point (x, y) of the border line with index border_line_no.
    pub fn side_of_border_line(&self, x: i32, y: i32, border_line_no: i32) -> Side {
        let tmp = match border_line_no {
            0 => sub(self.bottom_y, y),
            1 => sub(sub(x, y), self.lower_right_diagonal_x),
            2 => sub(x, self.right_x),
            3 => sub(add(x, y), self.upper_right_diagonal_x),
            4 => sub(y, self.top_y),
            5 => sub(add(self.upper_left_diagonal_x, y), x),
            6 => sub(self.left_x, x),
            7 => sub(sub(self.lower_left_diagonal_x, x), y),
            _ => {
                log::warn!("IntOctagon.sideOfBorderLine: borderLineNo out of range");
                0
            }
        };
        if tmp < 0 {
            Side::OnTheLeft
        } else if tmp > 0 {
            Side::OnTheRight
        } else {
            Side::Collinear
        }
    }

    /// Checks if this normalized octagon is contained in box.
    pub fn is_contained_in(&self, b: &IntBox) -> bool {
        self.left_x >= b.ll.x
            && self.bottom_y >= b.ll.y
            && self.right_x <= b.ur.x
            && self.top_y <= b.ur.y
    }

    pub fn is_contained_in_int_octagon(&self, other: &IntOctagon) -> bool {
        self.left_x >= other.left_x
            && self.bottom_y >= other.bottom_y
            && self.right_x <= other.right_x
            && self.top_y <= other.top_y
            && self.lower_left_diagonal_x >= other.lower_left_diagonal_x
            && self.upper_left_diagonal_x >= other.upper_left_diagonal_x
            && self.lower_right_diagonal_x <= other.lower_right_diagonal_x
            && self.upper_right_diagonal_x <= other.upper_right_diagonal_x
    }

    pub fn intersects_int_box(&self, other: &IntBox) -> bool {
        self.intersects_int_octagon(&other.to_int_octagon())
    }

    /// Checks if two normalized octagons intersect.
    pub fn intersects_int_octagon(&self, other: &IntOctagon) -> bool {
        let is_lx = other.left_x.max(self.left_x);
        let is_rx = other.right_x.min(self.right_x);
        if is_lx > is_rx {
            return false;
        }
        let is_ly = other.bottom_y.max(self.bottom_y);
        let is_uy = other.top_y.min(self.top_y);
        if is_ly > is_uy {
            return false;
        }
        let is_llx = other.lower_left_diagonal_x.max(self.lower_left_diagonal_x);
        let is_urx = other
            .upper_right_diagonal_x
            .min(self.upper_right_diagonal_x);
        if is_llx > is_urx {
            return false;
        }
        let is_ulx = other.upper_left_diagonal_x.max(self.upper_left_diagonal_x);
        let is_lrx = other
            .lower_right_diagonal_x
            .min(self.lower_right_diagonal_x);
        is_ulx <= is_lrx
    }

    pub fn intersects_simplex(&self, other: &Simplex) -> bool {
        other.intersects_int_octagon(self)
    }

    pub fn intersects_circle(&self, other: &Circle) -> bool {
        other.intersects_int_octagon(self)
    }

    /// Returns true, if this octagon intersects with other and the intersection is
    /// 2-dimensional.
    pub fn overlaps(&self, other: &IntOctagon) -> bool {
        let is_lx = other.left_x.max(self.left_x);
        let is_rx = other.right_x.min(self.right_x);
        if is_lx >= is_rx {
            return false;
        }
        let is_ly = other.bottom_y.max(self.bottom_y);
        let is_uy = other.top_y.min(self.top_y);
        if is_ly >= is_uy {
            return false;
        }
        let is_llx = other.lower_left_diagonal_x.max(self.lower_left_diagonal_x);
        let is_urx = other
            .upper_right_diagonal_x
            .min(self.upper_right_diagonal_x);
        if is_llx >= is_urx {
            return false;
        }
        let is_ulx = other.upper_left_diagonal_x.max(self.upper_left_diagonal_x);
        let is_lrx = other
            .lower_right_diagonal_x
            .min(self.lower_right_diagonal_x);
        is_ulx < is_lrx
    }

    /// Computes the x value of the left boundary of this octagon at y.
    pub fn left_x_value(&self, y: i32) -> i32 {
        let result = self.left_x.max(add(self.upper_left_diagonal_x, y));
        result.max(sub(self.lower_left_diagonal_x, y))
    }

    /// Computes the x value of the right boundary of this octagon at y.
    pub fn right_x_value(&self, y: i32) -> i32 {
        let result = self.right_x.min(sub(self.upper_right_diagonal_x, y));
        result.min(add(self.lower_right_diagonal_x, y))
    }

    /// Computes the y value of the lower boundary of this octagon at x.
    pub fn lower_y_value(&self, x: i32) -> i32 {
        let result = self.bottom_y.max(sub(self.lower_left_diagonal_x, x));
        result.max(sub(x, self.lower_right_diagonal_x))
    }

    /// Computes the y value of the upper boundary of this octagon at x.
    pub fn upper_y_value(&self, x: i32) -> i32 {
        let result = self.top_y.min(sub(x, self.upper_left_diagonal_x));
        result.min(sub(self.upper_right_diagonal_x, x))
    }

    pub fn compare(&self, other: &RegularTileShape, edge_index: i32) -> Side {
        let result = match other {
            RegularTileShape::IntBox(b) => b.compare_int_octagon(self, edge_index),
            RegularTileShape::IntOctagon(o) => o.compare_int_octagon(self, edge_index),
        };
        result.negate()
    }

    pub fn compare_int_octagon(&self, other: &IntOctagon, edge_index: i32) -> Side {
        // left_if_greater: a > b -> ON_THE_LEFT
        let cmp = |a: i32, b: i32, left_if_greater: bool| -> Side {
            if a > b {
                if left_if_greater {
                    Side::OnTheLeft
                } else {
                    Side::OnTheRight
                }
            } else if a < b {
                if left_if_greater {
                    Side::OnTheRight
                } else {
                    Side::OnTheLeft
                }
            } else {
                Side::Collinear
            }
        };
        match edge_index {
            0 => cmp(self.bottom_y, other.bottom_y, true),
            1 => cmp(
                self.lower_right_diagonal_x,
                other.lower_right_diagonal_x,
                false,
            ),
            2 => cmp(self.right_x, other.right_x, false),
            3 => cmp(
                self.upper_right_diagonal_x,
                other.upper_right_diagonal_x,
                false,
            ),
            4 => cmp(self.top_y, other.top_y, false),
            5 => cmp(
                self.upper_left_diagonal_x,
                other.upper_left_diagonal_x,
                true,
            ),
            6 => cmp(self.left_x, other.left_x, true),
            7 => cmp(
                self.lower_left_diagonal_x,
                other.lower_left_diagonal_x,
                true,
            ),
            _ => panic!("IntBox.compare: edgeIndex out of range"),
        }
    }

    pub fn compare_int_box(&self, other: &IntBox, edge_index: i32) -> Side {
        self.compare_int_octagon(&other.to_int_octagon(), edge_index)
    }

    pub fn border_line_index(&self, _line: &Line) -> i32 {
        log::warn!("edge_index_of_line not yet implemented for octagons");
        -1
    }

    /// Calculates the border point of this octagon from point into the 45 degree direction dir.
    /// If this border point is not an IntPoint, the nearest outside IntPoint is returned.
    pub fn border_point(&self, point: &IntPoint, dir: FortyfiveDegreeDirection) -> IntPoint {
        let (px, py) = (point.x, point.y);
        let (result_x, result_y);
        match dir {
            FortyfiveDegreeDirection::Right => {
                let mut rx = self.right_x.min(sub(self.upper_right_diagonal_x, py));
                rx = rx.min(add(self.lower_right_diagonal_x, py));
                result_x = rx;
                result_y = py;
            }
            FortyfiveDegreeDirection::Left => {
                let mut rx = self.left_x.max(add(self.upper_left_diagonal_x, py));
                rx = rx.max(sub(self.lower_left_diagonal_x, py));
                result_x = rx;
                result_y = py;
            }
            FortyfiveDegreeDirection::Up => {
                result_x = px;
                let mut ry = self.top_y.min(sub(px, self.upper_left_diagonal_x));
                ry = ry.min(sub(self.upper_right_diagonal_x, px));
                result_y = ry;
            }
            FortyfiveDegreeDirection::Down => {
                result_x = px;
                let mut ry = self.bottom_y.max(sub(self.lower_left_diagonal_x, px));
                ry = ry.max(sub(px, self.lower_right_diagonal_x));
                result_y = ry;
            }
            FortyfiveDegreeDirection::Right45 => {
                let mut rx =
                    (0.5 * add(sub(px, py), self.upper_right_diagonal_x) as f64).ceil() as i32;
                rx = rx.min(self.right_x);
                rx = rx.min(add(sub(px, py), self.top_y));
                result_x = rx;
                result_y = add(sub(py, px), rx);
            }
            FortyfiveDegreeDirection::Up45 => {
                let mut rx =
                    (0.5 * add(add(px, py), self.upper_left_diagonal_x) as f64).floor() as i32;
                rx = rx.max(self.left_x);
                rx = rx.max(sub(add(px, py), self.top_y));
                result_x = rx;
                result_y = sub(add(py, px), rx);
            }
            FortyfiveDegreeDirection::Left45 => {
                let mut rx =
                    (0.5 * add(sub(px, py), self.lower_left_diagonal_x) as f64).floor() as i32;
                rx = rx.max(self.left_x);
                rx = rx.max(add(sub(px, py), self.bottom_y));
                result_x = rx;
                result_y = add(sub(py, px), rx);
            }
            FortyfiveDegreeDirection::Down45 => {
                let mut rx =
                    (0.5 * add(add(px, py), self.lower_right_diagonal_x) as f64).ceil() as i32;
                rx = rx.min(self.right_x);
                rx = rx.min(sub(add(px, py), self.bottom_y));
                result_x = rx;
                result_y = sub(add(py, px), rx);
            }
        }
        IntPoint::new(result_x, result_y)
    }

    /// Calculates the sorted max_result_points nearest points on the border of this octagon in
    /// the 45-degree directions. point is assumed to be located in the interior of this octagon.
    pub fn nearest_border_projections(
        &self,
        point: &IntPoint,
        max_result_points: i32,
    ) -> Vec<IntPoint> {
        if !TileShapeImpl::contains(self, &Point::Int(*point)) || max_result_points <= 0 {
            return Vec::new();
        }
        let n = max_result_points.min(8) as usize;
        let mut result: Vec<Option<IntPoint>> = vec![None; n];
        let mut min_dist = vec![f64::MAX; n];
        let inside_point = point.to_float();
        for current_direction in FortyfiveDegreeDirection::VALUES {
            let current_border_point = self.border_point(point, current_direction);
            let current_distance = inside_point.distance_square(&current_border_point.to_float());
            for i in 0..n {
                if current_distance < min_dist[i] {
                    let mut k = n - 1;
                    while k > i {
                        min_dist[k] = min_dist[k - 1];
                        result[k] = result[k - 1];
                        k -= 1;
                    }
                    min_dist[i] = current_distance;
                    result[i] = Some(current_border_point);
                    break;
                }
            }
        }
        result
            .into_iter()
            .map(|p| p.expect("8 border points always fill the result"))
            .collect()
    }

    /// Package private `borderLineSideOf(FloatPoint, int, double)`.
    pub fn border_line_side_of(&self, point: &FloatPoint, line_index: i32, tolerance: f64) -> Side {
        let side3 = |right: bool, left: bool| -> Side {
            if right {
                Side::OnTheRight
            } else if left {
                Side::OnTheLeft
            } else {
                Side::Collinear
            }
        };
        match line_index {
            0 => side3(
                point.y > self.bottom_y as f64 + tolerance,
                point.y < self.bottom_y as f64 - tolerance,
            ),
            2 => side3(
                point.x < self.right_x as f64 - tolerance,
                point.x > self.right_x as f64 + tolerance,
            ),
            4 => side3(
                point.y < self.top_y as f64 - tolerance,
                point.y > self.top_y as f64 + tolerance,
            ),
            6 => side3(
                point.x > self.left_x as f64 + tolerance,
                point.x < self.left_x as f64 - tolerance,
            ),
            1 => {
                let tmp = point.y - point.x + self.lower_right_diagonal_x as f64;
                side3(tmp > tolerance, tmp < -tolerance)
            }
            3 => {
                let tmp = point.x + point.y - self.upper_right_diagonal_x as f64;
                side3(tmp < -tolerance, tmp > tolerance)
            }
            5 => {
                let tmp = point.y - point.x + self.upper_left_diagonal_x as f64;
                side3(tmp < -tolerance, tmp > tolerance)
            }
            7 => {
                let tmp = point.x + point.y - self.lower_left_diagonal_x as f64;
                side3(tmp > tolerance, tmp < -tolerance)
            }
            _ => {
                log::warn!("IntOctagon.borderLineSideOf: lineIndex out of range");
                Side::Collinear
            }
        }
    }

    /// Checks, if this octagon can be converted to an IntBox.
    pub fn is_int_box(&self) -> bool {
        if self.lower_left_diagonal_x != add(self.left_x, self.bottom_y) {
            return false;
        }
        if self.lower_right_diagonal_x != sub(self.right_x, self.bottom_y) {
            return false;
        }
        if self.upper_right_diagonal_x != add(self.right_x, self.top_y) {
            return false;
        }
        self.upper_left_diagonal_x == sub(self.left_x, self.top_y)
    }

    pub fn simplify(&self) -> TileShape {
        if self.is_int_box() {
            return TileShape::IntBox(self.bounding_box());
        }
        TileShape::IntOctagon(*self)
    }

    pub fn cutout(&self, shape: &TileShape) -> Option<Vec<TileShape>> {
        shape.cutout_from_int_octagon(self)
    }

    /// Package private `cutoutFrom(IntBox d)`: divide d minus this octagon into 8 convex pieces,
    /// from which 4 have cut off a corner.
    pub fn cutout_from_int_box(&self, d: &IntBox) -> Vec<IntOctagon> {
        let c = self.intersection_int_box(d);
        if self.is_empty() || c.dimension() < self.dimension() {
            // there is only an overlap at the border
            return vec![d.to_int_octagon()];
        }
        let mut boxes = [
            // left box
            IntBox::new(
                d.ll.x,
                sub(c.lower_left_diagonal_x, c.left_x),
                c.left_x,
                sub(c.left_x, c.upper_left_diagonal_x),
            ),
            // right box
            IntBox::new(
                c.right_x,
                sub(c.right_x, c.lower_right_diagonal_x),
                d.ur.x,
                sub(c.upper_right_diagonal_x, c.right_x),
            ),
            // lower box
            IntBox::new(
                sub(c.lower_left_diagonal_x, c.bottom_y),
                d.ll.y,
                add(c.lower_right_diagonal_x, c.bottom_y),
                c.bottom_y,
            ),
            // upper box
            IntBox::new(
                add(c.upper_left_diagonal_x, c.top_y),
                c.top_y,
                sub(c.upper_right_diagonal_x, c.top_y),
                d.ur.y,
            ),
        ];
        let mut octagons = [
            // upper left octagon
            IntOctagon::new(
                d.ll.x,
                boxes[0].ur.y,
                boxes[3].ll.x,
                d.ur.y,
                -CRIT_INT,
                c.upper_left_diagonal_x,
                -CRIT_INT,
                CRIT_INT,
            )
            .normalize(),
            // lower left octagon
            IntOctagon::new(
                d.ll.x,
                d.ll.y,
                boxes[2].ll.x,
                boxes[0].ll.y,
                -CRIT_INT,
                CRIT_INT,
                -CRIT_INT,
                c.lower_left_diagonal_x,
            )
            .normalize(),
            // lower right octagon
            IntOctagon::new(
                boxes[2].ur.x,
                d.ll.y,
                d.ur.x,
                boxes[1].ll.y,
                c.lower_right_diagonal_x,
                CRIT_INT,
                -CRIT_INT,
                CRIT_INT,
            )
            .normalize(),
            // upper right octagon
            IntOctagon::new(
                boxes[3].ur.x,
                boxes[1].ur.y,
                d.ur.x,
                d.ur.y,
                -CRIT_INT,
                CRIT_INT,
                c.upper_right_diagonal_x,
                CRIT_INT,
            )
            .normalize(),
        ];

        // optimise the result to minimum cumulative circumference
        let with = |o: &IntOctagon, lx: i32, ly: i32, rx: i32, uy: i32| {
            IntOctagon::new(
                lx,
                ly,
                rx,
                uy,
                o.upper_left_diagonal_x,
                o.lower_right_diagonal_x,
                o.lower_left_diagonal_x,
                o.upper_right_diagonal_x,
            )
            .normalize()
        };

        let b = boxes[0];
        let o = octagons[0];
        if sub(b.ur.x, b.ll.x) > sub(o.top_y, o.bottom_y) {
            // switch the horizontal upper left divide line to vertical
            boxes[0] = IntBox::new(b.ll.x, b.ll.y, b.ur.x, o.top_y);
            octagons[0] = with(&o, b.ur.x, o.bottom_y, o.right_x, o.top_y);
        }
        let b = boxes[3];
        let o = octagons[0];
        if sub(b.ur.y, b.ll.y) > sub(o.right_x, o.left_x) {
            // switch the vertical upper left divide line to horizontal
            boxes[3] = IntBox::new(o.left_x, b.ll.y, b.ur.x, b.ur.y);
            octagons[0] = with(&o, o.left_x, o.bottom_y, o.right_x, b.ll.y);
        }
        let b = boxes[3];
        let o = octagons[3];
        if sub(b.ur.y, b.ll.y) > sub(o.right_x, o.left_x) {
            // switch the vertical upper right divide line to horizontal
            boxes[3] = IntBox::new(b.ll.x, b.ll.y, o.right_x, b.ur.y);
            octagons[3] = with(&o, o.left_x, o.bottom_y, o.right_x, o.top_y);
        }
        let b = boxes[1];
        let o = octagons[3];
        if sub(b.ur.x, b.ll.x) > sub(o.top_y, o.bottom_y) {
            // switch the horizontal upper right divide line to vertical
            boxes[1] = IntBox::new(b.ll.x, b.ll.y, b.ur.x, o.top_y);
            octagons[3] = with(&o, o.left_x, o.bottom_y, b.ll.x, o.top_y);
        }
        let b = boxes[1];
        let o = octagons[2];
        if sub(b.ur.x, b.ll.x) > sub(o.top_y, o.bottom_y) {
            // switch the horizontal lower right divide line to vertical
            boxes[1] = IntBox::new(b.ll.x, o.bottom_y, b.ur.x, b.ur.y);
            octagons[2] = with(&o, o.left_x, o.bottom_y, b.ll.x, o.top_y);
        }
        let b = boxes[2];
        let o = octagons[2];
        if sub(b.ur.y, b.ll.y) > sub(o.right_x, o.left_x) {
            // switch the vertical lower right divide line to horizontal
            boxes[2] = IntBox::new(b.ll.x, b.ll.y, o.right_x, b.ur.y);
            octagons[2] = with(&o, o.left_x, b.ur.y, o.right_x, o.top_y);
        }
        let b = boxes[2];
        let o = octagons[1];
        if sub(b.ur.y, b.ll.y) > sub(o.right_x, o.left_x) {
            // switch the vertical lower left divide line to horizontal
            boxes[2] = IntBox::new(o.left_x, b.ll.y, b.ur.x, b.ur.y);
            octagons[1] = with(&o, o.left_x, b.ur.y, o.right_x, o.top_y);
        }
        let b = boxes[0];
        let o = octagons[1];
        if sub(b.ur.x, b.ll.x) > sub(o.top_y, o.bottom_y) {
            // switch the horizontal lower left divide line to vertical
            boxes[0] = IntBox::new(b.ll.x, o.bottom_y, b.ur.x, b.ur.y);
            octagons[1] = with(&o, b.ur.x, o.bottom_y, o.right_x, o.top_y);
        }

        let mut result: Vec<IntOctagon> = boxes.iter().map(|b| b.to_int_octagon()).collect();
        result.extend_from_slice(&octagons);
        result
    }

    /// Package private `cutoutFrom(IntOctagon d)`: divide d minus this octagon into 8 convex
    /// pieces without sharp angles.
    pub fn cutout_from_int_octagon(&self, d: &IntOctagon) -> Vec<IntOctagon> {
        let c = self.intersection_int_octagon(d);
        if self.is_empty() || c.dimension() < self.dimension() {
            // there is only an overlap at the border
            return vec![*d];
        }
        let (dulx, dlrx, dllx, durx) = (
            d.upper_left_diagonal_x,
            d.lower_right_diagonal_x,
            d.lower_left_diagonal_x,
            d.upper_right_diagonal_x,
        );
        let mut result = [IntOctagon::EMPTY; 8];

        let mut tmp = sub(c.lower_left_diagonal_x, c.left_x);
        result[0] = IntOctagon::new(
            d.left_x,
            tmp,
            c.left_x,
            sub(c.left_x, c.upper_left_diagonal_x),
            dulx,
            dlrx,
            dllx,
            durx,
        );

        let mut tmp2 = sub(c.lower_left_diagonal_x, c.bottom_y);
        result[1] = IntOctagon::new(
            d.left_x,
            d.bottom_y,
            tmp2,
            tmp,
            dulx,
            dlrx,
            dllx,
            c.lower_left_diagonal_x,
        );

        tmp = add(c.lower_right_diagonal_x, c.bottom_y);
        result[2] = IntOctagon::new(tmp2, d.bottom_y, tmp, c.bottom_y, dulx, dlrx, dllx, durx);

        tmp2 = sub(c.right_x, c.lower_right_diagonal_x);
        result[3] = IntOctagon::new(
            tmp,
            d.bottom_y,
            d.right_x,
            tmp2,
            c.lower_right_diagonal_x,
            dlrx,
            dllx,
            durx,
        );

        tmp = sub(c.upper_right_diagonal_x, c.right_x);
        result[4] = IntOctagon::new(c.right_x, tmp2, d.right_x, tmp, dulx, dlrx, dllx, durx);

        tmp2 = sub(c.upper_right_diagonal_x, c.top_y);
        result[5] = IntOctagon::new(
            tmp2,
            tmp,
            d.right_x,
            d.top_y,
            dulx,
            dlrx,
            c.upper_right_diagonal_x,
            durx,
        );

        tmp = add(c.upper_left_diagonal_x, c.top_y);
        result[6] = IntOctagon::new(tmp, c.top_y, tmp2, d.top_y, dulx, dlrx, dllx, durx);

        tmp2 = sub(c.left_x, c.upper_left_diagonal_x);
        result[7] = IntOctagon::new(
            d.left_x,
            tmp2,
            tmp,
            d.top_y,
            dulx,
            c.upper_left_diagonal_x,
            dllx,
            durx,
        );

        for r in result.iter_mut() {
            *r = r.normalize();
        }

        let mut curr1 = result[0];
        let mut curr2 = result[7];
        if !(curr1.is_empty() || curr2.is_empty())
            && sub(curr1.right_x, curr1.left_x_value(curr1.top_y))
                > sub(curr2.upper_y_value(curr1.right_x), curr2.bottom_y)
        {
            // switch the horizontal upper left divide line to vertical
            curr1 = IntOctagon::new(
                curr1.left_x.min(curr2.left_x),
                curr1.bottom_y,
                curr1.right_x,
                curr2.top_y,
                curr2.upper_left_diagonal_x,
                curr1.lower_right_diagonal_x,
                curr1.lower_left_diagonal_x,
                curr2.upper_right_diagonal_x,
            );
            curr2 = IntOctagon::new(
                curr1.right_x,
                curr2.bottom_y,
                curr2.right_x,
                curr2.top_y,
                curr2.upper_left_diagonal_x,
                curr2.lower_right_diagonal_x,
                curr2.lower_left_diagonal_x,
                curr2.upper_right_diagonal_x,
            );
            result[0] = curr1.normalize();
            result[7] = curr2.normalize();
        }
        curr1 = result[7];
        curr2 = result[6];
        if !(curr1.is_empty() || curr2.is_empty())
            && sub(curr2.upper_y_value(curr1.right_x), curr2.bottom_y)
                > sub(curr1.right_x, curr1.left_x_value(curr2.bottom_y))
        {
            // switch the vertical upper left divide line to horizontal
            curr2 = IntOctagon::new(
                curr1.left_x,
                curr2.bottom_y,
                curr2.right_x,
                curr2.top_y.max(curr1.top_y),
                curr1.upper_left_diagonal_x,
                curr2.lower_right_diagonal_x,
                curr1.lower_left_diagonal_x,
                curr2.upper_right_diagonal_x,
            );
            curr1 = IntOctagon::new(
                curr1.left_x,
                curr1.bottom_y,
                curr1.right_x,
                curr2.bottom_y,
                curr1.upper_left_diagonal_x,
                curr1.lower_right_diagonal_x,
                curr1.lower_left_diagonal_x,
                curr1.upper_right_diagonal_x,
            );
            result[7] = curr1.normalize();
            result[6] = curr2.normalize();
        }
        curr1 = result[6];
        curr2 = result[5];
        if !(curr1.is_empty() || curr2.is_empty())
            && sub(curr2.upper_y_value(curr1.right_x), curr1.bottom_y)
                > sub(curr2.right_x_value(curr1.bottom_y), curr2.left_x)
        {
            // switch the vertical upper right divide line to horizontal
            curr1 = IntOctagon::new(
                curr1.left_x,
                curr1.bottom_y,
                curr2.right_x,
                curr2.top_y.max(curr1.top_y),
                curr1.upper_left_diagonal_x,
                curr2.lower_right_diagonal_x,
                curr1.lower_left_diagonal_x,
                curr2.upper_right_diagonal_x,
            );
            curr2 = IntOctagon::new(
                curr2.left_x,
                curr2.bottom_y,
                curr2.right_x,
                curr1.bottom_y,
                curr2.upper_left_diagonal_x,
                curr2.lower_right_diagonal_x,
                curr2.lower_left_diagonal_x,
                curr2.upper_right_diagonal_x,
            );
            result[6] = curr1.normalize();
            result[5] = curr2.normalize();
        }
        curr1 = result[5];
        curr2 = result[4];
        if !(curr1.is_empty() || curr2.is_empty())
            && sub(curr2.right_x_value(curr2.top_y), curr2.left_x)
                > sub(curr1.upper_y_value(curr2.left_x), curr2.top_y)
        {
            // switch the horizontal upper right divide line to vertical
            curr2 = IntOctagon::new(
                curr2.left_x,
                curr2.bottom_y,
                curr2.right_x.max(curr1.right_x),
                curr1.top_y,
                curr1.upper_left_diagonal_x,
                curr2.lower_right_diagonal_x,
                curr2.lower_left_diagonal_x,
                curr1.upper_right_diagonal_x,
            );
            curr1 = IntOctagon::new(
                curr1.left_x,
                curr1.bottom_y,
                curr2.left_x,
                curr1.top_y,
                curr1.upper_left_diagonal_x,
                curr1.lower_right_diagonal_x,
                curr1.lower_left_diagonal_x,
                curr1.upper_right_diagonal_x,
            );
            result[5] = curr1.normalize();
            result[4] = curr2.normalize();
        }
        curr1 = result[4];
        curr2 = result[3];
        if !(curr1.is_empty() || curr2.is_empty())
            && sub(curr1.right_x_value(curr1.bottom_y), curr1.left_x)
                > sub(curr1.bottom_y, curr2.lower_y_value(curr1.left_x))
        {
            // switch the horizontal lower right divide line to vertical
            curr1 = IntOctagon::new(
                curr1.left_x,
                curr2.bottom_y,
                curr2.right_x.max(curr1.right_x),
                curr1.top_y,
                curr1.upper_left_diagonal_x,
                curr2.lower_right_diagonal_x,
                curr2.lower_left_diagonal_x,
                curr1.upper_right_diagonal_x,
            );
            curr2 = IntOctagon::new(
                curr2.left_x,
                curr2.bottom_y,
                curr1.left_x,
                curr2.top_y,
                curr2.upper_left_diagonal_x,
                curr2.lower_right_diagonal_x,
                curr2.lower_left_diagonal_x,
                curr2.upper_right_diagonal_x,
            );
            result[4] = curr1.normalize();
            result[3] = curr2.normalize();
        }
        curr1 = result[3];
        curr2 = result[2];
        if !(curr1.is_empty() || curr2.is_empty())
            && sub(curr2.top_y, curr2.lower_y_value(curr2.right_x))
                > sub(curr1.right_x_value(curr2.top_y), curr2.right_x)
        {
            // switch the vertical lower right divide line to horizontal
            curr2 = IntOctagon::new(
                curr2.left_x,
                curr1.bottom_y.min(curr2.bottom_y),
                curr1.right_x,
                curr2.top_y,
                curr2.upper_left_diagonal_x,
                curr1.lower_right_diagonal_x,
                curr2.lower_left_diagonal_x,
                curr1.upper_right_diagonal_x,
            );
            curr1 = IntOctagon::new(
                curr1.left_x,
                curr2.top_y,
                curr1.right_x,
                curr1.top_y,
                curr1.upper_left_diagonal_x,
                curr1.lower_right_diagonal_x,
                curr1.lower_left_diagonal_x,
                curr1.upper_right_diagonal_x,
            );
            result[3] = curr1.normalize();
            result[2] = curr2.normalize();
        }
        curr1 = result[2];
        curr2 = result[1];
        if !(curr1.is_empty() || curr2.is_empty())
            && sub(curr1.top_y, curr1.lower_y_value(curr1.left_x))
                > sub(curr1.left_x, curr2.left_x_value(curr1.top_y))
        {
            // switch the vertical lower left divide line to horizontal
            curr1 = IntOctagon::new(
                curr2.left_x,
                curr1.bottom_y.min(curr2.bottom_y),
                curr1.right_x,
                curr1.top_y,
                curr2.upper_left_diagonal_x,
                curr1.lower_right_diagonal_x,
                curr2.lower_left_diagonal_x,
                curr1.upper_right_diagonal_x,
            );
            curr2 = IntOctagon::new(
                curr2.left_x,
                curr1.top_y,
                curr2.right_x,
                curr2.top_y,
                curr2.upper_left_diagonal_x,
                curr2.lower_right_diagonal_x,
                curr2.lower_left_diagonal_x,
                curr2.upper_right_diagonal_x,
            );
            result[2] = curr1.normalize();
            result[1] = curr2.normalize();
        }
        curr1 = result[1];
        curr2 = result[0];
        if !(curr1.is_empty() || curr2.is_empty())
            && sub(curr2.right_x, curr2.left_x_value(curr2.bottom_y))
                > sub(curr2.bottom_y, curr1.lower_y_value(curr2.right_x))
        {
            // switch the horizontal lower left divide line to vertical
            curr2 = IntOctagon::new(
                curr2.left_x.min(curr1.left_x),
                curr1.bottom_y,
                curr2.right_x,
                curr2.top_y,
                curr2.upper_left_diagonal_x,
                curr1.lower_right_diagonal_x,
                curr1.lower_left_diagonal_x,
                curr2.upper_right_diagonal_x,
            );
            curr1 = IntOctagon::new(
                curr2.right_x,
                curr1.bottom_y,
                curr1.right_x,
                curr1.top_y,
                curr1.upper_left_diagonal_x,
                curr1.lower_right_diagonal_x,
                curr1.lower_left_diagonal_x,
                curr1.upper_right_diagonal_x,
            );
            result[1] = curr1.normalize();
            result[0] = curr2.normalize();
        }
        result.to_vec()
    }

    /// Package private `cutoutFrom(Simplex)`.
    pub fn cutout_from_simplex(&self, simplex: &Simplex) -> Option<Vec<Simplex>> {
        self.to_simplex().cutout_from_simplex(simplex)
    }
}

impl std::fmt::Display for IntOctagon {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "IntOctagon(leftX={}, bottomY={}, rightX={}, topY={}, upperLeftDiagonalX={}, lowerRightDiagonalX={}, lowerLeftDiagonalX={}, upperRightDiagonalX={})",
            self.left_x,
            self.bottom_y,
            self.right_x,
            self.top_y,
            self.upper_left_diagonal_x,
            self.lower_right_diagonal_x,
            self.lower_left_diagonal_x,
            self.upper_right_diagonal_x
        )
    }
}

impl crate::polyline_shape::PolylineShapeImpl for IntOctagon {
    fn border_line_count(&self) -> i32 {
        8
    }
    fn corner(&self, no: i32) -> Point {
        Point::Int(IntOctagon::corner(self, no))
    }
    fn border_line(&self, no: i32) -> Line {
        IntOctagon::border_line(self, no)
    }
    fn corner_is_bounded(&self, _no: i32) -> bool {
        true
    }
    fn is_empty(&self) -> bool {
        IntOctagon::is_empty(self)
    }
    fn is_bounded(&self) -> bool {
        true
    }
    fn dimension(&self) -> i32 {
        IntOctagon::dimension(self)
    }
    fn bounding_box(&self) -> IntBox {
        IntOctagon::bounding_box(self)
    }
    fn corner_approx(&self, no: i32) -> FloatPoint {
        IntOctagon::corner(self, no).to_float()
    }
    fn is_contained_in(&self, b: &IntBox) -> bool {
        IntOctagon::is_contained_in(self, b)
    }
}

impl TileShapeImpl for IntOctagon {
    fn to_tile_shape(&self) -> TileShape {
        TileShape::IntOctagon(*self)
    }
    fn simplify(&self) -> TileShape {
        IntOctagon::simplify(self)
    }
    fn get_id(&self) -> i32 {
        IntOctagon::get_id(self)
    }
    fn is_int_box(&self) -> bool {
        IntOctagon::is_int_box(self)
    }
    fn is_int_octagon(&self) -> bool {
        true
    }
    fn intersection_int_box_tile(&self, other: &IntBox) -> TileShape {
        TileShape::IntOctagon(self.intersection_int_box(other))
    }
    fn intersection_int_octagon_tile(&self, other: &IntOctagon) -> TileShape {
        TileShape::IntOctagon(self.intersection_int_octagon(other))
    }
    fn intersection_simplex_tile(&self, other: &Simplex) -> TileShape {
        TileShape::Simplex(self.intersection_simplex(other))
    }
    fn border_line_index(&self, line: &Line) -> i32 {
        IntOctagon::border_line_index(self, line)
    }
    fn to_simplex(&self) -> Simplex {
        IntOctagon::to_simplex(self)
    }
    fn offset_tile(&self, distance: f64) -> TileShape {
        TileShape::IntOctagon(self.offset(distance))
    }
    fn max_width(&self) -> f64 {
        IntOctagon::max_width(self)
    }
    fn min_width(&self) -> f64 {
        IntOctagon::min_width(self)
    }
    fn translate_by_tile(&self, vector: &Vector) -> TileShape {
        TileShape::IntOctagon(self.translate_by(vector))
    }
    fn cutout_tile(&self, shape: &TileShape) -> Option<Vec<TileShape>> {
        shape.cutout_from_int_octagon(self)
    }
    fn cutout_from_int_box_tile(&self, shape: &IntBox) -> Option<Vec<TileShape>> {
        Some(
            self.cutout_from_int_box(shape)
                .into_iter()
                .map(TileShape::IntOctagon)
                .collect(),
        )
    }
    fn cutout_from_int_octagon_tile(&self, shape: &IntOctagon) -> Option<Vec<TileShape>> {
        Some(
            self.cutout_from_int_octagon(shape)
                .into_iter()
                .map(TileShape::IntOctagon)
                .collect(),
        )
    }
    fn cutout_from_simplex_tile(&self, shape: &Simplex) -> Option<Vec<TileShape>> {
        self.to_simplex()
            .cutout_from_simplex(shape)
            .map(|v| v.into_iter().map(TileShape::Simplex).collect())
    }
    fn bounding_octagon_opt(&self) -> Option<IntOctagon> {
        Some(*self)
    }
    fn enlarge_tile(&self, offset: f64) -> TileShape {
        TileShape::IntOctagon(self.enlarge(offset))
    }
    fn intersects_int_box(&self, other: &IntBox) -> bool {
        IntOctagon::intersects_int_box(self, other)
    }
    fn intersects_int_octagon(&self, other: &IntOctagon) -> bool {
        IntOctagon::intersects_int_octagon(self, other)
    }
    fn intersects_simplex(&self, other: &Simplex) -> bool {
        IntOctagon::intersects_simplex(self, other)
    }
    fn intersects_circle(&self, other: &Circle) -> bool {
        IntOctagon::intersects_circle(self, other)
    }
    fn bounding_shape_tile(&self, dirs: &ShapeBoundingDirections) -> Option<RegularTileShape> {
        Some(self.bounding_shape(dirs))
    }
    fn area(&self) -> f64 {
        IntOctagon::area(self)
    }
    fn contains_float(&self, point: &FloatPoint) -> bool {
        IntOctagon::contains_float(self, point)
    }
}
