//! fastroute: parallel autorouting pass.
//!
//! Freerouting routes the connections of a pass one after the other on one board. Here the
//! pass is processed in batches: each batch takes the next connections (in pass order) whose
//! surroundings do not overlap and whose nets differ, routes each of them on its own clone of
//! the board state at batch start (in parallel; the connections dispatched before the board
//! changes share one snapshot, and a clone shares the item and tree storage with it until it
//! writes, see [`crate::datastructures::cow_vec`]), and then commits the results in pass order.
//! A worker keeps only its changes and copies of the items involved ([`Condensed`]), so no
//! board stays alive while its result waits for its turn. A result is committed by copying
//! its changes (inserted, removed and changed traces/vias, found by comparing item ids and
//! content versions with the batch-start board) onto the board, if
//! * it only changed traces and vias,
//! * the items it removed or changed are unchanged on the board, and
//! * its changed area does not come close to the changes committed before in the batch, and
//! * the copied items have no clearance violations.
//!
//! Otherwise the connection is routed again, sequentially, on the board. The result only
//! depends on the batch-start boards, so it is the same for any thread timing (it does depend
//! on the batch size, i.e. the thread count).

use std::collections::{BTreeSet, HashMap};

use fr_geom::int_box::IntBox;

use crate::board::{Item, ItemKey, ItemKind, ItemSet, RoutingBoard};
use crate::datastructures::IdGenerator as _;
use crate::drc::clearance_violation::clearance_violation_count;
use crate::ids::{ItemId, NetNo};

/// Area around a connection that another connection of the same batch must not touch, and
/// around committed changes (in mm).
pub(super) const FOOTPRINT_MARGIN_MM: f64 = 1.5;

/// A box as (min x, min y, max x, max y) in board units.
pub(super) type Rect = (i64, i64, i64, i64);

pub(super) fn rect_of(b: &IntBox, margin: i64) -> Rect {
    (b.ll.x as i64 - margin, b.ll.y as i64 - margin, b.ur.x as i64 + margin, b.ur.y as i64 + margin)
}

pub(super) fn union(a: Option<Rect>, b: Rect) -> Rect {
    match a {
        None => b,
        Some(a) => (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3)),
    }
}

pub(super) fn overlaps(a: &Rect, b: &Rect) -> bool {
    a.0 <= b.2 && b.0 <= a.2 && a.1 <= b.3 && b.1 <= a.3
}

/// Board units per mm.
pub(super) fn units_per_mm(board: &RoutingBoard) -> f64 {
    board.communication.resolution.max(1) as f64 * 1000.0
}

/// Where routing `key` (net `net`) will most likely change the board: the item and the
/// nearest item of its net it is not connected to yet (planes excluded), plus a margin.
/// `None` if nothing is left to connect.
pub(super) fn connection_footprint(board: &RoutingBoard, key: ItemKey, net: NetNo, margin: i64) -> Option<Rect> {
    let unconnected: ItemSet = board.unconnected_set(key, net);
    if unconnected.is_empty() {
        return None;
    }
    let own = board.item(key).bounding_box(board);
    let (cx, cy) = ((own.ll.x as f64 + own.ur.x as f64) / 2.0, (own.ll.y as f64 + own.ur.y as f64) / 2.0);
    let mut nearest: Option<(f64, IntBox)> = None;
    for k in unconnected.iter() {
        let it = board.item(k);
        if it.is_conduction_area() {
            continue;
        }
        let b = it.bounding_box(board);
        let dx = (b.ll.x as f64 + b.ur.x as f64) / 2.0 - cx;
        let dy = (b.ll.y as f64 + b.ur.y as f64) / 2.0 - cy;
        let d = dx * dx + dy * dy;
        if nearest.as_ref().map_or(true, |(nd, _)| d < *nd) {
            nearest = Some((d, b));
        }
    }
    let mut r = rect_of(&own, margin);
    if let Some((_, b)) = nearest {
        r = union(Some(r), rect_of(&b, margin));
    }
    Some(r)
}

/// Item id -> (key, content version) of a board.
pub(super) type BoardIndex = HashMap<i32, (ItemKey, u64)>;

pub(super) fn index(board: &RoutingBoard) -> BoardIndex {
    board.get_items().into_iter().map(|k| (board.item(k).id().0, (k, board.items.version(k)))).collect()
}

/// The changes a worker made compared with the batch-start board.
pub(super) struct Changes {
    /// Items of the base that are gone or changed (id, base key, base version).
    removed: Vec<(i32, ItemKey, u64)>,
    /// Worker items to copy: (worker key, keep the id) - changed items keep their id,
    /// new items get a new one on the target board.
    inserted: Vec<(ItemKey, bool)>,
    /// Changed area (with margin).
    pub region: Option<Rect>,
    /// False if something other than traces or vias changed.
    pub copyable: bool,
    /// The routed net (diagnostics).
    pub net: NetNo,
    pub plane_net: bool,
}

