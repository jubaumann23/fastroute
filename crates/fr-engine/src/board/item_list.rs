//! The board item storage: Java `UndoableObjects` (the item list, a `ConcurrentSkipListMap`
//! ordered by `Item.compareTo`, i.e. by **descending id**) plus the arena owning the items and
//! indexes the Java code computes by scanning the list.
//!
//! * Items live in a generational arena ([`ItemKey`]). Removing an item from the list keeps it
//!   readable as a tombstone until [`ItemRepository::compact`] (Java code keeps using removed
//!   items, e.g. the split pieces in `PolylineTrace.split`).
//! * [`ItemListCursor`] reproduces the weakly consistent `ConcurrentSkipListMap` iterator used
//!   through `startReadObject`/`readObject`: the next entry is prefetched when the previous one
//!   is returned, so an item removed after it was prefetched is still returned once; items
//!   inserted during the iteration have larger ids and are never visited.
//! * The undo levels of `UndoableObjects` are not ported (the board is snapshotted by cloning).
//! * Indexes: id -> key, net -> items of the net, component -> items of the component. They
//!   contain exactly the items of the list and iterate in list order (descending id).

use std::cmp::Reverse;
use std::collections::{BTreeMap, HashMap};
use std::ops::Bound;

use crate::ids::{ComponentNo, ItemId, NetNo};

use super::item::{Item, ItemKey};

#[derive(Clone, Debug)]
struct Slot {
    generation: u32,
    item: Option<Item>,
}

/// Ordered map `descending id -> key` (Java `TreeSet<Item>` / the item list order).
pub type IdOrderedMap = BTreeMap<Reverse<i32>, ItemKey>;

/// Java `TreeSet<Item>`: a set of items ordered by descending id. Adding an item whose id is
/// already contained keeps the old entry (Java `TreeSet.add` returns false).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ItemSet {
    map: IdOrderedMap,
}

impl ItemSet {
    pub fn new() -> Self {
        ItemSet { map: BTreeMap::new() }
    }

    /// Java `add`: true if the set did not contain an item with this id.
    pub fn insert(&mut self, id: ItemId, key: ItemKey) -> bool {
        match self.map.entry(Reverse(id.0)) {
            std::collections::btree_map::Entry::Occupied(_) => false,
            std::collections::btree_map::Entry::Vacant(v) => {
                v.insert(key);
                true
            }
        }
    }

    /// Java `contains` (compares by id).
    pub fn contains(&self, id: ItemId) -> bool {
        self.map.contains_key(&Reverse(id.0))
    }

    /// Java `remove` (by id).
    pub fn remove(&mut self, id: ItemId) -> bool {
        self.map.remove(&Reverse(id.0)).is_some()
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// The keys in set order (descending id).
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = ItemKey> + '_ {
        self.map.values().copied()
    }

    /// `(id, key)` pairs in set order.
    pub fn entries(&self) -> impl DoubleEndedIterator<Item = (ItemId, ItemKey)> + '_ {
        self.map.iter().map(|(r, k)| (ItemId(r.0), *k))
    }

    /// The ids in set order.
    pub fn ids(&self) -> impl DoubleEndedIterator<Item = ItemId> + '_ {
        self.map.keys().map(|r| ItemId(r.0))
    }

    /// The first element (largest id).
    pub fn first(&self) -> Option<ItemKey> {
        self.map.values().next().copied()
    }

    /// Java `addAll`.
    pub fn extend_from(&mut self, other: &ItemSet) {
        for (id, key) in other.entries() {
            self.insert(id, key);
        }
    }

    /// Java `removeAll`.
    pub fn remove_all(&mut self, other: &ItemSet) {
        for id in other.ids() {
            self.remove(id);
        }
    }

    /// Java `Collections.disjoint`.
    pub fn is_disjoint(&self, other: &ItemSet) -> bool {
        other.ids().all(|id| !self.contains(id))
    }

    /// Removes the elements for which `f` returns false.
    pub fn retain(&mut self, mut f: impl FnMut(ItemKey) -> bool) {
        self.map.retain(|_, k| f(*k));
    }
}

