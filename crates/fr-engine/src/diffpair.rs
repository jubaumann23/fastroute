//! fastroute: differential pair coupling after routing.
//!
//! Freerouting routes the two nets of a pair independently. This pass makes the N net run
//! parallel to the P net at the pair spacing: the P traces are offset sideways by
//! `d = half width P + half width N + gap`, the parts of the offset that have room (no
//! clearance violation with a slightly wider probe) become N traces, and the maze router
//! connects them to the N pins and to each other. The old N traces are removed first.
//!
//! ```text
//!   P  ●━━━━━━━━━━━━━━━━━━━━━━━━┓
//!   N  ●─┐━━━━━━━━━━━━━━━━━━━━━┓┃        ━ offset of P (kept where it has room)
//!        └─ (maze router)       ┃┃
//! ```
//!
//! The whole change is made on a clone of the board and kept only if every N connection is
//! made, no other net loses a connection, the new N items have no clearance violation and
//! the coupled length grows. Both directions are tried (N following P, P following N).

use std::collections::HashSet;

use fr_geom::{FloatPoint, IntPoint, Line, Polyline};
use fr_settings::RouterSettings;

use crate::autoroute::control::AutorouteControl;
use crate::board::{AutorouteAttemptState, ItemKey, RoutingBoard, StopConnectionOption};
use crate::datastructures::StopToken;
use crate::drc::clearance_violation::clearance_violation_count;
use crate::ids::{FixedState, LayerNo, NetNo};
use crate::pipeline::stats::incomplete_count;

/// Extra half width of the clearance probe, micrometres (as in length tuning).
const SAFETY_MARGIN_UM: f64 = 5.0;
/// Length of the pieces of the offset path that are checked one by one, mm.
const PIECE_MM: f64 = 0.2;
/// Shortest coupled run that is kept, mm.
const MIN_RUN_MM: f64 = 0.6;

/// One pair to couple.
#[derive(Clone, Debug)]
pub struct DiffPair {
    pub p: String,
    pub n: String,
    /// Copper gap between the two traces, mm (`None`: the clearance between them).
    pub gap_mm: Option<f64>,
    /// Gaps for single layers (layer name, mm), e.g. from controlled-impedance rules.
    pub layer_gaps_mm: Vec<(String, f64)>,
}

/// Result for one pair.
#[derive(Clone, Debug)]
pub struct PairResult {
    pub p: String,
    pub n: String,
    /// Length of P, mm.
    pub p_length_mm: f64,
    /// N length running parallel to P at the pair spacing, before and after, mm.
    pub coupled_before_mm: f64,
    pub coupled_after_mm: f64,
    /// Length difference N - P after, mm.
    pub skew_mm: f64,
    pub message: String,
}

fn units_per_mm(board: &RoutingBoard) -> f64 {
    board.communication.resolution.max(1) as f64 * 1000.0
}

fn net_by_name(board: &RoutingBoard, name: &str) -> Option<NetNo> {
    (1..=board.rules.nets.max_net_number()).find(|&n| board.rules.nets.get(n).map(|x| x.name == name).unwrap_or(false))
}

fn net_traces(board: &RoutingBoard, net: NetNo) -> Vec<ItemKey> {
    board.get_connectable_items(net).into_iter().filter(|&k| board.item(k).is_trace()).collect()
}

fn net_length(board: &RoutingBoard, net: NetNo) -> f64 {
    crate::tuning::net_length(board, net)
}

