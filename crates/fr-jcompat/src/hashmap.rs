//! Iteration-order-exact emulation of `java.util.HashMap<Integer, V>` / `HashSet<Integer>`.
//!
//! Reproduced from the OpenJDK source (JDK 8+ layout, unchanged through JDK 25):
//! * `hash(key) = h ^ (h >>> 16)` with `Integer.hashCode() == value`; bin = `hash & (cap - 1)`.
//! * lazy table allocation, default capacity 16, load factor 0.75, `tableSizeFor` for an explicit
//!   initial capacity, `resize()` doubling with the order-preserving lo/hi split.
//! * `put`/`putIfAbsent` (`putVal`: append at the tail of a bin, resize *after* inserting when
//!   `++size > threshold`), `computeIfAbsent` (inserts at the *head* of a bin and resizes *before*
//!   the lookup when `size > threshold`), `remove`, `clear` (keeps the table).
//! * tree bins: `treeifyBin` (resize instead while capacity < 64), `TreeNode.treeify`,
//!   `putTreeVal` (new node linked after its tree parent), `moveRootToFront`, `split` on resize
//!   (with `untreeify` at <= 6 nodes), `removeTreeNode` + `balanceDeletion` (with the
//!   "too small" untreeify rule). Iteration order in a tree bin follows the `next` links,
//!   which these operations permute, so they are emulated exactly.
//!
//! Integer keys have distinct hashes (the spread function is a bijection), so the tree-bin
//! ordering never needs `compareComparables`/`tieBreakOrder`.
//!
//! Not emulated: `putAll`/`new HashMap(Map)` pre-sizing, `compute`/`merge`, `LinkedHashMap`.

const NIL: u32 = u32::MAX;
const DEFAULT_INITIAL_CAPACITY: usize = 16;
const MAXIMUM_CAPACITY: usize = 1 << 30;
const TREEIFY_THRESHOLD: usize = 8;
const UNTREEIFY_THRESHOLD: usize = 6;
const MIN_TREEIFY_CAPACITY: usize = 64;
const LOAD_FACTOR: f32 = 0.75;

/// `HashMap.hash(Object)` for an `Integer` key.
#[inline]
pub fn spread_hash(key: i32) -> i32 {
    key ^ ((key as u32) >> 16) as i32
}

/// `HashMap.tableSizeFor(int)`.
pub fn table_size_for(cap: i32) -> i32 {
    // -1 >>> Integer.numberOfLeadingZeros(cap - 1)  (Java masks the shift count by 31)
    let nlz = (cap.wrapping_sub(1) as u32).leading_zeros();
    let n = ((-1i32 as u32) >> (nlz & 31)) as i32;
    if n < 0 {
        1
    } else if n as usize >= MAXIMUM_CAPACITY {
        MAXIMUM_CAPACITY as i32
    } else {
        n + 1
    }
}

#[derive(Clone)]
struct HNode<V> {
    hash: i32,
    key: i32,
    value: Option<V>,
    next: u32,
    // TreeNode fields
    tree: bool,
    prev: u32,
    parent: u32,
    left: u32,
    right: u32,
    red: bool,
}

/// `java.util.HashMap<Integer, V>` with Java iteration order.
#[derive(Clone)]
pub struct JavaIntHashMap<V> {
    table: Vec<u32>, // empty == null table
    nodes: Vec<HNode<V>>,
    free: Vec<u32>,
    size: usize,
    threshold: i32,
    mod_count: u64,
}

impl<V> Default for JavaIntHashMap<V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<V: std::fmt::Debug> std::fmt::Debug for JavaIntHashMap<V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

impl<V> JavaIntHashMap<V> {
    /// `new HashMap<>()`.
    pub fn new() -> Self {
        JavaIntHashMap { table: Vec::new(), nodes: Vec::new(), free: Vec::new(), size: 0, threshold: 0, mod_count: 0 }
    }

    /// `new HashMap<>(initialCapacity)`.
    pub fn with_capacity(initial_capacity: i32) -> Self {
        assert!(initial_capacity >= 0, "Illegal initial capacity");
        let mut m = Self::new();
        m.threshold = table_size_for(initial_capacity);
        m
    }

    pub fn len(&self) -> usize {
        self.size
    }

    pub fn is_empty(&self) -> bool {
        self.size == 0
    }

    /// Current table length (0 before the first insertion).
    pub fn capacity(&self) -> usize {
        self.table.len()
    }

    /// Structural modification counter (`modCount`).
    pub fn mod_count(&self) -> u64 {
        self.mod_count
    }

    /// `true` if any bin is currently a tree bin (treeified).
    pub fn has_tree_bins(&self) -> bool {
        self.table.iter().any(|&f| f != NIL && self.n(f).tree)
    }

