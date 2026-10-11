//! pcbkit recovery stage (`router.recover_attempts`, default off): targeted rip-up-and-reroute of the connections
//! the routing stage left open.
//!
//! The batch loop gives a failing connection the same rip-up cost as every other connection of that pass, and that
//! cost grows with the pass number (`start_ripup_costs * pass`), so late passes mostly flip the same few nets back and
//! forth (A rips B, B rips A) or leave the connection to fail for good. This stage works on the final board instead
//! and only ever keeps an improvement:
//!
//! 1. for each connection still open (in item order) and each attempt `a` (up to `router.recover_attempts`), copy the
//!    board and route that one connection with a rip-up cost that rises with `a` (`1 + a / 2` times the first
//!    pass's), randomised by a deterministic noise that differs per attempt (`route_connection_noisy`), so the
//!    attempts rip different neighbours;
//! 2. the connections it ripped are then re-routed in sweeps over the open connections, in a deterministic shuffled
//!    order (seeded by the attempt and the item), with the ordinary rip-up cost of a mid-run pass, no net-wide rip;
//! 3. the copy replaces the board only if it has fewer open connections and no more clearance violations.
//!
//! Everything is a function of the board and the setting (counts, no clock, no threads), so the result stays
//! deterministic. Each accepted attempt lowers the open count, so the rounds end.

use std::collections::HashMap;

use fr_settings::RouterSettings;

use crate::autoroute::router::{route_connection_noisy, RecoverTuning};
use crate::board::{AutorouteAttemptState, ItemSet, RoutingBoard, StopConnectionOption};
use crate::ids::ItemId;

use super::{incomplete_count, BatchAutorouter, ConnectionRouterParams, PipelineContext, StatsCache};

/// Sweeps over the open connections after a forced connection (stops early when a sweep gains nothing).
const SETTLE_SWEEPS: usize = 3;
/// Rip-up pass number of the re-routing sweeps (the cost of an ordinary mid-run pass).
const SETTLE_PASS_NO: i32 = 3;
/// Attempts per connection of the strict-width last resort (it only breaks strict width where strict recovery failed).
const LAST_RESORT_ATTEMPTS: u32 = 2;
/// A hard bound on the rounds over the open connections (each accepted attempt already lowers the open count).
const MAX_ROUNDS: usize = 6;

/// The outcome of [`BatchAutorouter::recover`], for the log and the tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RecoverReport {
    pub open_before: i32,
    pub open_after: i32,
    pub attempts_run: u32,
    pub attempts_kept: u32,
}

/// The `n`-th rip-up noise number: 4, 5, 7, 8, 10, ... (`ripup_pass_no >= 4 && % 3 != 0` switches the noise on).
fn noise_pass_no(n: u32) -> i32 {
    (4..).filter(|v| v % 3 != 0).nth(n as usize).expect("unbounded iterator")
}

/// The rip-up pass number (cost scale) of attempt `a`.
fn attempt_cost_scale(a: u32) -> i32 {
    1 + (a / 2) as i32
}

impl BatchAutorouter {
    /// Runs the recovery stage on `board` (see the module doc). Returns what it did.
    pub fn recover(&mut self, board: &mut RoutingBoard, settings: &RouterSettings, ctx: &PipelineContext) -> RecoverReport {
        let attempts = settings.get_recover_attempts();
        let ignored = super::super::stats::ignored_incomplete_count(board);
        let mut report = RecoverReport { open_before: incomplete_count(board, None), ..RecoverReport::default() };
        report.open_after = report.open_before;
        if attempts == 0 || report.open_before - ignored <= 0 {
            return report;
        }
        let params = self.params();
        // a connection that fails at full width is retried at the fab's minimum trace width (the same
        // `retry_connection_necked` a set `router.neck_width_um` turns on for the whole run, here only for the
        // attempts of this stage); `router.min_trace_width_um` unset leaves the settings as they are
        let necked = Self::necked_settings(settings);
        self.recover_rounds(board, &necked, attempts, ignored, &params, ctx, &mut report);
        // pcbkit W2: a connection strict width left open gets a last attempt that may break it (the whole
        // attempt, rip-ups included, runs without strict width: a neighbour ripped for it must be able to neck too)
        if settings.is_strict_width() && settings.is_strict_last_resort() && report.open_after - ignored > 0 {
            let mut loose = settings.clone();
            loose.strict_width = None;
            let loose = Self::necked_settings(&loose);
            self.recover_rounds(board, &loose, attempts.min(LAST_RESORT_ATTEMPTS), ignored, &params, ctx, &mut report);
        }
        log::info!(
            "Recovery: {} -> {} unrouted connections ({} attempts, {} kept).",
            report.open_before,
            report.open_after,
            report.attempts_run,
            report.attempts_kept
        );
        report
    }

