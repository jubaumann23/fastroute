//! Op `blockers` (capability `blockers`, SPEC 5.6): why a connection is open.
//!
//! The connection is routed alone on a copy of the board as loaded plus the locked wiring (every
//! unlocked routed trace and via removed) with core hook H3 (`collect_blockers`, rip-up off).
//! Failure: `blocked`. Success: `congestion`. The named objects are causal, checked by the same
//! alone-route (falsified on the corpus boards by `tests/serve_blockers.rs`):
//!
//! * the H3 hits of a failed attempt are every item the search touched, hundreds on a real board;
//!   [`causal_set`] removes the wires and vias among them, round by round, until the attempt routes,
//!   then keeps the shortest prefix (nearest first) that is enough. Pins, keepouts and the boundary
//!   cannot be removed, so they are named only when removing every wire and via is not enough;
//! * `congestion` is attributed on the board with the routed wiring kept: when the connection fails
//!   there too, its causal set names the routed wires that stand in the way. The wiring on the
//!   free path (first answer of this op) is the fallback when the attempt routes on the kept board;
//! * an attempt that routes but adds nothing found the two ends already joined by copper that the
//!   connectivity does not accept (a fixed wire ending on a corner of another, a T-junction): the
//!   class is `blocked` and the two items at the gap are named.

use fr_engine::board::{BasicBoard, ItemKey, ItemKind, ObstacleKind, RoutingBoard};
use fr_engine::ids::FixedState;
use fr_engine::pipeline::BatchAutorouter;
use serde_json::{json, Map, Value};

use crate::facts::{self, Unrouted};
use crate::ops::lock::LockRegistry;
use crate::proto::{as_int, as_obj, as_str, check_keys, req, ProtoError, R};
use crate::session::Session;

/// True once this module implements the op and passes its conformance check.
pub const CLAIMED: bool = true;

const DEFAULT_MAX: i64 = 20;
/// Samples along the connection's line when looking for the conflict point inside an object.
const LINE_SAMPLES: i64 = 64;

type Pt = (f64, f64);

/// Distance from `p` to the segment `a-b`.
fn dist_to_segment(p: Pt, a: Pt, b: Pt) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0.0 { 0.0 } else { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0) };
    ((p.0 - (a.0 + t * dx)).powi(2) + (p.1 - (a.1 + t * dy)).powi(2)).sqrt()
}

/// Net number of `name`.
fn net_no(board: &BasicBoard, name: &str) -> Option<i32> {
    (1..=board.rules.nets.max_net_number()).find(|&n| board.rules.nets.get(n).is_some_and(|x| x.name == name))
}

fn net_name(board: &BasicBoard, n: i32) -> Option<String> {
    (n > 0).then(|| board.rules.nets.get(n).map(|x| x.name.clone())).flatten()
}

/// The pin item labelled `REF-PIN`.
pub(crate) fn pin_by_label(board: &BasicBoard, label: &str) -> Option<ItemKey> {
    board.get_items().into_iter().find(|&k| {
        let it = board.item(k);
        it.is_pin() && it.component_no() > 0 && facts::label(board, k, (0, 0)) == label
    })
}

/// The pin of `net` closest to `xy` (end of a connection that is not a pin, e.g. a fixed wire).
fn nearest_pin(board: &BasicBoard, net: i32, xy: [i64; 2], not: Option<ItemKey>) -> Option<ItemKey> {
    let mut best: Option<(f64, ItemKey)> = None;
    for k in board.get_items() {
        let it = board.item(k);
        if !it.is_pin() || !it.net_numbers().contains(&net) || Some(k) == not {
            continue;
        }
        let bb = it.bounding_box(board);
        let c = ((bb.ll.x as f64 + bb.ur.x as f64) / 2.0, (bb.ll.y as f64 + bb.ur.y as f64) / 2.0);
        let d = (c.0 - xy[0] as f64).hypot(c.1 - xy[1] as f64);
        if best.map_or(true, |(bd, _)| d < bd) {
            best = Some((d, k));
        }
    }
    best.map(|(_, k)| k)
}

