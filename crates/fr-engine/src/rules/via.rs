//! Port of `rules/ViaInfo.java`, `ViaInfos.java` and `ViaRule.java`.
//!
//! Java shares `ViaInfo` and `ViaRule` objects by reference (a rule lists via infos, a net
//! class points to a rule) and removes them from their lists while references remain (e.g.
//! `RulesReader` replaces a via info, `Network.addViaRule` replaces a rule). To keep that
//! behaviour, the objects live in arenas addressed by stable ids ([`ViaInfoId`],
//! [`ViaRuleId`]); the Java lists are ordered id lists and removal only unlinks the id.

use std::ops::{Index, IndexMut};

use fr_jcompat::java_string_compare;

use crate::ids::{ClearanceClassNo, LayerNo, PadstackNo};
use crate::library::Padstacks;

/// Stable handle of a [`ViaInfo`] in [`ViaInfos`] (Java object identity).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ViaInfoId(pub u32);

/// Stable handle of a [`ViaRule`] in [`ViaRules`] (Java object identity).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ViaRuleId(pub u32);

/// A via padstack with clearance class and drill-to-SMD setting (Java `ViaInfo`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViaInfo {
    name: String,
    padstack: PadstackNo,
    clearance_class_index: ClearanceClassNo,
    attach_smd_allowed: bool,
}

impl ViaInfo {
    pub fn new(
        name: impl Into<String>,
        padstack: PadstackNo,
        clearance_class_index: ClearanceClassNo,
        drill_to_smd_allowed: bool,
    ) -> Self {
        ViaInfo {
            name: name.into(),
            padstack,
            clearance_class_index,
            attach_smd_allowed: drill_to_smd_allowed,
        }
    }

    pub fn get_name(&self) -> &str {
        &self.name
    }

    pub fn set_name(&mut self, name: impl Into<String>) {
        self.name = name.into();
    }

    pub fn get_padstack(&self) -> PadstackNo {
        self.padstack
    }

    pub fn set_padstack(&mut self, padstack: PadstackNo) {
        self.padstack = padstack;
    }

    pub fn get_clearance_class_index(&self) -> ClearanceClassNo {
        self.clearance_class_index
    }

    pub fn set_clearance_class_index(&mut self, index: ClearanceClassNo) {
        self.clearance_class_index = index;
    }

    /// Whether this via may attach to an SMD pad.
    pub fn attach_smd_allowed(&self) -> bool {
        self.attach_smd_allowed
    }

    pub fn set_attach_smd_allowed(&mut self, value: bool) {
        self.attach_smd_allowed = value;
    }

    /// Java `compareTo`: `String.compareTo` of the names.
    pub fn compare_to(&self, other: &ViaInfo) -> i32 {
        java_string_compare(&self.name, &other.name)
    }
}

impl std::fmt::Display for ViaInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.name)
    }
}

/// The via definitions usable in routing (Java `ViaInfos`), plus the arena of all via infos.
#[derive(Clone, Debug, Default)]
pub struct ViaInfos {
    arena: Vec<ViaInfo>,
    list: Vec<ViaInfoId>,
}

impl ViaInfos {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a via definition; `None` (Java `false`) if the name (case-sensitive) exists.
    pub fn add(&mut self, via_info: ViaInfo) -> Option<ViaInfoId> {
        if self.name_exists(&via_info.name) {
            return None;
        }
        let id = ViaInfoId(self.arena.len() as u32);
        self.arena.push(via_info);
        self.list.push(id);
        Some(id)
    }

    /// The number of vias in the list.
    pub fn count(&self) -> i32 {
        self.list.len() as i32
    }

    /// The via at `index` in the list. Panics if out of range.
    pub fn get(&self, index: i32) -> ViaInfoId {
        self.list[index as usize]
    }

    /// The via with the given name (case-sensitive) in the list, if any.
    pub fn get_by_name(&self, name: &str) -> Option<ViaInfoId> {
        self.list.iter().copied().find(|&id| self[id].name == name)
    }

