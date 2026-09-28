//! Undo snapshots of the board item list: port of the node bookkeeping of
//! `datastructures/UndoableObjects.java` as used by `BasicBoard.generateSnapshot / undo /
//! popSnapshot` and `RoutingBoardUndoFacade.applyUndoRedoSideEffects`.
//!
//! # Why not a whole-board clone
//!
//! `docs/PORTING.md` plans clone-based snapshots. A plain clone restores more than Java does,
//! and Java's undo leaves observable traces that a clone cannot reproduce:
//! * Java only restores items that were *saved for undo* (`saveForUndo`: `PolylineTrace.change`,
//!   `combine`, `Item.moveBy`, `assignNetNo`, `ShapeTraceEntries.fastCutoutTrace`), inserted or
//!   deleted after the snapshot. In-place changes without `saveForUndo` (e.g. `setFixedState` in
//!   `PolylineTrace.swapConnectionToPin`, conduction area flags) survive an undo, and a deleted
//!   item is restored in the state it had when it was deleted.
//! * The restored items are re-inserted into the search trees (after the cancelled ones were
//!   removed), so the tree structure after an undo differs from the structure at snapshot time.
//!   The autoroute trees walked by `completeShape` depend on it.
//! * Everything outside the item list and the components is not undone (id generator, revision,
//!   trace half width extrema, rules, changed area, failure log, `normalizeSuppressedNetNos`).
//!
//! This module therefore journals exactly what `UndoableObjects` records. The journal is empty
//! (and costs nothing) while no snapshot exists. Saved and deleted item states are stored as
//! clones; restored items get new [`ItemKey`]s (Java restores the saved copies, which are other
//! objects than the cancelled ones). Redo (GUI only) is not ported.

use std::collections::HashMap;
use std::sync::Arc;

use crate::ids::ItemId;
use crate::structure::Components;

use super::basic_board::BasicBoard;
use super::item::{Item, ItemKey};

/// The object of an undo node (Java `UndoableObjectNode.object`).
#[derive(Clone, Debug)]
enum NodeObject {
    /// An item in the board arena.
    Current(ItemKey),
    /// A saved copy (Java `object.clone()` in `saveForUndo`, or the deleted object).
    Saved(Box<Item>),
}

/// Java `UndoableObjectNode`.
#[derive(Clone, Debug)]
struct Node {
    object: NodeObject,
    level: i32,
    undo: Option<usize>,
    redo: Option<usize>,
}

/// The undo bookkeeping of the board item list (see the module docs).
#[derive(Clone, Debug, Default)]
pub struct UndoJournal {
    stack_level: i32,
    /// Node of each listed item that has one (items without entry have an implicit node of
    /// level 0 without undo/redo links).
    map: HashMap<i32, usize>,
    nodes: Vec<Node>,
    /// Java `deletedObjectsStack`.
    deleted: Vec<Vec<usize>>,
    /// The components at each snapshot level (Java `Components.undoList`).
    components: Vec<Arc<Components>>,
}

impl UndoJournal {
    /// Java `stackLevel`.
    pub fn stack_level(&self) -> i32 {
        self.stack_level
    }

    fn push(&mut self, node: Node) -> usize {
        self.nodes.push(node);
        self.nodes.len() - 1
    }

    /// The node of a listed item, created (level 0, no links) if it is implicit.
    fn node_of(&mut self, id: ItemId, key: ItemKey) -> usize {
        if let Some(&n) = self.map.get(&id.0) {
            return n;
        }
        let n = self.push(Node { object: NodeObject::Current(key), level: 0, undo: None, redo: None });
        self.map.insert(id.0, n);
        n
    }

    /// Java `UndoableObjects.insert`.
    pub(crate) fn on_insert(&mut self, id: ItemId, key: ItemKey) {
        if self.stack_level == 0 {
            self.map.remove(&id.0);
            return;
        }
        let n = self.push(Node { object: NodeObject::Current(key), level: self.stack_level, undo: None, redo: None });
        self.map.insert(id.0, n);
    }

