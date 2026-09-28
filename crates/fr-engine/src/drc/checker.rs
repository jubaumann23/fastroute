//! Port of the routing-relevant part of `drc/DesignRulesChecker.java` and of
//! `drc/UnconnectedItems.java`. The KiCad report, JSON, summary and zone island checks are not
//! ported (GUI/API only). The `DesignRulesCheckerSettings` are not used by the ported methods.

use std::collections::{BTreeSet, HashSet};

use fr_jcompat::JavaIntHashMap;

use crate::board::{BasicBoard, ItemKey, ItemSet};
use crate::ids::NetNo;

use super::clearance_violation::{clearance_violations, ClearanceViolation};
use super::net_incompletes::{AirLine, NetIncompletes};

/// Java `DesignRulesChecker.getAllClearanceViolations()`: the violations of all items in item
/// list order, each pair of items on a layer reported once (the first report wins).
pub fn all_clearance_violations(board: &BasicBoard) -> Vec<ClearanceViolation> {
    let mut all = Vec::new();
    let mut seen: HashSet<(i32, i32, i32)> = HashSet::new();
    for key in board.get_items() {
        for v in clearance_violations(board, key) {
            let id1 = board.item(v.first_item).id().0;
            let id2 = board.item(v.second_item).id().0;
            let k = if id1 < id2 { (id1, id2, v.layer) } else { (id2, id1, v.layer) };
            if seen.insert(k) {
                all.push(v);
            }
        }
    }
    all
}

/// Java `DesignRulesChecker` (the incomplete connection state and the clearance check).
#[derive(Clone, Debug, Default)]
pub struct DesignRulesChecker {
    /// Java `maxConnections` (set by [`Self::calculate_all_incompletes`]).
    pub max_connections: i32,
    net_incompletes: Option<Vec<NetIncompletes>>,
}

impl DesignRulesChecker {
    /// Java `new DesignRulesChecker(board, drcSettings)`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Java `getAllClearanceViolations()`.
    pub fn get_all_clearance_violations(&self, board: &BasicBoard) -> Vec<ClearanceViolation> {
        all_clearance_violations(board)
    }

    /// Java `calculateAllIncompletes()`.
    pub fn calculate_all_incompletes(&mut self, board: &BasicBoard) {
        let max_net_no = board.rules.nets.max_net_number().max(0) as usize;
        let mut net_item_lists: Vec<Vec<ItemKey>> = vec![Vec::new(); max_net_no];
        for key in board.get_items() {
            let item = board.item(key);
            if item.is_connectable_class() {
                for &n in item.net_numbers() {
                    // Java: ArrayIndexOutOfBounds for net numbers outside 1..maxNetNo
                    if n >= 1 && (n as usize) <= max_net_no {
                        net_item_lists[n as usize - 1].push(key);
                    }
                }
            }
        }
        let mut max_connections = 0i32;
        for list in &net_item_lists {
            if list.is_empty() {
                continue;
            }
            let endpoints = list
                .iter()
                .filter(|k| {
                    let it = board.item(**k);
                    it.is_pin() || it.is_conduction_area()
                })
                .count() as i64;
            max_connections = max_connections.wrapping_add((endpoints - 1).max(0) as i32);
        }
        self.max_connections = max_connections;
        let incompletes = net_item_lists
            .iter()
            .enumerate()
            .map(|(i, list)| NetIncompletes::new(board, i as NetNo + 1, list))
            .collect();
        self.net_incompletes = Some(incompletes);
    }

    fn ensure(&mut self, board: &BasicBoard) -> &mut Vec<NetIncompletes> {
        if self.net_incompletes.is_none() {
            self.calculate_all_incompletes(board);
        }
        self.net_incompletes.as_mut().unwrap()
    }

    /// Java `recalculateNetIncompletes(netNumber)`.
    pub fn recalculate_net_incompletes(&mut self, board: &BasicBoard, net_number: NetNo) {
        if self.net_incompletes.is_none() {
            self.calculate_all_incompletes(board);
            return;
        }
        let list = self.net_incompletes.as_mut().unwrap();
        if net_number >= 1 && net_number as usize <= list.len() {
            let items = board.get_connectable_items(net_number);
            list[net_number as usize - 1] = NetIncompletes::new(board, net_number, &items);
        }
    }

    /// Java `recalculateNetIncompletes(netNumber, itemList)`.
    pub fn recalculate_net_incompletes_with(&mut self, board: &BasicBoard, net_number: NetNo, items: &[ItemKey]) {
        let list = self.ensure(board);
        if net_number >= 1 && net_number as usize <= list.len() {
            list[net_number as usize - 1] = NetIncompletes::new(board, net_number, items);
        }
    }

    /// Java `getIncompleteCount()`.
    pub fn get_incomplete_count(&mut self, board: &BasicBoard) -> i32 {
        self.ensure(board).iter().map(|n| n.count()).fold(0i32, |a, b| a.wrapping_add(b))
    }

    /// Java `incompleteNetNumbers()` (ascending).
    pub fn incomplete_net_numbers(&mut self, board: &BasicBoard) -> BTreeSet<NetNo> {
        self.ensure(board)
            .iter()
            .enumerate()
            .filter(|(_, n)| n.count() > 0)
            .map(|(i, _)| i as NetNo + 1)
            .collect()
    }