struct Query {
    net: String,
    net_no: i32,
    from: String,
    to: String,
}

fn parse_query(board: &BasicBoard, args: &Map<String, Value>) -> R<Query> {
    let c = as_obj(req(args, "connection", "blockers")?, "blockers.connection")?;
    check_keys(c, &["net", "from", "to"], "blockers.connection")?;
    let net = as_str(req(c, "net", "blockers.connection")?, "blockers.connection.net")?.to_string();
    let from = as_str(req(c, "from", "blockers.connection")?, "blockers.connection.from")?.to_string();
    let to = as_str(req(c, "to", "blockers.connection")?, "blockers.connection.to")?.to_string();
    let Some(no) = net_no(board, &net) else {
        return Err(ProtoError::new("unknown_net", format!("the board has no net '{net}'")).with_details(json!({ "name": net })));
    };
    Ok(Query { net, net_no: no, from, to })
}

/// A pin label the board does not have (wire / via ends, `kind@x,y`, are checked against the open connections).
fn check_pin(board: &BasicBoard, q: &Query, label: &str) -> R<()> {
    if label.contains('@') {
        return Ok(());
    }
    match pin_by_label(board, label) {
        None => Err(ProtoError::new("unknown_pin", format!("the board has no pin '{label}'")).with_details(json!({ "pin": label }))),
        Some(k) if !board.item(k).net_numbers().contains(&q.net_no) => Err(ProtoError::new(
            "unknown_connection",
            format!("pin '{label}' is not on net '{}'", q.net),
        )
        .with_details(json!({ "net": q.net, "pin": label }))),
        Some(_) => Ok(()),
    }
}

/// One object of the answer with its sort key.
struct Found {
    dist: i64,
    kind: &'static str,
    ref_: Option<String>,
    pin: Option<String>,
    id: Option<i32>,
    json: Value,
}

/// The shapes of an item as `(layer, corners)`.
fn shape_corners(board: &BasicBoard, key: ItemKey) -> Vec<Vec<Pt>> {
    let mut out = Vec::new();
    for i in 0..board.tile_shape_count(key) {
        let Some(shape) = board.tile_shape(key, i) else { continue };
        if shape.is_empty() || !shape.is_bounded() {
            continue;
        }
        out.push(shape.corner_approx_arr().iter().map(|p| (p.x, p.y)).collect());
    }
    out
}

