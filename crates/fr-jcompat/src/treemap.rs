//! Exact emulation of `java.util.TreeMap` / `java.util.TreeSet` (OpenJDK red-black tree).
//!
//! The tree shape, the comparator call sequence (`compare(key, node.key)`, starting at the
//! root) and all deletion details (`deleteEntry` copies the successor's key/value into a node
//! with two children; `fixAfterDeletion`) follow the JDK source, so results are identical even
//! for comparators that are not transitive or not antisymmetric (e.g. tolerance-based ones).
//!
//! Nodes live in an arena (`Vec` + free list, `u32` links). An [`EntryId`] names a *node*, like a
//! Java `TreeMap.Entry` reference: when an entry with two children is deleted the JDK moves the
//! successor's key/value into that node and unlinks the successor's node instead. [`TreeCursor`]
//! reproduces `PrivateEntryIterator.remove()` including this subtlety.
//!
//! Not emulated: sub-map views (`headMap`/`tailMap`/`subMap`), `buildFromSorted` (bulk
//! construction from a `SortedMap`/`SortedSet` with the same comparator — `TreeSet.addAll`
//! into an *empty* set from a `SortedSet`, `new TreeMap(SortedMap)`), `compute*`/`merge`.
//! `putAll`/`addAll` of an unsorted collection is plain repeated `put`/`add` in Java too.

use std::cmp::Ordering;
use std::fmt;

/// A comparator, like `java.util.Comparator<K>`. Only the sign of the result matters to TreeMap.
pub trait JavaComparator<K: ?Sized> {
    fn compare(&self, a: &K, b: &K) -> Ordering;
}

impl<K: ?Sized, F: Fn(&K, &K) -> Ordering> JavaComparator<K> for F {
    #[inline]
    fn compare(&self, a: &K, b: &K) -> Ordering {
        self(a, b)
    }
}

/// Natural ordering (`Comparable.compareTo`) via Rust `Ord`.
#[derive(Clone, Copy, Debug, Default)]
pub struct NaturalOrder;

impl<K: Ord + ?Sized> JavaComparator<K> for NaturalOrder {
    #[inline]
    fn compare(&self, a: &K, b: &K) -> Ordering {
        a.cmp(b)
    }
}

/// Converts a Java comparator result (`int`) to an [`Ordering`] (only the sign matters).
#[inline]
pub fn ordering_from_i32(c: i32) -> Ordering {
    c.cmp(&0)
}

const NIL: u32 = u32::MAX;
const RED: bool = false;
const BLACK: bool = true;

/// Handle to a tree node (a Java `TreeMap.Entry`). Valid until that node is unlinked.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EntryId(pub u32);

#[derive(Clone)]
struct Node<K, V> {
    kv: Option<(K, V)>,
    left: u32,
    right: u32,
    parent: u32,
    color: bool,
}

/// `java.util.TreeMap<K, V>` with comparator `C`.
///
/// Methods without suffix use the stored comparator; the `*_by` methods take an explicit
/// comparator (useful when the ordering needs external context such as a board reference).
/// Use the same comparator consistently, exactly as Java would.
#[derive(Clone)]
pub struct JavaTreeMap<K, V, C = NaturalOrder> {
    nodes: Vec<Node<K, V>>,
    free: Vec<u32>,
    root: u32,
    size: usize,
    mod_count: u64,
    cmp: C,
}

impl<K: fmt::Debug, V: fmt::Debug, C> fmt::Debug for JavaTreeMap<K, V, C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

impl<K, V, C: Default> Default for JavaTreeMap<K, V, C> {
    fn default() -> Self {
        Self::new(C::default())
    }
}

impl<K, V, C> JavaTreeMap<K, V, C> {
    /// `new TreeMap<>(comparator)`.
    pub fn new(cmp: C) -> Self {
        JavaTreeMap { nodes: Vec::new(), free: Vec::new(), root: NIL, size: 0, mod_count: 0, cmp }
    }

    pub fn comparator(&self) -> &C {
        &self.cmp
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.size
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.size == 0
    }

    /// Incremented on every structural modification (like Java's `modCount`).
    pub fn mod_count(&self) -> u64 {
        self.mod_count
    }

    /// `TreeMap.clear()`.
    pub fn clear(&mut self) {
        self.mod_count += 1;
        self.size = 0;
        self.root = NIL;
        self.nodes.clear();
        self.free.clear();
    }

    // ----- node accessors (null-safe like the static helpers in TreeMap) -----

