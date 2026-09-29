//! Java `autoroute/pipeline/{BatchAutorouter, AutorouteBatchLoop, AutoroutePassRunner}`
//! (single-thread pass only; `runMultiThread` / `BatchAutorouterThread` are dead code) and the
//! strict DRC part of `AutorouteConnectionRouter`.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::time::Instant;

use fr_settings::{ExpansionCostFactor, RouterSettings};

use crate::autoroute::router::{route_connection, ConnectionRouterParams};
use crate::autoroute::AutorouteAttemptResult;
use crate::board::{AutorouteAttemptState, ItemKey, ItemSet, RoutingBoard, StopConnectionOption};
use crate::datastructures::StopToken;
use crate::drc::clearance_violation::clearance_violation_count;
use crate::ids::{ItemId, NetNo};

use super::fanout;
use super::history::{board_hash, BoardHistory, MAX_HISTORY_SIZE};
use super::stats::{format_score, incomplete_count, StatsCache};
use super::PipelineContext;

/// Java `BOARD_RANK_LIMIT`.
const BOARD_RANK_LIMIT: i32 = MAX_HISTORY_SIZE as i32;
/// Java `MAXIMUM_TRIES_ON_THE_SAME_BOARD`.
const MAXIMUM_TRIES_ON_THE_SAME_BOARD: i32 = 3;
/// Java `TIME_LIMIT_TO_PREVENT_ENDLESS_LOOP`.
const TIME_LIMIT_TO_PREVENT_ENDLESS_LOOP: i32 = 1000;
/// Java `STOP_AT_PASS_MINIMUM`.
const STOP_AT_PASS_MINIMUM: i32 = 8;
/// Java `STOP_AT_PASS_MODULO`.
const STOP_AT_PASS_MODULO: i32 = 4;
/// Java `STAGNATION_PASS_LIMIT`.
const STAGNATION_PASS_LIMIT: i32 = 10;
/// Java `FANOUT_RECOVERY_STAGNATION_PASSES`.
const FANOUT_RECOVERY_STAGNATION_PASSES: i32 = 3;
/// Java `STAGNATION_SCORE_THRESHOLD`.
const STAGNATION_SCORE_THRESHOLD: f32 = 0.5;

/// Java `BatchAutorouter` (with the state of its `AutoroutePassRunner`).
pub struct BatchAutorouter {
    pub remove_unconnected_vias: bool,
    pub trace_costs: Vec<ExpansionCostFactor>,
    pub retain_autoroute_database: bool,
    pub start_ripup_costs: i32,
    pub trace_pull_tight_accuracy: i32,
    pub total_items_routed: i32,
    pub is_optimizer_autorouter: bool,
    /// fastroute: when set, a pass only routes items of these nets (the optimizer re-routes
    /// the nets of the ripped item instead of every unrouted connection on the board).
    pub net_filter: Option<BTreeSet<NetNo>>,
    pub fanout_timed_out: bool,
    /// Java `initialUnroutedCount`.
    pub initial_unrouted_count: i32,
    // AutoroutePassRunner state
    previous_incomplete_nets: BTreeSet<NetNo>,
    previous_incomplete_count: i32,
    stagnation_count: i32,
}

/// Counters of one pass (for logging).
#[derive(Clone, Copy, Debug, Default)]
pub struct PassCounters {
    pub routed: i32,
    pub not_routed: i32,
    pub skipped: i32,
    pub ripped: i32,
}

impl BatchAutorouter {
    /// Java `new BatchAutorouter(job)`.
    pub fn for_job(board: &RoutingBoard, settings: &RouterSettings) -> Self {
        Self::new(
            board,
            settings,
            !settings.is_fanout_enabled(),
            true,
            settings.get_start_ripup_costs(),
            settings.trace_pull_tight_accuracy.unwrap_or(500),
        )
    }

