//! Port of `board/searchtree/{ShapeSearchTree, ShapeSearchTree45Degree, ShapeSearchTree90Degree,
//! SearchTreeManager, SearchTreeObject}` and `board/actions/ItemSearchTreesInfo`.
//!
//! * A [`ShapeSearchTree`] is a [`MinAreaTree`] of [`TreeObject`]s (board items and complete
//!   free space expansion rooms) plus side tables: per item the leaves and the precalculated
//!   tree shapes (Java `ItemSearchTreesInfo`), per room its shape, layer, id and leaf.
//! * The three Java classes are one struct with a [`TreeKind`]; the overridden methods
//!   (`calculateTreeShapes`, `offsetShape(s)`, `completeShape`, `divideLargeRoom`) dispatch on
//!   the kind.
//! * [`SearchTreeManager`] holds the trees in Java list order; the default tree is always the
//!   first. Trees are identified by their compensated clearance class number (Java
//!   `getAutorouteTree` returns the first tree with that number).
//! * Queries needing item data are methods of [`BasicBoard`] taking the tree index.
//! * Result order: Java `MinAreaTree.overlaps` sorts the leaves with `Leaf.compareTo`
//!   (object `compareTo`, then shape index): rooms first (descending id), then items
//!   (descending id).

use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::Arc;

use fr_geom::{
    Circle, ConvexShape, FloatPoint, Line, Point, Polyline, RegularTileShape, Shape,
    ShapeBoundingDirections, TileShape,
};

use crate::autoroute::rooms::RoomKey;
use crate::datastructures::{LeafId, MinAreaTree};
use crate::ids::{AngleRestriction, ClearanceClassNo, LayerNo, NetNo};
use crate::rules::BoardRules;
use crate::structure::Unit;

use super::basic_board::BasicBoard;
use super::item::{Item, ItemKey, ItemKind};

/// Java `ShapeSearchTree.DRILL_HOLE_CLEARANCE_MARGIN`.
const DRILL_HOLE_CLEARANCE_MARGIN: i32 = 10;

/// The Java class of a search tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TreeKind {
    /// `ShapeSearchTree` (45 degree bounding directions).
    Default,
    /// `ShapeSearchTree45Degree`.
    FortyfiveDegree,
    /// `ShapeSearchTree90Degree` (orthogonal bounding directions).
    NinetyDegree,
}

/// A leaf object of a search tree (Java `SearchTreeObject`: `Item` or
/// `CompleteFreeSpaceExpansionRoom`). The Java id is stored inline for the ordering.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TreeObject {
    Item { key: ItemKey, id: i32 },
    Room { key: RoomKey, id: i32 },
}

impl TreeObject {
    /// Java `compareTo` of the stored objects.
    #[inline]
    pub fn java_cmp(&self, other: &TreeObject) -> Ordering {
        match (self, other) {
            (TreeObject::Item { id: a, .. }, TreeObject::Item { id: b, .. }) => b.wrapping_sub(*a).cmp(&0),
            (TreeObject::Item { .. }, TreeObject::Room { .. }) => Ordering::Greater,
            (TreeObject::Room { .. }, TreeObject::Item { .. }) => Ordering::Less,
            (TreeObject::Room { id: a, .. }, TreeObject::Room { id: b, .. }) => b.wrapping_sub(*a).cmp(&0),
        }
    }

    /// The item key, if this is an item.
    #[inline]
    pub fn item(&self) -> Option<ItemKey> {
        match self {
            TreeObject::Item { key, .. } => Some(*key),
            TreeObject::Room { .. } => None,
        }
    }

    /// The room key, if this is a room.
    #[inline]
    pub fn room(&self) -> Option<RoomKey> {
        match self {
            TreeObject::Room { key, .. } => Some(*key),
            TreeObject::Item { .. } => None,
        }
    }

    /// Java `getId()`.
    #[inline]
    pub fn id(&self) -> i32 {
        match self {
            TreeObject::Item { id, .. } | TreeObject::Room { id, .. } => *id,
        }
    }
}

/// Java `ShapeTree.TreeEntry`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TreeEntry {
    pub object: TreeObject,
    pub shape_index: i32,
}

/// Java `ItemSearchTreesInfo.SearchTreeInfo` for one item and tree.
#[derive(Clone, Debug)]
pub(crate) struct ItemTreeInfo {
    generation: u32,
    pub(crate) leaves: Option<Vec<Option<LeafId>>>,
    pub(crate) shapes: Option<Arc<[Option<TileShape>]>>,
}

#[derive(Clone, Debug)]
pub(crate) struct RoomTreeInfo {
    pub(crate) shape: TileShape,
    pub(crate) layer: LayerNo,
    pub(crate) id: i32,
    pub(crate) leaf: Option<LeafId>,
}

/// Java `ShapeSearchTree` (+ the 45/90 degree subclasses).
#[derive(Clone, Debug)]
pub struct ShapeSearchTree {
    pub kind: TreeKind,
    /// The clearance class for which the shapes of this tree are compensated (0 = none).
    pub compensated_clearance_class_no: ClearanceClassNo,
    pub(crate) tree: MinAreaTree<TreeObject>,
    item_info: Vec<Option<ItemTreeInfo>>,
    rooms: HashMap<RoomKey, RoomTreeInfo>,
}

impl ShapeSearchTree {
    pub(crate) fn new(kind: TreeKind, compensated_clearance_class_no: ClearanceClassNo) -> Self {
        let dirs = match kind {
            TreeKind::NinetyDegree => ShapeBoundingDirections::Orthogonal,
            _ => ShapeBoundingDirections::FortyfiveDegree,
        };
        ShapeSearchTree {
            kind,
            compensated_clearance_class_no,
            tree: MinAreaTree::new(dirs),
            item_info: Vec::new(),
            rooms: HashMap::new(),
        }
    }

    /// Java `boundingDirections`.
    pub fn bounding_directions(&self) -> ShapeBoundingDirections {
        self.tree.bounding_directions()
    }

    /// The underlying tree (read only).
    pub fn min_area_tree(&self) -> &MinAreaTree<TreeObject> {
        &self.tree
    }

    /// Java `key` / `toString()`.
    pub fn key(&self) -> String {
        let (class, dirs) = match self.kind {
            TreeKind::Default => ("ShapeSearchTree", "FortyfiveDegree"),
            TreeKind::FortyfiveDegree => ("ShapeSearchTree45Degree", "FortyfiveDegree"),
            TreeKind::NinetyDegree => ("ShapeSearchTree90Degree", "Orthogonal"),
        };
        format!("{}_{}_cc{}", class, dirs, self.compensated_clearance_class_no)
    }

    /// Java `isClearanceCompensationUsed()`.
    pub fn is_clearance_compensation_used(&self) -> bool {
        self.compensated_clearance_class_no > 0
    }

    /// Java `clearanceCompensationValue(clearanceClassIndex, layer)`.
    pub fn clearance_compensation_value(&self, rules: &BoardRules, clearance_class: ClearanceClassNo, layer: LayerNo) -> i32 {
        if clearance_class <= 0 {
            return 0;
        }
        let m = &rules.clearance_matrix;
        let result = m
            .get_value(clearance_class, self.compensated_clearance_class_no, layer, false)
            .wrapping_sub(m.clearance_compensation_value(self.compensated_clearance_class_no, layer));
        result.max(0)
    }

    // ------------------------------------------------------------------------------------------
    // item side table

    fn info(&self, key: ItemKey) -> Option<&ItemTreeInfo> {
        match self.item_info.get(key.index as usize) {
            Some(Some(info)) if info.generation == key.generation => Some(info),
            _ => None,
        }
    }

    fn info_mut(&mut self, key: ItemKey) -> &mut ItemTreeInfo {
        let index = key.index as usize;
        if self.item_info.len() <= index {
            self.item_info.resize(index + 1, None);
        }
        let slot = &mut self.item_info[index];
        match slot {
            Some(info) if info.generation == key.generation => {}
            _ => *slot = Some(ItemTreeInfo { generation: key.generation, leaves: None, shapes: None }),
        }
        slot.as_mut().unwrap()
    }