    #[inline]
    fn n(&self, i: u32) -> &Node<K, V> {
        &self.nodes[i as usize]
    }
    #[inline]
    fn nm(&mut self, i: u32) -> &mut Node<K, V> {
        &mut self.nodes[i as usize]
    }
    #[inline]
    fn key_of(&self, i: u32) -> &K {
        &self.n(i).kv.as_ref().expect("live node").0
    }
    #[inline]
    fn parent_of(&self, i: u32) -> u32 {
        if i == NIL {
            NIL
        } else {
            self.n(i).parent
        }
    }
    #[inline]
    fn left_of(&self, i: u32) -> u32 {
        if i == NIL {
            NIL
        } else {
            self.n(i).left
        }
    }
    #[inline]
    fn right_of(&self, i: u32) -> u32 {
        if i == NIL {
            NIL
        } else {
            self.n(i).right
        }
    }
    #[inline]
    fn color_of(&self, i: u32) -> bool {
        if i == NIL {
            BLACK
        } else {
            self.n(i).color
        }
    }
    #[inline]
    fn set_color(&mut self, i: u32, c: bool) {
        if i != NIL {
            self.nm(i).color = c;
        }
    }

    fn alloc(&mut self, key: K, value: V, parent: u32) -> u32 {
        let node = Node { kv: Some((key, value)), left: NIL, right: NIL, parent, color: BLACK };
        if let Some(i) = self.free.pop() {
            self.nodes[i as usize] = node;
            i
        } else {
            assert!(self.nodes.len() < NIL as usize, "JavaTreeMap arena full");
            self.nodes.push(node);
            (self.nodes.len() - 1) as u32
        }
    }

    fn release(&mut self, i: u32) -> (K, V) {
        let n = self.nm(i);
        n.left = NIL;
        n.right = NIL;
        n.parent = NIL;
        let kv = n.kv.take().expect("live node");
        self.free.push(i);
        kv
    }

    // ----- public entry access -----

    /// Key of a live entry.
    pub fn key(&self, e: EntryId) -> &K {
        self.key_of(e.0)
    }

    /// Value of a live entry.
    pub fn value(&self, e: EntryId) -> &V {
        &self.n(e.0).kv.as_ref().expect("live node").1
    }

    /// Mutable value of a live entry (`Entry.setValue`).
    pub fn value_mut(&mut self, e: EntryId) -> &mut V {
        &mut self.nm(e.0).kv.as_mut().expect("live node").1
    }

    /// Key and value of a live entry.
    pub fn entry(&self, e: EntryId) -> (&K, &V) {
        let (k, v) = self.n(e.0).kv.as_ref().expect("live node");
        (k, v)
    }

    /// `true` if `e` names a node currently linked into the tree.
    pub fn is_live(&self, e: EntryId) -> bool {
        (e.0 as usize) < self.nodes.len() && self.nodes[e.0 as usize].kv.is_some()
    }

    /// `getFirstEntry()`.
    pub fn first_entry(&self) -> Option<EntryId> {
        let mut p = self.root;
        if p != NIL {
            while self.n(p).left != NIL {
                p = self.n(p).left;
            }
        }
        wrap(p)
    }

    /// `getLastEntry()`.
    pub fn last_entry(&self) -> Option<EntryId> {
        let mut p = self.root;
        if p != NIL {
            while self.n(p).right != NIL {
                p = self.n(p).right;
            }
        }
        wrap(p)
    }

    /// `firstKey()` (returns `None` instead of throwing).
    pub fn first_key(&self) -> Option<&K> {
        self.first_entry().map(|e| self.key(e))
    }

    /// `lastKey()` (returns `None` instead of throwing).
    pub fn last_key(&self) -> Option<&K> {
        self.last_entry().map(|e| self.key(e))
    }

    /// `firstEntry()`.
    pub fn first_key_value(&self) -> Option<(&K, &V)> {
        self.first_entry().map(|e| self.entry(e))
    }

    /// `lastEntry()`.
    pub fn last_key_value(&self) -> Option<(&K, &V)> {
        self.last_entry().map(|e| self.entry(e))
    }

    /// `TreeMap.successor(t)`.
    pub fn successor(&self, e: EntryId) -> Option<EntryId> {
        wrap(self.successor_raw(e.0))
    }

    /// `TreeMap.predecessor(t)`.
    pub fn predecessor(&self, e: EntryId) -> Option<EntryId> {
        wrap(self.predecessor_raw(e.0))
    }

    fn successor_raw(&self, t: u32) -> u32 {
        if t == NIL {
            NIL
        } else if self.n(t).right != NIL {
            let mut p = self.n(t).right;
            while self.n(p).left != NIL {
                p = self.n(p).left;
            }
            p
        } else {
            let mut p = self.n(t).parent;
            let mut ch = t;
            while p != NIL && ch == self.n(p).right {
                ch = p;
                p = self.n(p).parent;
            }
            p
        }
    }

    fn predecessor_raw(&self, t: u32) -> u32 {
        if t == NIL {
            NIL
        } else if self.n(t).left != NIL {
            let mut p = self.n(t).left;
            while self.n(p).right != NIL {
                p = self.n(p).right;
            }
            p
        } else {
            let mut p = self.n(t).parent;
            let mut ch = t;
            while p != NIL && ch == self.n(p).left {
                ch = p;
                p = self.n(p).parent;
            }
            p
        }
    }

