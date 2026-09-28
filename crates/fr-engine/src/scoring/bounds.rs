//! Port of `core/scoring/BoardStatisticsBoundsCalculator.java`: board-only lower bounds (MST
//! wire length, bends, vias) for the V2 optimizer score.
//!
//! Java caches the result per board object (`WeakHashMap`, identity) forever, even when the board
//! changes afterwards. The result only depends on the terminal items (non-routable connectable
//! items: pins, conduction areas, user-fixed traces/vias), their layers and the via rules, which
//! do not change while routing, so recomputing gives the same value. Callers that want the Java
//! caching (or cheaper statistics) pass a precomputed value to
//! [`BoardStatistics::new_with_bounds`](super::BoardStatistics::new_with_bounds).

use std::collections::{BTreeMap, BTreeSet};

use crate::board::{BasicBoard, ItemKey};
use crate::ids::{LayerNo, NetNo};
use crate::structure::Unit;

/// Java `BoardStatisticsBounds`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BoardStatisticsBounds {
    pub min_trace_length_mm: Option<f32>,
    pub min_via_count: Option<i32>,
    pub min_bend_count: Option<i32>,
}

struct Terminal {
    x: f64,
    y: f64,
    signal_layers: BTreeSet<LayerNo>,
}

/// Java `BoardStatisticsBoundsCalculator.calculateUncached(board)`.
pub fn calculate_bounds(board: &BasicBoard) -> BoardStatisticsBounds {
    let mut min_trace_length = 0.0f64;
    let mut min_via_count = 0i32;
    let mut min_bend_count = 0i32;
    let comm = &board.communication;
    let board_unit_to_mm = Unit::scale(1.0, comm.unit, Unit::Mm) / (if comm.resolution > 0 { comm.resolution } else { 1 }) as f64;
    for net_number in 1..=board.rules.nets.max_net_number() {
        if board.rules.nets.get(net_number).is_none() {
            continue;
        }
        let terminals = get_terminals(board, net_number);
        if terminals.len() < 2 {
            continue;
        }
        let (length, bends) = calculate_mst(&terminals);
        min_trace_length += length * board_unit_to_mm;
        min_bend_count = min_bend_count.wrapping_add(bends);
        min_via_count = min_via_count.wrapping_add(calculate_minimum_via_count(board, net_number, &terminals));
    }
    BoardStatisticsBounds {
        min_trace_length_mm: Some(min_trace_length as f32),
        min_via_count: Some(min_via_count),
        min_bend_count: Some(min_bend_count),
    }
}

fn get_terminals(board: &BasicBoard, net_number: NetNo) -> Vec<Terminal> {
    board
        .net_terminal_items(net_number)
        .into_iter()
        .filter_map(|k| to_terminal(board, k))
        .filter(|t| !t.signal_layers.is_empty())
        .collect()
}

fn to_terminal(board: &BasicBoard, key: ItemKey) -> Option<Terminal> {
    let item = board.item(key);
    let mut signal_layers = BTreeSet::new();
    let (x, y);
    if item.is_pin() {
        let center = item.center(board).to_float();
        x = center.x;
        y = center.y;
        let first = item.first_layer(board);
        for layer in first..=item.last_layer(board) {
            if item.drill_shape(board, layer - first).is_some() {
                add_signal_layer(board, layer, &mut signal_layers);
            }
        }
    } else if let Some(area) = item.as_conduction_area() {
        let center = area.area.get_area(board).get_border().centre_of_gravity();
        x = center.x;
        y = center.y;
        add_signal_layer(board, area.area.layer(), &mut signal_layers);
    } else {
        return None;
    }
    Some(Terminal { x, y, signal_layers })
}

fn add_signal_layer(board: &BasicBoard, layer: LayerNo, signal_layers: &mut BTreeSet<LayerNo>) {
    let layers = &board.layer_structure.layers;
    if layer >= 0 && (layer as usize) < layers.len() && layers[layer as usize].is_signal {
        signal_layers.insert(layer);
    }
}

