//! pcbkit hook H1: `BasicBoard::place_component` must give the same board as loading a DSN
//! whose place record already carries the new pose.
//!
//! (This test lives in fr-io because fr-engine cannot depend on the DSN loader.)

use fr_engine::board::{BasicBoard, ItemKey};
use fr_geom::{ConvexShape, IntBox, TileShape};
use fr_io::{build_board, load_bytes};

const SRC: &str = include_str!("../testdata/dsn/synth_rules.dsn");
const R1_OLD: &str = "(place R1 10000 10000 front 0 (PN 10k))";
const U2_OLD: &str = "(place U2 40000 25000 front 135.5)";

fn board_of(src: &str) -> BasicBoard {
    build_board(load_bytes(src.as_bytes()).expect("load"))
}

fn component_no(b: &BasicBoard, name: &str) -> i32 {
    b.components.get_all().find(|c| c.name == name).unwrap_or_else(|| panic!("no component {name}")).id
}

fn describe(b: &BasicBoard, key: ItemKey) -> String {
    let item = b.item(key);
    let mut s = format!("id={} fixed={:?} nets={:?}", item.id().0, item.fixed_state(), item.net_numbers());
    if item.is_pin() {
        s += &format!(" center={:?} shapes={:?}", item.center(b), item.drill_shapes(b));
    }
    if let Some(a) = item.as_obstacle_area() {
        s += &format!(" kind={:?} area={:?}", a.kind, a.get_area(b));
    }
    if let Some(a) = item.as_conduction_area() {
        s += &format!(" cond={:?}", a.area().get_area(b));
    }
    if let Some(o) = item.as_component_outline() {
        s += &format!(" outline={:?}", o.get_area(b));
    }
    s
}

fn ids_overlapping(b: &BasicBoard, shape: &TileShape) -> Vec<i32> {
    let mut ids = Vec::new();
    for layer in 0..b.layer_count() {
        for key in b.overlapping_items_of(&ConvexShape::Tile(shape.clone()), layer).iter() {
            ids.push(layer * 1_000_000 + b.item(key).id().0);
        }
    }
    ids.sort();
    ids
}

/// Moves `name` on `moved` to the pose it has on `reference` and compares everything.
fn assert_move_equals_reload(name: &str, edited_src: &str) {
    let mut moved = board_of(SRC);
    let reference = board_of(edited_src);
    let no = component_no(&moved, name);
    assert_eq!(no, component_no(&reference, name));
    let (loc, rot, front) = {
        let c = reference.components.get(no);
        (c.get_location().cloned().expect("placed"), c.get_rotation_in_degree(), c.placed_on_front())
    };
    let before = moved.components.get(no).get_location().cloned();
    assert_ne!(before, Some(loc.clone()), "the test must actually move the part");
    let Point::Int(loc) = loc else { panic!("integer location expected") };
    let rev = moved.revision();
    moved.place_component(no, loc, rot, front).expect("place_component");
    assert_ne!(moved.revision(), rev, "revision is bumped");

    assert_eq!(moved.components.get(no).get_location(), reference.components.get(no).get_location());
    assert_eq!(moved.components.get(no).get_rotation_in_degree(), reference.components.get(no).get_rotation_in_degree());

    let a = moved.get_component_items(no);
    let b = reference.get_component_items(no);
    assert!(a.len() > 3, "pins, keepouts and outlines expected, got {}", a.len());
    assert_eq!(a.len(), b.len());
    let mut probes = Vec::new();
    for (ka, kb) in a.iter().zip(b.iter()) {
        assert_eq!(describe(&moved, *ka), describe(&reference, *kb));
        let ta = moved.item_tree_shapes(0, *ka);
        let tb = reference.item_tree_shapes(0, *kb);
        assert_eq!(format!("{ta:?}"), format!("{tb:?}"));
        probes.extend(tb.iter().flatten().cloned());
    }
    // tree queries: every item shape of the component and the whole board
    let whole = TileShape::IntBox(reference.bounding_box());
    probes.push(whole);
    probes.push(TileShape::IntBox(IntBox::new(-100_000, -100_000, 100_000, 100_000)));
    for p in &probes {
        assert_eq!(ids_overlapping(&moved, p), ids_overlapping(&reference, p), "query {p:?}");
    }
    // the component keeps being movable and the other components are untouched
    for other in reference.components.get_all() {
        if other.id != no {
            assert_eq!(moved.components.get(other.id).get_location(), other.get_location());
        }
    }
}

use fr_geom::Point;

#[test]
fn move_and_rotate_90_equals_reload() {
    let edited = SRC.replace(R1_OLD, "(place R1 13000 8000 front 90 (PN 10k))");
    assert_ne!(edited, SRC);
    assert_move_equals_reload("R1", &edited);
}

#[test]
fn move_and_rotate_45_equals_reload() {
    let edited = SRC.replace(R1_OLD, "(place R1 12500 9000 front 45 (PN 10k))");
    assert_move_equals_reload("R1", &edited);
}

#[test]
fn pure_translation_equals_reload() {
    let edited = SRC.replace(R1_OLD, "(place R1 11000 10500 front 0 (PN 10k))");
    assert_move_equals_reload("R1", &edited);
}

#[test]
fn non_right_angle_part_moves_to_another_odd_angle() {
    let edited = SRC.replace(U2_OLD, "(place U2 41000 24000 front 20.25)");
    assert_move_equals_reload("U2", &edited);
}

#[test]
fn side_change_and_bad_input_are_refused() {
    let mut b = board_of(SRC);
    let no = component_no(&b, "R1");
    let loc = match b.components.get(no).get_location().cloned().unwrap() {
        Point::Int(p) => p,
        _ => panic!(),
    };
    let rev = b.revision();
    assert!(b.place_component(no, loc, 0.0, false).is_err());
    assert!(b.place_component(0, loc, 0.0, true).is_err());
    assert!(b.place_component(9999, loc, 0.0, true).is_err());
    assert_eq!(b.revision(), rev, "a refused move changes nothing");
}