pub(super) fn changes(base: &RoutingBoard, base_index: &BoardIndex, worker: &RoutingBoard, margin: i64, net: NetNo) -> Changes {
    let plane_net = base.rules.nets.get(net).map(|n| n.contains_plane()).unwrap_or(false);
    let mut c = Changes { removed: Vec::new(), inserted: Vec::new(), region: None, copyable: true, net, plane_net };
    let copyable_kind = |k: &ItemKind| matches!(k, ItemKind::Trace(_) | ItemKind::Via(_));
    let mut seen: BTreeSet<i32> = BTreeSet::new();
    for wk in worker.get_items() {
        let it = worker.item(wk);
        let id = it.id().0;
        seen.insert(id);
        match base_index.get(&id) {
            Some(&(bk, bv)) if bk == wk && worker.items.version(wk) == bv => {}
            Some(&(bk, bv)) => {
                if !copyable_kind(&it.kind) || !copyable_kind(&base.item(bk).kind) {
                    c.copyable = false;
                }
                c.removed.push((id, bk, bv));
                c.inserted.push((wk, true));
                c.region = Some(union(c.region, rect_of(&base.item(bk).bounding_box(base), margin)));
                c.region = Some(union(c.region, rect_of(&it.bounding_box(worker), margin)));
            }
            None => {
                if !copyable_kind(&it.kind) {
                    c.copyable = false;
                }
                c.inserted.push((wk, false));
                c.region = Some(union(c.region, rect_of(&it.bounding_box(worker), margin)));
            }
        }
    }
    for (&id, &(bk, bv)) in base_index.iter() {
        if !seen.contains(&id) {
            if !copyable_kind(&base.item(bk).kind) {
                c.copyable = false;
            }
            c.removed.push((id, bk, bv));
            c.region = Some(union(c.region, rect_of(&base.item(bk).bounding_box(base), margin)));
        }
    }
    // deterministic order (the index is a hash map)
    c.removed.sort_by_key(|r| r.0);
    c.inserted.sort_by_key(|&(k, _)| worker.item(k).id().0);
    c
}

/// A worker's changes with copies of the items involved, so that the worker board and the
/// batch-start board can be dropped as soon as the worker is done (two full boards per
/// connection in flight otherwise; a board copy is tens of MB on large boards).
pub(super) struct Condensed {
    pub changes: Changes,
    /// The batch-start items of `changes.removed` (same order).
    removed_items: Vec<Item>,
    /// The items to copy (`changes.inserted` order): the worker item, whether it keeps its id,
    /// and its clearance violation count on the worker board.
    inserted_items: Vec<(Item, bool, usize)>,
}

/// Takes what [`apply`] needs from `base` and `worker`.
pub(super) fn condense(changes: Changes, base: &RoutingBoard, worker: &RoutingBoard) -> Condensed {
    let removed_items = changes.removed.iter().map(|&(_, bk, _)| base.item(bk).clone()).collect();
    let inserted_items = changes
        .inserted
        .iter()
        .map(|&(wk, keep_id)| (worker.item(wk).clone(), keep_id, clearance_violation_count(worker, wk)))
        .collect();
    Condensed { changes, removed_items, inserted_items }
}

/// Why a result could not be copied (statistics).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Reject {
    Changed,
    Clearance,
}

/// Same traces/vias (geometry, nets, class, fixed state): an item that was re-inserted or
/// touched without being changed does not conflict.
fn same_content(a: &Item, b: &Item) -> bool {
    if a.net_numbers != b.net_numbers || a.clearance_class != b.clearance_class || a.fixed_state != b.fixed_state {
        return false;
    }
    match (&a.kind, &b.kind) {
        (ItemKind::Via(x), ItemKind::Via(y)) => x.padstack == y.padstack && x.center == y.center && x.attach_allowed == y.attach_allowed,
        (ItemKind::Trace(x), ItemKind::Trace(y)) => {
            x.layer == y.layer && x.half_width == y.half_width && x.polyline.lines[..] == y.polyline.lines[..]
        }
        _ => false,
    }
}

/// Copies the condensed changes onto a clone of `board`. `Err` if the items they replace were
/// changed on `board` in the meantime or the copied items violate a clearance.
pub(super) fn apply(cd: &Condensed, board: &RoutingBoard) -> Result<RoutingBoard, Reject> {
    let c = &cd.changes;
    for (&(id, bk, bv), base_item) in c.removed.iter().zip(&cd.removed_items) {
        match board.get_item(ItemId(id)) {
            Some(k) if k == bk && board.items.version(k) == bv => {}
            Some(k) if same_content(board.item(k), base_item) => {}
            other => {
                if log::log_enabled!(target: "fr_engine::pipeline::diag", log::Level::Debug) {
                    let bi = base_item;
                    let why = match other {
                        None => "gone".to_string(),
                        Some(k) if k != bk => "other key".to_string(),
                        Some(k) => {
                            let now = board.item(k);
                            format!("version {} -> {} same content {}", bv, board.items.version(k), format!("{:?}", now.kind) == format!("{:?}", bi.kind))
                        }
                    };
                    log::debug!(target: "fr_engine::pipeline::diag", "reject changed: {} {} of {} nets: {why}", if bi.is_trace() {"trace"} else if bi.is_via() {"via"} else {"other"}, if bi.net_numbers().contains(&c.net) { "own" } else { "other" }, if c.plane_net { "plane" } else { "signal" });
                }
                return Err(Reject::Changed);
            }
        }
    }
    let mut m = board.clone();
    let keys: Vec<ItemKey> = c.removed.iter().filter_map(|&(id, _, _)| m.get_item(ItemId(id))).collect();
    if !keys.is_empty() {
        m.remove_items(keys);
    }
    let mut new_keys = Vec::with_capacity(cd.inserted_items.len());
    for (item, keep_id, worker_violations) in &cd.inserted_items {
        let id = if *keep_id { item.id() } else { ItemId(m.communication.id_generator.new_id()) };
        new_keys.push((m.insert_item(item.copy_with_id(id)), *worker_violations));
    }
    // no violations beyond the ones the item already had on the worker board
    for (k, worker_violations) in new_keys {
        if clearance_violation_count(&m, k) > worker_violations {
            return Err(Reject::Clearance);
        }
    }
    Ok(m)
}
