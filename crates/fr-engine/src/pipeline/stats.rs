//! Memoized board statistics for the pipeline.
//!
//! Java recomputes `new BoardStatistics(board)` (a full-board DRC) many times per pass, often
//! on an unchanged board. [`BoardStatistics`] is a pure function of the board content, so the
//! pipeline keeps the last result keyed by [`board_hash`](super::history::board_hash).

use fr_settings::RouterSettings;

use crate::board::BasicBoard;
use crate::drc::DesignRulesChecker;
use crate::ids::NetNo;
use crate::scoring::BoardStatistics;

use super::history::board_hash;

/// One-entry cache of the full statistics (`new BoardStatistics(board)`).
#[derive(Default)]
pub struct StatsCache {
    last: Option<(u64, BoardStatistics)>,
}

/// The values of a full statistics used by the pipeline.
#[derive(Clone, Copy, Debug)]
pub struct Score {
    pub router_score: f32,
    pub optimizer_score: f32,
    pub incomplete_count: i32,
    pub clearance_violation_count: i32,
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
        let s = self.statistics(board);
        Score {
            router_score: s.get_router_score(Some(settings)),
            optimizer_score: s.get_optimizer_score(Some(settings)),
            incomplete_count: s.connections.incomplete_count.unwrap_or(0),
            clearance_violation_count: s.clearance_violations.total_count.unwrap_or(0),
        }
    }
}

/// Java `BatchAutorouter.calculateIncompleteCount(board, incompleteNets)`.
pub fn incomplete_count(board: &BasicBoard, incomplete_nets: Option<&mut std::collections::BTreeSet<NetNo>>) -> i32 {
    let mut drc = DesignRulesChecker::new();
    drc.calculate_all_incompletes(board);
    if let Some(nets) = incomplete_nets {
        nets.extend(drc.incomplete_net_numbers(board));
    }
    drc.get_incomplete_count(board)
}

/// Java `FRLogger.formatScore(score, incomplete, violations)`.
pub fn format_score(score: f32, incomplete: i32, violations: i32) -> String {
    format!(
        "{:.2} ({} unrouted and {} {})",
        score as f64,
        incomplete,
        violations,
        if violations == 1 { "violation" } else { "violations" }
    )
}
