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

use fr_geom::{IntBox, RegularTileShape, ShapeBoundingDirections, TileShape};

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
    parent: u32,
    generation: u32,
    kind: NodeKind<O>,
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
#[derive(Clone, Debug)]
pub struct MinAreaTree<O> {
    bounding_directions: ShapeBoundingDirections,
    nodes: Vec<Node<O>>,
    free_head: u32,
    root: u32,
    leaf_count: i32,
    /// Incremented on every structural change; lets [`TreeCursor`] detect misuse in debug builds.
    revision: u64,
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

    fn alloc(&mut self, bounds: RegularTileShape, parent: u32, kind: NodeKind<O>) -> u32 {
        if self.free_head != NONE {
            let idx = self.free_head;
            let node = &mut self.nodes[idx as usize];
            self.free_head = match node.kind {
                NodeKind::Free { next_free } => next_free,
                _ => unreachable!("free list corrupted"),
            };
            node.bounds = bounds;
            node.parent = parent;
            node.kind = kind;
            idx
        } else {
            let idx = u32::try_from(self.nodes.len()).expect("MinAreaTree: too many nodes");
            assert!(idx != NONE, "MinAreaTree: too many nodes");
            self.nodes.push(Node { bounds, parent, generation: 0, kind });
            idx
        }
    }

    fn free(&mut self, idx: u32) {
        let node = &mut self.nodes[idx as usize];
        node.generation = node.generation.wrapping_add(1);
        node.parent = NONE;
        node.bounds = RegularTileShape::IntBox(IntBox::EMPTY);
        node.kind = NodeKind::Free { next_free: self.free_head };
        self.free_head = idx;
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
        let Some(bounding_shape) = shape.bounding_shape(&self.bounding_directions) else {
            log::warn!("ShapeTree.insert: bounding shape of TreeObject is null");
            return None;
        };
        Some(self.insert_bounds(object, shape_index, bounding_shape))
    }

    /// Java `new Leaf(object, index, null, boundingShape)` followed by `insert(Leaf)`.
    pub fn insert_bounds(&mut self, object: O, shape_index: i32, bounding_shape: RegularTileShape) -> LeafId {
        let leaf = self.alloc(bounding_shape, NONE, NodeKind::Leaf { object, shape_index });
        self.insert_leaf_node(leaf);
        self.leaf_id_of(leaf)
    }

    /// Java `MinAreaTree.insertUnlocked(Leaf)`.
    fn insert_leaf_node(&mut self, leaf: u32) {
        self.revision += 1;
        self.leaf_count += 1;

        // Tree is empty - just insert the new leaf
        if self.root == NONE {
            self.root = leaf;
            return;
        }

        // Non-empty tree - do a recursive location for leaf replacement
        let leaf_bounds = self.nodes[leaf as usize].bounds;
        let leaf_to_replace = self.position_locate(&leaf_bounds);

        // Construct a new node - whenever a leaf is added so is a new node
        let new_bounds = leaf_bounds.union(&self.nodes[leaf_to_replace as usize].bounds);
        let current_parent = self.nodes[leaf_to_replace as usize].parent;
        let new_node = self.alloc(new_bounds, current_parent, NodeKind::Inner { first: leaf_to_replace, second: leaf });

        if current_parent != NONE {
            // Replace the pointer from the parent to the leaf with our new node
            if let NodeKind::Inner { first, second } = &mut self.nodes[current_parent as usize].kind {
                if *first == leaf_to_replace {
                    *first = new_node;
                } else {
                    *second = new_node;
                }
            }
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
    fn position_locate(&mut self, leaf_bounds: &RegularTileShape) -> u32 {
        let mut node = self.root;
        loop {
            let (first, second) = match self.nodes[node as usize].kind {
                NodeKind::Inner { first, second } => (first, second),
                NodeKind::Leaf { .. } => return node,
                NodeKind::Free { .. } => unreachable!("MinAreaTree: free node reachable"),
            };
            let enlarged = leaf_bounds.union(&self.nodes[node as usize].bounds);
            self.nodes[node as usize].bounds = enlarged;

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
        }
        self.free(parent);

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

    /// Java `Leaf.compareTo`: `object.compareTo(other.object)`, then the shape indices.
    pub fn compare_leaves(&self, a: LeafId, b: LeafId, mut cmp: impl FnMut(&O, &O) -> Ordering) -> Ordering {
        let (oa, ia) = self.payload(a);
        let (ob, ib) = self.payload(b);
        leaf_order(&oa, ia, &ob, ib, &mut cmp)
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

    /// Continues the traversal and returns the next leaf whose bounding shape intersects
    /// `query`, or `None` when the stack is exhausted.
    pub fn next_leaf<O: Copy>(&mut self, tree: &MinAreaTree<O>, query: &RegularTileShape) -> Option<LeafId> {
        debug_assert_eq!(self.revision, tree.revision, "MinAreaTree modified during traversal");
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

#[cfg(test)]
#[path = "tests/min_area_tree.rs"]
mod tests;