/// Couples all pairs; returns what was done.
pub fn couple_pairs(board: &mut RoutingBoard, pairs: &[DiffPair], settings: &RouterSettings, stop: &StopToken) -> Vec<PairResult> {
    let upm = units_per_mm(board);
    let mut out = Vec::new();
    for pair in pairs {
        if stop.is_stop_requested() {
            break;
        }
        let (Some(p), Some(n)) = (net_by_name(board, &pair.p), net_by_name(board, &pair.n)) else {
            log::warn!(target: "fr_engine::pipeline", "diff pair {}/{}: net not found", pair.p, pair.n);
            continue;
        };
        let geo = PairGeometry::new(board, p, n, pair, settings);
        let before = coupled_length(board, p, n, &geo);
        // all pairs count: routing this one may shove another one away from its partner
        let score_before = total_score(board, pairs, settings);
        let mut result = PairResult {
            p: pair.p.clone(),
            n: pair.n.clone(),
            p_length_mm: net_length(board, p) / upm,
            coupled_before_mm: before / upm,
            coupled_after_mm: before / upm,
            skew_mm: 0.0,
            message: String::new(),
        };
        // both nets are routed once more with N swapped with P: a pair whose P is the
        // detour-rich one couples better the other way round
        let mut best: Option<(RoutingBoard, f64)> = None;
        let mut reason = String::new();
        for (lead, follow) in [(p, n), (n, p)] {
            let geo = PairGeometry::new(board, lead, follow, pair, settings);
            match couple(board, lead, follow, &geo, settings, stop) {
                Ok(next) => {
                    let got = total_score(&next, pairs, settings);
                    if best.as_ref().map(|b| got > b.1).unwrap_or(true) {
                        best = Some((next, got));
                    }
                }
                Err(e) => {
                    if reason.is_empty() {
                        reason = e;
                    }
                }
            }
        }
        match best {
            Some((next, got)) if got > score_before + 0.5 * upm => {
                *board = next;
                result.coupled_after_mm = coupled_length(board, p, n, &geo) / upm;
            }
            Some(_) => result.message = "no improvement".into(),
            None => result.message = reason,
        }
        result.skew_mm = (net_length(board, n) - net_length(board, p)) / upm;
        out.push(result);
    }
    out
}

/// Widths, clearance class and spacing of the follower net per layer.
struct PairGeometry {
    /// Half width of the follower's traces per layer.
    half_width: Vec<i32>,
    clearance_class: crate::ids::ClearanceClassNo,
    /// Wanted copper gap per layer (board units), if given.
    gap: Vec<Option<f64>>,
    /// Minimum copper gap per layer (the clearance between the two nets).
    min_gap: Vec<f64>,
    /// Probe margin, board units.
    margin: f64,
}

impl PairGeometry {
    fn new(board: &RoutingBoard, lead: NetNo, follow: NetNo, pair: &DiffPair, settings: &RouterSettings) -> Self {
        let upm = units_per_mm(board);
        let cf = AutorouteControl::new_default(board, follow, settings);
        let cl = AutorouteControl::new_default(board, lead, settings);
        let layers = cf.trace_half_width.len();
        let min_gap = (0..layers)
            .map(|l| board.rules.clearance_matrix.get_value(cl.trace_clearance_class, cf.trace_clearance_class, l as LayerNo, false) as f64)
            .collect();
        let margin = SAFETY_MARGIN_UM * board.communication.resolution.max(1) as f64;
        let mut gap: Vec<Option<f64>> = vec![pair.gap_mm.map(|g| g * upm); layers];
        for (name, g) in &pair.layer_gaps_mm {
            let no = board.layer_structure.get_no(name);
            if no >= 0 && (no as usize) < layers {
                gap[no as usize] = Some(g * upm);
            } else {
                log::warn!(target: "fr_engine::pipeline", "diff pair {}/{}: unknown layer '{name}'", pair.p, pair.n);
            }
        }
        PairGeometry { half_width: cf.trace_half_width.clone(), clearance_class: cf.trace_clearance_class, gap, min_gap, margin }
    }

    /// Centre distance of a follower trace next to a lead trace of half width `lead_hw`.
    fn distance(&self, layer: LayerNo, lead_hw: i32) -> f64 {
        let l = layer as usize;
        // the minimum plus twice the probe margin: the probe must not touch the lead
        let gap = self.gap[l].unwrap_or(0.0).max(self.min_gap[l] + 2.0 * self.margin);
        lead_hw as f64 + self.half_width[l] as f64 + gap
    }
}

/// Weight of the intra-pair skew against the coupled length.
const SKEW_WEIGHT: f64 = 0.5;

/// Quality of a pair: coupled length minus a share of the length difference (a coupled
/// route with a long detour at one end is worse than it looks).
fn pair_score(board: &RoutingBoard, p: NetNo, n: NetNo, geo: &PairGeometry) -> f64 {
    coupled_length(board, p, n, geo) - SKEW_WEIGHT * (net_length(board, p) - net_length(board, n)).abs()
}

/// Sum of [`pair_score`] over all pairs.
fn total_score(board: &RoutingBoard, pairs: &[DiffPair], settings: &RouterSettings) -> f64 {
    pairs
        .iter()
        .filter_map(|pair| {
            let (p, n) = (net_by_name(board, &pair.p)?, net_by_name(board, &pair.n)?);
            Some(pair_score(board, p, n, &PairGeometry::new(board, p, n, pair, settings)))
        })
        .sum()
}

