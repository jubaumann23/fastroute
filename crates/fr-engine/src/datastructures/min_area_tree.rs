//! Port of `datastructures/ShapeTree.java` and `datastructures/MinAreaTree.java`.
//!
//! A binary tree of bounding shapes (`RegularTileShape`: `IntBox` for orthogonal,
//! `IntOctagon` for 45 degree bounding directions). The stored shapes are in the leaves.
//! A new shape descends from the root to the child whose bounding shape grows least
//! (area of the union minus area of the child, ties to the first child) and replaces the leaf
//! it reaches by an inner node holding the old and the new leaf.
//!
//! # Rust representation
//!
//! * Arena of nodes (`Vec` + free list, `u32` indices). A [`LeafId`] carries a generation, so a
//!   leaf id that was removed (and whose slot may have been reused) is recognised as stale:
//!   removing it again is a no-op, like removing a detached leaf in Java.
//! * Leaves store `(object, shape index, bounding shape)`. The object type `O` is a small `Copy`
//!   key chosen by the caller (the Java `ShapeTree.Storable` reference). Result ordering needs the
//!   Java `Storable.compareTo`, which is supplied as a comparator (or `O: Ord`).
//! * The Java `setSearchTreeEntries` callback is replaced by returning the leaf ids.
//! * No locks, no `ThreadLocal`: read queries take `&self` and either allocate a small stack or
//!   use a caller supplied reusable one ([`TreeCursor`]).
//! * Unlike Java, the payload of a removed leaf is not readable any more (its slot is freed).
//!
//! The tree shape depends on the exact insertion / removal sequence and it matters: the
//! complete-shape queries of the 45 and 90 degree search trees visit leaves in traversal order
//! while shrinking the query shape, so their results depend on the order. [`TreeCursor`]
//! reproduces that traversal (`ArrayStack` LIFO order: second child popped first).

use std::cmp::Ordering;
use std::sync::OnceLock;

use fr_geom::{IntBox, RegularTileShape, ShapeBoundingDirections, TileShape};

use super::leaf_grid::{xy_range, LayeredGrid};

/// Minimum number of leaves for building the secondary grid index.
const GRID_MIN_LEAVES: i32 = 128;

const NONE: u32 = u32::MAX;

/// Handle of a leaf (Java `ShapeTree.Leaf` reference).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct LeafId {
    index: u32,
    generation: u32,
}

impl LeafId {
    /// Arena slot of the leaf (stable while the leaf is in the tree).
    pub fn index(&self) -> u32 {
        self.index
    }
}

#[derive(Clone, Debug)]
enum NodeKind<O> {
    Inner { first: u32, second: u32 },
    Leaf { object: O, shape_index: i32 },
    Free { next_free: u32 },
}

#[derive(Clone, Debug)]
struct Node<O> {
    bounds: RegularTileShape,
    /// Leaf: the mask given at insertion (a superset of the layers of the leaf, see
    /// [`MinAreaTree::insert_shape_masked`]); inner node: the union of the masks of its children.
    mask: u64,
    parent: u32,
    generation: u32,
    kind: NodeKind<O>,
}

/// The traversal data of a node: the parameters of its bounding shape (`IntBox`: `ll.x, ll.y,
/// ur.x, ur.y`; `IntOctagon`: its 8 fields in declaration order), its mask and its children
/// (`first == NONE` for leaves and free nodes).
#[derive(Clone, Copy, Debug)]
struct HotNode {
    p: [i32; 8],
    mask: u64,
    first: u32,
    second: u32,
}

/// Whether all stored bounding shapes (leaves and inner nodes) have the same variant. The
/// search trees always store one variant (given by their bounding directions); then the
/// traversals use a specialized, branch-light copy of `regular_intersects` on [`HotNode::p`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BoundsKind {
    None,
    Box,
    Octagon,
    Mixed,
}

#[inline]
fn params(s: &RegularTileShape) -> ([i32; 8], BoundsKind) {
    match s {
        RegularTileShape::IntBox(b) => ([b.ll.x, b.ll.y, b.ur.x, b.ur.y, 0, 0, 0, 0], BoundsKind::Box),
        RegularTileShape::IntOctagon(o) => (
            [
                o.left_x,
                o.bottom_y,
                o.right_x,
                o.top_y,
                o.upper_left_diagonal_x,
                o.lower_right_diagonal_x,
                o.lower_left_diagonal_x,
                o.upper_right_diagonal_x,
            ],
            BoundsKind::Octagon,
        ),
    }
}

/// `regular_intersects(a, q)` for two octagons given by their parameters
/// (`IntOctagon::intersects_int_octagon`).
#[inline(always)]
fn octagon_params_intersect(a: &[i32; 8], q: &[i32; 8]) -> bool {
    (a[0].max(q[0]) <= a[2].min(q[2]))
        & (a[1].max(q[1]) <= a[3].min(q[3]))
        & (a[6].max(q[6]) <= a[7].min(q[7]))
        & (a[4].max(q[4]) <= a[5].min(q[5]))
}

/// `regular_intersects(a, q)` for two boxes given by their parameters
/// (`q.intersects_int_box(a)`).
#[inline(always)]
fn box_params_intersect(a: &[i32; 8], q: &[i32; 8]) -> bool {
    (a[0] <= q[2]) & (a[1] <= q[3]) & (q[0] <= a[2]) & (q[1] <= a[3])
}

