//! Java `autoroute/pipeline/BatchOptimizer` (with `OptimizeCandidateTask`, `WorkerBoardState`,
//! `ReadSortedRouteItems`) and `autoroute/ItemRouteResult`.
//!
//! Each optimizer pass evaluates the route items of the board (vias and traces in ascending
//! x order) as candidates: a candidate rips the connections of the item and reroutes them on a
//! worker board; only the single best improving candidate of the pass is applied.
//!
//! Two evaluation modes ([`OptimizerMode`]):
//! * [`OptimizerMode::JavaCompat`]: Java with `optimizer.max_threads = 1` (the parity target).
//!   One worker board per pass (a deep copy of the baseline), reused across candidates: a
//!   candidate is evaluated after `generateSnapshot`, and undone (`undo`) unless it improved
//!   (then the worker board becomes the candidate result and the next candidate starts on a
//!   new deep copy). The id generator and the failure log of the worker board are not undone,
//!   so the ids of later candidates depend on the earlier ones, like in Java.
//! * [`OptimizerMode::Parallel`]: every candidate is evaluated on a fresh clone of the baseline
//!   (rayon). The result does not depend on the thread count, but is not identical to Java.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::time::{Duration, Instant};

use fr_geom::FloatPoint;
use fr_settings::{ItemSelectionStrategy, RouterSettings};

use crate::ids::FixedState;
use crate::board::{BasicBoard, ItemKey, ItemSet, RoutingBoard, StopConnectionOption};
use crate::datastructures::StopToken;
use crate::ids::ItemId;
use crate::rules::BoardRules;

use super::autorouter::BatchAutorouter;
use super::stats::{format_score, incomplete_count, StatsCache};
use super::PipelineContext;

/// How the optimizer evaluates its candidates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OptimizerMode {
    /// Sequential, one reused worker board per pass: identical to Java with
    /// `optimizer.max_threads = 1`.
    JavaCompat,
    /// Each candidate on a fresh clone of the baseline, evaluated by `threads` rayon threads.
    Parallel { threads: usize },
}

/// Java `ItemRouteResult`.
#[derive(Clone, Debug)]
pub struct ItemRouteResult {
    pub item_id: i32,
    improvement_percentage: f32,
    via_count_before: i32,
    via_count_after: i32,
    trace_length_before: f64,
    trace_length_after: f64,
    incomplete_count_before: i32,
    incomplete_count_after: i32,
    improved: bool,
}

impl ItemRouteResult {
    /// Java `new ItemRouteResult(itemId)` (not improved).
    pub fn unimproved(item_id: i32) -> Self {
        let mut r = Self::new(item_id, 0, 0, 0.0, 0.0, 0, 1);
        r.improved = false;
        r
    }

    /// Java `new ItemRouteResult(itemId, viaCountBefore, viaCountAfter, traceLengthBefore,
    /// traceLengthAfter, incompleteCountBefore, incompleteCountAfter)`.
    pub fn new(
        item_id: i32,
        via_count_before: i32,
        via_count_after: i32,
        trace_length_before: f64,
        trace_length_after: f64,
        incomplete_count_before: i32,
        incomplete_count_after: i32,
    ) -> Self {
        let improved = if incomplete_count_after < incomplete_count_before {
            true
        } else if incomplete_count_after > incomplete_count_before {
            false
        } else if via_count_after < via_count_before {
            true
        } else if via_count_after > via_count_before {
            false
        } else {
            trace_length_after < trace_length_before
        };
        let improvement_percentage = if via_count_before != 0 && trace_length_before != 0.0 {
            // (viaCountAfter / viaCountBefore) is an int division in Java
            (1.0 - ((via_count_after.wrapping_div(via_count_before)) as f64 + trace_length_after / trace_length_before) / 2.0) as f32
        } else {
            0.0
        };
        ItemRouteResult {
            item_id,
            improvement_percentage,
            via_count_before,
            via_count_after,
            trace_length_before,
            trace_length_after,
            incomplete_count_before,
            incomplete_count_after,
            improved,
        }
    }

    /// Java `compareTo(r)`.
    pub fn compare_to(&self, r: &ItemRouteResult) -> Ordering {
        if self.incomplete_count_after != r.incomplete_count_after {
            return self.incomplete_count_after.cmp(&r.incomplete_count_after);
        }
        if self.via_count_after != r.via_count_after {
            return self.via_count_after.cmp(&r.via_count_after);
        }
        if self.trace_length_after < r.trace_length_after {
            return Ordering::Less;
        }
        if self.trace_length_after > r.trace_length_after {
            return Ordering::Greater;
        }
        self.item_id.cmp(&r.item_id)
    }