/// Length of N traces running parallel to a P trace on the same layer at the pair distance.
fn coupled_length(board: &RoutingBoard, p: NetNo, n: NetNo, geo: &PairGeometry) -> f64 {
    let upm = units_per_mm(board);
    let step = 0.1 * upm;
    let p_traces: Vec<(LayerNo, i32, Polyline)> = net_traces(board, p)
        .into_iter()
        .filter_map(|k| board.item(k).as_trace().map(|t| (t.layer(), t.half_width(), t.polyline().clone())))
        .collect();
    let mut total = 0.0;
    for k in net_traces(board, n) {
        let Some(t) = board.item(k).as_trace() else { continue };
        let corners = t.polyline().corner_approx_arr();
        for w in corners.windows(2) {
            let len = w[0].distance(&w[1]);
            let samples = (len / step).ceil().max(1.0) as usize;
            for s in 0..samples {
                let f = (s as f64 + 0.5) / samples as f64;
                let q = FloatPoint::new(w[0].x + (w[1].x - w[0].x) * f, w[0].y + (w[1].y - w[0].y) * f);
                let coupled = p_traces.iter().any(|(layer, hw, pl)| {
                    if *layer != t.layer() {
                        return false;
                    }
                    let d = geo.distance(*layer, *hw);
                    (pl.distance(&q) - d).abs() <= 0.1 * d + 0.002 * upm
                });
                if coupled {
                    total += len / samples as f64;
                }
            }
        }
    }
    total
}

/// A P trace in walking order (from the start pin outwards).
struct Oriented {
    layer: LayerNo,
    half_width: i32,
    polyline: Polyline,
}

/// The lead net's traces, oriented along the paths from its first pin (depth first), so that
/// "left" is the same side of the pair along a path.
fn oriented_traces(board: &RoutingBoard, lead: NetNo, follow: NetNo) -> (Vec<Oriented>, Option<i32>) {
    let upm = units_per_mm(board);
    let tol = 0.002 * upm;
    let traces = net_traces(board, lead);
    let pins: Vec<FloatPoint> = board
        .get_connectable_items(lead)
        .into_iter()
        .filter(|&k| board.item(k).is_pin())
        .map(|k| board.item(k).center(board).to_float())
        .collect();
    let follow_pins: Vec<FloatPoint> = board
        .get_connectable_items(follow)
        .into_iter()
        .filter(|&k| board.item(k).is_pin())
        .map(|k| board.item(k).center(board).to_float())
        .collect();
    // start at the lead pin closest to a follower pin: the pair's two pads sit side by side
    let start = pins
        .iter()
        .min_by(|a, b| {
            let da = follow_pins.iter().map(|f| f.distance(a)).fold(f64::MAX, f64::min);
            let db = follow_pins.iter().map(|f| f.distance(b)).fold(f64::MAX, f64::min);
            da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
        })
        .cloned();
    let mut used: HashSet<ItemKey> = HashSet::new();
    let mut out: Vec<Oriented> = Vec::new();
    let mut side: Option<i32> = None;
    let mut frontier: Vec<FloatPoint> = start.iter().cloned().collect();
    // also start from the other pins (branches not reachable from the first one)
    let mut other_starts: Vec<FloatPoint> = pins.clone();
    loop {
        while let Some(at) = frontier.pop() {
            for &k in &traces {
                if used.contains(&k) {
                    continue;
                }
                let t = board.item(k).as_trace().unwrap();
                let (a, b) = (t.first_corner().to_float(), t.last_corner().to_float());
                let polyline = if a.distance(&at) <= tol {
                    t.polyline().clone()
                } else if b.distance(&at) <= tol {
                    t.polyline().reverse()
                } else {
                    continue;
                };
                used.insert(k);
                if side.is_none() {
                    side = seed_side(&polyline, &follow_pins);
                }
                frontier.push(polyline.last_corner().to_float());
                out.push(Oriented { layer: t.layer(), half_width: t.half_width(), polyline });
            }
        }
        match other_starts.pop() {
            Some(s) => frontier.push(s),
            None => break,
        }
    }
    (merge_chains(out), side)
}

