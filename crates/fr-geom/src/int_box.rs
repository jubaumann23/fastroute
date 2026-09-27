//! Port of `IntBox.java`: orthogonal rectangles in the plane with integer coordinates.
//!
//! Coordinate arithmetic uses wrapping i32 operations: boxes of unbounded shapes carry
//! saturated `Integer.MAX_VALUE` coordinates and Java silently overflows on them.

use crate::circle::Circle;
use crate::float_point::{java_max, java_min, FloatPoint};
use crate::int_direction::IntDirection;
use crate::int_octagon::IntOctagon;
use crate::int_point::IntPoint;
use crate::java_compat::math_round_i32;
use crate::jmath;
use crate::limits::CRIT_INT;
use crate::line::Line;
use crate::point::Point;
use crate::regular_tile_shape::RegularTileShape;
use crate::shape_bounding_directions::ShapeBoundingDirections;
use crate::side::Side;
use crate::simplex::Simplex;
use crate::tile_shape::TileShape;
use crate::vector::Vector;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct IntBox {
    /// Lower-left corner.
    pub ll: IntPoint,
    /// Upper-right corner.
    pub ur: IntPoint,
}

impl IntBox {
    /// Standard implementation of an empty box.
    pub const EMPTY: IntBox = IntBox {
        ll: IntPoint::new(CRIT_INT, CRIT_INT),
        ur: IntPoint::new(-CRIT_INT, -CRIT_INT),
    };

    /// Creates an IntBox from its lower left and upper right corners.
    #[inline]
    pub const fn from_points(ll: IntPoint, ur: IntPoint) -> IntBox {
        IntBox { ll, ur }
    }

    /// Creates an IntBox from the coordinates of its lower-left and upper-right corners.
    #[inline]
    pub const fn new(
        lower_left_x: i32,
        lower_left_y: i32,
        upper_right_x: i32,
        upper_right_y: i32,
    ) -> IntBox {
        IntBox {
            ll: IntPoint::new(lower_left_x, lower_left_y),
            ur: IntPoint::new(upper_right_x, upper_right_y),
        }
    }

    #[inline]
    pub fn is_int_octagon(&self) -> bool {
        true
    }