    /// True if a via with the given name (case-sensitive) is in the list.
    pub fn name_exists(&self, name: &str) -> bool {
        self.get_by_name(name).is_some()
    }

    /// Removes the via from the list (it stays valid for existing references). False if it
    /// was not in the list.
    pub fn remove(&mut self, via_info: ViaInfoId) -> bool {
        match self.list.iter().position(|&id| id == via_info) {
            Some(i) => {
                self.list.remove(i);
                true
            }
            None => false,
        }
    }

    /// The ids in list order.
    pub fn iter(&self) -> impl Iterator<Item = ViaInfoId> + '_ {
        self.list.iter().copied()
    }
}

impl Index<ViaInfoId> for ViaInfos {
    type Output = ViaInfo;
    fn index(&self, id: ViaInfoId) -> &ViaInfo {
        &self.arena[id.0 as usize]
    }
}

impl IndexMut<ViaInfoId> for ViaInfos {
    fn index_mut(&mut self, id: ViaInfoId) -> &mut ViaInfo {
        &mut self.arena[id.0 as usize]
    }
}

/// An ordered list of vias used for routing; earlier vias are preferred (Java `ViaRule`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViaRule {
    pub name: String,
    list: Vec<ViaInfoId>,
}

impl ViaRule {
    pub fn new(name: impl Into<String>) -> Self {
        ViaRule {
            name: name.into(),
            list: Vec::new(),
        }
    }

    /// Java `ViaRule.EMPTY`.
    pub fn empty() -> Self {
        ViaRule::new("empty")
    }

    pub fn append_via(&mut self, via: ViaInfoId) {
        self.list.push(via);
    }

    /// Removes the first occurrence of `via`; false if not contained.
    pub fn remove_via(&mut self, via: ViaInfoId) -> bool {
        match self.list.iter().position(|&v| v == via) {
            Some(i) => {
                self.list.remove(i);
                true
            }
            None => false,
        }
    }

    pub fn via_count(&self) -> i32 {
        self.list.len() as i32
    }

    /// The via at `index`. Panics if out of range.
    pub fn get_via(&self, index: i32) -> ViaInfoId {
        self.list[index as usize]
    }

    pub fn vias(&self) -> &[ViaInfoId] {
        &self.list
    }

    /// True if `via_info` is in this rule.
    pub fn contains(&self, via_info: ViaInfoId) -> bool {
        self.list.contains(&via_info)
    }

    /// True if this rule contains a via with the given padstack.
    pub fn contains_padstack(&self, padstack: PadstackNo, via_infos: &ViaInfos) -> bool {
        self.list.iter().any(|&v| via_infos[v].padstack == padstack)
    }

    /// The first via with the given first and last layer, if any.
    pub fn get_layer_range(
        &self,
        from_layer: LayerNo,
        to_layer: LayerNo,
        via_infos: &ViaInfos,
        padstacks: &Padstacks,
    ) -> Option<ViaInfoId> {
        self.list.iter().copied().find(|&v| {
            let p = padstacks
                .get(via_infos[v].padstack)
                .expect("ViaRule: via padstack not found");
            p.from_layer() == from_layer && p.to_layer() == to_layer
        })
    }

    /// Swaps the (first) positions of `first` and `second`; false if either is missing.
    pub fn swap(&mut self, first: ViaInfoId, second: ViaInfoId) -> bool {
        let index1 = self.list.iter().position(|&v| v == first);
        let index2 = self.list.iter().position(|&v| v == second);
        let (Some(index1), Some(index2)) = (index1, index2) else {
            return false;
        };
        if index1 == index2 {
            return true;
        }
        self.list[index1] = second;
        self.list[index2] = first;
        true
    }
}

impl std::fmt::Display for ViaRule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.name)
    }
}

/// The via rules of the board (Java `BoardRules.viaRules`, a `Vector<ViaRule>`), plus the arena
/// of all rules ever added.
#[derive(Clone, Debug, Default)]
pub struct ViaRules {
    arena: Vec<ViaRule>,
    list: Vec<ViaRuleId>,
}