    /// Java `improvedOver(r)`.
    pub fn improved_over(&self, r: &ItemRouteResult) -> bool {
        self.compare_to(r) == Ordering::Less
    }

    pub fn improved(&self) -> bool {
        self.improved
    }

    pub fn improvement_percentage(&self) -> f32 {
        self.improvement_percentage
    }

    /// Java `viaCountReduced()`.
    pub fn via_count_reduced(&self) -> i32 {
        self.via_count_before - self.via_count_after
    }

    /// Java `lengthReduced()`.
    pub fn length_reduced(&self) -> f64 {
        self.trace_length_before - self.trace_length_after
    }

    /// Java `incompleteCountBefore()`.
    pub fn incomplete_count_before(&self) -> i32 {
        self.incomplete_count_before
    }
}

/// `PriorityQueue<ItemRouteResult>` element (min-heap by `compareTo`).
struct PqEntry(ItemRouteResult);

impl PartialEq for PqEntry {
    fn eq(&self, other: &Self) -> bool {
        self.0.compare_to(&other.0) == Ordering::Equal
    }
}
impl Eq for PqEntry {}
impl PartialOrd for PqEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for PqEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        other.0.compare_to(&self.0)
    }
}

/// The via count and `totalWeightedLength` of `new BoardStatistics(board, null, false)`.
fn light_statistics(board: &BasicBoard) -> (i32, f32) {
    let mut vias = 0i32;
    let mut weighted = 0.0f32;
    let default_class = BoardRules::default_clearance_class();
    for k in board.items.iter() {
        let it = board.item(k);
        if it.is_via() {
            vias += 1;
        } else if it.is_trace() {
            let fixed = it.fixed_state();
            if fixed == FixedState::Unfixed || fixed == FixedState::ShoveFixed {
                let t = it.trace();
                let mut w = t.length() * t.half_width().wrapping_add(board.clearance_value(it.clearance_class(), default_class, t.layer())) as f64;
                if fixed == FixedState::ShoveFixed {
                    w /= 2.0;
                }
                weighted += w as f32;
            }
        }
    }
    // BoardStatistics converts the lengths to mm (unit == null)
    let from = board.communication.unit;
    if from != crate::structure::Unit::Mm {
        weighted = crate::structure::Unit::scale(weighted as f64, from, crate::structure::Unit::Mm) as f32;
    }
    (vias, weighted)
}

/// Java `ReadSortedRouteItems`: the ids of the unfixed vias and traces (not connected to an
/// unfixed via) in ascending (x, y, layer) order of the via center / the larger trace end
/// corner; vias before traces and earlier list positions first at equal keys, each key once.
pub fn sorted_route_item_ids(board: &RoutingBoard) -> Vec<i32> {
    struct Cand {
        x: f64,
        y: f64,
        layer: i32,
        rank: usize,
        id: i32,
    }
    let items = board.get_items();
    let mut cands: Vec<Cand> = Vec::new();
    for (i, &k) in items.iter().enumerate() {
        let it = board.item(k);
        if it.is_via() && !it.is_user_fixed() {
            let c = it.center(board).to_float();
            cands.push(Cand { x: c.x, y: c.y, layer: it.first_layer(board), rank: i, id: it.id().0 });
        }
    }
    let n = items.len();
    for (i, &k) in items.iter().enumerate() {
        let it = board.item(k);
        if it.is_trace() && !board.is_shove_fixed(k) {
            let first: FloatPoint = it.first_corner().to_float();
            let last: FloatPoint = it.last_corner().to_float();
            let compare = if first.x < last.x || (first.x == last.x && first.y < last.y) { last } else { first };
            let connected_to_via = board.normal_contacts(k).iter().any(|c| {
                let ci = board.item(c);
                ci.is_via() && !ci.is_user_fixed()
            });
            if !connected_to_via {
                cands.push(Cand { x: compare.x, y: compare.y, layer: it.trace().layer(), rank: n + i, id: it.id().0 });
            }
        }
    }
    let key_cmp = |a: &Cand, b: &Cand| a.x.partial_cmp(&b.x).unwrap().then(a.y.partial_cmp(&b.y).unwrap()).then(a.layer.cmp(&b.layer));
    cands.sort_by(|a, b| key_cmp(a, b).then(a.rank.cmp(&b.rank)));
    let mut result = Vec::new();
    let mut last: Option<&Cand> = None;
    for c in &cands {
        if let Some(l) = last {
            if key_cmp(l, c) == Ordering::Equal {
                continue;
            }
        }
        // Java starts at (Integer.MIN_VALUE, Integer.MIN_VALUE, -1) and ends at
        // (Integer.MAX_VALUE, Integer.MAX_VALUE, Integer.MAX_VALUE) (exclusive)
        if c.x >= i32::MAX as f64 && c.y >= i32::MAX as f64 {
            break;
        }
        result.push(c.id);
        last = Some(c);
    }
    result
}