    /// `true` if the bin that `key` maps to is currently a tree bin.
    pub fn bin_is_tree(&self, key: i32) -> bool {
        if self.table.is_empty() {
            return false;
        }
        let f = self.table[(spread_hash(key) as u32 as usize) & (self.table.len() - 1)];
        f != NIL && self.n(f).tree
    }

    #[inline]
    fn n(&self, i: u32) -> &HNode<V> {
        &self.nodes[i as usize]
    }
    #[inline]
    fn nm(&mut self, i: u32) -> &mut HNode<V> {
        &mut self.nodes[i as usize]
    }

    fn new_node(&mut self, hash: i32, key: i32, value: V, next: u32) -> u32 {
        let node = HNode {
            hash,
            key,
            value: Some(value),
            next,
            tree: false,
            prev: NIL,
            parent: NIL,
            left: NIL,
            right: NIL,
            red: false,
        };
        if let Some(i) = self.free.pop() {
            self.nodes[i as usize] = node;
            i
        } else {
            assert!(self.nodes.len() < NIL as usize);
            self.nodes.push(node);
            (self.nodes.len() - 1) as u32
        }
    }

    fn release(&mut self, i: u32) -> V {
        let n = self.nm(i);
        n.next = NIL;
        n.prev = NIL;
        n.parent = NIL;
        n.left = NIL;
        n.right = NIL;
        n.tree = false;
        let v = n.value.take().expect("live node");
        self.free.push(i);
        v
    }

    fn find_node(&self, key: i32) -> u32 {
        if self.table.is_empty() {
            return NIL;
        }
        let hash = spread_hash(key);
        let mut e = self.table[(hash as u32 as usize) & (self.table.len() - 1)];
        while e != NIL {
            if self.n(e).key == key {
                return e;
            }
            e = self.n(e).next;
        }
        NIL
    }

    /// `get(key)`.
    pub fn get(&self, key: i32) -> Option<&V> {
        let e = self.find_node(key);
        if e == NIL {
            None
        } else {
            self.n(e).value.as_ref()
        }
    }

    /// Mutable `get(key)`.
    pub fn get_mut(&mut self, key: i32) -> Option<&mut V> {
        let e = self.find_node(key);
        if e == NIL {
            None
        } else {
            self.nm(e).value.as_mut()
        }
    }

    /// `containsKey(key)`.
    pub fn contains_key(&self, key: i32) -> bool {
        self.find_node(key) != NIL
    }

    /// `put(key, value)`: returns the previous value.
    pub fn insert(&mut self, key: i32, value: V) -> Option<V> {
        self.put_val(key, value, false).err()
    }

    /// Alias of [`insert`](Self::insert) (Java name).
    pub fn put(&mut self, key: i32, value: V) -> Option<V> {
        self.insert(key, value)
    }

    /// `putIfAbsent(key, value)`: `true` if inserted; an existing value is never replaced
    /// (values are non-null here).
    pub fn put_if_absent(&mut self, key: i32, value: V) -> bool {
        self.put_val(key, value, true).is_ok()
    }

    /// `putVal`: Ok(()) if a new node was inserted, Err(old or rejected value) otherwise.
    fn put_val(&mut self, key: i32, value: V, only_if_absent: bool) -> Result<(), V> {
        let hash = spread_hash(key);
        if self.table.is_empty() {
            self.resize();
        }
        let n = self.table.len();
        let i = (hash as u32 as usize) & (n - 1);
        let first = self.table[i];
        if first == NIL {
            let nn = self.new_node(hash, key, value, NIL);
            self.table[i] = nn;
            return self.after_insert_put();
        }
        if self.n(first).key == key {
            return Err(self.replace_value(first, value, only_if_absent));
        }
        if self.n(first).tree {
            return match self.put_tree_val(first, hash, key, value) {
                Ok(()) => self.after_insert_put(),
                Err((found, v)) => Err(self.replace_value(found, v, only_if_absent)),
            };
        }
        let mut p = first;
        let mut bin_count = 0usize;
        loop {
            let nx = self.n(p).next;
            if nx == NIL {
                let nn = self.new_node(hash, key, value, NIL);
                self.nm(p).next = nn;
                if bin_count >= TREEIFY_THRESHOLD - 1 {
                    // -1 for 1st
                    self.treeify_bin(hash);
                }
                return self.after_insert_put();
            }
            if self.n(nx).key == key {
                return Err(self.replace_value(nx, value, only_if_absent));
            }
            p = nx;
            bin_count += 1;
        }
    }