    /// Java `item.getSearchTreeEntries(tree)`.
    pub fn item_leaves(&self, key: ItemKey) -> Option<&[Option<LeafId>]> {
        self.info(key)?.leaves.as_deref()
    }

    /// The precalculated tree shapes of an item, if present.
    pub(crate) fn cached_shapes(&self, key: ItemKey) -> Option<&Arc<[Option<TileShape>]>> {
        self.info(key)?.shapes.as_ref()
    }

    pub(crate) fn set_leaves(&mut self, key: ItemKey, leaves: Vec<Option<LeafId>>) {
        self.info_mut(key).leaves = Some(leaves);
    }

    pub(crate) fn set_shapes(&mut self, key: ItemKey, shapes: Arc<[Option<TileShape>]>) {
        self.info_mut(key).shapes = Some(shapes);
    }

    /// Java `clearSearchTreeEntries` for this tree (drops leaves and shapes).
    pub(crate) fn clear_info(&mut self, key: ItemKey) {
        if let Some(slot) = self.item_info.get_mut(key.index as usize) {
            if matches!(slot, Some(info) if info.generation == key.generation) {
                *slot = None;
            }
        }
    }

    /// Java `ItemSearchTreesInfo.clearPrecalculatedTreeShapes` for this tree.
    pub(crate) fn clear_shapes(&mut self, key: ItemKey) {
        if let Some(Some(info)) = self.item_info.get_mut(key.index as usize) {
            if info.generation == key.generation {
                info.shapes = None;
            }
        }
    }

    /// Java `ShapeTree.insert(Storable)` for an item whose tree shapes are given: inserts every
    /// shape (leaf `None` where the shape or its bounding shape is null) and stores the leaves
    /// and shapes. Items without shapes get no entries (Java returns before
    /// `setSearchTreeEntries`).
    pub(crate) fn insert_item(&mut self, key: ItemKey, id: i32, shapes: Arc<[Option<TileShape>]>) {
        self.set_shapes(key, shapes.clone());
        if shapes.is_empty() {
            return;
        }
        let object = TreeObject::Item { key, id };
        let mut leaves = Vec::with_capacity(shapes.len());
        for (i, s) in shapes.iter().enumerate() {
            leaves.push(match s {
                Some(shape) => self.tree.insert_shape(object, i as i32, shape),
                None => None,
            });
        }
        self.set_leaves(key, leaves);
    }

    /// Java `insert(object, index)` for a single shape index of an item (shape taken from the
    /// precalculated shapes).
    pub(crate) fn insert_item_shape(&mut self, key: ItemKey, id: i32, index: i32) -> Option<LeafId> {
        let shape = self.cached_shapes(key)?.get(index as usize)?.clone()?;
        self.tree.insert_shape(TreeObject::Item { key, id }, index, &shape)
    }

    /// Removes all leaves of an item from this tree (the side table entry is kept; see
    /// [`Self::clear_info`]).
    pub(crate) fn remove_item_leaves(&mut self, key: ItemKey) {
        if let Some(leaves) = self.info(key).and_then(|i| i.leaves.clone()) {
            self.tree.remove(&leaves);
        }
    }

    // ------------------------------------------------------------------------------------------
    // rooms

    /// Inserts a complete free space expansion room (Java `tree.insert(room)`). Returns the leaf.
    pub fn insert_room(&mut self, key: RoomKey, id: i32, shape: TileShape, layer: LayerNo) -> Option<LeafId> {
        let leaf = self.tree.insert_shape(TreeObject::Room { key, id }, 0, &shape);
        self.rooms.insert(key, RoomTreeInfo { shape, layer, id, leaf });
        leaf
    }

    /// Removes a room (Java `room.removeFromTree(tree)`). No-op if the room is not stored.
    pub fn remove_room(&mut self, key: RoomKey) {
        if let Some(info) = self.rooms.remove(&key) {
            if let Some(leaf) = info.leaf {
                self.tree.remove_leaf(leaf);
            }
        }
    }

    /// True if the room is stored in this tree.
    pub fn contains_room(&self, key: RoomKey) -> bool {
        self.rooms.contains_key(&key)
    }

    /// The stored shape of a room.
    pub fn room_shape(&self, key: RoomKey) -> Option<&TileShape> {
        self.rooms.get(&key).map(|r| &r.shape)
    }

    /// Keys of all stored rooms (arbitrary order).
    pub fn room_keys(&self) -> Vec<RoomKey> {
        self.rooms.keys().copied().collect()
    }

    /// Java `Leaf.compareTo` comparator for this tree's objects.
    #[inline]
    pub(crate) fn object_cmp(a: &TreeObject, b: &TreeObject) -> Ordering {
        a.java_cmp(b)
    }

    /// Java `offsetShape(polyline, halfWidth, no)` (overridden by the 90 degree tree).
    pub fn offset_shape(&self, polyline: &Polyline, half_width: i32, no: i32) -> Option<TileShape> {
        match self.kind {
            TreeKind::NinetyDegree => Some(TileShape::IntBox(polyline.offset_box(half_width, no))),
            _ => polyline.offset_shape(half_width, no),
        }
    }

    /// Java `offsetShapes(polyline, halfWidth, fromNo, toNo)` (overridden by the 90 degree tree).
    pub fn offset_shapes(&self, polyline: &Polyline, half_width: i32, from_no: i32, to_no: i32) -> Vec<TileShape> {
        match self.kind {
            TreeKind::NinetyDegree => {
                let from_no = from_no.max(0);
                let to_no = to_no.min(polyline.lines.len() as i32 - 1);
                let mut shapes = Vec::with_capacity((to_no - from_no - 1).max(0) as usize);
                for j in from_no..to_no - 1 {
                    shapes.push(TileShape::IntBox(polyline.offset_box(half_width, j)));
                }
                shapes
            }
            _ => polyline.offset_shapes_range(half_width, from_no, to_no),
        }
    }
}

/// Java `SearchTreeManager`.
#[derive(Clone, Debug)]
pub struct SearchTreeManager {
    /// The trees in Java list order; `trees[0]` is the default tree.
    pub(crate) trees: Vec<ShapeSearchTree>,
    clearance_compensation_used: bool,
}

impl Default for SearchTreeManager {
    fn default() -> Self {
        SearchTreeManager { trees: vec![ShapeSearchTree::new(TreeKind::Default, 0)], clearance_compensation_used: false }
    }
}

/// Index of the default tree in [`SearchTreeManager::trees`].
pub const DEFAULT_TREE: usize = 0;

impl SearchTreeManager {
    /// Java `isClearanceCompensationUsed()`.
    pub fn is_clearance_compensation_used(&self) -> bool {
        self.clearance_compensation_used
    }

    /// The default tree.
    pub fn default_tree(&self) -> &ShapeSearchTree {
        &self.trees[DEFAULT_TREE]
    }

    /// All trees in list order.
    pub fn trees(&self) -> &[ShapeSearchTree] {
        &self.trees
    }

    /// Mutable access to a tree (e.g. for inserting rooms).
    pub fn tree_mut(&mut self, index: usize) -> &mut ShapeSearchTree {
        &mut self.trees[index]
    }

    /// The index of the tree compensated for `clearance_class`, if it exists.
    pub fn tree_index(&self, clearance_class: ClearanceClassNo) -> Option<usize> {
        self.trees.iter().position(|t| t.compensated_clearance_class_no == clearance_class)
    }

    /// Java `resetCompensatedTrees()`: removes all trees except the default tree.
    pub fn reset_compensated_trees(&mut self) {
        self.trees.truncate(1);
    }

    /// Java `clearanceClassRemoved(no)`.
    pub fn clearance_class_removed(&mut self, no: ClearanceClassNo) {
        if no == self.trees[DEFAULT_TREE].compensated_clearance_class_no {
            log::warn!("SearchtreeManager.clearance_class_removed: unable to remove default tree");
            return;
        }
        let mut i = 0;
        self.trees.retain(|t| {
            let keep = i == 0 || t.compensated_clearance_class_no != no;
            i += 1;
            keep
        });
    }
}

// ================================================================================================
// board level functions
// ================================================================================================