/// Java `containsOnlyUnfixedTraces(itemList)`.
fn contains_only_unfixed_traces(board: &RoutingBoard, items: &ItemSet) -> bool {
    items.iter().all(|k| {
        let it = board.item(k);
        !it.is_user_fixed() && it.is_trace()
    })
}

/// The inputs of one candidate evaluation.
#[derive(Clone, Copy)]
struct CandidateParams<'a> {
    settings: &'a RouterSettings,
    baseline_trace_length: f64,
    with_preferred_directions: bool,
    use_increased_ripup_costs: bool,
    stop: &'a StopToken,
    deadline: Option<Instant>,
}

fn deadline_passed(deadline: Option<Instant>) -> bool {
    deadline.map(|d| Instant::now() >= d).unwrap_or(false)
}

/// Java `optRouteItemOnBoard(job, board, item, baselineTraceLength, withPreferredDirections,
/// useIncreasedRipupCosts, thread, deadlineMs)`.
fn opt_route_item_on_board(board: &mut RoutingBoard, item: ItemKey, p: &CandidateParams<'_>) -> ItemRouteResult {
    let settings = p.settings;
    let item_id = board.item(item).id().0;
    let (via_count_before, weighted_before) = light_statistics(board);
    let baseline = if p.baseline_trace_length > 0.0 { p.baseline_trace_length } else { weighted_before as f64 };
    let incomplete_before = incomplete_count(board, None);

    let mut ripped_items = ItemSet::new();
    ripped_items.insert(ItemId(item_id), item);
    let is_trace = board.item(item).is_trace();
    if is_trace {
        let mut contacts = board.trace_start_contacts(item);
        for _ in 0..2 {
            if contains_only_unfixed_traces(board, &contacts) {
                ripped_items.extend_from(&contacts);
            }
            contacts = board.trace_end_contacts(item);
        }
    }
    let mut ripped_connections = ItemSet::new();
    for k in ripped_items.iter() {
        ripped_connections.extend_from(&board.get_connection_items(k, StopConnectionOption::None));
    }
    if ripped_connections.iter().any(|k| board.item(k).is_user_fixed()) {
        return ItemRouteResult::unimproved(item_id);
    }
    let nets = board.item(item).net_numbers().to_vec();
    board.remove_items(ripped_connections.iter().collect::<Vec<_>>());
    for net in nets {
        board.combine_traces(net);
    }

    let mut ripup_costs = settings.get_start_ripup_costs();
    if p.use_increased_ripup_costs {
        ripup_costs = ripup_costs.wrapping_mul(settings.optimizer.additional_ripup_cost_factor_at_start.expect("NullPointerException"));
    }
    if is_trace {
        let f = settings.optimizer.trace_ripup_cost_factor.expect("NullPointerException");
        ripup_costs = fr_jcompat::math_round(f as f64 * ripup_costs as f64) as i32;
    }
    let max_autoroute_passes = settings.optimizer.max_autoroute_passes.expect("NullPointerException");
    let accuracy = settings.trace_pull_tight_accuracy.expect("NullPointerException");
    let mut router = BatchAutorouter::new(board, settings, true, p.with_preferred_directions, ripup_costs, accuracy);
    router.autoroute_passes_for_optimizing_item(board, settings, max_autoroute_passes, p.stop);

    let (via_count_after, weighted_after) = light_statistics(board);
    let incomplete_after = incomplete_count(board, None);
    let mut result = ItemRouteResult::new(
        item_id,
        via_count_before,
        via_count_after,
        baseline,
        weighted_after as f64,
        incomplete_before,
        incomplete_after,
    );
    log::debug!(
        "candidate {item_id}: vias {via_count_before}->{via_count_after}, length {baseline}->{}, incomplete {incomplete_before}->{incomplete_after}, improved {}",
        weighted_after as f64,
        result.improved
    );
    let route_improved = !p.stop.is_stop_requested() && !deadline_passed(p.deadline) && result.improved;
    result.improved = route_improved;
    result
}

/// Java `CandidateResult`.
struct CandidateResult {
    result: ItemRouteResult,
    board: Option<RoutingBoard>,
}

