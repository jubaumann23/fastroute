//! Java `autoroute/BoardHistory` and `BasicBoard.getHash()`.
//!
//! Java stores the serialized board (`serialize(false)`) with its MD5 "trace state" hash
//! (`getHash()`: the serialization of the traces, the vias and the item list) and its router
//! score; a restore deserializes the stored bytes (a fresh deep copy on every restore).
//!
//! The port stores a [`RoutingBoard::clone`] (the state at `add` time) and returns
//! [`RoutingBoard::deep_copy`] of it on a restore: `deep_copy` is a function of the board
//! content (it rebuilds the search trees from the item list), so this is the Java
//! serialization round trip of the board at `add` time.
//!
//! [`board_hash`] replaces the MD5 of the Java serialization by a hash of the same content:
//! every listed item (list order) with its class, id, nets, clearance class, component, fixed
//! state and geometry, plus the undo stack level. Differences to Java (documented, believed
//! not to matter for the headless flow): Java also serializes non-transient caches
//! (`Item.smallestClearance`, `DrillItem.precalculated*`) and the sharing pattern of `Line` /
//! `Point` objects, so Java can see two boards with equal content as different.

use std::cmp::Ordering;
use std::hash::{Hash, Hasher};

use fr_settings::RouterSettings;

use crate::board::{BasicBoard, ItemKind, RoutingBoard};
use crate::scoring::BoardStatistics;

/// Java `BoardHistory.MAX_HISTORY_SIZE`.
pub const MAX_HISTORY_SIZE: usize = 30;

/// Content hash of a board (the port of Java `BasicBoard.getHash()`, see the module docs).
pub fn board_hash(board: &BasicBoard) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    board.undo_journal().stack_level().hash(&mut h);
    for key in board.items.iter() {
        let it = board.item(key);
        it.id().0.hash(&mut h);
        it.net_numbers().hash(&mut h);
        it.clearance_class().hash(&mut h);
        it.component_no().hash(&mut h);
        (it.fixed_state() as u8).hash(&mut h);
        match &it.kind {
            ItemKind::Trace(t) => {
                0u8.hash(&mut h);
                t.layer.hash(&mut h);
                t.half_width.hash(&mut h);
                for line in t.polyline.lines.iter() {
                    line.a.hash(&mut h);
                    line.b.hash(&mut h);
                }
            }
            ItemKind::Via(v) => {
                1u8.hash(&mut h);
                v.padstack.hash(&mut h);
                v.center.hash(&mut h);
                v.attach_allowed.hash(&mut h);
                v.is_escape_via.hash(&mut h);
                v.escape_via_smd_layer.hash(&mut h);
            }
            ItemKind::Pin(p) => {
                2u8.hash(&mut h);
                p.pin_index.hash(&mut h);
                p.changed_to.map(|i| i.0).hash(&mut h);
            }
            ItemKind::ConductionArea(c) => {
                3u8.hash(&mut h);
                c.is_obstacle.hash(&mut h);
                c.area.layer.hash(&mut h);
            }
            ItemKind::ObstacleArea(o) => {
                4u8.hash(&mut h);
                o.layer.hash(&mut h);
            }
            ItemKind::ComponentOutline(_) => 5u8.hash(&mut h),
            ItemKind::BoardOutline(_) => 6u8.hash(&mut h),
        }
    }
    h.finish()
}

/// Java `Float.compare(a, b)`.
pub fn java_float_compare(a: f32, b: f32) -> Ordering {
    if a < b {
        return Ordering::Less;
    }
    if a > b {
        return Ordering::Greater;
    }
    let bits = |x: f32| -> i32 {
        if x.is_nan() {
            0x7fc0_0000
        } else {
            x.to_bits() as i32
        }
    };
    bits(a).cmp(&bits(b))
}

struct Entry {
    board: RoutingBoard,
    hash: u64,
    score: f32,
    restore_count: i32,
}

/// Java `BoardHistory`.
pub struct BoardHistory {
    max_history_size: usize,
    entries: Vec<Entry>,
}

impl Default for BoardHistory {
    fn default() -> Self {
        Self::new()
    }
}

impl BoardHistory {
    /// Java `new BoardHistory(routerSettings)`.
    pub fn new() -> Self {
        BoardHistory { max_history_size: MAX_HISTORY_SIZE, entries: Vec::new() }
    }

