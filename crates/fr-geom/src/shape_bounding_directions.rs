//! Port of `ShapeBoundingDirections.java`: the fixed directions of a RegularTileShape
//! (implemented by `OrthogonalBoundingDirections` and `FortyfiveDegreeBoundingDirections`).

use crate::circle::Circle;
use crate::convex_shape::ConvexShape;
use crate::int_box::IntBox;
use crate::int_octagon::IntOctagon;
use crate::polygon_shape::PolygonShape;
use crate::regular_tile_shape::RegularTileShape;
use crate::simplex::Simplex;
use crate::tile_shape::TileShape;

/// Closed set of the two Java singletons.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShapeBoundingDirections {
    /// `OrthogonalBoundingDirections.INSTANCE`: the 4 orthogonal directions.
    Orthogonal,
    /// `FortyfiveDegreeBoundingDirections.INSTANCE`: the 8 multiples of 45 degree.
    FortyfiveDegree,
}

impl ShapeBoundingDirections {
    /// Returns the count of the fixed directions.
    pub fn count(&self) -> i32 {
        match self {
            ShapeBoundingDirections::Orthogonal => 4,
            ShapeBoundingDirections::FortyfiveDegree => 8,
        }
    }

    /// Calculates for an arbitrary ConvexShape a surrounding RegularTileShape with these fixed
    /// directions. None only if a simplex has no bounding octagon (Java returns null).
    pub fn bounds(&self, shape: &ConvexShape) -> Option<RegularTileShape> {
        // shape.boundingShape(this) -> this.bounds(<type of shape>)
        match shape {
            ConvexShape::Tile(TileShape::IntBox(b)) => Some(self.bounds_int_box(b)),
            ConvexShape::Tile(TileShape::IntOctagon(o)) => Some(self.bounds_int_octagon(o)),
            ConvexShape::Tile(TileShape::Simplex(s)) => self.bounds_simplex(s),
            ConvexShape::Circle(c) => Some(self.bounds_circle(c)),
        }
    }

    pub fn bounds_int_box(&self, b: &IntBox) -> RegularTileShape {
        match self {
            ShapeBoundingDirections::Orthogonal => RegularTileShape::IntBox(*b),
            ShapeBoundingDirections::FortyfiveDegree => {
                RegularTileShape::IntOctagon(b.to_int_octagon())
            }
        }
    }

    pub fn bounds_int_octagon(&self, oct: &IntOctagon) -> RegularTileShape {
        match self {
            ShapeBoundingDirections::Orthogonal => RegularTileShape::IntBox(oct.bounding_box()),
            ShapeBoundingDirections::FortyfiveDegree => RegularTileShape::IntOctagon(*oct),
        }
    }

    pub fn bounds_simplex(&self, simplex: &Simplex) -> Option<RegularTileShape> {
        match self {
            ShapeBoundingDirections::Orthogonal => {
                Some(RegularTileShape::IntBox(simplex.bounding_box()))
            }
            ShapeBoundingDirections::FortyfiveDegree => {
                simplex.bounding_octagon().map(RegularTileShape::IntOctagon)
            }
        }
    }

    pub fn bounds_circle(&self, circle: &Circle) -> RegularTileShape {
        match self {
            ShapeBoundingDirections::Orthogonal => RegularTileShape::IntBox(circle.bounding_box()),
            ShapeBoundingDirections::FortyfiveDegree => {
                RegularTileShape::IntOctagon(circle.bounding_octagon())
            }
        }
    }

    pub fn bounds_polygon(&self, polygon: &PolygonShape) -> RegularTileShape {
        match self {
            ShapeBoundingDirections::Orthogonal => RegularTileShape::IntBox(polygon.bounding_box()),
            ShapeBoundingDirections::FortyfiveDegree => {
                RegularTileShape::IntOctagon(polygon.bounding_octagon())
            }
        }
    }
}
