//! Port of `board/actions` (the shove helpers of porting unit U7).
//!
//! | Module | Java |
//! |---|---|
//! | [`drill_item_mover`] | `DrillItemMover`, `DrillItem.moveBy` |
//! | [`forced_pad_router`] | `ForcedPadRouter` |
//! | [`forced_via_inserter`] | `ForcedViaInserter` |
//!
//! `MoveComponent` (GUI only) is not ported; `ItemSelectionFilter` and `ItemSearchTreesInfo`
//! are part of [`super::selection_filter`] and [`super::search_tree`], `ItemIdGenerator` of
//! [`crate::datastructures`].

pub mod drill_item_mover;
pub mod forced_pad_router;
pub mod forced_via_inserter;

pub use forced_pad_router::CheckDrillResult;
