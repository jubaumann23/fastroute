//! Tests of `MinAreaTree`: JDK ground truth (tree shape and query results of the real Java
//! classes, see `testdata/DatastructuresGen.java`), brute force comparisons and the ports of
//! the Java `MinAreaTreeLeafLifecycleTest` cases.

use super::*;
use fr_geom::{IntOctagon, ShapeBoundingDirections, TileShape};
use fr_jcompat::JavaRandom;
use std::collections::BTreeMap;

fn parse_shape(tok: &mut std::slice::Iter<'_, &str>) -> TileShape {
    let kind = *tok.next().unwrap();
    let n = |t: &mut std::slice::Iter<'_, &str>| t.next().unwrap().parse::<i32>().unwrap();
    match kind {
        "B" => {
            let (a, b, c, d) = (n(tok), n(tok), n(tok), n(tok));
            TileShape::IntBox(IntBox::new(a, b, c, d))
        }
        "O" => {
            let v: Vec<i32> = (0..8).map(|_| n(tok)).collect();
            TileShape::IntOctagon(IntOctagon::new(v[0], v[1], v[2], v[3], v[4], v[5], v[6], v[7]))
        }
        k => panic!("bad shape kind {k}"),
    }
}

/// Brute force: all live leaves whose bounding shape intersects `q`, sorted like Java.
fn brute_force(tree: &MinAreaTree<i32>, live: &BTreeMap<i32, Vec<Option<LeafId>>>, q: &RegularTileShape) -> Vec<LeafId> {
    let mut v: Vec<LeafId> = live
        .values()
        .flatten()
        .flatten()
        .copied()
        .filter(|l| regular_intersects(tree.leaf_bounding_shape(*l), q))
        .collect();
    v.sort_by(|a, b| tree.compare_leaves(*a, *b, |x, y| x.cmp(y)));
    v
}

fn fmt_leaves(tree: &MinAreaTree<i32>, v: &[LeafId]) -> String {
    v.iter()
        .map(|l| format!(" {}:{}", tree.leaf_object(*l), tree.leaf_shape_index(*l)))
        .collect()
}

#[test]
fn jdk_ground_truth_tree_shape_and_queries() {
    let data = include_str!("../testdata/tree.txt");
    let mut tree: Option<MinAreaTree<i32>> = None;
    let mut live: BTreeMap<i32, Vec<Option<LeafId>>> = BTreeMap::new();
    let mut counts = [0usize; 4];
    let mut cursor = TreeCursor::new();
    for (line_no, line) in data.lines().enumerate() {
        let toks: Vec<&str> = line.split(' ').collect();
        let mut it = toks.iter();
        let ctx = || format!("line {}: {}", line_no + 1, &line[..line.len().min(120)]);
        match *it.next().unwrap() {
            "T" => {
                let dirs = match *it.next().unwrap() {
                    "45" => ShapeBoundingDirections::FortyfiveDegree,
                    _ => ShapeBoundingDirections::Orthogonal,
                };
                tree = Some(MinAreaTree::new(dirs));
                live.clear();
            }
            "I" => {
                let t = tree.as_mut().unwrap();
                let id: i32 = it.next().unwrap().parse().unwrap();
                let n: usize = it.next().unwrap().parse().unwrap();
                let shapes: Vec<TileShape> = (0..n).map(|_| parse_shape(&mut it)).collect();
                let entries = t.insert_shapes(id, &shapes);
                assert!(entries.iter().all(|e| e.is_some()));
                live.insert(id, entries);
                counts[0] += 1;
            }
            "R" => {
                let t = tree.as_mut().unwrap();
                let id: i32 = it.next().unwrap().parse().unwrap();
                let entries = live.remove(&id).unwrap();
                t.remove(&entries);
                // removing again is a no-op
                let size = t.size();
                t.remove(&entries);
                assert_eq!(size, t.size());
                counts[1] += 1;
            }
            "Q" => {
                let t = tree.as_ref().unwrap();
                let q = parse_shape(&mut it).bounding_shape(&t.bounding_directions()).unwrap();
                assert_eq!(*it.next().unwrap(), ":");
                let expected: String = it.map(|s| format!(" {s}")).collect();
                let got = t.overlaps(&q);
                assert_eq!(fmt_leaves(t, &got), expected, "{}", ctx());
                assert_eq!(got, brute_force(t, &live, &q), "{}", ctx());
                // cursor traversal == unsorted traversal
                let mut unsorted = Vec::new();
                t.overlapping_leaves_unsorted(&q, &mut Vec::new(), &mut unsorted);
                cursor.start(t);
                let mut via_cursor = Vec::new();
                while let Some(l) = cursor.next_leaf(t, &q) {
                    via_cursor.push(l);
                }
                assert_eq!(unsorted, via_cursor);
                counts[2] += 1;
            }
            "A" => {
                let t = tree.as_ref().unwrap();
                let expected: String = it.map(|s| format!(" {s}")).collect();
                let got: String = t
                    .to_array()
                    .iter()
                    .map(|l| format!(" {}:{}:{}", t.leaf_object(*l), t.leaf_shape_index(*l), t.distance_to_root(*l)))
                    .collect();
                assert_eq!(got, expected, "{}", ctx());
                counts[3] += 1;
            }
            k => panic!("unknown record {k}"),
        }
        if let Some(t) = &tree {
            t.validate().unwrap_or_else(|e| panic!("{}: {e}", ctx()));
        }
    }
    assert!(counts.iter().all(|&c| c > 50), "{counts:?}");
}