    fn replace_value(&mut self, e: u32, value: V, only_if_absent: bool) -> V {
        if only_if_absent {
            value
        } else {
            self.nm(e).value.replace(value).expect("live node")
        }
    }

    fn after_insert_put(&mut self) -> Result<(), V> {
        self.mod_count += 1;
        self.size += 1;
        if self.size as i64 > self.threshold as i64 {
            self.resize();
        }
        Ok(())
    }

    /// `computeIfAbsent(key, k -> f())`: returns the (existing or new) value.
    ///
    /// Note Java's implementation differs from `put`: it resizes *before* the lookup when
    /// `size > threshold` and inserts new nodes at the *head* of the bin.
    pub fn compute_if_absent(&mut self, key: i32, f: impl FnOnce() -> V) -> &mut V {
        let hash = spread_hash(key);
        if self.size as i64 > self.threshold as i64 || self.table.is_empty() {
            self.resize();
        }
        let n = self.table.len();
        let i = (hash as u32 as usize) & (n - 1);
        let first = self.table[i];
        let mut bin_count = 0usize;
        let mut t = NIL;
        let mut old = NIL;
        if first != NIL {
            if self.n(first).tree {
                t = first;
                old = self.find_in_bin(first, key);
            } else {
                let mut e = first;
                while e != NIL {
                    if self.n(e).key == key {
                        old = e;
                        break;
                    }
                    bin_count += 1;
                    e = self.n(e).next;
                }
            }
            if old != NIL {
                return self.nm(old).value.as_mut().expect("live node");
            }
        }
        let v = f();
        let node;
        if t != NIL {
            match self.put_tree_val(t, hash, key, v) {
                Ok(()) => node = self.find_node(key),
                Err(_) => unreachable!(),
            }
        } else {
            node = self.new_node(hash, key, v, first);
            self.table[i] = node;
            if bin_count >= TREEIFY_THRESHOLD - 1 {
                self.treeify_bin(hash);
            }
        }
        self.mod_count += 1;
        self.size += 1;
        self.nm(node).value.as_mut().expect("live node")
    }

    fn find_in_bin(&self, first: u32, key: i32) -> u32 {
        let mut e = first;
        while e != NIL {
            if self.n(e).key == key {
                return e;
            }
            e = self.n(e).next;
        }
        NIL
    }

    /// `remove(key)`: returns the removed value.
    pub fn remove(&mut self, key: i32) -> Option<V> {
        if self.table.is_empty() {
            return None;
        }
        let hash = spread_hash(key);
        let index = (hash as u32 as usize) & (self.table.len() - 1);
        let mut p = self.table[index];
        if p == NIL {
            return None;
        }
        let mut node = NIL;
        if self.n(p).key == key {
            node = p;
        } else {
            let mut e = self.n(p).next;
            if e != NIL {
                if self.n(p).tree {
                    node = self.find_in_bin(p, key);
                } else {
                    while e != NIL {
                        if self.n(e).key == key {
                            node = e;
                            break;
                        }
                        p = e;
                        e = self.n(e).next;
                    }
                }
            }
        }
        if node == NIL {
            return None;
        }
        if self.n(node).tree {
            self.remove_tree_node(node, true);
        } else if node == p {
            self.table[index] = self.n(node).next;
        } else {
            let nx = self.n(node).next;
            self.nm(p).next = nx;
        }
        self.mod_count += 1;
        self.size -= 1;
        Some(self.release(node))
    }

    /// `clear()` (keeps the table capacity, like Java).
    pub fn clear(&mut self) {
        self.mod_count += 1;
        if !self.table.is_empty() && self.size > 0 {
            self.size = 0;
            for b in self.table.iter_mut() {
                *b = NIL;
            }
        }
        self.nodes.clear();
        self.free.clear();
    }

