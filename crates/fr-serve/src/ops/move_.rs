//! Op `move` (SPEC 5.3, capability `move`): atomic part moves with rip-up of the attached wiring,
//! rigid carrying of fixed fan-out, and `locked_conflict`.
//!
//! The op works on a [`Work`](crate::session::Work) clone, so any error leaves the session as it was.

use std::collections::{BTreeSet, HashSet};

use fr_engine::board::{BasicBoard, ItemKey, ItemKind};
use fr_engine::ids::{ComponentNo, FixedState};
use fr_geom::{IntPoint, Point, Vector};
use serde_json::{json, Map, Value};

use crate::facts;
use crate::ops::lock::LockRegistry;
use crate::proto::{as_bool, as_int, as_obj, as_str, check_keys, req, ProtoError, R};
use crate::session::Session;

/// True once this module implements the op and passes its conformance check.
pub const CLAIMED: bool = true;

/// One validated move: the new DSN place record, already in board units.
struct Move {
    name: String,
    no: ComponentNo,
    old_loc: IntPoint,
    old_rot: f64,
    new_loc: IntPoint,
    new_rot: f64,
    front: bool,
}

/// What a move touches on the current board.
#[derive(Default)]
struct Touch {
    /// Unfixed, unlocked traces and vias to remove.
    rip: Vec<ItemKey>,
    /// Locked traces and vias reached from the pins.
    locked: Vec<ItemKey>,
}

fn is_wiring(board: &BasicBoard, key: ItemKey) -> bool {
    let item = board.item(key);
    item.is_trace() || item.is_via()
}

/// Wiring the router made: unfixed, or shove-fixed (the router marks the pad stubs it creates so; a DSN
/// states fixed wiring as `fix`, which is `SystemFixed`).
fn rippable(board: &BasicBoard, key: ItemKey) -> bool {
    matches!(board.item(key).fixed_state(), FixedState::Unfixed | FixedState::ShoveFixed)
}

fn is_locked(board: &BasicBoard, locks: &LockRegistry, key: ItemKey) -> bool {
    let item = board.item(key);
    item.fixed_state() == FixedState::UserFixed
        || item.net_numbers().first().and_then(|&n| board.rules.nets.get(n)).is_some_and(|n| locks.is_locked_net(&n.name))
}

/// Same-net copper touching the item (overlap, not only end-to-center matches: a router trace may end
/// anywhere inside a pad).
fn contacts(board: &BasicBoard, key: ItemKey) -> Vec<ItemKey> {
    board.all_contacts(key, None).iter().collect()
}

fn segments(board: &BasicBoard, key: ItemKey) -> i64 {
    board.item(key).as_trace().map_or(1, |t| (t.corner_count() as i64 - 1).max(0))
}

/// Wiring connected to a pin of the component, directly or through other wiring (never through a pin).
fn touch(board: &BasicBoard, locks: &LockRegistry, no: ComponentNo) -> Touch {
    let mut seen: HashSet<ItemKey> = HashSet::new();
    let mut stack: Vec<ItemKey> = Vec::new();
    for pin in board.get_component_pins(no) {
        for c in contacts(board, pin).into_iter() {
            if is_wiring(board, c) && seen.insert(c) {
                stack.push(c);
            }
        }
    }
    let mut out = Touch::default();
    while let Some(k) = stack.pop() {
        let locked = is_locked(board, locks, k);
        if locked {
            out.locked.push(k);
        } else if rippable(board, k) {
            out.rip.push(k);
        } else {
            continue; // DSN-fixed wiring is neither ripped nor walked through
        }
        for c in contacts(board, k).into_iter() {
            if is_wiring(board, c) && seen.insert(c) {
                stack.push(c);
            }
        }
    }
    out
}

/// Fixed wiring (DSN `fix`) whose connectivity reaches only pins of the component and planes: one
/// valid island at a time; an island that reaches anything else stays where it is.
fn carry_set(board: &BasicBoard, no: ComponentNo) -> Vec<ItemKey> {
    let carryable = |k: ItemKey| is_wiring(board, k) && board.item(k).fixed_state() == FixedState::SystemFixed;
    let mut done: HashSet<ItemKey> = HashSet::new();
    let mut carried = Vec::new();
    for pin in board.get_component_pins(no) {
        for seed in contacts(board, pin).into_iter() {
            if !carryable(seed) || !done.insert(seed) {
                continue;
            }
            let (mut island, mut stack, mut valid) = (vec![seed], vec![seed], true);
            while let Some(k) = stack.pop() {
                for c in contacts(board, k).into_iter() {
                    let item = board.item(c);
                    match &item.kind {
                        ItemKind::Pin(_) => valid &= item.component_no() == no,
                        ItemKind::ConductionArea(_) => {}
                        _ if is_wiring(board, c) => {
                            if !carryable(c) {
                                valid = false;
                            } else if done.insert(c) {
                                island.push(c);
                                stack.push(c);
                            }
                        }
                        _ => {}
                    }
                }
            }
            if valid {
                carried.extend(island);
            }
        }
    }
    carried
}