/// Read access to a leaf.
#[derive(Clone, Copy, Debug)]
pub struct LeafRef<'a, O> {
    pub id: LeafId,
    pub object: O,
    pub shape_index_in_object: i32,
    pub bounding_shape: &'a RegularTileShape,
}

/// Result of [`MinAreaTree::statistics`] (Java `ShapeTree.statistics` logs these values).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TreeStatistics {
    pub entry_count: i32,
    pub average_depth: f64,
    pub maximum_depth: i32,
}

/// Java `MinAreaTree` (the only concrete `ShapeTree`).
#[derive(Debug)]
pub struct MinAreaTree<O> {
    bounding_directions: ShapeBoundingDirections,
    nodes: Vec<Node<O>>,
    free_head: u32,
    root: u32,
    leaf_count: i32,
    /// Incremented on every structural change; lets [`TreeCursor`] detect misuse in debug builds.
    revision: u64,
    /// Compact copy of the data the traversals read (bounds parameters, mask, children), kept in
    /// sync with `nodes` by [`Self::sync_hot`].
    hot: Vec<HotNode>,
    /// The variant of all bounding shapes (see [`BoundsKind`]).
    bounds_kind: BoundsKind,
    /// Secondary index over the leaves for [`Self::overlapping_leaves_indexed`], built lazily by
    /// the first query and maintained by insert/remove (dropped when the tree has grown a lot).
    grid: OnceLock<LayeredGrid>,
}

/// Cloning does not copy the secondary grid index (it is rebuilt lazily by the first indexed
/// query of the clone): most clones (board snapshots, deep copies) are never queried.
impl<O: Clone> Clone for MinAreaTree<O> {
    fn clone(&self) -> Self {
        MinAreaTree {
            bounding_directions: self.bounding_directions,
            nodes: self.nodes.clone(),
            free_head: self.free_head,
            root: self.root,
            leaf_count: self.leaf_count,
            revision: self.revision,
            hot: self.hot.clone(),
            bounds_kind: self.bounds_kind,
            grid: OnceLock::new(),
        }
    }
}

/// The abstract Java base class has a single implementation.
pub type ShapeTree<O> = MinAreaTree<O>;

/// Java `RegularTileShape.intersects(Shape)` for two regular tile shapes (double dispatch:
/// `a.intersects(b)` calls `b.intersects(<type of a> a)`).
#[inline]
pub fn regular_intersects(a: &RegularTileShape, b: &RegularTileShape) -> bool {
    match (a, b) {
        (RegularTileShape::IntBox(a), RegularTileShape::IntBox(b)) => b.intersects_int_box(a),
        (RegularTileShape::IntBox(a), RegularTileShape::IntOctagon(b)) => b.intersects_int_box(a),
        (RegularTileShape::IntOctagon(a), RegularTileShape::IntBox(b)) => b.intersects_int_octagon(a),
        (RegularTileShape::IntOctagon(a), RegularTileShape::IntOctagon(b)) => b.intersects_int_octagon(a),
    }
}

/// Java `RegularTileShape.area()` (virtual: `IntBox.area` / `IntOctagon.area`).
#[inline]
pub fn regular_area(s: &RegularTileShape) -> f64 {
    match s {
        RegularTileShape::IntBox(b) => b.area(),
        RegularTileShape::IntOctagon(o) => o.area(),
    }
}

impl<O: Copy> MinAreaTree<O> {
    /// Java `new MinAreaTree(directions)`.
    pub fn new(bounding_directions: ShapeBoundingDirections) -> Self {
        MinAreaTree {
            bounding_directions,
            nodes: Vec::new(),
            free_head: NONE,
            root: NONE,
            leaf_count: 0,
            revision: 0,
            hot: Vec::new(),
            bounds_kind: BoundsKind::None,
            grid: OnceLock::new(),
        }
    }

    /// Java `boundingDirections`.
    pub fn bounding_directions(&self) -> ShapeBoundingDirections {
        self.bounding_directions
    }

    /// Java `size()`: the number of leaves.
    pub fn size(&self) -> i32 {
        self.leaf_count
    }

    /// Java `root == null`.
    pub fn is_empty(&self) -> bool {
        self.root == NONE
    }

    /// Structural revision counter (changes on every insert/remove).
    pub fn revision(&self) -> u64 {
        self.revision
    }

    // ----------------------------------------------------------------------------------------
    // arena

    fn alloc(&mut self, bounds: RegularTileShape, mask: u64, parent: u32, kind: NodeKind<O>) -> u32 {
        if self.free_head != NONE {
            let idx = self.free_head;
            let node = &mut self.nodes[idx as usize];
            self.free_head = match node.kind {
                NodeKind::Free { next_free } => next_free,
                _ => unreachable!("free list corrupted"),
            };
            node.bounds = bounds;
            node.mask = mask;
            node.parent = parent;
            node.kind = kind;
            self.sync_hot(idx);
            idx
        } else {
            let idx = u32::try_from(self.nodes.len()).expect("MinAreaTree: too many nodes");
            assert!(idx != NONE, "MinAreaTree: too many nodes");
            self.nodes.push(Node { bounds, mask, parent, generation: 0, kind });
            self.hot.push(HotNode { p: [0; 8], mask: 0, first: NONE, second: NONE });
            self.sync_hot(idx);
            idx
        }
    }