/// Joins traces that continue each other on the same layer (one trace per segment in
/// imported boards) so that the offset keeps its corners.
fn merge_chains(mut paths: Vec<Oriented>) -> Vec<Oriented> {
    let degree = |paths: &[Oriented], p: &fr_geom::Point| {
        paths.iter().filter(|o| o.polyline.first_corner() == *p || o.polyline.last_corner() == *p).count()
    };
    loop {
        let mut merged = false;
        'search: for i in 0..paths.len() {
            let end = paths[i].polyline.last_corner();
            if degree(&paths, &end) != 2 {
                continue;
            }
            for j in 0..paths.len() {
                if i == j || paths[j].layer != paths[i].layer || paths[j].half_width != paths[i].half_width {
                    continue;
                }
                if paths[j].polyline.first_corner() == end {
                    let combined = paths[i].polyline.combine(Some(&paths[j].polyline));
                    if combined.lines.len() < 3 || combined.first_corner() != paths[i].polyline.first_corner() {
                        continue;
                    }
                    paths[i].polyline = combined;
                    paths.remove(j);
                    merged = true;
                    break 'search;
                }
            }
        }
        if !merged {
            return paths;
        }
    }
}

/// The side (+1 left, -1 right) of the first segment on which the nearest follower pin lies.
fn seed_side(polyline: &Polyline, follow_pins: &[FloatPoint]) -> Option<i32> {
    let c = polyline.corner_approx_arr();
    if c.len() < 2 {
        return None;
    }
    let f = follow_pins.iter().min_by(|a, b| a.distance(&c[0]).partial_cmp(&b.distance(&c[0])).unwrap_or(std::cmp::Ordering::Equal))?;
    let (dx, dy) = (c[1].x - c[0].x, c[1].y - c[0].y);
    let cross = dx * (f.y - c[0].y) - dy * (f.x - c[0].x);
    if cross.abs() < 1e-9 {
        None
    } else {
        Some(if cross > 0.0 { 1 } else { -1 })
    }
}

/// The polyline offset by `dist` to the left (negative: right); `None` if a segment collapses
/// or turns around (a segment shorter than the offset at an inner corner).
fn offset_polyline(polyline: &Polyline, dist: f64) -> Option<Polyline> {
    let lines = &polyline.lines;
    let m = lines.len();
    if m < 3 {
        return None;
    }
    let mut new_lines: Vec<Line> = Vec::with_capacity(m);
    new_lines.push(lines[0].clone());
    for l in &lines[1..m - 1] {
        new_lines.push(l.translate(dist));
    }
    new_lines.push(lines[m - 1].clone());
    let off = Polyline::from_lines(new_lines);
    if off.lines.len() != m {
        return None;
    }
    let a = polyline.corner_approx_arr();
    let b = off.corner_approx_arr();
    for i in 0..a.len() - 1 {
        let (ax, ay) = (a[i + 1].x - a[i].x, a[i + 1].y - a[i].y);
        let (bx, by) = (b[i + 1].x - b[i].x, b[i + 1].y - b[i].y);
        if ax * bx + ay * by <= 0.0 {
            return None;
        }
    }
    Some(off)
}

/// A line perpendicular to `line` through the point closest to `q`.
fn perpendicular_through(line: &Line, q: &FloatPoint) -> Line {
    let dir = line.direction().as_int();
    let p = IntPoint::new(q.x.round() as i32, q.y.round() as i32);
    Line::from_int_points(p, IntPoint::new(p.x - dir.y, p.y + dir.x))
}

/// The part of `off` from segment `s0` at fraction `f0` to segment `s1` at fraction `f1`.
fn section(off: &Polyline, corners: &[FloatPoint], s0: usize, f0: f64, s1: usize, f1: f64) -> Option<Polyline> {
    let at = |s: usize, f: f64| FloatPoint::new(corners[s].x + (corners[s + 1].x - corners[s].x) * f, corners[s].y + (corners[s + 1].y - corners[s].y) * f);
    let mut lines = vec![perpendicular_through(&off.lines[s0 + 1], &at(s0, f0))];
    lines.extend_from_slice(&off.lines[s0 + 1..=s1 + 1]);
    lines.push(perpendicular_through(&off.lines[s1 + 1], &at(s1, f1)));
    let p = Polyline::from_lines(lines);
    if p.lines.len() < 3 {
        None
    } else {
        Some(p)
    }
}