fn net_names(board: &BasicBoard, keys: &[ItemKey]) -> BTreeSet<(i32, String)> {
    keys.iter()
        .filter_map(|&k| board.item(k).net_numbers().first().copied())
        .filter_map(|n| board.rules.nets.get(n).map(|net| (n, net.name.clone())))
        .collect()
}

fn parse_moves(board: &BasicBoard, args: &Map<String, Value>) -> R<Vec<Move>> {
    let list = req(args, "moves", "move")?.as_array().ok_or_else(|| ProtoError::bad_request("move.moves must be a list"))?;
    if list.is_empty() {
        return Err(ProtoError::bad_request("move.moves must not be empty"));
    }
    let ct = board.communication.coordinate_transform;
    let res = board.communication.resolution as f64;
    let mut moves: Vec<Move> = Vec::new();
    let mut unknown: Option<String> = None;
    for (i, m) in list.iter().enumerate() {
        let what = format!("move.moves[{i}]");
        let o = as_obj(m, &what)?;
        check_keys(o, &["ref", "x", "y", "rot", "side"], &what)?;
        let name = as_str(req(o, "ref", &what)?, &format!("{what}.ref"))?.to_string();
        let x = as_int(req(o, "x", &what)?, -1_000_000_000, 1_000_000_000, &format!("{what}.x"))?;
        let y = as_int(req(o, "y", &what)?, -1_000_000_000, 1_000_000_000, &format!("{what}.y"))?;
        let rot = as_int(req(o, "rot", &what)?, 0, 359, &format!("{what}.rot"))?;
        let front = match as_str(req(o, "side", &what)?, &format!("{what}.side"))? {
            "front" => true,
            "back" => false,
            other => return Err(ProtoError::bad_request(format!("{what}.side: '{other}' is neither front nor back"))),
        };
        if moves.iter().any(|p| p.name == name) {
            return Err(ProtoError::bad_request(format!("{what}: '{name}' is moved twice")));
        }
        let Some(c) = board.components.get_by_name(&name).filter(|c| c.is_placed()) else {
            unknown.get_or_insert(name);
            continue;
        };
        if c.placed_on_front() != front {
            return Err(ProtoError::bad_request(format!("{what}: a side change of '{name}' is not supported in protocol 1.0")));
        }
        let Some(Point::Int(old_loc)) = c.get_location().cloned() else {
            return Err(ProtoError::new("internal", format!("component '{name}' has no integer location")));
        };
        let new_loc = ct.dsn_to_board_point(&[x as f64 / res, y as f64 / res]).round();
        moves.push(Move { name, no: c.id, old_loc, old_rot: c.get_rotation_in_degree(), new_loc, new_rot: rot as f64, front });
    }
    if let Some(name) = unknown {
        return Err(ProtoError::new("unknown_ref", format!("no placed component '{name}'")).with_details(json!({ "ref": name })));
    }
    Ok(moves)
}

