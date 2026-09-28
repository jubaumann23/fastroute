//! Secondary spatial index over the leaves of a [`MinAreaTree`](super::MinAreaTree) (performance
//! only): a hierarchy of uniform grids over the x/y ranges of the leaf bounding shapes.
//!
//! It answers "which leaves may intersect this query" with a superset of candidates; the caller
//! applies the same exact `RegularTileShape.intersects` test the tree traversal applies to the
//! leaves. For every combination of `IntBox` / `IntOctagon` that test fails when
//! `leaf.lx > q.rx || q.lx > leaf.rx` (same for y), so for a query with a proper x/y range
//! (`lx <= rx`, `ly <= ry`) every leaf with a proper range that passes the test is found through
//! the cells of the query's x/y range. Leaves with an improper range are kept in a list that is
//! tested for every query; queries with an improper range are not supported (callers use the tree
//! traversal).
//!
//! Level `k` has cells of side `2^(shift + k)`; a leaf is stored in the finest level where it
//! spans at most `SPAN + 1` cells per axis, and in each of those cells. It is reported only in
//! the cell of the reference point `(max(leaf.lx, q.lx), max(leaf.ly, q.ly))`, so no
//! deduplication is needed. [`LayeredGrid`] keeps one such grid per layer (leaf mask bit).
//!
//! The set of leaves found equals the set the Java traversal finds: the bounding shape of an
//! inner node contains the bounding shapes of its children (in terms of the box / octagon
//! parameters, which is what the intersection test compares), so every leaf that intersects the
//! query is reached by the traversal. The randomized test
//! `indexed_and_masked_queries_match_the_traversal` and the `FASTROUTE_VERIFY_CACHES=1` mode check
//! this.
//!
//! Storage is flat (cell heads + an entry slab with free list), so cloning is a few `memcpy`s.

use fr_geom::RegularTileShape;

const NONE: u32 = u32::MAX;
/// A leaf is stored in the finest level where it spans at most `SPAN + 1` cells per axis.
const SPAN: i64 = 3;
/// Finest level cells per leaf.
const DENS: i64 = 2;

/// The x/y range `(lx, ly, rx, ry)` of a bounding shape (Java field names: `ll`/`ur` of a box,
/// `leftX`/`bottomY`/`rightX`/`topY` of an octagon).
#[inline]
pub(crate) fn xy_range(s: &RegularTileShape) -> (i32, i32, i32, i32) {
    match s {
        RegularTileShape::IntBox(b) => (b.ll.x, b.ll.y, b.ur.x, b.ur.y),
        RegularTileShape::IntOctagon(o) => (o.left_x, o.bottom_y, o.right_x, o.top_y),
    }
}

#[derive(Clone, Debug)]
struct Entry {
    node: u32,
    next: u32,
    mask: u64,
    lx: i32,
    ly: i32,
    rx: i32,
    ry: i32,
}

#[derive(Clone, Copy, Debug)]
struct Level {
    shift: u32,
    nx: i64,
    ny: i64,
    /// Index of the first cell of this level in `heads`.
    offset: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct LeafGrid {
    origin_x: i64,
    origin_y: i64,
    levels: Vec<Level>,
    heads: Vec<u32>,
    entries: Vec<Entry>,
    free_entry: u32,
    /// Leaves with an improper x/y range (node, mask), tested by every query.
    improper: Vec<(u32, u64)>,
}

/// Position of a leaf in the grid: level and cell range.
type Span = (usize, i64, i64, i64, i64);

impl LeafGrid {
    /// A grid covering `extent` (x/y range) with about `target_cells` cells in the finest level.
    pub(crate) fn new(extent: (i32, i32, i32, i32), target_cells: i64) -> LeafGrid {
        let (lx, ly, rx, ry) = extent;
        let w = (rx as i64 - lx as i64).max(1);
        let h = (ry as i64 - ly as i64).max(1);
        let target = target_cells.clamp(16, 1 << 20);
        // finest cell side: smallest power of two with (w / side + 1) * (h / side + 1) <= target
        let mut shift = 0u32;
        while shift < 40 && ((w >> shift) + 1) * ((h >> shift) + 1) > target {
            shift += 1;
        }
        let mut levels = Vec::new();
        let mut offset = 0usize;
        loop {
            let nx = (w >> shift) + 1;
            let ny = (h >> shift) + 1;
            levels.push(Level { shift, nx, ny, offset });
            offset += (nx * ny) as usize;
            if nx <= 2 && ny <= 2 {
                break;
            }
            shift += 1;
        }
        LeafGrid {
            origin_x: lx as i64,
            origin_y: ly as i64,
            levels,
            heads: vec![NONE; offset],
            entries: Vec::new(),
            free_entry: NONE,
            improper: Vec::new(),
        }
    }

