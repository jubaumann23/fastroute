//! Port of `PolylineShape.java`: shapes whose borders consist of straight lines
//! (`TileShape | PolygonShape`).
//!
//! The Java abstract class is split into the trait [`PolylineShapeImpl`] (abstract methods plus
//! the inherited default implementations, which subclasses may override) and the closed enum
//! [`PolylineShape`] used as the value type.

use crate::float_line::FloatLine;
use crate::float_point::FloatPoint;
use crate::int_box::IntBox;
use crate::int_octagon::IntOctagon;
use crate::int_point::IntPoint;
use crate::line::Line;
use crate::point::Point;
use crate::polygon_shape::PolygonShape;
use crate::polyline::Polyline;
use crate::shape::Shape;
use crate::side::Side;
use crate::tile_shape::TileShape;
use crate::vector::Vector;

/// Abstract part and default methods of Java `PolylineShape`.
pub trait PolylineShapeImpl {
    /// Returns the number of border lines of the shape.
    fn border_line_count(&self) -> i32;
    /// Returns the no-th corner of this shape for no between 0 and border_line_count() - 1.
    fn corner(&self, no: i32) -> Point;
    /// Returns the no-th border line of this shape.
    fn border_line(&self, no: i32) -> Line;
    /// Returns true if the shape has no infinite part at this corner.
    fn corner_is_bounded(&self, no: i32) -> bool;
    fn is_empty(&self) -> bool;
    fn is_bounded(&self) -> bool;
    fn dimension(&self) -> i32;
    fn bounding_box(&self) -> IntBox;

    /// Approximation of the no-th corner. If the shape is not bounded at this corner, the
    /// coordinates are Integer.MAX_VALUE.
    fn corner_approx(&self, no: i32) -> FloatPoint {
        self.corner(no).to_float()
    }

    /// Like `corner_approx`, but None where Java's `cornerApprox` returns null without failing
    /// (only an empty Simplex). Used by the defaults below that read a corner before a loop.
    fn corner_approx_opt(&self, no: i32) -> Option<FloatPoint> {
        Some(self.corner_approx(no))
    }

    /// Approximation of all corners of this shape.
    fn corner_approx_arr(&self) -> Vec<FloatPoint> {
        (0..self.border_line_count())
            .map(|i| self.corner_approx(i))
            .collect()
    }

    /// Returns all bounded corners of this shape.
    fn bounded_corners(&self) -> Vec<Point> {
        (0..self.border_line_count())
            .filter(|&i| self.corner_is_bounded(i))
            .map(|i| self.corner(i))
            .collect()
    }

    /// If point is equal to a corner of this shape, the number of that corner is returned; -1
    /// otherwise.
    fn equals_corner(&self, point: &Point) -> i32 {
        for i in 0..self.border_line_count() {
            if *point == self.corner(i) {
                return i;
            }
        }
        -1
    }

    /// Cumulative border line length; Integer.MAX_VALUE if the shape is unbounded.
    fn circumference(&self) -> f64 {
        if !self.is_bounded() {
            return i32::MAX as f64;
        }
        let corner_count = self.border_line_count();
        let mut result = 0.0;
        let mut prev_corner = self.corner_approx_opt(corner_count - 1);
        for i in 0..corner_count {
            let current_corner = self.corner_approx(i);
            result += current_corner.distance(prev_corner.as_ref().expect("NullPointerException"));
            prev_corner = Some(current_corner);
        }
        result
    }

    /// Returns the arithmetic middle of the corners of this shape.
    fn centre_of_gravity(&self) -> FloatPoint {
        let corner_count = self.border_line_count();
        let mut x = 0.0;
        let mut y = 0.0;
        for i in 0..corner_count {
            let p = self.corner_approx(i);
            x += p.x;
            y += p.y;
        }
        x /= corner_count as f64;
        y /= corner_count as f64;
        FloatPoint::new(x, y)
    }

    /// Checks if this shape is completely contained in box.
    fn is_contained_in(&self, b: &IntBox) -> bool {
        // box.contains(boundingBox()) -> boundingBox().isContainedIn(box)
        self.bounding_box().is_contained_in(b)
    }