/// The object `key` of `board` in protocol terms, or `None` for items that are no blockers (component outlines).
fn describe(board: &BasicBoard, locks: &LockRegistry, key: ItemKey, a: Pt, b: Pt) -> Option<Found> {
    let it = board.item(key);
    let (kind, id) = match &it.kind {
        ItemKind::Pin(_) => ("pin", None),
        ItemKind::Via(_) => ("via", Some(it.id().0)),
        ItemKind::Trace(_) => ("wire", Some(it.id().0)),
        ItemKind::ObstacleArea(o) => match o.kind {
            ObstacleKind::ViaKeepout => ("via_keepout", None),
            _ => ("keepout", None),
        },
        ItemKind::BoardOutline(_) => ("boundary", None),
        ItemKind::ConductionArea(_) => ("plane", None),
        ItemKind::ComponentOutline(_) => return None,
    };
    let (mut ref_, mut pin) = (None, None);
    if let ItemKind::Pin(p) = &it.kind {
        if it.component_no() > 0 {
            let comp = board.components.get(it.component_no());
            ref_ = Some(comp.name.clone());
            pin = board.library.packages.get(comp.get_package()).get_pin(p.pin_index).map(|x| x.name.clone());
        }
    }
    let net = it.net_numbers().first().and_then(|&n| net_name(board, n));
    let (l0, l1) = (it.first_layer(board), it.last_layer(board));
    let layer = (l0 == l1).then(|| board.layer_structure.layers[l0 as usize].name.clone());
    let wiring = id.is_some();
    let locked = wiring && locks.is_locked_wire(it.id().0);
    let fixed = if wiring {
        match it.fixed_state() {
            FixedState::SystemFixed => true,
            FixedState::UserFixed => !locked,
            _ => false,
        }
    } else {
        true
    };
    let bb = it.bounding_box(board);
    let bbox = [bb.ll.x as i64, bb.ll.y as i64, bb.ur.x as i64, bb.ur.y as i64];

    // the point of the conflict closest to the connection's line: object corners, plus the points
    // of the line itself that lie inside one of the object's shapes
    let mut cand: Vec<Pt> = shape_corners(board, key).into_iter().flatten().collect();
    let (x0, y0, x1, y1) = (bb.ll.x as f64, bb.ll.y as f64, bb.ur.x as f64, bb.ur.y as f64);
    let shapes: Vec<_> = (0..board.tile_shape_count(key)).filter_map(|i| board.tile_shape(key, i)).collect();
    for s in 0..=LINE_SAMPLES {
        let t = s as f64 / LINE_SAMPLES as f64;
        let p = (a.0 + t * (b.0 - a.0), a.1 + t * (b.1 - a.1));
        let probe = fr_geom::IntBox::new(p.0.floor() as i32, p.1.floor() as i32, p.0.ceil() as i32, p.1.ceil() as i32);
        if shapes.iter().any(|sh| sh.is_bounded() && sh.intersects_int_box(&probe)) {
            cand.push(p);
        }
    }
    if cand.is_empty() {
        cand.push(((x0 + x1) / 2.0, (y0 + y1) / 2.0));
    }
    let at = cand
        .into_iter()
        .map(|p| (dist_to_segment(p, a, b), p))
        .min_by(|x, y| x.0.total_cmp(&y.0).then(x.1 .0.total_cmp(&y.1 .0)).then(x.1 .1.total_cmp(&y.1 .1)))
        .map(|(_, p)| p)?;
    let at = [at.0.round() as i64, at.1.round() as i64];
    let dist = dist_to_segment((at[0] as f64, at[1] as f64), a, b).round() as i64;
    let json = json!({
        "kind": kind, "id": id, "ref": ref_, "pin": pin, "net": net, "layer": layer,
        "fixed": fixed, "locked": locked, "at": at, "bbox": bbox,
    });
    Some(Found { dist, kind, ref_, pin, id, json })
}

/// Routed trace and via items of other nets that come within clearance of the items the alone
/// route added (`routed` is the board that route ran on).
fn congestion_keys(current: &BasicBoard, routed: &RoutingBoard, added: &[fr_engine::ids::ItemId], net: i32) -> Vec<ItemKey> {
    let mut path: Vec<(i32, i32, fr_geom::tile_shape::TileShape)> = Vec::new(); // (layer, clearance class, shape)
    for id in added {
        let Some(k) = routed.get_item(*id) else { continue };
        for i in 0..routed.tile_shape_count(k) {
            if let Some(s) = routed.tile_shape(k, i) {
                path.push((routed.shape_layer(k, i), routed.item(k).clearance_class(), s));
            }
        }
    }
    // The items on the path first; when the free path touches none (the router lost the connection
    // to ordering), the nearest items of other nets: the margin grows until something is within it.
    let mut out = Vec::new();
    for factor in [0.0, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0] {
        out = touching(current, &path, net, factor);
        if !out.is_empty() {
            break;
        }
    }
    out
}

/// Trace and via items of other nets within clearance plus `factor` track pitches of the path.
fn touching(current: &BasicBoard, path: &[(i32, i32, fr_geom::tile_shape::TileShape)], net: i32, factor: f64) -> Vec<ItemKey> {
    let mut out = Vec::new();
    for key in current.get_items() {
        let it = current.item(key);
        if !(it.is_trace() || it.is_via()) || it.net_numbers().contains(&net) {
            continue;
        }
        let hit = (0..current.tile_shape_count(key)).any(|i| {
            let (Some(s), l) = (current.tile_shape(key, i), current.shape_layer(key, i)) else { return false };
            path.iter().any(|(pl, cc, ps)| {
                *pl == l && {
                    let clr = current.rules.clearance_matrix.get_value(*cc, it.clearance_class(), l, false) as f64;
                    let pitch = clr + 2.0 * current.rules.get_default_trace_half_width(l) as f64;
                    ps.offset(clr + factor * pitch).intersects_tile(&s)
                }
            })
        });
        if hit {
            out.push(key);
        }
    }
    out
}