    /// Java `new BatchAutorouter(thread, board, settings, removeUnconnectedVias,
    /// withPreferredDirections, startRipupCosts, pullTightAccuracy)`.
    pub fn new(
        board: &RoutingBoard,
        settings: &RouterSettings,
        remove_unconnected_vias: bool,
        with_preferred_directions: bool,
        start_ripup_costs: i32,
        pull_tight_accuracy: i32,
    ) -> Self {
        let trace_costs = if with_preferred_directions {
            settings.get_trace_costs()
        } else {
            (0..board.layer_count() as usize)
                .map(|i| {
                    let c = settings.get_preferred_direction_trace_costs(i);
                    ExpansionCostFactor { horizontal: c, vertical: c }
                })
                .collect()
        };
        BatchAutorouter {
            remove_unconnected_vias,
            trace_costs,
            retain_autoroute_database: false,
            start_ripup_costs,
            trace_pull_tight_accuracy: pull_tight_accuracy,
            total_items_routed: 0,
            is_optimizer_autorouter: false,
            net_filter: None,
            fanout_timed_out: false,
            initial_unrouted_count: 0,
            previous_incomplete_nets: BTreeSet::new(),
            previous_incomplete_count: -1,
            stagnation_count: 0,
        }
    }

    /// Java `AutoroutePassRunner.resetAntiOscillationState()`.
    pub fn reset_anti_oscillation_state(&mut self) {
        self.previous_incomplete_nets.clear();
        self.previous_incomplete_count = -1;
        self.stagnation_count = 0;
    }

    fn params(&self) -> ConnectionRouterParams {
        ConnectionRouterParams {
            trace_costs: self.trace_costs.clone(),
            start_ripup_costs: self.start_ripup_costs,
            remove_unconnected_vias: self.remove_unconnected_vias,
            retain_autoroute_database: self.retain_autoroute_database,
            trace_pull_tight_accuracy: self.trace_pull_tight_accuracy,
        }
    }

    /// Java `removeTails(stopConnectionOption)`.
    pub fn remove_tails(&self, board: &mut RoutingBoard, option: StopConnectionOption, stop: Option<&StopToken>) {
        board.start_marking_changed_area();
        board.remove_trace_tails(-1, option);
        board.opt_changed_area(&[], None, self.trace_pull_tight_accuracy, Some(&self.trace_costs), stop, TIME_LIMIT_TO_PREVENT_ENDLESS_LOOP);
    }

    /// Java `getAutorouteItems(board)`: the connectable, not routable items (pins, conduction
    /// areas, fixed traces and vias) not yet connected to all items of one of their nets, plane
    /// net items first.
    pub fn get_autoroute_items(board: &RoutingBoard) -> Vec<ItemKey> {
        let mut plane_items = Vec::new();
        let mut signal_items = Vec::new();
        let mut handled: HashSet<i32> = HashSet::new();
        for key in board.get_items() {
            let item = board.item(key);
            if !item.is_connectable_class() || item.is_routable() {
                continue;
            }
            if handled.contains(&item.id().0) {
                continue;
            }
            let mut needs_routing = false;
            let mut has_plane_net = false;
            let net_count = item.net_count();
            for i in 0..net_count {
                let net_no = board.item(key).net_number(i);
                let connected = board.connected_set(key, net_no, false);
                for c in connected.iter() {
                    if board.item(c).net_count() <= 1 {
                        handled.insert(board.item(c).id().0);
                    }
                }
                let net_item_count = board.connectable_item_count(net_no);
                if (connected.len() as i32) < net_item_count && !has_ignored_nets(board, key) {
                    let is_plane = board.rules.nets.get(net_no).map(|n| n.contains_plane()).unwrap_or(false);
                    if is_plane {
                        if connected.iter().any(|c| board.item(c).is_conduction_area()) {
                            continue;
                        }
                        has_plane_net = true;
                    }
                    needs_routing = true;
                }
            }
            if needs_routing {
                if has_plane_net {
                    plane_items.push(key);
                } else {
                    signal_items.push(key);
                }
            }
        }
        plane_items.extend(signal_items);
        plane_items
    }