    /// fastroute: keeps at most `n` boards (every entry is a copy of the board: 30 copies of a
    /// 23 000-item board are gigabytes).
    pub fn with_max_size(n: usize) -> Self {
        BoardHistory { max_history_size: n.max(1), entries: Vec::new() }
    }

    /// Java `add(board)`. `hash` is `board_hash(board)`; `router_score` computes Java
    /// `new BoardStatistics(board).getRouterScore(routerSettings)` (called at most once).
    pub fn add_with(&mut self, board: &RoutingBoard, hash: u64, router_score: impl FnOnce() -> f32) {
        if self.contains_hash(hash) {
            return;
        }
        let mut score_fn = Some(router_score);
        let mut score: Option<f32> = None;
        if self.entries.len() >= self.max_history_size {
            let new_score = (score_fn.take().unwrap())();
            score = Some(new_score);
            let mut worst_index = 0;
            let mut worst_score = self.entries[0].score;
            for (i, e) in self.entries.iter().enumerate().skip(1) {
                if e.score < worst_score {
                    worst_score = e.score;
                    worst_index = i;
                }
            }
            if new_score <= worst_score {
                return;
            }
            self.entries.remove(worst_index);
        }
        let score = match score {
            Some(s) => s,
            None => (score_fn.take().unwrap())(),
        };
        self.entries.push(Entry { board: board.clone(), hash, score, restore_count: 0 });
    }

    /// Java `add(board)` computing the hash and the statistics itself.
    pub fn add(&mut self, board: &RoutingBoard, settings: &RouterSettings) {
        let hash = board_hash(board);
        self.add_with(board, hash, || BoardStatistics::from_board(board).get_router_score(Some(settings)));
    }

    /// Java `clear()`.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Java `contains(board)`.
    pub fn contains(&self, board: &RoutingBoard) -> bool {
        self.contains_hash(board_hash(board))
    }

    fn contains_hash(&self, hash: u64) -> bool {
        self.entries.iter().any(|e| e.hash == hash)
    }

    /// Java `remove(board)`.
    pub fn remove(&mut self, board: &RoutingBoard) {
        let hash = board_hash(board);
        if let Some(i) = self.entries.iter().position(|e| e.hash == hash) {
            self.entries.remove(i);
        }
    }

    /// Java `getMaxScore()` (0 for an empty history).
    pub fn get_max_score(&self) -> f32 {
        let mut max_score = 0.0f32;
        for e in &self.entries {
            if e.score > max_score {
                max_score = e.score;
            }
        }
        max_score
    }

    /// Java `restoreBoard(maxAllowedRestoreCount)`: sorts the history by descending score
    /// (stable) and returns a copy of the first board restored at most
    /// `max_allowed_restore_count` times (any count if `<= 0`).
    pub fn restore_board(&mut self, max_allowed_restore_count: i32) -> Option<RoutingBoard> {
        let max_allowed = if max_allowed_restore_count <= 0 { i32::MAX } else { max_allowed_restore_count };
        self.entries.sort_by(|o1, o2| java_float_compare(o2.score, o1.score));
        for e in self.entries.iter_mut() {
            if e.restore_count <= max_allowed {
                e.restore_count += 1;
                return Some(e.board.deep_copy());
            }
        }
        None
    }

    /// Java `restoreBestBoard()`.
    pub fn restore_best_board(&mut self) -> Option<RoutingBoard> {
        self.restore_board(0)
    }

    /// Java `size()`.
    pub fn size(&self) -> usize {
        self.entries.len()
    }

    /// Java `getRank(board)`: 1-based position in the (last sorted) history, -1 if absent.
    pub fn get_rank(&self, board: &RoutingBoard) -> i32 {
        let hash = board_hash(board);
        match self.entries.iter().position(|e| e.hash == hash) {
            Some(i) => i as i32 + 1,
            None => -1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn float_compare_is_java_float_compare() {
        assert_eq!(java_float_compare(1.0, 2.0), Ordering::Less);
        assert_eq!(java_float_compare(2.0, 1.0), Ordering::Greater);
        assert_eq!(java_float_compare(1.0, 1.0), Ordering::Equal);
        assert_eq!(java_float_compare(-0.0, 0.0), Ordering::Less);
        assert_eq!(java_float_compare(0.0, -0.0), Ordering::Greater);
        assert_eq!(java_float_compare(f32::NAN, f32::INFINITY), Ordering::Greater);
        assert_eq!(java_float_compare(f32::NAN, f32::NAN), Ordering::Equal);
    }
}