/// Makes the follower run along the lead on a clone of the board.
fn couple(board: &RoutingBoard, lead: NetNo, follow: NetNo, geo: &PairGeometry, settings: &RouterSettings, stop: &StopToken) -> Result<RoutingBoard, String> {
    let upm = units_per_mm(board);
    let (paths, seed) = oriented_traces(board, lead, follow);
    log::debug!(target: "fr_engine::pipeline::diag", "diff pair: net {lead} -> {follow}: {} traces, seed side {seed:?}", paths.len());
    if paths.is_empty() {
        return Err("not routed".into());
    }
    let incomplete_before = incomplete_count(board, None);
    let follow_groups_before = pin_groups(board, follow);
    log::debug!(target: "fr_engine::pipeline::diag", "diff pair: follower {follow}: {follow_groups_before} pin groups, {} groups before", follow_components(board, follow));
    if log::log_enabled!(target: "fr_engine::pipeline::diag", log::Level::Debug) {
        for k in board.get_connectable_items(follow) {
            let it = board.item(k);
            if it.is_pin() {
                let set = board.connected_set(k, follow, false);
                log::debug!(target: "fr_engine::pipeline::diag", "  pin at {:?}: group of {} items", it.center(board).to_float(), set.len());
            }
        }
    }

    let mut best: Option<RoutingBoard> = None;
    let mut last_err = String::from("no room beside the pair");
    let sides: Vec<i32> = match seed {
        Some(s) => vec![s, -s],
        None => vec![1, -1],
    };
    for side in sides {
        if stop.is_stop_requested() {
            break;
        }
        let mut next = board.clone();
        // the follower's own routing goes; locked (SYSTEM_FIXED) copper stays
        let old: Vec<ItemKey> = next
            .get_connectable_items(follow)
            .into_iter()
            .filter(|&k| {
                let it = next.item(k);
                (it.is_trace() || it.is_via()) && it.fixed_state() < FixedState::SystemFixed && it.net_count() == 1
            })
            .collect();
        for &k in &old {
            if next.item(k).is_user_fixed() {
                next.items.get_mut(k).set_fixed_state(FixedState::Unfixed);
            }
        }
        next.remove_items(old);

        let mut runs = coupled_runs(&next, &paths, geo, side, follow, upm);
        if runs.is_empty() {
            continue;
        }
        // longest runs first; if the router cannot join all of them, fewer (short runs in
        // crowded spots are the hard ones)
        runs.sort_by(|a, b| b.1.length_approx().partial_cmp(&a.1.length_approx()).unwrap_or(std::cmp::Ordering::Equal));
        let mut counts: Vec<usize> = vec![runs.len(), runs.len().div_ceil(2), 2, 1];
        counts.retain(|&c| c >= 1 && c <= runs.len());
        counts.dedup();
        let base = next;
        for count in counts {
            if stop.is_stop_requested() {
                break;
            }
            let mut next = base.clone();
            for (layer, polyline) in &runs[..count] {
                next.insert_trace_without_cleaning(polyline.clone(), *layer, geo.half_width[*layer as usize], &[follow], geo.clearance_class, FixedState::Unfixed);
            }
            if let Err(e) = connect_net(&mut next, follow, follow_groups_before, settings, stop) {
                last_err = e;
                continue;
            }
            next.remove_trace_tails(follow, StopConnectionOption::None);
            if pin_groups(&next, follow) > follow_groups_before {
                last_err = "follower not connected".into();
                continue;
            }
            if incomplete_count(&next, None) > incomplete_before {
                last_err = "other connections lost".into();
                continue;
            }
            let violating = next.get_connectable_items(follow).into_iter().any(|k| {
                let it = next.item(k);
                (it.is_trace() || it.is_via()) && clearance_violation_count(&next, k) > 0
            });
            if violating {
                last_err = "clearance violation".into();
                continue;
            }
            log::debug!(target: "fr_engine::pipeline::diag", "diff pair: side {side}, {count} of {} runs joined, {} pin groups, {} groups", runs.len(), pin_groups(&next, follow), follow_components(&next, follow));
            let better = match &best {
                Some(b) => pair_score(&next, lead, follow, geo) > pair_score(b, lead, follow, geo),
                None => true,
            };
            if better {
                best = Some(next);
            }
        }
    }
    best.ok_or(last_err)
}

