//! pcbkit hook H3: blocker items out of the maze search. Default off (empty); on, a connection
//! walled in by a fixed GUARD wire reports that wire.

use fr_engine::board::{ItemKey, RoutingBoard, TimeLimitPolicy};
use fr_engine::pipeline::BatchAutorouter;

fn load(dsn: &[u8]) -> (RoutingBoard, fr_settings::RouterSettings) {
    let mut settings = fr_settings::default_settings(1);
    settings.optimizer.enabled = Some(false);
    let mut board = fr_io::post_load::load_from_specctra_dsn(dsn, &mut settings).expect("load");
    fr_io::post_load::prepare_for_routing(&mut board, &mut settings, None);
    board.time_limits = TimeLimitPolicy::disabled();
    (board, settings)
}

fn net_no(board: &RoutingBoard, name: &str) -> i32 {
    (1..=board.rules.nets.max_net_number()).find(|&n| board.rules.nets.get(n).is_some_and(|x| x.name == name)).expect("net")
}

fn pins_of(board: &RoutingBoard, net: i32) -> Vec<ItemKey> {
    board.get_items().into_iter().filter(|&k| board.item(k).is_pin() && board.item(k).net_number(0) == net).collect()
}

#[test]
fn blocked_connection_names_guard_wire() {
    let (board, settings) = load(include_bytes!("data/blocked.dsn"));
    let pins = pins_of(&board, net_no(&board, "N4"));
    assert_eq!(pins.len(), 2);
    let (routed, blockers, added) = BatchAutorouter::route_connection_alone(&board, pins[0], pins[1], &settings);
    assert!(!routed, "N4 is walled in");
    assert!(added.is_empty());
    let guard = net_no(&board, "GUARD");
    assert!(
        blockers.iter().any(|&k| board.item(k).is_trace() && board.item(k).net_number(0) == guard),
        "blockers must include the GUARD wire: {} items",
        blockers.len()
    );
    let mut dedup = blockers.clone();
    dedup.dedup();
    assert_eq!(dedup.len(), blockers.len());
}

#[test]
fn routable_connection_has_no_blockers() {
    let (board, settings) = load(include_bytes!("data/blocked.dsn"));
    let pins = pins_of(&board, net_no(&board, "N1"));
    assert_eq!(pins.len(), 2);
    let (routed, blockers, added) = BatchAutorouter::route_connection_alone(&board, pins[0], pins[1], &settings);
    assert!(routed);
    assert!(!added.is_empty());
    assert!(blockers.is_empty());
}