impl BasicBoard {
    /// The search tree with index `t` (see [`SearchTreeManager::trees`]).
    #[inline]
    pub fn search_tree(&self, t: usize) -> &ShapeSearchTree {
        &self.search_trees.trees[t]
    }

    /// Java `searchTreeManager.getDefaultTree()`.
    #[inline]
    pub fn default_tree(&self) -> &ShapeSearchTree {
        &self.search_trees.trees[DEFAULT_TREE]
    }

    /// Java `searchTreeManager.isClearanceCompensationUsed()`.
    pub fn is_clearance_compensation_used(&self) -> bool {
        self.search_trees.clearance_compensation_used
    }

    /// Java `ShapeSearchTree.clearanceCompensationValue` of tree `t`.
    pub fn clearance_compensation_value(&self, t: usize, clearance_class: ClearanceClassNo, layer: LayerNo) -> i32 {
        self.search_trees.trees[t].clearance_compensation_value(&self.rules, clearance_class, layer)
    }

    // ------------------------------------------------------------------------------------------
    // tree shapes

    /// Java `Item.calculateTreeShapes(tree)` (dispatching to `ShapeSearchTree.calculateTreeShapes`
    /// of the tree class).
    pub fn calculate_tree_shapes(&self, tree: &ShapeSearchTree, item: &Item) -> Arc<[Option<TileShape>]> {
        let result: Vec<Option<TileShape>> = match &item.kind {
            ItemKind::Pin(_) | ItemKind::Via(_) => self.calculate_drill_tree_shapes(tree, item),
            ItemKind::Trace(t) => {
                let offset_width = t.half_width + tree.clearance_compensation_value(&self.rules, item.clearance_class, t.layer);
                (0..t.tile_shape_count()).map(|i| tree.offset_shape(&t.polyline, offset_width, i)).collect()
            }
            ItemKind::ObstacleArea(a) => self.calculate_obstacle_tree_shapes(tree, item, a),
            ItemKind::ConductionArea(c) => self.calculate_obstacle_tree_shapes(tree, item, &c.area),
            ItemKind::ComponentOutline(_) => Vec::new(),
            ItemKind::BoardOutline(_) => self.calculate_outline_tree_shapes(tree, item),
        };
        Arc::from(result)
    }

    fn calculate_drill_tree_shapes(&self, tree: &ShapeSearchTree, item: &Item) -> Vec<Option<TileShape>> {
        let count = item.drill_tile_shape_count(self);
        let angle_restriction = self.rules.get_trace_angle_restriction();
        let mut result = Vec::with_capacity(count.max(0) as usize);
        for i in 0..count {
            let mut current_shape = item.drill_shape(self, i);
            if current_shape.is_none() {
                current_shape = self.drill_hole_obstacle(item);
            }
            let Some(current_shape) = current_shape else {
                result.push(None);
                continue;
            };
            let layer = item.shape_layer_with_count(self, i, || 0);
            let offset_width = tree
                .clearance_compensation_value(&self.rules, item.clearance_class, layer)
                .wrapping_add(self.drill_hole_clearance_delta(tree, item, &current_shape, layer));
            let shape = match tree.kind {
                TreeKind::Default => {
                    let tile = match angle_restriction {
                        AngleRestriction::NinetyDegree => Some(TileShape::IntBox(current_shape.bounding_box())),
                        AngleRestriction::FortyfiveDegree => current_shape.bounding_octagon().map(TileShape::IntOctagon),
                        AngleRestriction::None => Some(current_shape.bounding_tile()),
                    };
                    match tile {
                        None => {
                            log::warn!("ShapeSearchTree.calculate_tree_shapes: shape is null");
                            None
                        }
                        Some(t) => Some(t.enlarge(offset_width as f64)),
                    }
                }
                TreeKind::FortyfiveDegree => {
                    let mut tile = TileShape::IntOctagon(
                        current_shape.bounding_octagon().expect("ShapeSearchTree45Degree: bounding octagon is null"),
                    );
                    if tile.is_int_box() {
                        // To avoid small corner cutoffs when taking the offset as an octagon.
                        tile = TileShape::IntBox(current_shape.bounding_box());
                    }
                    let tile = tile.offset(offset_width as f64);
                    tile.bounding_octagon().map(TileShape::IntOctagon)
                }
                TreeKind::NinetyDegree => Some(TileShape::IntBox(current_shape.bounding_box().offset(offset_width as f64))),
            };
            result.push(shape);
        }
        result
    }

    fn calculate_obstacle_tree_shapes(&self, tree: &ShapeSearchTree, item: &Item, area: &super::item::ObstacleArea) -> Vec<Option<TileShape>> {
        let Some(convex_shapes) = area.split_to_convex(self) else {
            return Vec::new();
        };
        let mut max_tree_shape_width = 50000.0;
        if self.communication.host_cad_exists() {
            max_tree_shape_width = (500.0 * self.communication.get_resolution(Unit::Mil)).min(max_tree_shape_width);
        }
        let mut result: Vec<Option<TileShape>> = Vec::new();
        for convex in &convex_shapes {
            let offset_width = tree.clearance_compensation_value(&self.rules, item.clearance_class, area.layer);
            let enlarged = convex.enlarge(offset_width as f64);
            for s in enlarged.divide_into_sections(max_tree_shape_width) {
                result.push(Some(s));
            }
        }
        match tree.kind {
            TreeKind::Default => {}
            TreeKind::FortyfiveDegree => {
                for s in result.iter_mut() {
                    if let Some(shape) = s {
                        *s = shape.bounding_octagon().map(TileShape::IntOctagon);
                    }
                }
            }
            TreeKind::NinetyDegree => {
                for s in result.iter_mut() {
                    if let Some(shape) = s {
                        *s = Some(TileShape::IntBox(shape.bounding_box()));
                    }
                }
            }
        }
        result
    }

    fn calculate_outline_tree_shapes(&self, tree: &ShapeSearchTree, item: &Item) -> Vec<Option<TileShape>> {
        let outline = item.as_board_outline().unwrap();
        let layer_count = self.layer_count();
        let mut result: Vec<Option<TileShape>> = Vec::new();
        if outline.keepout_outside_outline {
            let Some(convex_shapes) = outline.keepout_area(&self.bounding_box).split_to_convex() else {
                return Vec::new();
            };
            for layer in 0..layer_count {
                for convex in &convex_shapes {
                    let offset_width = tree.clearance_compensation_value(&self.rules, item.clearance_class, layer);
                    result.push(Some(convex.enlarge(offset_width as f64)));
                }
            }
        } else {
            // Only the line shapes of the outline are inserted as obstacles into the tree.
            let half_width = BoardOutlineConsts::HALF_WIDTH;
            for layer in 0..layer_count {
                for shape in outline.shapes.iter() {
                    let border_line_count = shape.border_line_count();
                    let mut prev: Line = shape.border_line(border_line_count - 1);
                    for i in 0..border_line_count {
                        let cur = shape.border_line(i);
                        let next = shape.border_line((i + 1) % border_line_count);
                        let tmp_polyline = Polyline::from_lines(vec![prev.clone(), cur.clone(), next]);
                        let cmp_value = tree.clearance_compensation_value(&self.rules, item.clearance_class, layer);
                        result.push(tmp_polyline.offset_shape(half_width + cmp_value, 0));
                        prev = cur;
                    }
                }
            }
        }
        match tree.kind {
            TreeKind::Default => {}
            TreeKind::FortyfiveDegree => {
                for s in result.iter_mut() {
                    if let Some(shape) = s {
                        *s = shape.bounding_octagon().map(TileShape::IntOctagon);
                    }
                }
            }
            TreeKind::NinetyDegree => {
                for s in result.iter_mut() {
                    if let Some(shape) = s {
                        *s = Some(TileShape::IntBox(shape.bounding_box()));
                    }
                }
            }
        }
        result
    }

