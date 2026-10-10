//! Op `widen` (capability `widen`), SPEC 5.11: after a route, widen every trace segment narrower than its
//! net class width (or the width the request names for its net) as far as clearance allows.
//!
//! Per trace that is narrower than its target (router and shove-fixed wiring; DSN `fix` wiring and locked
//! wiring are never touched):
//! 1. try the whole trace at the target width;
//! 2. if that violates a clearance and the trace has several segments, split it into one trace per
//!    segment (same nets, class, layer, width, fixed state), so a squeeze at one pad does not hold the
//!    rest of the trace narrow;
//! 3. a single segment that cannot reach the target is bisected for the widest width that works (down
//!    to one micrometre).
//!
//! "Works" is judged by the clearance engine on the real board: the item is replaced at the candidate
//! width and `clearance_violations` of the new item must not name an item the old one did not. The check
//! is strict (`clearance_tolerance_um` is 0 while it runs): the router counts a shortfall of up to a
//! micrometre as clear, KiCad does not. A
//! candidate that adds a violation is put back. So a widen never adds a clearance violation, and a
//! violation that was already there (an unfixable pad-to-pad one, say) does not stop a segment that
//! does not touch it. Widened traces are new items (new ids); split traces keep their line geometry.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use fr_engine::board::optimize::tracked::TPolyline;
use fr_engine::board::{BasicBoard, ItemKey};
use fr_engine::drc::clearance_violations;
use fr_engine::ids::{FixedState, ItemId, LayerNo, NetNo};
use fr_geom::Polyline;
use serde_json::{json, Map, Value};

use crate::facts;
use crate::proto::{as_int, as_obj, as_str, check_keys, ProtoError, R};
use crate::session::Session;

/// True once this module implements the op and passes its conformance check.
pub const CLAIMED: bool = true;

/// The bisection stops when the interval is this many micrometres wide (full width).
const STEP_UM: f64 = 1.0;
/// Longest list of nets left narrow in a result.
const MAX_NET_ROWS: usize = 50;

/// What a trace is, copied out of the board before it is replaced.
struct Trace {
    poly: TPolyline,
    layer: LayerNo,
    half: i32,
    nets: Vec<NetNo>,
    class: i32,
    fixed: FixedState,
}

fn read_trace(board: &BasicBoard, key: ItemKey) -> Option<Trace> {
    let item = board.try_item(key)?;
    let t = item.as_trace()?;
    Some(Trace {
        poly: t.tpolyline(),
        layer: t.layer(),
        half: t.half_width(),
        nets: item.net_numbers().to_vec(),
        class: item.clearance_class(),
        fixed: item.fixed_state(),
    })
}

/// Ids of the items a trace violates clearance against.
fn violated(board: &BasicBoard, key: ItemKey) -> BTreeSet<(ItemId, LayerNo)> {
    clearance_violations(board, key)
        .into_iter()
        .map(|v| {
            let other = if v.first_item == key { v.second_item } else { v.first_item };
            (board.item(other).id(), v.layer)
        })
        .collect()
}

/// Replaces `key` by the same trace at half width `half`. The new key, or `None` if nothing was inserted.
fn replace_at(board: &mut BasicBoard, key: ItemKey, t: &Trace, half: i32) -> Option<ItemKey> {
    board.remove_item(key);
    board.insert_trace_without_cleaning_tracked(t.poly.clone(), t.layer, half, &t.nets, t.class, t.fixed)
}

/// The trace `t` at `half` in place of `key` if that adds no violation to `allowed`; else the trace is put
/// back at `keep`. Returns the key of whatever is on the board and whether `half` was taken.
fn try_half(board: &mut BasicBoard, key: ItemKey, t: &Trace, allowed: &BTreeSet<(ItemId, LayerNo)>, half: i32, keep: i32) -> (ItemKey, bool) {
    let Some(new_key) = replace_at(board, key, t, half) else {
        // nothing inserted (cannot happen for a trace that was on the board): put the original back
        return (replace_at(board, key, t, keep).unwrap_or(key), false);
    };
    if violated(board, new_key).is_subset(allowed) {
        return (new_key, true);
    }
    (replace_at(board, new_key, t, keep).unwrap_or(new_key), false)
}

