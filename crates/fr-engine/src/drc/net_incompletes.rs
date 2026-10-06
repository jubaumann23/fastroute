//! Port of `drc/NetIncompletes.java` and `drc/AirLine.java`: the incomplete connections
//! (airlines) of one net, from a Delaunay triangulation of the net items' ratsnest corners and a
//! Kruskal pass over the triangulation edges sorted by length.
//!
//! # Ordering (documented deviation)
//!
//! Java groups the net items by iterating an identity `HashSet<Item>` (`calculateNetItems`), so
//! the order of the grouped items — and with it the corner insertion order of the triangulation
//! and which of several equally long edges becomes an airline — depends on identity hash codes
//! and differs between JVM runs. The port iterates the items in the order they are passed (item
//! list order, descending id). The **number** of airlines is the number of connected groups
//! minus the number of components of the triangulation graph, which is independent of that
//! order (the triangulation of a point set is connected, degenerate edges join equal corners of
//! different items), so counts match Java exactly; the airlines themselves may differ where the
//! triangulation is not unique (co-circular corners) or edges tie in length and corners.

use std::cmp::Ordering;
use std::collections::HashSet;

use fr_geom::{FloatPoint, Point};

use crate::board::{BasicBoard, ItemKey, ItemSet};
use crate::datastructures::planar_delaunay_triangulation::PlanarDelaunayTriangulation;
use crate::ids::NetNo;

/// Java `AirLine`: an incomplete connection between two items of a net.
#[derive(Clone, Debug, PartialEq)]
pub struct AirLine {
    pub net_number: NetNo,
    pub from_item: ItemKey,
    pub from_corner: FloatPoint,
    pub to_item: ItemKey,
    pub to_corner: FloatPoint,
}

/// Java `NetIncompletes`.
#[derive(Clone, Debug)]
pub struct NetIncompletes {
    pub net_number: NetNo,
    /// Java `incompletes`.
    pub incompletes: Vec<AirLine>,
    /// Java `drawMarkerRadius` (GUI).
    pub draw_marker_radius: f64,
    length_violation: f64,
    connected_group_count: i32,
}

/// Java `NetIncompletes.Edge`.
struct Edge {
    from: usize,
    from_corner: FloatPoint,
    to: usize,
    to_corner: FloatPoint,
    length_square: f64,
}

/// Java `Signum.asInt(double)` as an ordering (NaN counts as equal).
fn signum_order(v: f64) -> Ordering {
    if v > 0.0 {
        Ordering::Greater
    } else if v < 0.0 {
        Ordering::Less
    } else {
        Ordering::Equal
    }
}

/// Java `Edge.compareTo`.
fn edge_cmp(a: &Edge, b: &Edge) -> Ordering {
    let mut result = a.length_square - b.length_square;
    if result == 0.0 {
        result = a.from_corner.x - b.from_corner.x;
        if result == 0.0 {
            result = a.from_corner.y - b.from_corner.y;
        }
        if result == 0.0 {
            result = a.to_corner.x - b.to_corner.x;
        }
        if result == 0.0 {
            result = a.to_corner.y - b.to_corner.y;
        }
    }
    signum_order(result)
}

