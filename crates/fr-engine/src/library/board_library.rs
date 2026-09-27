//! Port of `core/library/BoardLibrary.java`.
//!
//! Board-dependent method (TODO, implement on the Board):
//! * `isUsed(Padstack padstack, BasicBoard board)`: true if any `DrillItem` on the board uses
//!   the padstack (needs the board item list), or any package pin of `packages` has
//!   `padstack_id == padstack.id` (packages scanned 1..=count, pins in order). Inputs: board
//!   items, `self.packages`, padstack id.

use super::logical_part::LogicalParts;
use super::package::Packages;
use super::padstack::{Padstack, Padstacks};
use crate::ids::PadstackNo;

/// Board library of packages and padstacks (Java `BoardLibrary`).
///
/// Java's no-arg constructor leaves `padstacks`/`packages` null until the loader assigns
/// them; here [`Default`] creates empty ones (with 0 board layers) that the loader replaces.
#[derive(Clone, Debug, Default)]
pub struct BoardLibrary {
    pub padstacks: Padstacks,
    pub packages: Packages,
    /// Gate swap and pin swap information.
    pub logical_parts: LogicalParts,
    /// The subset of padstacks usable as vias in routing (`None` for Java null).
    via_padstacks: Option<Vec<PadstackNo>>,
}

impl BoardLibrary {
    pub fn new(padstacks: Padstacks, packages: Packages) -> Self {
        BoardLibrary {
            padstacks,
            packages,
            logical_parts: LogicalParts::new(),
            via_padstacks: None,
        }
    }

    /// The count of via padstacks usable in routing.
    pub fn via_padstack_count(&self) -> i32 {
        self.via_padstacks.as_ref().map_or(0, |v| v.len() as i32)
    }

    /// The via padstack with the given index, if any.
    pub fn get_via_padstack(&self, no: i32) -> Option<&Padstack> {
        let v = self.via_padstacks.as_ref()?;
        if no < 0 || no as usize >= v.len() {
            return None;
        }
        self.padstacks.get(v[no as usize])
    }

    /// The via padstack with the given name (case-sensitive), if any.
    pub fn get_via_padstack_by_name(&self, name: &str) -> Option<&Padstack> {
        self.via_padstacks
            .as_ref()?
            .iter()
            .filter_map(|&id| self.padstacks.get(id))
            .find(|p| p.name == name)
    }

    /// Ids of the via padstacks usable in routing.
    pub fn get_via_padstacks(&self) -> Vec<PadstackNo> {
        self.via_padstacks.clone().unwrap_or_default()
    }

    /// Sets the via padstacks usable in routing.
    pub fn set_via_padstacks(&mut self, padstacks: Vec<PadstackNo>) {
        self.via_padstacks = Some(padstacks);
    }

    /// Appends a via padstack; false if one with the same name (case-sensitive) exists.
    pub fn add_via_padstack(&mut self, padstack: PadstackNo) -> bool {
        let Some(name) = self.padstacks.get(padstack).map(|p| p.name.clone()) else {
            return false;
        };
        if self.get_via_padstack_by_name(&name).is_some() {
            return false;
        }
        self.via_padstacks
            .get_or_insert_with(Vec::new)
            .push(padstack);
        true
    }

    /// Removes a padstack from the via padstack list; false if not found. (Java NPEs if the
    /// list was never set; here that returns false.)
    pub fn remove_via_padstack(&mut self, padstack: PadstackNo) -> bool {
        let Some(v) = self.via_padstacks.as_mut() else {
            return false;
        };
        match v.iter().position(|&p| p == padstack) {
            Some(i) => {
                v.remove(i);
                true
            }
            None => false,
        }
    }

    /// The via padstack mirrored to the back side of the board, if any. Panics if the via
    /// padstack list was never set and the padstack is not a through via (Java NPE).
    pub fn get_mirrored_via_padstack(&self, via_padstack: PadstackNo) -> Option<PadstackNo> {
        let layer_count = self.padstacks.board_layer_count();
        let vp = self.padstacks.get(via_padstack)?;
        if vp.from_layer() == 0 && vp.to_layer() == layer_count - 1 {
            return Some(via_padstack);
        }
        let new_from_layer = layer_count - vp.to_layer() - 1;
        let new_to_layer = layer_count - vp.from_layer() - 1;
        let list = self
            .via_padstacks
            .as_ref()
            .expect("BoardLibrary.getMirroredViaPadstack: viaPadstacks is null");
        list.iter().copied().find(|&id| {
            self.padstacks
                .get(id)
                .is_some_and(|p| p.from_layer() == new_from_layer && p.to_layer() == new_to_layer)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::structure::{Layer, LayerStructure};
    use fr_geom::{Circle, ConvexShape, IntPoint};

    #[test]
    fn via_padstacks() {
        let ls = LayerStructure::new((0..4).map(|i| Layer::new(format!("L{i}"), true)).collect());
        let mut ps = Padstacks::new(&ls);
        let c = ConvexShape::Circle(Circle::new(IntPoint::new(0, 0), 100));
        let through = ps.add_shape_range(&c, 0, 3);
        let top = ps.add_shape_range(&c, 0, 1);
        let bottom = ps.add_shape_range(&c, 2, 3);
        let inner = ps.add_shape_range(&c, 1, 2);
        let mut lib = BoardLibrary::new(ps, Packages::new());
        assert_eq!(lib.via_padstack_count(), 0);
        assert!(lib.get_via_padstack(0).is_none());
        assert!(lib.add_via_padstack(through));
        assert!(!lib.add_via_padstack(through));
        assert!(lib.add_via_padstack(top));
        assert!(lib.add_via_padstack(bottom));
        assert_eq!(lib.via_padstack_count(), 3);
        assert_eq!(lib.get_via_padstack(1).unwrap().id, top);
        assert_eq!(
            lib.get_via_padstack_by_name("padstack#3").unwrap().id,
            bottom
        );
        assert_eq!(lib.get_mirrored_via_padstack(through), Some(through));
        assert_eq!(lib.get_mirrored_via_padstack(top), Some(bottom));
        assert_eq!(lib.get_mirrored_via_padstack(bottom), Some(top));
        assert_eq!(lib.get_mirrored_via_padstack(inner), None);
        assert!(lib.remove_via_padstack(top));
        assert!(!lib.remove_via_padstack(top));
        assert_eq!(lib.get_via_padstacks(), vec![through, bottom]);
    }
}