    /// The rounds of [`Self::recover`] over the connections still open.
    #[allow(clippy::too_many_arguments)]
    fn recover_rounds(
        &mut self,
        board: &mut RoutingBoard,
        settings: &RouterSettings,
        attempts: u32,
        ignored: i32,
        params: &ConnectionRouterParams,
        ctx: &PipelineContext,
        report: &mut RecoverReport,
    ) {
        for _round in 0..MAX_ROUNDS {
            let mut progress = false;
            let open = Self::get_autoroute_items(board);
            let ids: Vec<ItemId> = open.iter().map(|&k| board.item(k).id()).collect();
            for id in ids {
                if ctx.stop.is_stop_requested() || report.open_after - ignored <= 0 {
                    break;
                }
                for attempt in 0..attempts {
                    if ctx.stop.is_stop_requested() {
                        break;
                    }
                    // an earlier attempt may have connected this item on its way
                    let Some(key) = board.get_item(id) else { break };
                    if board.item(key).net_count() != 1 || !Self::get_autoroute_items(board).contains(&key) {
                        break;
                    }
                    report.attempts_run += 1;
                    let mut trial = board.clone_for_worker();
                    let worth_another = self.recover_attempt(&mut trial, settings, params, id, attempt, ctx);
                    let before = StatsCache::new().score(board, settings);
                    let after = StatsCache::new().score(&trial, settings);
                    if after.incomplete_count < before.incomplete_count && after.clearance_violation_count <= before.clearance_violation_count {
                        log::debug!(
                            "Recovery: item #{} attempt {attempt} kept ({} -> {} open).",
                            id.0,
                            before.incomplete_count,
                            after.incomplete_count
                        );
                        *board = trial;
                        report.attempts_kept += 1;
                        report.open_after = after.incomplete_count;
                        progress = true;
                        break;
                    }
                    if !worth_another {
                        break;
                    }
                }
            }
            if !progress {
                break;
            }
        }
    }

    /// `settings`, with the neck-down retry width set to the minimum trace width unless one is set.
    fn necked_settings(settings: &RouterSettings) -> RouterSettings {
        let mut s = settings.clone();
        let min = settings.get_min_trace_width_um();
        if s.get_neck_width_um() <= 0.0 && min > 0.0 {
            s.neck_width_um = Some(min);
        }
        s
    }

    /// One attempt on `trial`: forces the connection of item `id` in, then re-routes what that ripped.
    /// Returns false when more attempts at this connection are futile: the search ran to the end without a way through
    /// (rip-up costs and noise only reorder the search, they cannot open a way that is not there).
    fn recover_attempt(&mut self, trial: &mut RoutingBoard, settings: &RouterSettings, params: &ConnectionRouterParams, id: ItemId, attempt: u32, ctx: &PipelineContext) -> bool {
        let stop = &ctx.stop;
        let Some(key) = trial.get_item(id) else { return false };
        let net = trial.item(key).net_number(0);
        trial.start_marking_changed_area();
        let mut ripped = ItemSet::new();
        let forced = route_connection_noisy(
            trial,
            settings,
            params,
            key,
            net,
            &mut ripped,
            None,
            attempt_cost_scale(attempt),
            Some(&RecoverTuning { noise_pass_no: noise_pass_no(attempt), rip_zone_vias: true }),
            Some(stop),
        );
        if forced.result.state != AutorouteAttemptState::Routed {
            return forced.result.details.contains("expansion cap");
        }
        let mut rnd = fr_jcompat::random::JavaRandom::new((attempt as i64).wrapping_mul(7919).wrapping_add(id.0 as i64));
        let mut left = incomplete_count(trial, None);
        for _sweep in 0..SETTLE_SWEEPS {
            let mut open = Self::get_autoroute_items(trial);
            if open.is_empty() || stop.is_stop_requested() {
                break;
            }
            open.sort_by_key(|&k| (trial.item(k).net_number(0), trial.item(k).id().0));
            fr_jcompat::random::shuffle(&mut open, &mut rnd);
            let ids: Vec<ItemId> = open.iter().map(|&k| trial.item(k).id()).collect();
            for sid in ids {
                if stop.is_stop_requested() {
                    break;
                }
                let Some(k) = trial.get_item(sid) else { continue };
                if trial.item(k).net_count() != 1 {
                    continue;
                }
                let n = trial.item(k).net_number(0);
                trial.start_marking_changed_area();
                let mut r = ItemSet::new();
                let mut costs: HashMap<_, i32> = HashMap::new();
                route_connection_noisy(trial, settings, params, k, n, &mut r, Some(&mut costs), SETTLE_PASS_NO, None, Some(stop));
            }
            let option = if self.remove_unconnected_vias { StopConnectionOption::None } else { StopConnectionOption::FanoutVia };
            self.remove_tails(trial, option, Some(stop));
            let now = incomplete_count(trial, None);
            if now == 0 || now >= left {
                break;
            }
            left = now;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_numbers_enable_the_noise_and_skip_multiples_of_three() {
        let v: Vec<i32> = (0..6).map(noise_pass_no).collect();
        assert_eq!(v, vec![4, 5, 7, 8, 10, 11]);
        assert!(v.iter().all(|&n| n >= 4 && n % 3 != 0));
    }

    #[test]
    fn the_rip_up_cost_scale_rises_with_the_attempt() {
        let s: Vec<i32> = (0..6).map(attempt_cost_scale).collect();
        assert_eq!(s, vec![1, 1, 2, 2, 3, 3]);
    }
}
