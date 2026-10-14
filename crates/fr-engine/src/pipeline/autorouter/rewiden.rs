//! pcbkit W5 re-widen stage (`router.rewiden`, default off): after routing (recovery, chains and the strict-width
//! fallback included) and before the optimizer, every trace the router left narrower than its net-class width is put
//! back at the class width.
//!
//! The router narrows a track where the class width did not fit when it was routed (a neck-down at a pitch-limited
//! pad, the micro neck-down of a forced insert, the strict-width last resort and the necked retry of recovery and the
//! rip-up chains). Everything routed afterwards treats the narrow track as it is and packs its copper at the minimum
//! clearance against it, so a widen that only checks clearance (protocol `widen`, the toolkit's `route_necks.widen`)
//! cannot put the width back. This stage moves the neighbours instead, and only real pinch points (pads, fixed
//! copper, a corridor with no other way through) stay narrow.
//!
//! Per narrow trace (item id order; editable traces only: router wiring and shove-fixed, never `fix` wiring), each
//! step on a copy of the board, kept only if no connection opened and the board has no more clearance violations than
//! before (counted with `clearance_tolerance_um` 0, as KiCad counts):
//! 1. shove: remove the trace and insert its polyline at the class width with `insert_forced_trace_polyline`; the whole
//!    polyline must go in;
//! 2. a trace of several segments is split into one trace per segment (same geometry and width), so a neck at one pad
//!    does not hold the rest of the trace narrow; each segment gets step 1 on its own, then
//! 3. rip: the segment is put in at the class width as it is, the other-net router wiring it then violates (at most
//!    [`MAX_RIPPED`] traces and vias, never pins, fixed wiring or areas) is removed, and those nets' open connections
//!    are routed again without rip-up (so the widened segment stays) under the run's settings;
//! 4. if no segment widened, the trace stays as it was (unsplit).
//!
//! Counts only, no clock, no threads: the result is a function of the board and the settings.

use std::collections::BTreeSet;

use fr_geom::Polyline;
use fr_settings::RouterSettings;

use crate::autoroute::router::{route_connection_noisy, ConnectionRouterParams, RecoverTuning};
use crate::board::{ForcedTraceEnd, ItemKey, ItemSet, RoutingBoard, StopConnectionOption};
use crate::drc::{all_clearance_violations, clearance_violations};
use crate::ids::{ClearanceClassNo, FixedState, LayerNo, NetNo};

use super::{incomplete_count, BatchAutorouter, PipelineContext};

/// Shove recursion depths of the forced insert (the autorouter's defaults, `AutorouteControl`).
const MAX_SHOVE_TRACE_RECURSION: i32 = 20;
const MAX_SHOVE_VIA_RECURSION: i32 = 8;
const MAX_SPRING_OVER_RECURSION: i32 = 8;
/// Pull-tight accuracy handed to the forced insert (unused: no pull-tight, tidy width 0).
const PULL_TIGHT_ACCURACY: i32 = 500;
/// The most other-net items one segment may rip (more is a corridor, not a squeeze).
pub const MAX_RIPPED: usize = 4;
/// Sweeps over the ripped nets' open connections.
const REROUTE_SWEEPS: usize = 2;

/// What the stage did, for the log and the tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RewidenReport {
    /// Traces narrower than their class width at the start.
    pub narrow_traces: u32,
    /// Of those, traces widened whole by the shove.
    pub widened_whole: u32,
    /// Segments widened by the shove after a split.
    pub shoved_segments: u32,
    /// Segments widened by ripping and re-routing their neighbours.
    pub ripped_segments: u32,
    /// Segments still narrower than their class width at the end.
    pub narrow_segments_left: u32,
}

/// A narrow trace, copied out of the board.
struct Narrow {
    key: ItemKey,
    poly: Polyline,
    layer: LayerNo,
    half: i32,
    target: i32,
    nets: Vec<NetNo>,
    class: ClearanceClassNo,
    fixed: FixedState,
}

/// (open connections, clearance violations): what a widen must not make worse.
fn score(board: &RoutingBoard) -> (i32, usize) {
    (incomplete_count(board, None), all_clearance_violations(board).len())
}

fn no_worse(board: &RoutingBoard, base: (i32, usize)) -> bool {
    let after = score(board);
    after.0 <= base.0 && after.1 <= base.1
}

fn segment_count(p: &Polyline) -> usize {
    (p.corner_count() - 1).max(0) as usize
}

/// The segments of `p`, each the polyline of the three lines that bound it (its corners are exactly those of `p`,
/// rational ones included; fr-serve `widen` splits the same way).
fn segment_polylines(p: &Polyline) -> Vec<Polyline> {
    (0..segment_count(p)).map(|k| Polyline::from_lines(p.lines[k..k + 3].to_vec())).collect()
}

fn is_editable(fixed: FixedState) -> bool {
    matches!(fixed, FixedState::Unfixed | FixedState::ShoveFixed)
}