fn random_shape(r: &mut JavaRandom, octagon: bool, range: i32, max_size: i32) -> TileShape {
    let lx = r.next_int_bound(2 * range) - range;
    let ly = r.next_int_bound(2 * range) - range;
    let w = if r.next_int_bound(8) == 0 { 0 } else { r.next_int_bound(max_size) };
    let h = if r.next_int_bound(8) == 0 { 0 } else { r.next_int_bound(max_size) };
    let (rx, uy) = (lx + w, ly + h);
    if octagon && r.next_boolean() {
        let s = (w + h) / 2 + 1;
        let o = IntOctagon::new(
            lx,
            ly,
            rx,
            uy,
            lx - uy + r.next_int_bound(s),
            rx - ly - r.next_int_bound(s),
            lx + ly + r.next_int_bound(s),
            rx + uy - r.next_int_bound(s),
        )
        .normalize();
        if !o.is_empty() {
            return TileShape::IntOctagon(o);
        }
    }
    TileShape::IntBox(IntBox::new(lx, ly, rx, uy))
}

#[test]
fn random_operations_against_brute_force() {
    for (seed, dirs) in [
        (1i64, ShapeBoundingDirections::Orthogonal),
        (2, ShapeBoundingDirections::FortyfiveDegree),
        (3, ShapeBoundingDirections::FortyfiveDegree),
    ] {
        let octagons = dirs == ShapeBoundingDirections::FortyfiveDegree;
        let mut r = JavaRandom::new(seed);
        let mut tree: MinAreaTree<i32> = MinAreaTree::new(dirs);
        let mut live: BTreeMap<i32, Vec<Option<LeafId>>> = BTreeMap::new();
        let mut next_id = 1;
        let mut stack = Vec::new();
        let mut out = Vec::new();
        for _ in 0..3000 {
            let op = r.next_int_bound(10);
            if op < 5 || live.is_empty() {
                let n = 1 + r.next_int_bound(3);
                let shapes: Vec<TileShape> = (0..n).map(|_| random_shape(&mut r, octagons, 5000, 800)).collect();
                // ids are inserted in random order to exercise the comparator
                let id = if r.next_boolean() { next_id } else { -next_id };
                next_id += 1;
                live.insert(id, tree.insert_shapes(id, &shapes));
            } else if op < 8 {
                let k = r.next_int_bound(live.len() as i32) as usize;
                let id = *live.keys().nth(k).unwrap();
                let entries = live.remove(&id).unwrap();
                // remove single leaves in random order
                let mut e = entries.clone();
                fr_jcompat::shuffle(&mut e, &mut r);
                for leaf in e.into_iter().flatten() {
                    tree.remove_leaf(leaf);
                    assert!(!tree.contains_leaf(leaf));
                }
            } else {
                let q = random_shape(&mut r, octagons, 5000, 3000).bounding_shape(&dirs).unwrap();
                tree.overlaps_into(&q, &mut stack, &mut out, |a, b| a.cmp(b));
                assert_eq!(out, brute_force(&tree, &live, &q));
            }
            let expected_size: usize = live.values().map(|v| v.len()).sum();
            assert_eq!(tree.size() as usize, expected_size);
            tree.validate().unwrap();
        }
        // query everything
        let all = RegularTileShape::IntBox(IntBox::new(-100_000, -100_000, 100_000, 100_000));
        assert_eq!(tree.overlaps(&all).len(), tree.size() as usize);
        assert_eq!(tree.to_array().len(), tree.size() as usize);
        let stats = tree.statistics();
        assert_eq!(stats.entry_count, tree.size());
        assert!(stats.maximum_depth as f64 >= stats.average_depth);
    }
}

#[test]
fn result_order_uses_comparator_then_shape_index() {
    let mut tree: MinAreaTree<i32> = MinAreaTree::new(ShapeBoundingDirections::Orthogonal);
    let b = TileShape::IntBox(IntBox::new(0, 0, 10, 10));
    for id in [5, 1, 3] {
        tree.insert_shapes(id, &[b.clone(), b.clone()]);
    }
    let q = RegularTileShape::IntBox(IntBox::new(1, 1, 2, 2));
    // descending ids, like Java Item.compareTo (other.id - id)
    let v = tree.overlaps_by(&q, |a, b| b.cmp(a));
    let s: Vec<(i32, i32)> = v.iter().map(|l| (tree.leaf_object(*l), tree.leaf_shape_index(*l))).collect();
    assert_eq!(s, vec![(5, 0), (5, 1), (3, 0), (3, 1), (1, 0), (1, 1)]);
}