    /// Java `getIncompleteCount(netNumber)`.
    pub fn get_net_incomplete_count(&mut self, board: &BasicBoard, net_number: NetNo) -> i32 {
        let list = self.ensure(board);
        if net_number <= 0 || net_number as usize > list.len() {
            return 0;
        }
        list[net_number as usize - 1].count()
    }

    /// Java `getLengthViolationCount()`.
    pub fn get_length_violation_count(&mut self, board: &BasicBoard) -> i32 {
        self.ensure(board).iter().filter(|n| n.length_violation() != 0.0).count() as i32
    }

    /// Java `getLengthViolation(netNumber)`.
    pub fn get_length_violation(&mut self, board: &BasicBoard, net_number: NetNo) -> f64 {
        let list = self.ensure(board);
        if net_number <= 0 || net_number as usize > list.len() {
            return 0.0;
        }
        list[net_number as usize - 1].length_violation()
    }

    /// Java `recalculateLengthViolations()`.
    pub fn recalculate_length_violations(&mut self, board: &BasicBoard) -> bool {
        if self.net_incompletes.is_none() {
            self.calculate_all_incompletes(board);
            return true;
        }
        let mut result = false;
        for n in self.net_incompletes.as_mut().unwrap() {
            if n.calc_length_violation(board) {
                result = true;
            }
        }
        result
    }

    /// Java `getAllAirlines()` (net order, then the airline order of each net).
    pub fn get_all_airlines(&mut self, board: &BasicBoard) -> Vec<AirLine> {
        self.ensure(board).iter().flat_map(|n| n.incompletes.iter().cloned()).collect()
    }

    /// Java `getNetIncompletes(netNumber)`.
    pub fn get_net_incompletes(&mut self, board: &BasicBoard, net_number: NetNo) -> Option<&NetIncompletes> {
        let list = self.ensure(board);
        if net_number <= 0 || net_number as usize > list.len() {
            return None;
        }
        Some(&list[net_number as usize - 1])
    }

    /// Java `getAllUnconnectedItems()`. Java iterates identity `HashSet`s when choosing the
    /// representative items and listing the group items; the port uses descending id order.
    pub fn get_all_unconnected_items(&self, board: &BasicBoard) -> Vec<UnconnectedItems> {
        let mut result: Vec<UnconnectedItems> = Vec::new();
        let mut items_by_net: JavaIntHashMap<Vec<ItemKey>> = JavaIntHashMap::new();
        for key in board.get_items() {
            let item = board.item(key);
            if item.is_connectable_class() && item.net_count() > 0 {
                items_by_net.compute_if_absent(item.net_number(0), Vec::new).push(key);
            }
        }
        for (net_number, net_items) in items_by_net.iter() {
            if net_items.len() <= 1 {
                continue;
            }
            let net_item_set: HashSet<ItemKey> = net_items.iter().copied().collect();
            let mut connected_sets: Vec<ItemSet> = Vec::new();
            let mut processed: HashSet<ItemKey> = HashSet::new();
            for &k in net_items {
                if processed.contains(&k) {
                    continue;
                }
                let mut set = board.connected_set(k, net_number, false);
                set.retain(|x| net_item_set.contains(&x));
                if !set.is_empty() {
                    processed.extend(set.iter());
                    connected_sets.push(set);
                }
            }
            if connected_sets.len() >= 2 {
                let item1 = find_representative_item(board, &connected_sets[0]);
                let item2 = find_representative_item(board, &connected_sets[1]);
                if let (Some(a), Some(b)) = (item1, item2) {
                    let all: Vec<ItemKey> = connected_sets[0].iter().chain(connected_sets[1].iter()).collect();
                    result.push(UnconnectedItems { first_item: a, second_item: Some(b), all_items: all, kind: "unconnectedItems" });
                }
            }
        }
        // dangling traces
        for key in board.get_items() {
            if board.item(key).is_trace()
                && (board.trace_start_contacts(key).is_empty() || board.trace_end_contacts(key).is_empty())
                && !result.iter().any(|u| u.first_item == key)
            {
                result.push(UnconnectedItems::single(key, "track_dangling"));
            }
        }
        // dangling vias
        for key in board.get_items() {
            if board.item(key).is_via() && board.is_tail(key) {
                result.push(UnconnectedItems::single(key, "via_dangling"));
            }
        }
        result
    }
}

/// Java `findRepresentativeItem`: a pin, else a trace, else any item.
fn find_representative_item(board: &BasicBoard, set: &ItemSet) -> Option<ItemKey> {
    set.iter()
        .find(|k| board.item(*k).is_pin())
        .or_else(|| set.iter().find(|k| board.item(*k).is_trace()))
        .or_else(|| set.first())
}

/// Java `UnconnectedItems`: an unconnected net (two representative items of different groups)
/// or a dangling trace/via.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnconnectedItems {
    pub first_item: ItemKey,
    pub second_item: Option<ItemKey>,
    pub all_items: Vec<ItemKey>,
    /// Java `type`: "unconnectedItems", "track_dangling" or "via_dangling".
    pub kind: &'static str,
}

impl UnconnectedItems {
    fn single(item: ItemKey, kind: &'static str) -> Self {
        // Java: Arrays.asList(firstItem, null)
        UnconnectedItems { first_item: item, second_item: None, all_items: vec![item], kind }
    }
}