    /// Copies the traversal data of node `idx` to `hot` and updates `bounds_kind`.
    #[inline]
    fn sync_hot(&mut self, idx: u32) {
        let node = &self.nodes[idx as usize];
        let (first, second) = match node.kind {
            NodeKind::Inner { first, second } => (first, second),
            _ => (NONE, NONE),
        };
        if matches!(node.kind, NodeKind::Free { .. }) {
            self.hot[idx as usize] = HotNode { p: [0; 8], mask: 0, first, second };
            return;
        }
        let (p, kind) = params(&node.bounds);
        self.hot[idx as usize] = HotNode { p, mask: node.mask, first, second };
        if self.bounds_kind != kind {
            self.bounds_kind = if self.bounds_kind == BoundsKind::None { kind } else { BoundsKind::Mixed };
        }
    }

    /// The specialized intersection test for `query`, if all bounds and the query have the same
    /// variant: returns the query parameters and whether they are octagon parameters.
    #[inline]
    fn fast_query(&self, query: &RegularTileShape) -> Option<([i32; 8], bool)> {
        let (q, kind) = params(query);
        if kind == self.bounds_kind {
            Some((q, kind == BoundsKind::Octagon))
        } else {
            None
        }
    }

    fn free(&mut self, idx: u32) {
        let node = &mut self.nodes[idx as usize];
        node.generation = node.generation.wrapping_add(1);
        node.parent = NONE;
        node.bounds = RegularTileShape::IntBox(IntBox::EMPTY);
        node.mask = 0;
        node.kind = NodeKind::Free { next_free: self.free_head };
        self.free_head = idx;
        self.sync_hot(idx);
    }

    #[inline]
    fn leaf_id_of(&self, idx: u32) -> LeafId {
        LeafId { index: idx, generation: self.nodes[idx as usize].generation }
    }

    /// Returns true, if `id` denotes a leaf currently stored in this tree.
    pub fn contains_leaf(&self, id: LeafId) -> bool {
        match self.nodes.get(id.index as usize) {
            Some(node) => node.generation == id.generation && matches!(node.kind, NodeKind::Leaf { .. }),
            None => false,
        }
    }