/// Java `OptimizeCandidateTask.call()` with the `WorkerBoardState` of the single executor
/// thread (java-compat mode). `None` stands for a task that ended with an exception.
fn evaluate_compat(worker: &mut Option<RoutingBoard>, baseline: &RoutingBoard, item_id: i32, p: &CandidateParams<'_>) -> Option<CandidateResult> {
    if p.stop.is_stop_requested() || deadline_passed(p.deadline) {
        return Some(CandidateResult { result: ItemRouteResult::unimproved(item_id), board: None });
    }
    let w = worker.get_or_insert_with(|| baseline.deep_copy());
    w.generate_snapshot();
    let finish = |worker: &mut Option<RoutingBoard>, transfer: bool| -> Option<RoutingBoard> {
        let w = worker.as_mut()?;
        let restored = if transfer { w.pop_snapshot() } else { w.undo(None) };
        w.clear_transient_autoroute_state();
        if !restored {
            log::warn!("BatchOptimizer: failed to restore the worker-board snapshot");
            let b = worker.take();
            return if transfer { b } else { None };
        }
        if transfer {
            return worker.take();
        }
        None
    };
    let Some(key) = w.get_item(ItemId(item_id)) else {
        finish(worker, false);
        return Some(CandidateResult { result: ItemRouteResult::unimproved(item_id), board: None });
    };
    let r = catch_unwind(AssertUnwindSafe(|| opt_route_item_on_board(worker.as_mut().unwrap(), key, p)));
    match r {
        Ok(result) => {
            let improved = result.improved;
            let board = finish(worker, improved);
            Some(CandidateResult { result, board: if improved { board } else { None } })
        }
        Err(_) => {
            log::error!("Error in candidate optimization task");
            finish(worker, false);
            None
        }
    }
}

/// Parallel mode: evaluates a candidate on a fresh clone of the baseline.
fn evaluate_fresh(baseline: &RoutingBoard, item_id: i32, p: &CandidateParams<'_>) -> Option<CandidateResult> {
    if p.stop.is_stop_requested() || deadline_passed(p.deadline) {
        return Some(CandidateResult { result: ItemRouteResult::unimproved(item_id), board: None });
    }
    let mut w = baseline.clone();
    let Some(key) = w.get_item(ItemId(item_id)) else {
        return Some(CandidateResult { result: ItemRouteResult::unimproved(item_id), board: None });
    };
    let r = catch_unwind(AssertUnwindSafe(|| opt_route_item_on_board(&mut w, key, p)));
    match r {
        Ok(result) => {
            if result.improved {
                w.clear_transient_autoroute_state();
                Some(CandidateResult { result, board: Some(w) })
            } else {
                Some(CandidateResult { result, board: None })
            }
        }
        Err(_) => {
            log::error!("Error in candidate optimization task");
            None
        }
    }
}

/// Java `BatchOptimizer`.
pub struct BatchOptimizer {
    pub mode: OptimizerMode,
    use_increased_ripup_costs: bool,
    min_cumulative_trace_length: f64,
    total_items_optimized: i32,
    deadline: Option<Instant>,
    pub timed_out: bool,
    best_board: Option<RoutingBoard>,
    best_score: f32,
    best_incomplete_count: i32,
    best_clearance_violation_count: i32,
    result_map: HashMap<i32, ItemRouteResult>,
    pool: Option<rayon::ThreadPool>,
}

impl BatchOptimizer {
    pub fn new(mode: OptimizerMode) -> Self {
        let pool = match mode {
            OptimizerMode::Parallel { threads } => rayon::ThreadPoolBuilder::new().num_threads(threads.max(1)).build().ok(),
            OptimizerMode::JavaCompat => None,
        };
        BatchOptimizer {
            mode,
            use_increased_ripup_costs: false,
            min_cumulative_trace_length: 0.0,
            total_items_optimized: 0,
            deadline: None,
            timed_out: false,
            best_board: None,
            best_score: 0.0,
            best_incomplete_count: 0,
            best_clearance_violation_count: 0,
            result_map: HashMap::new(),
            pool,
        }
    }

