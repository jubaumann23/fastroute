//! Port of `ConvexShape.java`: shapes for which each segment between two contained points is
//! contained (`TileShape | Circle`).

use crate::circle::Circle;
use crate::int_box::IntBox;
use crate::int_octagon::IntOctagon;
use crate::regular_tile_shape::RegularTileShape;
use crate::shape::Shape;
use crate::shape_bounding_directions::ShapeBoundingDirections;
use crate::tile_shape::TileShape;

/// Closed hierarchy `ConvexShape = TileShape | Circle`.
#[derive(Clone, Debug)]
pub enum ConvexShape {
    Tile(TileShape),
    Circle(Circle),
}

impl ConvexShape {
    /// Calculates the offset shape by distance (enlarged if distance > 0).
    pub fn offset(&self, distance: f64) -> ConvexShape {
        match self {
            ConvexShape::Tile(t) => ConvexShape::Tile(t.offset(distance)),
            ConvexShape::Circle(c) => ConvexShape::Circle(c.offset(distance)),
        }
    }

    /// Shrinks the shape by offset. The result shape will not be empty.
    pub fn shrink(&self, offset: f64) -> ConvexShape {
        match self {
            ConvexShape::Tile(t) => ConvexShape::Tile(t.shrink(offset)),
            ConvexShape::Circle(c) => ConvexShape::Circle(c.shrink(offset)),
        }
    }

    /// Returns the maximum diameter of the shape.
    pub fn max_width(&self) -> f64 {
        match self {
            ConvexShape::Tile(t) => t.max_width(),
            ConvexShape::Circle(c) => c.max_width(),
        }
    }

    /// Returns the minimum diameter of the shape.
    pub fn min_width(&self) -> f64 {
        match self {
            ConvexShape::Tile(t) => t.min_width(),
            ConvexShape::Circle(c) => c.min_width(),
        }
    }

    pub fn is_empty(&self) -> bool {
        match self {
            ConvexShape::Tile(t) => t.is_empty(),
            ConvexShape::Circle(c) => c.is_empty(),
        }
    }

    pub fn dimension(&self) -> i32 {
        match self {
            ConvexShape::Tile(t) => t.dimension(),
            ConvexShape::Circle(c) => c.dimension(),
        }
    }

    pub fn bounding_box(&self) -> IntBox {
        match self {
            ConvexShape::Tile(t) => t.bounding_box(),
            ConvexShape::Circle(c) => c.bounding_box(),
        }
    }

    pub fn bounding_octagon(&self) -> Option<IntOctagon> {
        match self {
            ConvexShape::Tile(t) => t.bounding_octagon(),
            ConvexShape::Circle(c) => Some(c.bounding_octagon()),
        }
    }

    pub fn bounding_shape(&self, dirs: &ShapeBoundingDirections) -> Option<RegularTileShape> {
        dirs.bounds(self)
    }

    /// Upcast to Shape.
    pub fn to_shape(&self) -> Shape {
        match self {
            ConvexShape::Tile(t) => Shape::Tile(t.clone()),
            ConvexShape::Circle(c) => Shape::Circle(*c),
        }
    }

    pub fn as_tile_shape(&self) -> Option<&TileShape> {
        match self {
            ConvexShape::Tile(t) => Some(t),
            _ => None,
        }
    }

    pub fn as_circle(&self) -> Option<&Circle> {
        match self {
            ConvexShape::Circle(c) => Some(c),
            _ => None,
        }
    }
}

impl From<TileShape> for ConvexShape {
    fn from(t: TileShape) -> Self {
        ConvexShape::Tile(t)
    }
}

impl From<Circle> for ConvexShape {
    fn from(c: Circle) -> Self {
        ConvexShape::Circle(c)
    }
}