    /// Java `UndoableObjects.delete` (called before the item leaves the list; `item` is its state
    /// at that time).
    pub(crate) fn on_delete(&mut self, id: ItemId, key: ItemKey, item: &Item) {
        if self.stack_level == 0 {
            self.map.remove(&id.0);
            return;
        }
        let n = self.node_of(id, key);
        let node_level = self.nodes[n].level;
        if node_level < self.stack_level {
            // the node object itself is restored by undo: keep its state at deletion time
            self.nodes[n].object = NodeObject::Saved(Box::new(item.clone()));
            self.deleted.last_mut().expect("delete list").push(n);
        } else if let Some(u) = self.nodes[n].undo {
            self.deleted.last_mut().expect("delete list").push(u);
        }
        self.map.remove(&id.0);
    }

    /// Java `UndoableObjects.saveForUndo`: must be called before an item existing before the
    /// last snapshot is modified.
    pub(crate) fn on_save(&mut self, id: ItemId, key: ItemKey, item: &Item) {
        if self.stack_level == 0 {
            return;
        }
        let n = self.node_of(id, key);
        if self.nodes[n].level < self.stack_level {
            let old = Node {
                object: NodeObject::Saved(Box::new(item.copy_with_id(item.id()))),
                level: self.nodes[n].level,
                undo: self.nodes[n].undo,
                redo: Some(n),
            };
            let o = self.push(old);
            self.nodes[n].undo = Some(o);
            self.nodes[n].level = self.stack_level;
        }
    }

    fn collect_garbage(&mut self) {
        if self.stack_level == 0 {
            // at level 0 no node can be restored any more: the links have no effect
            self.map.clear();
            self.nodes.clear();
            self.deleted.clear();
            self.components.clear();
        }
    }
}

impl BasicBoard {
    /// The undo journal.
    pub fn undo_journal(&self) -> &UndoJournal {
        &self.undo
    }

    /// Java `generateSnapshot()`: makes the current item list and components restorable by
    /// [`Self::undo`].
    pub fn generate_snapshot(&mut self) {
        self.undo.deleted.push(Vec::new());
        self.undo.stack_level += 1;
        self.undo.components.push(self.components.clone());
    }

    /// Java `popSnapshot()`: removes the top snapshot, keeping the current state. Returns false
    /// if there is no snapshot.
    pub fn pop_snapshot(&mut self) -> bool {
        let j = &mut self.undo;
        if j.stack_level == 0 {
            return false;
        }
        let stack_level = j.stack_level;
        let current: Vec<usize> = j.map.values().copied().collect();
        for n in current {
            if j.nodes[n].level == stack_level - 1 {
                if let Some(r) = j.nodes[n].redo {
                    if j.nodes[r].level == stack_level {
                        j.nodes[r].undo = j.nodes[n].undo;
                        if let Some(u) = j.nodes[n].undo {
                            j.nodes[u].redo = Some(r);
                        }
                    }
                }
            } else if j.nodes[n].level >= stack_level {
                j.nodes[n].level -= 1;
            }
        }
        let size = j.deleted.len();
        if size >= 2 {
            let from = j.deleted[size - 1].clone();
            for n in from {
                if j.nodes[n].level < stack_level - 1 {
                    j.deleted[size - 2].push(n);
                } else if let Some(u) = j.nodes[n].undo {
                    j.deleted[size - 2].push(u);
                }
            }
        }
        j.deleted.pop();
        j.components.pop();
        j.stack_level -= 1;
        j.collect_garbage();
        true
    }