    #[inline]
    fn cell_x(&self, level: &Level, x: i32) -> i64 {
        ((x as i64 - self.origin_x) >> level.shift).clamp(0, level.nx - 1)
    }

    #[inline]
    fn cell_y(&self, level: &Level, y: i32) -> i64 {
        ((y as i64 - self.origin_y) >> level.shift).clamp(0, level.ny - 1)
    }

    /// The level and cell range of a leaf with a proper range.
    #[inline]
    fn span(&self, lx: i32, ly: i32, rx: i32, ry: i32) -> Span {
        let last = self.levels.len() - 1;
        for (k, level) in self.levels.iter().enumerate() {
            let (cx0, cx1) = (self.cell_x(level, lx), self.cell_x(level, rx));
            let (cy0, cy1) = (self.cell_y(level, ly), self.cell_y(level, ry));
            if (cx1 - cx0 <= SPAN && cy1 - cy0 <= SPAN) || k == last {
                return (k, cx0, cx1, cy0, cy1);
            }
        }
        unreachable!()
    }

    pub(crate) fn insert(&mut self, node: u32, bounds: &RegularTileShape, mask: u64) {
        let (lx, ly, rx, ry) = xy_range(bounds);
        if lx > rx || ly > ry {
            self.improper.push((node, mask));
            return;
        }
        let (k, cx0, cx1, cy0, cy1) = self.span(lx, ly, rx, ry);
        let level = self.levels[k];
        for cy in cy0..=cy1 {
            for cx in cx0..=cx1 {
                let cell = level.offset + (cy * level.nx + cx) as usize;
                let entry = Entry { node, next: self.heads[cell], mask, lx, ly, rx, ry };
                let e = if self.free_entry != NONE {
                    let e = self.free_entry;
                    self.free_entry = self.entries[e as usize].next;
                    self.entries[e as usize] = entry;
                    e
                } else {
                    self.entries.push(entry);
                    (self.entries.len() - 1) as u32
                };
                self.heads[cell] = e;
            }
        }
    }

    pub(crate) fn remove(&mut self, node: u32, bounds: &RegularTileShape) {
        let (lx, ly, rx, ry) = xy_range(bounds);
        if lx > rx || ly > ry {
            let pos = self.improper.iter().position(|&(n, _)| n == node).expect("LeafGrid: leaf not stored");
            self.improper.swap_remove(pos);
            return;
        }
        let (k, cx0, cx1, cy0, cy1) = self.span(lx, ly, rx, ry);
        let level = self.levels[k];
        for cy in cy0..=cy1 {
            for cx in cx0..=cx1 {
                let cell = level.offset + (cy * level.nx + cx) as usize;
                let mut prev = NONE;
                let mut e = self.heads[cell];
                while e != NONE && self.entries[e as usize].node != node {
                    prev = e;
                    e = self.entries[e as usize].next;
                }
                assert!(e != NONE, "LeafGrid: leaf not in cell");
                let next = self.entries[e as usize].next;
                if prev == NONE {
                    self.heads[cell] = next;
                } else {
                    self.entries[prev as usize].next = next;
                }
                self.entries[e as usize].next = self.free_entry;
                self.free_entry = e;
            }
        }
    }