    /// Read access to a stored leaf; `None` if the leaf was removed.
    pub fn leaf(&self, id: LeafId) -> Option<LeafRef<'_, O>> {
        let node = self.nodes.get(id.index as usize)?;
        if node.generation != id.generation {
            return None;
        }
        match node.kind {
            NodeKind::Leaf { object, shape_index } => Some(LeafRef {
                id,
                object,
                shape_index_in_object: shape_index,
                bounding_shape: &node.bounds,
            }),
            _ => None,
        }
    }

    /// Java `leaf.object` (panics for a removed leaf, like a stale arena access would be a bug).
    pub fn leaf_object(&self, id: LeafId) -> O {
        self.leaf(id).expect("MinAreaTree: stale leaf id").object
    }

    /// Java `leaf.shapeIndexInObject`.
    pub fn leaf_shape_index(&self, id: LeafId) -> i32 {
        self.leaf(id).expect("MinAreaTree: stale leaf id").shape_index_in_object
    }

    /// Java `leaf.boundingShape`.
    pub fn leaf_bounding_shape(&self, id: LeafId) -> &RegularTileShape {
        self.leaf(id).expect("MinAreaTree: stale leaf id").bounding_shape
    }

    fn leaf_payload_mut(&mut self, id: LeafId) -> (&mut O, &mut i32) {
        let node = self.nodes.get_mut(id.index as usize).expect("MinAreaTree: invalid leaf id");
        assert!(node.generation == id.generation, "MinAreaTree: stale leaf id");
        match &mut node.kind {
            NodeKind::Leaf { object, shape_index } => (object, shape_index),
            _ => panic!("MinAreaTree: stale leaf id"),
        }
    }

    /// Java `leaf.object = object` (used by `ShapeSearchTree.mergeEntries*` / `splitEntries`,
    /// which move leaves between traces without reinserting them).
    pub fn set_leaf_object(&mut self, id: LeafId, object: O) {
        *self.leaf_payload_mut(id).0 = object;
    }

    /// Java `leaf.shapeIndexInObject = index`.
    pub fn set_leaf_shape_index(&mut self, id: LeafId, shape_index: i32) {
        *self.leaf_payload_mut(id).1 = shape_index;
    }

    // ----------------------------------------------------------------------------------------
    // insertion

    /// Java `ShapeTree.insert(Storable)`: inserts all shapes of `object`, shape `i` with shape
    /// index `i`. Returns the leaf entries (Java `setSearchTreeEntries`); an entry is `None`
    /// where the shape has no bounding shape. For an empty slice Java returns without calling
    /// `setSearchTreeEntries`; here an empty vector is returned.
    pub fn insert_shapes(&mut self, object: O, shapes: &[TileShape]) -> Vec<Option<LeafId>> {
        let mut result = Vec::with_capacity(shapes.len());
        for (i, shape) in shapes.iter().enumerate() {
            result.push(self.insert_shape(object, i as i32, shape));
        }
        result
    }

    /// Java `ShapeTree.insert(Storable, index)`: computes the bounding shape of `shape` with the
    /// directions of this tree and inserts a new leaf. `None` (with a warning) if the shape has
    /// no bounding shape.
    pub fn insert_shape(&mut self, object: O, shape_index: i32, shape: &TileShape) -> Option<LeafId> {
        self.insert_shape_masked(object, shape_index, shape, u64::MAX)
    }

    /// [`Self::insert_shape`] with a mask for the pruned queries ([`TreeCursor::next_leaf_masked`],
    /// [`Self::overlapping_leaves_masked`]): a query with mask `m` skips the leaves (and whole
    /// subtrees) whose mask has no bit in common with `m`. It must stay valid while the leaf is
    /// stored (the search trees use the bit of the layer of the leaf).
    pub fn insert_shape_masked(&mut self, object: O, shape_index: i32, shape: &TileShape, mask: u64) -> Option<LeafId> {
        let Some(bounding_shape) = shape.bounding_shape(&self.bounding_directions) else {
            log::warn!("ShapeTree.insert: bounding shape of TreeObject is null");
            return None;
        };
        Some(self.insert_bounds_masked(object, shape_index, bounding_shape, mask))
    }

    /// Java `new Leaf(object, index, null, boundingShape)` followed by `insert(Leaf)`.
    pub fn insert_bounds(&mut self, object: O, shape_index: i32, bounding_shape: RegularTileShape) -> LeafId {
        self.insert_bounds_masked(object, shape_index, bounding_shape, u64::MAX)
    }

    /// [`Self::insert_bounds`] with a mask (see [`Self::insert_shape_masked`]).
    pub fn insert_bounds_masked(&mut self, object: O, shape_index: i32, bounding_shape: RegularTileShape, mask: u64) -> LeafId {
        let leaf = self.alloc(bounding_shape, mask, NONE, NodeKind::Leaf { object, shape_index });
        self.insert_leaf_node(leaf);
        self.leaf_id_of(leaf)
    }

    /// Java `MinAreaTree.insertUnlocked(Leaf)`.
    fn insert_leaf_node(&mut self, leaf: u32) {
        self.revision += 1;
        self.leaf_count += 1;
        if let Some(grid) = self.grid.get_mut() {
            if self.leaf_count > 4 * grid.built_leaf_count.max(GRID_MIN_LEAVES) {
                self.grid = OnceLock::new();
            } else {
                grid.insert(leaf, &self.nodes[leaf as usize].bounds, self.nodes[leaf as usize].mask);
            }
        }

        // Tree is empty - just insert the new leaf
        if self.root == NONE {
            self.root = leaf;
            return;
        }

        // Non-empty tree - do a recursive location for leaf replacement
        let leaf_bounds = self.nodes[leaf as usize].bounds;
        let leaf_mask = self.nodes[leaf as usize].mask;
        let leaf_to_replace = self.position_locate(&leaf_bounds, leaf_mask);

        // Construct a new node - whenever a leaf is added so is a new node
        let new_bounds = leaf_bounds.union(&self.nodes[leaf_to_replace as usize].bounds);
        let new_mask = leaf_mask | self.nodes[leaf_to_replace as usize].mask;
        let current_parent = self.nodes[leaf_to_replace as usize].parent;
        let new_node = self.alloc(new_bounds, new_mask, current_parent, NodeKind::Inner { first: leaf_to_replace, second: leaf });

        if current_parent != NONE {
            // Replace the pointer from the parent to the leaf with our new node
            if let NodeKind::Inner { first, second } = &mut self.nodes[current_parent as usize].kind {
                if *first == leaf_to_replace {
                    *first = new_node;
                } else {
                    *second = new_node;
                }
            }
            self.sync_hot(current_parent);
        }
        // Update the parent pointers of the old leaf and new leaf to point to new node
        self.nodes[leaf_to_replace as usize].parent = new_node;
        self.nodes[leaf as usize].parent = new_node;

        if self.root == leaf_to_replace {
            self.root = new_node;
        }
    }

    /// Java `MinAreaTree.positionLocate`: descends to the child with the minimal area increase,
    /// enlarging the bounding shapes of the visited inner nodes on the way.
    fn position_locate(&mut self, leaf_bounds: &RegularTileShape, leaf_mask: u64) -> u32 {
        let mut node = self.root;
        loop {
            let (first, second) = match self.nodes[node as usize].kind {
                NodeKind::Inner { first, second } => (first, second),
                NodeKind::Leaf { .. } => return node,
                NodeKind::Free { .. } => unreachable!("MinAreaTree: free node reachable"),
            };
            let enlarged = leaf_bounds.union(&self.nodes[node as usize].bounds);
            self.nodes[node as usize].bounds = enlarged;
            self.nodes[node as usize].mask |= leaf_mask;
            self.sync_hot(node);

            // Choose the child, so that the area increase of that child after taking the union
            // with the shape of leafToInsert is minimal.
            let first_child_shape = &self.nodes[first as usize].bounds;
            let union_with_first = leaf_bounds.union(first_child_shape);
            let first_area_increase = regular_area(&union_with_first) - regular_area(first_child_shape);

            let second_child_shape = &self.nodes[second as usize].bounds;
            let union_with_second = leaf_bounds.union(second_child_shape);
            let second_area_increase = regular_area(&union_with_second) - regular_area(second_child_shape);

            node = if first_area_increase <= second_area_increase { first } else { second };
        }
    }

    // ----------------------------------------------------------------------------------------
    // removal

    /// Java `ShapeTree.remove(Leaf[])`: removes all given entries (`None` entries are skipped,
    /// like Java `removeLeaf(null)`).
    pub fn remove(&mut self, entries: &[Option<LeafId>]) {
        for entry in entries.iter().flatten() {
            self.remove_leaf(*entry);
        }
    }

    /// Java `MinAreaTree.removeLeaf`. Removing a leaf that is no longer in the tree is a no-op.
    pub fn remove_leaf(&mut self, id: LeafId) {
        if !self.contains_leaf(id) {
            return;
        }
        let leaf = id.index;
        let parent = self.nodes[leaf as usize].parent;
        if parent == NONE && self.root != leaf {
            return;
        }
        self.revision += 1;
        self.leaf_count -= 1;
        if let Some(grid) = self.grid.get_mut() {
            grid.remove(leaf, &self.nodes[leaf as usize].bounds, self.nodes[leaf as usize].mask);
        }
        self.free(leaf);
        if parent == NONE {
            // tree gets empty
            self.root = NONE;
            return;
        }
        // find the other leaf of the parent
        let other = match self.nodes[parent as usize].kind {
            NodeKind::Inner { first, second } => {
                if second == leaf {
                    first
                } else if first == leaf {
                    second
                } else {
                    // Java logs "parent inconsistent" and then dereferences null.
                    panic!("MinAreaTree.remove_leaf: parent inconsistent");
                }
            }
            _ => unreachable!("MinAreaTree: leaf parent is not an inner node"),
        };
        // link the other leaf to the grandParent and remove the parent node
        let grand_parent = self.nodes[parent as usize].parent;
        self.nodes[other as usize].parent = grand_parent;
        if grand_parent == NONE {
            // only one leaf left in the tree
            self.root = other;
        } else if let NodeKind::Inner { first, second } = &mut self.nodes[grand_parent as usize].kind {
            if *second == parent {
                *second = other;
            } else if *first == parent {
                *first = other;
            } else {
                log::warn!("MinAreaTree.remove_leaf: grandParent inconsistent");
            }
            self.sync_hot(grand_parent);
        }
        self.free(parent);

        // recalculate the masks of the ancestors (as long as they change)
        let mut node_to_recalculate = grand_parent;
        while node_to_recalculate != NONE {
            let (first, second) = match self.nodes[node_to_recalculate as usize].kind {
                NodeKind::Inner { first, second } => (first, second),
                _ => unreachable!("MinAreaTree: ancestor is not an inner node"),
            };
            let new_mask = self.nodes[first as usize].mask | self.nodes[second as usize].mask;
            if new_mask == self.nodes[node_to_recalculate as usize].mask {
                break;
            }
            self.nodes[node_to_recalculate as usize].mask = new_mask;
            self.sync_hot(node_to_recalculate);
            node_to_recalculate = self.nodes[node_to_recalculate as usize].parent;
        }

        // recalculate the bounding shapes of the ancestors
        // as long as it gets smaller after removing leaf
        let mut node_to_recalculate = grand_parent;
        while node_to_recalculate != NONE {
            let (first, second) = match self.nodes[node_to_recalculate as usize].kind {
                NodeKind::Inner { first, second } => (first, second),
                _ => unreachable!("MinAreaTree: ancestor is not an inner node"),
            };
            let new_bounds = self.nodes[second as usize].bounds.union(&self.nodes[first as usize].bounds);
            if new_bounds.contains(&self.nodes[node_to_recalculate as usize].bounds) {
                // the new bounds are not smaller, no further recalculate necessary
                break;
            }
            self.nodes[node_to_recalculate as usize].bounds = new_bounds;
            self.sync_hot(node_to_recalculate);
            node_to_recalculate = self.nodes[node_to_recalculate as usize].parent;
        }
    }

    // ----------------------------------------------------------------------------------------
    // queries

    /// Java `ShapeTree.toArray()`: the leaves from left to right.
    pub fn to_array(&self) -> Vec<LeafId> {
        let mut result = Vec::with_capacity(self.leaf_count.max(0) as usize);
        if self.root == NONE {
            return result;
        }
        let mut current = self.root;
        loop {
            // go down from currentNode to the left most leaf
            while let NodeKind::Inner { first, .. } = self.nodes[current as usize].kind {
                current = first;
            }
            result.push(self.leaf_id_of(current));
            // go up until parent.secondChild != currentNode, which means we came from firstChild
            let mut parent = self.nodes[current as usize].parent;
            while parent != NONE && self.second_child(parent) == current {
                current = parent;
                parent = self.nodes[current as usize].parent;
            }
            if parent == NONE {
                break;
            }
            current = self.second_child(parent);
        }
        result
    }

    #[inline]
    fn second_child(&self, inner: u32) -> u32 {
        match self.nodes[inner as usize].kind {
            NodeKind::Inner { second, .. } => second,
            _ => unreachable!(),
        }
    }

    /// The leaves whose bounding shape intersects `shape`, in traversal order (unsorted).
    pub fn overlapping_leaves_unsorted(&self, shape: &RegularTileShape, stack: &mut Vec<u32>, out: &mut Vec<LeafId>) {
        stack.clear();
        if self.root == NONE {
            return;
        }
        stack.push(self.root);
        if let Some((q, oct)) = self.fast_query(shape) {
            while let Some(n) = if oct { walk::<false, true>(&self.hot, stack, &q, 0) } else { walk::<false, false>(&self.hot, stack, &q, 0) } {
                out.push(LeafId { index: n, generation: self.nodes[n as usize].generation });
            }
            return;
        }
        while let Some(n) = stack.pop() {
            let node = &self.nodes[n as usize];
            if regular_intersects(&node.bounds, shape) {
                match node.kind {
                    NodeKind::Leaf { .. } => out.push(LeafId { index: n, generation: node.generation }),
                    NodeKind::Inner { first, second } => {
                        stack.push(first);
                        stack.push(second);
                    }
                    NodeKind::Free { .. } => unreachable!(),
                }
            }
        }
    }

    /// [`Self::overlapping_leaves_unsorted`] restricted to the leaves whose mask intersects
    /// `mask` (subtrees without such leaves are skipped).
    pub fn overlapping_leaves_masked(&self, shape: &RegularTileShape, mask: u64, stack: &mut Vec<u32>, out: &mut Vec<LeafId>) {
        stack.clear();
        if self.root == NONE {
            return;
        }
        stack.push(self.root);
        if let Some((q, oct)) = self.fast_query(shape) {
            while let Some(n) = if oct { walk::<true, true>(&self.hot, stack, &q, mask) } else { walk::<true, false>(&self.hot, stack, &q, mask) } {
                out.push(LeafId { index: n, generation: self.nodes[n as usize].generation });
            }
            return;
        }
        while let Some(n) = stack.pop() {
            let node = &self.nodes[n as usize];
            if node.mask & mask != 0 && regular_intersects(&node.bounds, shape) {
                match node.kind {
                    NodeKind::Leaf { .. } => out.push(LeafId { index: n, generation: node.generation }),
                    NodeKind::Inner { first, second } => {
                        stack.push(first);
                        stack.push(second);
                    }
                    NodeKind::Free { .. } => unreachable!(),
                }
            }
        }
    }

    /// Checks that the mask of every inner node is the union of the masks of its children and
    /// that `leaf_ok(object, shape_index, mask)` holds for every leaf (for tests and the
    /// `FASTROUTE_VERIFY_CACHES` mode).
    pub fn validate_masks(&self, mut leaf_ok: impl FnMut(O, i32, u64) -> bool) -> Result<(), String> {
        if self.root == NONE {
            return Ok(());
        }
        let mut stack = vec![self.root];
        while let Some(n) = stack.pop() {
            let node = &self.nodes[n as usize];
            match node.kind {
                NodeKind::Leaf { object, shape_index } => {
                    if !leaf_ok(object, shape_index, node.mask) {
                        return Err(format!("leaf {n}: mask {:#x} rejected", node.mask));
                    }
                }
                NodeKind::Inner { first, second } => {
                    if node.mask != self.nodes[first as usize].mask | self.nodes[second as usize].mask {
                        return Err(format!("inner node {n}: mask is not the union of the children"));
                    }
                    stack.push(first);
                    stack.push(second);
                }
                NodeKind::Free { .. } => return Err(format!("free node {n} reachable")),
            }
        }
        Ok(())
    }

    /// The mask of a stored leaf.
    pub fn leaf_mask(&self, id: LeafId) -> u64 {
        self.nodes[id.index as usize].mask
    }

    /// The same set of leaves as [`Self::overlapping_leaves_unsorted`] (all leaves whose
    /// bounding shape intersects `shape`; the bounding shapes of the inner nodes contain those of
    /// their children, so the traversal reaches every such leaf), in unspecified order, answered
    /// from the secondary grid index when the tree is large enough.
    /// Restricted to the leaves whose mask intersects `mask` (`u64::MAX`: all leaves).
    pub fn overlapping_leaves_indexed(&self, shape: &RegularTileShape, mask: u64, stack: &mut Vec<u32>, out: &mut Vec<LeafId>) {
        if self.leaf_count < GRID_MIN_LEAVES {
            self.overlapping_leaves_masked(shape, mask, stack, out);
            return;
        }
        let grid = self.grid.get_or_init(|| self.build_grid());
        let supported = grid.candidates(shape, mask, |n| {
            let node = &self.nodes[n as usize];
            if regular_intersects(&node.bounds, shape) {
                out.push(LeafId { index: n, generation: node.generation });
            }
        });
        if !supported {
            self.overlapping_leaves_masked(shape, mask, stack, out);
        }
    }

    fn build_grid(&self) -> LayeredGrid {
        let extent = xy_range(&self.nodes[self.root as usize].bounds);
        let mut counts = [0i64; 65];
        for node in &self.nodes {
            if let NodeKind::Leaf { .. } = node.kind {
                if node.mask.count_ones() == 1 {
                    counts[node.mask.trailing_zeros() as usize] += 1;
                } else {
                    counts[64] += 1;
                }
            }
        }
        let mut grid = LayeredGrid::new(extent, &counts, self.leaf_count);
        for (i, node) in self.nodes.iter().enumerate() {
            if let NodeKind::Leaf { .. } = node.kind {
                grid.insert(i as u32, &node.bounds, node.mask);
            }
        }
        grid
    }

    /// Java `Leaf.compareTo`: `object.compareTo(other.object)`, then the shape indices.
    pub fn compare_leaves(&self, a: LeafId, b: LeafId, mut cmp: impl FnMut(&O, &O) -> Ordering) -> Ordering {
        let (oa, ia) = self.payload(a);
        let (ob, ib) = self.payload(b);
        leaf_order(&oa, ia, &ob, ib, &mut cmp)
    }

    /// The object and shape index of a leaf known to be stored (unchecked generation).
    #[inline]
    pub fn leaf_payload(&self, id: LeafId) -> (O, i32) {
        self.payload(id)
    }

    #[inline]
    fn payload(&self, id: LeafId) -> (O, i32) {
        match self.nodes[id.index as usize].kind {
            NodeKind::Leaf { object, shape_index } => (object, shape_index),
            _ => panic!("MinAreaTree: stale leaf id"),
        }
    }

    /// Java `MinAreaTree.overlaps(RegularTileShape)` with a reusable stack and output vector
    /// (`out` is cleared first). The result is sorted with Java `Leaf.compareTo`, where `cmp` is
    /// the `Storable.compareTo` of the objects. The sort is stable (like Java's), applied to the
    /// same traversal order as Java.
    pub fn overlaps_into(
        &self,
        shape: &RegularTileShape,
        stack: &mut Vec<u32>,
        out: &mut Vec<LeafId>,
        mut cmp: impl FnMut(&O, &O) -> Ordering,
    ) {
        out.clear();
        self.overlapping_leaves_unsorted(shape, stack, out);
        out.sort_by(|a, b| {
            let (oa, ia) = self.payload(*a);
            let (ob, ib) = self.payload(*b);
            leaf_order(&oa, ia, &ob, ib, &mut cmp)
        });
    }

    /// Java `MinAreaTree.overlaps(RegularTileShape)` with a comparator for the objects.
    pub fn overlaps_by(&self, shape: &RegularTileShape, cmp: impl FnMut(&O, &O) -> Ordering) -> Vec<LeafId> {
        let mut stack = Vec::new();
        let mut out = Vec::new();
        self.overlaps_into(shape, &mut stack, &mut out, cmp);
        out
    }

    /// Java `MinAreaTree.overlaps(RegularTileShape)` where `O: Ord` is the Java `compareTo`.
    pub fn overlaps(&self, shape: &RegularTileShape) -> Vec<LeafId>
    where
        O: Ord,
    {
        self.overlaps_by(shape, |a, b| a.cmp(b))
    }

    /// Java `Leaf.distanceToRoot()`: the number of inner nodes above the leaf. (Java throws a
    /// NullPointerException for a leaf that is the root; 0 is returned here.)
    pub fn distance_to_root(&self, id: LeafId) -> i32 {
        assert!(self.contains_leaf(id), "MinAreaTree: stale leaf id");
        let mut result = 0;
        let mut p = self.nodes[id.index as usize].parent;
        while p != NONE {
            result += 1;
            p = self.nodes[p as usize].parent;
        }
        result
    }

    /// Java `ShapeTree.statistics(message)` (returns the values instead of logging them).
    pub fn statistics(&self) -> TreeStatistics {
        let leaves = self.to_array();
        let mut cumulative_depth = 0.0;
        let mut maximum_depth = 0;
        for leaf in &leaves {
            let d = self.distance_to_root(*leaf);
            cumulative_depth += d as f64;
            maximum_depth = maximum_depth.max(d);
        }
        TreeStatistics {
            entry_count: leaves.len() as i32,
            average_depth: cumulative_depth / leaves.len() as f64,
            maximum_depth,
        }
    }

    /// Checks the structural invariants (parent/child links, leaf count, every inner bounding
    /// shape contains the bounding shapes of its children). For tests and debugging.
    pub fn validate(&self) -> Result<(), String> {
        if self.root == NONE {
            return if self.leaf_count == 0 { Ok(()) } else { Err(format!("empty tree with leaf_count {}", self.leaf_count)) };
        }
        if self.nodes[self.root as usize].parent != NONE {
            return Err("root has a parent".into());
        }
        let mut leaves = 0;
        let mut stack = vec![self.root];
        while let Some(n) = stack.pop() {
            let node = &self.nodes[n as usize];
            match node.kind {
                NodeKind::Leaf { .. } => leaves += 1,
                NodeKind::Inner { first, second } => {
                    for c in [first, second] {
                        let child = &self.nodes[c as usize];
                        if child.parent != n {
                            return Err(format!("child {c} of {n} has parent {}", child.parent));
                        }
                        if !node.bounds.contains(&child.bounds) {
                            return Err(format!("bounds of {n} do not contain bounds of child {c}"));
                        }
                        stack.push(c);
                    }
                }
                NodeKind::Free { .. } => return Err(format!("free node {n} reachable")),
            }
        }
        if leaves != self.leaf_count {
            return Err(format!("leaf_count {} but {} leaves reachable", self.leaf_count, leaves));
        }
        Ok(())
    }
}