    /// In-order iteration (`entrySet().iterator()`).
    pub fn iter(&self) -> Iter<'_, K, V, C> {
        Iter { map: self, next: self.first_entry().map_or(NIL, |e| e.0), back: false }
    }

    /// Descending iteration (`descendingMap().entrySet().iterator()`).
    pub fn iter_rev(&self) -> Iter<'_, K, V, C> {
        Iter { map: self, next: self.last_entry().map_or(NIL, |e| e.0), back: true }
    }

    /// Keys in ascending order.
    pub fn keys(&self) -> impl Iterator<Item = &K> + '_ {
        self.iter().map(|(k, _)| k)
    }

    /// Values in ascending key order.
    pub fn values(&self) -> impl Iterator<Item = &V> + '_ {
        self.iter().map(|(_, v)| v)
    }

    /// A detached ascending iterator supporting `remove()` (Java `PrivateEntryIterator`).
    pub fn cursor(&self) -> TreeCursor {
        TreeCursor { next: self.first_entry().map_or(NIL, |e| e.0), last_returned: NIL }
    }

    /// `entrySet().removeIf(...)`: iterates ascending and removes via the iterator.
    pub fn retain(&mut self, mut keep: impl FnMut(&K, &mut V) -> bool) {
        let mut c = self.cursor();
        while let Some(e) = c.next(self) {
            let (k, v) = self.nodes[e.0 as usize].kv.as_mut().expect("live node");
            if !keep(k, v) {
                c.remove(self);
            }
        }
    }

    // ----- structural algorithms (transliterated from OpenJDK TreeMap) -----

    /// `deleteEntry(p)`. Returns the removed key/value (the one that was stored in `p`).
    ///
    /// If `p` has two children, the successor's key/value is moved into `p` and the successor's
    /// node is unlinked instead (so `p` stays live, holding the successor's mapping).
    pub fn remove_entry(&mut self, e: EntryId) -> (K, V) {
        let mut p = e.0;
        self.mod_count += 1;
        self.size -= 1;

        // If strictly internal, copy successor's element to p and then make p point to successor.
        if self.n(p).left != NIL && self.n(p).right != NIL {
            let s = self.successor_raw(p);
            let skv = self.nm(s).kv.take();
            let pkv = std::mem::replace(&mut self.nm(p).kv, skv);
            self.nm(s).kv = pkv;
            p = s;
        }

        // Start fixup at replacement node, if it exists.
        let replacement = if self.n(p).left != NIL { self.n(p).left } else { self.n(p).right };

        if replacement != NIL {
            // Link replacement to parent
            let pp = self.n(p).parent;
            self.nm(replacement).parent = pp;
            if pp == NIL {
                self.root = replacement;
            } else if p == self.n(pp).left {
                self.nm(pp).left = replacement;
            } else {
                self.nm(pp).right = replacement;
            }
            // Null out links so they are OK to use by fixAfterDeletion.
            {
                let n = self.nm(p);
                n.left = NIL;
                n.right = NIL;
                n.parent = NIL;
            }
            if self.n(p).color == BLACK {
                self.fix_after_deletion(replacement);
            }
        } else if self.n(p).parent == NIL {
            // return if we are the only node.
            self.root = NIL;
        } else {
            // No children. Use self as phantom replacement and unlink.
            if self.n(p).color == BLACK {
                self.fix_after_deletion(p);
            }
            let pp = self.n(p).parent;
            if pp != NIL {
                if p == self.n(pp).left {
                    self.nm(pp).left = NIL;
                } else if p == self.n(pp).right {
                    self.nm(pp).right = NIL;
                }
                self.nm(p).parent = NIL;
            }
        }
        self.release(p)
    }

    /// `pollFirstEntry()`.
    pub fn poll_first(&mut self) -> Option<(K, V)> {
        self.first_entry().map(|e| self.remove_entry(e))
    }

    /// `pollLastEntry()`.
    pub fn poll_last(&mut self) -> Option<(K, V)> {
        self.last_entry().map(|e| self.remove_entry(e))
    }

    fn rotate_left(&mut self, p: u32) {
        if p != NIL {
            let r = self.n(p).right;
            let rl = self.n(r).left;
            self.nm(p).right = rl;
            if rl != NIL {
                self.nm(rl).parent = p;
            }
            let pp = self.n(p).parent;
            self.nm(r).parent = pp;
            if pp == NIL {
                self.root = r;
            } else if self.n(pp).left == p {
                self.nm(pp).left = r;
            } else {
                self.nm(pp).right = r;
            }
            self.nm(r).left = p;
            self.nm(p).parent = r;
        }
    }

    fn rotate_right(&mut self, p: u32) {
        if p != NIL {
            let l = self.n(p).left;
            let lr = self.n(l).right;
            self.nm(p).left = lr;
            if lr != NIL {
                self.nm(lr).parent = p;
            }
            let pp = self.n(p).parent;
            self.nm(l).parent = pp;
            if pp == NIL {
                self.root = l;
            } else if self.n(pp).right == p {
                self.nm(pp).right = l;
            } else {
                self.nm(pp).left = l;
            }
            self.nm(l).right = p;
            self.nm(p).parent = l;
        }
    }

    fn fix_after_insertion(&mut self, mut x: u32) {
        self.nm(x).color = RED;
        while x != NIL && x != self.root && self.n(self.n(x).parent).color == RED {
            let xp = self.parent_of(x);
            let xpp = self.parent_of(xp);
            if xp == self.left_of(xpp) {
                let y = self.right_of(xpp);
                if self.color_of(y) == RED {
                    self.set_color(xp, BLACK);
                    self.set_color(y, BLACK);
                    self.set_color(xpp, RED);
                    x = xpp;
                } else {
                    if x == self.right_of(self.parent_of(x)) {
                        x = self.parent_of(x);
                        self.rotate_left(x);
                    }
                    let xp = self.parent_of(x);
                    self.set_color(xp, BLACK);
                    let xpp = self.parent_of(xp);
                    self.set_color(xpp, RED);
                    self.rotate_right(xpp);
                }
            } else {
                let y = self.left_of(xpp);
                if self.color_of(y) == RED {
                    self.set_color(xp, BLACK);
                    self.set_color(y, BLACK);
                    self.set_color(xpp, RED);
                    x = xpp;
                } else {
                    if x == self.left_of(self.parent_of(x)) {
                        x = self.parent_of(x);
                        self.rotate_right(x);
                    }
                    let xp = self.parent_of(x);
                    self.set_color(xp, BLACK);
                    let xpp = self.parent_of(xp);
                    self.set_color(xpp, RED);
                    self.rotate_left(xpp);
                }
            }
        }
        let root = self.root;
        self.nm(root).color = BLACK;
    }

    fn fix_after_deletion(&mut self, mut x: u32) {
        while x != self.root && self.color_of(x) == BLACK {
            if x == self.left_of(self.parent_of(x)) {
                let mut sib = self.right_of(self.parent_of(x));
                if self.color_of(sib) == RED {
                    self.set_color(sib, BLACK);
                    self.set_color(self.parent_of(x), RED);
                    self.rotate_left(self.parent_of(x));
                    sib = self.right_of(self.parent_of(x));
                }
                if self.color_of(self.left_of(sib)) == BLACK && self.color_of(self.right_of(sib)) == BLACK {
                    self.set_color(sib, RED);
                    x = self.parent_of(x);
                } else {
                    if self.color_of(self.right_of(sib)) == BLACK {
                        self.set_color(self.left_of(sib), BLACK);
                        self.set_color(sib, RED);
                        self.rotate_right(sib);
                        sib = self.right_of(self.parent_of(x));
                    }
                    self.set_color(sib, self.color_of(self.parent_of(x)));
                    self.set_color(self.parent_of(x), BLACK);
                    self.set_color(self.right_of(sib), BLACK);
                    self.rotate_left(self.parent_of(x));
                    x = self.root;
                }
            } else {
                // symmetric
                let mut sib = self.left_of(self.parent_of(x));
                if self.color_of(sib) == RED {
                    self.set_color(sib, BLACK);
                    self.set_color(self.parent_of(x), RED);
                    self.rotate_right(self.parent_of(x));
                    sib = self.left_of(self.parent_of(x));
                }
                if self.color_of(self.right_of(sib)) == BLACK && self.color_of(self.left_of(sib)) == BLACK {
                    self.set_color(sib, RED);
                    x = self.parent_of(x);
                } else {
                    if self.color_of(self.left_of(sib)) == BLACK {
                        self.set_color(self.right_of(sib), BLACK);
                        self.set_color(sib, RED);
                        self.rotate_left(sib);
                        sib = self.left_of(self.parent_of(x));
                    }
                    self.set_color(sib, self.color_of(self.parent_of(x)));
                    self.set_color(self.parent_of(x), BLACK);
                    self.set_color(self.left_of(sib), BLACK);
                    self.rotate_right(self.parent_of(x));
                    x = self.root;
                }
            }
        }
        self.set_color(x, BLACK);
    }

    // ----- comparator-driven operations with explicit comparator -----

    /// `TreeMap.put(key, value)` with an explicit comparator. Returns the previous value; on an
    /// equal key (compare == 0) the existing key is kept and only the value is replaced.
    pub fn put_by(&mut self, key: K, value: V, mut cmp: impl FnMut(&K, &K) -> Ordering) -> Option<V> {
        match self.find_insert_point(&key, &mut cmp) {
            Ok(e) => Some(std::mem::replace(self.value_mut(EntryId(e)), value)),
            Err((parent, left)) => {
                self.insert_at(key, value, parent, left);
                None
            }
        }
    }

    /// `TreeMap.putIfAbsent(key, value)` (values are never null here, so an existing mapping is
    /// never replaced). Returns `Err(value)` with the rejected value if the key was present.
    pub fn put_if_absent_by(
        &mut self,
        key: K,
        value: V,
        mut cmp: impl FnMut(&K, &K) -> Ordering,
    ) -> Result<EntryId, (EntryId, V)> {
        match self.find_insert_point(&key, &mut cmp) {
            Ok(e) => Err((EntryId(e), value)),
            Err((parent, left)) => Ok(EntryId(self.insert_at(key, value, parent, left))),
        }
    }

    /// Ok(existing node) or Err((parent, add_to_left)); parent == NIL means empty map.
    fn find_insert_point(&self, key: &K, cmp: &mut impl FnMut(&K, &K) -> Ordering) -> Result<u32, (u32, bool)> {
        let mut t = self.root;
        if t == NIL {
            // addEntryToEmptyMap: compare(key, key) is called as a type (and possibly null) check.
            let _ = cmp(key, key);
            return Err((NIL, false));
        }
        loop {
            let parent = t;
            let c = cmp(key, self.key_of(t));
            match c {
                Ordering::Less => t = self.n(t).left,
                Ordering::Greater => t = self.n(t).right,
                Ordering::Equal => return Ok(t),
            }
            if t == NIL {
                return Err((parent, c == Ordering::Less));
            }
        }
    }

    fn insert_at(&mut self, key: K, value: V, parent: u32, left: bool) -> u32 {
        if parent == NIL {
            let e = self.alloc(key, value, NIL);
            self.root = e;
            self.size = 1;
            self.mod_count += 1;
            return e;
        }
        let e = self.alloc(key, value, parent);
        if left {
            self.nm(parent).left = e;
        } else {
            self.nm(parent).right = e;
        }
        self.fix_after_insertion(e);
        self.size += 1;
        self.mod_count += 1;
        e
    }

    /// `getEntryUsingComparator(key)`.
    pub fn get_entry_by(&self, key: &K, mut cmp: impl FnMut(&K, &K) -> Ordering) -> Option<EntryId> {
        let mut p = self.root;
        while p != NIL {
            match cmp(key, self.key_of(p)) {
                Ordering::Less => p = self.n(p).left,
                Ordering::Greater => p = self.n(p).right,
                Ordering::Equal => return Some(EntryId(p)),
            }
        }
        None
    }

    /// `TreeMap.remove(key)`: returns the removed mapping (key as stored in the map).
    pub fn remove_by(&mut self, key: &K, cmp: impl FnMut(&K, &K) -> Ordering) -> Option<(K, V)> {
        let e = self.get_entry_by(key, cmp)?;
        Some(self.remove_entry(e))
    }

    /// `getCeilingEntry(key)`: least entry >= key.
    pub fn ceiling_entry_by(&self, key: &K, mut cmp: impl FnMut(&K, &K) -> Ordering) -> Option<EntryId> {
        let mut p = self.root;
        while p != NIL {
            match cmp(key, self.key_of(p)) {
                Ordering::Less => {
                    if self.n(p).left != NIL {
                        p = self.n(p).left;
                    } else {
                        return Some(EntryId(p));
                    }
                }
                Ordering::Greater => {
                    if self.n(p).right != NIL {
                        p = self.n(p).right;
                    } else {
                        return wrap(self.climb_from_right(p));
                    }
                }
                Ordering::Equal => return Some(EntryId(p)),
            }
        }
        None
    }

    /// `getFloorEntry(key)`: greatest entry <= key.
    pub fn floor_entry_by(&self, key: &K, mut cmp: impl FnMut(&K, &K) -> Ordering) -> Option<EntryId> {
        let mut p = self.root;
        while p != NIL {
            match cmp(key, self.key_of(p)) {
                Ordering::Greater => {
                    if self.n(p).right != NIL {
                        p = self.n(p).right;
                    } else {
                        return Some(EntryId(p));
                    }
                }
                Ordering::Less => {
                    if self.n(p).left != NIL {
                        p = self.n(p).left;
                    } else {
                        return wrap(self.climb_from_left(p));
                    }
                }
                Ordering::Equal => return Some(EntryId(p)),
            }
        }
        None
    }

    /// `getHigherEntry(key)`: least entry > key.
    pub fn higher_entry_by(&self, key: &K, mut cmp: impl FnMut(&K, &K) -> Ordering) -> Option<EntryId> {
        let mut p = self.root;
        while p != NIL {
            if cmp(key, self.key_of(p)) == Ordering::Less {
                if self.n(p).left != NIL {
                    p = self.n(p).left;
                } else {
                    return Some(EntryId(p));
                }
            } else if self.n(p).right != NIL {
                p = self.n(p).right;
            } else {
                return wrap(self.climb_from_right(p));
            }
        }
        None
    }

    /// `getLowerEntry(key)`: greatest entry < key.
    pub fn lower_entry_by(&self, key: &K, mut cmp: impl FnMut(&K, &K) -> Ordering) -> Option<EntryId> {
        let mut p = self.root;
        while p != NIL {
            if cmp(key, self.key_of(p)) == Ordering::Greater {
                if self.n(p).right != NIL {
                    p = self.n(p).right;
                } else {
                    return Some(EntryId(p));
                }
            } else if self.n(p).left != NIL {
                p = self.n(p).left;
            } else {
                return wrap(self.climb_from_left(p));
            }
        }
        None
    }

    /// Climb while `ch` is a right child; return the first ancestor reached from the left.
    fn climb_from_right(&self, p: u32) -> u32 {
        let mut parent = self.n(p).parent;
        let mut ch = p;
        while parent != NIL && ch == self.n(parent).right {
            ch = parent;
            parent = self.n(parent).parent;
        }
        parent
    }

    fn climb_from_left(&self, p: u32) -> u32 {
        let mut parent = self.n(p).parent;
        let mut ch = p;
        while parent != NIL && ch == self.n(parent).left {
            ch = parent;
            parent = self.n(parent).parent;
        }
        parent
    }

    /// Checks red-black and link invariants (not ordering). For tests.
    pub fn check_invariants(&self) {
        fn walk<K, V, C>(m: &JavaTreeMap<K, V, C>, x: u32, parent: u32, count: &mut usize) -> usize {
            if x == NIL {
                return 1;
            }
            *count += 1;
            let n = m.n(x);
            assert!(n.kv.is_some());
            assert_eq!(n.parent, parent, "parent link");
            if n.color == RED {
                assert_eq!(m.color_of(n.left), BLACK, "red-red");
                assert_eq!(m.color_of(n.right), BLACK, "red-red");
            }
            let l = walk(m, n.left, x, count);
            let r = walk(m, n.right, x, count);
            assert_eq!(l, r, "black height");
            l + usize::from(n.color == BLACK)
        }
        let mut count = 0;
        if self.root != NIL {
            assert_eq!(self.n(self.root).color, BLACK);
        }
        walk(self, self.root, NIL, &mut count);
        assert_eq!(count, self.size);
        assert_eq!(self.nodes.len() - self.free.len(), self.size);
    }
}