    /// Java `undo(null)` (`BasicBoard.undo` / `RoutingBoardUndoFacade.undo`): restores the item
    /// list and the components of the last snapshot. Returns false if there is no snapshot.
    /// Returns the ids of the changed nets in `changed_nets` if given (Java `changedNets`).
    pub fn undo(&mut self, mut changed_nets: Option<&mut std::collections::BTreeSet<i32>>) -> bool {
        if self.undo.stack_level == 0 {
            return false;
        }
        let stack_level = self.undo.stack_level;
        let mut cancelled: Vec<ItemKey> = Vec::new();
        let mut restored: Vec<usize> = Vec::new();
        // current nodes of the top level, in list order
        for key in self.items.keys() {
            let id = self.item(key).id();
            let Some(&n) = self.undo.map.get(&id.0) else { continue };
            if self.undo.nodes[n].level != stack_level {
                continue;
            }
            if let Some(u) = self.undo.nodes[n].undo {
                self.undo.nodes[u].redo = Some(n);
                self.undo.map.insert(id.0, u);
                restored.push(u);
            } else {
                // created after the snapshot: skipped by readObject until disableRedo removes it
                self.undo.map.remove(&id.0);
            }
            cancelled.push(key);
        }
        let delete_list = std::mem::take(&mut self.undo.deleted[(stack_level - 1) as usize]);
        for &n in &delete_list {
            restored.push(n);
        }
        self.undo.deleted.pop();
        let components = self.undo.components.pop().expect("components snapshot");
        self.undo.stack_level -= 1;
        // list: cancel, then restore (the list is ordered by id, the order does not matter)
        for &key in &cancelled {
            self.items.list_remove(key);
        }
        let mut restored_keys: Vec<ItemKey> = Vec::with_capacity(restored.len());
        for &n in &restored {
            let key = match std::mem::replace(&mut self.undo.nodes[n].object, NodeObject::Current(ItemKey { index: u32::MAX, generation: 0 })) {
                NodeObject::Saved(item) => {
                    let mut item = *item;
                    item.on_board = false;
                    self.items.alloc(item)
                }
                NodeObject::Current(key) => key,
            };
            self.undo.nodes[n].object = NodeObject::Current(key);
            let id = self.item(key).id();
            self.undo.map.insert(id.0, n);
            self.items.list_insert(key);
            restored_keys.push(key);
        }
        self.components = components;
        // side effects (Java applyUndoRedoSideEffects)
        for &key in &cancelled {
            self.tree_remove(key);
            if let Some(nets) = changed_nets.as_deref_mut() {
                nets.extend(self.item(key).net_numbers().iter().copied());
            }
        }
        for &key in &restored_keys {
            self.clear_search_tree_entries(key);
            self.tree_insert(key);
            if let Some(nets) = changed_nets.as_deref_mut() {
                nets.extend(self.item(key).net_numbers().iter().copied());
            }
        }
        self.invalidate_edge_pin_net_cache();
        self.undo.collect_garbage();
        true
    }

    /// Journals `saveForUndo` of an item on the board list.
    pub(crate) fn save_for_undo(&mut self, key: ItemKey) {
        if self.undo.stack_level == 0 || !self.items.is_listed(key) {
            // (Java: "object node not found" for items outside the list)
            return;
        }
        let item = self.items.get(key);
        let id = item.id();
        // split borrows: the journal is a separate field
        let journal = &mut self.undo;
        journal.on_save(id, key, item);
    }

    /// Journals `UndoableObjects.insert` of an item just put into the list.
    pub(crate) fn journal_insert(&mut self, key: ItemKey) {
        if self.undo.stack_level == 0 && self.undo.map.is_empty() {
            return;
        }
        let id = self.items.get(key).id();
        self.undo.on_insert(id, key);
    }

    /// Journals `UndoableObjects.delete` of an item about to leave the list.
    pub(crate) fn journal_delete(&mut self, key: ItemKey) {
        if (self.undo.stack_level == 0 && self.undo.map.is_empty()) || !self.items.is_listed(key) {
            // (Java UndoableObjects.delete returns false for objects outside the list)
            return;
        }
        let item = self.items.get(key);
        let id = item.id();
        let journal = &mut self.undo;
        journal.on_delete(id, key, item);
    }
}