    /// Index of the corner such that all other points are to the right of the line from
    /// from_point to this corner.
    fn index_of_left_most_corner(&self, from_point: &FloatPoint) -> i32 {
        let mut left_most_corner = self.corner_approx_opt(0);
        let mut result = 0;
        for i in 1..self.border_line_count() {
            let current_corner = self.corner_approx(i);
            let lmc = left_most_corner.as_ref().expect("NullPointerException");
            if current_corner.side_of(from_point, lmc) == Side::OnTheLeft {
                left_most_corner = Some(current_corner);
                result = i;
            }
        }
        result
    }

    /// Index of the corner such that all other points are to the left of the line from
    /// from_point to this corner.
    fn index_of_right_most_corner(&self, from_point: &FloatPoint) -> i32 {
        let mut right_most_corner = self.corner_approx_opt(0);
        let mut result = 0;
        for i in 1..self.border_line_count() {
            let current_corner = self.corner_approx(i);
            let rmc = right_most_corner.as_ref().expect("NullPointerException");
            if current_corner.side_of(from_point, rmc) == Side::OnTheRight {
                right_most_corner = Some(current_corner);
                result = i;
            }
        }
        result
    }

    /// FloatLine whose a is the left most and b the right most corner seen from from_point.
    /// None if the shape is empty.
    fn polar_line_segment(&self, from_point: &FloatPoint) -> Option<FloatLine> {
        if self.is_empty() {
            log::warn!("PolylineShape.polarLineSegment: shape is empty");
            return None;
        }
        let mut left_most_corner = self.corner_approx(0);
        let mut right_most_corner = self.corner_approx(0);
        for i in 1..self.border_line_count() {
            let current_corner = self.corner_approx(i);
            if current_corner.side_of(from_point, &right_most_corner) == Side::OnTheRight {
                right_most_corner = current_corner;
            }
            if current_corner.side_of(from_point, &left_most_corner) == Side::OnTheLeft {
                left_most_corner = current_corner;
            }
        }
        Some(FloatLine::new(left_most_corner, right_most_corner))
    }

    /// Returns the previous border line or corner number of this shape.
    fn prev_no(&self, no: i32) -> i32 {
        if no == 0 {
            self.border_line_count() - 1
        } else {
            no - 1
        }
    }

    /// Returns the next border line or corner number of this shape.
    fn next_no(&self, no: i32) -> i32 {
        (no + 1) % self.border_line_count()
    }

    /// Checks, if this shape and line have a common point (Java `intersects(Line)`).
    fn intersects_line(&self, line: &Line) -> bool {
        let side_of_first_corner = line.side_of(&self.corner(0));
        if side_of_first_corner == Side::Collinear {
            return true;
        }
        for i in 1..self.border_line_count() {
            if line.side_of(&self.corner(i)) != side_of_first_corner {
                return true;
            }
        }
        false
    }

    /// Calculates the left most corner of this shape, when looked at from from_point.
    fn left_most_corner(&self, from_point: &Point) -> Point {
        if self.is_empty() {
            return from_point.clone();
        }
        let mut result = self.corner(0);
        for i in 1..self.border_line_count() {
            let current_corner = self.corner(i);
            if current_corner.side_of(from_point, &result) == Side::OnTheLeft {
                result = current_corner;
            }
        }
        result
    }

    /// Calculates the right most corner of this shape, when looked at from from_point.
    fn right_most_corner(&self, from_point: &Point) -> Point {
        if self.is_empty() {
            return from_point.clone();
        }
        let mut result = self.corner(0);
        for i in 1..self.border_line_count() {
            let current_corner = self.corner(i);
            if current_corner.side_of(from_point, &result) == Side::OnTheRight {
                result = current_corner;
            }
        }
        result
    }
}

/// Closed hierarchy `PolylineShape = TileShape | PolygonShape`.
#[derive(Clone, Debug)]
pub enum PolylineShape {
    Tile(TileShape),
    Polygon(PolygonShape),
}

macro_rules! pl_dispatch {
    ($self:expr, $s:ident => $e:expr) => {
        match $self {
            PolylineShape::Tile(TileShape::IntBox($s)) => $e,
            PolylineShape::Tile(TileShape::IntOctagon($s)) => $e,
            PolylineShape::Tile(TileShape::Simplex($s)) => $e,
            PolylineShape::Polygon($s) => $e,
        }
    };
}

