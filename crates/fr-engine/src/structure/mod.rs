//! Board structure and state (porting unit U4).
//!
//! Java sources: `board/model/structure/{Layer, LayerStructure, Unit, Component, Components,
//! ShapeEntrySide, ShapeAndEntrySide}`, `board/state/{Communication, ChangedArea}` and
//! `io/CoordinateTransform`. `FixedState` and `AngleRestriction` live in [`crate::ids`];
//! `BoardOutline` is an item (U5).

pub mod changed_area;
pub mod communication;
pub mod component;
pub mod coordinate_transform;
pub mod layer;
pub mod shape_entry_side;
pub mod unit;

pub use changed_area::ChangedArea;
pub use communication::{Communication, SpecctraParserInfo, WriteResolution};
pub use component::{Component, Components};
pub use coordinate_transform::{CoordinateTransform, DsnShapeCoords};
pub use layer::{Layer, LayerStructure};
pub use shape_entry_side::{ShapeAndEntrySide, ShapeEntrySide};
pub use unit::Unit;
