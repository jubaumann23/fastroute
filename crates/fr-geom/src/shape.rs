//! Port of `Shape.java` (and the `Area` methods it inherits): simply connected 2-dimensional
//! shapes (`TileShape | Circle | PolygonShape`).

use crate::circle::Circle;
use crate::convex_shape::ConvexShape;
use crate::float_point::FloatPoint;
use crate::int_box::IntBox;
use crate::int_octagon::IntOctagon;
use crate::int_point::IntPoint;
use crate::point::Point;
use crate::polygon_shape::PolygonShape;
use crate::polyline::Polyline;
use crate::polyline_shape::{PolylineShape, PolylineShapeImpl};
use crate::regular_tile_shape::RegularTileShape;
use crate::shape_bounding_directions::ShapeBoundingDirections;
use crate::simplex::Simplex;
use crate::tile_shape::TileShape;
use crate::vector::Vector;

/// Closed hierarchy `Shape = TileShape | Circle | PolygonShape`.
#[derive(Clone, Debug)]
pub enum Shape {
    Tile(TileShape),
    Circle(Circle),
    Polygon(PolygonShape),
}

impl Shape {
    pub fn is_empty(&self) -> bool {
        match self {
            Shape::Tile(t) => t.is_empty(),
            Shape::Circle(c) => c.is_empty(),
            Shape::Polygon(p) => PolylineShapeImpl::is_empty(p),
        }
    }

    pub fn is_bounded(&self) -> bool {
        match self {
            Shape::Tile(t) => t.is_bounded(),
            Shape::Circle(c) => c.is_bounded(),
            Shape::Polygon(p) => PolylineShapeImpl::is_bounded(p),
        }
    }

    /// 2 for two-dimensional shapes, 1 for curves, 0 for points, -1 if empty.
    pub fn dimension(&self) -> i32 {
        match self {
            Shape::Tile(t) => t.dimension(),
            Shape::Circle(c) => c.dimension(),
            Shape::Polygon(p) => PolylineShapeImpl::dimension(p),
        }
    }

    /// Checks, if this shape is completely contained in box.
    pub fn is_contained_in(&self, b: &IntBox) -> bool {
        match self {
            Shape::Tile(t) => t.is_contained_in(b),
            Shape::Circle(c) => c.is_contained_in(b),
            Shape::Polygon(p) => PolylineShapeImpl::is_contained_in(p, b),
        }
    }

    /// Returns the border shape (this shape).
    pub fn get_border(&self) -> Shape {
        self.clone()
    }

    /// Returns the (empty) holes of this shape.
    pub fn get_holes(&self) -> Vec<Shape> {
        Vec::new()
    }

    pub fn bounding_box(&self) -> IntBox {
        match self {
            Shape::Tile(t) => t.bounding_box(),
            Shape::Circle(c) => c.bounding_box(),
            Shape::Polygon(p) => p.bounding_box(),
        }
    }

    /// None only for unbounded simplices (Java returns null).
    pub fn bounding_octagon(&self) -> Option<IntOctagon> {
        match self {
            Shape::Tile(t) => t.bounding_octagon(),
            Shape::Circle(c) => Some(c.bounding_octagon()),
            Shape::Polygon(p) => Some(p.bounding_octagon()),
        }
    }

    /// Java `contains(FloatPoint)`.
    pub fn contains_float(&self, point: &FloatPoint) -> bool {
        match self {
            Shape::Tile(t) => t.contains_float(point),
            Shape::Circle(c) => c.contains_float(point),
            Shape::Polygon(p) => p.contains_float(point),
        }
    }

    /// Java `contains(Point)`: inside or on the border.
    pub fn contains(&self, point: &Point) -> bool {
        match self {
            Shape::Tile(t) => t.contains(point),
            Shape::Circle(c) => c.contains(point),
            Shape::Polygon(p) => p.contains(point),
        }
    }

