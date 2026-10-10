//! Op `check` (capability `check`), SPEC 5.12: a fast rule check of the CURRENT board against the DSN
//! rules, from the clearance engine and the connectivity the router already holds.
//!
//! Checked:
//! * clearance between copper items (the router's own `all_clearance_violations` with no tolerance: one
//!   entry per pair of items and layer), by pair kind: `track_track`, `track_pad`, `track_via`, `via_via`, `via_pad`,
//!   `pad_pad`, `edge` (the board outline, which carries the copper-to-edge clearance), `keepout`,
//!   `plane`, `other`;
//! * track width: `width_class` (a segment below its net class width on its layer, locked and router
//!   wiring; DSN `fix` wiring is the input and not judged) and `width_min` (below the minimum width: the
//!   request's `min_width`, else the session's `router.min_trace_width_um`);
//! * vias: `annular_ring` and `drill` against the request's `min_annular` / `min_drill`. The drill comes
//!   from the via padstack name (`Via[0-1]_600:300_um`, as the toolkit's DSN writer names it); a via
//!   padstack without a drill in its name is counted in `vias_unchecked` instead;
//! * `unconnected`: connections still open.
//!
//! NOT checked (KiCad DRC remains the authority for fab output): zone fill and thermal reliefs
//! (planes are conduction areas here, not filled copper), silkscreen, courtyards, solder mask, hole to
//! hole and hole to copper distance, the KiCad-only rule areas and custom rules, board-level `min_*`
//! constraints other than the ones above.

use std::collections::BTreeMap;
use std::time::Instant;

use fr_engine::board::{BasicBoard, Item};
use fr_engine::drc::all_clearance_violations;
use fr_engine::ids::FixedState;
use serde_json::{json, Map, Value};

use crate::facts;
use crate::proto::{as_int, check_keys, ProtoError, R};
use crate::session::Session;

/// True once this module implements the op and passes its conformance check.
pub const CLAIMED: bool = true;

/// Default and largest length of the item list.
const DEFAULT_MAX: i64 = 50;
const MAX_MAX: i64 = 10_000;

/// What the check does not look at, in every result (so a client cannot mistake it for KiCad DRC).
pub const NOT_CHECKED: [&str; 7] =
    ["zone_fill", "thermal_reliefs", "silk", "courtyards", "solder_mask", "hole_clearance", "custom_rules"];

/// The result types, in report order.
pub const TYPES: [&str; 15] = [
    "clearance_track_track",
    "clearance_track_pad",
    "clearance_track_via",
    "clearance_via_via",
    "clearance_via_pad",
    "clearance_pad_pad",
    "clearance_edge",
    "clearance_keepout",
    "clearance_plane",
    "clearance_other",
    "width_class",
    "width_min",
    "annular_ring",
    "drill",
    "unconnected",
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Track,
    Via,
    Pad,
    Edge,
    Keepout,
    Plane,
    Other,
}

fn kind(item: &Item) -> Kind {
    if item.is_trace() {
        Kind::Track
    } else if item.is_via() {
        Kind::Via
    } else if item.is_pin() {
        Kind::Pad
    } else if item.is_board_outline() {
        Kind::Edge
    } else if item.is_obstacle_area() {
        Kind::Keepout
    } else if item.is_conduction_area() {
        Kind::Plane
    } else {
        Kind::Other
    }
}

/// The result type of a clearance violation between two kinds of item.
fn clearance_type(a: Kind, b: Kind) -> &'static str {
    use Kind::*;
    let has = |k: Kind| a == k || b == k;
    if has(Edge) {
        "clearance_edge"
    } else if has(Keepout) {
        "clearance_keepout"
    } else if has(Plane) {
        "clearance_plane"
    } else if has(Other) {
        "clearance_other"
    } else if a == Track && b == Track {
        "clearance_track_track"
    } else if has(Track) && has(Via) {
        "clearance_track_via"
    } else if has(Track) {
        "clearance_track_pad"
    } else if a == Via && b == Via {
        "clearance_via_via"
    } else if has(Via) {
        "clearance_via_pad"
    } else {
        "clearance_pad_pad"
    }
}

/// One finding.
struct Finding {
    kind: &'static str,
    net: String,
    net2: Option<String>,
    layer: Option<String>,
    at: [i64; 2],
    /// Required and actual value (units), where the check has one.
    required: Option<i64>,
    actual: Option<i64>,
}