    /// Java `isPlaneItem(item, board)`.
    fn is_plane_item(board: &RoutingBoard, key: ItemKey) -> bool {
        board.item(key).net_numbers().iter().any(|&n| board.rules.nets.get(n).map(|n| n.contains_plane()).unwrap_or(false))
    }

    /// Java `AutoroutePassRunner.reorderSingleThreadItems(list, passNo)`.
    fn reorder_items(&self, board: &RoutingBoard, list: &mut Vec<ItemKey>, pass_no: i32) {
        let (mut plane_items, mut signal_items): (Vec<ItemKey>, Vec<ItemKey>) = (Vec::new(), Vec::new());
        for &k in list.iter() {
            if Self::is_plane_item(board, k) {
                plane_items.push(k);
            } else {
                signal_items.push(k);
            }
        }
        if signal_items.is_empty() {
            return;
        }
        let sort_key = |k: &ItemKey| {
            let it = board.item(*k);
            (if it.net_count() > 0 { it.net_number(0) } else { 0 }, it.id().0)
        };
        if self.stagnation_count >= 2 {
            signal_items.sort_by_key(sort_key);
            let seed = (pass_no as i64).wrapping_mul(1000003).wrapping_add(board.rules.nets.max_net_number() as i64);
            let mut rnd = fr_jcompat::random::JavaRandom::new(seed);
            fr_jcompat::random::shuffle(&mut signal_items, &mut rnd);
            log::debug!(
                "Pass #{pass_no}: applied deterministic permutation to {} signal items (stagnation count: {}).",
                signal_items.len(),
                self.stagnation_count
            );
        } else if !self.previous_incomplete_nets.is_empty() {
            let (mut persistent, mut other): (Vec<ItemKey>, Vec<ItemKey>) = (Vec::new(), Vec::new());
            for k in signal_items {
                if board.item(k).net_numbers().iter().any(|n| self.previous_incomplete_nets.contains(n)) {
                    persistent.push(k);
                } else {
                    other.push(k);
                }
            }
            persistent.sort_by_key(sort_key);
            other.sort_by_key(sort_key);
            persistent.extend(other);
            signal_items = persistent;
        } else {
            signal_items.sort_by_key(sort_key);
        }
        list.clear();
        list.extend(plane_items);
        list.extend(signal_items);
    }

    /// Java `autoroutePass(passNo)` (`AutoroutePassRunner.runSingleThread`): routes all
    /// incomplete items once. Returns false if the board is already completely routed (or
    /// nothing was attempted).
    pub fn autoroute_pass(&mut self, board: &mut RoutingBoard, settings: &RouterSettings, pass_no: i32, stop: &StopToken) -> (bool, PassCounters) {
        let r = catch_unwind(AssertUnwindSafe(|| self.autoroute_pass_impl(board, settings, pass_no, stop)));
        match r {
            Ok(v) => v,
            Err(_) => {
                log::error!("Something went wrong during the auto-routing");
                (false, PassCounters::default())
            }
        }
    }

