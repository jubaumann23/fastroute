//! Port of `rules/NetClass.java` and `rules/NetClasses.java`.
//!
//! Java nets share and mutate `NetClass` objects by reference; here nets store a
//! [`NetClassId`] (stable handle into the [`NetClasses`] arena, Java object identity).
//! `NetClasses.remove` only unlinks the class from the ordered list, like Java.

use std::ops::{Index, IndexMut};

use super::default_item_clearance_classes::DefaultItemClearanceClasses;
use super::via::ViaRuleId;
use crate::ids::{ClearanceClassNo, LayerNo};
use crate::structure::LayerStructure;

/// Stable handle of a [`NetClass`] (Java object identity).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NetClassId(pub u32);

/// Routing rules for a group of nets (Java `NetClass`).
#[derive(Clone, Debug, PartialEq)]
pub struct NetClass {
    name: String,
    /// `isSignal` of the board layers (Java reads them through its layer structure reference).
    layer_is_signal: Vec<bool>,
    trace_half_width_arr: Vec<i32>,
    active_routing_layer_arr: Vec<bool>,
    /// Clearance classes of the item types (from a DSN class).
    pub default_item_clearance_classes: DefaultItemClearanceClasses,
    pub is_ignored_by_autorouter: bool,
    /// fastroute: traces of this class keep their width at pins (controlled impedance): no
    /// neck-down, no retry with narrower traces.
    pub no_neckdown: bool,
    via_rule: Option<ViaRuleId>,
    trace_clearance_class: ClearanceClassNo,
    shove_fixed: bool,
    pull_tight: bool,
    ignore_cycles_with_areas: bool,
    minimum_trace_length: f64,
    maximum_trace_length: f64,
}

impl NetClass {
    pub fn new(
        name: impl Into<String>,
        layer_structure: &LayerStructure,
        ignored_by_autorouter: bool,
    ) -> Self {
        let n = layer_structure.layers.len();
        let layer_is_signal: Vec<bool> =
            layer_structure.layers.iter().map(|l| l.is_signal).collect();
        NetClass {
            name: name.into(),
            active_routing_layer_arr: layer_is_signal.clone(),
            layer_is_signal,
            trace_half_width_arr: vec![0; n],
            default_item_clearance_classes: DefaultItemClearanceClasses::new(),
            is_ignored_by_autorouter: ignored_by_autorouter,
            no_neckdown: false,
            via_rule: None,
            trace_clearance_class: 0,
            shove_fixed: false,
            pull_tight: true,
            ignore_cycles_with_areas: false,
            minimum_trace_length: 0.0,
            maximum_trace_length: 0.0,
        }
    }

    pub fn get_name(&self) -> &str {
        &self.name
    }

    pub fn set_name(&mut self, name: impl Into<String>) {
        self.name = name.into();
    }

    /// Sets the routing trace half width on all layers.
    pub fn set_trace_half_width(&mut self, value: i32) {
        self.trace_half_width_arr.fill(value);
    }

    /// Sets the routing trace half width on `layer`. Panics if out of range.
    pub fn set_trace_half_width_on_layer(&mut self, layer: LayerNo, value: i32) {
        self.trace_half_width_arr[layer as usize] = value;
    }

    /// Sets the routing trace half width on all inner layers.
    pub fn set_trace_half_width_on_inner(&mut self, value: i32) {
        let n = self.trace_half_width_arr.len();
        for i in 1..n.saturating_sub(1) {
            self.trace_half_width_arr[i] = value;
        }
    }

    /// The number of layers.
    pub fn layer_count(&self) -> i32 {
        self.trace_half_width_arr.len() as i32
    }

    /// The routing trace half width on `layer`; 0 (with a warning) if out of range.
    pub fn get_trace_half_width(&self, layer: LayerNo) -> i32 {
        if layer < 0 || layer as usize >= self.trace_half_width_arr.len() {
            log::warn!(" NetClass.get_trace_half_width: layer out of range");
            return 0;
        }
        self.trace_half_width_arr[layer as usize]
    }

    pub fn get_trace_clearance_class(&self) -> ClearanceClassNo {
        self.trace_clearance_class
    }

    pub fn set_trace_clearance_class(&mut self, clearance_class: ClearanceClassNo) {
        self.trace_clearance_class = clearance_class;
    }

    pub fn get_via_rule(&self) -> Option<ViaRuleId> {
        self.via_rule
    }

    pub fn set_via_rule(&mut self, via_rule: Option<ViaRuleId>) {
        self.via_rule = via_rule;
    }

    /// Whether traces and vias of this class may not be pushed.
    pub fn is_shove_fixed(&self) -> bool {
        self.shove_fixed
    }

    pub fn set_shove_fixed(&mut self, value: bool) {
        self.shove_fixed = value;
    }

    /// Whether traces of this class are pulled tight (default true).
    pub fn get_pull_tight(&self) -> bool {
        self.pull_tight
    }

    pub fn set_pull_tight(&mut self, value: bool) {
        self.pull_tight = value;
    }

