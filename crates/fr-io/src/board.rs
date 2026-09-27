//! The board step: replays a [`LoadedDesign`] on a `fr_engine::board::BasicBoard`
//! (the rest of Java `DsnReader.readBoard` after parsing), following the contract in the
//! crate docs.

use fr_engine::board::{BasicBoard, ItemKey, ItemSelectionFilter, ObstacleKind, SelectableChoices};
use fr_engine::ids::FixedState;
use fr_geom::Point;

use crate::loader::LoadedDesign;
use crate::requests::{AreaRequest, InsertRequest};

/// Java `Wiring.viaExists`.
fn via_exists(board: &BasicBoard, center: &Point, padstack: i32, nets: &[i32]) -> bool {
    let (from, to) = {
        let p = board.library.padstacks.get(padstack).expect("via padstack");
        (p.from_layer(), p.to_layer())
    };
    let filter = ItemSelectionFilter::single(SelectableChoices::Vias);
    let picked = board.pick_items(center, from, Some(&filter));
    let exists = picked.iter().any(|k| {
        let v = board.item(k);
        v.nets_equal_nos(nets)
            && v.center(board) == *center
            && v.first_layer(board) == from
            && v.last_layer(board) == to
    });
    exists
}

/// Java `Wiring.tryCorrectNet`.
fn try_correct_net(board: &mut BasicBoard, key: ItemKey) {
    let (first, last) = {
        let t = board.item(key);
        (t.first_corner(), t.last_corner())
    };
    let mut contacts = board.trace_normal_contacts_at(key, &first, true);
    contacts.extend_from(&board.trace_normal_contacts_at(key, &last, true));
    let corrected = contacts
        .iter()
        .map(|k| board.item(k))
        .find(|it| it.net_count() == 1)
        .map(|it| it.net_number(0))
        .unwrap_or(0);
    if corrected != 0 {
        board.assign_net_no(key, corrected);
    }
}

fn insert_area(board: &mut BasicBoard, kind: ObstacleKind, a: AreaRequest) {
    board.insert_obstacle_area(
        kind,
        a.area,
        a.layer,
        a.translation,
        a.rotation_in_degree,
        a.side_changed,
        a.clearance_class,
        a.component_id,
        a.name,
        a.fixed,
    );
}

/// Builds the board Java's `DsnReader.readBoard` returns: creates the board (which inserts the
/// outline), replays the requests (duplicate via check, `tryCorrectNet`), runs
/// `normalizeAllTraces` (a panic there is caught, like the Java exception) and applies the
/// plane adjustment. `logical_part_assignments` are not applied (fr-engine's `Components` has
/// no mutable per-component accessor); they only matter for pin/gate swap.
pub fn build_board(design: LoadedDesign) -> BasicBoard {
    let mut requests = design.requests.into_iter();
    let first = requests.next();
    assert!(
        matches!(first, Some(InsertRequest::Outline { .. })),
        "the first request is the board outline"
    );
    let mut board = BasicBoard::new(
        design.bounding_box,
        design.layer_structure,
        design.outline_shapes,
        design.outline_clearance_class,
        design.rules,
        design.library,
        design.components,
        design.communication,
    );
    let mut request_keys: Vec<Option<ItemKey>> = vec![None];
    for r in requests {
        let key = match r {
            InsertRequest::Outline { .. } => unreachable!("only one outline"),
            InsertRequest::Obstacle(a) => {
                insert_area(&mut board, ObstacleKind::Keepout, a);
                None
            }
            InsertRequest::ViaObstacle(a) => {
                insert_area(&mut board, ObstacleKind::ViaKeepout, a);
                None
            }
            InsertRequest::ComponentObstacle(a) => {
                insert_area(&mut board, ObstacleKind::ComponentKeepout, a);
                None
            }
            InsertRequest::ComponentOutline {
                area,
                is_front,
                translation,
                rotation_in_degree,
                component_id,
                is_courtyard,
                is_fabrication,
                is_closed,
                fixed,
            } => {
                board.insert_component_outline(
                    area,
                    is_front,
                    translation,
                    rotation_in_degree,
                    component_id,
                    is_courtyard,
                    is_fabrication,
                    is_closed,
                    fixed,
                );
                None
            }
            InsertRequest::ConductionArea {
                area,
                layer,
                nets,
                clearance_class,
                is_obstacle,
                fixed,
            } => Some(board.insert_conduction_area(
                area,
                layer,
                &nets,
                clearance_class,
                is_obstacle,
                fixed,
            )),
            InsertRequest::Pin {
                component_id,
                pin_index,
                nets,
                clearance_class,
                fixed,
            } => {
                board.insert_pin(component_id, pin_index, &nets, clearance_class, fixed);
                None
            }
            InsertRequest::Trace {
                polyline,
                layer,
                half_width,
                nets,
                clearance_class,
                fixed,
                try_correct_net: correct,
            } => {
                let key = board.insert_trace_without_cleaning(
                    polyline,
                    layer,
                    half_width,
                    &nets,
                    clearance_class,
                    fixed,
                );
                if let Some(k) = key {
                    if correct && board.item(k).net_count() == 0 {
                        try_correct_net(&mut board, k);
                    }
                }
                None
            }
            InsertRequest::Via {
                padstack,
                center,
                nets,
                clearance_class,
                fixed,
                attach_allowed,
                ..
            } => {
                let center = Point::Int(center);
                if via_exists(&board, &center, padstack, &nets) {
                    log::warn!("Wiring: duplicate via skipped");
                } else {
                    board.insert_via(
                        padstack,
                        center,
                        &nets,
                        clearance_class,
                        fixed,
                        attach_allowed,
                    );
                }
                None
            }
        };
        request_keys.push(key);
    }
    // Wiring.readScope: board.normalizeAllTraces() (exceptions are caught by Java).
    let normalized = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        board.normalize_all_traces();
    }));
    if normalized.is_err() {
        log::debug!("Wiring: normalization of traces failed");
    }
    // DsnReader: adjustPlaneAutorouteSettings when there is no autoroute_settings scope.
    if let Some(adj) = &design.plane_adjustment {
        adj.apply_to_rules(board.rules_mut());
        for &i in &adj.fix_requests {
            if let Some(k) = request_keys[i] {
                let item = board.item_mut(k);
                if item.fixed_state() < FixedState::UserFixed {
                    item.set_fixed_state(FixedState::UserFixed);
                }
            }
        }
    }
    board
}