impl Finding {
    fn to_json(&self) -> Value {
        json!({
            "type": self.kind, "net": self.net, "net2": self.net2, "layer": self.layer,
            "at": self.at, "required": self.required, "actual": self.actual,
        })
    }
}

fn first_net(board: &BasicBoard, item: &Item) -> String {
    item.net_numbers().first().map(|&n| facts::net_name(board, n)).unwrap_or_default()
}

fn layer_name(board: &BasicBoard, layer: i32) -> Option<String> {
    board.layer_structure.layers.get(layer as usize).map(|l| l.name.clone())
}

fn mid(board: &BasicBoard, a: fr_geom::Point, b: fr_geom::Point) -> [i64; 2] {
    let _ = board;
    let (a, b) = (a.to_float(), b.to_float());
    [((a.x + b.x) / 2.0).round() as i64, ((a.y + b.y) / 2.0).round() as i64]
}

fn clearance_findings(board: &BasicBoard, out: &mut Vec<Finding>, unfixable: &mut i64) {
    for v in all_clearance_violations(board) {
        let (a, b) = (board.item(v.first_item), board.item(v.second_item));
        if v.is_unfixable(board) {
            *unfixable += 1;
        }
        let c = v.shape.centre_of_gravity();
        let net = first_net(board, a);
        let net2 = first_net(board, b);
        out.push(Finding {
            kind: clearance_type(kind(a), kind(b)),
            // the net of the routed item first (a pad or the outline may have none)
            net: if net.is_empty() { net2.clone() } else { net.clone() },
            net2: (!net.is_empty() && !net2.is_empty() && net != net2).then_some(net2),
            layer: layer_name(board, v.layer),
            at: [c.x.round() as i64, c.y.round() as i64],
            required: Some(v.expected_clearance.round() as i64),
            actual: Some(v.actual_clearance.round() as i64),
        });
    }
}

fn width_findings(board: &BasicBoard, min_width: i64, out: &mut Vec<Finding>) {
    for key in board.get_traces() {
        let item = board.item(key);
        let (Some(t), Some(&net)) = (item.as_trace(), item.net_numbers().first()) else { continue };
        if net < 1 || item.fixed_state() == FixedState::SystemFixed {
            continue;
        }
        let width = 2 * t.half_width() as i64;
        let class_width = 2 * board.rules.get_trace_half_width(net, t.layer()) as i64;
        let corners = t.polyline().corners();
        for w in corners.windows(2) {
            let mk = |kind: &'static str, required: i64| Finding {
                kind,
                net: facts::net_name(board, net),
                net2: None,
                layer: layer_name(board, t.layer()),
                at: mid(board, w[0].clone(), w[1].clone()),
                required: Some(required),
                actual: Some(width),
            };
            if width < class_width {
                out.push(mk("width_class", class_width));
            }
            if min_width > 0 && width < min_width {
                out.push(mk("width_min", min_width));
            }
        }
    }
}

/// Annular ring and drill of every via; the count of vias whose padstack name carries no drill.
fn via_findings(board: &BasicBoard, min_annular: i64, min_drill: i64, out: &mut Vec<Finding>) -> i64 {
    let mut unchecked = 0;
    if min_annular <= 0 && min_drill <= 0 {
        return 0;
    }
    for key in board.get_vias() {
        let item = board.item(key);
        let Some(via) = item.as_via() else { continue };
        let Some(ps) = board.library.padstacks.get(via.padstack_no()) else { continue };
        if !ps.name.contains(':') {
            unchecked += 1;
            continue;
        }
        let drill_r = ps.get_drill_radius();
        let mut pad_r = f64::MAX;
        for l in ps.from_layer()..=ps.to_layer() {
            if let Some(s) = ps.get_shape(l) {
                let b = s.bounding_box();
                pad_r = pad_r.min(b.width().min(b.height()) as f64 / 2.0);
            }
        }
        if pad_r == f64::MAX {
            continue;
        }
        let c = item.center(board).to_float();
        let at = [c.x.round() as i64, c.y.round() as i64];
        let mk = |kind: &'static str, required: i64, actual: i64| Finding {
            kind,
            net: first_net(board, item),
            net2: None,
            layer: None,
            at,
            required: Some(required),
            actual: Some(actual),
        };
        let annular = (pad_r - drill_r).round() as i64;
        if min_annular > 0 && annular < min_annular {
            out.push(mk("annular_ring", min_annular, annular));
        }
        let drill = (2.0 * drill_r).round() as i64;
        if min_drill > 0 && drill < min_drill {
            out.push(mk("drill", min_drill, drill));
        }
    }
    unchecked
}

