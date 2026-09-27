//! The parser netlist (Java `io.specctra.parser.{NetList, Net}`): a `TreeMap` from
//! `(net name, subnet number)` to the sorted pin set of the subnet.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use fr_jcompat::java_string_compare;

/// Java `Net.Id` with its `compareTo` (`String.compareTo`, then subnet subtraction).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetId {
    pub name: String,
    pub subnet: i32,
}

impl Ord for NetId {
    fn cmp(&self, other: &Self) -> Ordering {
        let r = java_string_compare(&self.name, &other.name);
        let r = if r == 0 {
            self.subnet.wrapping_sub(other.subnet)
        } else {
            r
        };
        r.cmp(&0)
    }
}

impl PartialOrd for NetId {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Java `Net.Pin` (`componentName`, `pinName`), ordered by `String.compareTo`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PinKey {
    pub component: String,
    pub pin: String,
}

impl Ord for PinKey {
    fn cmp(&self, other: &Self) -> Ordering {
        let r = java_string_compare(&self.component, &other.component);
        let r = if r == 0 {
            java_string_compare(&self.pin, &other.pin)
        } else {
            r
        };
        r.cmp(&0)
    }
}

impl PartialOrd for PinKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Clone, Debug, Default)]
pub struct NetList {
    /// `None` pins: the net was added without `setPins` (planes).
    nets: BTreeMap<NetId, Option<BTreeSet<PinKey>>>,
}

impl NetList {
    pub fn contains(&self, id: &NetId) -> bool {
        self.nets.contains_key(id)
    }

    /// Java `addNet`: false (Java `null`) if the id exists already.
    pub fn add_net(&mut self, id: NetId) -> bool {
        if self.nets.contains_key(&id) {
            return false;
        }
        self.nets.insert(id, None);
        true
    }

    pub fn set_pins(&mut self, id: &NetId, pins: BTreeSet<PinKey>) {
        if let Some(p) = self.nets.get_mut(id) {
            *p = Some(pins);
        }
    }

    /// Java `getNets(componentName, pinName)` in `TreeMap` order.
    pub fn get_nets(&self, component: &str, pin: &str) -> Vec<&NetId> {
        let key = PinKey {
            component: component.to_string(),
            pin: pin.to_string(),
        };
        self.nets
            .iter()
            .filter(|(_, pins)| pins.as_ref().is_some_and(|p| p.contains(&key)))
            .map(|(id, _)| id)
            .collect()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&NetId, Option<&BTreeSet<PinKey>>)> {
        self.nets.iter().map(|(k, v)| (k, v.as_ref()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: &str, s: i32) -> NetId {
        NetId {
            name: n.into(),
            subnet: s,
        }
    }

    #[test]
    fn tree_map_order() {
        let mut nl = NetList::default();
        for (n, s) in [("b", 1), ("B", 2), ("B", 1), ("a", 1)] {
            assert!(nl.add_net(id(n, s)));
        }
        assert!(!nl.add_net(id("B", 1)));
        let order: Vec<(String, i32)> =
            nl.iter().map(|(k, _)| (k.name.clone(), k.subnet)).collect();
        // String.compareTo: upper case before lower case, then subnet number.
        assert_eq!(
            order,
            vec![
                ("B".into(), 1),
                ("B".into(), 2),
                ("a".into(), 1),
                ("b".into(), 1)
            ]
        );
        let pin = |c: &str, p: &str| PinKey {
            component: c.into(),
            pin: p.into(),
        };
        nl.set_pins(&id("b", 1), [pin("U1", "1")].into_iter().collect());
        nl.set_pins(
            &id("B", 2),
            [pin("U1", "1"), pin("U2", "3")].into_iter().collect(),
        );
        let nets: Vec<&NetId> = nl.get_nets("U1", "1");
        assert_eq!(nets, vec![&id("B", 2), &id("b", 1)]);
        assert!(nl.get_nets("U3", "1").is_empty());
    }
}