/// Fallback when the free path touches nothing: the routed items of other nets nearest to the line.
fn nearest_wiring(board: &BasicBoard, net: i32, a: Pt, b: Pt, take: usize) -> Vec<ItemKey> {
    let mut v: Vec<(i64, i32, ItemKey)> = board
        .get_items()
        .into_iter()
        .filter(|&k| {
            let it = board.item(k);
            (it.is_trace() || it.is_via()) && !it.net_numbers().contains(&net)
        })
        .map(|k| {
            let it = board.item(k);
            let bb = it.bounding_box(board);
            let c = ((bb.ll.x as f64 + bb.ur.x as f64) / 2.0, (bb.ll.y as f64 + bb.ur.y as f64) / 2.0);
            (dist_to_segment(c, a, b).round() as i64, it.id().0, k)
        })
        .collect();
    v.sort_by_key(|&(d, id, _)| (d, id));
    v.into_iter().take(take).map(|(_, _, k)| k).collect()
}

/// The alone-route of one open connection, with the board it ran on.
pub(crate) struct AloneRun {
    pub routed: bool,
    pub hits: Vec<ItemKey>,
    pub added: Vec<fr_engine::ids::ItemId>,
    pub after: RoutingBoard,
    pub alone: RoutingBoard,
}

/// Routes `conn` alone on `current` as loaded plus locked wiring (every unlocked routed trace and via
/// removed), with core hook H3 (`collect_blockers`, rip-up off). `remove` lists item ids that are also
/// taken off that board first (the falsification test removes the objects a `blocked` answer named;
/// the ends of the connection are never removed); `keep_routed` keeps the routed wiring too (the board
/// a targeted route after a rip-up sees). `None` when an end has no pin.
pub(crate) fn alone_route(
    current: &RoutingBoard,
    conn: &Unrouted,
    net_no: i32,
    settings: &fr_settings::RouterSettings,
    remove: &[i32],
    keep_routed: bool,
) -> Option<AloneRun> {
    // the board as loaded plus locked wiring: every unlocked routed trace and via goes
    let mut alone = current.clone();
    let doomed: Vec<ItemKey> = alone
        .get_items()
        .into_iter()
        .filter(|&k| {
            let it = alone.item(k);
            (it.is_trace() || it.is_via()) && matches!(it.fixed_state(), FixedState::Unfixed | FixedState::ShoveFixed)
        })
        .collect();
    if !keep_routed {
        alone.remove_items(doomed);
    }
    if !remove.is_empty() {
        let ends = [current.item(conn.from_item).id().0, current.item(conn.to_item).id().0];
        let gone: Vec<ItemKey> = remove
            .iter()
            .filter(|id| !ends.contains(id))
            .filter_map(|&id| alone.get_item(fr_engine::ids::ItemId(id)))
            .collect();
        for k in gone {
            // fixed wiring is a deliberate removal here (a copy): demote it so the board lets it go;
            // pins (component pads) stay, the board never removes them
            if alone.item(k).is_trace() || alone.item(k).is_via() {
                alone.item_mut(k).set_fixed_state(FixedState::Unfixed);
            }
            alone.remove_item(k);
        }
    }

    // A pin end is the pin. Another end (a wire or via of fixed or locked wiring) is that item, which
    // the alone board keeps; if it was removed with the routed wiring, the nearest other pin of the net.
    let kept = |k: ItemKey| alone.get_item(current.item(k).id());
    let from_pin = pin_by_label(&alone, &conn.from).or_else(|| kept(conn.from_item));
    let to_pin = pin_by_label(&alone, &conn.to).or_else(|| kept(conn.to_item));
    let from_pin = from_pin.or_else(|| nearest_pin(&alone, net_no, conn.from_xy, to_pin));
    let to_pin = to_pin.or_else(|| nearest_pin(&alone, net_no, conn.to_xy, from_pin));
    let (from_pin, to_pin) = (from_pin?, to_pin?);
    let (routed, hits, added, after) = BatchAutorouter::route_connection_alone_on(&alone, from_pin, to_pin, settings);
    Some(AloneRun { routed, hits, added, after, alone })
}

