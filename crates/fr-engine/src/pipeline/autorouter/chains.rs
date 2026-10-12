//! pcbkit rip-up chains (`router.chain_depth`, default off): a deterministic local search for the connections the
//! routing stage and recovery left open.
//!
//! On a congested (not capacity-limited) placement every open connection routes alone if it may rip one or two
//! neighbours, but the batch passes and the recovery sweeps re-route those neighbours with ordinary costs and without
//! any memory of why they were ripped, so a neighbour rips the connection back (A rips B, B rips A). A chain instead:
//!
//! 1. routes the open connection `c` with rip-up, as a min-cost search over the board (the ripped items of the maze
//!    search are the smallest set of blockers by rip-up cost); several cost variants (doubling cost scale, deterministic noise)
//!    give different blocker sets, tried in a fixed order; a variant that rips more than [`MAX_VICTIMS`] connections is
//!    rejected;
//! 2. protects everything the chain has routed (items created after it started may not be ripped again) and re-routes
//!    each ripped connection `b` on the new board: first without rip-up, then, above the depth limit, as a chain of its
//!    own (it may rip unprotected neighbours, which are re-routed in turn);
//! 3. keeps a level only if the open count strictly drops; the whole chain replaces the board only if it leaves fewer
//!    connections open and no more clearance violations (else the board copy is dropped).
//!
//! Work is bounded by counts, never by a clock: variants per level, [`MAX_VICTIMS`], the depth, [`MAX_ROUNDS`] and
//! `router.chain_max_routes` (every connection search counts one). No threads, no hash order: deterministic.
//!
//! A chain that failed is not tried again until another chain was kept: the board is the same, so it would fail the
//! same way. Two opt-in refinements (pcbkit R9):
//! - `router.chain_reverse`: the maze search is not complete (a connection it cannot find from the rest of the net
//!   may be found from the item's end), so a depth-0 search that finds no way is repeated in reverse;
//! - `router.chain_prune`: a variant ends at the first ripped connection that stays open after both of its ends were
//!   tried (the chain needs them all), and a variant that rips the same connections as an earlier one of its level
//!   is skipped. On cypress_small8 energy-2 this cuts the stage from about 2000 to 550-700 routes at the same result.
//!
//! With strict width and its last resort (`router.strict_last_resort`) the chains route without strict width: they
//! run after the strict recovery and its loose last resort, and a chain held to the class width finds no way there.

use fr_settings::RouterSettings;

use crate::autoroute::router::{route_connection_noisy, RecoverTuning};
use crate::board::{AutorouteAttemptState, ItemSet, RoutingBoard, StopConnectionOption};
use crate::datastructures::{IdGenerator, StopToken};
use crate::ids::ItemId;

use super::recovery::{noise_pass_no, RouteBudget};
use super::{incomplete_count, BatchAutorouter, ConnectionRouterParams, PipelineContext, StatsCache};

/// Rip-up variants of the open connection itself (depth 0).
const VARIANTS_TOP: u32 = 4;
/// Rip-up variants of a ripped connection (depth >= 1).
const VARIANTS_DEEP: u32 = 2;
/// The most connections one rip-up may open (the brief's "1-2, at most 3" blockers).
pub(super) const MAX_VICTIMS: i32 = 3;
/// Rounds over the open connections (a round without a kept chain ends the stage).
const MAX_ROUNDS: usize = 6;

/// The outcome of [`BatchAutorouter::chains`], for the log and the tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ChainReport {
    pub open_before: i32,
    pub open_after: i32,
    pub chains_tried: u32,
    pub chains_kept: u32,
    /// Connection searches of the stage (what `router.chain_max_routes` caps).
    pub routes: u64,
    /// True when the stage ended at `router.chain_max_routes`.
    pub capped: bool,
    /// Why rip-up variants failed: the search found no way (`no_way`), it ripped more than [`MAX_VICTIMS`] connections
    /// (`too_many`), or its ripped connections could not all be re-routed (`unsettled`); all depths.
    pub no_way: u32,
    pub too_many: u32,
    pub unsettled: u32,
}

