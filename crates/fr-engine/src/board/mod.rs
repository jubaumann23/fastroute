//! The board item model, search trees and `BasicBoard` (porting unit U5).
//!
//! | Module | Java |
//! |---|---|
//! | [`item`] | `board/model/items/*`, `board/model/structure/BoardOutline`, `PolylineTrace` data |
//! | [`item_list`] | `datastructures/UndoableObjects` (item list), `BoardItemRepository` indexes |
//! | [`search_tree`] | `board/searchtree/{ShapeSearchTree, ShapeSearchTree45Degree, ShapeSearchTree90Degree, SearchTreeManager, SearchTreeObject}`, `board/actions/ItemSearchTreesInfo` |
//! | [`complete_shape`] | `completeShape`/`restrainShape` of the three search tree classes |
//! | [`basic_board`] | `board/facade/{BasicBoard, BoardItemRepository, BoardConnectivityQueries}` |
//! | [`connectivity`] | connectivity methods of `Item`, `DrillItem`, `Trace`, `ConductionArea` |
//! | [`trace_ops`] | `PolylineTrace.combine/split/change`, `PolylineTraceNormalization`, `PolylineTraceSearchTreeAdapter` |
//! | [`selection_filter`] | `board/actions/ItemSelectionFilter` |
//! | [`pin_exits`] | pin exit restrictions of `Pin`, `PolylineTrace.check/correct/swapConnectionToPin` |
//! | [`shape_trace_entries`] | `board/searchtree/ShapeTraceEntries` |
//! | [`routing_board`] | `board/facade/{RoutingBoard, RoutingBoardOperations, RoutingBoardSearchFacade, RoutingBoardUndoFacade}`, `autoroute/RoutingFailureLog` (U7) |
//! | [`undo`] | `datastructures/UndoableObjects` (snapshots / undo of the item list, U7) |
//! | [`optimize`] | `board/optimize` (U7) |
//! | [`actions`] | `board/actions/{DrillItemMover, ForcedPadRouter, ForcedViaInserter}` (U7) |
//!
//! See `docs/PORTING.md` ("Rust representation") for the design: items in an arena with
//! [`ItemKey`]s, the list ordered by descending [`ItemId`](crate::ids::ItemId), search trees
//! with per-tree side tables, tombstones for removed items, `Clone` snapshots.

pub mod actions;
pub mod basic_board;
pub mod complete_shape;
pub mod connectivity;
pub(crate) mod epoch;
pub mod item;
pub mod item_list;
pub mod optimize;
pub mod pin_exits;
pub mod routing_board;
pub mod rules_queries;
pub mod search_tree;
pub mod selection_filter;
pub mod shape_trace_entries;
pub mod trace_ops;
pub mod undo;

pub use basic_board::{AutorouteMaintenance, BasicBoard, JavaVariant, MAX_NORMALIZE_ITERATIONS};
pub use item::{
    BoardItemType, BoardOutline, ComponentOutline, ConductionArea, Item, ItemKey, ItemKind, ObstacleArea, ObstacleKind, Pin,
    PolylineTrace, StopConnectionOption, Via,
};
pub use item_list::{ItemListCursor, ItemRepository, ItemSet};
pub use pin_exits::TraceExitRestriction;
pub use routing_board::{AutorouteAttemptState, ForcedTraceEnd, RoutingBoard, RoutingFailureLog, TimeLimitMode, TimeLimitPolicy};
pub use search_tree::{SearchTreeManager, ShapeSearchTree, TreeEntry, TreeKind, TreeObject, DEFAULT_TREE};
pub use selection_filter::{ItemSelectionFilter, SelectableChoices};
pub use shape_trace_entries::ShapeTraceEntries;
pub use trace_ops::LineIdentity;