impl PolylineShape {
    pub fn border_line_count(&self) -> i32 {
        pl_dispatch!(self, s => PolylineShapeImpl::border_line_count(s))
    }
    pub fn corner(&self, no: i32) -> Point {
        pl_dispatch!(self, s => PolylineShapeImpl::corner(s, no))
    }
    pub fn border_line(&self, no: i32) -> Line {
        pl_dispatch!(self, s => PolylineShapeImpl::border_line(s, no))
    }
    pub fn corner_is_bounded(&self, no: i32) -> bool {
        pl_dispatch!(self, s => PolylineShapeImpl::corner_is_bounded(s, no))
    }
    pub fn is_empty(&self) -> bool {
        pl_dispatch!(self, s => PolylineShapeImpl::is_empty(s))
    }
    pub fn is_bounded(&self) -> bool {
        pl_dispatch!(self, s => PolylineShapeImpl::is_bounded(s))
    }
    pub fn dimension(&self) -> i32 {
        pl_dispatch!(self, s => PolylineShapeImpl::dimension(s))
    }
    pub fn bounding_box(&self) -> IntBox {
        pl_dispatch!(self, s => PolylineShapeImpl::bounding_box(s))
    }
    pub fn corner_approx(&self, no: i32) -> FloatPoint {
        pl_dispatch!(self, s => PolylineShapeImpl::corner_approx(s, no))
    }
    pub fn corner_approx_arr(&self) -> Vec<FloatPoint> {
        pl_dispatch!(self, s => PolylineShapeImpl::corner_approx_arr(s))
    }
    pub fn bounded_corners(&self) -> Vec<Point> {
        pl_dispatch!(self, s => PolylineShapeImpl::bounded_corners(s))
    }
    pub fn equals_corner(&self, point: &Point) -> i32 {
        pl_dispatch!(self, s => PolylineShapeImpl::equals_corner(s, point))
    }
    pub fn circumference(&self) -> f64 {
        pl_dispatch!(self, s => PolylineShapeImpl::circumference(s))
    }
    pub fn centre_of_gravity(&self) -> FloatPoint {
        pl_dispatch!(self, s => PolylineShapeImpl::centre_of_gravity(s))
    }
    pub fn is_contained_in(&self, b: &IntBox) -> bool {
        pl_dispatch!(self, s => PolylineShapeImpl::is_contained_in(s, b))
    }
    pub fn index_of_left_most_corner(&self, from_point: &FloatPoint) -> i32 {
        pl_dispatch!(self, s => PolylineShapeImpl::index_of_left_most_corner(s, from_point))
    }
    pub fn index_of_right_most_corner(&self, from_point: &FloatPoint) -> i32 {
        pl_dispatch!(self, s => PolylineShapeImpl::index_of_right_most_corner(s, from_point))
    }
    pub fn polar_line_segment(&self, from_point: &FloatPoint) -> Option<FloatLine> {
        pl_dispatch!(self, s => PolylineShapeImpl::polar_line_segment(s, from_point))
    }
    pub fn prev_no(&self, no: i32) -> i32 {
        pl_dispatch!(self, s => PolylineShapeImpl::prev_no(s, no))
    }
    pub fn next_no(&self, no: i32) -> i32 {
        pl_dispatch!(self, s => PolylineShapeImpl::next_no(s, no))
    }
    /// Java `intersects(Line)`.
    pub fn intersects_line(&self, line: &Line) -> bool {
        pl_dispatch!(self, s => PolylineShapeImpl::intersects_line(s, line))
    }
    pub fn left_most_corner(&self, from_point: &Point) -> Point {
        pl_dispatch!(self, s => PolylineShapeImpl::left_most_corner(s, from_point))
    }
    pub fn right_most_corner(&self, from_point: &Point) -> Point {
        pl_dispatch!(self, s => PolylineShapeImpl::right_most_corner(s, from_point))
    }

    /// Returns this shape (Java `getBorder()`).
    pub fn get_border(&self) -> PolylineShape {
        self.clone()
    }