/// What every level of one chain shares.
struct Chain<'a> {
    settings: &'a RouterSettings,
    params: &'a ConnectionRouterParams,
    stop: &'a StopToken,
    max_depth: u32,
    /// `router.chain_reverse`: a search that finds no way is tried again from the item's own end.
    reverse: bool,
    /// `router.chain_prune`: a variant ends at its first ripped connection that cannot be re-routed, and a variant
    /// that rips the same connections as an earlier one of the same level is skipped.
    prune: bool,
    /// Items with a larger id were routed by this chain and may not be ripped.
    protect_above: i32,
    budget: &'a mut RouteBudget,
    /// The last search stopped early (expansion cap or budget), so another variant may still find a way.
    capped: bool,
    /// Failure counts for the report.
    no_way: u32,
    too_many: u32,
    unsettled: u32,
}

/// How one connection search of a chain may rip.
#[derive(Clone, Copy, Debug)]
enum Rip {
    /// No rip-up at all.
    None,
    /// Rip-up variant `v`: 0 = plain min-cost, then cost scale and noise vary.
    Variant(u32),
}

impl Rip {
    fn tuning(self, protect_above: i32) -> (i32, RecoverTuning) {
        match self {
            Rip::None => (1, RecoverTuning { noise_pass_no: 1, rip_zone_vias: false, rip_protect_above: Some(i32::MIN), reverse: false }),
            Rip::Variant(v) => {
                // pass number 1 switches the noise off; 4, 5, 7, ... switch it on with different seeds
                let noise = if v == 0 { 1 } else { noise_pass_no(v - 1) };
                (variant_cost_scale(v), RecoverTuning { noise_pass_no: noise, rip_zone_vias: true, rip_protect_above: Some(protect_above), reverse: false })
            }
        }
    }
}

/// The rip-up cost scale of variant `v`: 1, 2, 4, 8, ... (a dearer rip-up makes the search detour rather than rip, so
/// later variants rip fewer connections).
fn variant_cost_scale(v: u32) -> i32 {
    1 << v.min(10)
}

/// The ids of the open single-net items (see [`BatchAutorouter::get_autoroute_items`]), in item order.
fn open_ids(board: &RoutingBoard) -> Vec<ItemId> {
    BatchAutorouter::get_autoroute_items(board).into_iter().filter(|&k| board.item(k).net_count() == 1).map(|k| board.item(k).id()).collect()
}

/// True when item `id` still has an unconnected item of its net.
fn still_open(board: &RoutingBoard, id: ItemId) -> bool {
    let Some(key) = board.get_item(id) else { return false };
    let item = board.item(key);
    item.net_count() == 1 && !board.unconnected_set(key, item.net_number(0)).is_empty()
}