/// True for the items the causal search may take off: wires and vias; with `routed_only` just the
/// routed ones (unfixed), which are the ones a rip-up can remove.
fn removable(board: &BasicBoard, key: ItemKey, routed_only: bool) -> bool {
    let it = board.item(key);
    (it.is_trace() || it.is_via()) && (!routed_only || matches!(it.fixed_state(), FixedState::Unfixed | FixedState::ShoveFixed))
}

/// Wires and vias that make `conn` fail, found by removing them: `Some(keys)` is a set that, taken off
/// the board of the attempt, lets the connection route, shortest first-by-round prefix; `None` when
/// removing every wire and via the search touched, round after round, never lets it route.
fn causal_set(
    current: &RoutingBoard,
    conn: &Unrouted,
    net_no: i32,
    settings: &fr_settings::RouterSettings,
    keep_routed: bool,
    first: &AloneRun,
    near: &dyn Fn(ItemKey) -> i64,
) -> Option<Vec<ItemKey>> {
    const ROUNDS: usize = 16;
    let mut order: Vec<ItemKey> = Vec::new();
    let mut hits = first.hits.clone();
    for _ in 0..ROUNDS {
        let mut fresh: Vec<ItemKey> = hits.iter().copied().filter(|&k| removable(&first.alone, k, keep_routed) && !order.contains(&k)).collect();
        if fresh.is_empty() && keep_routed {
            // The search names only what it ran into; routed wiring can stand in the way without
            // being named. Every other net's routed item, nearest to the line first, is a candidate.
            let b = &first.alone;
            fresh = b
                .get_items()
                .into_iter()
                .filter(|&k| removable(b, k, true) && !b.item(k).net_numbers().contains(&net_no) && !order.contains(&k))
                .collect();
        }
        if fresh.is_empty() {
            return None;
        }
        fresh.sort_by_key(|&k| (near(k), first.alone.item(k).id().0));
        order.extend(fresh);
        let ids: Vec<i32> = order.iter().map(|&k| first.alone.item(k).id().0).collect();
        let run = alone_route(current, conn, net_no, settings, &ids, keep_routed)?;
        if run.routed {
            // shortest prefix of `order` that is enough (removing more never hurts)
            let (mut lo, mut hi) = (1, order.len());
            while lo < hi {
                let mid = (lo + hi) / 2;
                let ids: Vec<i32> = order[..mid].iter().map(|&k| first.alone.item(k).id().0).collect();
                if alone_route(current, conn, net_no, settings, &ids, keep_routed).is_some_and(|r| r.routed) {
                    hi = mid;
                } else {
                    lo = mid + 1;
                }
            }
            order.truncate(lo);
            return Some(order);
        }
        hits = run.hits;
    }
    None
}

/// The nearest pair of items of the two sides of an open connection (by bounding-box gap): the
/// place where copper meets without being connected, or the narrowest gap to route across.
fn gap_items(board: &BasicBoard, conn: &Unrouted) -> Vec<ItemKey> {
    let net = conn.net_no;
    let side = |k: ItemKey| -> Vec<ItemKey> { board.connected_set(k, net, false).iter().collect() };
    let (a, b) = (side(conn.from_item), side(conn.to_item));
    let gap = |x: ItemKey, y: ItemKey| -> i64 {
        let (p, q) = (board.item(x).bounding_box(board), board.item(y).bounding_box(board));
        let dx = (p.ll.x.max(q.ll.x) as i64 - p.ur.x.min(q.ur.x) as i64).max(0);
        let dy = (p.ll.y.max(q.ll.y) as i64 - p.ur.y.min(q.ur.y) as i64).max(0);
        dx.max(dy)
    };
    let mut best: Option<(i64, i32, i32, ItemKey, ItemKey)> = None;
    for &x in &a {
        for &y in &b {
            let cand = (gap(x, y), board.item(x).id().0, board.item(y).id().0, x, y);
            if best.map_or(true, |bb| (cand.0, cand.1, cand.2) < (bb.0, bb.1, bb.2)) {
                best = Some(cand);
            }
        }
    }
    best.map(|(_, _, _, x, y)| vec![x, y]).unwrap_or_default()
}