    /// Java `ShapeSearchTree.drillHoleObstacle`.
    fn drill_hole_obstacle(&self, item: &Item) -> Option<Shape> {
        if self.rules.get_hole_clearance() <= 0 {
            return None;
        }
        let padstack = item.padstack(self)?;
        let drill_radius = padstack.get_drill_radius();
        if drill_radius <= 0.0 {
            return None;
        }
        let center = match item.center(self) {
            Point::Int(p) => p,
            other => other.to_float().round(),
        };
        Some(Shape::Circle(Circle::new(center, drill_radius.ceil() as i32)))
    }

    /// Java `ShapeSearchTree.drillHoleClearanceDelta`.
    fn drill_hole_clearance_delta(&self, tree: &ShapeSearchTree, item: &Item, shape: &Shape, layer: LayerNo) -> i32 {
        let hole_clearance = self.rules.get_hole_clearance();
        if hole_clearance <= 0 {
            return 0;
        }
        let Some(padstack) = item.padstack(self) else {
            return 0;
        };
        let drill_radius = padstack.get_drill_radius();
        if drill_radius <= 0.0 {
            return 0;
        }
        let copper_radius = if padstack.hole_only {
            drill_radius
        } else {
            let mut r = shape.border_distance(&item.center(self).to_float());
            if r <= 0.0 {
                r = match padstack.get_shape(layer) {
                    None => drill_radius,
                    Some(pad) => pad.to_shape().border_distance(&FloatPoint::ZERO),
                };
            }
            r
        };
        let clearance_class = if tree.compensated_clearance_class_no > 0 {
            tree.compensated_clearance_class_no
        } else {
            BoardRules::DEFAULT_CLEARANCE_CLASS
        };
        let copper_clearance = self.rules.clearance_matrix.get_value(item.clearance_class, clearance_class, layer, false);
        let v = (drill_radius + hole_clearance as f64 + DRILL_HOLE_CLEARANCE_MARGIN as f64 - copper_radius - copper_clearance as f64).ceil() as i32;
        v.max(0)
    }

    /// The tree shapes of an item in tree `t`: the precalculated shapes if present, else newly
    /// calculated ones (Java caches them in `ItemSearchTreesInfo`; the values are the same).
    pub fn item_tree_shapes(&self, t: usize, key: ItemKey) -> Arc<[Option<TileShape>]> {
        let tree = &self.search_trees.trees[t];
        if let Some(shapes) = tree.cached_shapes(key) {
            return shapes.clone();
        }
        self.calculate_tree_shapes(tree, self.items.get(key))
    }

    /// Java `Item.getTreeShape(tree, index)`.
    pub fn tree_shape(&self, t: usize, key: ItemKey, index: i32) -> Option<TileShape> {
        let shapes = self.item_tree_shapes(t, key);
        if index < 0 || index as usize >= shapes.len() {
            // Java clears the derived data and recalculates.
            let shapes = self.calculate_tree_shapes(&self.search_trees.trees[t], self.items.get(key));
            return shapes.get(index as usize).cloned().flatten();
        }
        shapes[index as usize].clone()
    }

    /// Java `Item.treeShapeCount(tree)`.
    pub fn tree_shape_count(&self, t: usize, key: ItemKey) -> i32 {
        self.item_tree_shapes(t, key).len() as i32
    }

    /// Java `Item.getTileShape(index)`: the shape in the default tree.
    pub fn tile_shape(&self, key: ItemKey, index: i32) -> Option<TileShape> {
        self.tree_shape(DEFAULT_TREE, key, index)
    }

    /// Java `Item.tileShapeCount()`.
    pub fn tile_shape_count(&self, key: ItemKey) -> i32 {
        self.item_tile_shape_count(self.items.get(key), Some(key))
    }

    /// Java `tileShapeCount()` of an item (also for items not in the arena, `key == None`).
    pub fn item_tile_shape_count(&self, item: &Item, key: Option<ItemKey>) -> i32 {
        match &item.kind {
            ItemKind::Pin(_) | ItemKind::Via(_) => item.drill_tile_shape_count(self),
            ItemKind::Trace(t) => t.tile_shape_count(),
            ItemKind::ObstacleArea(_) | ItemKind::ConductionArea(_) => match key {
                Some(k) => self.tree_shape_count(DEFAULT_TREE, k),
                None => self.calculate_tree_shapes(self.default_tree(), item).len() as i32,
            },
            ItemKind::ComponentOutline(_) => 0,
            ItemKind::BoardOutline(o) => {
                // Same count as the default tree shapes (convex pieces or lines times layers);
                // use the cached shapes to avoid splitting the keepout area on every call.
                if let Some(shapes) = key.and_then(|k| self.default_tree().cached_shapes(k)) {
                    return shapes.len() as i32;
                }
                if o.keepout_outside_outline {
                    match o.keepout_area(&self.bounding_box).split_to_convex() {
                        None => 0,
                        Some(v) => v.len() as i32 * self.layer_count(),
                    }
                } else {
                    o.line_count() * self.layer_count()
                }
            }
        }
    }

    /// The tile shapes (default tree shapes) of an item that is not necessarily in the arena.
    pub fn item_tile_shapes(&self, item: &Item) -> Arc<[Option<TileShape>]> {
        self.calculate_tree_shapes(self.default_tree(), item)
    }

    /// Java `shapeLayer(index)` of an item.
    pub fn shape_layer(&self, key: ItemKey, index: i32) -> LayerNo {
        let item = self.items.get(key);
        item.shape_layer_with_count(self, index, || self.item_tile_shape_count(item, Some(key)))
    }

    /// Java `shapeLayer(index)` of an item not necessarily in the arena.
    pub fn item_shape_layer(&self, item: &Item, index: i32) -> LayerNo {
        item.shape_layer_with_count(self, index, || self.item_tile_shape_count(item, None))
    }

    /// Java `SearchTreeObject.shapeLayer` of a tree object.
    pub fn object_shape_layer(&self, t: usize, object: TreeObject, index: i32) -> LayerNo {
        match object {
            TreeObject::Item { key, .. } => self.shape_layer(key, index),
            TreeObject::Room { key, .. } => self.search_trees.trees[t].rooms[&key].layer,
        }
    }

    /// Java `SearchTreeObject.isObstacle(netNumber)`.
    pub fn object_is_obstacle(&self, object: TreeObject, net_number: NetNo) -> bool {
        match object {
            TreeObject::Item { key, .. } => self.items.get(key).is_obstacle_for_net(net_number),
            TreeObject::Room { .. } => true,
        }
    }

    /// Java `SearchTreeObject.isTraceObstacle(netNumber)`.
    pub fn object_is_trace_obstacle(&self, object: TreeObject, net_number: NetNo) -> bool {
        match object {
            TreeObject::Item { key, .. } => self.is_trace_obstacle(self.items.get(key), net_number),
            TreeObject::Room { .. } => true,
        }
    }

    /// Java `Item.isTraceObstacle(netNumber)` with overrides.
    pub fn is_trace_obstacle(&self, item: &Item, net_number: NetNo) -> bool {
        match &item.kind {
            ItemKind::ObstacleArea(a) => match a.kind {
                super::item::ObstacleKind::Keepout => !item.contains_net(net_number),
                _ => false,
            },
            ItemKind::ConductionArea(c) => c.is_obstacle && !item.contains_net(net_number),
            ItemKind::BoardOutline(_) if self.java_variant == super::basic_board::JavaVariant::Source => {
                !(net_number > 0 && self.edge_pin_nets().contains(&net_number))
            }
            _ => !item.contains_net(net_number),
        }
    }

    /// Java `getTreeShape(tree, index)` of a tree object.
    pub fn object_tree_shape(&self, t: usize, object: TreeObject, index: i32) -> Option<TileShape> {
        match object {
            TreeObject::Item { key, .. } => self.tree_shape(t, key, index),
            TreeObject::Room { key, .. } => Some(self.search_trees.trees[t].rooms[&key].shape.clone()),
        }
    }

    // ------------------------------------------------------------------------------------------
    // insert / remove (SearchTreeManager)

    /// Java `SearchTreeManager.insert(item)`: inserts the item into all trees and marks it as on
    /// the board.
    pub(crate) fn tree_insert(&mut self, key: ItemKey) {
        let id = self.items.get(key).id().0;
        for t in 0..self.search_trees.trees.len() {
            let shapes = self.item_tree_shapes(t, key);
            self.search_trees.trees[t].insert_item(key, id, shapes);
        }
        self.items.get_mut(key).on_board = true;
    }