impl BatchAutorouter {
    /// Runs the chain stage on `board` (see the module doc). Returns what it did.
    pub fn chains(&mut self, board: &mut RoutingBoard, settings: &RouterSettings, ctx: &PipelineContext) -> ChainReport {
        let max_depth = settings.get_chain_depth();
        let ignored = super::super::stats::ignored_incomplete_count(board);
        let mut report = ChainReport { open_before: incomplete_count(board, None), ..ChainReport::default() };
        report.open_after = report.open_before;
        if max_depth == 0 || report.open_before - ignored <= 0 {
            return report;
        }
        let params = self.params();
        // strict width with its last resort (W2): the chains come after the strict recovery and its loose last resort,
        // so they are a last resort too and route without strict width (a chain at strict width is cut off where the
        // class width does not fit, which is why these connections are still open)
        let necked = if settings.is_strict_width() && settings.is_strict_last_resort() {
            let mut loose = settings.clone();
            loose.strict_width = None;
            Self::necked_settings(&loose)
        } else {
            Self::necked_settings(settings)
        };
        let mut budget = RouteBudget::new(settings.get_chain_max_routes());
        // the number of kept chains when an item's chain last failed: the board is unchanged until the next kept chain,
        // and the search is deterministic, so a chain that failed on it would fail again (and only cost routes)
        let mut failed_at: std::collections::HashMap<i32, u32> = std::collections::HashMap::new();
        for _round in 0..MAX_ROUNDS {
            let mut progress = false;
            for id in open_ids(board) {
                if failed_at.get(&id.0) == Some(&report.chains_kept) {
                    continue;
                }
                if ctx.stop.is_stop_requested() || report.open_after - ignored <= 0 || budget.exhausted() {
                    break;
                }
                // an earlier chain may have connected this item on its way
                if !still_open(board, id) {
                    continue;
                }
                report.chains_tried += 1;
                let mut trial = board.clone_for_worker();
                let protect_above = IdGenerator::max_generated_id(&trial.communication.id_generator);
                let mut chain = Chain { settings: &necked, params: &params, stop: &ctx.stop, max_depth, reverse: settings.is_chain_reverse(), prune: settings.is_chain_prune(), protect_above, budget: &mut budget, capped: false, no_way: 0, too_many: 0, unsettled: 0 };
                let kept = self.chain_route(&mut trial, &mut chain, id, 0);
                report.no_way += chain.no_way;
                report.too_many += chain.too_many;
                report.unsettled += chain.unsettled;
                if !kept {
                    failed_at.insert(id.0, report.chains_kept);
                    continue;
                }
                let option = if self.remove_unconnected_vias { StopConnectionOption::None } else { StopConnectionOption::FanoutVia };
                self.remove_tails(&mut trial, option, Some(&ctx.stop));
                let before = StatsCache::new().score(board, settings);
                let after = StatsCache::new().score(&trial, settings);
                if after.incomplete_count < before.incomplete_count && after.clearance_violation_count <= before.clearance_violation_count {
                    log::debug!("Chains: item #{} kept ({} -> {} open).", id.0, before.incomplete_count, after.incomplete_count);
                    *board = trial;
                    report.chains_kept += 1;
                    report.open_after = after.incomplete_count;
                    progress = true;
                } else {
                    failed_at.insert(id.0, report.chains_kept);
                }
            }
            if !progress || budget.exhausted() || ctx.stop.is_stop_requested() {
                break;
            }
        }
        report.routes = budget.used;
        report.capped = budget.exhausted();
        log::info!(
            "Chains: {} -> {} unrouted connections ({} chains, {} kept, {} routes{}; failed variants: {} no way, {} too many ripped, {} not re-routed).",
            report.open_before,
            report.open_after,
            report.chains_tried,
            report.chains_kept,
            report.routes,
            if report.capped { ", stopped at router.chain_max_routes" } else { "" },
            report.no_way,
            report.too_many,
            report.unsettled
        );
        report
    }

    /// Routes the connection of item `id` on `board` at chain depth `depth`. True (and `board` changed) when the open
    /// count of `board` strictly dropped; false leaves `board` as it was.
    fn chain_route(&mut self, board: &mut RoutingBoard, chain: &mut Chain<'_>, id: ItemId, depth: u32) -> bool {
        if !still_open(board, id) {
            return false;
        }
        let open0 = incomplete_count(board, None);
        // 1. alone, ripping nothing
        if let Some(t) = self.chain_search(board, chain, id, Rip::None, depth) {
            if incomplete_count(&t, None) < open0 {
                *board = t;
                return true;
            }
        }
        if depth >= chain.max_depth {
            return false;
        }
        // 2. rip-up variants, each judged with its ripped connections re-routed
        let before: Vec<ItemId> = open_ids(board);
        let variants = if depth == 0 { VARIANTS_TOP } else { VARIANTS_DEEP };
        // `router.chain_prune`: the victim sets of the variants tried so far (a variant that rips the same
        // connections as an earlier, failed one is not re-routed again)
        let mut tried_victims: Vec<Vec<ItemId>> = Vec::new();
        for v in 0..variants {
            if chain.stop.is_stop_requested() || chain.budget.exhausted() {
                break;
            }
            let Some(mut t) = self.chain_search(board, chain, id, Rip::Variant(v), depth) else {
                // only the expansion cap or the budget stop a search early; costs cannot open a way that is not there
                chain.no_way += 1;
                if chain.capped {
                    continue;
                }
                break;
            };
            let now = incomplete_count(&t, None);
            if now < open0 {
                *board = t;
                return true;
            }
            if now - (open0 - 1) > MAX_VICTIMS {
                chain.too_many += 1;
                continue;
            }
            let mut victims: Vec<ItemId> = open_ids(&t).into_iter().filter(|v| !before.contains(v)).collect();
            victims.sort_by_key(|&vid| t.get_item(vid).map(|k| (t.item(k).net_number(0), vid.0)).unwrap_or((i32::MAX, vid.0)));
            if chain.prune {
                if tried_victims.contains(&victims) {
                    chain.unsettled += 1;
                    continue;
                }
                tried_victims.push(victims.clone());
            }
            let net_of = |b: &RoutingBoard, vid: ItemId| b.get_item(vid).map(|k| b.item(k).net_number(0));
            for (i, &vid) in victims.iter().enumerate() {
                if chain.stop.is_stop_requested() || chain.budget.exhausted() {
                    break;
                }
                self.chain_route(&mut t, chain, vid, depth + 1);
                // `router.chain_prune`: every ripped connection has to be re-routed for the open count to drop, so a
                // net still open after the last of its items (both ends of a ripped connection are tried: the search
                // is not symmetric) ends this variant
                let net = net_of(&t, vid);
                let last_of_net = victims.get(i + 1).map_or(true, |&next| net_of(&t, next) != net);
                if chain.prune && last_of_net && victims[..=i].iter().any(|&v| net_of(&t, v) == net && still_open(&t, v)) {
                    break;
                }
            }
            if incomplete_count(&t, None) < open0 {
                *board = t;
                return true;
            }
            chain.unsettled += 1;
        }
        false
    }