pub fn handle(session: &mut Session, args: &Map<String, Value>) -> R<Value> {
    check_keys(args, &["connection", "max"], "blockers")?;
    let max = args.get("max").map(|v| as_int(v, 1, 1 << 20, "blockers.max")).transpose()?.unwrap_or(DEFAULT_MAX) as usize;
    let loaded = session.loaded()?;
    let current = &loaded.board;
    let q = parse_query(current, args)?;
    check_pin(current, &q, &q.from)?;
    check_pin(current, &q, &q.to)?;
    let echo = json!({ "net": q.net, "from": q.from, "to": q.to });

    let (_, open) = facts::connections(current);
    let same = |u: &Unrouted| {
        u.net_no == q.net_no && ((u.from == q.from && u.to == q.to) || (u.from == q.to && u.to == q.from))
    };
    let Some(conn) = open.iter().find(|u| same(u)) else {
        return Ok(json!({ "connection": echo, "class": "routed", "blockers": [] }));
    };

    let echo_err = echo.clone();
    let alone_run = alone_route(current, conn, q.net_no, &loaded.settings, &[], false)
        .ok_or_else(|| ProtoError::new("unknown_connection", "the connection has no pin ends on this board").with_details(echo_err))?;
    let AloneRun { routed, added, .. } = &alone_run;
    let (routed, alone, after) = (*routed, &alone_run.alone, &alone_run.after);

    let a = (conn.from_xy[0] as f64, conn.from_xy[1] as f64);
    let b = (conn.to_xy[0] as f64, conn.to_xy[1] as f64);
    let locks = &session.locks;
    let settings = &loaded.settings;
    let near = |board: &BasicBoard, k: ItemKey| describe(board, locks, k, a, b).map_or(i64::MAX, |f| f.dist);
    let (class, mut found): (&str, Vec<Found>) = if routed && added.is_empty() {
        // routed with nothing added: the ends are joined by copper the connectivity does not accept
        ("blocked", gap_items(current, conn).into_iter().filter_map(|k| describe(current, locks, k, a, b)).collect())
    } else if routed {
        // attribute on the board with the routed wiring kept; the free path is the fallback
        let keep = alone_route(current, conn, q.net_no, settings, &[], true);
        let causal = keep.as_ref().filter(|r| !r.routed).and_then(|r| {
            causal_set(current, conn, q.net_no, settings, true, r, &|k| near(&r.alone, k)).map(|keys| (keys, r))
        });
        let keys = match causal {
            Some((keys, _)) => keys,
            None => {
                let mut keys = congestion_keys(current, after, added, q.net_no);
                if keys.is_empty() {
                    keys = nearest_wiring(current, q.net_no, a, b, max.saturating_mul(4));
                }
                keys
            }
        };
        ("congestion", keys.into_iter().filter_map(|k| describe(current, locks, k, a, b)).collect())
    } else {
        let keys = causal_set(current, conn, q.net_no, settings, false, &alone_run, &|k| near(alone, k)).unwrap_or_else(|| alone_run.hits.clone());
        ("blocked", keys.into_iter().filter_map(|k| describe(alone, locks, k, a, b)).collect())
    };
    found.sort_by(|x, y| (x.dist, x.kind, &x.ref_, &x.pin, x.id).cmp(&(y.dist, y.kind, &y.ref_, &y.pin, y.id)));
    found.truncate(max);
    let blockers: Vec<Value> = found.into_iter().map(|f| f.json).collect();
    Ok(json!({ "connection": echo, "class": class, "blockers": blockers }))
}
