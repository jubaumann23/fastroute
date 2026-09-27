//! Port of `rules/Net.java` and `rules/Nets.java`.
//!
//! Java `Nets` holds a board back-reference (`getBoard`/`setBoard`) used by `Net`'s item
//! queries; these become Board methods. Board-dependent methods (TODO, implement on the
//! Board; all iterate the board item list in its order):
//! * `Net.getTerminalItems()`: items that are `Connectable`, `containsNet(netNumber)` and
//!   `!isRoutable()`. Inputs: board items, net number.
//! * `Net.getPins()`: `Pin` items with `containsNet(netNumber)`.
//! * `Net.getItems()`: all items with `containsNet(netNumber)`.
//! * `Net.getTraceLength()`: sum of `Trace.getLength()` over
//!   `board.getConnectableItems(netNumber)` (double summation in that order).
//! * `Net.getViaCount()`: count of `Via`s in `board.getConnectableItems(netNumber)`.
//!
//! `Nets.newNet(Locale)` (GUI, localized name) is not ported. The Java `Net` constructor reads
//! the default net class via the board (`rules.getDefaultNetClass()`, which may create it);
//! use [`BoardRules::add_net`](super::BoardRules::add_net) for that behaviour.

use fr_jcompat::compare_to_ignore_case;

use super::net_class::NetClassId;
use crate::ids::NetNo;

/// Properties of an electrical net (Java `Net`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Net {
    pub name: String,
    /// Used only if a net is divided internally (e.g. from-to rules); normally 1.
    pub subnet_number: i32,
    /// The unique positive net number.
    pub net_number: NetNo,
    contains_plane: bool,
    net_class: NetClassId,
}

impl Net {
    pub fn new(
        name: impl Into<String>,
        subnet_number: i32,
        number: NetNo,
        contains_plane: bool,
        net_class: NetClassId,
    ) -> Self {
        Net {
            name: name.into(),
            subnet_number,
            net_number: number,
            contains_plane,
            net_class,
        }
    }

    /// Compares by name ignoring case (Java `compareTo`).
    pub fn compare_to(&self, other: &Net) -> i32 {
        compare_to_ignore_case(&self.name, &other.name)
    }

    pub fn get_net_class(&self) -> NetClassId {
        self.net_class
    }

    pub fn set_class(&mut self, net_class: NetClassId) {
        self.net_class = net_class;
    }

    pub fn set_contains_plane(&mut self, value: bool) {
        self.contains_plane = value;
    }

    /// Whether this net contains a power plane (cheap plane via costs in the autorouter).
    pub fn contains_plane(&self) -> bool {
        self.contains_plane
    }
}

impl std::fmt::Display for Net {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Net #{} ({})", self.net_number, self.name)
    }
}

/// The electrical nets of the board (Java `Nets`); net `n` is at index `n - 1`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Nets {
    nets: Vec<Net>,
}

impl Nets {
    /// The maximum legal net number.
    pub const MAX_LEGAL_NET_NUMBER: NetNo = 9_999_999;
    /// The auxiliary net number for internal use.
    pub const HIDDEN_NET_NUMBER: NetNo = 10_000_001;

    pub fn new() -> Self {
        Self::default()
    }

    /// False if `net_number` belongs to an internally used special-purpose net.
    pub fn is_normal_net_number(net_number: NetNo) -> bool {
        net_number > 0 && net_number <= Self::MAX_LEGAL_NET_NUMBER
    }

    /// The biggest net number (= count).
    pub fn max_net_number(&self) -> i32 {
        self.nets.len() as i32
    }

    /// The net with the given name (ignoring case) and subnet number, if any.
    pub fn get_by_name(&self, name: &str, subnet_number: i32) -> Option<&Net> {
        self.nets.iter().find(|n| {
            compare_to_ignore_case(&n.name, name) == 0 && n.subnet_number == subnet_number
        })
    }

    /// All subnets with the given name (ignoring case), in net number order.
    pub fn get_all_by_name(&self, name: &str) -> Vec<&Net> {
        self.nets
            .iter()
            .filter(|n| compare_to_ignore_case(&n.name, name) == 0)
            .collect()
    }

    /// The net with the given number, if any.
    pub fn get(&self, net_number: NetNo) -> Option<&Net> {
        if net_number < 1 || net_number as usize > self.nets.len() {
            return None;
        }
        let result = &self.nets[(net_number - 1) as usize];
        if result.net_number != net_number {
            log::warn!("Nets.get: inconsistent netNumber");
        }
        Some(result)
    }

    pub fn get_mut(&mut self, net_number: NetNo) -> Option<&mut Net> {
        if net_number < 1 {
            return None;
        }
        self.nets.get_mut((net_number - 1) as usize)
    }

    /// All nets in net number order.
    pub fn iter(&self) -> std::slice::Iter<'_, Net> {
        self.nets.iter()
    }

    /// Adds a net with the next number and the given net class; returns its number.
    pub fn add(
        &mut self,
        name: impl Into<String>,
        subnet_number: i32,
        contains_plane: bool,
        net_class: NetClassId,
    ) -> NetNo {
        let new_net_no = self.nets.len() as NetNo + 1;
        if new_net_no >= Self::MAX_LEGAL_NET_NUMBER {
            log::warn!("Nets.add_net: maxNetNo out of range");
        }
        self.nets.push(Net::new(
            name,
            subnet_number,
            new_net_no,
            contains_plane,
            net_class,
        ));
        new_net_no
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nets() {
        let mut nets = Nets::new();
        let c = NetClassId(0);
        assert_eq!(nets.add("GND", 1, true, c), 1);
        assert_eq!(nets.add("gnd", 2, false, c), 2);
        assert_eq!(nets.add("VCC", 1, false, c), 3);
        assert_eq!(nets.max_net_number(), 3);
        assert_eq!(nets.get_by_name("Gnd", 2).unwrap().net_number, 2);
        assert_eq!(nets.get_by_name("GND", 1).unwrap().net_number, 1);
        assert!(nets.get_by_name("GND", 3).is_none());
        assert_eq!(nets.get_all_by_name("gNd").len(), 2);
        assert!(nets.get(0).is_none() && nets.get(4).is_none());
        assert!(nets.get(1).unwrap().contains_plane());
        assert_eq!(nets.get(3).unwrap().to_string(), "Net #3 (VCC)");
        assert!(nets.get(3).unwrap().compare_to(nets.get(1).unwrap()) > 0);
        nets.get_mut(3).unwrap().set_class(NetClassId(4));
        assert_eq!(nets.get(3).unwrap().get_net_class(), NetClassId(4));
        assert!(Nets::is_normal_net_number(1));
        assert!(!Nets::is_normal_net_number(0));
        assert!(!Nets::is_normal_net_number(Nets::HIDDEN_NET_NUMBER));
    }
}