    /// Java `evaluatePreFlightGuards(stats)`: the reason to skip the optimizer, if any.
    fn evaluate_pre_flight_guards(board: &RoutingBoard, settings: &RouterSettings, stats: &crate::scoring::BoardStatistics) -> Option<String> {
        if settings.optimizer.enable_preflight_guards == Some(false) {
            return None;
        }
        let incomplete = stats.connections.incomplete_count.unwrap_or(0);
        if incomplete > 0 {
            return Some(format!("the board has {incomplete} unrouted connection(s) (optimizer only runs on completely routed boards)"));
        }
        let initial_score = stats.get_optimizer_score(Some(settings));
        let via_count = stats.vias.total_count.unwrap_or(0);
        let min_len = stats.bounds.min_trace_length_mm;
        let total_len = stats.traces.total_length_mm;
        if via_count == 0 {
            if initial_score >= 950.0 {
                return Some(format!(
                    "the board has no vias to eliminate and initial optimizer score ({:.2}) is already >= 950.00",
                    initial_score as f64
                ));
            }
            if let (Some(min), Some(total)) = (min_len, total_len) {
                if min > 0.0 && total <= min * 1.05f32 {
                    return Some(format!(
                        "the board has no vias to eliminate and trace length ({:.2} mm) is within 5% of theoretical minimum ({:.2} mm)",
                        total as f64, min as f64
                    ));
                }
            }
        }
        if initial_score >= 995.0 {
            return Some(format!(
                "the initial optimizer score ({:.2}) is already at or near theoretical maximum (995.00)",
                initial_score as f64
            ));
        }
        if let (Some(min), Some(total)) = (min_len, total_len) {
            if min > 0.0 && total <= min * 1.02f32 && stats.bounds.min_via_count.map(|m| via_count <= m).unwrap_or(true) {
                return Some(format!(
                    "total trace length ({:.2} mm) is already within 2% of the theoretical minimum ({:.2} mm)",
                    total as f64, min as f64
                ));
            }
        }
        if Self::are_all_vias_mandatory_layer_transitions(board) {
            return Some("all vias on the board are mandatory layer transitions between SMD pins that cannot be eliminated".to_string());
        }
        None
    }

    /// Java `areAllViasMandatoryLayerTransitions(board)`.
    pub fn are_all_vias_mandatory_layer_transitions(board: &RoutingBoard) -> bool {
        let vias = board.get_vias();
        if vias.is_empty() {
            return false;
        }
        for via in vias {
            let v = board.item(via);
            if v.is_user_fixed() {
                continue;
            }
            if v.net_count() == 0 {
                return false;
            }
            let connected = board.connected_set(via, v.net_number(0), true);
            let mut smd_pins = Vec::new();
            let mut only_smd_pins_and_traces = true;
            for c in connected.iter() {
                let it = board.item(c);
                if it.is_pin() {
                    if it.first_layer(board) == it.last_layer(board) {
                        smd_pins.push(c);
                    } else {
                        only_smd_pins_and_traces = false;
                        break;
                    }
                } else if it.is_via() {
                    if c != via {
                        only_smd_pins_and_traces = false;
                        break;
                    }
                } else if !it.is_trace() {
                    only_smd_pins_and_traces = false;
                    break;
                }
            }
            if !only_smd_pins_and_traces || smd_pins.len() < 2 {
                return false;
            }
            let first_layer = board.item(smd_pins[0]).first_layer(board);
            if !smd_pins.iter().any(|&p| board.item(p).first_layer(board) != first_layer) {
                return false;
            }
        }
        true
    }

