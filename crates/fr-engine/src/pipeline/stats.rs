//! Memoized board statistics for the pipeline.
//!
//! Java recomputes `new BoardStatistics(board)` (a full-board DRC) many times per pass, often
//! on an unchanged board. [`BoardStatistics`] is a pure function of the board content, so the
//! pipeline keeps the last result keyed by [`board_hash`](super::history::board_hash).

use std::cell::RefCell;
use std::collections::HashMap;

use fr_settings::RouterSettings;

use crate::board::connectivity::verify_caches;
use crate::board::{BasicBoard, ItemKey};
use crate::drc::NetIncompletes;
use crate::ids::NetNo;
use crate::scoring::BoardStatistics;

use super::history::board_hash;

/// One-entry cache of the full statistics (`new BoardStatistics(board)`).
#[derive(Default)]
pub struct StatsCache {
    last: Option<(u64, BoardStatistics)>,
    /// fastroute: report the optimizer score without the clamp at 0.
    pub unclamped_optimizer_score: bool,
}

/// The values of a full statistics used by the pipeline.
#[derive(Clone, Copy, Debug)]
pub struct Score {
    pub router_score: f32,
    pub optimizer_score: f32,
    pub incomplete_count: i32,
    pub clearance_violation_count: i32,
    /// fastroute: violations between two unroutable (fixed) items, which the router cannot
    /// fix: part of `clearance_violation_count`.
    pub unfixable_violation_count: i32,
}

impl StatsCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// `new BoardStatistics(board)` (cached by content hash).
    pub fn statistics(&mut self, board: &BasicBoard) -> &mut BoardStatistics {
        let hash = board_hash(board);
        let hit = matches!(&self.last, Some((h, _)) if *h == hash);
        if !hit {
            self.last = Some((hash, BoardStatistics::from_board(board)));
        }
        &mut self.last.as_mut().unwrap().1
    }

    /// The scores of `new BoardStatistics(board)`.
    pub fn score(&mut self, board: &BasicBoard, settings: &RouterSettings) -> Score {
        let unclamped = self.unclamped_optimizer_score;
        let s = self.statistics(board);
        Score {
            router_score: s.get_router_score(Some(settings)),
            optimizer_score: if unclamped {
                s.get_optimizer_score_unclamped(Some(settings))
            } else {
                s.get_optimizer_score(Some(settings))
            },
            incomplete_count: s.connections.incomplete_count.unwrap_or(0),
            clearance_violation_count: s.clearance_violations.total_count.unwrap_or(0),
            unfixable_violation_count: s.clearance_violations.unfixable_count.unwrap_or(0),
        }
    }
}

/// Java `BatchAutorouter.calculateIncompleteCount(board, incompleteNets)`.
///
/// The incomplete count of a net (`NetIncompletes.count()`) depends only on the connectable
/// items of the net (their contacts are items of the same net), so it is memoized per net,
/// keyed by the keys and content versions of these items (see [`ItemRepository::version`]).
/// The optimizer counts the incompletes before and after every candidate, on boards that
/// differ in a few nets only.
///
/// [`ItemRepository::version`]: crate::board::ItemRepository::version
pub fn incomplete_count(board: &BasicBoard, incomplete_nets: Option<&mut std::collections::BTreeSet<NetNo>>) -> i32 {
    // the net item lists of `DesignRulesChecker.calculateAllIncompletes`
    let max_net_no = board.rules.nets.max_net_number().max(0) as usize;
    let mut net_item_lists: Vec<Vec<ItemKey>> = vec![Vec::new(); max_net_no];
    for key in board.items.iter() {
        let item = board.item(key);
        if item.is_connectable_class() {
            for &n in item.net_numbers() {
                if n >= 1 && (n as usize) <= max_net_no {
                    net_item_lists[n as usize - 1].push(key);
                }
            }
        }
    }
    let global = board.items.global_epoch();
    let mut total = 0i32;
    let mut incomplete: Vec<NetNo> = Vec::new();
    NET_COUNTS.with(|memo| {
        let mut memo = memo.borrow_mut();
        for (i, list) in net_item_lists.iter().enumerate() {
            let net_no = i as NetNo + 1;
            let key: Vec<(ItemKey, u64)> = list.iter().map(|&k| (k, board.items.version(k))).collect();
            let entries = memo.entry(net_no).or_default();
            let cached = entries.iter().find(|e| e.global == global && e.items == key).map(|e| e.count);
            let count = match cached {
                Some(c) => {
                    if verify_caches() {
                        assert_eq!(c, NetIncompletes::new(board, net_no, list).count(), "net incompletes cache out of date");
                    }
                    c
                }
                None => {
                    let c = NetIncompletes::new(board, net_no, list).count();
                    if entries.len() >= NET_COUNT_SLOTS {
                        entries.remove(0);
                    }
                    entries.push(NetCount { global, items: key, count: c });
                    c
                }
            };
            total = total.wrapping_add(count);
            if count > 0 {
                incomplete.push(net_no);
            }
        }
    });
    if let Some(nets) = incomplete_nets {
        nets.extend(incomplete);
    }
    total
}

struct NetCount {
    global: u64,
    items: Vec<(ItemKey, u64)>,
    count: i32,
}

/// Entries kept per net (the optimizer alternates between the baseline and a candidate).
const NET_COUNT_SLOTS: usize = 3;

thread_local! {
    static NET_COUNTS: RefCell<HashMap<NetNo, Vec<NetCount>>> = RefCell::new(HashMap::new());
}

/// Java `FRLogger.formatScore(score, incomplete, violations)`.
pub fn format_score(score: f32, incomplete: i32, violations: i32) -> String {
    format_score_with_unfixable(score, incomplete, violations, 0)
}

/// [`format_score`] naming the violations between fixed items (which no pass can fix).
pub fn format_score_with_unfixable(score: f32, incomplete: i32, violations: i32, unfixable: i32) -> String {
    format!(
        "{:.2} ({} unrouted and {} {}{})",
        score as f64,
        incomplete,
        violations,
        if violations == 1 { "violation" } else { "violations" },
        if unfixable > 0 { format!(", {unfixable} of them pre-existing between fixed items") } else { String::new() }
    )
}

/// fastroute: the part of [`incomplete_count`] on nets whose class the autorouter ignores
/// (`autorouter.ignore_net_classes`): these connections are never routed, so they must not
/// count as work left (multi-start, the pass rollback and stagnation thresholds) and are
/// reported separately.
pub fn ignored_incomplete_count(board: &BasicBoard) -> i32 {
    let ignored: std::collections::HashSet<NetNo> = (1..=board.rules.nets.max_net_number())
        .filter(|&n| board.rules.nets.get(n).is_some_and(|net| board.rules.net_classes[net.get_net_class()].is_ignored_by_autorouter))
        .collect();
    if ignored.is_empty() {
        return 0;
    }
    let mut items: HashMap<NetNo, Vec<ItemKey>> = HashMap::new();
    for key in board.items.iter() {
        let item = board.item(key);
        if item.is_connectable_class() {
            for &n in item.net_numbers() {
                if ignored.contains(&n) {
                    items.entry(n).or_default().push(key);
                }
            }
        }
    }
    let mut nets: Vec<NetNo> = items.keys().copied().collect();
    nets.sort_unstable();
    nets.iter().map(|n| NetIncompletes::new(board, *n, &items[n]).count()).sum()
}