#[inline]
fn leaf_order<O>(oa: &O, ia: i32, ob: &O, ib: i32, cmp: &mut impl FnMut(&O, &O) -> Ordering) -> Ordering {
    match cmp(oa, ob) {
        Ordering::Equal => ia.wrapping_sub(ib).cmp(&0),
        other => other,
    }
}

/// Reusable depth-first traversal over the leaves of a [`MinAreaTree`], reproducing the
/// `ArrayStack` traversal of `ShapeSearchTree45Degree/90Degree.completeShape`: pop a node,
/// test its bounding shape against the *current* query shape, push `firstChild` then
/// `secondChild` of inner nodes. The query shape may change between calls to
/// [`next_leaf`](Self::next_leaf) (the Java loop shrinks it while processing obstacles).
///
/// The tree must not be modified during a traversal (Java holds the read lock).
#[derive(Clone, Debug, Default)]
pub struct TreeCursor {
    stack: Vec<u32>,
    revision: u64,
}

impl TreeCursor {
    pub fn new() -> Self {
        TreeCursor { stack: Vec::with_capacity(64), revision: 0 }
    }

    /// Java `stack.reset(); stack.push(root)`.
    pub fn start<O: Copy>(&mut self, tree: &MinAreaTree<O>) {
        self.stack.clear();
        self.revision = tree.revision;
        if tree.root != NONE {
            self.stack.push(tree.root);
        }
    }