/// The parts of the offset paths that have room, as (layer, polyline) of the follower.
fn coupled_runs(board: &RoutingBoard, paths: &[Oriented], geo: &PairGeometry, side: i32, follow: NetNo, upm: f64) -> Vec<(LayerNo, Polyline)> {
    let margin = (SAFETY_MARGIN_UM * board.communication.resolution.max(1) as f64).round() as i32;
    let mut probe = board.clone();
    let mut runs = Vec::new();
    for path in paths {
        let dist = geo.distance(path.layer, path.half_width) * side as f64;
        for off in offset_parts(&path.polyline, dist) {
        let corners = off.corner_approx_arr();
        let hw = geo.half_width[path.layer as usize];
        // pieces in path order: (segment, from, to, ok)
        let mut pieces: Vec<(usize, f64, f64, bool)> = Vec::new();
        for s in 0..corners.len() - 1 {
            let len = corners[s].distance(&corners[s + 1]);
            let count = (len / (PIECE_MM * upm)).ceil().max(1.0) as usize;
            for i in 0..count {
                let (f0, f1) = (i as f64 / count as f64, (i + 1) as f64 / count as f64);
                let ok = match section(&off, &corners, s, f0, s, f1) {
                    Some(pl) => match probe.insert_trace_without_cleaning(pl, path.layer, hw + margin, &[follow], geo.clearance_class, FixedState::Unfixed) {
                        Some(k) => {
                            let v = crate::drc::clearance_violation::clearance_violations(&probe, k);
                            if !v.is_empty() && log::log_enabled!(target: "fr_engine::pipeline::diag", log::Level::Debug) {
                                let o = probe.item(v[0].second_item);
                                log::debug!(target: "fr_engine::pipeline::diag", "piece blocked by nets {:?} pin {} trace {} (exp {:.0} act {:.0})", o.net_numbers(), o.is_pin(), o.is_trace(), v[0].expected_clearance, v[0].actual_clearance);
                            }
                            let ok = v.is_empty();
                            probe.remove_item(k);
                            ok
                        }
                        None => {
                            log::debug!(target: "fr_engine::pipeline::diag", "piece not inserted");
                            false
                        }
                    },
                    None => false,
                };
                pieces.push((s, f0, f1, ok));
            }
        }
        log::debug!(target: "fr_engine::pipeline::diag", "diff pair: layer {} side {side}: {} of {} pieces free", path.layer, pieces.iter().filter(|p| p.3).count(), pieces.len());
        // maximal runs of good pieces
        let mut i = 0;
        while i < pieces.len() {
            if !pieces[i].3 {
                i += 1;
                continue;
            }
            let mut j = i;
            while j + 1 < pieces.len() && pieces[j + 1].3 {
                j += 1;
            }
            let (s0, f0) = (pieces[i].0, pieces[i].1);
            let (s1, f1) = (pieces[j].0, pieces[j].2);
            if let Some(pl) = section(&off, &corners, s0, f0, s1, f1) {
                if pl.length_approx() >= MIN_RUN_MM * upm {
                    runs.push((path.layer, pl));
                }
            }
            i = j + 1;
        }
        }
    }
    runs
}

/// The offset of a polyline in parts: where a short segment collapses at an inner corner the
/// polyline is split (its segments' neighbouring lines serve as end lines of the parts).
fn offset_parts(polyline: &Polyline, dist: f64) -> Vec<Polyline> {
    if let Some(off) = offset_polyline(polyline, dist) {
        return vec![off];
    }
    let lines = &polyline.lines;
    let segments = lines.len().saturating_sub(2);
    // segments i..=j with their neighbour lines as end lines
    let sub = |i: usize, j: usize| Polyline::from_lines(lines[i..=j + 2].to_vec());
    let mut parts = Vec::new();
    let mut a = 0;
    while a < segments {
        let mut last: Option<(usize, Polyline)> = None;
        let mut b = a;
        while b < segments {
            let part = sub(a, b);
            match (part.lines.len() == b - a + 3).then(|| offset_polyline(&part, dist)).flatten() {
                Some(off) => last = Some((b, off)),
                None => break,
            }
            b += 1;
        }
        match last {
            Some((end, off)) => {
                parts.push(off);
                a = end + 1;
            }
            None => a += 1,
        }
    }
    log::debug!(target: "fr_engine::pipeline::diag", "diff pair: offset of a {}-corner trace split into {} parts", polyline.corner_count(), parts.len());
    parts
}

