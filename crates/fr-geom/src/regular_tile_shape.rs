//! Port of `RegularTileShape.java`: TileShapes whose border lines have directions out of a fixed
//! set (orthogonal: IntBox, 45 degree: IntOctagon).

use crate::int_box::IntBox;
use crate::int_octagon::IntOctagon;
use crate::side::Side;
use crate::tile_shape::TileShape;

/// Closed hierarchy `RegularTileShape = IntBox | IntOctagon`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RegularTileShape {
    IntBox(IntBox),
    IntOctagon(IntOctagon),
}

impl RegularTileShape {
    /// Compares the edge lines of index edge_index of this shape and other: ON_THE_LEFT, if the
    /// edge line of this shape is to the left of the edge line of other; COLLINEAR if equal.
    pub fn compare(&self, other: &RegularTileShape, edge_index: i32) -> Side {
        match self {
            RegularTileShape::IntBox(b) => b.compare(other, edge_index),
            RegularTileShape::IntOctagon(o) => o.compare(other, edge_index),
        }
    }

    pub fn compare_int_box(&self, other: &IntBox, edge_index: i32) -> Side {
        match self {
            RegularTileShape::IntBox(b) => b.compare_int_box(other, edge_index),
            RegularTileShape::IntOctagon(o) => o.compare_int_box(other, edge_index),
        }
    }

    pub fn compare_int_octagon(&self, other: &IntOctagon, edge_index: i32) -> Side {
        match self {
            RegularTileShape::IntBox(b) => b.compare_int_octagon(other, edge_index),
            RegularTileShape::IntOctagon(o) => o.compare_int_octagon(other, edge_index),
        }
    }

    /// Calculates the smallest RegularTileShape containing this shape and other.
    pub fn union(&self, other: &RegularTileShape) -> RegularTileShape {
        match self {
            RegularTileShape::IntBox(b) => b.union(other),
            RegularTileShape::IntOctagon(o) => o.union(other),
        }
    }

    pub fn union_int_box(&self, other: &IntBox) -> RegularTileShape {
        match self {
            RegularTileShape::IntBox(b) => RegularTileShape::IntBox(b.union_int_box(other)),
            RegularTileShape::IntOctagon(o) => RegularTileShape::IntOctagon(o.union_int_box(other)),
        }
    }

    pub fn union_int_octagon(&self, other: &IntOctagon) -> RegularTileShape {
        match self {
            RegularTileShape::IntBox(b) => RegularTileShape::IntOctagon(b.union_int_octagon(other)),
            RegularTileShape::IntOctagon(o) => {
                RegularTileShape::IntOctagon(o.union_int_octagon(other))
            }
        }
    }

    /// Java `contains(RegularTileShape)`.
    pub fn contains(&self, other: &RegularTileShape) -> bool {
        match self {
            RegularTileShape::IntBox(b) => b.contains_regular(other),
            RegularTileShape::IntOctagon(o) => o.contains_regular(other),
        }
    }

    pub fn is_contained_in(&self, other: &IntBox) -> bool {
        match self {
            RegularTileShape::IntBox(b) => b.is_contained_in(other),
            RegularTileShape::IntOctagon(o) => o.is_contained_in(other),
        }
    }

    pub fn is_contained_in_int_octagon(&self, other: &IntOctagon) -> bool {
        match self {
            RegularTileShape::IntBox(b) => b.is_contained_in_int_octagon(other),
            RegularTileShape::IntOctagon(o) => o.is_contained_in_int_octagon(other),
        }
    }

    /// Upcast to TileShape.
    pub fn to_tile_shape(&self) -> TileShape {
        match self {
            RegularTileShape::IntBox(b) => TileShape::IntBox(*b),
            RegularTileShape::IntOctagon(o) => TileShape::IntOctagon(*o),
        }
    }

    pub fn is_empty(&self) -> bool {
        match self {
            RegularTileShape::IntBox(b) => b.is_empty(),
            RegularTileShape::IntOctagon(o) => o.is_empty(),
        }
    }

    pub fn bounding_box(&self) -> IntBox {
        match self {
            RegularTileShape::IntBox(b) => *b,
            RegularTileShape::IntOctagon(o) => o.bounding_box(),
        }
    }

    pub fn bounding_octagon(&self) -> IntOctagon {
        match self {
            RegularTileShape::IntBox(b) => b.bounding_octagon(),
            RegularTileShape::IntOctagon(o) => *o,
        }
    }

    pub fn dimension(&self) -> i32 {
        match self {
            RegularTileShape::IntBox(b) => b.dimension(),
            RegularTileShape::IntOctagon(o) => o.dimension(),
        }
    }

    pub fn as_int_box(&self) -> Option<&IntBox> {
        match self {
            RegularTileShape::IntBox(b) => Some(b),
            _ => None,
        }
    }

    pub fn as_int_octagon(&self) -> Option<&IntOctagon> {
        match self {
            RegularTileShape::IntOctagon(o) => Some(o),
            _ => None,
        }
    }
}

impl From<IntBox> for RegularTileShape {
    fn from(b: IntBox) -> Self {
        RegularTileShape::IntBox(b)
    }
}

impl From<IntOctagon> for RegularTileShape {
    fn from(o: IntOctagon) -> Self {
        RegularTileShape::IntOctagon(o)
    }
}
