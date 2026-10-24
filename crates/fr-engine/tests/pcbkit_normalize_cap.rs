//! pcbkit pair budget: `normalize_traces` ends at a stop request and leaves a net of more than
//! `MAX_NORMALIZE_TRACES` traces alone.
//!
//! The pair attempt of rp2040env energy-15 never answered its 840 s budget: the rip-up passes with the pairs held
//! fragmented the VDD_1V1 net into 300 to 950 traces, and one `normalize_traces` round over them took 3 to 330 s
//! (cubic in the piece count) with no stop check. The board here is a bare two-layer board; net "a" has two collinear
//! traces that touch end to end, which one normalization joins into one.

use fr_engine::board::*;
use fr_engine::datastructures::{ItemIdGenerator, StopToken};
use fr_engine::ids::FixedState;
use fr_engine::library::{BoardLibrary, Packages, Padstacks};
use fr_engine::rules::{BoardRules, ClearanceMatrix};
use fr_engine::structure::{Communication, Components, CoordinateTransform, Layer, LayerStructure, SpecctraParserInfo, Unit};
use fr_geom::*;

const NET_A: i32 = 1;

fn board() -> BasicBoard {
    let ls = LayerStructure::new(vec![Layer::new("F.Cu", true), Layer::new("B.Cu", true)]);
    let matrix = ClearanceMatrix::get_default_instance(&ls, 200);
    let mut rules = BoardRules::new(ls.clone(), matrix);
    let class = rules.net_classes.append("default", &ls, false);
    rules.nets.add("a", 1, false, class);
    let padstacks = Padstacks::new(&ls);
    let comm = Communication::new(Unit::Mil, 1, Some(SpecctraParserInfo::default()), CoordinateTransform::new(1.0, 0.0, 0.0), ItemIdGenerator::new());
    let outline = PolylineShape::Tile(TileShape::IntBox(IntBox::new(0, 0, 1_000_000, 1_000_000)));
    BasicBoard::new(
        IntBox::new(-1000, -1000, 1_001_000, 1_001_000),
        ls,
        vec![outline],
        1,
        rules,
        BoardLibrary::new(padstacks, Packages::new()),
        Components::new(),
        comm,
    )
}

fn segment(b: &mut BasicBoard, layer: i32, x0: i32, x1: i32, y: i32) {
    let p = Polyline::from_int_points(&[IntPoint::new(x0, y), IntPoint::new(x1, y)]);
    b.insert_trace_without_cleaning(p, layer, 100, &[NET_A], 1, FixedState::Unfixed).expect("trace");
}

/// Two traces that join, plus `strays` short traces far from them (never joined: gaps between them).
fn board_with(strays: usize) -> BasicBoard {
    let mut b = board();
    segment(&mut b, 0, 1000, 5000, 1000);
    segment(&mut b, 0, 5000, 9000, 1000);
    for i in 0..strays as i32 {
        let (layer, y) = (i % 2, 20_000 + (i / 2) * 1000);
        segment(&mut b, layer, 100_000, 100_500, y);
    }
    b
}

fn traces(b: &BasicBoard) -> usize {
    b.get_items().iter().filter(|k| b.item(**k).as_trace().is_some() && b.item(**k).is_on_board()).count()
}

#[test]
fn joins_collinear_traces_of_a_small_net() {
    let mut b = board_with(5);
    let before = traces(&b);
    assert!(b.normalize_traces(NET_A));
    assert_eq!(traces(&b), before - 1, "the two touching collinear traces became one");
}

#[test]
fn leaves_a_net_of_more_than_the_cap_alone() {
    let mut b = board_with(MAX_NORMALIZE_TRACES);
    let before = traces(&b);
    assert!(before > MAX_NORMALIZE_TRACES);
    assert!(!b.normalize_traces(NET_A), "no normalization over {before} traces");
    assert_eq!(traces(&b), before);
}

#[test]
fn a_net_at_the_cap_is_still_normalized() {
    let mut b = board_with(MAX_NORMALIZE_TRACES - 2);
    let before = traces(&b);
    assert_eq!(before, MAX_NORMALIZE_TRACES);
    assert!(b.normalize_traces(NET_A));
    assert_eq!(traces(&b), before - 1);
}

#[test]
fn a_requested_stop_ends_the_normalization_before_it_changes_anything() {
    let mut b = board_with(5);
    let before = traces(&b);
    let stop = StopToken::new();
    stop.request_stop();
    assert!(!b.normalize_traces_stoppable(NET_A, Some(&stop)));
    assert_eq!(traces(&b), before);
}

#[test]
fn without_a_stop_request_the_stoppable_form_is_the_plain_one() {
    let mut b = board_with(5);
    let before = traces(&b);
    let stop = StopToken::new();
    assert!(b.normalize_traces_stoppable(NET_A, Some(&stop)));
    assert_eq!(traces(&b), before - 1);
}