    /// Returns true, if the box is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.ll.x > self.ur.x || self.ll.y > self.ur.y
    }

    #[inline]
    pub fn border_line_count(&self) -> i32 {
        4
    }

    /// Returns the horizontal extension of the box.
    #[inline]
    pub fn width(&self) -> i32 {
        self.ur.x.wrapping_sub(self.ll.x)
    }

    /// Returns the vertical extension of the box.
    #[inline]
    pub fn height(&self) -> i32 {
        self.ur.y.wrapping_sub(self.ll.y)
    }

    pub fn max_width(&self) -> f64 {
        self.width().max(self.height()) as f64
    }

    pub fn min_width(&self) -> f64 {
        self.width().min(self.height()) as f64
    }

    pub fn area(&self) -> f64 {
        self.width() as f64 * self.height() as f64
    }

    pub fn circumference(&self) -> f64 {
        (2i32.wrapping_mul(self.width().wrapping_add(self.height()))) as f64
    }

    /// Returns the no-th corner (0..=3). Panics otherwise (Java: IllegalArgumentException).
    pub fn corner(&self, no: i32) -> IntPoint {
        match no {
            0 => self.ll,
            1 => IntPoint::new(self.ur.x, self.ll.y),
            2 => self.ur,
            3 => IntPoint::new(self.ll.x, self.ur.y),
            _ => panic!("IntBox.corner: no out of range"),
        }
    }

    pub fn dimension(&self) -> i32 {
        if self.is_empty() {
            return -1;
        }
        if self.ll == self.ur {
            return 0;
        }
        if self.ur.x == self.ll.x || self.ll.y == self.ur.y {
            return 1;
        }
        2
    }

    /// Checks, if point is located in the interior of this box (Java `containsInside(IntPoint)`).
    #[inline]
    pub fn contains_inside_int_point(&self, point: &IntPoint) -> bool {
        point.x > self.ll.x && point.x < self.ur.x && point.y > self.ll.y && point.y < self.ur.y
    }

    #[inline]
    pub fn is_int_box(&self) -> bool {
        true
    }

    pub fn simplify(&self) -> TileShape {
        TileShape::IntBox(*self)
    }

    /// Calculates the nearest point of this box to from_point (Java `nearestPoint(FloatPoint)`).
    pub fn nearest_point_float(&self, from_point: &FloatPoint) -> FloatPoint {
        let x = if from_point.x <= self.ll.x as f64 {
            self.ll.x as f64
        } else if from_point.x >= self.ur.x as f64 {
            self.ur.x as f64
        } else {
            from_point.x
        };
        let y = if from_point.y <= self.ll.y as f64 {
            self.ll.y as f64
        } else if from_point.y >= self.ur.y as f64 {
            self.ur.y as f64
        } else {
            from_point.y
        };
        FloatPoint::new(x, y)
    }

    /// Calculates the sorted max_result_points nearest points on the border of this box. point is
    /// assumed to be located in the interior of this box. Only implemented for
    /// max_result_points <= 2.
    pub fn nearest_border_projections(
        &self,
        point: &IntPoint,
        max_result_points: i32,
    ) -> Vec<IntPoint> {
        if max_result_points <= 0 {
            return Vec::new();
        }
        let max_result_points = max_result_points.min(2);
        let lower_horizontal_difference = point.x - self.ll.x;
        let upper_horizontal_difference = self.ur.x - point.x;
        let lower_vertical_difference = point.y - self.ll.y;
        let upper_vertical_difference = self.ur.y - point.y;

        let mut min_diff;
        let mut second_min_diff;
        let mut nearest_projection_x;
        let mut nearest_projection_y = point.y;
        let mut second_nearest_projection_x;
        let mut second_nearest_projection_y = point.y;
        if lower_horizontal_difference <= upper_horizontal_difference {
            min_diff = lower_horizontal_difference;
            second_min_diff = upper_horizontal_difference;
            nearest_projection_x = self.ll.x;
            second_nearest_projection_x = self.ur.x;
        } else {
            min_diff = upper_horizontal_difference;
            second_min_diff = lower_horizontal_difference;
            nearest_projection_x = self.ur.x;
            second_nearest_projection_x = self.ll.x;
        }
        if lower_vertical_difference < min_diff {
            second_min_diff = min_diff;
            min_diff = lower_vertical_difference;
            second_nearest_projection_x = nearest_projection_x;
            second_nearest_projection_y = nearest_projection_y;
            nearest_projection_x = point.x;
            nearest_projection_y = self.ll.y;
        } else if lower_vertical_difference < second_min_diff {
            second_min_diff = lower_vertical_difference;
            second_nearest_projection_x = point.x;
            second_nearest_projection_y = self.ll.y;
        }
        if upper_vertical_difference < min_diff {
            second_nearest_projection_x = nearest_projection_x;
            second_nearest_projection_y = nearest_projection_y;
            nearest_projection_x = point.x;
            nearest_projection_y = self.ur.y;
        } else if upper_vertical_difference < second_min_diff {
            second_nearest_projection_x = point.x;
            second_nearest_projection_y = self.ur.y;
        }
        let mut result = vec![IntPoint::new(nearest_projection_x, nearest_projection_y)];
        if max_result_points > 1 {
            result.push(IntPoint::new(
                second_nearest_projection_x,
                second_nearest_projection_y,
            ));
        }
        result
    }

    /// Calculates distance of this box to from_point.
    pub fn distance(&self, from_point: &FloatPoint) -> f64 {
        from_point.distance(&self.nearest_point_float(from_point))
    }

    /// Computes the weighted distance to the box other.
    pub fn weighted_distance(
        &self,
        other: &IntBox,
        horizontal_weight: f64,
        vertical_weight: f64,
    ) -> f64 {
        let max_ll_x = java_max(self.ll.x as f64, other.ll.x as f64);
        let max_ll_y = java_max(self.ll.y as f64, other.ll.y as f64);
        let min_ur_x = java_min(self.ur.x as f64, other.ur.x as f64);
        let min_ur_y = java_min(self.ur.y as f64, other.ur.y as f64);

        if min_ur_x >= max_ll_x {
            java_max(vertical_weight * (max_ll_y - min_ur_y), 0.0)
        } else if min_ur_y >= max_ll_y {
            java_max(horizontal_weight * (max_ll_x - min_ur_x), 0.0)
        } else {
            let mut delta_x = max_ll_x - min_ur_x;
            let mut delta_y = max_ll_y - min_ur_y;
            delta_x *= horizontal_weight;
            delta_y *= vertical_weight;
            jmath::sqrt(delta_x * delta_x + delta_y * delta_y)
        }
    }

    #[inline]
    pub fn bounding_box(&self) -> IntBox {
        *self
    }

    pub fn get_id(&self) -> i32 {
        self.ll
            .get_id()
            .wrapping_mul(31)
            .wrapping_add(self.ur.get_id())
    }

    #[inline]
    pub fn bounding_octagon(&self) -> IntOctagon {
        self.to_int_octagon()
    }

    #[inline]
    pub fn is_bounded(&self) -> bool {
        true
    }

    #[inline]
    pub fn bounding_tile(&self) -> IntBox {
        *self
    }

    #[inline]
    pub fn corner_is_bounded(&self, _no: i32) -> bool {
        true
    }

    /// Calculates the smallest RegularTileShape containing this shape and other.
    pub fn union(&self, other: &RegularTileShape) -> RegularTileShape {
        // other.union(this)
        match other {
            RegularTileShape::IntBox(o) => RegularTileShape::IntBox(o.union_int_box(self)),
            RegularTileShape::IntOctagon(o) => RegularTileShape::IntOctagon(o.union_int_box(self)),
        }
    }

    pub fn union_int_box(&self, other: &IntBox) -> IntBox {
        let lower_left_x = self.ll.x.min(other.ll.x);
        let lower_left_y = self.ll.y.min(other.ll.y);
        let upper_right_x = self.ur.x.max(other.ur.x);
        let upper_right_y = self.ur.y.max(other.ur.y);
        IntBox::new(lower_left_x, lower_left_y, upper_right_x, upper_right_y)
    }

    pub fn union_int_octagon(&self, other: &IntOctagon) -> IntOctagon {
        other.union_int_octagon(&self.to_int_octagon())
    }

    /// Returns the intersection of this box with an IntBox.
    pub fn intersection_int_box(&self, other: &IntBox) -> IntBox {
        if other.ll.x > self.ur.x {
            return IntBox::EMPTY;
        }
        if other.ll.y > self.ur.y {
            return IntBox::EMPTY;
        }
        if self.ll.x > other.ur.x {
            return IntBox::EMPTY;
        }
        if self.ll.y > other.ur.y {
            return IntBox::EMPTY;
        }
        let lower_left_x = self.ll.x.max(other.ll.x);
        let upper_right_x = self.ur.x.min(other.ur.x);
        let lower_left_y = self.ll.y.max(other.ll.y);
        let upper_right_y = self.ur.y.min(other.ur.y);
        IntBox::new(lower_left_x, lower_left_y, upper_right_x, upper_right_y)
    }

    /// Package private `intersection(IntOctagon)`.
    pub fn intersection_int_octagon(&self, other: &IntOctagon) -> IntOctagon {
        other.intersection_int_octagon(&self.to_int_octagon())
    }

    /// Package private `intersection(Simplex)`.
    pub fn intersection_simplex(&self, other: &Simplex) -> Simplex {
        other.intersection_simplex(&self.to_simplex())
    }

    /// Returns the intersection of this box with a TileShape.
    pub fn intersection(&self, other: &TileShape) -> TileShape {
        TileShape::IntBox(*self).intersection(other)
    }

    pub fn intersects_int_box(&self, other: &IntBox) -> bool {
        if other.ll.x > self.ur.x {
            return false;
        }
        if other.ll.y > self.ur.y {
            return false;
        }
        if self.ll.x > other.ur.x {
            return false;
        }
        self.ll.y <= other.ur.y
    }

    pub fn intersects_int_octagon(&self, other: &IntOctagon) -> bool {
        other.intersects_int_octagon(&self.to_int_octagon())
    }

    pub fn intersects_simplex(&self, other: &Simplex) -> bool {
        other.intersects_simplex(&self.to_simplex())
    }

    pub fn intersects_circle(&self, other: &Circle) -> bool {
        other.intersects_int_box(self)
    }

    /// Returns true, if this box intersects with other and the intersection is 2-dimensional.
    pub fn overlaps(&self, other: &IntBox) -> bool {
        if other.ll.x >= self.ur.x {
            return false;
        }
        if other.ll.y >= self.ur.y {
            return false;
        }
        if self.ll.x >= other.ur.x {
            return false;
        }
        self.ll.y < other.ur.y
    }

    /// Java `contains(RegularTileShape)`.
    pub fn contains_regular(&self, other: &RegularTileShape) -> bool {
        other.is_contained_in(self)
    }

    pub fn bounding_shape(&self, dirs: &ShapeBoundingDirections) -> RegularTileShape {
        dirs.bounds_int_box(self)
    }

    /// Enlarges the box by offset. Contrary to offset() the result is an IntOctagon.
    pub fn enlarge(&self, offset: f64) -> IntOctagon {
        self.bounding_octagon().offset(offset)
    }

    /// Only implemented for IntVectors (Java casts the translated corners to IntPoint).
    pub fn translate_by(&self, rel_coor: &Vector) -> IntBox {
        if *rel_coor == Vector::ZERO {
            return *self;
        }
        let new_ll = Point::Int(self.ll).translate_by(rel_coor).as_int();
        let new_ur = Point::Int(self.ur).translate_by(rel_coor).as_int();
        IntBox::from_points(new_ll, new_ur)
    }

    pub fn turn_90_degree(&self, factor: i32, pole: &IntPoint) -> IntBox {
        let pole = Point::Int(*pole);
        let p1 = Point::Int(self.ll).turn_90_degree(factor, &pole).as_int();
        let p2 = Point::Int(self.ur).turn_90_degree(factor, &pole).as_int();
        IntBox::new(
            p1.x.min(p2.x),
            p1.y.min(p2.y),
            p1.x.max(p2.x),
            p1.y.max(p2.y),
        )
    }

    /// Returns the no-th border line (0..=3). Panics otherwise (IllegalArgumentException).
    pub fn border_line(&self, no: i32) -> Line {
        match no {
            0 => Line::new_ints(0, self.ll.y, 1, self.ll.y), // lower boundary line
            1 => Line::new_ints(self.ur.x, 0, self.ur.x, 1), // right boundary line
            2 => Line::new_ints(0, self.ur.y, -1, self.ur.y), // upper boundary line
            3 => Line::new_ints(self.ll.x, 0, self.ll.x, -1), // left boundary line
            _ => panic!("IntBox.borderLine: no out of range"),
        }
    }

    pub fn border_line_index(&self, _line: &Line) -> i32 {
        log::warn!("borderLineIndex not yet implemented for IntBoxes");
        -1
    }

    /// Returns the box offsetted by dist. If dist > 0, the offset is to the outside, else to the
    /// inside.
    pub fn offset(&self, dist: f64) -> IntBox {
        if dist == 0.0 || self.is_empty() {
            return *self;
        }
        let d = math_round_i32(dist);
        IntBox::new(
            self.ll.x.wrapping_sub(d),
            self.ll.y.wrapping_sub(d),
            self.ur.x.wrapping_add(d),
            self.ur.y.wrapping_add(d),
        )
    }

    /// Returns the box, where the horizontal boundary is offsetted by dist.
    pub fn horizontal_offset(&self, dist: f64) -> IntBox {
        if dist == 0.0 || self.is_empty() {
            return *self;
        }
        let d = math_round_i32(dist);
        IntBox::new(
            self.ll.x.wrapping_sub(d),
            self.ll.y,
            self.ur.x.wrapping_add(d),
            self.ur.y,
        )
    }

    /// Returns the box, where the vertical boundary is offsetted by dist.
    pub fn vertical_offset(&self, dist: f64) -> IntBox {
        if dist == 0.0 || self.is_empty() {
            return *self;
        }
        let d = math_round_i32(dist);
        IntBox::new(
            self.ll.x,
            self.ll.y.wrapping_sub(d),
            self.ur.x,
            self.ur.y.wrapping_add(d),
        )
    }

    /// Shrinks the width and height of the box by the input width. The box will not vanish
    /// completely. (Java `shrink(int)`.)
    pub fn shrink_int(&self, width: i32) -> IntBox {
        let (lower_left_x, upper_right_x) = if 2i32.wrapping_mul(width) <= self.width() {
            (self.ll.x.wrapping_add(width), self.ur.x.wrapping_sub(width))
        } else {
            let v = self.ll.x.wrapping_add(self.ur.x) / 2;
            (v, v)
        };
        let (lower_left_y, upper_right_y) = if 2i32.wrapping_mul(width) <= self.height() {
            (self.ll.y.wrapping_add(width), self.ur.y.wrapping_sub(width))
        } else {
            let v = self.ll.y.wrapping_add(self.ur.y) / 2;
            (v, v)
        };
        IntBox::new(lower_left_x, lower_left_y, upper_right_x, upper_right_y)
    }

    /// Compares the edge lines of index edge_index of this shape and other.
    pub fn compare(&self, other: &RegularTileShape, edge_index: i32) -> Side {
        // other.compare(this, edgeIndex).negate()
        let result = match other {
            RegularTileShape::IntBox(o) => o.compare_int_box(self, edge_index),
            RegularTileShape::IntOctagon(o) => o.compare_int_box(self, edge_index),
        };
        result.negate()
    }

    pub fn compare_int_box(&self, other: &IntBox, edge_index: i32) -> Side {
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
            0 => cmp(self.ll.y, other.ll.y, true),  // lower edge line
            1 => cmp(self.ur.x, other.ur.x, false), // right edge line
            2 => cmp(self.ur.y, other.ur.y, false), // upper edge line
            3 => cmp(self.ll.x, other.ll.x, true),  // left edge line
            _ => panic!("IntBox.compare: edgeIndex out of range"),
        }
    }

    pub fn compare_int_octagon(&self, other: &IntOctagon, edge_index: i32) -> Side {
        self.to_int_octagon().compare_int_octagon(other, edge_index)
    }

    /// Returns an IntOctagon defining the same shape.
    #[inline]
    pub fn to_int_octagon(&self) -> IntOctagon {
        IntOctagon::new(
            self.ll.x,
            self.ll.y,
            self.ur.x,
            self.ur.y,
            self.ll.x.wrapping_sub(self.ur.y),
            self.ur.x.wrapping_sub(self.ll.y),
            self.ll.x.wrapping_add(self.ll.y),
            self.ur.x.wrapping_add(self.ur.y),
        )
    }

    /// Returns a Simplex defining the same shape.
    pub fn to_simplex(&self) -> Simplex {
        if self.is_empty() {
            return Simplex::new(Vec::new());
        }
        let lines = vec![
            Line::get_instance(Point::Int(self.ll), &IntDirection::RIGHT.to_direction()),
            Line::get_instance(Point::Int(self.ur), &IntDirection::UP.to_direction()),
            Line::get_instance(Point::Int(self.ur), &IntDirection::LEFT.to_direction()),
            Line::get_instance(Point::Int(self.ll), &IntDirection::DOWN.to_direction()),
        ];
        Simplex::new(lines)
    }

    /// Checks if this box is contained in other.
    pub fn is_contained_in(&self, other: &IntBox) -> bool {
        if self.is_empty() || self == other {
            return true;
        }
        self.ll.x >= other.ll.x
            && self.ll.y >= other.ll.y
            && self.ur.x <= other.ur.x
            && self.ur.y <= other.ur.y
    }

    pub fn is_contained_in_int_octagon(&self, other: &IntOctagon) -> bool {
        other.contains_int_octagon(&self.to_int_octagon())
    }

    /// Return true, if other is contained in the interior of this box.
    pub fn contains_in_interior(&self, other: &IntBox) -> bool {
        if other.is_empty() {
            return true;
        }
        other.ll.x > self.ll.x
            && other.ll.y > self.ll.y
            && other.ur.x < self.ur.x
            && other.ur.y < self.ur.y
    }

    /// Calculates the part of from_box, which has minimal distance to this box.
    pub fn nearest_part(&self, from_box: &IntBox) -> IntBox {
        let ll_x = if from_box.ll.x >= self.ll.x {
            from_box.ll.x
        } else {
            from_box.ur.x.min(self.ll.x)
        };
        let ur_x = if from_box.ur.x <= self.ur.x {
            from_box.ur.x
        } else {
            from_box.ll.x.max(self.ur.x)
        };
        let ll_y = if from_box.ll.y >= self.ll.y {
            from_box.ll.y
        } else {
            from_box.ur.y.min(self.ll.y)
        };
        let ur_y = if from_box.ur.y <= self.ur.y {
            from_box.ur.y
        } else {
            from_box.ll.y.max(self.ur.y)
        };
        IntBox::new(ll_x, ll_y, ur_x, ur_y)
    }

    /// Divides this box into sections with width and height at most max_section_width of about
    /// equal size.
    pub fn divide_into_sections(&self, max_section_width: f64) -> Vec<IntBox> {
        if max_section_width <= 0.0 {
            return Vec::new();
        }
        let length = self.width() as f64;
        let height = self.height() as f64;
        let x_count = (length / max_section_width).ceil() as i32;
        let y_count = (height / max_section_width).ceil() as i32;
        let section_length_x = (length / x_count as f64).ceil() as i32;
        let section_length_y = (height / y_count as f64).ceil() as i32;
        let mut result = Vec::with_capacity((x_count.wrapping_mul(y_count)).max(0) as usize);
        for j in 0..y_count {
            let current_ll_y = self.ll.y.wrapping_add(j.wrapping_mul(section_length_y));
            let current_ur_y = if j == y_count - 1 {
                self.ur.y
            } else {
                current_ll_y.wrapping_add(section_length_y)
            };
            for i in 0..x_count {
                let current_ll_x = self.ll.x.wrapping_add(i.wrapping_mul(section_length_x));
                let current_ur_x = if i == x_count - 1 {
                    self.ur.x
                } else {
                    current_ll_x.wrapping_add(section_length_x)
                };
                result.push(IntBox::new(
                    current_ll_x,
                    current_ll_y,
                    current_ur_x,
                    current_ur_y,
                ));
            }
        }
        result
    }

    /// Cuts shape out of this box and divides the result into convex pieces.
    pub fn cutout(&self, shape: &TileShape) -> Vec<TileShape> {
        let tmp = shape
            .cutout_from_int_box(self)
            .expect("NullPointerException: cutoutFrom returned null");
        tmp.into_iter().map(|s| s.simplify()).collect()
    }

    /// Package private `cutoutFrom(IntBox d)`: d minus this box.
    pub fn cutout_from_int_box(&self, d: &IntBox) -> Vec<IntBox> {
        let c = self.intersection_int_box(d);
        if self.is_empty() || c.dimension() < self.dimension() {
            // there is only an overlap at the border
            return vec![*d];
        }
        let mut result = [
            IntBox::new(d.ll.x, d.ll.y, c.ur.x, c.ll.y),
            IntBox::new(d.ll.x, c.ll.y, c.ll.x, d.ur.y),
            IntBox::new(c.ur.x, d.ll.y, d.ur.x, c.ur.y),
            IntBox::new(c.ll.x, c.ur.y, d.ur.x, d.ur.y),
        ];
        // now the division will be optimised, so that the cumulative circumference is minimal.
        let w = |a: i32, b: i32| a.wrapping_sub(b);
        if w(c.ll.x, d.ll.x) > w(c.ll.y, d.ll.y) {
            // switch left dividing line to lower
            let b = result[0];
            result[0] = IntBox::new(c.ll.x, b.ll.y, b.ur.x, b.ur.y);
            let b = result[1];
            result[1] = IntBox::new(b.ll.x, d.ll.y, b.ur.x, b.ur.y);
        }
        if w(d.ur.y, c.ur.y) > w(c.ll.x, d.ll.x) {
            // switch upper dividing line to the left
            let b = result[1];
            result[1] = IntBox::new(b.ll.x, b.ll.y, b.ur.x, c.ur.y);
            let b = result[3];
            result[3] = IntBox::new(d.ll.x, b.ll.y, b.ur.x, b.ur.y);
        }
        if w(d.ur.x, c.ur.x) > w(d.ur.y, c.ur.y) {
            // switch right dividing line to upper
            let b = result[2];
            result[2] = IntBox::new(b.ll.x, b.ll.y, b.ur.x, d.ur.y);
            let b = result[3];
            result[3] = IntBox::new(b.ll.x, b.ll.y, c.ur.x, b.ur.y);
        }
        if w(c.ll.y, d.ll.y) > w(d.ur.x, c.ur.x) {
            // switch lower dividing line to the left
            let b = result[0];
            result[0] = IntBox::new(b.ll.x, b.ll.y, d.ur.x, b.ur.y);
            let b = result[2];
            result[2] = IntBox::new(b.ll.x, c.ll.y, b.ur.x, b.ur.y);
        }
        result.to_vec()
    }

    /// Package private `cutoutFrom(Simplex)`.
    pub fn cutout_from_simplex(&self, simplex: &Simplex) -> Option<Vec<Simplex>> {
        self.to_simplex().cutout_from_simplex(simplex)
    }

    /// Package private `cutoutFrom(IntOctagon)`.
    pub fn cutout_from_int_octagon(&self, oct: &IntOctagon) -> Vec<IntOctagon> {
        self.to_int_octagon().cutout_from_int_octagon(oct)
    }
}

