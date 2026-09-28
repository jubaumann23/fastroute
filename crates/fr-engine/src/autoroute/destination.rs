//! Port of `autoroute/maze/DestinationDistance.java`: a lower bound for the distance between a
//! new maze expansion element and the destination set.

use fr_geom::{FloatPoint, IntBox};
use fr_settings::ExpansionCostFactor;

use super::control::jmin;

/// Java `DestinationDistance`.
#[derive(Clone, Debug)]
pub struct DestinationDistance {
    trace_costs: Vec<ExpansionCostFactor>,
    layer_count: i32,
    active_layer_count: i32,
    min_cheap_via_cost: f64,
    min_component_side_trace_cost: f64,
    max_component_side_trace_cost: f64,
    min_solder_side_trace_cost: f64,
    max_solder_side_trace_cost: f64,
    max_inner_side_trace_cost: f64,
    min_component_inner_trace_cost: f64,
    min_solder_inner_trace_cost: f64,
    min_component_solder_inner_trace_cost: f64,
    min_normal_via_cost: f64,
    component_side_box: IntBox,
    solder_side_box: IntBox,
    inner_side_box: IntBox,
    box_is_empty: bool,
    component_side_box_is_empty: bool,
    solder_side_box_is_empty: bool,
    inner_side_box_is_empty: bool,
}

impl DestinationDistance {
    /// Java `new DestinationDistance(traceCosts, layerActive, minNormalViaCost, minCheapViaCost)`.
    pub fn new(trace_costs: &[ExpansionCostFactor], layer_active: &[bool], min_normal_via_cost: f64, min_cheap_via_cost: f64) -> Self {
        let layer_count = layer_active.len() as i32;
        let active_layer_count = layer_active.iter().filter(|a| **a).count() as i32;
        let mut min_component_side_trace_cost = 0.0;
        let mut max_component_side_trace_cost = 0.0;
        let mut min_solder_side_trace_cost = 0.0;
        let mut max_solder_side_trace_cost = 0.0;
        if layer_active[0] {
            if trace_costs[0].horizontal < trace_costs[0].vertical {
                min_component_side_trace_cost = trace_costs[0].horizontal;
                max_component_side_trace_cost = trace_costs[0].vertical;
            } else {
                min_component_side_trace_cost = trace_costs[0].vertical;
                max_component_side_trace_cost = trace_costs[0].horizontal;
            }
        }
        let last = (layer_count - 1) as usize;
        if layer_active[last] {
            let c = trace_costs[last];
            if c.horizontal < c.vertical {
                min_solder_side_trace_cost = c.horizontal;
                max_solder_side_trace_cost = c.vertical;
            } else {
                min_solder_side_trace_cost = c.vertical;
                max_solder_side_trace_cost = c.horizontal;
            }
        }
        // Note: for inner layers we assume, that cost in preferred direction is 1
        let mut max_inner_side_trace_cost = jmin(max_component_side_trace_cost, max_solder_side_trace_cost);
        for ind2 in 1..(layer_count - 1).max(1) as usize {
            if !layer_active[ind2] {
                continue;
            }
            let current_max_cost = super::control::jmax(trace_costs[ind2].horizontal, trace_costs[ind2].vertical);
            max_inner_side_trace_cost = jmin(max_inner_side_trace_cost, current_max_cost);
        }
        let min_component_inner_trace_cost = jmin(min_component_side_trace_cost, max_inner_side_trace_cost);
        let min_solder_inner_trace_cost = jmin(min_solder_side_trace_cost, max_inner_side_trace_cost);
        let min_component_solder_inner_trace_cost = jmin(min_component_inner_trace_cost, min_solder_inner_trace_cost);
        DestinationDistance {
            trace_costs: trace_costs.to_vec(),
            layer_count,
            active_layer_count,
            min_cheap_via_cost,
            min_component_side_trace_cost,
            max_component_side_trace_cost,
            min_solder_side_trace_cost,
            max_solder_side_trace_cost,
            max_inner_side_trace_cost,
            min_component_inner_trace_cost,
            min_solder_inner_trace_cost,
            min_component_solder_inner_trace_cost,
            min_normal_via_cost,
            component_side_box: IntBox::EMPTY,
            solder_side_box: IntBox::EMPTY,
            inner_side_box: IntBox::EMPTY,
            box_is_empty: true,
            component_side_box_is_empty: true,
            solder_side_box_is_empty: true,
            inner_side_box_is_empty: true,
        }
    }

    /// Java `join(box, layer)`.
    pub fn join(&mut self, b: &IntBox, layer: i32) {
        if layer == 0 {
            self.component_side_box = self.component_side_box.union_int_box(b);
            self.component_side_box_is_empty = false;
        } else if layer == self.layer_count - 1 {
            self.solder_side_box = self.solder_side_box.union_int_box(b);
            self.solder_side_box_is_empty = false;
        } else {
            self.inner_side_box = self.inner_side_box.union_int_box(b);
            self.inner_side_box_is_empty = false;
        }
        self.box_is_empty = false;
    }

    /// Java `calculate(FloatPoint point, int layer)`.
    pub fn calculate_point(&self, point: &FloatPoint, layer: i32) -> f64 {
        self.calculate(&point.bounding_box(), layer)
    }

    /// Java `calculateCheapDistance(box, layer)`.
    pub fn calculate_cheap_distance(&mut self, b: &IntBox, layer: i32) -> f64 {
        let save = self.min_normal_via_cost;
        self.min_normal_via_cost = self.min_cheap_via_cost;
        let result = self.calculate(b, layer);
        self.min_normal_via_cost = save;
        result
    }