    /// Iteration in Java `entrySet()` order.
    pub fn iter(&self) -> Iter<'_, V> {
        Iter { map: self, bin: 0, next: NIL }
    }

    /// Keys in Java iteration order.
    pub fn keys(&self) -> impl Iterator<Item = i32> + '_ {
        self.iter().map(|(k, _)| k)
    }

    /// Values in Java iteration order.
    pub fn values(&self) -> impl Iterator<Item = &V> + '_ {
        self.iter().map(|(_, v)| v)
    }

    /// Removes entries for which `keep` returns false (`entrySet().removeIf`); the order of the
    /// remaining entries is what Java's iterator-remove would produce.
    pub fn retain(&mut self, mut keep: impl FnMut(i32, &mut V) -> bool) {
        let keys: Vec<i32> = self.keys().collect();
        for k in keys {
            let e = self.find_node(k);
            let v = self.nm(e).value.as_mut().expect("live node");
            if !keep(k, v) {
                // HashIterator.remove() calls removeNode(..., movable = false)
                self.remove_impl(k, false);
            }
        }
    }

    fn remove_impl(&mut self, key: i32, movable: bool) -> Option<V> {
        if movable {
            return self.remove(key);
        }
        let hash = spread_hash(key);
        let index = (hash as u32 as usize) & (self.table.len() - 1);
        let mut p = self.table[index];
        let node;
        if self.n(p).key == key {
            node = p;
        } else if self.n(p).tree {
            node = self.find_in_bin(p, key);
        } else {
            let mut e = self.n(p).next;
            while self.n(e).key != key {
                p = e;
                e = self.n(e).next;
            }
            node = e;
        }
        if self.n(node).tree {
            self.remove_tree_node(node, false);
        } else if node == p {
            self.table[index] = self.n(node).next;
        } else {
            let nx = self.n(node).next;
            self.nm(p).next = nx;
        }
        self.mod_count += 1;
        self.size -= 1;
        Some(self.release(node))
    }

    // ----- resize -----

    fn resize(&mut self) {
        let old_cap = self.table.len();
        let old_thr = self.threshold;
        let new_cap: usize;
        let mut new_thr: i32 = 0;
        if old_cap > 0 {
            if old_cap >= MAXIMUM_CAPACITY {
                self.threshold = i32::MAX;
                return;
            }
            new_cap = old_cap << 1;
            if new_cap < MAXIMUM_CAPACITY && old_cap >= DEFAULT_INITIAL_CAPACITY {
                new_thr = old_thr << 1;
            }
        } else if old_thr > 0 {
            new_cap = old_thr as usize;
        } else {
            new_cap = DEFAULT_INITIAL_CAPACITY;
            new_thr = (LOAD_FACTOR * DEFAULT_INITIAL_CAPACITY as f32) as i32;
        }
        if new_thr == 0 {
            let ft = new_cap as f32 * LOAD_FACTOR;
            new_thr = if new_cap < MAXIMUM_CAPACITY && ft < MAXIMUM_CAPACITY as f32 { ft as i32 } else { i32::MAX };
        }
        self.threshold = new_thr;
        let old_tab = std::mem::replace(&mut self.table, vec![NIL; new_cap]);
        for (j, &first) in old_tab.iter().enumerate() {
            if first == NIL {
                continue;
            }
            let e = first;
            if self.n(e).next == NIL {
                let h = self.n(e).hash;
                self.table[(h as u32 as usize) & (new_cap - 1)] = e;
            } else if self.n(e).tree {
                self.split(e, j, old_cap);
            } else {
                let (mut lo_head, mut lo_tail, mut hi_head, mut hi_tail) = (NIL, NIL, NIL, NIL);
                let mut e = e;
                while e != NIL {
                    let next = self.n(e).next;
                    if (self.n(e).hash as u32 as usize) & old_cap == 0 {
                        if lo_tail == NIL {
                            lo_head = e;
                        } else {
                            self.nm(lo_tail).next = e;
                        }
                        lo_tail = e;
                    } else {
                        if hi_tail == NIL {
                            hi_head = e;
                        } else {
                            self.nm(hi_tail).next = e;
                        }
                        hi_tail = e;
                    }
                    e = next;
                }
                if lo_tail != NIL {
                    self.nm(lo_tail).next = NIL;
                    self.table[j] = lo_head;
                }
                if hi_tail != NIL {
                    self.nm(hi_tail).next = NIL;
                    self.table[j + old_cap] = hi_head;
                }
            }
        }
    }

    // ----- tree bins (HashMap.TreeNode) -----

    fn treeify_bin(&mut self, hash: i32) {
        let n = self.table.len();
        if n < MIN_TREEIFY_CAPACITY {
            self.resize();
            return;
        }
        let index = (hash as u32 as usize) & (n - 1);
        let mut e = self.table[index];
        if e == NIL {
            return;
        }
        // replacementTreeNode for each node, keeping the list order (prev links added).
        let mut tl = NIL;
        while e != NIL {
            {
                let nd = self.nm(e);
                nd.tree = true;
                nd.prev = tl;
                nd.parent = NIL;
                nd.left = NIL;
                nd.right = NIL;
                nd.red = false;
            }
            tl = e;
            e = self.n(e).next;
        }
        let hd = self.table[index];
        self.treeify(hd);
    }

    /// `TreeNode.treeify(tab)` called on list head `hd`.
    fn treeify(&mut self, hd: u32) {
        let mut root = NIL;
        let mut x = hd;
        while x != NIL {
            let next = self.n(x).next;
            self.nm(x).left = NIL;
            self.nm(x).right = NIL;
            if root == NIL {
                self.nm(x).parent = NIL;
                self.nm(x).red = false;
                root = x;
            } else {
                let h = self.n(x).hash;
                let mut p = root;
                loop {
                    let ph = self.n(p).hash;
                    // Integer keys: equal hash <=> equal key, which cannot occur here.
                    let dir: i32 = if ph > h { -1 } else { 1 };
                    let xp = p;
                    p = if dir <= 0 { self.n(p).left } else { self.n(p).right };
                    if p == NIL {
                        self.nm(x).parent = xp;
                        if dir <= 0 {
                            self.nm(xp).left = x;
                        } else {
                            self.nm(xp).right = x;
                        }
                        root = self.balance_insertion(root, x);
                        break;
                    }
                }
            }
            x = next;
        }
        self.move_root_to_front(root);
    }

    /// `TreeNode.untreeify(map)`: converts the list starting at `hd` to plain nodes (same order).
    fn untreeify(&mut self, hd: u32) -> u32 {
        let mut q = hd;
        while q != NIL {
            let nd = self.nm(q);
            nd.tree = false;
            nd.prev = NIL;
            nd.parent = NIL;
            nd.left = NIL;
            nd.right = NIL;
            nd.red = false;
            q = nd.next;
        }
        hd
    }

    fn tree_root(&self, mut r: u32) -> u32 {
        loop {
            let p = self.n(r).parent;
            if p == NIL {
                return r;
            }
            r = p;
        }
    }

    /// `putTreeVal`: Ok(()) if inserted, Err((existing node, value)) if the key is present.
    fn put_tree_val(&mut self, first: u32, h: i32, key: i32, value: V) -> Result<(), (u32, V)> {
        let root = if self.n(first).parent != NIL { self.tree_root(first) } else { first };
        let mut p = root;
        loop {
            let ph = self.n(p).hash;
            let dir: i32 = if ph > h {
                -1
            } else if ph < h {
                1
            } else {
                debug_assert_eq!(self.n(p).key, key);
                return Err((p, value));
            };
            let xp = p;
            p = if dir <= 0 { self.n(p).left } else { self.n(p).right };
            if p == NIL {
                let xpn = self.n(xp).next;
                let x = self.new_node(h, key, value, xpn);
                self.nm(x).tree = true;
                if dir <= 0 {
                    self.nm(xp).left = x;
                } else {
                    self.nm(xp).right = x;
                }
                self.nm(xp).next = x;
                self.nm(x).parent = xp;
                self.nm(x).prev = xp;
                if xpn != NIL {
                    self.nm(xpn).prev = x;
                }
                let r = self.balance_insertion(root, x);
                self.move_root_to_front(r);
                return Ok(());
            }
        }
    }

    fn move_root_to_front(&mut self, root: u32) {
        let n = self.table.len();
        if root != NIL && n > 0 {
            let index = (self.n(root).hash as u32 as usize) & (n - 1);
            let first = self.table[index];
            if root != first {
                self.table[index] = root;
                let rp = self.n(root).prev;
                let rn = self.n(root).next;
                if rn != NIL {
                    self.nm(rn).prev = rp;
                }
                if rp != NIL {
                    self.nm(rp).next = rn;
                }
                if first != NIL {
                    self.nm(first).prev = root;
                }
                self.nm(root).next = first;
                self.nm(root).prev = NIL;
            }
        }
    }

    /// `TreeNode.split(map, tab, index, bit)` for the tree bin headed by `b`.
    fn split(&mut self, b: u32, index: usize, bit: usize) {
        let (mut lo_head, mut lo_tail, mut hi_head, mut hi_tail) = (NIL, NIL, NIL, NIL);
        let (mut lc, mut hc) = (0usize, 0usize);
        let mut e = b;
        while e != NIL {
            let next = self.n(e).next;
            self.nm(e).next = NIL;
            if (self.n(e).hash as u32 as usize) & bit == 0 {
                self.nm(e).prev = lo_tail;
                if lo_tail == NIL {
                    lo_head = e;
                } else {
                    self.nm(lo_tail).next = e;
                }
                lo_tail = e;
                lc += 1;
            } else {
                self.nm(e).prev = hi_tail;
                if hi_tail == NIL {
                    hi_head = e;
                } else {
                    self.nm(hi_tail).next = e;
                }
                hi_tail = e;
                hc += 1;
            }
            e = next;
        }
        if lo_head != NIL {
            if lc <= UNTREEIFY_THRESHOLD {
                self.table[index] = self.untreeify(lo_head);
            } else {
                self.table[index] = lo_head;
                if hi_head != NIL {
                    self.treeify(lo_head);
                }
            }
        }
        if hi_head != NIL {
            if hc <= UNTREEIFY_THRESHOLD {
                self.table[index + bit] = self.untreeify(hi_head);
            } else {
                self.table[index + bit] = hi_head;
                if lo_head != NIL {
                    self.treeify(hi_head);
                }
            }
        }
    }

    fn rotate_left(&mut self, mut root: u32, p: u32) -> u32 {
        if p != NIL {
            let r = self.n(p).right;
            if r != NIL {
                let rl = self.n(r).left;
                self.nm(p).right = rl;
                if rl != NIL {
                    self.nm(rl).parent = p;
                }
                let pp = self.n(p).parent;
                self.nm(r).parent = pp;
                if pp == NIL {
                    root = r;
                    self.nm(r).red = false;
                } else if self.n(pp).left == p {
                    self.nm(pp).left = r;
                } else {
                    self.nm(pp).right = r;
                }
                self.nm(r).left = p;
                self.nm(p).parent = r;
            }
        }
        root
    }

    fn rotate_right(&mut self, mut root: u32, p: u32) -> u32 {
        if p != NIL {
            let l = self.n(p).left;
            if l != NIL {
                let lr = self.n(l).right;
                self.nm(p).left = lr;
                if lr != NIL {
                    self.nm(lr).parent = p;
                }
                let pp = self.n(p).parent;
                self.nm(l).parent = pp;
                if pp == NIL {
                    root = l;
                    self.nm(l).red = false;
                } else if self.n(pp).right == p {
                    self.nm(pp).right = l;
                } else {
                    self.nm(pp).left = l;
                }
                self.nm(l).right = p;
                self.nm(p).parent = l;
            }
        }
        root
    }

    fn balance_insertion(&mut self, mut root: u32, mut x: u32) -> u32 {
        self.nm(x).red = true;
        loop {
            let mut xp = self.n(x).parent;
            if xp == NIL {
                self.nm(x).red = false;
                return x;
            }
            if !self.n(xp).red {
                return root;
            }
            let mut xpp = self.n(xp).parent;
            if xpp == NIL {
                return root;
            }
            let xppl = self.n(xpp).left;
            if xp == xppl {
                let xppr = self.n(xpp).right;
                if xppr != NIL && self.n(xppr).red {
                    self.nm(xppr).red = false;
                    self.nm(xp).red = false;
                    self.nm(xpp).red = true;
                    x = xpp;
                } else {
                    if x == self.n(xp).right {
                        x = xp;
                        root = self.rotate_left(root, x);
                        xp = self.n(x).parent;
                        xpp = if xp == NIL { NIL } else { self.n(xp).parent };
                    }
                    if xp != NIL {
                        self.nm(xp).red = false;
                        if xpp != NIL {
                            self.nm(xpp).red = true;
                            root = self.rotate_right(root, xpp);
                        }
                    }
                }
            } else if xppl != NIL && self.n(xppl).red {
                self.nm(xppl).red = false;
                self.nm(xp).red = false;
                self.nm(xpp).red = true;
                x = xpp;
            } else {
                if x == self.n(xp).left {
                    x = xp;
                    root = self.rotate_right(root, x);
                    xp = self.n(x).parent;
                    xpp = if xp == NIL { NIL } else { self.n(xp).parent };
                }
                if xp != NIL {
                    self.nm(xp).red = false;
                    if xpp != NIL {
                        self.nm(xpp).red = true;
                        root = self.rotate_left(root, xpp);
                    }
                }
            }
        }
    }

    #[inline]
    fn is_red(&self, x: u32) -> bool {
        x != NIL && self.n(x).red
    }

    fn balance_deletion(&mut self, mut root: u32, mut x: u32) -> u32 {
        loop {
            if x == NIL || x == root {
                return root;
            }
            let mut xp = self.n(x).parent;
            if xp == NIL {
                self.nm(x).red = false;
                return x;
            } else if self.n(x).red {
                self.nm(x).red = false;
                return root;
            }
            let xpl = self.n(xp).left;
            if xpl == x {
                let mut xpr = self.n(xp).right;
                if xpr != NIL && self.n(xpr).red {
                    self.nm(xpr).red = false;
                    self.nm(xp).red = true;
                    root = self.rotate_left(root, xp);
                    xp = self.n(x).parent;
                    xpr = if xp == NIL { NIL } else { self.n(xp).right };
                }
                if xpr == NIL {
                    x = xp;
                } else {
                    let sl = self.n(xpr).left;
                    let mut sr = self.n(xpr).right;
                    if !self.is_red(sr) && !self.is_red(sl) {
                        self.nm(xpr).red = true;
                        x = xp;
                    } else {
                        if !self.is_red(sr) {
                            if sl != NIL {
                                self.nm(sl).red = false;
                            }
                            self.nm(xpr).red = true;
                            root = self.rotate_right(root, xpr);
                            xp = self.n(x).parent;
                            xpr = if xp == NIL { NIL } else { self.n(xp).right };
                        }
                        if xpr != NIL {
                            let c = if xp == NIL { false } else { self.n(xp).red };
                            self.nm(xpr).red = c;
                            sr = self.n(xpr).right;
                            if sr != NIL {
                                self.nm(sr).red = false;
                            }
                        }
                        if xp != NIL {
                            self.nm(xp).red = false;
                            root = self.rotate_left(root, xp);
                        }
                        x = root;
                    }
                }
            } else {
                // symmetric
                let mut xpl = xpl;
                if xpl != NIL && self.n(xpl).red {
                    self.nm(xpl).red = false;
                    self.nm(xp).red = true;
                    root = self.rotate_right(root, xp);
                    xp = self.n(x).parent;
                    xpl = if xp == NIL { NIL } else { self.n(xp).left };
                }
                if xpl == NIL {
                    x = xp;
                } else {
                    let mut sl = self.n(xpl).left;
                    let sr = self.n(xpl).right;
                    if !self.is_red(sl) && !self.is_red(sr) {
                        self.nm(xpl).red = true;
                        x = xp;
                    } else {
                        if !self.is_red(sl) {
                            if sr != NIL {
                                self.nm(sr).red = false;
                            }
                            self.nm(xpl).red = true;
                            root = self.rotate_left(root, xpl);
                            xp = self.n(x).parent;
                            xpl = if xp == NIL { NIL } else { self.n(xp).left };
                        }
                        if xpl != NIL {
                            let c = if xp == NIL { false } else { self.n(xp).red };
                            self.nm(xpl).red = c;
                            sl = self.n(xpl).left;
                            if sl != NIL {
                                self.nm(sl).red = false;
                            }
                        }
                        if xp != NIL {
                            self.nm(xp).red = false;
                            root = self.rotate_right(root, xp);
                        }
                        x = root;
                    }
                }
            }
        }
    }

    /// `TreeNode.removeTreeNode(map, tab, movable)` for node `this_`.
    fn remove_tree_node(&mut self, this_: u32, movable: bool) {
        let n = self.table.len();
        if n == 0 {
            return;
        }
        let index = (self.n(this_).hash as u32 as usize) & (n - 1);
        let mut first = self.table[index];
        let mut root = first;
        let succ = self.n(this_).next;
        let pred = self.n(this_).prev;
        if pred == NIL {
            first = succ;
            self.table[index] = first;
        } else {
            self.nm(pred).next = succ;
        }
        if succ != NIL {
            self.nm(succ).prev = pred;
        }
        if first == NIL {
            return;
        }
        if self.n(root).parent != NIL {
            root = self.tree_root(root);
        }
        if root == NIL
            || (movable && {
                let rr = self.n(root).right;
                let rl = self.n(root).left;
                rr == NIL || rl == NIL || self.n(rl).left == NIL
            })
        {
            self.table[index] = self.untreeify(first); // too small
            return;
        }
        let p = this_;
        let pl = self.n(p).left;
        let pr = self.n(p).right;
        let replacement;
        if pl != NIL && pr != NIL {
            let mut s = pr;
            loop {
                let sl = self.n(s).left;
                if sl == NIL {
                    break;
                }
                s = sl;
            }
            // swap colors
            let c = self.n(s).red;
            let pred_ = self.n(p).red;
            self.nm(s).red = pred_;
            self.nm(p).red = c;
            let sr = self.n(s).right;
            let pp = self.n(p).parent;
            if s == pr {
                // p was s's direct parent
                self.nm(p).parent = s;
                self.nm(s).right = p;
            } else {
                let sp = self.n(s).parent;
                self.nm(p).parent = sp;
                if sp != NIL {
                    if s == self.n(sp).left {
                        self.nm(sp).left = p;
                    } else {
                        self.nm(sp).right = p;
                    }
                }
                self.nm(s).right = pr;
                if pr != NIL {
                    self.nm(pr).parent = s;
                }
            }
            self.nm(p).left = NIL;
            self.nm(p).right = sr;
            if sr != NIL {
                self.nm(sr).parent = p;
            }
            self.nm(s).left = pl;
            if pl != NIL {
                self.nm(pl).parent = s;
            }
            self.nm(s).parent = pp;
            if pp == NIL {
                root = s;
            } else if p == self.n(pp).left {
                self.nm(pp).left = s;
            } else {
                self.nm(pp).right = s;
            }
            replacement = if sr != NIL { sr } else { p };
        } else if pl != NIL {
            replacement = pl;
        } else if pr != NIL {
            replacement = pr;
        } else {
            replacement = p;
        }
        if replacement != p {
            let pp = self.n(p).parent;
            self.nm(replacement).parent = pp;
            if pp == NIL {
                root = replacement;
                self.nm(replacement).red = false;
            } else if p == self.n(pp).left {
                self.nm(pp).left = replacement;
            } else {
                self.nm(pp).right = replacement;
            }
            let nd = self.nm(p);
            nd.left = NIL;
            nd.right = NIL;
            nd.parent = NIL;
        }
        let r = if self.n(p).red { root } else { self.balance_deletion(root, replacement) };
        if replacement == p {
            // detach
            let pp = self.n(p).parent;
            self.nm(p).parent = NIL;
            if pp != NIL {
                if p == self.n(pp).left {
                    self.nm(pp).left = NIL;
                } else if p == self.n(pp).right {
                    self.nm(pp).right = NIL;
                }
            }
        }
        if movable {
            self.move_root_to_front(r);
        }
    }
}