/// Widens one single-segment trace towards `target`; returns the final half width.
fn widen_segment(board: &mut BasicBoard, key: ItemKey, target: i32, step_half: i32) -> i32 {
    let Some(t) = read_trace(board, key) else { return 0 };
    let allowed = violated(board, key);
    let (mut key, ok) = try_half(board, key, &t, &allowed, target, t.half);
    if ok {
        return target;
    }
    // invariant: the trace on the board is at `lo`, which is valid; `hi` is not
    let (mut lo, mut hi) = (t.half, target);
    while hi - lo > step_half {
        let mid = lo + (hi - lo) / 2;
        let (k, ok) = try_half(board, key, &t, &allowed, mid, lo);
        key = k;
        if ok {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lo
}

/// One segment's outcome: its final half width.
struct Piece {
    half: i32,
}

/// Widens the trace at `key` towards `target`; the outcome per segment of the result.
fn widen_trace(board: &mut BasicBoard, key: ItemKey, target: i32, step_half: i32) -> Vec<Piece> {
    let Some(t) = read_trace(board, key) else { return Vec::new() };
    let segments = (t.poly.polyline.corner_count() - 1).max(0) as usize;
    if segments <= 1 {
        return vec![Piece { half: widen_segment(board, key, target, step_half) }];
    }
    let allowed = violated(board, key);
    let (new_key, ok) = try_half(board, key, &t, &allowed, target, t.half);
    if ok {
        return (0..segments).map(|_| Piece { half: target }).collect();
    }
    // several segments: one trace per segment, each widened on its own
    let corners = t.poly.polyline.corners();
    board.remove_item(new_key);
    let mut out = Vec::with_capacity(segments);
    for w in corners.windows(2) {
        let poly = TPolyline::fresh(Polyline::from_points(&[w[0].clone(), w[1].clone()]));
        match board.insert_trace_without_cleaning_tracked(poly, t.layer, t.half, &t.nets, t.class, t.fixed) {
            Some(k) => out.push(Piece { half: widen_segment(board, k, target, step_half) }),
            None => out.push(Piece { half: t.half }),
        }
    }
    out
}

/// Per net class totals.
#[derive(Default)]
struct ClassRow {
    width: i64,
    segments: i64,
    narrow_before: i64,
    narrow_after: i64,
    min_width: Option<i64>,
}

/// Per net left narrow.
struct NetRow {
    class: String,
    target: i64,
    narrow: i64,
    min_width: i64,
}

fn is_editable(fixed: FixedState) -> bool {
    matches!(fixed, FixedState::Unfixed | FixedState::ShoveFixed)
}

/// Widens the board's traces in place. `widths`: net number -> full width wanted; `only`: restrict to these nets.
pub fn widen_board(board: &mut BasicBoard, widths: &BTreeMap<NetNo, i64>, only: Option<&BTreeSet<NetNo>>) -> Value {
    let started = Instant::now();
    let step_half = ((STEP_UM / facts::um_per_unit(board)) / 2.0).round().max(1.0) as i32;
    // strict: the router counts a shortfall up to `clearance_tolerance_um` as clear, KiCad does not
    let tolerance = std::mem::replace(&mut board.rules_mut().clearance_tolerance_um, 0.0);
    let mut classes: BTreeMap<String, ClassRow> = BTreeMap::new();
    let mut nets: BTreeMap<NetNo, NetRow> = BTreeMap::new();
    let (mut segments, mut narrow_before, mut widened, mut partial) = (0i64, 0i64, 0i64, 0i64);

    for key in board.get_traces() {
        let Some(t) = read_trace(board, key) else { continue };
        let Some(&net) = t.nets.first() else { continue };
        if net < 1 || t.fixed == FixedState::SystemFixed || only.is_some_and(|o| !o.contains(&net)) {
            continue;
        }
        let target = match widths.get(&net) {
            Some(w) => (*w / 2) as i32,
            None => board.rules.get_trace_half_width(net, t.layer),
        };
        let segs = (t.poly.polyline.corner_count() - 1).max(0) as i64;
        let class_name = board.rules.net_class_of(net).get_name().to_string();
        let row = classes.entry(class_name.clone()).or_default();
        row.segments += segs;
        row.width = row.width.max(2 * target as i64);
        segments += segs;
        let finals: Vec<i32> = if t.half >= target {
            vec![t.half; segs as usize]
        } else {
            narrow_before += segs;
            row.narrow_before += segs;
            if is_editable(t.fixed) {
                widen_trace(board, key, target, step_half).into_iter().map(|p| p.half).collect()
            } else {
                vec![t.half; segs as usize]
            }
        };
        let row = classes.entry(class_name.clone()).or_default();
        for half in finals {
            let w = 2 * half as i64;
            row.min_width = Some(row.min_width.map_or(w, |m| m.min(w)));
            if half > t.half {
                widened += 1;
            }
            if half < target {
                row.narrow_after += 1;
                if half > t.half {
                    partial += 1;
                }
                let n = nets.entry(net).or_insert(NetRow { class: class_name.clone(), target: 2 * target as i64, narrow: 0, min_width: w });
                n.narrow += 1;
                n.min_width = n.min_width.min(w);
            }
        }
    }
    board.rules_mut().clearance_tolerance_um = tolerance;
    let left_narrow: i64 = classes.values().map(|c| c.narrow_after).sum();
    let net_rows: Vec<Value> = nets
        .iter()
        .take(MAX_NET_ROWS)
        .map(|(n, r)| json!({ "net": facts::net_name(board, *n), "class": r.class, "target": r.target, "narrow": r.narrow, "min_width": r.min_width }))
        .collect();
    json!({
        "segments": segments,
        "narrow_before": narrow_before,
        "widened": widened,
        "partial": partial,
        "left_narrow": left_narrow,
        "wires": facts::wiring(board).wires,
        "classes": classes.iter().map(|(name, c)| json!({
            "name": name, "width": c.width, "segments": c.segments,
            "narrow_before": c.narrow_before, "narrow_after": c.narrow_after, "min_width": c.min_width,
        })).collect::<Vec<_>>(),
        "nets": net_rows,
        "wall_ms": started.elapsed().as_millis() as i64,
    })
}

pub fn handle(session: &mut Session, args: &Map<String, Value>) -> R<Value> {
    if !CLAIMED {
        return Err(ProtoError::unsupported("widen", "widen"));
    }
    check_keys(args, &["widths", "nets"], "widen")?;
    let mut work = session.begin()?;
    let board: &BasicBoard = &work.board.board;
    let unknown = |name: &str| ProtoError::new("unknown_net", format!("no net '{name}'")).with_details(json!({ "net": name }));
    let mut widths: BTreeMap<NetNo, i64> = BTreeMap::new();
    if let Some(w) = args.get("widths") {
        for (name, v) in as_obj(w, "widen.widths")? {
            let n = facts::net_number(board, name).ok_or_else(|| unknown(name))?;
            widths.insert(n, as_int(v, 2, 1 << 40, &format!("widen.widths.{name}"))?);
        }
    }
    let only = match args.get("nets") {
        None => None,
        Some(list) => {
            let list = list.as_array().ok_or_else(|| ProtoError::bad_request("widen.nets must be a list of net names"))?;
            let mut set = BTreeSet::new();
            for v in list {
                let name = as_str(v, "widen.nets[]")?;
                set.insert(facts::net_number(board, name).ok_or_else(|| unknown(name))?);
            }
            Some(set)
        }
    };
    let result = widen_board(&mut work.board.board, &widths, only.as_ref());
    session.commit(work);
    Ok(result)
}
