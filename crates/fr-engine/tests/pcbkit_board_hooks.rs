//! pcbkit hooks of the board (H2b: pieces of a split trace keep the fixed state).


use fr_engine::board::*;
use fr_engine::datastructures::ItemIdGenerator;
use fr_engine::ids::FixedState;
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

fn fixed_trace(b: &mut BasicBoard, fixed: FixedState) -> ItemKey {
    let p = Polyline::from_int_points(&[IntPoint::new(1000, 1000), IntPoint::new(9000, 1000)]);
    b.insert_trace_without_cleaning(p, 0, 100, &[1], 1, fixed).unwrap()
}

/// Fixed states of the traces on the board other than `parent` (the pieces).
fn piece_states(b: &BasicBoard, parent: ItemKey) -> Vec<FixedState> {
    b.get_items()
        .iter()
        .filter(|k| **k != parent && b.item(**k).as_trace().is_some() && b.item(**k).is_on_board())
        .map(|k| b.item(*k).fixed_state())
        .collect()
}

fn cut(b: &mut BasicBoard, key: ItemKey, x0: i32, x1: i32) {
    let shape = TileShape::IntBox(IntBox::new(x0, 0, x1, 2000));
    b.cutout_trace(key, &shape, 1);
}

#[test]
fn flag_defaults_off() {
    assert!(!board().keep_fixed_on_split);
}

// Upstream never removes a UserFixed item (`is_deletion_forbidden`), so a UserFixed parent
// survives a cut-out next to its pieces; the pieces are what the hook is about.
#[test]
fn user_fixed_pieces_keep_the_state_with_the_flag() {
    for fast_path in [true, false] {
        let mut b = board();
        b.keep_fixed_on_split = true;
        let t = fixed_trace(&mut b, FixedState::UserFixed);
        if fast_path { cut(&mut b, t, 4000, 5000) } else { cut(&mut b, t, 7000, 10_000) }
        let pieces = piece_states(&b, t);
        assert_eq!(pieces.len(), if fast_path { 2 } else { 1 });
        assert!(pieces.iter().all(|s| *s == FixedState::UserFixed), "{pieces:?}");
    }
}

#[test]
fn user_fixed_pieces_are_unfixed_without_the_flag() {
    for fast_path in [true, false] {
        let mut b = board();
        let t = fixed_trace(&mut b, FixedState::UserFixed);
        if fast_path { cut(&mut b, t, 4000, 5000) } else { cut(&mut b, t, 7000, 10_000) }
        let pieces = piece_states(&b, t);
        assert!(!pieces.is_empty());
        assert!(pieces.iter().all(|s| *s == FixedState::Unfixed), "{pieces:?}");
    }
}

#[test]
fn shove_fixed_parent_is_replaced_by_pieces_that_keep_the_state() {
    for (flag, expected) in [(true, FixedState::ShoveFixed), (false, FixedState::Unfixed)] {
        for fast_path in [true, false] {
            let mut b = board();
            b.keep_fixed_on_split = flag;
            let t = fixed_trace(&mut b, FixedState::ShoveFixed);
            if fast_path { cut(&mut b, t, 4000, 5000) } else { cut(&mut b, t, 7000, 10_000) }
            assert!(!b.item(t).is_on_board(), "the parent is replaced");
            let pieces = piece_states(&b, t);
            assert_eq!(pieces.len(), if fast_path { 2 } else { 1 });
            assert!(pieces.iter().all(|s| *s == expected), "flag={flag} fast={fast_path}: {pieces:?}");
        }
    }
}

#[test]
fn unfixed_parent_stays_unfixed_with_the_flag() {
    let mut b = board();
    b.keep_fixed_on_split = true;
    let t = fixed_trace(&mut b, FixedState::Unfixed);
    cut(&mut b, t, 4000, 5000);
    assert_eq!(piece_states(&b, t), vec![FixedState::Unfixed; 2]);
}