/// Iterator in Java `HashMap` order.
pub struct Iter<'a, V> {
    map: &'a JavaIntHashMap<V>,
    bin: usize,
    next: u32,
}

impl<'a, V> Iterator for Iter<'a, V> {
    type Item = (i32, &'a V);

    fn next(&mut self) -> Option<Self::Item> {
        while self.next == NIL {
            if self.bin >= self.map.table.len() {
                return None;
            }
            self.next = self.map.table[self.bin];
            self.bin += 1;
        }
        let e = self.next;
        let nd = self.map.n(e);
        self.next = nd.next;
        Some((nd.key, nd.value.as_ref().expect("live node")))
    }
}

impl<'a, V> IntoIterator for &'a JavaIntHashMap<V> {
    type Item = (i32, &'a V);
    type IntoIter = Iter<'a, V>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// `java.util.HashSet<Integer>` (a `HashMap<Integer, PRESENT>`).
#[derive(Clone, Default, Debug)]
pub struct JavaIntHashSet {
    map: JavaIntHashMap<()>,
}

impl JavaIntHashSet {
    /// `new HashSet<>()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// `new HashSet<>(initialCapacity)`.
    pub fn with_capacity(initial_capacity: i32) -> Self {
        JavaIntHashSet { map: JavaIntHashMap::with_capacity(initial_capacity) }
    }

    /// `add(e)`.
    pub fn add(&mut self, key: i32) -> bool {
        self.map.insert(key, ()).is_none()
    }

    /// `remove(o)`.
    pub fn remove(&mut self, key: i32) -> bool {
        self.map.remove(key).is_some()
    }

    /// `contains(o)`.
    pub fn contains(&self, key: i32) -> bool {
        self.map.contains_key(key)
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

    /// Elements in Java iteration order.
    pub fn iter(&self) -> impl Iterator<Item = i32> + '_ {
        self.map.keys()
    }

    /// The backing map.
    pub fn as_map(&self) -> &JavaIntHashMap<()> {
        &self.map
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_size_for_matches_java() {
        assert_eq!(table_size_for(0), 1);
        assert_eq!(table_size_for(1), 1);
        assert_eq!(table_size_for(2), 2);
        assert_eq!(table_size_for(3), 4);
        assert_eq!(table_size_for(16), 16);
        assert_eq!(table_size_for(17), 32);
        assert_eq!(table_size_for(i32::MAX), 1 << 30);
    }

    #[test]
    fn small_ints_iterate_ascending() {
        let mut m = JavaIntHashMap::new();
        for k in [5, 3, 9, 1, 12] {
            m.insert(k, k);
        }
        assert_eq!(m.keys().collect::<Vec<_>>(), vec![1, 3, 5, 9, 12]);
        // 16 and 0 share bin 0 in a 16-table: insertion order within the bin
        m.insert(16, 0);
        m.insert(0, 0);
        assert_eq!(m.keys().collect::<Vec<_>>(), vec![16, 0, 1, 3, 5, 9, 12]);
    }

    #[test]
    fn treeification_happens() {
        let mut m = JavaIntHashMap::new();
        for i in 0..12 {
            m.insert(i * 128, i);
        }
        assert!(m.has_tree_bins());
        assert_eq!(m.capacity(), 64);
        let mut keys: Vec<i32> = m.keys().collect();
        keys.sort();
        assert_eq!(keys, (0..12).map(|i| i * 128).collect::<Vec<_>>());
        for i in 0..12 {
            assert_eq!(m.remove(i * 128), Some(i));
        }
        assert!(m.is_empty());
    }
}