#[inline]
fn wrap(i: u32) -> Option<EntryId> {
    if i == NIL {
        None
    } else {
        Some(EntryId(i))
    }
}

impl<K, V, C: JavaComparator<K>> JavaTreeMap<K, V, C> {
    /// `TreeMap.put(key, value)`: returns the old value; an equal key keeps the *old* key object.
    pub fn put(&mut self, key: K, value: V) -> Option<V> {
        match self.find_insert_point_cmp(&key) {
            Ok(e) => Some(std::mem::replace(self.value_mut(EntryId(e)), value)),
            Err((parent, left)) => {
                self.insert_at(key, value, parent, left);
                None
            }
        }
    }

    fn find_insert_point_cmp(&self, key: &K) -> Result<u32, (u32, bool)> {
        let cmp = &self.cmp;
        self.find_insert_point(key, &mut |a: &K, b: &K| cmp.compare(a, b))
    }

    /// `TreeMap.putIfAbsent(key, value)`: `Ok(new entry)` or `Err((existing entry, value))`.
    pub fn put_if_absent(&mut self, key: K, value: V) -> Result<EntryId, (EntryId, V)> {
        match self.find_insert_point_cmp(&key) {
            Ok(e) => Err((EntryId(e), value)),
            Err((parent, left)) => Ok(EntryId(self.insert_at(key, value, parent, left))),
        }
    }