    /// Whether cycle removal ignores cycles involving conduction areas.
    pub fn get_ignore_cycles_with_areas(&self) -> bool {
        self.ignore_cycles_with_areas
    }

    pub fn set_ignore_cycles_with_areas(&mut self, value: bool) {
        self.ignore_cycles_with_areas = value;
    }

    /// Minimum trace length; `<= 0` means no restriction.
    pub fn get_minimum_trace_length(&self) -> f64 {
        self.minimum_trace_length
    }

    pub fn set_minimum_trace_length(&mut self, value: f64) {
        self.minimum_trace_length = value;
    }

    /// Maximum trace length; `<= 0` means no restriction.
    pub fn get_maximum_trace_length(&self) -> f64 {
        self.maximum_trace_length
    }

    pub fn set_maximum_trace_length(&mut self, value: f64) {
        self.maximum_trace_length = value;
    }

    /// Whether `layer_number` is active for routing (false if out of range).
    pub fn is_active_routing_layer(&self, layer_number: LayerNo) -> bool {
        if layer_number < 0 || layer_number as usize >= self.active_routing_layer_arr.len() {
            return false;
        }
        self.active_routing_layer_arr[layer_number as usize]
    }

    /// Activates or deactivates `layer_number` for routing (ignored if out of range).
    pub fn set_active_routing_layer(&mut self, layer_number: LayerNo, active: bool) {
        if layer_number < 0 || layer_number as usize >= self.active_routing_layer_arr.len() {
            return;
        }
        self.active_routing_layer_arr[layer_number as usize] = active;
    }

    /// Activates or deactivates all layers.
    pub fn set_all_layers_active(&mut self, value: bool) {
        self.active_routing_layer_arr.fill(value);
    }

    /// Activates or deactivates all inner layers.
    pub fn set_all_inner_layers_active(&mut self, value: bool) {
        let n = self.trace_half_width_arr.len();
        for i in 1..n.saturating_sub(1) {
            self.active_routing_layer_arr[i] = value;
        }
    }

    /// True if the trace width differs between signal layers (compared to layer 0).
    pub fn trace_width_is_layer_dependent(&self) -> bool {
        let compare_value = self.trace_half_width_arr[0];
        (1..self.trace_half_width_arr.len())
            .any(|i| self.layer_is_signal[i] && self.trace_half_width_arr[i] != compare_value)
    }

    /// True if the trace width differs between inner signal layers.
    pub fn trace_width_is_inner_layer_dependent(&self) -> bool {
        let n = self.trace_half_width_arr.len();
        if n <= 3 {
            return false;
        }
        let mut first_inner_layer_no = 1;
        // Java: no bounds check (throws if no signal layer follows).
        while !self.layer_is_signal[first_inner_layer_no] {
            first_inner_layer_no += 1;
        }
        if first_inner_layer_no >= n - 1 {
            return false;
        }
        let compare_width = self.trace_half_width_arr[first_inner_layer_no];
        (first_inner_layer_no + 1..n - 1)
            .any(|i| self.layer_is_signal[i] && self.trace_half_width_arr[i] != compare_width)
    }
}

impl std::fmt::Display for NetClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.name)
    }
}

/// The net classes of the board (Java `NetClasses`); index 0 is the default class.
#[derive(Clone, Debug, Default)]
pub struct NetClasses {
    arena: Vec<NetClass>,
    class_arr: Vec<NetClassId>,
}

impl NetClasses {
    pub fn new() -> Self {
        Self::default()
    }

    /// The number of classes in the list.
    pub fn count(&self) -> i32 {
        self.class_arr.len() as i32
    }

    /// The class at `index` in the list. Panics if out of range.
    pub fn get(&self, index: i32) -> NetClassId {
        self.class_arr[index as usize]
    }

    /// The class with the given name (case-sensitive), if any.
    pub fn get_by_name(&self, name: &str) -> Option<NetClassId> {
        self.class_arr
            .iter()
            .copied()
            .find(|&id| self[id].name == name)
    }

    /// Appends a new empty class.
    pub fn append(
        &mut self,
        name: impl Into<String>,
        layer_structure: &LayerStructure,
        ignored_by_autorouter: bool,
    ) -> NetClassId {
        let id = NetClassId(self.arena.len() as u32);
        self.arena
            .push(NetClass::new(name, layer_structure, ignored_by_autorouter));
        self.class_arr.push(id);
        id
    }

    /// Appends a new empty class named `class<n>` with the smallest unused `n >= 1`.
    pub fn append_unnamed(&mut self, layer_structure: &LayerStructure) -> NetClassId {
        let mut index = 0;
        let new_name = loop {
            index += 1;
            let name = format!("class{index}");
            if self.get_by_name(&name).is_none() {
                break name;
            }
        };
        self.append(new_name, layer_structure, false)
    }