#[test]
fn leaf_payload_can_be_changed() {
    let mut tree: MinAreaTree<i32> = MinAreaTree::new(ShapeBoundingDirections::FortyfiveDegree);
    let e = tree.insert_shapes(1, &[TileShape::IntBox(IntBox::new(0, 0, 10, 10))]);
    let leaf = e[0].unwrap();
    tree.set_leaf_object(leaf, 7);
    tree.set_leaf_shape_index(leaf, 3);
    let l = tree.leaf(leaf).unwrap();
    assert_eq!((l.object, l.shape_index_in_object), (7, 3));
    assert_eq!(*l.bounding_shape, RegularTileShape::IntOctagon(IntBox::new(0, 0, 10, 10).to_int_octagon()));
}

/// Port of `MinAreaTreeLeafLifecycleTest.removingSameLeafTwiceDoesNotCorruptTree`.
#[test]
fn removing_same_leaf_twice_does_not_corrupt_tree() {
    let query = RegularTileShape::IntBox(IntBox::new(-100, -100, 100, 100));
    let mut tree: MinAreaTree<i32> = MinAreaTree::new(ShapeBoundingDirections::FortyfiveDegree);
    let first = tree.insert_shapes(1, &[TileShape::IntBox(IntBox::new(-10, -10, 10, 10))]);
    tree.remove_leaf(first[0].unwrap());
    assert_eq!(tree.size(), 0);
    tree.remove_leaf(first[0].unwrap());
    assert_eq!(tree.size(), 0);
    // the freed slot is reused; the stale id must not alias the new leaf
    let second = tree.insert_shapes(2, &[TileShape::IntBox(IntBox::new(-20, -20, 20, 20))]);
    assert_eq!(first[0].unwrap().index(), second[0].unwrap().index());
    tree.remove_leaf(first[0].unwrap());
    assert_eq!(tree.size(), 1);
    assert!(tree.overlaps(&query).contains(&second[0].unwrap()));
    assert!(tree.leaf(first[0].unwrap()).is_none());
}

/// Port of the single-threaded part of `MinAreaTreeLeafLifecycleTest`: a surviving leaf stays
/// visible and a removed one disappears.
#[test]
fn removal_keeps_surviving_leaf() {
    let query = RegularTileShape::IntBox(IntBox::new(-100, -100, 100, 100));
    let mut tree: MinAreaTree<i32> = MinAreaTree::new(ShapeBoundingDirections::FortyfiveDegree);
    let removed = tree.insert_shapes(1, &[TileShape::IntBox(IntBox::new(-20, -20, -5, -5))]);
    let surviving = tree.insert_shapes(2, &[TileShape::IntBox(IntBox::new(5, 5, 20, 20))]);
    assert_eq!(tree.overlaps(&query).len(), 2);
    tree.remove_leaf(removed[0].unwrap());
    let after = tree.overlaps(&query);
    assert_eq!(after, vec![surviving[0].unwrap()]);
    assert_eq!(tree.size(), 1);
    tree.remove_leaf(surviving[0].unwrap());
    assert!(tree.overlaps(&query).is_empty());
    assert!(tree.is_empty());
}

/// The traversal order of `TreeCursor` follows the `ArrayStack` LIFO order and respects a query
/// shape that changes during the traversal.
#[test]
fn cursor_traversal_order_and_shrinking_query() {
    let mut tree: MinAreaTree<i32> = MinAreaTree::new(ShapeBoundingDirections::Orthogonal);
    for i in 0..4 {
        tree.insert_shapes(i, &[TileShape::IntBox(IntBox::new(i * 100, 0, i * 100 + 10, 10))]);
    }
    let mut cursor = TreeCursor::new();
    cursor.start(&tree);
    let all = RegularTileShape::IntBox(IntBox::new(-1000, -1000, 1000, 1000));
    let mut seen = Vec::new();
    while let Some(l) = cursor.next_leaf(&tree, &all) {
        seen.push(tree.leaf_object(l));
    }
    // reverse in-order for this shape: second children are popped first
    let mut in_order: Vec<i32> = tree.to_array().iter().map(|l| tree.leaf_object(*l)).collect();
    in_order.reverse();
    assert_eq!(seen, in_order);

    cursor.start(&tree);
    let first = cursor.next_leaf(&tree, &all).unwrap();
    let nothing = RegularTileShape::IntBox(IntBox::new(5000, 5000, 5001, 5001));
    assert!(cursor.next_leaf(&tree, &nothing).is_none());
    assert!(tree.contains_leaf(first));
}

#[test]
fn empty_tree() {
    let tree: MinAreaTree<i32> = MinAreaTree::new(ShapeBoundingDirections::Orthogonal);
    let q = RegularTileShape::IntBox(IntBox::new(0, 0, 1, 1));
    assert!(tree.overlaps(&q).is_empty());
    assert!(tree.to_array().is_empty());
    assert!(tree.validate().is_ok());
    let mut c = TreeCursor::new();
    c.start(&tree);
    assert!(c.next_leaf(&tree, &q).is_none());
}