    fn autoroute_pass_impl(&mut self, board: &mut RoutingBoard, settings: &RouterSettings, pass_no: i32, stop: &StopToken) -> (bool, PassCounters) {
        let mut items = Self::get_autoroute_items(board);
        if let Some(filter) = &self.net_filter {
            items.retain(|&k| board.item(k).net_numbers().iter().any(|n| filter.contains(n)));
        }
        if items.is_empty() {
            return (false, PassCounters::default());
        }
        if pass_no > 1 {
            self.reorder_items(board, &mut items, pass_no);
        }
        let params = self.params();
        let ids: Vec<ItemId> = items.iter().map(|&k| board.item(k).id()).collect();
        let mut c = PassCounters::default();
        let max_items = settings.autorouter.max_items;
        let mut item_index = 0;
        while item_index < items.len() {
            let mut current = items[item_index];
            item_index += 1;
            if stop.is_stop_autorouter_requested() {
                break;
            }
            let mut i = 0;
            while i < board.item(current).net_count() {
                if stop.is_stop_autorouter_requested() {
                    break;
                }
                if let Some(m) = max_items {
                    if m > 0 && self.total_items_routed >= m {
                        log::info!("Max items limit reached ({m}). Stopping auto-router.");
                        stop.request_stop();
                        break;
                    }
                }
                self.total_items_routed += 1;
                board.start_marking_changed_area();
                let net_no = board.item(current).net_number(i);
                let mut ripped = ItemSet::new();
                let mut ripped_costs: HashMap<ItemKey, i32> = HashMap::new();
                let snapshot = if settings.is_strict_drc() { Some(board.deep_copy()) } else { None };
                let outcome = route_connection(board, settings, &params, current, net_no, &mut ripped, Some(&mut ripped_costs), pass_no, Some(stop));
                let mut result = outcome.result;
                if result.state == AutorouteAttemptState::Routed {
                    if let Some(max_id) = outcome.max_item_id_before_route {
                        // AutorouteConnectionRouter.applyStrictDrcAfterRoute
                        if settings.is_strict_drc() || pass_no >= 3 {
                            if let Some(rejection) = enforce_strict_drc(board, net_no, max_id) {
                                if let Some(s) = snapshot {
                                    // Java replaces the board by the snapshot and continues with
                                    // the item objects of the old board; the port maps the
                                    // remaining items of the pass list to the snapshot by id.
                                    *board = s;
                                    for j in (item_index - 1)..items.len() {
                                        if let Some(k) = board.get_item(ids[j]) {
                                            items[j] = k;
                                        }
                                    }
                                    current = items[item_index - 1];
                                }
                                result = rejection;
                            }
                        }
                    }
                }
                match result.state {
                    AutorouteAttemptState::Routed => c.routed += 1,
                    AutorouteAttemptState::AlreadyConnected | AutorouteAttemptState::NoUnconnectedNets | AutorouteAttemptState::ConnectedToPlane => {
                        c.skipped += 1
                    }
                    _ => {
                        record_failure(board, current, pass_no, &result);
                        let failure_count = board.failure_log.get_failure_count(&board.basic, current);
                        if failure_count >= 2 {
                            let net_no = board.item(current).net_number(i);
                            let to_rip: Vec<ItemKey> = board
                                .get_connectable_items(net_no)
                                .into_iter()
                                .filter(|&k| {
                                    let it = board.item(k);
                                    (it.is_trace() || it.is_via()) && !it.is_user_fixed() && it.net_count() == 1
                                })
                                .collect();
                            if !to_rip.is_empty() {
                                board.remove_items(to_rip);
                            }
                        }
                        c.not_routed += 1;
                    }
                }
                c.ripped += ripped.len() as i32;
                i += 1;
            }
        }
        if self.remove_unconnected_vias {
            self.remove_tails(board, StopConnectionOption::None, Some(stop));
        } else {
            self.remove_tails(board, StopConnectionOption::FanoutVia, Some(stop));
        }
        let mut current_incomplete_nets = BTreeSet::new();
        let incomplete = incomplete_count(board, Some(&mut current_incomplete_nets));
        if self.previous_incomplete_count >= 0 && incomplete >= self.previous_incomplete_count {
            self.stagnation_count += 1;
        } else {
            self.stagnation_count = 0;
        }
        self.previous_incomplete_count = incomplete;
        self.previous_incomplete_nets = current_incomplete_nets;
        (c.routed > 0 || c.not_routed > 0, c)
    }

    /// Java `autoroutePassesForOptimizingItem(...)` body after the construction: routes up to
    /// `max_pass_count` passes, then removes the tails. Returns the number of passes.
    pub fn autoroute_passes_for_optimizing_item(&mut self, board: &mut RoutingBoard, settings: &RouterSettings, max_pass_count: i32, stop: &StopToken) -> i32 {
        self.is_optimizer_autorouter = true;
        let mut still_unrouted = true;
        let mut current_pass_no = 1;
        while still_unrouted && !stop.is_stop_autorouter_requested() && current_pass_no <= max_pass_count {
            still_unrouted = self.autoroute_pass(board, settings, current_pass_no, stop).0;
            current_pass_no += 1;
        }
        self.remove_tails(board, StopConnectionOption::None, Some(stop));
        if !still_unrouted {
            current_pass_no -= 1;
        }
        current_pass_no
    }

