//! Board rules (porting unit U2).
//!
//! Java sources: `rules/{ClearanceMatrix, BoardRules, NetClass, NetClasses, Net, Nets,
//! ViaInfo, ViaInfos, ViaRule, DefaultItemClearanceClasses}`.
//!
//! Shared mutable Java objects become stable ids into arenas owned by [`BoardRules`]:
//! [`NetClassId`] (in [`NetClasses`]), [`ViaInfoId`] (in [`ViaInfos`]), [`ViaRuleId`] (in
//! [`ViaRules`]). Padstack references are [`PadstackNo`](crate::ids::PadstackNo)s; methods
//! needing padstack geometry take the library's [`Padstacks`](crate::library::Padstacks).
//! Methods that scan board items are documented as TODOs in [`board_rules`] and [`net`].

pub mod board_rules;
pub mod clearance_matrix;
pub mod default_item_clearance_classes;
pub mod net;
pub mod net_class;
pub mod via;

pub use board_rules::BoardRules;
pub use clearance_matrix::{ClearanceMatrix, CLEARANCE_SAFETY_MARGIN};
pub use default_item_clearance_classes::{DefaultItemClearanceClasses, ItemClass};
pub use net::{Net, Nets};
pub use net_class::{NetClass, NetClassId, NetClasses};
pub use via::{ViaInfo, ViaInfoId, ViaInfos, ViaRule, ViaRuleId, ViaRules};