impl ViaRules {
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a rule to the list.
    pub fn add(&mut self, rule: ViaRule) -> ViaRuleId {
        let id = ViaRuleId(self.arena.len() as u32);
        self.arena.push(rule);
        self.list.push(id);
        id
    }

    /// Removes the rule from the list (it stays valid for net classes still referring to it).
    pub fn remove(&mut self, rule: ViaRuleId) -> bool {
        match self.list.iter().position(|&id| id == rule) {
            Some(i) => {
                self.list.remove(i);
                true
            }
            None => false,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    /// The rule at `index` in the list. Panics if out of range.
    pub fn get(&self, index: usize) -> ViaRuleId {
        self.list[index]
    }

    /// The first rule of the list, if any.
    pub fn get_first(&self) -> Option<ViaRuleId> {
        self.list.first().copied()
    }

    /// The ids in list order.
    pub fn iter(&self) -> impl Iterator<Item = ViaRuleId> + '_ {
        self.list.iter().copied()
    }
}

impl Index<ViaRuleId> for ViaRules {
    type Output = ViaRule;
    fn index(&self, id: ViaRuleId) -> &ViaRule {
        &self.arena[id.0 as usize]
    }
}

impl IndexMut<ViaRuleId> for ViaRules {
    fn index_mut(&mut self, id: ViaRuleId) -> &mut ViaRule {
        &mut self.arena[id.0 as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn via_infos() {
        let mut vi = ViaInfos::new();
        let a = vi.add(ViaInfo::new("via1", 1, 1, false)).unwrap();
        assert!(vi.add(ViaInfo::new("via1", 2, 1, false)).is_none());
        let b = vi.add(ViaInfo::new("VIA1", 2, 1, true)).unwrap();
        assert_eq!(vi.count(), 2);
        assert_eq!(vi.get(1), b);
        assert_eq!(vi.get_by_name("via1"), Some(a));
        assert!(vi[b].attach_smd_allowed());
        assert!(vi[a].compare_to(&vi[b]) > 0);
        assert!(vi.remove(a));
        assert!(!vi.remove(a));
        assert_eq!(vi.count(), 1);
        assert!(vi.get_by_name("via1").is_none());
        // removed infos stay addressable
        assert_eq!(vi[a].get_padstack(), 1);
        // the name can be added again
        let c = vi.add(ViaInfo::new("via1", 3, 2, false)).unwrap();
        assert_ne!(a, c);
        assert_eq!(vi.iter().collect::<Vec<_>>(), vec![b, c]);
    }

    #[test]
    fn via_rule() {
        let mut vi = ViaInfos::new();
        let a = vi.add(ViaInfo::new("a", 1, 1, false)).unwrap();
        let b = vi.add(ViaInfo::new("b", 2, 1, false)).unwrap();
        let c = vi.add(ViaInfo::new("c", 3, 1, false)).unwrap();
        let mut r = ViaRule::new("r");
        r.append_via(a);
        r.append_via(b);
        assert!(r.contains(a) && !r.contains(c));
        assert!(r.contains_padstack(2, &vi));
        assert!(!r.contains_padstack(3, &vi));
        assert!(r.swap(a, b));
        assert_eq!(r.vias(), &[b, a]);
        assert!(!r.swap(a, c));
        assert!(r.swap(a, a));
        assert!(r.remove_via(b));
        assert!(!r.remove_via(b));
        assert_eq!(r.get_via(0), a);
        assert_eq!(r.via_count(), 1);

        let mut rules = ViaRules::new();
        assert_eq!(rules.get_first(), None);
        let r1 = rules.add(r);
        let r2 = rules.add(ViaRule::new("r2"));
        assert_eq!(rules.get_first(), Some(r1));
        assert!(rules.remove(r1));
        assert_eq!(rules.get_first(), Some(r2));
        assert_eq!(rules[r1].name, "r");
    }
}