fn unconnected_findings(board: &BasicBoard, out: &mut Vec<Finding>) {
    for u in facts::connections(board).1 {
        out.push(Finding {
            kind: "unconnected",
            net: u.net.clone(),
            net2: None,
            layer: None,
            at: u.from_xy,
            required: None,
            actual: None,
        });
    }
}

/// Runs every check on `board`. `min_width`, `min_annular`, `min_drill` in units, 0 = not asked.
pub fn run(board: &BasicBoard, min_width: i64, min_annular: i64, min_drill: i64, max: usize) -> Value {
    let started = Instant::now();
    let mut found: Vec<Finding> = Vec::new();
    let mut unfixable = 0;
    clearance_findings(board, &mut found, &mut unfixable);
    width_findings(board, min_width, &mut found);
    let vias_unchecked = via_findings(board, min_annular, min_drill, &mut found);
    unconnected_findings(board, &mut found);

    let mut counts: BTreeMap<&str, i64> = TYPES.iter().map(|t| (*t, 0)).collect();
    for f in &found {
        *counts.entry(f.kind).or_insert(0) += 1;
    }
    let sum = |prefix: &str| counts.iter().filter(|(k, _)| k.starts_with(prefix)).map(|(_, v)| v).sum::<i64>();
    let (clearance, width, via, unconnected) =
        (sum("clearance_"), sum("width_"), counts["annular_ring"] + counts["drill"], counts["unconnected"]);

    let order = |t: &str| TYPES.iter().position(|x| *x == t).unwrap_or(TYPES.len());
    found.sort_by(|a, b| {
        (order(a.kind), &a.net, &a.layer, a.at, &a.net2).cmp(&(order(b.kind), &b.net, &b.layer, b.at, &b.net2))
    });
    let truncated = found.len() > max;
    let items: Vec<Value> = found.iter().take(max).map(Finding::to_json).collect();
    json!({
        "counts": TYPES.iter().map(|t| (t.to_string(), json!(counts[t]))).collect::<Map<String, Value>>(),
        "summary": { "clearance": clearance, "width": width, "via": via, "unconnected": unconnected,
                     "total": clearance + width + via + unconnected, "unfixable_clearance": unfixable },
        "items": items,
        "truncated": truncated,
        "vias_unchecked": vias_unchecked,
        "rules": { "min_width": min_width, "min_annular": min_annular, "min_drill": min_drill },
        "not_checked": NOT_CHECKED,
        "wall_ms": started.elapsed().as_millis() as i64,
    })
}

pub fn handle(session: &mut Session, args: &Map<String, Value>) -> R<Value> {
    if !CLAIMED {
        return Err(ProtoError::unsupported("check", "check"));
    }
    check_keys(args, &["max", "min_width", "min_annular", "min_drill"], "check")?;
    let get = |key: &str, lo: i64, hi: i64, default: i64| -> R<i64> {
        args.get(key).map(|v| as_int(v, lo, hi, &format!("check.{key}"))).transpose().map(|v| v.unwrap_or(default))
    };
    let max = get("max", 0, MAX_MAX, DEFAULT_MAX)? as usize;
    // strict like KiCad: a copy of the board with no clearance tolerance (the router counts up to
    // `clearance_tolerance_um`, 1 um by default, as clear); the session board is not changed
    let mut work = session.begin()?;
    work.board.board.rules_mut().clearance_tolerance_um = 0.0;
    let board = &work.board;
    let b: &BasicBoard = &board.board;
    // the session's own minimum width unless the request names one
    let from_settings = {
        let um = board.settings.get_min_trace_width_um();
        if um > 0.0 { (um / facts::um_per_unit(b)).round() as i64 } else { 0 }
    };
    let min_width = get("min_width", 0, 1 << 40, from_settings)?;
    let min_annular = get("min_annular", 0, 1 << 40, 0)?;
    let min_drill = get("min_drill", 0, 1 << 40, 0)?;
    Ok(run(b, min_width, min_annular, min_drill, max))
}