    pub fn nearest_point_approx(&self, from_point: &FloatPoint) -> Option<FloatPoint> {
        match self {
            Shape::Tile(t) => t.nearest_point_approx(from_point),
            Shape::Circle(c) => c.nearest_point_approx(from_point),
            Shape::Polygon(p) => p.nearest_point_approx(from_point),
        }
    }

    pub fn turn_90_degree(&self, factor: i32, pole: &IntPoint) -> Shape {
        match self {
            Shape::Tile(t) => Shape::Tile(t.turn_90_degree(factor, pole)),
            Shape::Circle(c) => Shape::Circle(c.turn_90_degree(factor, pole)),
            Shape::Polygon(p) => Shape::Polygon(p.turn_90_degree(factor, pole)),
        }
    }

    pub fn rotate_approx(&self, angle: f64, pole: &FloatPoint) -> Shape {
        match self {
            Shape::Tile(t) => Shape::Tile(t.rotate_approx(angle, pole)),
            Shape::Circle(c) => Shape::Circle(c.rotate_approx(angle, pole)),
            Shape::Polygon(p) => Shape::Polygon(p.rotate_approx(angle, pole)),
        }
    }

    pub fn translate_by(&self, vector: &Vector) -> Shape {
        match self {
            Shape::Tile(t) => Shape::Tile(t.translate_by(vector)),
            Shape::Circle(c) => Shape::Circle(c.translate_by(vector)),
            Shape::Polygon(p) => Shape::Polygon(p.translate_by(vector)),
        }
    }

    pub fn mirror_horizontal(&self, pole: &IntPoint) -> Shape {
        match self {
            Shape::Tile(t) => Shape::Tile(t.mirror_horizontal(pole)),
            Shape::Circle(c) => Shape::Circle(c.mirror_horizontal(pole)),
            Shape::Polygon(p) => Shape::Polygon(p.mirror_horizontal(pole)),
        }
    }

    pub fn mirror_vertical(&self, pole: &IntPoint) -> Shape {
        match self {
            Shape::Tile(t) => Shape::Tile(t.mirror_vertical(pole)),
            Shape::Circle(c) => Shape::Circle(c.mirror_vertical(pole)),
            Shape::Polygon(p) => Shape::Polygon(p.mirror_vertical(pole)),
        }
    }

    pub fn corner_approx_arr(&self) -> Vec<FloatPoint> {
        match self {
            Shape::Tile(t) => t.corner_approx_arr(),
            Shape::Circle(c) => c.corner_approx_arr(),
            Shape::Polygon(p) => PolylineShapeImpl::corner_approx_arr(p),
        }
    }

    /// Division into convex pieces (None if the division of a polygon failed).
    pub fn split_to_convex(&self) -> Option<Vec<TileShape>> {
        match self {
            Shape::Tile(t) => Some(t.split_to_convex()),
            Shape::Circle(c) => Some(c.split_to_convex()),
            Shape::Polygon(p) => p.split_to_convex(),
        }
    }

    /// Border length; Integer.MAX_VALUE if unbounded.
    pub fn circumference(&self) -> f64 {
        match self {
            Shape::Tile(t) => t.circumference(),
            Shape::Circle(c) => c.circumference(),
            Shape::Polygon(p) => PolylineShapeImpl::circumference(p),
        }
    }

    /// Area content; Double.MAX_VALUE if unbounded.
    pub fn area(&self) -> f64 {
        match self {
            Shape::Tile(t) => t.area(),
            Shape::Circle(c) => c.area(),
            Shape::Polygon(p) => p.area(),
        }
    }

    pub fn centre_of_gravity(&self) -> FloatPoint {
        match self {
            Shape::Tile(t) => t.centre_of_gravity(),
            Shape::Circle(c) => c.centre_of_gravity(),
            Shape::Polygon(p) => PolylineShapeImpl::centre_of_gravity(p),
        }
    }

    pub fn is_outside(&self, point: &Point) -> bool {
        match self {
            Shape::Tile(t) => t.is_outside(point),
            Shape::Circle(c) => c.is_outside(point),
            Shape::Polygon(p) => p.is_outside(point),
        }
    }