    /// One connection search for item `id` on a copy of `board`: the routed copy, or `None` (then `chain.capped` tells a
    /// search stopped by the expansion cap or the spent budget from one that ran to the end).
    fn chain_search(&self, board: &RoutingBoard, chain: &mut Chain<'_>, id: ItemId, rip: Rip, depth: u32) -> Option<RoutingBoard> {
        let found = self.chain_search_dir(board, chain, id, rip, false);
        // `router.chain_reverse`, only for the open connection itself (depth 0; a ripped neighbour that fails is left
        // to its own rip-up): the maze search is not complete, so a search that finds no way from the rest of the net
        // is tried again from the item's own end
        if found.is_some() || chain.capped || !chain.reverse || depth > 0 {
            return found;
        }
        self.chain_search_dir(board, chain, id, rip, true)
    }

    /// [`Self::chain_search`] in one direction (`reverse`: from the item's connected set to the rest of its net).
    fn chain_search_dir(&self, board: &RoutingBoard, chain: &mut Chain<'_>, id: ItemId, rip: Rip, reverse: bool) -> Option<RoutingBoard> {
        chain.capped = false;
        if !chain.budget.take() {
            chain.capped = true;
            return None;
        }
        let key = board.get_item(id)?;
        if board.item(key).net_count() != 1 {
            return None;
        }
        let net = board.item(key).net_number(0);
        let (scale, mut tuning) = rip.tuning(chain.protect_above);
        tuning.reverse = reverse;
        let mut t = board.clone_for_worker();
        t.start_marking_changed_area();
        let mut ripped = ItemSet::new();
        let out = route_connection_noisy(&mut t, chain.settings, chain.params, key, net, &mut ripped, None, scale, Some(&tuning), Some(chain.stop));
        if out.result.state != AutorouteAttemptState::Routed {
            chain.capped = out.result.details.contains("expansion cap");
            return None;
        }
        Some(t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variant_zero_is_the_plain_min_cost_search_and_later_variants_add_noise_and_cost() {
        let (s0, t0) = Rip::Variant(0).tuning(7);
        assert_eq!((s0, t0.noise_pass_no, t0.rip_protect_above), (1, 1, Some(7)));
        let noises: Vec<i32> = (1..4).map(|v| Rip::Variant(v).tuning(7).1.noise_pass_no).collect();
        assert!(noises.iter().all(|&n| n >= 4 && n % 3 != 0), "noise on: {noises:?}");
        let scales: Vec<i32> = (0..4).map(|v| Rip::Variant(v).tuning(7).0).collect();
        assert_eq!(scales, vec![1, 2, 4, 8], "the cost scale doubles");
        let (_, none) = Rip::None.tuning(7);
        assert_eq!(none.rip_protect_above, Some(i32::MIN), "no rip-up: every item is protected");
    }
}