/// Prim's MST with Manhattan distances: (length, number of edges that are not axis parallel).
fn calculate_mst(terminals: &[Terminal]) -> (f64, i32) {
    let n = terminals.len();
    let mut used = vec![false; n];
    let mut distances = vec![f64::INFINITY; n];
    let mut parents = vec![-1i64; n];
    distances[0] = 0.0;
    let mut length = 0.0;
    let mut bend_count = 0;
    for _ in 0..n {
        let mut current: i64 = -1;
        for index in 0..n {
            if !used[index] && (current < 0 || distances[index] < distances[current as usize]) {
                current = index as i64;
            }
        }
        if current < 0 {
            break;
        }
        let current = current as usize;
        used[current] = true;
        if parents[current] >= 0 {
            let from = &terminals[parents[current] as usize];
            let to = &terminals[current];
            length += distances[current];
            if from.x != to.x && from.y != to.y {
                bend_count += 1;
            }
        }
        for index in 0..n {
            if !used[index] {
                let distance = manhattan(&terminals[current], &terminals[index]);
                if distance < distances[index] {
                    distances[index] = distance;
                    parents[index] = current as i64;
                }
            }
        }
    }
    (length, bend_count)
}

fn manhattan(a: &Terminal, b: &Terminal) -> f64 {
    (a.x - b.x).abs() + (a.y - b.y).abs()
}

/// Java `calculateMinimumViaCount`: greedy cover of the layer groups by the via spans of the
/// net's via rule. (Java iterates `HashSet<Integer>`s; the result only depends on set sizes.)
fn calculate_minimum_via_count(board: &BasicBoard, net_number: NetNo, terminals: &[Terminal]) -> i32 {
    let mut groups = LayerGroups::default();
    for t in terminals {
        groups.add_layers(&t.signal_layers);
    }
    if groups.roots().len() <= 1 {
        return 0;
    }
    let Some(via_rule) = board.rules.net_class_of(net_number).get_via_rule() else {
        return 0;
    };
    let rule = &board.rules.via_rules[via_rule];
    let mut spans = Vec::new();
    for &via in rule.vias() {
        let padstack = board.library.padstacks.get(board.rules.via_infos[via].get_padstack()).expect("via padstack");
        let (first, last) = (padstack.from_layer(), padstack.to_layer());
        spans.push((first.min(last), first.max(last)));
    }
    let mut remaining = groups.roots();
    let mut via_count = 0;
    while !remaining.is_empty() {
        let mut best: Option<BTreeSet<LayerNo>> = None;
        for &span in &spans {
            let covered: BTreeSet<LayerNo> = groups.groups_covered_by(span).intersection(&remaining).copied().collect();
            if covered.len() > best.as_ref().map_or(0, |b| b.len()) {
                best = Some(covered);
            }
        }
        let Some(best) = best else {
            break;
        };
        if best.is_empty() {
            break;
        }
        for g in &best {
            remaining.remove(g);
        }
        via_count += 1;
    }
    via_count
}

/// Java `LayerGroups`: union-find over layer numbers.
#[derive(Default)]
struct LayerGroups {
    parent: BTreeMap<LayerNo, LayerNo>,
}

impl LayerGroups {
    fn add_layers(&mut self, layers: &BTreeSet<LayerNo>) {
        let mut first: Option<LayerNo> = None;
        for &layer in layers {
            self.parent.entry(layer).or_insert(layer);
            match first {
                None => first = Some(layer),
                Some(f) => self.union(f, layer),
            }
        }
    }

    fn roots(&self) -> BTreeSet<LayerNo> {
        self.parent.keys().map(|&l| self.find(l)).collect()
    }

    fn groups_covered_by(&self, span: (LayerNo, LayerNo)) -> BTreeSet<LayerNo> {
        self.parent.keys().filter(|&&l| l >= span.0 && l <= span.1).map(|&l| self.find(l)).collect()
    }

    fn find(&self, layer: LayerNo) -> LayerNo {
        let mut current = layer;
        loop {
            let p = self.parent[&current];
            if p == current {
                return current;
            }
            current = p;
        }
    }

    fn union(&mut self, a: LayerNo, b: LayerNo) {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra != rb {
            self.parent.insert(rb, ra);
        }
    }
}