    /// `getEntry(key)`.
    pub fn get_entry(&self, key: &K) -> Option<EntryId> {
        let cmp = &self.cmp;
        self.get_entry_by(key, |a, b| cmp.compare(a, b))
    }

    /// `TreeMap.get(key)`.
    pub fn get(&self, key: &K) -> Option<&V> {
        self.get_entry(key).map(|e| self.value(e))
    }

    /// Mutable `get`.
    pub fn get_mut(&mut self, key: &K) -> Option<&mut V> {
        let e = self.get_entry(key)?;
        Some(self.value_mut(e))
    }

    /// `TreeMap.containsKey(key)`.
    pub fn contains_key(&self, key: &K) -> bool {
        self.get_entry(key).is_some()
    }

    /// `TreeMap.remove(key)`: returns the removed mapping (with the key as stored in the map).
    pub fn remove(&mut self, key: &K) -> Option<(K, V)> {
        let e = self.get_entry(key)?;
        Some(self.remove_entry(e))
    }

    /// `ceilingEntry(key)`.
    pub fn ceiling_entry(&self, key: &K) -> Option<EntryId> {
        let cmp = &self.cmp;
        self.ceiling_entry_by(key, |a, b| cmp.compare(a, b))
    }

    /// `floorEntry(key)`.
    pub fn floor_entry(&self, key: &K) -> Option<EntryId> {
        let cmp = &self.cmp;
        self.floor_entry_by(key, |a, b| cmp.compare(a, b))
    }