    /// The first class with the given trace half width on all layers, trace clearance class and
    /// via rule (identity), if any.
    pub fn find(
        &self,
        trace_half_width: i32,
        trace_clearance_class: ClearanceClassNo,
        via_rule: Option<ViaRuleId>,
    ) -> Option<NetClassId> {
        self.class_arr.iter().copied().find(|&id| {
            let c = &self[id];
            c.trace_clearance_class == trace_clearance_class
                && c.via_rule == via_rule
                && (0..c.layer_count()).all(|i| c.get_trace_half_width(i) == trace_half_width)
        })
    }

    /// The first class with the given per-layer trace half widths, trace clearance class and via
    /// rule (identity), if any.
    pub fn find_per_layer(
        &self,
        trace_half_width_arr: &[i32],
        trace_clearance_class: ClearanceClassNo,
        via_rule: Option<ViaRuleId>,
    ) -> Option<NetClassId> {
        self.class_arr.iter().copied().find(|&id| {
            let c = &self[id];
            c.trace_clearance_class == trace_clearance_class
                && c.via_rule == via_rule
                && trace_half_width_arr.len() as i32 == c.layer_count()
                && (0..c.layer_count())
                    .all(|i| c.get_trace_half_width(i) == trace_half_width_arr[i as usize])
        })
    }

    /// Removes the class from the list (it stays valid for nets still referring to it). False
    /// if it was not in the list.
    pub fn remove(&mut self, net_class: NetClassId) -> bool {
        match self.class_arr.iter().position(|&id| id == net_class) {
            Some(i) => {
                self.class_arr.remove(i);
                true
            }
            None => false,
        }
    }

    /// The ids in list order.
    pub fn iter(&self) -> impl Iterator<Item = NetClassId> + '_ {
        self.class_arr.iter().copied()
    }
}

impl Index<NetClassId> for NetClasses {
    type Output = NetClass;
    fn index(&self, id: NetClassId) -> &NetClass {
        &self.arena[id.0 as usize]
    }
}

impl IndexMut<NetClassId> for NetClasses {
    fn index_mut(&mut self, id: NetClassId) -> &mut NetClass {
        &mut self.arena[id.0 as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::structure::Layer;

    fn ls() -> LayerStructure {
        LayerStructure::new(vec![
            Layer::new("F", true),
            Layer::new("GND", false),
            Layer::new("In1", true),
            Layer::new("In2", true),
            Layer::new("B", true),
        ])
    }

    #[test]
    fn net_class_widths_and_layers() {
        let mut c = NetClass::new("c", &ls(), false);
        assert!(c.get_pull_tight());
        assert!(!c.is_active_routing_layer(1));
        assert!(c.is_active_routing_layer(2));
        assert!(!c.is_active_routing_layer(5));
        c.set_trace_half_width(100);
        assert!(!c.trace_width_is_layer_dependent());
        // non-signal layers are ignored
        c.set_trace_half_width_on_layer(1, 7);
        assert!(!c.trace_width_is_layer_dependent());
        assert!(!c.trace_width_is_inner_layer_dependent());
        c.set_trace_half_width_on_layer(3, 50);
        assert!(c.trace_width_is_layer_dependent());
        assert!(c.trace_width_is_inner_layer_dependent());
        c.set_trace_half_width_on_inner(60);
        assert_eq!(
            (0..5)
                .map(|l| c.get_trace_half_width(l))
                .collect::<Vec<_>>(),
            [100, 60, 60, 60, 100]
        );
        assert!(!c.trace_width_is_inner_layer_dependent());
        assert_eq!(c.get_trace_half_width(9), 0);
        c.set_all_inner_layers_active(false);
        assert!(c.is_active_routing_layer(0) && !c.is_active_routing_layer(2));
        c.set_all_layers_active(true);
        assert!(c.is_active_routing_layer(1));
    }

    #[test]
    fn net_classes() {
        let ls = ls();
        let mut nc = NetClasses::new();
        let a = nc.append_unnamed(&ls);
        let b = nc.append("class3", &ls, false);
        let c = nc.append_unnamed(&ls);
        assert_eq!(nc[a].get_name(), "class1");
        assert_eq!(nc[c].get_name(), "class2");
        assert_eq!(nc.get(1), b);
        assert_eq!(nc.get_by_name("class3"), Some(b));
        assert_eq!(nc.get_by_name("CLASS3"), None);
        nc[b].set_trace_half_width(10);
        nc[b].set_trace_clearance_class(2);
        assert_eq!(nc.find(10, 2, None), Some(b));
        assert_eq!(nc.find(10, 2, Some(ViaRuleId(0))), None);
        assert_eq!(nc.find_per_layer(&[10; 5], 2, None), Some(b));
        assert_eq!(nc.find_per_layer(&[10; 4], 2, None), None);
        // a and c both match (0 width, class 0): the first in list order wins
        assert_eq!(nc.find(0, 0, None), Some(a));
        assert!(nc.remove(a));
        assert!(!nc.remove(a));
        assert_eq!(nc.find(0, 0, None), Some(c));
        assert_eq!(nc.count(), 2);
        assert_eq!(nc[a].get_name(), "class1");
        let d = nc.append_unnamed(&ls);
        assert_eq!(nc[d].get_name(), "class1");
    }
}
