//! Port of `board/actions/ItemSelectionFilter.java` (and the `isSelectedByFilter` methods of
//! the item classes).

use super::basic_board::BasicBoard;
use super::item::{Item, ItemKind, ObstacleKind};
use super::item_list::ItemSet;

/// Java `ItemSelectionFilter.SelectableChoices` (the order is the ordinal).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SelectableChoices {
    Traces,
    Vias,
    Pins,
    Conduction,
    Keepout,
    ViaKeepout,
    ComponentKeepout,
    BoardOutline,
    Fixed,
    Unfixed,
}

const CHOICE_COUNT: usize = 10;

/// Java `ItemSelectionFilter`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemSelectionFilter {
    values: [bool; CHOICE_COUNT],
}

impl Default for ItemSelectionFilter {
    /// Java `new ItemSelectionFilter()`: all types except the keepouts, conduction areas and
    /// the board outline.
    fn default() -> Self {
        let mut values = [true; CHOICE_COUNT];
        values[SelectableChoices::Keepout as usize] = false;
        values[SelectableChoices::ViaKeepout as usize] = false;
        values[SelectableChoices::ComponentKeepout as usize] = false;
        values[SelectableChoices::Conduction as usize] = false;
        values[SelectableChoices::BoardOutline as usize] = false;
        ItemSelectionFilter { values }
    }
}

impl ItemSelectionFilter {
    /// Java `new ItemSelectionFilter(itemType)`: only `item_type` (plus fixed and unfixed).
    pub fn single(item_type: SelectableChoices) -> Self {
        Self::of(&[item_type])
    }

    /// Java `new ItemSelectionFilter(itemTypes[])`.
    pub fn of(item_types: &[SelectableChoices]) -> Self {
        let mut values = [false; CHOICE_COUNT];
        for t in item_types {
            values[*t as usize] = true;
        }
        values[SelectableChoices::Fixed as usize] = true;
        values[SelectableChoices::Unfixed as usize] = true;
        ItemSelectionFilter { values }
    }

    /// Java `setSelected`.
    pub fn set_selected(&mut self, choice: SelectableChoices, value: bool) {
        self.values[choice as usize] = value;
    }

    /// Java `selectAll`.
    pub fn select_all(&mut self) {
        self.values = [true; CHOICE_COUNT];
    }

    /// Java `deselectAll`.
    pub fn deselect_all(&mut self) {
        self.values = [false; CHOICE_COUNT];
    }

    /// Java `isSelected`.
    pub fn is_selected(&self, choice: SelectableChoices) -> bool {
        self.values[choice as usize]
    }

    /// Java `Item.isSelectedByFilter(filter)`.
    pub fn selects(&self, item: &Item) -> bool {
        let fixed_ok = || {
            if item.is_user_fixed() {
                self.is_selected(SelectableChoices::Fixed)
            } else {
                self.is_selected(SelectableChoices::Unfixed)
            }
        };
        let choice = match &item.kind {
            ItemKind::ComponentOutline(_) => return false,
            ItemKind::Trace(_) => SelectableChoices::Traces,
            ItemKind::Via(_) => SelectableChoices::Vias,
            ItemKind::Pin(_) => SelectableChoices::Pins,
            ItemKind::ConductionArea(_) => SelectableChoices::Conduction,
            ItemKind::ObstacleArea(a) => match a.kind {
                ObstacleKind::Keepout => SelectableChoices::Keepout,
                ObstacleKind::ViaKeepout => SelectableChoices::ViaKeepout,
                ObstacleKind::ComponentKeepout => SelectableChoices::ComponentKeepout,
            },
            ItemKind::BoardOutline(_) => SelectableChoices::BoardOutline,
        };
        fixed_ok() && self.is_selected(choice)
    }

    /// Java `filter(items)`.
    pub fn filter(&self, board: &BasicBoard, items: &ItemSet) -> ItemSet {
        let mut result = ItemSet::new();
        for (id, key) in items.entries() {
            if self.selects(board.item(key)) {
                result.insert(id, key);
            }
        }
        result
    }
}