    /// `higherEntry(key)`.
    pub fn higher_entry(&self, key: &K) -> Option<EntryId> {
        let cmp = &self.cmp;
        self.higher_entry_by(key, |a, b| cmp.compare(a, b))
    }

    /// `lowerEntry(key)`.
    pub fn lower_entry(&self, key: &K) -> Option<EntryId> {
        let cmp = &self.cmp;
        self.lower_entry_by(key, |a, b| cmp.compare(a, b))
    }

    /// `ceilingKey(key)`.
    pub fn ceiling_key(&self, key: &K) -> Option<&K> {
        self.ceiling_entry(key).map(|e| self.key(e))
    }

    /// `floorKey(key)`.
    pub fn floor_key(&self, key: &K) -> Option<&K> {
        self.floor_entry(key).map(|e| self.key(e))
    }

    /// `higherKey(key)`.
    pub fn higher_key(&self, key: &K) -> Option<&K> {
        self.higher_entry(key).map(|e| self.key(e))
    }

    /// `lowerKey(key)`.
    pub fn lower_key(&self, key: &K) -> Option<&K> {
        self.lower_entry(key).map(|e| self.key(e))
    }
}

/// Borrowing in-order iterator.
pub struct Iter<'a, K, V, C> {
    map: &'a JavaTreeMap<K, V, C>,
    next: u32,
    back: bool,
}