    /// Java `SearchTreeManager.remove(item)`.
    pub(crate) fn tree_remove(&mut self, key: ItemKey) {
        if !self.items.get(key).is_on_board() {
            return;
        }
        for tree in self.search_trees.trees.iter_mut() {
            tree.remove_item_leaves(key);
        }
        self.clear_search_tree_entries(key);
        self.items.get_mut(key).on_board = false;
    }

    /// Java `Item.clearSearchTreeEntries()`: drops the tree info of all trees.
    pub(crate) fn clear_search_tree_entries(&mut self, key: ItemKey) {
        for tree in self.search_trees.trees.iter_mut() {
            tree.clear_info(key);
        }
    }

    /// Java `Item.clearDerivedData()`: the item caches and the precalculated tree shapes.
    pub fn clear_derived_data(&mut self, key: ItemKey) {
        for tree in self.search_trees.trees.iter_mut() {
            tree.clear_shapes(key);
        }
        self.items.get_mut(key).clear_derived_data();
    }

    /// Java `SearchTreeManager.setClearanceCompensationUsed(value)`.
    pub fn set_clearance_compensation_used(&mut self, value: bool) {
        if self.search_trees.clearance_compensation_used == value {
            return;
        }
        self.search_trees.clearance_compensation_used = value;
        self.remove_all_board_items_from_trees();
        let cl = if value { 1 } else { 0 };
        self.search_trees.trees.clear();
        self.search_trees.trees.push(ShapeSearchTree::new(TreeKind::Default, cl));
        self.insert_all_board_items_into_trees();
    }

    /// Java `SearchTreeManager.clearanceValueChanged()`.
    pub fn clearance_value_changed(&mut self) {
        let default_cl = self.search_trees.trees[DEFAULT_TREE].compensated_clearance_class_no;
        self.search_trees.trees.retain(|t| t.compensated_clearance_class_no == default_cl);
        if self.search_trees.clearance_compensation_used {
            self.remove_all_board_items_from_trees();
            self.insert_all_board_items_into_trees();
        }
    }

    /// Java `SearchTreeManager.reinsertTreeItems()`.
    pub fn reinsert_tree_items(&mut self) {
        self.remove_all_board_items_from_trees();
        let mut cursor = self.items.cursor();
        while let Some(key) = self.items.cursor_next(&mut cursor) {
            self.clear_derived_data(key);
        }
        self.insert_all_board_items_into_trees();
    }

    fn remove_all_board_items_from_trees(&mut self) {
        let mut cursor = self.items.cursor();
        while let Some(key) = self.items.cursor_next(&mut cursor) {
            self.tree_remove(key);
        }
    }

    fn insert_all_board_items_into_trees(&mut self) {
        let mut cursor = self.items.cursor();
        while let Some(key) = self.items.cursor_next(&mut cursor) {
            self.clear_derived_data(key);
            self.tree_insert(key);
        }
    }

    /// Java `SearchTreeManager.getAutorouteTree(clearanceClassIndex)`: the tree compensated for
    /// the class, created (and filled in item list order) if necessary. Returns its index.
    pub fn get_autoroute_tree(&mut self, clearance_class: ClearanceClassNo) -> usize {
        if let Some(t) = self.search_trees.tree_index(clearance_class) {
            return t;
        }
        let kind = match self.rules.get_trace_angle_restriction() {
            AngleRestriction::NinetyDegree => TreeKind::NinetyDegree,
            AngleRestriction::FortyfiveDegree => TreeKind::FortyfiveDegree,
            AngleRestriction::None => TreeKind::Default,
        };
        self.search_trees.trees.push(ShapeSearchTree::new(kind, clearance_class));
        let t = self.search_trees.trees.len() - 1;
        let mut cursor = self.items.cursor();
        while let Some(key) = self.items.cursor_next(&mut cursor) {
            let id = self.items.get(key).id().0;
            let shapes = self.item_tree_shapes(t, key);
            self.search_trees.trees[t].insert_item(key, id, shapes);
        }
        t
    }

    /// Java `SearchTreeManager.resetCompensatedTrees()`.
    pub fn reset_compensated_trees(&mut self) {
        self.search_trees.reset_compensated_trees();
    }

    /// Java `SearchTreeManager.validateEntries(item)`.
    pub fn validate_tree_entries(&self, key: ItemKey) -> bool {
        let mut result = true;
        for tree in &self.search_trees.trees {
            match tree.item_leaves(key) {
                None => result = false,
                Some(leaves) => {
                    for (i, leaf) in leaves.iter().enumerate() {
                        match leaf.and_then(|l| tree.tree.leaf(l)) {
                            Some(l) if l.shape_index_in_object == i as i32 => {}
                            _ => {
                                log::warn!("tree entry inconsistent for Item");
                                result = false;
                                break;
                            }
                        }
                    }
                }
            }
        }
        result
    }

    // ------------------------------------------------------------------------------------------
    // queries

    /// The leaves whose bounding shape intersects `bounds`, sorted like Java
    /// `MinAreaTree.overlaps`.
    pub fn tree_overlaps(&self, t: usize, bounds: &RegularTileShape) -> Vec<LeafId> {
        self.search_trees.trees[t].tree.overlaps_by(bounds, ShapeSearchTree::object_cmp)
    }

    /// Java `ShapeSearchTree.overlappingTreeEntries(shape, layer, ignoreNetNos, treeEntries)`.
    pub fn overlapping_tree_entries(&self, t: usize, shape: &ConvexShape, layer: LayerNo, ignore_net_nos: &[NetNo], out: &mut Vec<TreeEntry>) {
        let tree = &self.search_trees.trees[t];
        let Some(bounds) = shape.bounding_shape(&tree.bounding_directions()) else {
            log::warn!("ShapeSearchTree.overlaps: shape not bounded");
            return;
        };
        let leaves = self.tree_overlaps(t, &bounds);
        let is_45_degree = matches!(shape, ConvexShape::Tile(TileShape::IntOctagon(_)));
        let query_shape = shape.to_shape();
        for leaf in leaves {
            let l = tree.tree.leaf(leaf).unwrap();
            let object = l.object;
            let shape_index = l.shape_index_in_object;
            let mut ignore_object = layer >= 0 && self.object_shape_layer(t, object, shape_index) != layer;
            if !ignore_object {
                for &n in ignore_net_nos {
                    if !self.object_is_obstacle(object, n) {
                        ignore_object = true;
                    }
                }
            }
            if !ignore_object {
                let current_shape = self
                    .object_tree_shape(t, object, shape_index)
                    .expect("ShapeSearchTree.overlappingTreeEntries: tree shape is null");
                let add_item = if is_45_degree && matches!(current_shape, TileShape::IntOctagon(_)) {
                    true
                } else {
                    current_shape.intersects(&query_shape)
                };
                if add_item {
                    out.push(TreeEntry { object, shape_index });
                }
            }
        }
    }

    /// Java `overlappingTreeEntries(shape, layer, treeEntries)` on tree `t`, returning a new list.
    pub fn overlapping_tree_entries_list(&self, t: usize, shape: &ConvexShape, layer: LayerNo, ignore_net_nos: &[NetNo]) -> Vec<TreeEntry> {
        let mut out = Vec::new();
        self.overlapping_tree_entries(t, shape, layer, ignore_net_nos, &mut out);
        out
    }