    /// [`Self::next_leaf`] skipping the leaves and subtrees whose mask has no bit in common with
    /// `mask`. If the caller ignores all such leaves (and they do not change the query), the
    /// sequence of the other leaves is the same as with [`Self::next_leaf`].
    pub fn next_leaf_masked<O: Copy>(&mut self, tree: &MinAreaTree<O>, query: &RegularTileShape, mask: u64) -> Option<LeafId> {
        debug_assert_eq!(self.revision, tree.revision, "MinAreaTree modified during traversal");
        if let Some((q, oct)) = tree.fast_query(query) {
            let n = if oct { walk::<true, true>(&tree.hot, &mut self.stack, &q, mask) } else { walk::<true, false>(&tree.hot, &mut self.stack, &q, mask) };
            return n.map(|n| LeafId { index: n, generation: tree.nodes[n as usize].generation });
        }
        while let Some(n) = self.stack.pop() {
            let node = &tree.nodes[n as usize];
            if node.mask & mask != 0 && regular_intersects(&node.bounds, query) {
                match node.kind {
                    NodeKind::Leaf { .. } => return Some(LeafId { index: n, generation: node.generation }),
                    NodeKind::Inner { first, second } => {
                        self.stack.push(first);
                        self.stack.push(second);
                    }
                    NodeKind::Free { .. } => unreachable!(),
                }
            }
        }
        None
    }

