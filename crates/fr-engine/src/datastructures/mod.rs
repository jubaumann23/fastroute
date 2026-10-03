//! Port of `app.freerouting.datastructures` (the parts the headless engine needs).
//!
//! | Module | Java |
//! |---|---|
//! | [`min_area_tree`] | `ShapeTree`, `MinAreaTree` (+ the traversal stack of `ArrayStack`) |
//! | [`time_limit`] | `TimeLimit`, `core.StoppableThread` stop requests |
//! | [`identifier_type`] | `IdentifierType` |
//! | [`indent_file_writer`] | `IndentFileWriter` |
//! | [`id_generator`] | `IdGenerator`, `board.actions.ItemIdGenerator` |
//! | [`planar_delaunay_triangulation`] | `PlanarDelaunayTriangulation` |
//!
//! Not ported: `UndoableObjects` (the board keeps its own item storage and snapshots),
//! `Observers` (GUI only), `BigIntAux`/`Signum`/`Stoppable` (in `fr-geom`), `ArrayStack` (a plain
//! `Vec` is used as the reusable traversal stack; Java's 40 000 element depth limit, which throws
//! `IllegalStateException`, is not reproduced).

pub mod cow_vec;
pub mod id_generator;
pub mod identifier_type;
pub mod indent_file_writer;
mod leaf_grid;
pub mod min_area_tree;
pub mod planar_delaunay_triangulation;
pub mod time_limit;

pub use id_generator::{IdGenerator, ItemIdGenerator};
pub use identifier_type::IdentifierType;
pub use indent_file_writer::IndentFileWriter;
pub use min_area_tree::{LeafId, LeafRef, MinAreaTree, ShapeTree, TreeCursor, TreeStatistics};
pub use planar_delaunay_triangulation::{PlanarDelaunayTriangulation, ResultEdge, TriangulationPoint};
pub use time_limit::{StopToken, TimeLimit};