    /// Java `ShapeSearchTree.overlappingTreeEntriesWithClearance(shape, layer, ignoreNetNos,
    /// clearanceClassIndex, obstacleEntries)` (the variant not using clearance compensation).
    pub fn overlapping_tree_entries_with_clearance_raw(
        &self,
        t: usize,
        shape: &ConvexShape,
        layer: LayerNo,
        ignore_net_nos: &[NetNo],
        clearance_class: ClearanceClassNo,
        out: &mut Vec<TreeEntry>,
    ) {
        let tree = &self.search_trees.trees[t];
        let cl_matrix = &self.rules.clearance_matrix;
        let bounds = match shape.bounding_shape(&tree.bounding_directions()) {
            Some(b) => b,
            None => {
                log::warn!("ShapeSearchTree.overlaps_with_clearance: shape is not bounded");
                tree.bounding_directions().bounds_int_box(&self.bounding_box)
            }
        };
        let max_clearance = (1.2 * cl_matrix.max_value(clearance_class, layer) as f64) as i32;
        // search with the bounds enlarged by the maximum clearance to get all candidates
        let offset_bounds = match bounds {
            RegularTileShape::IntBox(b) => RegularTileShape::IntBox(b.offset(max_clearance as f64)),
            RegularTileShape::IntOctagon(o) => RegularTileShape::IntOctagon(o.offset(max_clearance as f64)),
        };
        let leaves = self.tree_overlaps(t, &offset_bounds);
        // sort the found items by their clearances to clearance_class on layer
        let mut sorted: Vec<(i32, LeafId, TreeObject, i32)> = Vec::new();
        for leaf in leaves {
            let l = tree.tree.leaf(leaf).unwrap();
            let object = l.object;
            let key = object.item().expect("ShapeSearchTree.overlaps_with_clearance: object is not an Item");
            let shape_index = l.shape_index_in_object;
            let mut ignore_item = layer >= 0 && self.shape_layer(key, shape_index) != layer;
            if !ignore_item {
                for &n in ignore_net_nos {
                    if !self.items.get(key).is_obstacle_for_net(n) {
                        ignore_item = true;
                    }
                }
            }
            if !ignore_item {
                let current_clearance = cl_matrix.get_value(clearance_class, self.items.get(key).clearance_class, layer, true);
                sorted.push((current_clearance, leaf, object, shape_index));
            }
        }
        // TreeSet<EntrySortedByClearance>: clearance, then insertion order (stable sort)
        sorted.sort_by(|a, b| a.0.wrapping_sub(b.0).cmp(&0));
        let mut current_half_clearance = 0;
        let query_shape = shape.to_shape();
        let mut current_offset_shape = query_shape.clone();
        for (clearance, _leaf, object, shape_index) in sorted {
            let tmp_half_clearance = clearance / 2;
            if tmp_half_clearance != current_half_clearance {
                current_half_clearance = tmp_half_clearance;
                current_offset_shape = query_shape.enlarge(current_half_clearance as f64).expect("enlarge");
            }
            let tmp_shape = self.object_tree_shape(t, object, shape_index).expect("tree shape is null");
            // enlarge both item shapes by the half clearance to create symmetry.
            let tmp_offset_shape = tmp_shape.enlarge(current_half_clearance as f64);
            if current_offset_shape.intersects(&Shape::Tile(tmp_offset_shape)) {
                out.push(TreeEntry { object, shape_index });
            }
        }
    }

    /// Java `ShapeSearchTree.overlappingTreeEntriesWithClearance(shape, layer, ignoreNetNos,
    /// clearanceClassIndex)`: uses the compensated shapes if the tree is compensated.
    pub fn overlapping_tree_entries_with_clearance(
        &self,
        t: usize,
        shape: &ConvexShape,
        layer: LayerNo,
        ignore_net_nos: &[NetNo],
        clearance_class: ClearanceClassNo,
    ) -> Vec<TreeEntry> {
        let mut result = Vec::new();
        if self.search_trees.trees[t].is_clearance_compensation_used() {
            self.overlapping_tree_entries(t, shape, layer, ignore_net_nos, &mut result);
        } else {
            self.overlapping_tree_entries_with_clearance_raw(t, shape, layer, ignore_net_nos, clearance_class, &mut result);
        }
        result
    }

    /// Java `ShapeSearchTree.overlappingObjects(shape, layer)`: the objects (Java `TreeSet`, so
    /// sorted by `compareTo` without duplicates).
    pub fn overlapping_objects_in(&self, t: usize, shape: &ConvexShape, layer: LayerNo, ignore_net_nos: &[NetNo]) -> Vec<TreeObject> {
        let mut entries = Vec::new();
        self.overlapping_tree_entries(t, shape, layer, ignore_net_nos, &mut entries);
        objects_of(entries)
    }

    /// Java `ShapeSearchTree.overlappingObjectsWithClearance(shape, layer, ignoreNetNos,
    /// clearanceClassIndex, Set obstacles)` (Java `TreeSet`). The source version always uses the
    /// raw (uncompensated) query; the 2.4.1 jar ([`JavaVariant::Jar241`]) uses the compensated
    /// shapes if the tree is compensated.
    ///
    /// [`JavaVariant::Jar241`]: super::basic_board::JavaVariant::Jar241
    pub fn overlapping_objects_with_clearance_in(
        &self,
        t: usize,
        shape: &ConvexShape,
        layer: LayerNo,
        ignore_net_nos: &[NetNo],
        clearance_class: ClearanceClassNo,
    ) -> Vec<TreeObject> {
        let mut entries = Vec::new();
        self.overlapping_entries_with_clearance_variant(t, shape, layer, ignore_net_nos, clearance_class, &mut entries);
        objects_of(entries)
    }

    fn overlapping_entries_with_clearance_variant(
        &self,
        t: usize,
        shape: &ConvexShape,
        layer: LayerNo,
        ignore_net_nos: &[NetNo],
        clearance_class: ClearanceClassNo,
        entries: &mut Vec<TreeEntry>,
    ) {
        if self.java_variant == super::basic_board::JavaVariant::Jar241 && self.search_trees.trees[t].is_clearance_compensation_used() {
            self.overlapping_tree_entries(t, shape, layer, ignore_net_nos, entries);
        } else {
            self.overlapping_tree_entries_with_clearance_raw(t, shape, layer, ignore_net_nos, clearance_class, entries);
        }
    }

    /// Java `ShapeSearchTree.overlappingItemsWithClearance(shape, layer, ignoreNetNos,
    /// clearanceClassIndex)` (raw variant in the source; see
    /// [`Self::overlapping_objects_with_clearance_in`] for the jar variant).
    pub fn overlapping_items_with_clearance_in(
        &self,
        t: usize,
        shape: &ConvexShape,
        layer: LayerNo,
        ignore_net_nos: &[NetNo],
        clearance_class: ClearanceClassNo,
    ) -> super::item_list::ItemSet {
        let mut entries = Vec::new();
        self.overlapping_entries_with_clearance_variant(t, shape, layer, ignore_net_nos, clearance_class, &mut entries);
        let mut result = super::item_list::ItemSet::new();
        for e in entries {
            if let TreeObject::Item { key, id } = e.object {
                result.insert(crate::ids::ItemId(id), key);
            }
        }
        result
    }

    // ------------------------------------------------------------------------------------------
    // entry changes for traces (performance paths that reuse leaves)

    /// Java `ShapeSearchTree.changeEntries` on all trees (`SearchTreeManager.changeEntries`).
    pub(crate) fn change_entries(&mut self, key: ItemKey, new_polyline: &Polyline, keep_at_start_count: i32, keep_at_end_count: i32) {
        for t in 0..self.search_trees.trees.len() {
            self.change_entries_in(t, key, new_polyline, keep_at_start_count, keep_at_end_count);
        }
    }