impl<'a, K, V, C> Iterator for Iter<'a, K, V, C> {
    type Item = (&'a K, &'a V);

    fn next(&mut self) -> Option<Self::Item> {
        if self.next == NIL {
            return None;
        }
        let e = self.next;
        self.next = if self.back { self.map.predecessor_raw(e) } else { self.map.successor_raw(e) };
        let (k, v) = self.map.n(e).kv.as_ref().expect("live node");
        Some((k, v))
    }
}

impl<'a, K, V, C> IntoIterator for &'a JavaTreeMap<K, V, C> {
    type Item = (&'a K, &'a V);
    type IntoIter = Iter<'a, K, V, C>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// Detached ascending iterator with removal, reproducing `TreeMap.PrivateEntryIterator`:
/// `next = successor(e)` is computed when `e` is returned, and `remove()` of an entry with two
/// children sets `next = lastReturned` (the successor's mapping is moved into that node).
///
/// The map may be modified only through [`TreeCursor::remove`] while a cursor is in use
/// (Java would throw `ConcurrentModificationException`).
#[derive(Clone, Copy, Debug)]
pub struct TreeCursor {
    next: u32,
    last_returned: u32,
}

impl TreeCursor {
    /// `hasNext()`.
    pub fn has_next(&self) -> bool {
        self.next != NIL
    }

    /// `nextEntry()`.
    pub fn next<K, V, C>(&mut self, map: &JavaTreeMap<K, V, C>) -> Option<EntryId> {
        let e = self.next;
        if e == NIL {
            return None;
        }
        self.next = map.successor_raw(e);
        self.last_returned = e;
        Some(EntryId(e))
    }

    /// `remove()`: removes the entry last returned by [`next`](Self::next); returns its mapping.
    /// Panics if there is none (Java: `IllegalStateException`).
    pub fn remove<K, V, C>(&mut self, map: &mut JavaTreeMap<K, V, C>) -> (K, V) {
        let lr = self.last_returned;
        assert!(lr != NIL, "IllegalStateException: no element to remove");
        // deleted entries are replaced by their successors
        if map.n(lr).left != NIL && map.n(lr).right != NIL {
            self.next = lr;
        }
        let kv = map.remove_entry(EntryId(lr));
        self.last_returned = NIL;
        kv
    }
}

// ---------------------------------------------------------------------------------------------

/// `java.util.TreeSet<K>` (a `TreeMap<K, PRESENT>`).
#[derive(Clone)]
pub struct JavaTreeSet<K, C = NaturalOrder> {
    map: JavaTreeMap<K, (), C>,
}

impl<K: fmt::Debug, C> fmt::Debug for JavaTreeSet<K, C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set().entries(self.iter()).finish()
    }
}

impl<K, C: Default> Default for JavaTreeSet<K, C> {
    fn default() -> Self {
        Self::new(C::default())
    }
}

impl<K, C> JavaTreeSet<K, C> {
    /// `new TreeSet<>(comparator)`.
    pub fn new(cmp: C) -> Self {
        JavaTreeSet { map: JavaTreeMap::new(cmp) }
    }

    /// The backing map.
    pub fn as_map(&self) -> &JavaTreeMap<K, (), C> {
        &self.map
    }

    /// The backing map (mutable; e.g. for [`JavaTreeMap::remove_entry`] or cursors).
    pub fn as_map_mut(&mut self) -> &mut JavaTreeMap<K, (), C> {
        &mut self.map
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn clear(&mut self) {
        self.map.clear()
    }

    /// `first()` (`None` instead of throwing).
    pub fn first(&self) -> Option<&K> {
        self.map.first_key()
    }

    /// `last()` (`None` instead of throwing).
    pub fn last(&self) -> Option<&K> {
        self.map.last_key()
    }

    /// `pollFirst()`.
    pub fn poll_first(&mut self) -> Option<K> {
        self.map.poll_first().map(|(k, _)| k)
    }

    /// `pollLast()`.
    pub fn poll_last(&mut self) -> Option<K> {
        self.map.poll_last().map(|(k, _)| k)
    }

    /// Ascending iteration.
    pub fn iter(&self) -> impl Iterator<Item = &K> + '_ {
        self.map.keys()
    }