    /// Java `runBatchLoop()` (`AutorouteBatchLoop.run`): fanout, then autoroute passes until the
    /// board is completed or a stop rule fires. Returns true if not stopped.
    #[allow(clippy::collapsible_if)]
    pub fn run_batch_loop(&mut self, board: &mut RoutingBoard, settings: &RouterSettings, ctx: &PipelineContext) -> bool {
        let stop = &ctx.stop;
        let any_routable = (0..settings.get_layer_count())
            .any(|i| settings.get_layer_active(i) && board.layer_structure.layers.get(i).map(|l| l.is_signal).unwrap_or(false));
        if !any_routable {
            log::error!("Cannot start autorouter: all layers are disabled.");
            return false;
        }
        let optimizer_threads = settings.optimizer.max_threads.map(|t| t.max(1)).unwrap_or(1);
        log::info!(
            "Pipeline thread limits: autorouter.max_threads={}, optimizer.max_threads={}.",
            settings.autorouter.max_threads.unwrap_or(1),
            optimizer_threads
        );
        self.initial_unrouted_count = incomplete_count(board, None);
        self.reset_anti_oscillation_state();
        let mut bh = BoardHistory::new();
        let mut stats = StatsCache::new();

        if settings.is_fanout_enabled() {
            if board.get_smd_pins().is_empty() {
                log::info!("Fanout stage is enabled but skipped because the board has no SMD pins.");
            } else {
                let t = Instant::now();
                let summary = fanout::fanout_board(board, settings, ctx);
                self.fanout_timed_out = summary.timed_out;
                log::info!(
                    "Fanout stage {} {} passes, completed in {:.2} seconds.",
                    if summary.timed_out { "completed with timeout:" } else if stop.is_stop_autorouter_requested() { "interrupted:" } else { "completed:" },
                    summary.completed_passes,
                    t.elapsed().as_secs_f64()
                );
            }
        }

        let current_unrouted = incomplete_count(board, None);
        let is_router_enabled = settings.get_run_router() && settings.autorouter.max_passes.map(|m| m >= 0).unwrap_or(true);
        let stage_start = Instant::now();
        if is_router_enabled {
            let s = stats.score(board, settings);
            log::info!(
                "Auto-routing stage started with baseline score {:.2} for {} unrouted item{}.",
                s.router_score as f64,
                current_unrouted,
                if current_unrouted == 1 { "" } else { "s" }
            );
        }
        let mut continue_autorouting = is_router_enabled;
        let mut current_pass = 1;
        let mut consecutive_no_improvement_passes = 0;
        let mut fanout_recovery_applied = false;
        let mut last_best_score = f32::NEG_INFINITY;
        let mut global_best_score = f32::NEG_INFINITY;
        let mut pass_of_best_score = 0;
        let mut incomplete_count_at_best_score = 0;
        while continue_autorouting && !stop.is_stop_autorouter_requested() {
            if let Some(m) = settings.autorouter.max_passes {
                if m > 0 && current_pass > m {
                    stop.request_stop_autorouter();
                    break;
                }
            }
            let pass_start = Instant::now();
            {
                let hash = board_hash(board);
                let b: &RoutingBoard = board;
                bh.add_with(b, hash, || stats.score(b, settings).router_score);
            }
            let (cont, counters) = self.autoroute_pass(board, settings, current_pass, stop);
            continue_autorouting = cont;
            let mut after = stats.score(board, settings);

            if bh.size() >= STOP_AT_PASS_MINIMUM as usize || stop.is_stop_autorouter_requested() {
                if (current_pass % STOP_AT_PASS_MODULO == 0 && current_pass >= STOP_AT_PASS_MINIMUM) || stop.is_stop_autorouter_requested() {
                    if bh.get_max_score() > after.router_score {
                        let Some(to_restore) = bh.restore_board(MAXIMUM_TRIES_ON_THE_SAME_BOARD) else {
                            log::info!("The router was not able to improve the board, stopping the auto-router.");
                            stop.request_stop_autorouter();
                            break;
                        };
                        let rank = bh.get_rank(&to_restore);
                        if rank > BOARD_RANK_LIMIT {
                            stop.request_stop_autorouter();
                            break;
                        }
                        *board = to_restore;
                        self.reset_anti_oscillation_state();
                        consecutive_no_improvement_passes = 0;
                        after = stats.score(board, settings);
                        last_best_score = after.router_score;
                        log::debug!(
                            "Restoring an earlier board that has the score of {}.",
                            format_score(after.router_score, after.incomplete_count, after.clearance_violation_count)
                        );
                    }
                }
            }
            if !self.is_optimizer_autorouter {
                log::info!(
                    "Auto-routing pass #{} was completed in {:.2} seconds with score {} (routed {}, failed {}, ripped {}).",
                    current_pass,
                    pass_start.elapsed().as_secs_f64(),
                    format_score(after.router_score, after.incomplete_count, after.clearance_violation_count),
                    counters.routed,
                    counters.not_routed,
                    counters.ripped
                );
            }

            if current_pass >= STOP_AT_PASS_MINIMUM && continue_autorouting {
                if after.router_score > last_best_score + STAGNATION_SCORE_THRESHOLD {
                    consecutive_no_improvement_passes = 0;
                    last_best_score = after.router_score;
                } else {
                    consecutive_no_improvement_passes += 1;
                    if settings.is_fanout_enabled()
                        && !fanout_recovery_applied
                        && after.incomplete_count > 0
                        && consecutive_no_improvement_passes >= FANOUT_RECOVERY_STAGNATION_PASSES
                    {
                        let before = after.incomplete_count;
                        self.remove_tails(board, StopConnectionOption::None, Some(stop));
                        after = stats.score(board, settings);
                        last_best_score = after.router_score;
                        consecutive_no_improvement_passes = 0;
                        fanout_recovery_applied = true;
                        log::debug!(
                            "Applied one-time fanout recovery cleanup (removed fanout tails/vias). Incompletes: {} -> {}.",
                            before,
                            after.incomplete_count
                        );
                    }
                    if consecutive_no_improvement_passes >= STAGNATION_PASS_LIMIT {
                        log::info!(
                            "The router's score ({:.2}) has not improved by more than {} points in the last {} passes ({} item{} still unconnected). Stopping the auto-router.",
                            after.router_score as f64,
                            STAGNATION_SCORE_THRESHOLD,
                            STAGNATION_PASS_LIMIT,
                            after.incomplete_count,
                            if after.incomplete_count == 1 { "" } else { "s" }
                        );
                        stop.request_stop_autorouter();
                        break;
                    }
                }
                if after.router_score > global_best_score + STAGNATION_SCORE_THRESHOLD {
                    global_best_score = after.router_score;
                    pass_of_best_score = current_pass;
                    incomplete_count_at_best_score = after.incomplete_count;
                } else if current_pass - pass_of_best_score >= STAGNATION_PASS_LIMIT {
                    log::info!(
                        "The router's best score ({:.2}) has not improved by more than {} points since pass #{}. Stopping the auto-router after {} passes ({} item{} still unconnected).",
                        global_best_score as f64,
                        STAGNATION_SCORE_THRESHOLD,
                        pass_of_best_score,
                        current_pass,
                        incomplete_count_at_best_score,
                        if incomplete_count_at_best_score == 1 { "" } else { "s" }
                    );
                    stop.request_stop_autorouter();
                    break;
                }
            } else if after.incomplete_count == 0 && after.router_score > STAGNATION_SCORE_THRESHOLD {
                consecutive_no_improvement_passes = 0;
                last_best_score = after.router_score;
            }

            if continue_autorouting && !stop.is_stop_autorouter_requested() {
                current_pass += 1;
            }
        }

        // Finish with the best board of the history.
        let current_final = stats.score(board, settings);
        let best_history_score = bh.get_max_score();
        if best_history_score > current_final.router_score {
            if let Some(best) = bh.restore_best_board() {
                *board = best;
                let b = stats.score(board, settings);
                log::debug!(
                    "The final board state (score {}) is worse than the best board seen during routing (score {}). Restoring the best board as the final result.",
                    format_score(current_final.router_score, current_final.incomplete_count, current_final.clearance_violation_count),
                    format_score(b.router_score, b.incomplete_count, b.clearance_violation_count)
                );
            }
        }
        let was_router_run = settings.get_run_router() && settings.autorouter.max_passes.map(|m| m >= 0).unwrap_or(true);
        // Java skips the final tail removal when the autorouter was stopped (also by its own
        // stagnation rule), leaving unused fanout escapes (via + stub) on the board.
        if was_router_run && (!stop.is_stop_autorouter_requested() || ctx.enhancements) {
            self.remove_tails(board, StopConnectionOption::None, Some(stop));
        }
        if is_router_enabled {
            let s = stats.score(board, settings);
            log::info!(
                "Auto-routing stage completed: started with {} unrouted nets, completed in {:.2} seconds, final score: {}.",
                self.initial_unrouted_count,
                stage_start.elapsed().as_secs_f64(),
                format_score(s.router_score, s.incomplete_count, s.clearance_violation_count)
            );
        }
        bh.clear();
        !stop.is_stop_autorouter_requested()
    }
}

