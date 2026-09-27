//! Faithful Rust port of the Freerouting planar geometry package
//! (`app.freerouting.geometry.planar`) plus the few helpers it needs from
//! `app.freerouting.datastructures` (`Signum`, `BigIntAux`, `Stoppable`).
//!
//! # Mapping conventions
//!
//! * Java `int`/`long`/`double` are `i32`/`i64`/`f64`. `Math.round` is
//!   [`java_compat::math_round`] (ties toward +infinity), `(int)` casts of doubles are Rust `as`
//!   casts (both saturate, NaN -> 0). Integer arithmetic on box/octagon coordinates and hash codes
//!   wraps like Java; elsewhere normal (debug-checked) arithmetic is used.
//! * `BigInteger` is `num_bigint::BigInt`. The general case of `Line::intersection` uses `i128`,
//!   which is provably exact there (see the comment in that function).
//! * Closed Java class hierarchies are enums: [`Point`], [`Vector`], [`Direction`],
//!   [`TileShape`], [`RegularTileShape`], [`ConvexShape`], [`PolylineShape`], [`Shape`],
//!   [`Area`], [`ShapeBoundingDirections`]. Where subclasses inherit implementation from an
//!   abstract class, that code lives in a trait with default methods ([`PolylineShapeImpl`],
//!   [`TileShapeImpl`]); import [`prelude`] to call these on the leaf types directly.
//! * Java double dispatch (`a.intersection(b)` calling `b.intersection(<static type of a>)`) is a
//!   `match` on the variant pair, keeping the Java call order (it matters, e.g. for the line
//!   order of simplex intersections).
//! * Overloads are distinguished by a suffix naming the parameter type:
//!   `intersects(&Shape)`, `intersects_int_box(&IntBox)`, `contains(&Point)`,
//!   `contains_float(&FloatPoint)`, `cutout(&TileShape)`, `cutout_polyline(&Polyline)`, ...
//! * Java `null` results become `Option`; Java casts that would throw (`(IntPoint) p`) and
//!   dereferences of `null` panic.
//! * Lazily computed transient caches (`precalculated*` fields) use `OnceLock`, so all types are
//!   `Send + Sync`. Types holding arrays (`Simplex`, `Polyline`, `PolygonShape`) are cheap to
//!   clone (reference counted), like Java references.
//!
//! Each module names the Java source it was ported from.

pub mod area;
pub mod big_int_aux;
pub mod big_int_direction;
pub mod circle;
pub mod convex_shape;
pub mod direction;
pub mod ellipse;
pub mod float_line;
pub mod float_point;
pub mod fortyfive_degree_bounding_directions;
pub mod fortyfive_degree_direction;
pub mod int_box;
pub mod int_direction;
pub mod int_octagon;
pub mod int_point;
pub mod int_vector;
pub mod java_compat;
pub mod java_sort;
pub mod limits;
pub mod line;
pub mod line_segment;
pub mod orthogonal_bounding_directions;
pub mod point;
pub mod polygon;
pub mod polygon_shape;
pub mod polyline;
pub mod polyline_area;
pub mod polyline_shape;
pub mod rational_point;
pub mod rational_vector;
pub mod regular_tile_shape;
pub mod shape;
pub mod shape_bounding_directions;
pub mod side;
pub mod signum;
pub mod simplex;
pub mod stoppable;
pub mod tile_shape;
pub mod vector;

pub use area::Area;
pub use big_int_direction::BigIntDirection;
pub use circle::Circle;
pub use convex_shape::ConvexShape;
pub use direction::Direction;
pub use ellipse::Ellipse;
pub use float_line::FloatLine;
pub use float_point::FloatPoint;
pub use fortyfive_degree_bounding_directions::FortyfiveDegreeBoundingDirections;
pub use fortyfive_degree_direction::FortyfiveDegreeDirection;
pub use int_box::IntBox;
pub use int_direction::IntDirection;
pub use int_octagon::IntOctagon;
pub use int_point::IntPoint;
pub use int_vector::IntVector;
pub use line::Line;
pub use line_segment::LineSegment;
pub use orthogonal_bounding_directions::OrthogonalBoundingDirections;
pub use point::Point;
pub use polygon::Polygon;
pub use polygon_shape::PolygonShape;
pub use polyline::Polyline;
pub use polyline_area::PolylineArea;
pub use polyline_shape::{PolylineShape, PolylineShapeImpl};
pub use rational_point::RationalPoint;
pub use rational_vector::RationalVector;
pub use regular_tile_shape::RegularTileShape;
pub use shape::Shape;
pub use shape_bounding_directions::ShapeBoundingDirections;
pub use side::Side;
pub use signum::Signum;
pub use simplex::Simplex;
pub use stoppable::Stoppable;
pub use tile_shape::{TileShape, TileShapeImpl};
pub use vector::Vector;

/// Glob-import this to call the inherited (trait) methods directly on `IntBox`, `IntOctagon`,
/// `Simplex` and `PolygonShape`.
pub mod prelude {
    pub use crate::polyline_shape::PolylineShapeImpl;
    pub use crate::tile_shape::TileShapeImpl;
}