/// Number of connected groups of the net's items that contain a pin.
fn pin_groups(board: &RoutingBoard, net: NetNo) -> usize {
    let mut seen: HashSet<ItemKey> = HashSet::new();
    let mut count = 0;
    for k in board.get_connectable_items(net) {
        if !board.item(k).is_pin() || seen.contains(&k) {
            continue;
        }
        count += 1;
        for c in board.connected_set(k, net, false).iter() {
            seen.insert(c);
        }
    }
    count
}

/// Number of connected groups of the net's items.
fn follow_components(board: &RoutingBoard, net: NetNo) -> usize {
    let items: Vec<ItemKey> = board.get_connectable_items(net);
    let mut seen: HashSet<ItemKey> = HashSet::new();
    let mut count = 0;
    for &k in &items {
        if seen.contains(&k) {
            continue;
        }
        count += 1;
        for c in board.connected_set(k, net, false).iter() {
            seen.insert(c);
        }
    }
    count
}

/// Routes the net's items together with the maze router (no rip-up of other nets).
/// Stops at `target` groups (the net's state before, which may be incomplete).
fn connect_net(board: &mut RoutingBoard, net: NetNo, target: usize, settings: &RouterSettings, stop: &StopToken) -> Result<(), String> {
    let mut failed: HashSet<ItemKey> = HashSet::new();
    for _ in 0..64 {
        if stop.is_stop_requested() {
            return Err("stopped".into());
        }
        let items = board.get_connectable_items(net);
        if follow_components(board, net) <= target.max(1) {
            return Ok(());
        }
        // one item per group, pins first (their connections are the ones that must exist)
        let mut starts: Vec<ItemKey> = Vec::new();
        let mut seen: HashSet<ItemKey> = HashSet::new();
        let mut ordered = items.clone();
        ordered.sort_by_key(|&k| if board.item(k).is_pin() { 0 } else { 1 });
        for k in ordered {
            if seen.contains(&k) {
                continue;
            }
            for c in board.connected_set(k, net, false).iter() {
                seen.insert(c);
            }
            starts.push(k);
        }
        let mut progress = false;
        for k in starts {
            if failed.contains(&k) || board.get_item(board.item(k).id()).is_none() {
                continue;
            }
            let r = board.autoroute(k, settings, settings.get_via_costs(), Some(stop), None);
            match r.state {
                AutorouteAttemptState::Routed => {
                    progress = true;
                    break;
                }
                AutorouteAttemptState::AlreadyConnected => {}
                _ => {
                    failed.insert(k);
                }
            }
        }
        if !progress {
            // runs that could not be joined are removed as tails afterwards
            if pin_groups(board, net) <= target.max(1) {
                return Ok(());
            }
            return Err("follower could not be connected".into());
        }
    }
    Err("too many steps".into())
}

/// Routes and couples the pairs before everything else (as a designer routes the critical
/// pairs first), then fixes their traces and vias (USER_FIXED) so that the autorouter and the
/// optimizer route around them. Returns the results and the fixed items; [`release_pairs`]
/// unfixes them after routing.
pub fn preroute_pairs(board: &mut RoutingBoard, pairs: &[DiffPair], settings: &RouterSettings, stop: &StopToken, hold: Option<FixedState>) -> (Vec<PairResult>, Vec<crate::ids::ItemId>) {
    let mut results = Vec::new();
    for pair in pairs {
        if stop.is_stop_requested() {
            break;
        }
        let (Some(p), Some(n)) = (net_by_name(board, &pair.p), net_by_name(board, &pair.n)) else {
            log::warn!(target: "fr_engine::pipeline", "diff pair {}/{}: net not found", pair.p, pair.n);
            continue;
        };
        for net in [p, n] {
            if let Err(e) = connect_net(board, net, 1, settings, stop) {
                log::debug!(target: "fr_engine::pipeline::diag", "diff pair: pre-routing net {net}: {e}");
            }
        }
        results.extend(couple_pairs(board, std::slice::from_ref(pair), settings, stop));
        for net in [p, n] {
            board.remove_trace_tails(net, StopConnectionOption::None);
        }
    }
    // more rounds while they help: pairs routed later may have made a detour of an earlier
    // pair unnecessary, or an earlier pair took the room of a later one
    for _round in 0..4 {
        if stop.is_stop_requested() {
            break;
        }
        let mut improved = false;
        for r2 in couple_pairs(board, pairs, settings, stop) {
            if r2.message.is_empty() {
                improved = true;
                if let Some(r) = results.iter_mut().find(|r| r.p == r2.p && r.n == r2.n) {
                    r.coupled_after_mm = r2.coupled_after_mm;
                    r.skew_mm = r2.skew_mm;
                    r.message.clear();
                }
            }
        }
        if !improved {
            break;
        }
    }
    // the final state (later rounds can shift a pair that did not improve itself)
    let upm = units_per_mm(board);
    for r in results.iter_mut() {
        let Some(pair) = pairs.iter().find(|x| x.p == r.p && x.n == r.n) else { continue };
        let (Some(p), Some(n)) = (net_by_name(board, &pair.p), net_by_name(board, &pair.n)) else { continue };
        r.coupled_after_mm = coupled_length(board, p, n, &PairGeometry::new(board, p, n, pair, settings)) / upm;
        r.skew_mm = (net_length(board, n) - net_length(board, p)) / upm;
        r.p_length_mm = net_length(board, p) / upm;
    }
    let nets: Vec<NetNo> = pairs.iter().flat_map(|o| [net_by_name(board, &o.p), net_by_name(board, &o.n)]).flatten().collect();
    let fixed = match hold {
        Some(state) => fix_nets(board, &nets, state),
        None => Vec::new(),
    };
    (results, fixed)
}