    /// Java `optimizerCandidateRejectionReason(...)`.
    #[allow(clippy::neg_cmp_op_on_partial_ord)]
    fn rejection_reason(&self, incomplete: i32, violations: i32, score: f32) -> Option<&'static str> {
        if incomplete > self.best_incomplete_count {
            return Some("CONNECTIVITY_REGRESSION");
        }
        if violations > self.best_clearance_violation_count {
            return Some("DRC_COUNT_REGRESSION");
        }
        if !(score > self.best_score) {
            return Some("OPTIMIZER_SCORE_NOT_IMPROVED");
        }
        None
    }

    /// Java `restoreIncumbentBoard()`.
    fn restore_incumbent_board(&self, board: &mut RoutingBoard) {
        *board = self.best_board.as_ref().expect("best board").deep_copy();
    }

    /// Java `runBatchLoop()`.
    pub fn run_batch_loop(&mut self, board: &mut RoutingBoard, settings: &mut RouterSettings, ctx: &PipelineContext, stats: &mut StatsCache) {
        self.use_increased_ripup_costs = true;
        let initial = stats.score(board, settings);
        if let Some(reason) = Self::evaluate_pre_flight_guards(board, settings, stats.statistics(board)) {
            log::info!("Skipping optimization stage: {reason}.");
            return;
        }
        let stage_start = Instant::now();
        self.best_board = Some(board.clone());
        self.best_score = initial.optimizer_score;
        self.best_incomplete_count = initial.incomplete_count;
        self.best_clearance_violation_count = initial.clearance_violation_count;
        log::info!(
            "Optimization stage started. Baseline router score: {:.2}, optimizer score: {:.2}, incomplete connections: {}, clearance violations: {}.",
            initial.router_score as f64,
            initial.optimizer_score as f64,
            initial.incomplete_count,
            initial.clearance_violation_count
        );
        if ctx.wall_clock_limits {
            if let Some(s) = settings.optimizer.timeout_string.as_deref().and_then(fr_settings::parse_timespan_string) {
                self.deadline = Some(stage_start + Duration::from_secs(s.max(0) as u64));
            }
        }
        if let Some(threshold) = settings.optimizer.optimization_improvement_threshold {
            if threshold.is_nan() || threshold.is_infinite() || threshold < 0.0 {
                log::warn!("Invalid optimizer improvement threshold: {threshold:.4}. Resetting to default.");
                settings.optimizer.optimization_improvement_threshold = Some(fr_settings::defaults::DEFAULT_OPTIMIZER_IMPROVEMENT_THRESHOLD);
            } else if threshold > 0.0 && threshold < 0.1 {
                settings.optimizer.optimization_improvement_threshold = Some(threshold * 100.0);
            }
        }
        let settings: &RouterSettings = settings;
        let stop = &ctx.stop;
        let mut current_pass = 0;
        loop {
            if !(settings.optimizer.max_passes.map(|m| current_pass < m).unwrap_or(true)
                && settings.optimizer.max_items.map(|m| self.total_items_optimized < m).unwrap_or(true)
                && !stop.is_stop_requested())
            {
                break;
            }
            if deadline_passed(self.deadline) {
                self.timed_out = true;
                log::info!("Optimizer stage timed out before starting pass #{}", current_pass + 1);
                break;
            }
            current_pass += 1;
            let score_before_pass = stats.score(board, settings).optimizer_score;
            let with_preferred_directions = current_pass % 2 != 0;
            self.opt_route_pass(board, settings, ctx, stats, current_pass, with_preferred_directions);
            if self.timed_out {
                break;
            }
            let pass_stats = stats.score(board, settings);
            let score_after_pass = pass_stats.optimizer_score;
            match self.rejection_reason(pass_stats.incomplete_count, pass_stats.clearance_violation_count, score_after_pass) {
                None => {
                    self.best_score = score_after_pass;
                    self.best_incomplete_count = pass_stats.incomplete_count;
                    self.best_clearance_violation_count = pass_stats.clearance_violation_count;
                    self.best_board = Some(board.clone());
                }
                Some(reason) => {
                    log::info!(
                        "Optimizer pass #{current_pass} candidate rejected: {reason}. Restoring incumbent (optimizer score {:.2}, incomplete connections: {}, clearance violations: {}).",
                        self.best_score as f64,
                        self.best_incomplete_count,
                        self.best_clearance_violation_count
                    );
                    self.restore_incumbent_board(board);
                }
            }
            let pass_improvement_fraction =
                if score_before_pass > 0.0 { (score_after_pass - score_before_pass) as f64 / score_before_pass as f64 } else { 0.0 };
            let pass_improvement_percent = pass_improvement_fraction * 100.0;
            log::info!(
                "Optimizer pass #{current_pass}: optimizer score {:.2} -> {:.2} ({}, {}), router score: {:.2}, incomplete connections: {}, clearance violations: {}.",
                score_before_pass as f64,
                score_after_pass as f64,
                if score_after_pass > score_before_pass {
                    "IMPROVED"
                } else if score_after_pass < score_before_pass {
                    "REGRESSED"
                } else {
                    "UNCHANGED"
                },
                if score_before_pass > 0.0 { format!("{pass_improvement_percent:.4}%") } else { "n/a (baseline was 0.00)".to_string() },
                pass_stats.router_score as f64,
                pass_stats.incomplete_count,
                pass_stats.clearance_violation_count
            );
            let score_improvement = if self.use_increased_ripup_costs && score_after_pass <= score_before_pass {
                self.use_increased_ripup_costs = false;
                -1.0
            } else {
                pass_improvement_percent
            };
            let threshold = settings.optimizer.optimization_improvement_threshold.expect("NullPointerException") as f64;
            if score_improvement != -1.0 && score_improvement < threshold {
                log::info!(
                    "Stopping optimizer because the improvement in this pass ({score_improvement:.4}%) is below the threshold ({threshold:.2}%)."
                );
                break;
            }
        }
        let final_score = stats.score(board, settings).optimizer_score;
        if final_score < self.best_score && self.best_board.is_some() {
            log::info!("Restoring best board achieved (score {:.2} vs final {:.2}).", self.best_score as f64, final_score as f64);
            self.restore_incumbent_board(board);
        }
        self.best_board = None;
        let f = stats.score(board, settings);
        log::info!(
            "Optimization stage {} Baseline router score: {:.2}, baseline optimizer score: {:.2}, final router score: {:.2}, final optimizer score: {:.2}, completed in {:.2} seconds.",
            if self.timed_out {
                "completed with timeout:"
            } else if stop.is_stop_requested() {
                "interrupted:"
            } else {
                "completed:"
            },
            initial.router_score as f64,
            initial.optimizer_score as f64,
            f.router_score as f64,
            f.optimizer_score as f64,
            stage_start.elapsed().as_secs_f64()
        );
    }

    /// Java `prepareCandidateItems()`.
    fn prepare_candidate_items(&mut self, board: &RoutingBoard, settings: &RouterSettings) -> Vec<i32> {
        let sorted = sorted_route_item_ids(board);
        let strategy = settings.optimizer.item_selection_strategy.unwrap_or(ItemSelectionStrategy::Sequential);
        let ids = if strategy == ItemSelectionStrategy::Prioritized && !self.result_map.is_empty() {
            let mut non_prioritized = Vec::new();
            let mut pq = BinaryHeap::new();
            for id in sorted {
                match self.result_map.get(&id) {
                    Some(r) => pq.push(PqEntry(r.clone())),
                    None => non_prioritized.push(id),
                }
            }
            let mut ids = Vec::new();
            while let Some(PqEntry(r)) = pq.pop() {
                ids.push(r.item_id);
            }
            ids.extend(non_prioritized);
            ids
        } else {
            sorted
        };
        self.result_map.clear();
        ids
    }

    /// Java `optRoutePass(passNo, withPreferredDirections)`.
    fn opt_route_pass(
        &mut self,
        board: &mut RoutingBoard,
        settings: &RouterSettings,
        ctx: &PipelineContext,
        stats: &mut StatsCache,
        pass_no: i32,
        with_preferred_directions: bool,
    ) -> f32 {
        let pass_start = Instant::now();
        self.min_cumulative_trace_length = stats.statistics(board).traces.total_weighted_length.unwrap_or(0.0) as f64;
        let mut candidates = self.prepare_candidate_items(board, settings);
        if let Some(m) = settings.optimizer.max_items {
            if m > 0 {
                let remaining = m.wrapping_sub(self.total_items_optimized);
                if remaining <= 0 {
                    log::info!("Max items limit reached ({m}). Stopping optimizer.");
                    return 0.0;
                }
                if candidates.len() > remaining as usize {
                    candidates.truncate(remaining as usize);
                }
            }
        }
        if candidates.is_empty() {
            return 0.0;
        }
        let thread_pool_size = match self.mode {
            OptimizerMode::JavaCompat => 1,
            OptimizerMode::Parallel { threads } => threads.max(1),
        };
        let chunk_size = (thread_pool_size * 4).max(8);
        let max_consecutive_failures = if pass_no == 1 {
            settings.optimizer.max_consecutive_failures_pass1.unwrap_or(12)
        } else {
            settings.optimizer.max_consecutive_failures.unwrap_or(50)
        };
        let params = CandidateParams {
            settings,
            baseline_trace_length: self.min_cumulative_trace_length,
            with_preferred_directions,
            use_increased_ripup_costs: self.use_increased_ripup_costs,
            stop: &ctx.stop,
            deadline: self.deadline,
        };
        let stop = &ctx.stop;
        let mut winning: Option<CandidateResult> = None;
        let mut stopped_or_timed_out = false;
        let mut consecutive_failures = 0;
        let mut worker: Option<RoutingBoard> = None;
        let mut evaluated = 0usize;
        'chunks: for chunk in candidates.chunks(chunk_size) {
            if deadline_passed(self.deadline) {
                log::info!("Optimizer stage timed out.");
                self.timed_out = true;
                stopped_or_timed_out = true;
                break;
            }
            if stop.is_stop_requested() {
                stopped_or_timed_out = true;
                break;
            }
            let results: Vec<Option<CandidateResult>> = match (&self.mode, &self.pool) {
                (OptimizerMode::Parallel { .. }, Some(pool)) => {
                    use rayon::prelude::*;
                    let b: &RoutingBoard = board;
                    pool.install(|| chunk.par_iter().map(|&id| evaluate_fresh(b, id, &params)).collect())
                }
                _ => {
                    // Java evaluates the tasks of a chunk in order on the executor thread; the
                    // results are consumed in the same order, so evaluating lazily (and not
                    // evaluating the tasks after an early stop) gives the same result.
                    let mut v = Vec::with_capacity(chunk.len());
                    for &id in chunk {
                        let r = evaluate_compat(&mut worker, board, id, &params);
                        v.push(r);
                        if would_stop(&v, consecutive_failures, max_consecutive_failures) {
                            break;
                        }
                    }
                    v
                }
            };
            for r in results {
                evaluated += 1;
                let Some(res) = r else { continue };
                self.total_items_optimized += 1;
                self.result_map.insert(res.result.item_id, res.result.clone());
                if res.result.improved {
                    consecutive_failures = 0;
                    let better = match &winning {
                        None => true,
                        Some(w) => res.result.improved_over(&w.result),
                    };
                    if better {
                        winning = Some(res);
                    }
                } else {
                    consecutive_failures += 1;
                    if consecutive_failures >= max_consecutive_failures {
                        log::info!(
                            "Stopping optimization pass #{pass_no} early after {consecutive_failures} consecutive items could not be improved."
                        );
                        break 'chunks;
                    }
                }
            }
        }
        let mut route_improved = 0.0f32;
        if !stopped_or_timed_out {
            if let Some(w) = winning {
                if w.result.improved {
                    *board = w.board.expect("improved candidate without board");
                    self.min_cumulative_trace_length = light_statistics(board).1 as f64;
                    route_improved = w.result.improvement_percentage;
                }
            }
        }
        if self.use_increased_ripup_costs && route_improved == 0.0 {
            self.use_increased_ripup_costs = false;
            route_improved = -1.0;
        }
        let s = stats.score(board, settings);
        log::info!(
            "Optimizer pass #{pass_no} was completed in {:.2} seconds ({} of {} candidates) with the score of {}.",
            pass_start.elapsed().as_secs_f64(),
            evaluated,
            candidates.len(),
            format_score(s.optimizer_score, s.incomplete_count, s.clearance_violation_count)
        );
        route_improved
    }
}