impl std::fmt::Display for IntBox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "IntBox(ll={}, ur={})", self.ll, self.ur)
    }
}

impl crate::polyline_shape::PolylineShapeImpl for IntBox {
    fn border_line_count(&self) -> i32 {
        4
    }
    fn corner(&self, no: i32) -> Point {
        Point::Int(IntBox::corner(self, no))
    }
    fn border_line(&self, no: i32) -> Line {
        IntBox::border_line(self, no)
    }
    fn corner_is_bounded(&self, _no: i32) -> bool {
        true
    }
    fn is_empty(&self) -> bool {
        IntBox::is_empty(self)
    }
    fn is_bounded(&self) -> bool {
        true
    }
    fn dimension(&self) -> i32 {
        IntBox::dimension(self)
    }
    fn bounding_box(&self) -> IntBox {
        *self
    }
    fn corner_approx(&self, no: i32) -> FloatPoint {
        IntBox::corner(self, no).to_float()
    }
    fn circumference(&self) -> f64 {
        IntBox::circumference(self)
    }
    fn is_contained_in(&self, other: &IntBox) -> bool {
        IntBox::is_contained_in(self, other)
    }
}

impl crate::tile_shape::TileShapeImpl for IntBox {
    fn to_tile_shape(&self) -> TileShape {
        TileShape::IntBox(*self)
    }
    fn simplify(&self) -> TileShape {
        IntBox::simplify(self)
    }
    fn get_id(&self) -> i32 {
        IntBox::get_id(self)
    }
    fn is_int_box(&self) -> bool {
        true
    }
    fn is_int_octagon(&self) -> bool {
        true
    }
    fn intersection_int_box_tile(&self, other: &IntBox) -> TileShape {
        TileShape::IntBox(self.intersection_int_box(other))
    }
    fn intersection_int_octagon_tile(&self, other: &IntOctagon) -> TileShape {
        TileShape::IntOctagon(self.intersection_int_octagon(other))
    }
    fn intersection_simplex_tile(&self, other: &Simplex) -> TileShape {
        TileShape::Simplex(self.intersection_simplex(other))
    }
    fn border_line_index(&self, line: &Line) -> i32 {
        IntBox::border_line_index(self, line)
    }
    fn to_simplex(&self) -> Simplex {
        IntBox::to_simplex(self)
    }
    fn offset_tile(&self, distance: f64) -> TileShape {
        TileShape::IntBox(self.offset(distance))
    }
    fn max_width(&self) -> f64 {
        IntBox::max_width(self)
    }
    fn min_width(&self) -> f64 {
        IntBox::min_width(self)
    }
    fn translate_by_tile(&self, vector: &Vector) -> TileShape {
        TileShape::IntBox(self.translate_by(vector))
    }
    fn cutout_tile(&self, shape: &TileShape) -> Option<Vec<TileShape>> {
        let tmp = shape
            .cutout_from_int_box(self)
            .expect("NullPointerException: cutoutFrom returned null");
        Some(tmp.into_iter().map(|s| s.simplify()).collect())
    }
    fn cutout_from_int_box_tile(&self, shape: &IntBox) -> Option<Vec<TileShape>> {
        Some(
            self.cutout_from_int_box(shape)
                .into_iter()
                .map(TileShape::IntBox)
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
        Some(self.bounding_octagon())
    }
    fn enlarge_tile(&self, offset: f64) -> TileShape {
        TileShape::IntOctagon(self.enlarge(offset))
    }
    fn intersects_int_box(&self, other: &IntBox) -> bool {
        IntBox::intersects_int_box(self, other)
    }
    fn intersects_int_octagon(&self, other: &IntOctagon) -> bool {
        IntBox::intersects_int_octagon(self, other)
    }
    fn intersects_simplex(&self, other: &Simplex) -> bool {
        IntBox::intersects_simplex(self, other)
    }
    fn intersects_circle(&self, other: &Circle) -> bool {
        IntBox::intersects_circle(self, other)
    }
    fn bounding_shape_tile(&self, dirs: &ShapeBoundingDirections) -> Option<RegularTileShape> {
        Some(self.bounding_shape(dirs))
    }
    fn area(&self) -> f64 {
        IntBox::area(self)
    }
    fn distance(&self, point: &FloatPoint) -> f64 {
        IntBox::distance(self, point)
    }
    fn turn_90_degree_tile(&self, factor: i32, pole: &IntPoint) -> TileShape {
        TileShape::IntBox(self.turn_90_degree(factor, pole))
    }
    fn divide_into_sections_tile(&self, max_section_width: f64) -> Vec<TileShape> {
        self.divide_into_sections(max_section_width)
            .into_iter()
            .map(TileShape::IntBox)
            .collect()
    }
}