    pub fn contains_inside(&self, point: &Point) -> bool {
        match self {
            Shape::Tile(t) => t.contains_inside(point),
            Shape::Circle(c) => c.contains_inside(point),
            Shape::Polygon(p) => p.contains_inside(point),
        }
    }

    pub fn contains_on_border(&self, point: &Point) -> bool {
        match self {
            Shape::Tile(t) => t.contains_on_border(point),
            Shape::Circle(c) => c.contains_on_border(point),
            Shape::Polygon(p) => p.contains_on_border(point),
        }
    }

    pub fn distance(&self, point: &FloatPoint) -> f64 {
        match self {
            Shape::Tile(t) => t.distance(point),
            Shape::Circle(c) => c.distance(point),
            Shape::Polygon(p) => p.distance(point),
        }
    }

    pub fn bounding_tile(&self) -> TileShape {
        match self {
            Shape::Tile(t) => t.bounding_tile(),
            Shape::Circle(c) => c.bounding_tile(),
            Shape::Polygon(p) => p.bounding_tile(),
        }
    }

    pub fn bounding_shape(&self, dirs: &ShapeBoundingDirections) -> Option<RegularTileShape> {
        match self {
            Shape::Tile(t) => t.bounding_shape(dirs),
            Shape::Circle(c) => Some(c.bounding_shape(dirs)),
            Shape::Polygon(p) => Some(p.bounding_shape(dirs)),
        }
    }

    pub fn border_distance(&self, point: &FloatPoint) -> f64 {
        match self {
            Shape::Tile(t) => t.border_distance(point),
            Shape::Circle(c) => c.border_distance(point),
            Shape::Polygon(p) => p.border_distance(point),
        }
    }

    pub fn smallest_radius(&self) -> f64 {
        match self {
            Shape::Tile(t) => t.smallest_radius(),
            Shape::Circle(c) => c.smallest_radius(),
            Shape::Polygon(p) => p.smallest_radius(),
        }
    }

    /// Offset shape; the variant may change (an enlarged IntBox is an IntOctagon). None for
    /// polygons with offset != 0 (not implemented in Java, returns null).
    pub fn enlarge(&self, offset: f64) -> Option<Shape> {
        match self {
            Shape::Tile(t) => Some(Shape::Tile(t.enlarge(offset))),
            Shape::Circle(c) => Some(Shape::Circle(c.enlarge(offset))),
            Shape::Polygon(p) => p.enlarge(offset).map(Shape::Polygon),
        }
    }

    /// Checks, if this shape and other have a nonempty intersection (Java double dispatch).
    ///
    /// Panics for two PolygonShapes: in Java that call recurses infinitely
    /// (StackOverflowError).
    pub fn intersects(&self, other: &Shape) -> bool {
        match self {
            Shape::Tile(t) => t.intersects(other),
            Shape::Circle(c) => other.intersects_circle(c),
            Shape::Polygon(p) => match other {
                // other.intersects((Shape) this) -> this.intersects(<type of other>)
                Shape::Tile(TileShape::IntBox(b)) => p.intersects_int_box(b),
                Shape::Tile(TileShape::IntOctagon(o)) => p.intersects_int_octagon(o),
                Shape::Tile(TileShape::Simplex(s)) => p.intersects_simplex(s),
                Shape::Circle(c) => p.intersects_circle(c),
                Shape::Polygon(_) => {
                    panic!("StackOverflowError: PolygonShape.intersects(PolygonShape) recurses infinitely in Java")
                }
            },
        }
    }

    pub fn intersects_int_box(&self, other: &IntBox) -> bool {
        match self {
            Shape::Tile(t) => t.intersects_int_box(other),
            Shape::Circle(c) => c.intersects_int_box(other),
            Shape::Polygon(p) => p.intersects_int_box(other),
        }
    }

    pub fn intersects_int_octagon(&self, other: &IntOctagon) -> bool {
        match self {
            Shape::Tile(t) => t.intersects_int_octagon(other),
            Shape::Circle(c) => c.intersects_int_octagon(other),
            Shape::Polygon(p) => p.intersects_int_octagon(other),
        }
    }