/// Fixes (USER_FIXED) the unfixed traces and vias of the nets; returns them for
/// [`release_pairs`].
fn fix_nets(board: &mut RoutingBoard, nets: &[NetNo], state: FixedState) -> Vec<crate::ids::ItemId> {
    let mut fixed = Vec::new();
    for &net in nets {
        for k in board.get_connectable_items(net) {
            let it = board.item(k);
            if (it.is_trace() || it.is_via()) && it.fixed_state() == FixedState::Unfixed && it.net_count() == 1 {
                fixed.push(it.id());
                board.items.get_mut(k).set_fixed_state(state);
            }
        }
    }
    fixed
}

/// Unfixes the items fixed by [`preroute_pairs`] (those that still exist).
pub fn release_pairs(board: &mut RoutingBoard, fixed: &[crate::ids::ItemId]) {
    for &id in fixed {
        if let Some(k) = board.get_item(id) {
            if matches!(board.item(k).fixed_state(), FixedState::UserFixed | FixedState::ShoveFixed) {
                board.items.get_mut(k).set_fixed_state(FixedState::Unfixed);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{offset_parts, offset_polyline};
    use fr_geom::{FloatPoint, IntPoint, Polyline};

    fn pl(points: &[(i32, i32)]) -> Polyline {
        Polyline::from_int_points(&points.iter().map(|&(x, y)| IntPoint::new(x, y)).collect::<Vec<_>>())
    }

    #[test]
    fn offset_keeps_the_distance_on_45_degree_paths() {
        // right, then 45 degrees up, then up
        let p = pl(&[(0, 0), (10_000, 0), (15_000, 5_000), (15_000, 20_000)]);
        for dist in [1_000.0, -1_000.0] {
            let off = offset_polyline(&p, dist).expect("offset");
            assert_eq!(off.corner_count(), p.corner_count());
            let c = off.corner_approx_arr();
            // every segment of the offset is `dist` away from the original path
            for w in c.windows(2) {
                let mid = FloatPoint::new((w[0].x + w[1].x) / 2.0, (w[0].y + w[1].y) / 2.0);
                assert!((p.distance(&mid) - dist.abs()).abs() < 2.0, "{} vs {dist}", p.distance(&mid));
            }
            // exact directions: horizontal, diagonal, vertical
            assert!((c[0].y - c[1].y).abs() < 1e-6);
            assert!(((c[2].x - c[1].x) - (c[2].y - c[1].y)).abs() < 1e-6);
            assert!((c[2].x - c[3].x).abs() < 1e-6);
        }
    }

    #[test]
    fn short_inner_segment_is_split() {
        // a hairpin 400 units wide: offsetting 1000 to the inside turns its end around
        let p = pl(&[(0, 0), (10_000, 0), (10_000, 400), (0, 400), (0, 20_000)]);
        let inner = if offset_polyline(&p, 1_000.0).is_none() { 1_000.0 } else { -1_000.0 };
        assert!(offset_polyline(&p, inner).is_none());
        let parts = offset_parts(&p, inner);
        assert!(!parts.is_empty());
        let total: f64 = parts.iter().map(|x| x.length_approx()).sum();
        assert!(total > 10_000.0, "{total}");
    }
}
