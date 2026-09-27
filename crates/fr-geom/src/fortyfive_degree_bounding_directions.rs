//! Port of `FortyfiveDegreeBoundingDirections.java`: the 8 directions which are multiples of
//! 45 degree (singleton).

use crate::shape_bounding_directions::ShapeBoundingDirections;

/// Namespace for the Java singleton; the behaviour lives in [`ShapeBoundingDirections`].
pub struct FortyfiveDegreeBoundingDirections;

impl FortyfiveDegreeBoundingDirections {
    /// The one and only instantiation.
    pub const INSTANCE: ShapeBoundingDirections = ShapeBoundingDirections::FortyfiveDegree;
}