/// Editable traces narrower than their class width, in item id order.
fn narrow_traces(board: &RoutingBoard) -> Vec<Narrow> {
    let mut out: Vec<(i32, Narrow)> = Vec::new();
    for key in board.get_traces() {
        let item = board.item(key);
        let Some(t) = item.as_trace() else { continue };
        let fixed = item.fixed_state();
        let Some(&net) = item.net_numbers().first() else { continue };
        if !is_editable(fixed) || net < 1 {
            continue;
        }
        let target = board.rules.get_trace_half_width(net, t.layer());
        if t.half_width() >= target {
            continue;
        }
        let n = Narrow {
            key,
            poly: t.polyline().clone(),
            layer: t.layer(),
            half: t.half_width(),
            target,
            nets: item.net_numbers().to_vec(),
            class: item.clearance_class(),
            fixed,
        };
        out.push((item.id().0, n));
    }
    out.sort_by_key(|(id, _)| *id);
    out.into_iter().map(|(_, n)| n).collect()
}

/// True while the trace `n` was copied from is still on the board unchanged.
fn still_there(board: &RoutingBoard, n: &Narrow) -> bool {
    board.try_item(n.key).and_then(|i| i.as_trace()).is_some_and(|t| t.half_width() == n.half && t.polyline().corners() == n.poly.corners())
}

/// Step 1 on a copy: the trace `key` replaced by `poly` at the class width, shoving. The copy if it is kept.
fn shove_wide(board: &RoutingBoard, key: ItemKey, poly: &Polyline, n: &Narrow, base: (i32, usize)) -> Option<RoutingBoard> {
    let mut copy = board.clone();
    copy.remove_item(key);
    let end = copy.insert_forced_trace_polyline(
        poly,
        n.target,
        n.layer,
        &n.nets,
        n.class,
        MAX_SHOVE_TRACE_RECURSION,
        MAX_SHOVE_VIA_RECURSION,
        MAX_SPRING_OVER_RECURSION,
        0,
        PULL_TIGHT_ACCURACY,
        true,
        None,
    );
    (matches!(end, ForcedTraceEnd::To(_)) && no_worse(&copy, base)).then_some(copy)
}

/// The other-net items the trace `key` violates, if all of them may be ripped (unfixed router traces and vias of one
/// net) and there are at most [`MAX_RIPPED`]; `None` otherwise.
fn rippable_victims(board: &RoutingBoard, key: ItemKey) -> Option<Vec<ItemKey>> {
    let mut victims: BTreeSet<(i32, ItemKey)> = BTreeSet::new();
    for v in clearance_violations(board, key) {
        let other = if v.first_item == key { v.second_item } else { v.first_item };
        let item = board.item(other);
        if !(item.is_trace() || item.is_via()) || item.fixed_state() != FixedState::Unfixed || item.net_count() != 1 {
            return None;
        }
        victims.insert((item.id().0, other));
    }
    (victims.len() <= MAX_RIPPED).then(|| victims.into_iter().map(|(_, k)| k).collect())
}

impl BatchAutorouter {
    /// Runs the stage on `board` (see the module doc). Returns what it did.
    pub fn rewiden(&mut self, board: &mut RoutingBoard, settings: &RouterSettings, ctx: &PipelineContext) -> RewidenReport {
        let tolerance = std::mem::replace(&mut board.rules_mut().clearance_tolerance_um, 0.0);
        let params = self.params();
        let mut report = RewidenReport::default();
        let candidates = narrow_traces(board);
        report.narrow_traces = candidates.len() as u32;
        let mut base = score(board);
        for n in &candidates {
            if ctx.stop.is_stop_requested() || !still_there(board, n) {
                // a trace an earlier widen replaced or shoved is not judged again (counted as left narrow)
                report.narrow_segments_left += segment_count(&n.poly) as u32;
                continue;
            }
            if let Some(copy) = shove_wide(board, n.key, &n.poly, n, base) {
                *board = copy;
                report.widened_whole += 1;
                base = score(board);
                continue;
            }
            let (shoved, ripped, left) = self.widen_segments(board, n, base, settings, &params, ctx);
            report.shoved_segments += shoved;
            report.ripped_segments += ripped;
            report.narrow_segments_left += left;
            if shoved + ripped > 0 {
                base = score(board);
            }
        }
        board.rules_mut().clearance_tolerance_um = tolerance;
        log::info!(
            "Re-widen: {} narrow traces, {} widened whole, {} segments shoved, {} segments widened by re-routing neighbours, {} segments left narrow.",
            report.narrow_traces,
            report.widened_whole,
            report.shoved_segments,
            report.ripped_segments,
            report.narrow_segments_left
        );
        report
    }

