//! Logical parts for pin/gate swap (Java `io.specctra.parser.PartLibrary` and
//! `Network.insertLogicalParts`).

use fr_dsn::sexpr::{Atom, List, Sexpr};
use fr_engine::library::PartPin;

use crate::error::{LResult, LoadError};
use crate::loader::Loader;

/// `(logical_part_mapping NAME (component C1 C2 ...))`: components sorted (Java `TreeSet`).
#[derive(Clone, Debug, PartialEq)]
pub struct LogicalPartMapping {
    pub name: String,
    pub components: Vec<String>,
}

/// One `(pin NAME SWAP GATE GATE_SWAP GATE_PIN GATE_PIN_SWAP ...)` of a logical part.
#[derive(Clone, Debug, PartialEq)]
pub struct PartPinDef {
    pub pin_name: String,
    pub gate_name: String,
    pub gate_swap_code: i32,
    pub gate_pin_name: String,
    pub gate_pin_swap_code: i32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LogicalPartDef {
    pub name: String,
    pub pins: Vec<PartPinDef>,
}

fn atoms(l: &List) -> Vec<&Atom> {
    l.args().iter().filter_map(Sexpr::as_atom).collect()
}

/// Reads a raw `(part_library ...)` scope; `Err` where the Java reader returns `false`.
pub fn read_part_library(
    l: &List,
) -> Result<(Vec<LogicalPartMapping>, Vec<LogicalPartDef>), String> {
    let mut mappings = Vec::new();
    let mut parts = Vec::new();
    for sub in l.sublists() {
        if sub.is("logical_part_mapping") {
            let name = sub
                .args()
                .first()
                .and_then(Sexpr::as_atom)
                .ok_or("PartLibrary.read_logical_part_mapping: string expected")?
                .text
                .clone();
            let comp = sub
                .sublists()
                .next()
                .filter(|c| c.is("component") || c.is("comp"))
                .ok_or("PartLibrary.read_logical_part_mapping: component scope expected")?;
            let mut components: Vec<String> = atoms(comp).iter().map(|a| a.text.clone()).collect();
            components.sort_by(|a, b| fr_jcompat::java_string_compare(a, b).cmp(&0));
            components.dedup();
            mappings.push(LogicalPartMapping { name, components });
        } else if sub.is("logical_part") {
            let name = sub
                .args()
                .first()
                .and_then(Sexpr::as_atom)
                .ok_or("PartLibrary.read_logical_part: string expected")?
                .text
                .clone();
            let mut pins = Vec::new();
            for p in sub.find_all("pin") {
                let a = atoms(p);
                let err = "PartLibrary.read_part_pin: malformed pin";
                if a.len() < 6 {
                    return Err(err.into());
                }
                a[1].as_int().ok_or(err)?;
                pins.push(PartPinDef {
                    pin_name: a[0].text.clone(),
                    gate_name: a[2].text.clone(),
                    gate_swap_code: a[3].as_int().ok_or(err)?,
                    gate_pin_name: a[4].text.clone(),
                    gate_pin_swap_code: a[5].as_int().ok_or(err)?,
                });
            }
            parts.push(LogicalPartDef { name, pins });
        }
    }
    Ok((mappings, parts))
}

impl Loader<'_> {
    pub(crate) fn read_part_library(&mut self) -> LResult<()> {
        let Some(l) = &self.dsn.part_library else {
            return Ok(());
        };
        let (m, p) = read_part_library(l).map_err(LoadError::ParseError)?;
        self.logical_part_mappings.extend(m);
        self.logical_parts.extend(p);
        Ok(())
    }

    /// Java `Network.insertLogicalParts` (its `false` result is ignored by Java).
    pub(crate) fn insert_logical_parts(&mut self) -> LResult<()> {
        let parts = std::mem::take(&mut self.logical_parts);
        let mappings = self.logical_part_mappings.clone();
        let board = self.board()?;
        for part in &parts {
            // searchLibPackage
            let Some(mapping) = mappings.iter().find(|m| m.name == part.name) else {
                log::warn!(
                    "Network.search_lib_package: library package '{}' not found",
                    part.name
                );
                return Ok(());
            };
            let Some(first) = mapping.components.first() else {
                return Ok(());
            };
            let Some(component) = board.components.get_by_name(first) else {
                return Ok(());
            };
            let package = board.library.packages.get(component.get_package());
            let mut pins = Vec::with_capacity(part.pins.len());
            for pp in &part.pins {
                let index = package.get_pin_index(&pp.pin_name);
                if index < 0 {
                    log::warn!("Network.insert_logical_parts: package pin not found");
                    return Ok(());
                }
                pins.push(PartPin {
                    pin_index: index,
                    pin_name: pp.pin_name.clone(),
                    gate_name: pp.gate_name.clone(),
                    gate_swap_code: pp.gate_swap_code,
                    gate_pin_name: pp.gate_pin_name.clone(),
                    gate_pin_swap_code: pp.gate_pin_swap_code,
                });
            }
            board.library.logical_parts.add(part.name.clone(), pins);
        }
        for m in &mappings {
            let lp = board
                .library
                .logical_parts
                .get_by_name(&m.name)
                .map(|p| p.id);
            for c in &m.components {
                match board.components.get_by_name(c) {
                    Some(comp) => {
                        let id = comp.id;
                        board.logical_part_assignments.push((id, lp));
                    }
                    None => log::warn!("Network.insert_logical_parts: board component not found"),
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse() {
        let src = b"(part_library (logical_part_mapping LP (component U3 U1 U2)) \
                    (logical_part LP (pin 1 0 A 1 1 0) (pin 2 0 A 1 2 0 (subgate x))))";
        let top = fr_dsn::sexpr::parse(src).unwrap();
        let (m, p) = read_part_library(top[0].as_list().unwrap()).unwrap();
        assert_eq!(m[0].components, vec!["U1", "U2", "U3"]);
        assert_eq!(p[0].pins.len(), 2);
        assert_eq!(p[0].pins[1].gate_pin_name, "2");
        let bad =
            fr_dsn::sexpr::parse(b"(part_library (logical_part LP (pin 1 x A 1 1 0)))").unwrap();
        assert!(read_part_library(bad[0].as_list().unwrap()).is_err());
    }
}
