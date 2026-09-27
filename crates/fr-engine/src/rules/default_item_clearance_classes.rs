//! Port of `rules/DefaultItemClearanceClasses.java`.

use crate::ids::ClearanceClassNo;

/// Item classes with a default clearance class (Java `DefaultItemClearanceClasses.ItemClass`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ItemClass {
    None,
    Trace,
    Via,
    Pin,
    Smd,
    Area,
}

impl ItemClass {
    /// Java `ItemClass.values()`, in ordinal order.
    pub const VALUES: [ItemClass; 6] = [
        ItemClass::None,
        ItemClass::Trace,
        ItemClass::Via,
        ItemClass::Pin,
        ItemClass::Smd,
        ItemClass::Area,
    ];

    #[inline]
    pub fn ordinal(self) -> usize {
        self as usize
    }
}

/// The default clearance class for each item class.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DefaultItemClearanceClasses {
    clearance_classes: [ClearanceClassNo; 6],
}

impl Default for DefaultItemClearanceClasses {
    /// All item classes except `None` (which stays 0) get class 1.
    fn default() -> Self {
        let mut result = DefaultItemClearanceClasses {
            clearance_classes: [0; 6],
        };
        result.set_all(1);
        result
    }
}

impl DefaultItemClearanceClasses {
    pub fn new() -> Self {
        Self::default()
    }

    /// The default clearance class of `item_class`.
    pub fn get(&self, item_class: ItemClass) -> ClearanceClassNo {
        self.clearance_classes[item_class.ordinal()]
    }

    /// Sets the default clearance class of `item_class`.
    pub fn set(&mut self, item_class: ItemClass, index: ClearanceClassNo) {
        self.clearance_classes[item_class.ordinal()] = index;
    }

    /// Sets all entries except `ItemClass::None` to `index`.
    pub fn set_all(&mut self, index: ClearanceClassNo) {
        for c in &mut self.clearance_classes[1..] {
            *c = index;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults() {
        let mut d = DefaultItemClearanceClasses::new();
        assert_eq!(d.get(ItemClass::None), 0);
        assert_eq!(d.get(ItemClass::Via), 1);
        d.set_all(3);
        assert_eq!(d.get(ItemClass::None), 0);
        assert_eq!(d.get(ItemClass::Area), 3);
        d.set(ItemClass::None, 5);
        assert_eq!(d.get(ItemClass::None), 5);
    }
}