    /// Steps 2-4 for one trace the whole-trace shove could not widen. Returns (shoved, ripped, left narrow) segments.
    fn widen_segments(
        &self,
        board: &mut RoutingBoard,
        n: &Narrow,
        base: (i32, usize),
        settings: &RouterSettings,
        params: &ConnectionRouterParams,
        ctx: &PipelineContext,
    ) -> (u32, u32, u32) {
        let segments = segment_polylines(&n.poly);
        let before_split = board.clone();
        let mut keys = Vec::with_capacity(segments.len());
        if segments.len() <= 1 {
            keys.push((n.key, n.poly.clone()));
        } else {
            board.remove_item(n.key);
            for s in segments {
                match board.insert_trace_without_cleaning(s.clone(), n.layer, n.half, &n.nets, n.class, n.fixed) {
                    Some(k) => keys.push((k, s)),
                    None => {
                        *board = before_split;
                        return (0, 0, segment_count(&n.poly) as u32);
                    }
                }
            }
        }
        let (mut shoved, mut ripped) = (0u32, 0u32);
        let mut base = base;
        for (k, s) in &keys {
            if ctx.stop.is_stop_requested() || board.try_item(*k).is_none() {
                continue;
            }
            if let Some(copy) = shove_wide(board, *k, s, n, base) {
                *board = copy;
                shoved += 1;
            } else if let Some(trial) = self.rip_wide(board, *k, s, n, base, settings, params, ctx) {
                *board = trial;
                ripped += 1;
            } else {
                continue;
            }
            base = score(board);
        }
        if shoved + ripped == 0 {
            *board = before_split;
        }
        (shoved, ripped, keys.len() as u32 - shoved - ripped)
    }

    /// Step 3 on a copy: the segment `key` at the class width as it is, its other-net victims ripped and their nets'
    /// open connections routed again without rip-up. The copy if it is kept.
    #[allow(clippy::too_many_arguments)]
    fn rip_wide(
        &self,
        board: &RoutingBoard,
        key: ItemKey,
        poly: &Polyline,
        n: &Narrow,
        base: (i32, usize),
        settings: &RouterSettings,
        params: &ConnectionRouterParams,
        ctx: &PipelineContext,
    ) -> Option<RoutingBoard> {
        let mut trial = board.clone_for_worker();
        trial.remove_item(key);
        let wide = trial.insert_trace_without_cleaning(poly.clone(), n.layer, n.target, &n.nets, n.class, n.fixed)?;
        let victims = rippable_victims(&trial, wide)?;
        if victims.is_empty() {
            return no_worse(&trial, base).then_some(trial);
        }
        let nets: BTreeSet<NetNo> = victims.iter().map(|&v| trial.item(v).net_number(0)).collect();
        trial.remove_items(victims);
        let no_rip = RecoverTuning { noise_pass_no: 1, rip_zone_vias: false, rip_protect_above: Some(i32::MIN), reverse: false };
        for _sweep in 0..REROUTE_SWEEPS {
            let mut open: Vec<ItemKey> =
                Self::get_autoroute_items(&trial).into_iter().filter(|&k| trial.item(k).net_count() == 1 && nets.contains(&trial.item(k).net_number(0))).collect();
            if open.is_empty() || ctx.stop.is_stop_requested() {
                break;
            }
            open.sort_by_key(|&k| (trial.item(k).net_number(0), trial.item(k).id().0));
            for id in open.iter().map(|&k| trial.item(k).id()).collect::<Vec<_>>() {
                let Some(k) = trial.get_item(id) else { continue };
                let net = trial.item(k).net_number(0);
                trial.start_marking_changed_area();
                let mut r = ItemSet::new();
                route_connection_noisy(&mut trial, settings, params, k, net, &mut r, None, 1, Some(&no_rip), Some(&ctx.stop));
            }
        }
        let option = if self.remove_unconnected_vias { StopConnectionOption::None } else { StopConnectionOption::FanoutVia };
        // tails only, no pull-tight of the changed area (it would replace the widened segment)
        trial.remove_trace_tails(-1, option);
        let wide_kept = trial.try_item(wide).and_then(|i| i.as_trace()).is_some_and(|t| t.half_width() == n.target);
        (wide_kept && no_worse(&trial, base)).then_some(trial)
    }
}

#[cfg(test)]
mod tests {
    use super::segment_polylines;
    use fr_geom::{Line, Polyline};

    #[test]
    fn segments_keep_the_corners() {
        let p = Polyline::from_lines(vec![
            Line::new_ints(0, 0, 0, 1),
            Line::new_ints(0, 0, 1, 0),
            Line::new_ints(5, 0, 5, 1),
            Line::new_ints(0, 4, 1, 4),
        ]);
        let corners = p.corners();
        let segs = segment_polylines(&p);
        assert_eq!(segs.len(), 2);
        for (k, s) in segs.iter().enumerate() {
            assert_eq!(s.corners(), vec![corners[k].clone(), corners[k + 1].clone()]);
        }
    }
}
