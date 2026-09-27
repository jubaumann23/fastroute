//! Port of `OrthogonalBoundingDirections.java`: the 4 orthogonal directions (singleton).

use crate::shape_bounding_directions::ShapeBoundingDirections;

/// Namespace for the Java singleton; the behaviour lives in [`ShapeBoundingDirections`].
pub struct OrthogonalBoundingDirections;

impl OrthogonalBoundingDirections {
    /// The one and only instantiation.
    pub const INSTANCE: ShapeBoundingDirections = ShapeBoundingDirections::Orthogonal;
}