/// Cursor over the item list with `ConcurrentSkipListMap` iterator semantics (see module docs).
#[derive(Clone, Debug)]
pub struct ItemListCursor {
    next: Option<(i32, ItemKey)>,
}

/// Arena + item list + indexes (Java `UndoableObjects` + `BoardItemRepository` scans).
#[derive(Clone, Debug, Default)]
pub struct ItemRepository {
    slots: Vec<Slot>,
    free: Vec<u32>,
    list: IdOrderedMap,
    by_id: HashMap<i32, ItemKey>,
    by_net: HashMap<NetNo, IdOrderedMap>,
    by_component: HashMap<ComponentNo, IdOrderedMap>,
}

impl ItemRepository {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an item to the arena (not to the list) and returns its key.
    pub fn alloc(&mut self, item: Item) -> ItemKey {
        if let Some(index) = self.free.pop() {
            let slot = &mut self.slots[index as usize];
            slot.item = Some(item);
            ItemKey { index, generation: slot.generation }
        } else {
            let index = self.slots.len() as u32;
            self.slots.push(Slot { generation: 0, item: Some(item) });
            ItemKey { index, generation: 0 }
        }
    }

    /// Read access to an item (also to removed items that were not yet compacted).
    #[inline]
    pub fn get(&self, key: ItemKey) -> &Item {
        self.try_get(key).expect("ItemRepository: stale item key")
    }

    #[inline]
    pub fn try_get(&self, key: ItemKey) -> Option<&Item> {
        let slot = self.slots.get(key.index as usize)?;
        if slot.generation != key.generation {
            return None;
        }
        slot.item.as_ref()
    }

    /// Mutable access. Do not change the id or the nets of a listed item through this (use
    /// [`Self::set_nets`]), the indexes would get inconsistent.
    #[inline]
    pub(crate) fn get_mut(&mut self, key: ItemKey) -> &mut Item {
        let slot = &mut self.slots[key.index as usize];
        assert!(slot.generation == key.generation, "ItemRepository: stale item key");
        slot.item.as_mut().expect("ItemRepository: freed item")
    }

    /// Number of arena slots (upper bound of `ItemKey::index`).
    pub fn slot_count(&self) -> usize {
        self.slots.len()
    }

    // ------------------------------------------------------------------------------------------
    // list

    /// Java `UndoableObjects.insert`: puts the item into the list (replacing an entry with the
    /// same id, like `ConcurrentSkipListMap.put`).
    pub fn list_insert(&mut self, key: ItemKey) {
        let (id, nets, component) = {
            let item = self.get(key);
            (item.id().0, item.net_numbers().to_vec(), item.component_no())
        };
        if let Some(old) = self.list.insert(Reverse(id), key) {
            if old != key {
                self.unindex(old, id);
            }
        }
        self.by_id.insert(id, key);
        for n in nets {
            if n > 0 {
                self.by_net.entry(n).or_default().insert(Reverse(id), key);
            }
        }
        self.by_component.entry(component).or_default().insert(Reverse(id), key);
    }

    fn unindex(&mut self, key: ItemKey, id: i32) {
        let (nets, component) = {
            let item = self.get(key);
            (item.net_numbers().to_vec(), item.component_no())
        };
        for n in nets {
            if let Some(m) = self.by_net.get_mut(&n) {
                if m.get(&Reverse(id)) == Some(&key) {
                    m.remove(&Reverse(id));
                }
            }
        }
        if let Some(m) = self.by_component.get_mut(&component) {
            if m.get(&Reverse(id)) == Some(&key) {
                m.remove(&Reverse(id));
            }
        }
    }

