//! The board-dependent methods of the rules and the library (documented as TODOs in
//! [`crate::rules::board_rules`], [`crate::rules::net`] and [`crate::library::board_library`]):
//! Java `Net.getTerminalItems/getPins/getItems/getTraceLength/getViaCount`,
//! `BoardLibrary.isUsed`, `BoardRules.changeClearanceClassIndex/removeClearanceClass`.

use crate::ids::{ClearanceClassNo, NetNo, PadstackNo};

use super::basic_board::BasicBoard;
use super::item::ItemKey;

impl BasicBoard {
    /// Java `Net.getTerminalItems()`: connectable, not routable items of the net (list order).
    pub fn net_terminal_items(&self, net_number: NetNo) -> Vec<ItemKey> {
        self.items
            .net_items(net_number)
            .filter(|k| {
                let item = self.item(*k);
                item.is_connectable_class() && !item.is_routable()
            })
            .collect()
    }

    /// Java `Net.getPins()`.
    pub fn net_pins(&self, net_number: NetNo) -> Vec<ItemKey> {
        self.items.net_items(net_number).filter(|k| self.item(*k).is_pin()).collect()
    }

    /// Java `Net.getItems()`.
    pub fn net_items(&self, net_number: NetNo) -> Vec<ItemKey> {
        self.items.net_items(net_number).collect()
    }

    /// Java `Net.getTraceLength()` (summed in list order).
    pub fn net_trace_length(&self, net_number: NetNo) -> f64 {
        let mut result = 0.0;
        for k in self.get_connectable_items(net_number) {
            if let Some(t) = self.item(k).as_trace() {
                result += t.length();
            }
        }
        result
    }

    /// Java `Net.getViaCount()`.
    pub fn net_via_count(&self, net_number: NetNo) -> i32 {
        self.get_connectable_items(net_number).iter().filter(|k| self.item(**k).is_via()).count() as i32
    }

    /// Java `BoardLibrary.isUsed(padstack, board)`.
    pub fn padstack_is_used(&self, padstack: PadstackNo) -> bool {
        for k in self.items.iter() {
            let item = self.item(k);
            if item.is_drill_item() {
                if let Some(p) = item.padstack(self) {
                    if p.id == padstack {
                        return true;
                    }
                }
            }
        }
        for package in self.library.packages.iter() {
            if package.pins().iter().any(|pin| pin.padstack_id == padstack) {
                return true;
            }
        }
        false
    }

    /// Java `BoardRules.changeClearanceClassIndex(fromIndex, toIndex, board.getItems())`.
    pub fn change_clearance_class_of_items_and_rules(&mut self, from_index: ClearanceClassNo, to_index: ClearanceClassNo) {
        for k in self.get_items() {
            if self.item(k).clearance_class() == from_index {
                self.set_clearance_class_index(k, to_index);
            }
        }
        self.rules_mut().change_clearance_class_index(from_index, to_index);
    }

    /// Java `BoardRules.removeClearanceClass(index, board.getItems())`: false if an item, a net
    /// class or a via info still uses the class.
    pub fn remove_clearance_class(&mut self, index: ClearanceClassNo) -> bool {
        if self.items.iter().any(|k| self.item(k).clearance_class() == index) {
            return false;
        }
        if !self.rules_mut().remove_clearance_class(index) {
            return false;
        }
        for k in self.get_items() {
            let cl = self.item(k).clearance_class();
            if cl > index {
                self.set_clearance_class_index(k, cl - 1);
            }
        }
        true
    }
}