    fn change_entries_in(&mut self, t: usize, key: ItemKey, new_polyline: &Polyline, keep_at_start_count: i32, keep_at_end_count: i32) {
        let (half_width, layer, cl, id) = {
            let item = self.items.get(key);
            let tr = item.trace();
            (tr.half_width, tr.layer, item.clearance_class, item.id().0)
        };
        let old_shapes = self.item_tree_shapes(t, key);
        let tree = &self.search_trees.trees[t];
        let compensated_half_width = half_width + tree.clearance_compensation_value(&self.rules, cl, layer);
        let changed_shapes = tree.offset_shapes(
            new_polyline,
            compensated_half_width,
            keep_at_start_count,
            new_polyline.lines.len() as i32 - 1 - keep_at_end_count,
        );
        let old_shape_count = old_shapes.len() as i32;
        let new_shape_count = changed_shapes.len() as i32 + keep_at_start_count + keep_at_end_count;
        let old_entries: Vec<Option<LeafId>> = tree.item_leaves(key).expect("changeEntries: no tree entries").to_vec();
        let mut new_leaves: Vec<Option<LeafId>> = vec![None; new_shape_count.max(0) as usize];
        let mut new_shapes: Vec<Option<TileShape>> = vec![None; new_shape_count.max(0) as usize];
        for i in 0..keep_at_start_count {
            new_leaves[i as usize] = old_entries[i as usize];
            new_shapes[i as usize] = old_shapes[i as usize].clone();
        }
        let tree = &mut self.search_trees.trees[t];
        for i in keep_at_start_count..old_shape_count - keep_at_end_count {
            if let Some(leaf) = old_entries[i as usize] {
                tree.tree.remove_leaf(leaf);
            }
        }
        for i in 0..keep_at_end_count {
            let new_index = new_shape_count - keep_at_end_count + i;
            let old_index = old_shape_count - keep_at_end_count + i;
            let leaf = old_entries[old_index as usize];
            new_leaves[new_index as usize] = leaf;
            if let Some(leaf) = leaf {
                tree.tree.set_leaf_shape_index(leaf, new_index);
            }
            new_shapes[new_index as usize] = old_shapes[old_index as usize].clone();
        }
        for (i, s) in changed_shapes.into_iter().enumerate() {
            new_shapes[keep_at_start_count as usize + i] = Some(s);
        }
        tree.set_shapes(key, Arc::from(new_shapes));
        for i in keep_at_start_count..new_shape_count - keep_at_end_count {
            new_leaves[i as usize] = tree.insert_item_shape(key, id, i);
        }
        tree.set_leaves(key, new_leaves);
    }

    /// Java `SearchTreeManager.mergeEntriesInFront` (all trees).
    pub(crate) fn merge_entries_in_front(&mut self, from: ItemKey, to: ItemKey, joined: &Polyline, from_entry_no: i32, to_entry_no: i32) {
        for t in 0..self.search_trees.trees.len() {
            self.merge_entries_in_front_in(t, from, to, joined, from_entry_no, to_entry_no);
        }
    }

    fn merge_entries_in_front_in(&mut self, t: usize, from: ItemKey, to: ItemKey, joined: &Polyline, from_entry_no: i32, to_entry_no: i32) {
        let (from_first, from_tile_count) = {
            let f = self.items.get(from);
            (f.first_corner(), f.trace().tile_shape_count())
        };
        let (to_first, to_half_width, to_layer, to_cl, to_id) = {
            let item = self.items.get(to);
            (item.first_corner(), item.trace().half_width, item.trace().layer, item.clearance_class, item.id().0)
        };
        let change_order = from_first == to_first;
        let from_shape_count_minus_1 = from_tile_count - 1;
        let remove_no = if change_order { 0 } else { from_shape_count_minus_1 };
        let from_shapes = self.item_tree_shapes(t, from);
        let to_shapes = self.item_tree_shapes(t, to);
        let tree = &mut self.search_trees.trees[t];
        let from_entries: Vec<Option<LeafId>> = tree.item_leaves(from).expect("mergeEntriesInFront: no entries").to_vec();
        let to_entries: Vec<Option<LeafId>> = tree.item_leaves(to).expect("mergeEntriesInFront: no entries").to_vec();
        if let Some(l) = from_entries[remove_no as usize] {
            tree.tree.remove_leaf(l);
        }
        if let Some(l) = to_entries[0] {
            tree.tree.remove_leaf(l);
        }
        let link_shapes = tree.offset_shapes(
            joined,
            to_half_width + tree.clearance_compensation_value(&self.rules, to_cl, to_layer),
            from_entry_no,
            to_entry_no,
        );
        let new_shape_count = from_entries.len() + link_shapes.len() + to_entries.len() - 2;
        let mut new_leaves: Vec<Option<LeafId>> = vec![None; new_shape_count];
        let old_to_shape_count = to_entries.len();
        let mut new_shapes: Vec<Option<TileShape>> = vec![None; new_shape_count];
        let to_object = TreeObject::Item { key: to, id: to_id };
        // transfer the tree entries except the last or first from `from` to `to`
        for i in 0..from_shape_count_minus_1.max(0) as usize {
            let from_no = if change_order { from_shape_count_minus_1 as usize - i } else { i };
            new_shapes[i] = from_shapes[from_no].clone();
            new_leaves[i] = from_entries[from_no];
            if let Some(l) = new_leaves[i] {
                tree.tree.set_leaf_object(l, to_object);
                tree.tree.set_leaf_shape_index(l, i as i32);
            }
        }
        for i in 1..old_to_shape_count {
            let current = from_shape_count_minus_1 as usize + link_shapes.len() + i - 1;
            new_shapes[current] = to_shapes[i].clone();
            new_leaves[current] = to_entries[i];
            if let Some(l) = new_leaves[current] {
                tree.tree.set_leaf_shape_index(l, current as i32);
            }
        }
        let link_count = link_shapes.len();
        for (i, s) in link_shapes.into_iter().enumerate() {
            new_shapes[from_shape_count_minus_1 as usize + i] = Some(s);
        }
        tree.set_shapes(to, Arc::from(new_shapes));
        for i in 0..link_count {
            let current = from_shape_count_minus_1 as usize + i;
            new_leaves[current] = tree.insert_item_shape(to, to_id, current as i32);
        }
        tree.set_leaves(to, new_leaves);
    }

    /// Java `SearchTreeManager.mergeEntriesAtEnd` (all trees).
    pub(crate) fn merge_entries_at_end(&mut self, from: ItemKey, to: ItemKey, joined: &Polyline, from_entry_no: i32, to_entry_no: i32) {
        for t in 0..self.search_trees.trees.len() {
            self.merge_entries_at_end_in(t, from, to, joined, from_entry_no, to_entry_no);
        }
    }

    fn merge_entries_at_end_in(&mut self, t: usize, from: ItemKey, to: ItemKey, joined: &Polyline, from_entry_no: i32, to_entry_no: i32) {
        let (from_last, from_tile_count) = {
            let f = self.items.get(from);
            (f.last_corner(), f.trace().tile_shape_count())
        };
        let (to_last, to_tile_count, to_half_width, to_layer, to_cl, to_id) = {
            let item = self.items.get(to);
            (item.last_corner(), item.trace().tile_shape_count(), item.trace().half_width, item.trace().layer, item.clearance_class, item.id().0)
        };
        let change_order = from_last == to_last;
        let from_shapes = self.item_tree_shapes(t, from);
        let to_shapes = self.item_tree_shapes(t, to);
        let tree = &mut self.search_trees.trees[t];
        let from_entries: Vec<Option<LeafId>> = tree.item_leaves(from).expect("mergeEntriesAtEnd: no entries").to_vec();
        let to_entries: Vec<Option<LeafId>> = tree.item_leaves(to).expect("mergeEntriesAtEnd: no entries").to_vec();
        let to_shape_count_minus_1 = to_tile_count - 1;
        if let Some(l) = to_entries[to_shape_count_minus_1 as usize] {
            tree.tree.remove_leaf(l);
        }
        let remove_no = if change_order { from_tile_count - 1 } else { 0 };
        if let Some(l) = from_entries[remove_no as usize] {
            tree.tree.remove_leaf(l);
        }
        let link_shapes = tree.offset_shapes(
            joined,
            to_half_width + tree.clearance_compensation_value(&self.rules, to_cl, to_layer),
            from_entry_no,
            to_entry_no,
        );
        let new_shape_count = from_entries.len() + link_shapes.len() + to_entries.len() - 2;
        let mut new_leaves: Vec<Option<LeafId>> = vec![None; new_shape_count];
        let mut new_shapes: Vec<Option<TileShape>> = vec![None; new_shape_count];
        let to_object = TreeObject::Item { key: to, id: to_id };
        for i in 0..to_shape_count_minus_1.max(0) as usize {
            new_shapes[i] = to_shapes[i].clone();
            new_leaves[i] = to_entries[i];
        }
        for i in 1..from_entries.len() {
            let current = to_shape_count_minus_1 as usize + link_shapes.len() + i - 1;
            let from_no = if change_order { from_entries.len() - i - 1 } else { i };
            new_shapes[current] = from_shapes[from_no].clone();
            new_leaves[current] = from_entries[from_no];
            if let Some(l) = new_leaves[current] {
                tree.tree.set_leaf_object(l, to_object);
                tree.tree.set_leaf_shape_index(l, current as i32);
            }
        }
        let link_count = link_shapes.len();
        for (i, s) in link_shapes.into_iter().enumerate() {
            new_shapes[to_shape_count_minus_1 as usize + i] = Some(s);
        }
        tree.set_shapes(to, Arc::from(new_shapes));
        for i in 0..link_count {
            let current = to_shape_count_minus_1 as usize + i;
            new_leaves[current] = tree.insert_item_shape(to, to_id, current as i32);
        }
        tree.set_leaves(to, new_leaves);
    }

