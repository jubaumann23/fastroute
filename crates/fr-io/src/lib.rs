//! DSN -> board construction (porting unit U11, loader half).
//!
//! Faithful port of the board-building part of the Java DSN reader
//! (`io.specctra.DsnReader.readBoard` and `io.specctra.parser.{Structure, Library, Package,
//! Network, Wiring, PartLibrary, Shape, ...}`), working on the typed model
//! [`fr_dsn::model::Dsn`] instead of the Java streaming parser. Everything except the actual
//! item insertion is done here: the result, [`LoadedDesign`], carries the layer structure,
//! rules, library, components and the list of [`InsertRequest`]s — the arguments of every
//! `BasicBoard.insert*` call in Java call order (which determines the item ids).
//!
//! # Board-builder contract
//!
//! To reproduce the Java board, the board step must:
//!
//! 1. Create the board with `bounding_box`, `layer_structure`, `rules`, `library`,
//!    `components` (flip style already set), `communication` (fresh id generator), apply
//!    `logical_part_assignments`, then replay `requests` in order. The first request is the
//!    board outline (`BasicBoard` constructor, id 1).
//! 2. Assign ids exactly like Java: every constructed item takes the next id. Constructions
//!    without insertion and early returns are documented on [`InsertRequest`]:
//!    component outlines with an unbounded area are skipped (no id); traces with fewer than 2
//!    corners are skipped (no id), traces with equal first/last corner and `fixed <
//!    USER_FIXED` consume an id but are not inserted; vias run Java's `Wiring.viaExists`
//!    check first (duplicate: no call, no id).
//! 3. Keep `rules.nets` untouched while replaying (all nets exist before the first item that
//!    references them); pins, keepouts and outlines compute their shapes from the component
//!    (see [`components`] for pure versions of `Pin.getShape` etc.).
//! 4. After a `Trace` request with `try_correct_net`, run `Wiring.tryCorrectNet` on the
//!    inserted trace (assign the net of the first single-net contact at either end).
//!    Via insertion runs `splitTraces` for each net on the layers `from..to` (exclusive).
//! 5. After the last request, run `normalizeAllTraces` (Java catches and ignores exceptions
//!    there), then apply [`LoadedDesign::plane_adjustment`] if present
//!    (`Net.setContainsPlane(true)` for `plane_nets`, raise the fixed state of the listed
//!    conduction areas to `USER_FIXED`).
//!    [`build_board`] implements steps 1-5 on `fr_engine::board::BasicBoard`.
//! 6. `HeadlessBoardManager` post-load processing (router settings, copper-to-edge and hole
//!    clearance overrides, plane nets/plane-as-obstacle/clearance-tolerance overrides,
//!    `expandBoundingBoxToIncludeAllItems`, `reduceNetsOfRouteItems`, validations) follows;
//!    it needs the live board; see [`post_load`] ([`post_load::load_from_specctra_dsn`],
//!    [`post_load::prepare_for_routing`]).
//!
//! Session files: [`ses_writer::write_ses`] (Java `SesWriter`, byte-identical) and
//! [`ses_reader::read_ses`] (Java `SesReader`).
//!
//! # Scope order
//!
//! Java reads the file in one pass; [`order::SourceOrder`] recovers the few order effects
//! the typed model loses. [`load_bytes`] extracts them from the raw file, [`load`] assumes the
//! usual layout (see [`order`]).

pub mod board;
pub mod components;
pub mod dump;
pub mod error;
pub mod library;
pub mod loader;
pub mod netlist;
pub mod network;
pub mod order;
pub mod part_library;
pub mod plane;
pub mod post_load;
pub mod requests;
pub mod ses_reader;
pub mod ses_writer;
pub mod shapes;
pub mod structure;
pub mod wiring;

pub use board::build_board;
pub use error::LoadError;
pub use loader::{load, load_bytes, load_with_order, KeepoutKind, LoadedDesign};
pub use order::SourceOrder;
pub use plane::PlaneAdjustment;
pub use requests::{AreaRequest, InsertRequest};