    /// Returns the (empty) holes of this shape.
    pub fn get_holes(&self) -> Vec<Shape> {
        Vec::new()
    }

    /// Upcast to Shape.
    pub fn to_shape(&self) -> Shape {
        match self {
            PolylineShape::Tile(t) => Shape::Tile(t.clone()),
            PolylineShape::Polygon(p) => Shape::Polygon(p.clone()),
        }
    }

    pub fn bounding_octagon(&self) -> Option<IntOctagon> {
        self.to_shape().bounding_octagon()
    }

    pub fn contains(&self, point: &Point) -> bool {
        match self {
            PolylineShape::Tile(t) => t.contains(point),
            PolylineShape::Polygon(p) => p.contains(point),
        }
    }

    pub fn contains_float(&self, point: &FloatPoint) -> bool {
        match self {
            PolylineShape::Tile(t) => t.contains_float(point),
            PolylineShape::Polygon(p) => p.contains_float(point),
        }
    }

    pub fn contains_inside(&self, point: &Point) -> bool {
        match self {
            PolylineShape::Tile(t) => t.contains_inside(point),
            PolylineShape::Polygon(p) => p.contains_inside(point),
        }
    }

    /// Returns a division of this shape into convex pieces (None if the division failed).
    pub fn split_to_convex(&self) -> Option<Vec<TileShape>> {
        match self {
            PolylineShape::Tile(t) => Some(t.split_to_convex()),
            PolylineShape::Polygon(p) => p.split_to_convex(),
        }
    }

    pub fn turn_90_degree(&self, factor: i32, pole: &IntPoint) -> PolylineShape {
        match self {
            PolylineShape::Tile(t) => PolylineShape::Tile(t.turn_90_degree(factor, pole)),
            PolylineShape::Polygon(p) => PolylineShape::Polygon(p.turn_90_degree(factor, pole)),
        }
    }

    pub fn rotate_approx(&self, angle: f64, pole: &FloatPoint) -> PolylineShape {
        match self {
            PolylineShape::Tile(t) => PolylineShape::Tile(t.rotate_approx(angle, pole)),
            PolylineShape::Polygon(p) => PolylineShape::Polygon(p.rotate_approx(angle, pole)),
        }
    }

    pub fn mirror_horizontal(&self, pole: &IntPoint) -> PolylineShape {
        match self {
            PolylineShape::Tile(t) => PolylineShape::Tile(t.mirror_horizontal(pole)),
            PolylineShape::Polygon(p) => PolylineShape::Polygon(p.mirror_horizontal(pole)),
        }
    }

    pub fn mirror_vertical(&self, pole: &IntPoint) -> PolylineShape {
        match self {
            PolylineShape::Tile(t) => PolylineShape::Tile(t.mirror_vertical(pole)),
            PolylineShape::Polygon(p) => PolylineShape::Polygon(p.mirror_vertical(pole)),
        }
    }

    pub fn translate_by(&self, vector: &Vector) -> PolylineShape {
        match self {
            PolylineShape::Tile(t) => PolylineShape::Tile(t.translate_by(vector)),
            PolylineShape::Polygon(p) => PolylineShape::Polygon(p.translate_by(vector)),
        }
    }

    /// Cuts out the parts of polyline in the interior of this shape (None where Java returns
    /// null).
    pub fn cutout_polyline(&self, polyline: &Polyline) -> Option<Vec<Polyline>> {
        self.to_shape().cutout_polyline(polyline)
    }

    pub fn as_tile_shape(&self) -> Option<&TileShape> {
        match self {
            PolylineShape::Tile(t) => Some(t),
            _ => None,
        }
    }

    pub fn as_polygon_shape(&self) -> Option<&PolygonShape> {
        match self {
            PolylineShape::Polygon(p) => Some(p),
            _ => None,
        }
    }
}

impl From<TileShape> for PolylineShape {
    fn from(t: TileShape) -> Self {
        PolylineShape::Tile(t)
    }
}

impl From<PolygonShape> for PolylineShape {
    fn from(p: PolygonShape) -> Self {
        PolylineShape::Polygon(p)
    }
}