    /// Java `SearchTreeManager.reuseEntriesAfterCutout` (all trees): transfers the leaves of
    /// `from` to the start and end pieces after a middle piece was cut out.
    pub(crate) fn reuse_entries_after_cutout(&mut self, from: ItemKey, start_piece: ItemKey, end_piece: ItemKey) {
        for t in 0..self.search_trees.trees.len() {
            let (start_len, start_id) = {
                let s = self.items.get(start_piece);
                (s.trace().polyline.lines.len() - 2, s.id().0)
            };
            let (end_len, end_id) = {
                let s = self.items.get(end_piece);
                (s.trace().polyline.lines.len() - 2, s.id().0)
            };
            // the pieces have no precalculated shapes yet: calculate them now (Java computes
            // them lazily in insert and in later queries; the values are the same).
            let start_shapes = self.item_tree_shapes(t, start_piece);
            let end_shapes = self.item_tree_shapes(t, end_piece);
            let tree = &mut self.search_trees.trees[t];
            let mut from_entries: Vec<Option<LeafId>> = tree.item_leaves(from).expect("reuseEntriesAfterCutout: no entries").to_vec();
            let mut start_leaves: Vec<Option<LeafId>> = vec![None; start_len];
            let start_object = TreeObject::Item { key: start_piece, id: start_id };
            for i in 0..start_len.saturating_sub(1) {
                start_leaves[i] = from_entries[i];
                if let Some(l) = start_leaves[i] {
                    tree.tree.set_leaf_object(l, start_object);
                    tree.tree.set_leaf_shape_index(l, i as i32);
                }
                from_entries[i] = None;
            }
            tree.set_shapes(start_piece, start_shapes);
            start_leaves[start_len - 1] = tree.insert_item_shape(start_piece, start_id, start_len as i32 - 1);
            let mut end_leaves: Vec<Option<LeafId>> = vec![None; end_len];
            tree.set_shapes(end_piece, end_shapes);
            end_leaves[0] = tree.insert_item_shape(end_piece, end_id, 0);
            let end_object = TreeObject::Item { key: end_piece, id: end_id };
            let from_len = from_entries.len();
            for (i, end_leaf) in end_leaves.iter_mut().enumerate().skip(1) {
                let from_index = from_len - end_len + i;
                *end_leaf = from_entries[from_index];
                if let Some(l) = *end_leaf {
                    tree.tree.set_leaf_object(l, end_object);
                    tree.tree.set_leaf_shape_index(l, i as i32);
                }
                from_entries[from_index] = None;
            }
            tree.set_leaves(from, from_entries);
            tree.set_leaves(start_piece, start_leaves);
            tree.set_leaves(end_piece, end_leaves);
        }
    }

    /// Java `ShapeSearchTree.changeItemShape(item, shapeIndex, newShape)` on tree `t`.
    pub fn change_item_shape(&mut self, t: usize, key: ItemKey, shape_index: i32, new_shape: TileShape) {
        let id = self.items.get(key).id().0;
        let old_shapes = self.item_tree_shapes(t, key);
        let tree = &mut self.search_trees.trees[t];
        let old_entries: Vec<Option<LeafId>> = tree.item_leaves(key).expect("changeItemShape: no entries").to_vec();
        let mut new_leaves: Vec<Option<LeafId>> = vec![None; old_entries.len()];
        let mut new_shapes: Vec<Option<TileShape>> = vec![None; old_entries.len()];
        if let Some(l) = old_entries[shape_index as usize] {
            tree.tree.remove_leaf(l);
        }
        for i in 0..old_entries.len() {
            if i as i32 == shape_index {
                new_shapes[i] = Some(new_shape.clone());
            } else {
                new_shapes[i] = old_shapes.get(i).cloned().flatten();
                new_leaves[i] = old_entries[i];
            }
        }
        tree.set_shapes(key, Arc::from(new_shapes));
        new_leaves[shape_index as usize] = tree.insert_item_shape(key, id, shape_index);
        tree.set_leaves(key, new_leaves);
    }

    /// Java `ShapeSearchTree.reduceTraceShapeAtTiePin(tiePin, trace)` on tree `t`.
    pub fn reduce_trace_shape_at_tie_pin(&mut self, t: usize, tie_pin: ItemKey, trace: ItemKey) {
        let trace_layer = self.items.get(trace).trace().layer;
        let pin_shape = {
            let pin = self.items.get(tie_pin);
            let from = pin.first_layer(self);
            let to = pin.last_layer(self);
            if trace_layer < from || trace_layer > to {
                log::warn!("DrillItem.get_tree_shape_on_layer: layer out of range");
                return;
            }
            self.tree_shape(t, tie_pin, trace_layer - from)
        };
        let Some(pin_shape) = pin_shape else { return };
        let pin_center = self.items.get(tie_pin).center(self);
        let (trace_shape_no, compare_corner) = {
            let item = self.items.get(trace);
            let tr = item.trace();
            if item.first_corner() == pin_center {
                (0, tr.polyline.corner_approx(1))
            } else if item.last_corner() == pin_center {
                (tr.corner_count() - 2, tr.polyline.corner_approx(tr.corner_count() - 2))
            } else {
                return;
            }
        };
        let Some(trace_shape) = self.tree_shape(t, trace, trace_shape_no) else { return };
        let intersection = trace_shape.intersection(&pin_shape);
        if intersection.dimension() < 2 {
            return;
        }
        let shape_pieces = trace_shape.cutout(&pin_shape).unwrap_or_default();
        let mut new_trace_shape: Option<TileShape> = None;
        for piece in shape_pieces {
            if piece.dimension() == 2 && (new_trace_shape.is_none() || piece.contains_float(&compare_corner)) {
                new_trace_shape = Some(piece);
            }
        }
        let new_trace_shape = new_trace_shape.unwrap_or_else(|| TileShape::Simplex(fr_geom::Simplex::empty()));
        self.change_item_shape(t, trace, trace_shape_no, new_trace_shape);
    }

    /// Java `searchTreeManager` access for the autorouter.
    pub fn search_trees(&self) -> &SearchTreeManager {
        &self.search_trees
    }

    /// Mutable access to the trees (rooms are inserted by the autorouter).
    pub fn search_trees_mut(&mut self) -> &mut SearchTreeManager {
        &mut self.search_trees
    }

}

/// Sorted, de-duplicated objects of tree entries (Java `TreeSet<SearchTreeObject>`).
pub(crate) fn objects_of(entries: Vec<TreeEntry>) -> Vec<TreeObject> {
    let mut result: Vec<TreeObject> = entries.into_iter().map(|e| e.object).collect();
    result.sort_by(|a, b| a.java_cmp(b));
    result.dedup_by(|a, b| a.java_cmp(b) == Ordering::Equal);
    result
}

/// Constants of `BoardOutline` used here.
struct BoardOutlineConsts;
impl BoardOutlineConsts {
    const HALF_WIDTH: i32 = super::item::BoardOutline::HALF_WIDTH;
}

impl ShapeSearchTree {
    /// The Java id of a stored room.
    pub fn room_id(&self, key: RoomKey) -> Option<i32> {
        self.rooms.get(&key).map(|r| r.id)
    }
}