    /// Descending iteration.
    pub fn iter_rev(&self) -> impl Iterator<Item = &K> + '_ {
        self.map.iter_rev().map(|(k, _)| k)
    }

    /// Detached iterator with removal (use with [`as_map`](Self::as_map) /
    /// [`as_map_mut`](Self::as_map_mut)).
    pub fn cursor(&self) -> TreeCursor {
        self.map.cursor()
    }

    /// `removeIf(pred)` complement: keeps elements for which `keep` returns true.
    pub fn retain(&mut self, mut keep: impl FnMut(&K) -> bool) {
        self.map.retain(|k, _| keep(k))
    }

    /// `add` with an explicit comparator.
    pub fn add_by(&mut self, key: K, cmp: impl FnMut(&K, &K) -> Ordering) -> bool {
        self.map.put_by(key, (), cmp).is_none()
    }

    /// `remove` with an explicit comparator; returns the removed element.
    pub fn remove_by(&mut self, key: &K, cmp: impl FnMut(&K, &K) -> Ordering) -> Option<K> {
        self.map.remove_by(key, cmp).map(|(k, _)| k)
    }

    /// `contains` with an explicit comparator.
    pub fn contains_by(&self, key: &K, cmp: impl FnMut(&K, &K) -> Ordering) -> bool {
        self.map.get_entry_by(key, cmp).is_some()
    }
}

impl<K, C: JavaComparator<K>> JavaTreeSet<K, C> {
    /// `TreeSet.add(e)`: returns false (and keeps the existing element) if an equal element exists.
    pub fn add(&mut self, key: K) -> bool {
        self.map.put(key, ()).is_none()
    }

    /// `TreeSet.remove(o)`: returns the removed element (as stored).
    pub fn remove(&mut self, key: &K) -> Option<K> {
        self.map.remove(key).map(|(k, _)| k)
    }

    /// `TreeSet.contains(o)`.
    pub fn contains(&self, key: &K) -> bool {
        self.map.contains_key(key)
    }

    /// `TreeSet.ceiling(e)`.
    pub fn ceiling(&self, key: &K) -> Option<&K> {
        self.map.ceiling_key(key)
    }

    /// `TreeSet.floor(e)`.
    pub fn floor(&self, key: &K) -> Option<&K> {
        self.map.floor_key(key)
    }

    /// `TreeSet.higher(e)`.
    pub fn higher(&self, key: &K) -> Option<&K> {
        self.map.higher_key(key)
    }

    /// `TreeSet.lower(e)`.
    pub fn lower(&self, key: &K) -> Option<&K> {
        self.map.lower_key(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_natural() {
        let mut m: JavaTreeMap<i32, i32> = JavaTreeMap::default();
        for i in [5, 3, 8, 1, 4, 7, 9, 2, 6] {
            assert_eq!(m.put(i, i * 10), None);
            m.check_invariants();
        }
        assert_eq!(m.put(4, 41), Some(40));
        assert_eq!(m.keys().copied().collect::<Vec<_>>(), (1..=9).collect::<Vec<_>>());
        assert_eq!(m.ceiling_key(&0), Some(&1));
        assert_eq!(m.higher_key(&9), None);
        assert_eq!(m.lower_key(&1), None);
        assert_eq!(m.floor_key(&100), Some(&9));
        assert_eq!(m.remove(&5), Some((5, 50)));
        m.check_invariants();
        let mut c = m.cursor();
        while let Some(e) = c.next(&m) {
            if m.key(e) % 2 == 0 {
                c.remove(&mut m);
                m.check_invariants();
            }
        }
        assert_eq!(m.keys().copied().collect::<Vec<_>>(), vec![1, 3, 7, 9]);
        assert_eq!(m.iter_rev().map(|(k, _)| *k).collect::<Vec<_>>(), vec![9, 7, 3, 1]);
        assert_eq!(m.poll_first(), Some((1, 10)));
        assert_eq!(m.poll_last(), Some((9, 90)));
        m.check_invariants();
    }

    #[test]
    fn set_keeps_old_element() {
        // compare only on the first component
        let mut s = JavaTreeSet::new(|a: &(i32, i32), b: &(i32, i32)| a.0.cmp(&b.0));
        assert!(s.add((1, 1)));
        assert!(!s.add((1, 2)));
        assert_eq!(s.first(), Some(&(1, 1)));
        let mut m = JavaTreeMap::new(|a: &(i32, i32), b: &(i32, i32)| a.0.cmp(&b.0));
        m.put((1, 1), 'a');
        assert_eq!(m.put((1, 2), 'b'), Some('a'));
        assert_eq!(m.first_key_value(), Some((&(1, 1), &'b')));
    }
}
