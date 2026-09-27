//! Identifiers shared across the engine.

use std::fmt;

/// The Java `Item.id`. It is also the ordering key of the board item list:
/// Java `Item.compareTo` sorts by **descending** id.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct ItemId(pub i32);

impl fmt::Debug for ItemId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// Net number as used by Java (`rules.Net.netNumber`), 1-based; 0 means no net.
pub type NetNo = i32;

/// Index into the clearance matrix (Java `clearanceClassIndex`).
pub type ClearanceClassNo = i32;

/// Layer index, 0 = component side.
pub type LayerNo = i32;

/// Java `Component.no`, 1-based; 0 means "not part of a component".
pub type ComponentNo = i32;

/// Java `Padstack.no`, 1-based.
pub type PadstackNo = i32;

/// Java `board.model.structure.FixedState`; the order matters (ordinal compares).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum FixedState {
    #[default]
    Unfixed,
    ShoveFixed,
    UserFixed,
    SystemFixed,
}

/// Java `board.model.structure.AngleRestriction`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AngleRestriction {
    None,
    #[default]
    FortyfiveDegree,
    NinetyDegree,
}