impl NetIncompletes {
    /// Java `new NetIncompletes(netNumber, netItems, board)`. `net_items` are the connectable
    /// items of the net (duplicates are ignored).
    pub fn new(board: &BasicBoard, net_number: NetNo, net_items: &[ItemKey]) -> NetIncompletes {
        let mut result = NetIncompletes {
            net_number,
            incompletes: Vec::new(),
            draw_marker_radius: (board.rules.get_min_trace_half_width() * 2) as f64,
            length_violation: 0.0,
            connected_group_count: 0,
        };

        // Filter out dangling items (tails) and items without contacts, except drill items (unrouted
        // pins legitimately have no contacts) and conduction areas.
        let mut filtered: Vec<ItemKey> = Vec::with_capacity(net_items.len());
        for &k in net_items {
            let item = board.item(k);
            // (fastroute: a stitching via is never a tail; the vias of a net form one group)
            if board.is_tail(k) && !board.is_stitching_via(k) {
                continue;
            }
            if !item.is_conduction_area() && !item.is_drill_item() && board.normal_contacts(k).is_empty() {
                continue;
            }
            filtered.push(k);
        }

        let (grouped, group_count) = calculate_net_items(board, net_number, &filtered);
        result.connected_group_count = group_count;
        if grouped.len() <= 1 {
            result.connected_group_count = grouped.len() as i32;
            return result;
        }
        let mut group_of: Vec<usize> = grouped.iter().map(|g| g.1).collect();

        // create a Delaunay triangulation for the net items
        let triangulation: PlanarDelaunayTriangulation<usize, Point> =
            PlanarDelaunayTriangulation::new_with_own_random(grouped.iter().enumerate().map(|(i, g)| (i, board.ratsnest_corners(g.0))));

        // sort the result edges by length (Java TreeSet: equal edges are dropped)
        let mut edges: Vec<Edge> = Vec::new();
        for line in triangulation.get_edge_lines() {
            let (Some(from), Some(to)) = (line.start_object, line.end_object) else {
                // Java: NullPointerException for an edge ending at a bounding corner
                log::warn!("NetIncompletes: triangulation edge without object");
                continue;
            };
            let from_corner = line.start_point.to_float();
            let to_corner = line.end_point.to_float();
            let length_square = to_corner.distance_square(&from_corner);
            edges.push(Edge { from, from_corner, to, to_corner, length_square });
        }
        edges.sort_by(edge_cmp); // stable: the first inserted of equal edges is kept
        edges.dedup_by(|b, a| edge_cmp(a, b) == Ordering::Equal);

        // Kruskal: skip edges whose items are already in the same connected set
        for edge in &edges {
            let from_group = group_of[edge.from];
            let to_group = group_of[edge.to];
            if from_group == to_group {
                continue;
            }
            result.incompletes.push(AirLine {
                net_number,
                from_item: grouped[edge.from].0,
                from_corner: edge.from_corner,
                to_item: grouped[edge.to].0,
                to_corner: edge.to_corner,
            });
            // joinConnectedSets
            for g in group_of.iter_mut() {
                if *g == from_group {
                    *g = to_group;
                }
            }
        }
        result.calc_length_violation(board);
        result
    }

    /// Java `count()`.
    pub fn count(&self) -> i32 {
        self.incompletes.len() as i32
    }

    /// Java `getConnectedGroupCount()`.
    pub fn connected_group_count(&self) -> i32 {
        self.connected_group_count
    }

    /// Java `getLengthViolation()`.
    pub fn length_violation(&self) -> f64 {
        self.length_violation
    }

    /// Java `calcLengthViolation()`: false if the violation did not change (by more than 0.1).
    pub fn calc_length_violation(&mut self, board: &BasicBoard) -> bool {
        let net_class = board.rules.net_class_of(self.net_number);
        let max_length = net_class.get_maximum_trace_length();
        let min_length = net_class.get_minimum_trace_length();
        if max_length <= 0.0 && min_length <= 0.0 {
            self.length_violation = 0.0;
            return false;
        }
        let mut new_violation = 0.0;
        let trace_length = board.net_trace_length(self.net_number);
        if max_length > 0.0 && trace_length > max_length {
            new_violation = trace_length - max_length;
        }
        if min_length > 0.0 && trace_length < min_length && self.incompletes.is_empty() {
            new_violation = trace_length - min_length;
        }
        let old_violation = self.length_violation;
        self.length_violation = new_violation;
        (new_violation - old_violation).abs() > 0.1
    }
}

/// Java `calculateNetItems`: the items with the index of their connected set, and the number
/// of distinct connected sets. Iterates `items` in the given order (Java: identity hash order).
fn calculate_net_items(board: &BasicBoard, net_number: NetNo, items: &[ItemKey]) -> (Vec<(ItemKey, usize)>, i32) {
    let mut remaining: Vec<ItemKey> = Vec::with_capacity(items.len());
    let mut in_remaining: HashSet<ItemKey> = HashSet::with_capacity(items.len());
    for &k in items {
        if in_remaining.insert(k) {
            remaining.push(k);
        }
    }
    let unique_count = remaining.len();
    let mut result = Vec::with_capacity(unique_count);
    let mut group_count = 0;
    let mut pos = 0;
    while pos < remaining.len() {
        let start = remaining[pos];
        if !in_remaining.contains(&start) {
            pos += 1;
            continue;
        }
        let connected: ItemSet = board.connected_set(start, net_number, false);
        let group = group_count;
        let mut found = false;
        for k in connected.iter() {
            if in_remaining.remove(&k) {
                result.push((k, group));
                found = true;
            }
        }
        if found {
            group_count += 1;
        } else {
            // Java would loop forever here (the start item is not in its own connected set).
            log::warn!("NetIncompletes.calculate_net_items: item not in its connected set");
            in_remaining.remove(&start);
        }
        pos += 1;
    }
    if result.len() != unique_count {
        log::warn!("NetIncompletes.calculate_net_items: item count mismatch");
    }
    (result, group_count as i32)
}
