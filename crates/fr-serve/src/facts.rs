//! Read-only facts about a board in protocol terms: counts, per-net rows, unrouted connections.
//! Everything is ordered deterministically (SPEC 5: net order = net numbers = DSN network order).

use fr_engine::board::{BasicBoard, ItemKey, ItemKind};
use fr_engine::drc::{all_clearance_violations, DesignRulesChecker};
use fr_engine::ids::FixedState;
use serde_json::{json, Value};

use crate::ops::lock::LockRegistry;

/// Trace segments of the wiring (a polyline trace with n corners has n - 1 segments).
fn segments(board: &BasicBoard, key: ItemKey) -> i64 {
    board.item(key).as_trace().map_or(0, |t| (t.corner_count() as i64 - 1).max(0))
}

/// Counts of the wiring on the board.
#[derive(Clone, Copy, Debug, Default)]
pub struct Wiring {
    pub wires: i64,
    pub vias: i64,
    /// Fixed wiring items (trace segments and vias that are not `Unfixed`).
    pub fixed: i64,
    pub length: i64,
}

pub fn wiring(board: &BasicBoard) -> Wiring {
    let mut w = Wiring::default();
    let mut length = 0.0;
    for key in board.get_items() {
        let item = board.item(key);
        let n = if item.is_trace() {
            let s = segments(board, key);
            w.wires += s;
            length += item.as_trace().map_or(0.0, |t| t.length());
            s
        } else if item.is_via() {
            w.vias += 1;
            1
        } else {
            continue;
        };
        if item.fixed_state() != FixedState::Unfixed {
            w.fixed += n;
        }
    }
    w.length = length.round() as i64;
    w
}

/// `[x1, y1, x2, y2]` of the board outline items (the whole board when there is no outline item).
pub fn boundary(board: &BasicBoard) -> [i64; 4] {
    let mut b = None;
    for key in board.get_items() {
        let item = board.item(key);
        if item.is_board_outline() {
            let bb = item.bounding_box(board);
            b = Some(b.map_or(bb, |u: fr_geom::IntBox| u.union_int_box(&bb)));
        }
    }
    let b = b.unwrap_or_else(|| board.bounding_box());
    [b.ll.x as i64, b.ll.y as i64, b.ur.x as i64, b.ur.y as i64]
}

/// `REF-PIN` for a pin, else `kind@x,y`.
pub(crate) fn label(board: &BasicBoard, key: ItemKey, at: (i64, i64)) -> String {
    let item = board.item(key);
    if let ItemKind::Pin(p) = &item.kind {
        if item.component_no() > 0 {
            let comp = board.components.get(item.component_no());
            let pkg = board.library.packages.get(comp.get_package());
            if let Some(pin) = pkg.get_pin(p.pin_index) {
                return format!("{}-{}", comp.name, pin.name);
            }
        }
    }
    let kind = match &item.kind {
        ItemKind::Via(_) => "via",
        ItemKind::Trace(_) => "wire",
        ItemKind::ConductionArea(_) => "plane",
        _ => "item",
    };
    format!("{kind}@{},{}", at.0, at.1)
}

/// One unrouted connection with its ends as pin names.
#[derive(Clone, Debug)]
pub struct Unrouted {
    pub net_no: i32,
    pub net: String,
    pub from: String,
    pub to: String,
    pub from_xy: [i64; 2],
    pub to_xy: [i64; 2],
    /// The board items at the ends (`from`/`to` may be swapped relative to the airline: they follow the names).
    pub from_item: ItemKey,
    pub to_item: ItemKey,
}

impl Unrouted {
    pub fn to_json(&self) -> Value {
        json!({ "net": self.net, "from": self.from, "to": self.to, "from_xy": self.from_xy, "to_xy": self.to_xy })
    }
}

/// The board's unrouted connections and the number of connections the nets need, sorted by net
/// order, then `from`, then `to` (the ends of one connection are ordered by name).
pub fn connections(board: &BasicBoard) -> (i64, Vec<Unrouted>) {
    let mut drc = DesignRulesChecker::new();
    let airlines = drc.get_all_airlines(board);
    let mut out: Vec<Unrouted> = airlines
        .iter()
        .map(|a| {
            let fxy = (a.from_corner.x.round() as i64, a.from_corner.y.round() as i64);
            let txy = (a.to_corner.x.round() as i64, a.to_corner.y.round() as i64);
            let (f, t) = (label(board, a.from_item, fxy), label(board, a.to_item, txy));
            let net = board.rules.nets.get(a.net_number).map(|n| n.name.clone()).unwrap_or_default();
            let swap = f > t;
            let (from, to, from_xy, to_xy, from_item, to_item) = if swap {
                (t, f, [txy.0, txy.1], [fxy.0, fxy.1], a.to_item, a.from_item)
            } else {
                (f, t, [fxy.0, fxy.1], [txy.0, txy.1], a.from_item, a.to_item)
            };
            Unrouted { net_no: a.net_number, net, from, to, from_xy, to_xy, from_item, to_item }
        })
        .collect();
    out.sort_by(|a, b| (a.net_no, &a.from, &a.to).cmp(&(b.net_no, &b.from, &b.to)));
    (drc.max_connections as i64, out)
}

pub fn violations(board: &BasicBoard) -> i64 {
    all_clearance_violations(board).len() as i64
}

/// One row per net with at least two pins (or planes), in net order.
pub fn net_rows(board: &BasicBoard, unrouted: &[Unrouted], locks: &LockRegistry) -> Vec<Value> {
    let max = board.rules.nets.max_net_number().max(0) as usize;
    let mut endpoints = vec![0i64; max + 1];
    let mut wires = vec![0i64; max + 1];
    let mut vias = vec![0i64; max + 1];
    let mut length = vec![0f64; max + 1];
    for key in board.get_items() {
        let item = board.item(key);
        let Some(&n) = item.net_numbers().first() else { continue };
        if n < 1 || n as usize > max {
            continue;
        }
        let n = n as usize;
        if item.is_pin() || item.is_conduction_area() {
            endpoints[n] += 1;
        } else if item.is_trace() {
            wires[n] += segments(board, key);
            length[n] += item.as_trace().map_or(0.0, |t| t.length());
        } else if item.is_via() {
            vias[n] += 1;
        }
    }
    let mut rows = Vec::new();
    for n in 1..=max {
        let Some(net) = board.rules.nets.get(n as i32) else { continue };
        if endpoints[n] < 2 {
            continue;
        }
        let open = unrouted.iter().filter(|u| u.net_no == n as i32).count() as i64;
        let status = if locks.is_locked_net(&net.name) {
            "locked"
        } else if open == 0 {
            "routed"
        } else if wires[n] + vias[n] == 0 {
            "unrouted"
        } else {
            "partial"
        };
        rows.push(json!({
            "name": net.name, "status": status, "unrouted": open,
            "wires": wires[n], "vias": vias[n], "length": length[n].round() as i64,
        }));
    }
    rows
}
