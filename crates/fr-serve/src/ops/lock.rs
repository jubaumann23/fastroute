//! Ops `lock` / `unlock` (capability `locking`) and the [`LockRegistry`] (SPEC 5.4).
//!
//! A trace or via is locked iff its `FixedState` is `UserFixed` and its id is in the registry. The
//! DSN's own `(type fix)` wiring stays `SystemFixed`, DSN `protect` wiring is remembered as the
//! `base` and never counts as locked. The board flag `keep_fixed_on_split` (hook H2b) is switched
//! on with the first lock, so the pieces of a split locked trace stay locked; the registry is
//! reconciled with the board after every route (`reconcile`), so pieces replace the id they came from.

use std::collections::BTreeSet;

use fr_engine::board::{BasicBoard, RoutingBoard};
use fr_engine::ids::{FixedState, ItemId};
use serde_json::{json, Map, Value};

use crate::proto::{as_bool, as_int, as_str, check_keys, ProtoError, R};
use crate::session::Session;

/// The `locking` capability is implemented and passes its conformance check.
pub const CLAIMED: bool = true;

/// Nets and wires frozen by `lock`.
#[derive(Clone, Debug, Default)]
pub struct LockRegistry {
    /// Locked nets in DSN network order.
    nets: Vec<String>,
    /// Ids of the locked traces and vias.
    wires: BTreeSet<i32>,
    /// `UserFixed` wiring that came from the DSN itself, never locked by us.
    base: BTreeSet<i32>,
}

fn is_wiring(board: &BasicBoard, key: fr_engine::board::ItemKey) -> bool {
    let it = board.item(key);
    it.is_trace() || it.is_via()
}

/// Net number of `name`, or `None`.
fn net_no(board: &BasicBoard, name: &str) -> Option<i32> {
    (1..=board.rules.nets.max_net_number()).find(|&n| board.rules.nets.get(n).is_some_and(|x| x.name == name))
}

fn net_name(board: &BasicBoard, n: i32) -> String {
    board.rules.nets.get(n).map(|x| x.name.clone()).unwrap_or_default()
}

impl LockRegistry {
    /// Locked nets in DSN network order.
    pub fn locked_nets(&self) -> &[String] {
        &self.nets
    }

    pub fn is_locked_net(&self, name: &str) -> bool {
        self.nets.iter().any(|n| n == name)
    }

    /// Locked wiring items.
    pub fn locked_wire_count(&self) -> usize {
        self.wires.len()
    }

    /// Ids of the `UserFixed` traces and vias on `board`.
    pub fn user_fixed_ids(board: &BasicBoard) -> BTreeSet<i32> {
        user_fixed_wiring(board)
    }

    /// Registry of a freshly loaded board: everything `UserFixed` now is the DSN's own, unless it is
    /// in `imported` (a session imported with `lock_initial`), which is locked.
    pub fn after_load(board: &mut RoutingBoard, own: &BTreeSet<i32>, lock_initial: bool) -> LockRegistry {
        let mut reg = LockRegistry { base: own.clone(), ..LockRegistry::default() };
        if lock_initial {
            board.keep_fixed_on_split = true;
        } else {
            reg.base = user_fixed_wiring(board);
        }
        reg.reconcile(board);
        reg
    }

    /// Re-derives the locked item ids from the board: after a route or optimize, pieces of a split
    /// locked trace replace the id they came from.
    pub fn reconcile(&mut self, board: &BasicBoard) {
        self.wires = user_fixed_wiring(board).difference(&self.base).copied().collect();
    }

    /// The route mask for this registry: `mask` (None = every net) minus the locked nets. `None` when
    /// nothing is excluded (so a board without locks is routed exactly like the stock CLI).
    pub fn route_mask(&self, board: &BasicBoard, mask: Option<Vec<bool>>) -> Option<Vec<bool>> {
        if self.nets.is_empty() {
            return mask;
        }
        let max = board.rules.nets.max_net_number().max(0) as usize;
        let mut m = mask.unwrap_or_else(|| vec![true; max + 1]);
        m.resize(max + 1, false);
        for name in &self.nets {
            if let Some(n) = net_no(board, name) {
                m[n as usize] = false;
            }
        }
        Some(m)
    }

    /// Locked wiring that touches a pin of `component_no`: the nets (DSN order) and wire ids (sorted).
    pub fn conflicts_with_pins(&self, board: &BasicBoard, component_no: i32) -> (Vec<String>, Vec<i64>) {
        let mut nets = BTreeSet::new();
        let mut ids = BTreeSet::new();
        for key in board.get_items() {
            let it = board.item(key);
            if !it.is_pin() || it.component_no() != component_no {
                continue;
            }
            for c in board.normal_contacts(key).iter() {
                let ci = board.item(c);
                if (ci.is_trace() || ci.is_via()) && self.wires.contains(&ci.id().0) {
                    ids.insert(ci.id().0 as i64);
                    nets.extend(ci.net_numbers().iter().copied());
                }
            }
        }
        (nets.into_iter().map(|n| net_name(board, n)).collect(), ids.into_iter().collect())
    }