    pub fn intersects_simplex(&self, other: &Simplex) -> bool {
        match self {
            Shape::Tile(t) => t.intersects_simplex(other),
            Shape::Circle(c) => c.intersects_simplex(other),
            Shape::Polygon(p) => p.intersects_simplex(other),
        }
    }

    pub fn intersects_circle(&self, other: &Circle) -> bool {
        match self {
            Shape::Tile(t) => t.intersects_circle(other),
            Shape::Circle(c) => c.intersects_circle(other),
            Shape::Polygon(p) => p.intersects_circle(other),
        }
    }

    /// Cuts out the parts of polyline in the interior of this shape (None where Java returns
    /// null: circles and polygons).
    pub fn cutout_polyline(&self, polyline: &Polyline) -> Option<Vec<Polyline>> {
        match self {
            Shape::Tile(t) => Some(t.cutout_polyline(polyline)),
            Shape::Circle(c) => c.cutout_polyline(polyline),
            Shape::Polygon(p) => p.cutout_polyline(polyline),
        }
    }

    // ----- casts -----

    pub fn as_tile_shape(&self) -> Option<&TileShape> {
        match self {
            Shape::Tile(t) => Some(t),
            _ => None,
        }
    }

    pub fn as_circle(&self) -> Option<&Circle> {
        match self {
            Shape::Circle(c) => Some(c),
            _ => None,
        }
    }

    pub fn as_polygon_shape(&self) -> Option<&PolygonShape> {
        match self {
            Shape::Polygon(p) => Some(p),
            _ => None,
        }
    }

    pub fn as_int_box(&self) -> Option<&IntBox> {
        self.as_tile_shape().and_then(|t| t.as_int_box())
    }

    pub fn as_int_octagon(&self) -> Option<&IntOctagon> {
        self.as_tile_shape().and_then(|t| t.as_int_octagon())
    }

    pub fn as_simplex(&self) -> Option<&Simplex> {
        self.as_tile_shape().and_then(|t| t.as_simplex())
    }

    /// Java `instanceof ConvexShape`.
    pub fn to_convex_shape(&self) -> Option<ConvexShape> {
        match self {
            Shape::Tile(t) => Some(ConvexShape::Tile(t.clone())),
            Shape::Circle(c) => Some(ConvexShape::Circle(*c)),
            Shape::Polygon(_) => None,
        }
    }

    /// Java `instanceof PolylineShape`.
    pub fn to_polyline_shape(&self) -> Option<PolylineShape> {
        match self {
            Shape::Tile(t) => Some(PolylineShape::Tile(t.clone())),
            Shape::Circle(_) => None,
            Shape::Polygon(p) => Some(PolylineShape::Polygon(p.clone())),
        }
    }
}

impl From<TileShape> for Shape {
    fn from(t: TileShape) -> Self {
        Shape::Tile(t)
    }
}

impl From<IntBox> for Shape {
    fn from(b: IntBox) -> Self {
        Shape::Tile(TileShape::IntBox(b))
    }
}

impl From<IntOctagon> for Shape {
    fn from(o: IntOctagon) -> Self {
        Shape::Tile(TileShape::IntOctagon(o))
    }
}

impl From<Simplex> for Shape {
    fn from(s: Simplex) -> Self {
        Shape::Tile(TileShape::Simplex(s))
    }
}

impl From<Circle> for Shape {
    fn from(c: Circle) -> Self {
        Shape::Circle(c)
    }
}

impl From<PolygonShape> for Shape {
    fn from(p: PolygonShape) -> Self {
        Shape::Polygon(p)
    }
}

impl From<ConvexShape> for Shape {
    fn from(c: ConvexShape) -> Self {
        c.to_shape()
    }
}

impl From<PolylineShape> for Shape {
    fn from(p: PolylineShape) -> Self {
        p.to_shape()
    }
}