    /// Calls `f(node)` for every leaf whose mask intersects `mask` and whose x/y range overlaps
    /// the x/y range of the query (each at most once), and for every leaf with an improper range
    /// and a matching mask. Returns false (without calling `f`) if the query has an improper
    /// range.
    #[inline]
    pub(crate) fn candidates(&self, query: &RegularTileShape, mask: u64, mut f: impl FnMut(u32)) -> bool {
        let (qlx, qly, qrx, qry) = xy_range(query);
        if qlx > qrx || qly > qry {
            return false;
        }
        for &(node, m) in &self.improper {
            if m & mask != 0 {
                f(node);
            }
        }
        for level in &self.levels {
            let (cx0, cx1) = (self.cell_x(level, qlx), self.cell_x(level, qrx));
            let (cy0, cy1) = (self.cell_y(level, qly), self.cell_y(level, qry));
            for cy in cy0..=cy1 {
                let row = level.offset + (cy * level.nx) as usize;
                for cx in cx0..=cx1 {
                    let mut e = self.heads[row + cx as usize];
                    while e != NONE {
                        let entry = &self.entries[e as usize];
                        e = entry.next;
                        if entry.mask & mask == 0 || entry.lx > qrx || entry.rx < qlx || entry.ly > qry || entry.ry < qly {
                            continue;
                        }
                        // report only in the cell of the reference point
                        if self.cell_x(level, entry.lx.max(qlx)) != cx || self.cell_y(level, entry.ly.max(qly)) != cy {
                            continue;
                        }
                        f(entry.node);
                    }
                }
            }
        }
        true
    }
}

/// One [`LeafGrid`] per single-bit leaf mask (the search trees use the bit of the layer of the
/// leaf) plus one for all other masks, so that a query restricted to one layer scans only the
/// leaves of that layer (and the multi-bit ones).
#[derive(Clone, Debug)]
pub(crate) struct LayeredGrid {
    single: Vec<Option<LeafGrid>>,
    multi: LeafGrid,
    extent: (i32, i32, i32, i32),
    /// Number of leaves when the grid was built (for the rebuild heuristic).
    pub(crate) built_leaf_count: i32,
}

impl LayeredGrid {
    /// `counts[b]`: number of leaves with mask `1 << b`, `counts[64]`: all others.
    pub(crate) fn new(extent: (i32, i32, i32, i32), counts: &[i64; 65], built_leaf_count: i32) -> LayeredGrid {
        let single = (0..64)
            .map(|b| if counts[b] > 0 { Some(LeafGrid::new(extent, DENS * counts[b])) } else { None })
            .collect();
        LayeredGrid { single, multi: LeafGrid::new(extent, DENS * counts[64]), extent, built_leaf_count }
    }

    #[inline]
    fn single_bit(mask: u64) -> Option<usize> {
        if mask.count_ones() == 1 {
            Some(mask.trailing_zeros() as usize)
        } else {
            None
        }
    }

    fn grid_mut(&mut self, mask: u64) -> &mut LeafGrid {
        match Self::single_bit(mask) {
            Some(b) => {
                let extent = self.extent;
                self.single[b].get_or_insert_with(|| LeafGrid::new(extent, DENS * 16))
            }
            None => &mut self.multi,
        }
    }

    pub(crate) fn insert(&mut self, node: u32, bounds: &RegularTileShape, mask: u64) {
        self.grid_mut(mask).insert(node, bounds, mask);
    }

    pub(crate) fn remove(&mut self, node: u32, bounds: &RegularTileShape, mask: u64) {
        self.grid_mut(mask).remove(node, bounds);
    }

    /// [`LeafGrid::candidates`] over all grids that can hold leaves matching `mask`.
    #[inline]
    pub(crate) fn candidates(&self, query: &RegularTileShape, mask: u64, mut f: impl FnMut(u32)) -> bool {
        if !self.multi.candidates(query, mask, &mut f) {
            return false;
        }
        match Self::single_bit(mask) {
            Some(b) => {
                if let Some(g) = &self.single[b] {
                    g.candidates(query, mask, &mut f);
                }
            }
            None => {
                for g in self.single.iter().flatten() {
                    g.candidates(query, mask, &mut f);
                }
            }
        }
        true
    }
}
