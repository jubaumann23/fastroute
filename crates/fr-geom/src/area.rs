//! Port of `Area.java`: a not necessarily simply connected shape, which may contain holes
//! (`Shape | PolylineArea`).

use crate::float_point::FloatPoint;
use crate::int_box::IntBox;
use crate::int_octagon::IntOctagon;
use crate::int_point::IntPoint;
use crate::point::Point;
use crate::polyline_area::PolylineArea;
use crate::shape::Shape;
use crate::tile_shape::TileShape;
use crate::vector::Vector;

/// Closed hierarchy `Area = Shape | PolylineArea`.
#[derive(Clone, Debug)]
pub enum Area {
    Shape(Shape),
    PolylineArea(PolylineArea),
}

impl Area {
    pub fn is_empty(&self) -> bool {
        match self {
            Area::Shape(s) => s.is_empty(),
            Area::PolylineArea(a) => a.is_empty(),
        }
    }

    pub fn is_bounded(&self) -> bool {
        match self {
            Area::Shape(s) => s.is_bounded(),
            Area::PolylineArea(a) => a.is_bounded(),
        }
    }

    /// 2 for two-dimensional shapes, 1 for curves, 0 for points, -1 if empty.
    pub fn dimension(&self) -> i32 {
        match self {
            Area::Shape(s) => s.dimension(),
            Area::PolylineArea(a) => a.dimension(),
        }
    }

    pub fn is_contained_in(&self, b: &IntBox) -> bool {
        match self {
            Area::Shape(s) => s.is_contained_in(b),
            Area::PolylineArea(a) => a.is_contained_in(b),
        }
    }

    /// Returns the border shape of this area.
    pub fn get_border(&self) -> Shape {
        match self {
            Area::Shape(s) => s.get_border(),
            Area::PolylineArea(a) => a.get_border().to_shape(),
        }
    }

    /// Returns the holes of this area.
    pub fn get_holes(&self) -> Vec<Shape> {
        match self {
            Area::Shape(s) => s.get_holes(),
            Area::PolylineArea(a) => a.get_holes().iter().map(|h| h.to_shape()).collect(),
        }
    }

    pub fn bounding_box(&self) -> IntBox {
        match self {
            Area::Shape(s) => s.bounding_box(),
            Area::PolylineArea(a) => a.bounding_box(),
        }
    }

    pub fn bounding_octagon(&self) -> Option<IntOctagon> {
        match self {
            Area::Shape(s) => s.bounding_octagon(),
            Area::PolylineArea(a) => a.bounding_octagon(),
        }
    }

    /// Java `contains(FloatPoint)`.
    pub fn contains_float(&self, point: &FloatPoint) -> bool {
        match self {
            Area::Shape(s) => s.contains_float(point),
            Area::PolylineArea(a) => a.contains_float(point),
        }
    }

    /// Java `contains(Point)`.
    pub fn contains(&self, point: &Point) -> bool {
        match self {
            Area::Shape(s) => s.contains(point),
            Area::PolylineArea(a) => a.contains(point),
        }
    }

    pub fn nearest_point_approx(&self, from_point: &FloatPoint) -> Option<FloatPoint> {
        match self {
            Area::Shape(s) => s.nearest_point_approx(from_point),
            Area::PolylineArea(a) => a.nearest_point_approx(from_point),
        }
    }

    pub fn turn_90_degree(&self, factor: i32, pole: &IntPoint) -> Area {
        match self {
            Area::Shape(s) => Area::Shape(s.turn_90_degree(factor, pole)),
            Area::PolylineArea(a) => Area::PolylineArea(a.turn_90_degree(factor, pole)),
        }
    }

    pub fn rotate_approx(&self, angle: f64, pole: &FloatPoint) -> Area {
        match self {
            Area::Shape(s) => Area::Shape(s.rotate_approx(angle, pole)),
            Area::PolylineArea(a) => Area::PolylineArea(a.rotate_approx(angle, pole)),
        }
    }

    pub fn translate_by(&self, vector: &Vector) -> Area {
        match self {
            Area::Shape(s) => Area::Shape(s.translate_by(vector)),
            Area::PolylineArea(a) => Area::PolylineArea(a.translate_by(vector)),
        }
    }

    pub fn mirror_horizontal(&self, pole: &IntPoint) -> Area {
        match self {
            Area::Shape(s) => Area::Shape(s.mirror_horizontal(pole)),
            Area::PolylineArea(a) => Area::PolylineArea(a.mirror_horizontal(pole)),
        }
    }

    pub fn mirror_vertical(&self, pole: &IntPoint) -> Area {
        match self {
            Area::Shape(s) => Area::Shape(s.mirror_vertical(pole)),
            Area::PolylineArea(a) => Area::PolylineArea(a.mirror_vertical(pole)),
        }
    }

    pub fn corner_approx_arr(&self) -> Vec<FloatPoint> {
        match self {
            Area::Shape(s) => s.corner_approx_arr(),
            Area::PolylineArea(a) => a.corner_approx_arr(),
        }
    }

    /// Division of this area into convex pieces (None if the division failed).
    pub fn split_to_convex(&self) -> Option<Vec<TileShape>> {
        match self {
            Area::Shape(s) => s.split_to_convex(),
            Area::PolylineArea(a) => a.split_to_convex(),
        }
    }

    pub fn as_shape(&self) -> Option<&Shape> {
        match self {
            Area::Shape(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_polyline_area(&self) -> Option<&PolylineArea> {
        match self {
            Area::PolylineArea(a) => Some(a),
            _ => None,
        }
    }
}

impl From<Shape> for Area {
    fn from(s: Shape) -> Self {
        Area::Shape(s)
    }
}

impl From<PolylineArea> for Area {
    fn from(a: PolylineArea) -> Self {
        Area::PolylineArea(a)
    }
}

impl From<TileShape> for Area {
    fn from(t: TileShape) -> Self {
        Area::Shape(Shape::Tile(t))
    }
}