/// True if the consumption loop of `opt_route_pass` stops early after the results in `v`
/// (given the consecutive failures before this chunk).
fn would_stop(v: &[Option<CandidateResult>], consecutive_before: i32, max: i32) -> bool {
    let mut c = consecutive_before;
    for r in v.iter().flatten() {
        if r.result.improved {
            c = 0;
        } else {
            c += 1;
            if c >= max {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_route_result_improvement_rules() {
        // fewer incompletes win, then fewer vias, then shorter traces
        assert!(ItemRouteResult::new(1, 5, 9, 10.0, 20.0, 2, 1).improved());
        assert!(!ItemRouteResult::new(1, 5, 1, 10.0, 1.0, 1, 2).improved());
        assert!(ItemRouteResult::new(1, 5, 4, 10.0, 20.0, 1, 1).improved());
        assert!(!ItemRouteResult::new(1, 5, 6, 10.0, 1.0, 1, 1).improved());
        assert!(ItemRouteResult::new(1, 5, 5, 10.0, 9.0, 1, 1).improved());
        assert!(!ItemRouteResult::new(1, 5, 5, 10.0, 10.0, 1, 1).improved());
        assert!(!ItemRouteResult::unimproved(7).improved());
        // int division viaCountAfter / viaCountBefore (4 / 5 == 0)
        let r = ItemRouteResult::new(1, 5, 4, 10.0, 9.0, 0, 0);
        assert_eq!(r.improvement_percentage(), (1.0 - (0.0 + 0.9) / 2.0) as f32);
        assert_eq!(ItemRouteResult::new(1, 0, 4, 10.0, 9.0, 0, 0).improvement_percentage(), 0.0);
    }

    #[test]
    fn item_route_result_order() {
        let a = ItemRouteResult::new(3, 5, 4, 10.0, 9.0, 0, 0);
        let b = ItemRouteResult::new(2, 5, 4, 10.0, 9.0, 0, 0);
        let c = ItemRouteResult::new(1, 5, 4, 10.0, 9.5, 0, 0);
        assert!(b.improved_over(&a)); // equal metrics: smaller item id first
        assert!(a.improved_over(&c));
        let mut pq = BinaryHeap::new();
        for r in [a, b, c] {
            pq.push(PqEntry(r));
        }
        let order: Vec<i32> = std::iter::from_fn(|| pq.pop().map(|e| e.0.item_id)).collect();
        assert_eq!(order, vec![2, 3, 1]);
    }
}