    /// Continues the traversal and returns the next leaf whose bounding shape intersects
    /// `query`, or `None` when the stack is exhausted.
    pub fn next_leaf<O: Copy>(&mut self, tree: &MinAreaTree<O>, query: &RegularTileShape) -> Option<LeafId> {
        debug_assert_eq!(self.revision, tree.revision, "MinAreaTree modified during traversal");
        if let Some((q, oct)) = tree.fast_query(query) {
            let n = if oct { walk::<false, true>(&tree.hot, &mut self.stack, &q, 0) } else { walk::<false, false>(&tree.hot, &mut self.stack, &q, 0) };
            return n.map(|n| LeafId { index: n, generation: tree.nodes[n as usize].generation });
        }
        while let Some(n) = self.stack.pop() {
            let node = &tree.nodes[n as usize];
            if regular_intersects(&node.bounds, query) {
                match node.kind {
                    NodeKind::Leaf { .. } => return Some(LeafId { index: n, generation: node.generation }),
                    NodeKind::Inner { first, second } => {
                        self.stack.push(first);
                        self.stack.push(second);
                    }
                    NodeKind::Free { .. } => unreachable!(),
                }
            }
        }
        None
    }
}

/// One step of the Java `ArrayStack` traversal on the compact node data: pops nodes until a leaf
/// whose bounds intersect the query `q` is found (inner nodes whose bounds intersect push
/// `first`, then `second`). `MASKED`: also skip nodes whose mask has no bit in common with
/// `mask`. `OCT`: octagon (else box) parameters.
#[inline(always)]
fn walk<const MASKED: bool, const OCT: bool>(hot: &[HotNode], stack: &mut Vec<u32>, q: &[i32; 8], mask: u64) -> Option<u32> {
    while let Some(n) = stack.pop() {
        let h = &hot[n as usize];
        if MASKED && h.mask & mask == 0 {
            continue;
        }
        let hit = if OCT { octagon_params_intersect(&h.p, q) } else { box_params_intersect(&h.p, q) };
        if hit {
            if h.first == NONE {
                return Some(n);
            }
            stack.push(h.first);
            stack.push(h.second);
        }
    }
    None
}

#[cfg(test)]
#[path = "tests/min_area_tree.rs"]
mod tests;