fn record_failure(board: &mut RoutingBoard, key: ItemKey, pass_no: i32, result: &AutorouteAttemptResult) {
    if log::log_enabled!(log::Level::Debug) {
        let item = board.item(key);
        let comp = if item.component_no() > 0 { board.components.get(item.component_no()).name.clone() } else { String::new() };
        let net = board.rules.nets.get(item.net_number(0)).map(|n| n.name.clone()).unwrap_or_default();
        let bb = item.bounding_box(board);
        let res = board.communication.resolution.max(1) as f64;
        log::debug!(
            "route failure pass {pass_no}: {} #{} {comp} net '{net}' at ({:.0}, {:.0}) um: {:?} {}",
            if item.is_pin() { "pin" } else if item.is_via() { "via" } else { "item" },
            item.id().0,
            (bb.ll.x as f64 + bb.ur.x as f64) / 2.0 / res,
            (bb.ll.y as f64 + bb.ur.y as f64) / 2.0 / res,
            result.state,
            result.details
        );
    }
    let RoutingBoard { basic, failure_log, .. } = board;
    failure_log.record_failure(basic, key, pass_no, result.state, &result.details);
}

/// Java `Item.hasIgnoredNets()`.
fn has_ignored_nets(board: &RoutingBoard, key: ItemKey) -> bool {
    board.item(key).net_numbers().iter().any(|&n| match board.rules.nets.get(n) {
        Some(net) => board.rules.net_classes[net.get_net_class()].is_ignored_by_autorouter,
        None => false,
    })
}

/// Java `BatchAutorouter.enforceStrictDrc(board, routeNetNo, maxItemIdBefore)`: if a trace or
/// via inserted after `max_item_id_before` on the net has a clearance violation, removes all
/// of them and returns the FAILED result.
pub fn enforce_strict_drc(board: &mut RoutingBoard, route_net_no: NetNo, max_item_id_before: i32) -> Option<AutorouteAttemptResult> {
    let mut new_items = Vec::new();
    let mut has_violation = false;
    for k in board.get_connectable_items(route_net_no) {
        let it = board.item(k);
        if it.id().0 <= max_item_id_before || !(it.is_trace() || it.is_via()) {
            continue;
        }
        new_items.push(k);
        if !has_violation && clearance_violation_count(&board.basic, k) > 0 {
            has_violation = true;
        }
    }
    if !has_violation {
        return None;
    }
    let n = new_items.len();
    board.remove_items(new_items);
    Some(AutorouteAttemptResult::with_details(
        AutorouteAttemptState::Failed,
        format!("strict_drc: connection ripped because {n} new item(s) included clearance violations"),
    ))
}