    /// The board as `export` writes it: locked wiring is written like any routed wiring (SPEC 5.9), so
    /// the lock marker (`protect`) does not reach the session. `None` when nothing is locked.
    pub fn written_as_routed(&self, board: &RoutingBoard) -> Option<RoutingBoard> {
        if self.wires.is_empty() {
            return None;
        }
        let mut view = board.clone();
        for key in view.get_items() {
            if self.wires.contains(&view.item(key).id().0) {
                view.item_mut(key).set_fixed_state(FixedState::Unfixed);
            }
        }
        Some(view)
    }

    fn sort_nets(&mut self, board: &BasicBoard) {
        self.nets.sort_by_key(|n| net_no(board, n).unwrap_or(i32::MAX));
        self.nets.dedup();
    }
}

pub(crate) fn user_fixed_wiring(board: &BasicBoard) -> BTreeSet<i32> {
    board
        .get_items()
        .into_iter()
        .filter(|&k| is_wiring(board, k) && board.item(k).fixed_state() == FixedState::UserFixed)
        .map(|k| board.item(k).id().0)
        .collect()
}

fn names(args: &Map<String, Value>, key: &str) -> R<Vec<String>> {
    let Some(v) = args.get(key) else { return Ok(Vec::new()) };
    let list = v.as_array().ok_or_else(|| ProtoError::bad_request(format!("{key} must be a list")))?;
    list.iter().map(|x| as_str(x, &format!("{key} item")).map(str::to_string)).collect()
}

fn wire_ids(args: &Map<String, Value>) -> R<Vec<i32>> {
    let Some(v) = args.get("wires") else { return Ok(Vec::new()) };
    let list = v.as_array().ok_or_else(|| ProtoError::bad_request("wires must be a list"))?;
    list.iter().map(|x| as_int(x, 0, i32::MAX as i64, "wires item").map(|i| i as i32)).collect()
}

/// Wiring items of net `n`, in board order.
fn net_wiring(board: &BasicBoard, n: i32) -> Vec<fr_engine::board::ItemKey> {
    board.get_items().into_iter().filter(|&k| is_wiring(board, k) && board.item(k).net_numbers().contains(&n)).collect()
}

pub fn handle(session: &mut Session, args: &Map<String, Value>, op: &str) -> R<Value> {
    let unlock = op == "unlock";
    check_keys(args, if unlock { &["nets", "wires", "all"] } else { &["nets", "wires"] }, op)?;
    let all = args.get("all").map(|v| as_bool(v, "unlock.all")).transpose()?.unwrap_or(false);
    let (nets, wires) = (names(args, "nets")?, wire_ids(args)?);
    // an explicit empty list is a query: the result is the current state
    if !args.contains_key("nets") && !args.contains_key("wires") && !all {
        return Err(ProtoError::bad_request(format!("{op}: give 'nets' and/or 'wires'{}", if unlock { " or all: true" } else { "" })));
    }
    let mut work = session.begin()?;
    let board = &mut work.board.board;
    let reg = &mut work.locks;
    let mut net_nos = Vec::new();
    for n in &nets {
        match net_no(board, n) {
            Some(no) => net_nos.push((n.clone(), no)),
            None => return Err(ProtoError::new("unknown_net", format!("no net '{n}'")).with_details(json!({ "net": n }))),
        }
    }
    let mut keys = Vec::new();
    for &id in &wires {
        match board.get_item(ItemId(id)).filter(|&k| is_wiring(board, k)) {
            Some(k) => keys.push(k),
            None => return Err(ProtoError::new("unknown_wire", format!("no wire {id}")).with_details(json!({ "wire": id }))),
        }
    }
    if unlock {
        let mut targets: Vec<_> = keys;
        if all {
            reg.nets.clear();
            targets.extend(board.get_items().into_iter().filter(|&k| is_wiring(board, k) && reg.wires.contains(&board.item(k).id().0)));
        }
        for (name, n) in &net_nos {
            reg.nets.retain(|x| x != name);
            targets.extend(net_wiring(board, *n).into_iter().filter(|&k| reg.wires.contains(&board.item(k).id().0)));
        }
        for k in targets {
            if reg.wires.contains(&board.item(k).id().0) {
                board.item_mut(k).set_fixed_state(FixedState::Unfixed);
            }
        }
    } else {
        board.keep_fixed_on_split = true;
        let mut targets = keys;
        for (name, n) in &net_nos {
            if !reg.nets.contains(name) {
                reg.nets.push(name.clone());
            }
            targets.extend(net_wiring(board, *n));
        }
        for k in targets {
            if board.item(k).fixed_state() < FixedState::UserFixed {
                board.item_mut(k).set_fixed_state(FixedState::UserFixed);
            }
        }
    }
    reg.sort_nets(board);
    reg.reconcile(board);
    let result = json!({ "nets": reg.nets, "wires": reg.wires.len() });
    session.commit(work);
    Ok(result)
}
