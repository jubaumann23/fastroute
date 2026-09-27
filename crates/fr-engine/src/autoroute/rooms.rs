//! Minimal data types of `autoroute/expansion/{IncompleteFreeSpaceExpansionRoom,
//! CompleteFreeSpaceExpansionRoom}` used by the board search trees.
//!
//! The autorouter port (U8) extends these (doors, target doors, arenas). Design:
//! * Complete free space rooms are leaf objects of the same [`MinAreaTree`] as the board items
//!   (Java inserts them into the autoroute search tree, and the 45/90 degree `completeShape`
//!   walks see items and rooms in tree order). The tree identifies a room by [`RoomKey`] (an
//!   index into the autoroute engine's room arena, chosen by the caller) and keeps a copy of
//!   the room's shape, layer and Java id in its side table, so tree queries need no access to
//!   the engine. Rooms must be removed from the tree when the engine clears them.
//! * [`TreeObject`](crate::board::TreeObject) orders rooms before items (Java
//!   `Item.compareTo(room) == 1`, `CompleteFreeSpaceExpansionRoom.compareTo(item) == -1`) and
//!   rooms by descending id (`other.id - this.id`, wrapping).
//!
//! [`MinAreaTree`]: crate::datastructures::MinAreaTree

use fr_geom::TileShape;

use crate::ids::LayerNo;

/// Handle of a complete free space expansion room (chosen by the autoroute engine; unique while
/// the room is stored in a tree).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RoomKey(pub u32);

/// Java `IncompleteFreeSpaceExpansionRoom`: an expansion room whose shape is not yet completely
/// calculated. `shape == None` means the whole plane.
#[derive(Clone, Debug)]
pub struct IncompleteFreeSpaceExpansionRoom {
    pub shape: Option<TileShape>,
    pub layer: LayerNo,
    /// A shape which should be contained in the completed shape.
    pub contained_shape: Option<TileShape>,
}

impl IncompleteFreeSpaceExpansionRoom {
    pub fn new(shape: Option<TileShape>, layer: LayerNo, contained_shape: Option<TileShape>) -> Self {
        IncompleteFreeSpaceExpansionRoom { shape, layer, contained_shape }
    }

    /// Java `getId()`: `31 * getShape().getId() + getLayer()` (int arithmetic). Panics for a
    /// room without shape (Java NullPointerException).
    pub fn get_id(&self) -> i32 {
        let shape = self.shape.as_ref().expect("IncompleteFreeSpaceExpansionRoom.getId: shape is null");
        31i32.wrapping_mul(shape.get_id()).wrapping_add(self.layer)
    }
}

/// Java `CompleteFreeSpaceExpansionRoom` (shape, layer and id; doors are added by U8).
#[derive(Clone, Debug)]
pub struct CompleteFreeSpaceExpansionRoom {
    pub shape: TileShape,
    pub layer: LayerNo,
    /// Identification number for implementing the Comparable interface.
    pub id: i32,
    /// Java `roomIsNetDependent`.
    pub net_dependent: bool,
}

impl CompleteFreeSpaceExpansionRoom {
    pub fn new(shape: TileShape, layer: LayerNo, id: i32) -> Self {
        CompleteFreeSpaceExpansionRoom { shape, layer, id, net_dependent: false }
    }
}
