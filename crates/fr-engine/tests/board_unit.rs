//! Unit tests of board internals that the Java comparison (`board_replay.rs`) does not reach
//! directly: item list cursor semantics, tombstones and compaction, snapshot clones.

use fr_engine::board::*;
use fr_engine::datastructures::ItemIdGenerator;
use fr_engine::ids::{FixedState, ItemId};
use fr_engine::library::{BoardLibrary, Packages, Padstacks};
use fr_engine::rules::{BoardRules, ClearanceMatrix};
use fr_engine::structure::{Communication, Components, CoordinateTransform, Layer, LayerStructure, SpecctraParserInfo, Unit};
use fr_geom::*;

fn board() -> BasicBoard {
    let ls = LayerStructure::new(vec![Layer::new("F.Cu", true), Layer::new("B.Cu", true)]);
    let matrix = ClearanceMatrix::get_default_instance(&ls, 200);
    let mut rules = BoardRules::new(ls.clone(), matrix);
    let class = rules.net_classes.append("default", &ls, false);
    for name in ["a", "b", "c"] {
        rules.nets.add(name, 1, false, class);
    }
    let padstacks = Padstacks::new(&ls);
    let comm = Communication::new(Unit::Mil, 1, Some(SpecctraParserInfo::default()), CoordinateTransform::new(1.0, 0.0, 0.0), ItemIdGenerator::new());
    let outline = PolylineShape::Tile(TileShape::IntBox(IntBox::new(0, 0, 100_000, 100_000)));
    BasicBoard::new(
        IntBox::new(-1000, -1000, 101_000, 101_000),
        ls,
        vec![outline],
        1,
        rules,
        BoardLibrary::new(padstacks, Packages::new()),
        Components::new(),
        comm,
    )
}

fn segment(board: &mut BasicBoard, x0: i32, y0: i32, x1: i32, y1: i32, net: i32) -> ItemKey {
    let p = Polyline::from_int_points(&[IntPoint::new(x0, y0), IntPoint::new(x1, y1)]);
    board.insert_trace_without_cleaning(p, 0, 100, &[net], 1, FixedState::Unfixed).unwrap()
}

#[test]
fn ids_and_list_order() {
    let mut b = board();
    let t1 = segment(&mut b, 1000, 1000, 5000, 1000, 1);
    let t2 = segment(&mut b, 1000, 3000, 5000, 3000, 2);
    // outline has id 1
    assert_eq!(b.item(t1).id(), ItemId(2));
    assert_eq!(b.item(t2).id(), ItemId(3));
    let ids: Vec<i32> = b.get_items().iter().map(|k| b.item(*k).id().0).collect();
    assert_eq!(ids, vec![3, 2, 1]);
    assert_eq!(b.get_item(ItemId(2)), Some(t1));
    assert_eq!(b.get_connectable_items(1), vec![t1]);
}

#[test]
fn cursor_prefetches_like_concurrent_skip_list_map() {
    let mut b = board();
    let keys: Vec<ItemKey> = (0..5).map(|i| segment(&mut b, 1000, 1000 + i * 2000, 5000, 1000 + i * 2000, 1)).collect();
    // list order: keys[4], keys[3], ..., keys[0], outline
    let mut cursor = b.item_cursor();
    assert_eq!(b.cursor_next(&mut cursor), Some(keys[4]));
    // keys[3] is prefetched now: removing it does not hide it from the cursor
    b.remove_item(keys[3]);
    assert_eq!(b.cursor_next(&mut cursor), Some(keys[3]));
    // keys[2] was prefetched when keys[3] was returned; removing keys[1] hides keys[1]
    b.remove_item(keys[1]);
    // inserted items have larger ids and are never visited
    segment(&mut b, 1000, 20000, 5000, 20000, 1);
    assert_eq!(b.cursor_next(&mut cursor), Some(keys[2]));
    assert_eq!(b.cursor_next(&mut cursor), Some(keys[0]));
    let outline = b.cursor_next(&mut cursor).unwrap();
    assert!(b.item(outline).is_board_outline());
    assert_eq!(b.cursor_next(&mut cursor), None);
}

#[test]
fn tombstones_and_compaction() {
    let mut b = board();
    let t1 = segment(&mut b, 1000, 1000, 5000, 1000, 1);
    let id = b.item(t1).id();
    b.remove_item(t1);
    // the removed item stays readable
    assert!(!b.item(t1).is_on_board());
    assert_eq!(b.item(t1).id(), id);
    assert_eq!(b.get_item(id), None);
    assert!(b.default_tree().item_leaves(t1).is_none());
    let freed = b.compact();
    assert_eq!(freed, vec![t1]);
    assert!(b.try_item(t1).is_none());
    // the slot is reused with a new generation
    let t2 = segment(&mut b, 1000, 3000, 5000, 3000, 1);
    assert_eq!(t2.index(), t1.index());
    assert_ne!(t2, t1);
    assert!(b.try_item(t1).is_none());
}

#[test]
fn clone_is_an_independent_snapshot() {
    let mut b = board();
    let t1 = segment(&mut b, 1000, 1000, 5000, 1000, 1);
    let snapshot = b.clone();
    b.remove_item(t1);
    segment(&mut b, 1000, 3000, 5000, 3000, 2);
    assert_eq!(snapshot.get_items().len(), 2);
    assert!(snapshot.item(t1).is_on_board());
    assert_eq!(b.get_items().len(), 2);
    assert!(!b.item(t1).is_on_board());
    // the id generator is part of the board state
    assert_eq!(b.communication.id_generator.clone().new_id_peek(), 4);
}

trait Peek {
    fn new_id_peek(&mut self) -> i32;
}

impl Peek for ItemIdGenerator {
    fn new_id_peek(&mut self) -> i32 {
        fr_engine::datastructures::IdGenerator::new_id(self)
    }
}

#[test]
fn normalize_combines_collinear_segments() {
    let mut b = board();
    let a = segment(&mut b, 1000, 1000, 3000, 1000, 1);
    segment(&mut b, 3000, 1000, 6000, 1000, 1);
    assert!(b.normalize_trace(a, None));
    let traces = b.get_traces();
    assert_eq!(traces.len(), 1);
    let t = b.item(traces[0]);
    assert_eq!(t.trace().corner_count(), 2);
    assert_eq!(t.first_corner(), Point::Int(IntPoint::new(1000, 1000)));
    assert_eq!(t.last_corner(), Point::Int(IntPoint::new(6000, 1000)));
    assert!(b.validate_tree_entries(traces[0]));
}

#[test]
fn crossing_traces_of_one_net_are_split() {
    let mut b = board();
    segment(&mut b, 1000, 5000, 9000, 5000, 1);
    let p = Polyline::from_int_points(&[IntPoint::new(5000, 1000), IntPoint::new(5000, 9000)]);
    b.insert_trace(p, 0, 100, &[1], 1, FixedState::Unfixed);
    // both traces are split at the crossing point
    assert_eq!(b.get_traces().len(), 4);
    let center = Point::Int(IntPoint::new(5000, 5000));
    for t in b.get_traces() {
        let item = b.item(t);
        assert!(item.first_corner() == center || item.last_corner() == center);
    }
}