    /// Java `UndoableObjects.delete`: removes the entry with the item's id from the list.
    /// Returns false if no such entry exists.
    pub fn list_remove(&mut self, key: ItemKey) -> bool {
        let id = self.get(key).id().0;
        let Some(listed) = self.list.remove(&Reverse(id)) else {
            return false;
        };
        self.unindex(listed, id);
        if self.by_id.get(&id) == Some(&listed) {
            self.by_id.remove(&id);
        }
        true
    }

    /// True if the item (this key) is in the list.
    pub fn is_listed(&self, key: ItemKey) -> bool {
        match self.try_get(key) {
            Some(item) => self.list.get(&Reverse(item.id().0)) == Some(&key),
            None => false,
        }
    }

    /// Changes the nets of an item, keeping the indexes consistent.
    pub(crate) fn set_nets(&mut self, key: ItemKey, nets: Vec<NetNo>) {
        let listed = self.is_listed(key);
        let id = self.get(key).id().0;
        if listed {
            self.unindex(key, id);
        }
        self.get_mut(key).net_numbers = nets;
        if listed {
            self.list_insert(key);
        }
    }

    /// Changes the component of an item, keeping the indexes consistent.
    pub(crate) fn set_component(&mut self, key: ItemKey, component: ComponentNo) {
        let listed = self.is_listed(key);
        let id = self.get(key).id().0;
        if listed {
            self.unindex(key, id);
        }
        self.get_mut(key).component_no = component;
        if listed {
            self.list_insert(key);
        }
    }

    /// Number of listed items.
    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// The listed items in list order (Java `getItems()`, a snapshot).
    pub fn keys(&self) -> Vec<ItemKey> {
        self.list.values().copied().collect()
    }

    /// Iterator over the listed items in list order (no concurrent modification possible).
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = ItemKey> + '_ {
        self.list.values().copied()
    }

    /// Java `getItem(id)` restricted to listed items.
    pub fn by_id(&self, id: ItemId) -> Option<ItemKey> {
        self.by_id.get(&id.0).copied()
    }

    /// The listed items containing net `net` (list order).
    pub fn net_items(&self, net: NetNo) -> impl DoubleEndedIterator<Item = ItemKey> + '_ {
        self.by_net.get(&net).into_iter().flat_map(|m| m.values().copied())
    }

    /// The listed items of a component (list order).
    pub fn component_items(&self, component: ComponentNo) -> impl DoubleEndedIterator<Item = ItemKey> + '_ {
        self.by_component.get(&component).into_iter().flat_map(|m| m.values().copied())
    }

    /// Java `startReadObject()`: a cursor prefetching the first entry.
    pub fn cursor(&self) -> ItemListCursor {
        ItemListCursor { next: self.list.iter().next().map(|(r, k)| (r.0, *k)) }
    }

    /// Java `readObject(it)`: returns the prefetched entry and prefetches its successor in the
    /// current list.
    pub fn cursor_next(&self, cursor: &mut ItemListCursor) -> Option<ItemKey> {
        let (id, key) = cursor.next.take()?;
        cursor.next = self
            .list
            .range((Bound::Excluded(Reverse(id)), Bound::Unbounded))
            .next()
            .map(|(r, k)| (r.0, *k));
        Some(key)
    }

    // ------------------------------------------------------------------------------------------
    // compaction

    /// Frees all arena slots of items that are neither listed nor on the board. Keys of freed
    /// items become stale (and their slots are reused). Call only at a point where no removed
    /// item is referenced any more.
    pub fn compact(&mut self) -> Vec<ItemKey> {
        let mut freed = Vec::new();
        for (index, slot) in self.slots.iter_mut().enumerate() {
            let Some(item) = &slot.item else { continue };
            let key = ItemKey { index: index as u32, generation: slot.generation };
            let listed = self.list.get(&Reverse(item.id().0)) == Some(&key);
            if !listed && !item.is_on_board() {
                slot.item = None;
                slot.generation = slot.generation.wrapping_add(1);
                self.free.push(index as u32);
                freed.push(key);
            }
        }
        freed
    }
}