    /// Java `calculate(IntBox box, int layer)`.
    pub fn calculate(&self, b: &IntBox, layer: i32) -> f64 {
        if self.box_is_empty {
            return i32::MAX as f64;
        }
        let (csdx, csdy) = deltas(b, &self.component_side_box);
        let (ssdx, ssdy) = deltas(b, &self.solder_side_box);
        let (isdx, isdy) = deltas(b, &self.inner_side_box);
        let (cs_max, cs_min) = if csdx > csdy { (csdx, csdy) } else { (csdy, csdx) };
        let (ss_max, ss_min) = if ssdx > ssdy { (ssdx, ssdy) } else { (ssdy, ssdx) };
        let (is_max, is_min) = if isdx > isdy { (isdx, isdy) } else { (isdy, isdx) };
        let v = self.min_normal_via_cost;
        let mut result = i32::MAX as f64;
        if layer == 0 {
            // calculate shortest distance to component side box
            if !self.component_side_box_is_empty {
                result = b.weighted_distance(&self.component_side_box, self.trace_costs[0].horizontal, self.trace_costs[0].vertical);
            }
            if self.active_layer_count <= 1 {
                return result;
            }
            // two layer distance on component and solder side
            let mut tmp = if self.min_solder_side_trace_cost < self.min_component_side_trace_cost {
                self.min_solder_side_trace_cost * ss_max + self.min_component_side_trace_cost * ss_min + v
            } else {
                self.min_component_side_trace_cost * ss_max + self.min_solder_side_trace_cost * ss_min + v
            };
            result = jmin(result, tmp);
            // two layer distance on component and solder side with two vias
            tmp = cs_max + cs_min * self.min_component_inner_trace_cost + 2.0 * v;
            result = jmin(result, tmp);
            if self.active_layer_count == 2 {
                return result;
            }
            // two layer distance on component side and an inner side
            tmp = is_max + is_min * self.min_component_inner_trace_cost + v;
            result = jmin(result, tmp);
            // three layer distance
            tmp = ss_max + self.min_component_solder_inner_trace_cost * ss_min + 2.0 * v;
            result = jmin(result, tmp);
            tmp = cs_max + cs_min + 2.0 * v;
            result = jmin(result, tmp);
            if self.active_layer_count == 3 {
                return result;
            }
            tmp = is_max + is_min + 2.0 * v;
            result = jmin(result, tmp);
            // four layer distance
            tmp = ss_max + ss_min + 3.0 * v;
            return jmin(result, tmp);
        }
        if layer == self.layer_count - 1 {
            // calculate the shortest distance to solder side box
            if !self.solder_side_box_is_empty {
                let c = self.trace_costs[layer as usize];
                result = b.weighted_distance(&self.solder_side_box, c.horizontal, c.vertical);
            }
            // two layer distance
            let mut tmp = if self.min_component_side_trace_cost < self.min_solder_side_trace_cost {
                self.min_component_side_trace_cost * cs_max + self.min_solder_side_trace_cost * cs_min + v
            } else {
                self.min_solder_side_trace_cost * cs_max + self.min_component_side_trace_cost * cs_min + v
            };
            result = jmin(result, tmp);
            tmp = ss_max + ss_min * self.min_solder_inner_trace_cost + 2.0 * v;
            result = jmin(result, tmp);
            if self.active_layer_count <= 2 {
                return result;
            }
            tmp = is_min * self.min_solder_inner_trace_cost + is_max + v;
            result = jmin(result, tmp);
            // three layer distance
            tmp = cs_max + self.min_component_solder_inner_trace_cost * cs_min + 2.0 * v;
            result = jmin(result, tmp);
            tmp = ss_max + ss_min + 2.0 * v;
            result = jmin(result, tmp);
            if self.active_layer_count == 3 {
                return result;
            }
            tmp = is_max + is_min + 2.0 * v;
            result = jmin(result, tmp);
            // four layer distance
            tmp = cs_max + cs_min + 3.0 * v;
            return jmin(result, tmp);
        }
        // calculate distance to inner layer box: one layer distance
        if !self.inner_side_box_is_empty {
            let c = self.trace_costs[layer as usize];
            result = b.weighted_distance(&self.inner_side_box, c.horizontal, c.vertical);
        }
        // two layer distance
        let mut tmp = is_max + is_min + v;
        result = jmin(result, tmp);
        tmp = cs_max + cs_min * self.min_component_inner_trace_cost + v;
        result = jmin(result, tmp);
        tmp = ss_max + ss_min * self.min_solder_inner_trace_cost + v;
        result = jmin(result, tmp);
        // three layer distance
        tmp = cs_max + cs_min + 2.0 * v;
        result = jmin(result, tmp);
        tmp = ss_max + ss_min + 2.0 * v;
        jmin(result, tmp)
    }

    /// The maximal trace costs (unused by the algorithm, kept for completeness).
    pub fn max_trace_costs(&self) -> (f64, f64, f64) {
        (self.max_component_side_trace_cost, self.max_solder_side_trace_cost, self.max_inner_side_trace_cost)
    }
}

/// The x and y distances of `b` to `other` (0 where they overlap). Java computes the int
/// differences and converts them to `double`.
fn deltas(b: &IntBox, other: &IntBox) -> (f64, f64) {
    let dx = if b.ll.x > other.ur.x {
        b.ll.x.wrapping_sub(other.ur.x) as f64
    } else if b.ur.x < other.ll.x {
        other.ll.x.wrapping_sub(b.ur.x) as f64
    } else {
        0.0
    };
    let dy = if b.ll.y > other.ur.y {
        b.ll.y.wrapping_sub(other.ur.y) as f64
    } else if b.ur.y < other.ll.y {
        other.ll.y.wrapping_sub(b.ur.y) as f64
    } else {
        0.0
    };
    (dx, dy)
}
