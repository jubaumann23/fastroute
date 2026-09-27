//! Board library (porting unit U3).
//!
//! Java sources: `core/library/{Padstack, Padstacks, Package, Packages, BoardLibrary,
//! LogicalPart, LogicalParts}`. Object references become 1-based ids
//! ([`PadstackNo`](crate::ids::PadstackNo), [`PackageNo`], [`LogicalPartNo`]); the back
//! references to the owning lists (only used for GUI printing) are dropped.

pub mod board_library;
pub mod logical_part;
pub mod package;
pub mod padstack;

/// Java `Package.id`, 1-based.
pub type PackageNo = i32;

/// Java `LogicalPart.id`, 1-based.
pub type LogicalPartNo = i32;

pub use board_library::BoardLibrary;
pub use logical_part::{LogicalPart, LogicalParts, PartPin};
pub use package::{Keepout, Package, PackagePin, Packages};
pub use padstack::{Padstack, Padstacks};