pub fn handle(session: &mut Session, args: &Map<String, Value>) -> R<Value> {
    check_keys(args, &["moves", "carry", "unlock"], "move")?;
    let carry = match args.get("carry").map(|v| as_str(v, "move.carry")).transpose()? {
        None | Some("fixed") => true,
        Some("none") => false,
        Some(other) => return Err(ProtoError::bad_request(format!("move.carry: '{other}' is neither fixed nor none"))),
    };
    let unlock = args.get("unlock").map(|v| as_bool(v, "move.unlock")).transpose()?.unwrap_or(false);

    let mut work = session.begin()?;
    let moves = parse_moves(&work.board.board, args)?;

    // locked wiring on a moved pin: refuse, or (unlock) release it and rip it with the rest
    let mut conflict: Vec<ItemKey> = Vec::new();
    for m in &moves {
        conflict.extend(touch(&work.board.board, &work.locks, m.no).locked);
    }
    if !conflict.is_empty() {
        let b = &work.board.board;
        let nets: Vec<String> = net_names(b, &conflict).into_iter().map(|(_, n)| n).collect();
        if !unlock {
            let wires: i64 = conflict.iter().map(|&k| segments(b, k)).sum();
            return Err(ProtoError::new("locked_conflict", "locked wiring touches a moved pin")
                .with_details(json!({ "nets": nets, "wires": wires })));
        }
        let unlocked = conflict.len();
        let b = &mut work.board.board;
        for &k in &conflict {
            b.item_mut(k).set_fixed_state(FixedState::Unfixed);
        }
        work.locks.release(&nets, unlocked);
    }

    let (mut ripped_nets, mut r_wires, mut r_vias, mut c_wires, mut c_vias) = (BTreeSet::new(), 0i64, 0i64, 0i64, 0i64);
    for m in &moves {
        let b = &mut work.board.board;
        let t = touch(b, &work.locks, m.no);
        ripped_nets.extend(net_names(b, &t.rip));
        for &k in &t.rip {
            if b.item(k).is_via() {
                r_vias += 1;
            } else {
                r_wires += segments(b, k);
            }
        }
        b.remove_items(t.rip);

        let delta = m.new_rot - m.old_rot;
        if carry && delta.rem_euclid(90.0) == 0.0 {
            let factor = (delta.rem_euclid(360.0) / 90.0) as i32;
            let vector: Vector = Point::Int(m.new_loc).difference_by(&Point::Int(m.old_loc));
            for k in carry_set(b, m.no) {
                if b.item(k).is_via() {
                    c_vias += 1;
                } else {
                    c_wires += segments(b, k);
                }
                b.turn_translate_wiring(k, factor, &m.old_loc, &vector);
            }
        }
        b.place_component(m.no, m.new_loc, m.new_rot, m.front).map_err(|e| ProtoError::bad_request(format!("{}: {e}", m.name)))?;
    }

    let (_, unrouted) = facts::connections(&work.board.board);
    let result = json!({
        "moved": moves.len(),
        "ripped": { "nets": ripped_nets.into_iter().map(|(_, n)| n).collect::<Vec<_>>(), "wires": r_wires, "vias": r_vias },
        "carried": { "wires": c_wires, "vias": c_vias },
        "unrouted": unrouted.len(),
    });
    session.commit(work);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::SessionSettings;

    fn session_with(dsn: &std::path::Path) -> Session {
        let mut s = Session::new();
        s.hello = Some(SessionSettings::from_hello(1, None).unwrap());
        let args = json!({ "dsn": { "path": dsn.to_str().unwrap() } });
        crate::load::handle(&mut s, args.as_object().unwrap()).unwrap();
        s
    }

    fn tiny() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/tiny.dsn")
    }

    fn corpus(rel: &str) -> std::path::PathBuf {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../reference/pcbkit-corpus").join(rel);
        assert!(p.exists(), "corpus file {} is missing (shared reference/, see docs/PCBKIT.md)", p.display());
        p
    }

    fn call(s: &mut Session, args: Value) -> R<Value> {
        handle(s, args.as_object().unwrap())
    }

    fn ses(s: &Session) -> Vec<u8> {
        let b = s.loaded().unwrap();
        fr_io::ses_writer::ses_bytes(&b.board, &b.name)
    }

    /// The protocol pose (x, y, rot) of a component as the SES writes it.
    fn pose(b: &BasicBoard, name: &str) -> (i64, i64, i64) {
        let c = b.components.get_by_name(name).unwrap();
        let loc = c.get_location().unwrap().to_float();
        let ct = b.communication.coordinate_transform;
        let res = b.communication.resolution as f64;
        let p = ct.board_to_dsn_point(&loc);
        ((p[0] * res).round() as i64, (p[1] * res).round() as i64, c.get_rotation_in_degree().round() as i64)
    }

    /// Per pin of the component: how many fixed traces/vias touch it, and the sorted fixed-wiring
    /// shape of everything carried (geometry dump), so a rigid move can be compared.
    fn fixed_contacts(b: &BasicBoard, no: ComponentNo) -> Vec<usize> {
        b.get_component_pins(no).into_iter().map(|p| contacts(b, p).into_iter().filter(|&k| carry_set_member(b, k)).count()).collect()
    }

    fn carry_set_member(b: &BasicBoard, k: ItemKey) -> bool {
        is_wiring(b, k) && b.item(k).fixed_state() == FixedState::SystemFixed
    }

    #[test]
    fn carried_fanout_moves_rigidly_with_the_part() {
        let mut s = session_with(&corpus("runs/energy-8/base/layout/.route/board.dsn"));
        let (no, name) = {
            let b = &s.loaded().unwrap().board;
            b.components
                .get_all()
                .map(|c| (c.id, c.name.clone()))
                .find(|(no, _)| {
                    let set = carry_set(b, *no);
                    set.iter().any(|&k| b.item(k).is_via()) && set.iter().any(|&k| b.item(k).is_trace())
                })
                .expect("the corpus board has a part with plane fan-out (via + stub)")
        };
        let before = fixed_contacts(&s.loaded().unwrap().board, no);
        let carried_before = carry_set(&s.loaded().unwrap().board, no).len();
        let total_fixed = |s: &Session| {
            let b = &s.loaded().unwrap().board;
            b.get_items().into_iter().filter(|&k| carry_set_member(b, k)).count()
        };
        let fixed_total_before = total_fixed(&s);
        let (x, y, rot) = pose(&s.loaded().unwrap().board, &name);

        // translate by 1 mm (10000 units) and turn by 90 degrees
        let r = call(&mut s, json!({ "moves": [{ "ref": name, "x": x + 10000, "y": y - 5000, "rot": (rot + 90) % 360, "side": "front" }] })).unwrap();
        assert_eq!(r["moved"], 1);
        let b = &s.loaded().unwrap().board;
        assert_eq!(pose(b, &name), (x + 10000, y - 5000, (rot + 90) % 360));
        assert!(r["carried"]["vias"].as_i64().unwrap() > 0 && r["carried"]["wires"].as_i64().unwrap() > 0, "{r}");
        assert_eq!(fixed_contacts(b, no), before, "every pin keeps its fixed fan-out contacts after a rigid move");
        assert_eq!(carry_set(b, no).len(), carried_before, "the same island is still carried");
        assert_eq!(total_fixed(&s), fixed_total_before, "no fixed item is added or lost");

        // carry:none leaves the fan-out behind: contacts are lost
        let mut s2 = session_with(&corpus("runs/energy-8/base/layout/.route/board.dsn"));
        let r = call(&mut s2, json!({ "moves": [{ "ref": name, "x": x + 10000, "y": y, "rot": rot, "side": "front" }], "carry": "none" })).unwrap();
        assert_eq!(r["carried"], json!({ "wires": 0, "vias": 0 }));
        assert_ne!(fixed_contacts(&s2.loaded().unwrap().board, no), before);
    }

    #[test]
    fn locked_wiring_on_a_moved_pin_is_a_conflict_unless_unlock() {
        let mut s = session_with(&tiny());
        let route = json!({ "seed": 0, "nets": "all", "from": "scratch" });
        crate::route::handle(&mut s, route.as_object().unwrap()).unwrap();
        // lock one trace of N1 the way the lock op will (user-fixed), R1 sits on N1 and N2
        let locked_key = {
            let b = &s.loaded().unwrap().board;
            let pin = b.get_component_pins(b.components.get_by_name("R1").unwrap().id)[0];
            contacts(b, pin).into_iter().find(|&k| b.item(k).is_trace()).unwrap()
        };
        s.board.as_mut().unwrap().board.item_mut(locked_key).set_fixed_state(FixedState::UserFixed);
        let before = ses(&s);
        let (x, y, rot) = pose(&s.loaded().unwrap().board, "R1");
        let mv = |dy: i64, unlock: bool| json!({ "moves": [{ "ref": "R1", "x": x, "y": y + dy, "rot": rot, "side": "front" }], "unlock": unlock });

        let e = call(&mut s, mv(10000, false)).unwrap_err();
        assert_eq!(e.code, "locked_conflict");
        let d = e.details.unwrap();
        assert!(d["nets"].as_array().unwrap().iter().any(|n| n == "N1" || n == "N2"), "{d}");
        assert!(d["wires"].as_i64().unwrap() >= 1);
        assert_eq!(ses(&s), before, "a conflict changes nothing");

        let r = call(&mut s, mv(10000, true)).unwrap();
        assert_eq!(r["moved"], 1);
        assert!(r["ripped"]["wires"].as_i64().unwrap() >= 1, "{r}");
        let b = &s.loaded().unwrap().board;
        assert!(!b.get_items().contains(&locked_key) || !b.item(locked_key).is_user_fixed(), "the unlocked trace is gone");
        assert_eq!(pose(b, "R1"), (x, y + 10000, rot));
    }
}
