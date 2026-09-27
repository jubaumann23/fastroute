//! Port of `core/library/Package.java` and `Packages.java`.

use fr_geom::{Area, Shape, Vector};
use fr_jcompat::compare_to_ignore_case;

use super::padstack::equals_ignore_case;
use super::PackageNo;
use crate::ids::{LayerNo, PadstackNo};

/// A pin of a package (Java `Package.Pin`).
#[derive(Clone, Debug, PartialEq)]
pub struct PackagePin {
    pub name: String,
    /// Id of the pin padstack.
    pub padstack_id: PadstackNo,
    /// Location relative to the package.
    pub relative_location: Vector,
    pub rotation_in_degree: f64,
}

impl PackagePin {
    pub fn new(
        name: impl Into<String>,
        padstack_id: PadstackNo,
        relative_location: Vector,
        rotation_in_degree: f64,
    ) -> Self {
        PackagePin {
            name: name.into(),
            padstack_id,
            relative_location,
            rotation_in_degree,
        }
    }
}

/// A named keepout of a package (Java `Package.Keepout`).
#[derive(Clone, Debug)]
pub struct Keepout {
    pub name: String,
    pub area: Area,
    pub layer: LayerNo,
}

impl Keepout {
    pub fn new(name: impl Into<String>, area: Area, layer: LayerNo) -> Self {
        Keepout {
            name: name.into(),
            area,
            layer,
        }
    }
}

/// Component package template: pins with padstacks and relative locations, outline and
/// keepouts (Java `Package`).
#[derive(Clone, Debug)]
pub struct Package {
    pub name: String,
    /// 1-based id in [`Packages`].
    pub id: PackageNo,
    /// The outline of the component, may be `None`; elements may be `None` (Java null).
    pub outline: Option<Vec<Option<Shape>>>,
    pub outline_widths: Option<Vec<f64>>,
    pub outline_is_closed: Option<Vec<bool>>,
    pub keepouts: Vec<Keepout>,
    pub via_keepouts: Vec<Keepout>,
    pub place_keepout_arr: Vec<Keepout>,
    /// If false, the package is placed on the back side of the board.
    pub is_front: bool,
    pins: Vec<PackagePin>,
}

impl Package {
    /// Compares by name ignoring case.
    pub fn compare_to(&self, other: &Package) -> i32 {
        compare_to_ignore_case(&self.name, &other.name)
    }

    /// The pin with the given index; `None` (with a warning) if out of range.
    pub fn get_pin(&self, pin_index: i32) -> Option<&PackagePin> {
        if pin_index < 0 || pin_index as usize >= self.pins.len() {
            log::warn!("Package.getPin: pinIndex out of range");
            return None;
        }
        Some(&self.pins[pin_index as usize])
    }

    /// Index of the pin with the given name (case-sensitive), or -1.
    pub fn get_pin_index(&self, name: &str) -> i32 {
        self.pins
            .iter()
            .position(|p| p.name == name)
            .map_or(-1, |i| i as i32)
    }

    pub fn pin_count(&self) -> i32 {
        self.pins.len() as i32
    }

    pub fn pins(&self) -> &[PackagePin] {
        &self.pins
    }
}

impl std::fmt::Display for Package {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.name)
    }
}

/// The library of component packages (Java `Packages`). Ids are 1-based.
#[derive(Clone, Debug, Default)]
pub struct Packages {
    packages: Vec<Package>,
}

/// `name.replaceAll("::\\d+$", "")`: removes a trailing `::<digits>` (Java `$` also matches
/// before a final line terminator).
fn strip_instance_suffix(name: &str) -> String {
    let terminators = ["\r\n", "\n", "\r", "\u{85}", "\u{2028}", "\u{2029}"];
    let (body, term) = terminators
        .iter()
        .find_map(|t| name.strip_suffix(t).map(|b| (b, *t)))
        .unwrap_or((name, ""));
    let try_strip = |s: &str| -> Option<String> {
        let digits = s.bytes().rev().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            return None;
        }
        let head = &s[..s.len() - digits];
        head.strip_suffix("::").map(str::to_string)
    };
    if let Some(stripped) = try_strip(body) {
        return format!("{stripped}{term}");
    }
    name.to_string()
}

impl Packages {
    pub fn new() -> Self {
        Self::default()
    }

    /// The package with the given name (ignoring case) and side. Falls back to the name without
    /// a `::<n>` suffix and finally to a package with that name on the other side.
    pub fn get_by_name(&self, name: &str, is_front: bool) -> Option<&Package> {
        let mut other_side_package: Option<&Package> = None;
        for current in &self.packages {
            if equals_ignore_case(&current.name, name) {
                if current.is_front == is_front {
                    return Some(current);
                }
                other_side_package = Some(current);
            }
        }
        let base_name = strip_instance_suffix(name);
        if !equals_ignore_case(&base_name, name) {
            for current in &self.packages {
                if equals_ignore_case(&current.name, &base_name) {
                    if current.is_front == is_front {
                        return Some(current);
                    }
                    other_side_package = Some(current);
                }
            }
        }
        other_side_package
    }

    /// The package with the given id (1-based). Panics if out of range (Java
    /// `ArrayIndexOutOfBoundsException`).
    pub fn get(&self, package_id: PackageNo) -> &Package {
        let result = &self.packages[(package_id - 1) as usize];
        if result.id != package_id {
            log::warn!("Packages.get: inconsistent package ID");
        }
        result
    }

    pub fn count(&self) -> i32 {
        self.packages.len() as i32
    }

    pub fn iter(&self) -> std::slice::Iter<'_, Package> {
        self.packages.iter()
    }

    /// Appends a package; returns its id.
    #[allow(clippy::too_many_arguments)]
    pub fn add(
        &mut self,
        name: impl Into<String>,
        pins: Vec<PackagePin>,
        outline: Option<Vec<Option<Shape>>>,
        outline_widths: Option<Vec<f64>>,
        outline_is_closed: Option<Vec<bool>>,
        keepouts: Vec<Keepout>,
        via_keepouts: Vec<Keepout>,
        place_keepout_arr: Vec<Keepout>,
        is_front: bool,
    ) -> PackageNo {
        let id = self.packages.len() as PackageNo + 1;
        self.packages.push(Package {
            name: name.into(),
            id,
            outline,
            outline_widths,
            outline_is_closed,
            keepouts,
            via_keepouts,
            place_keepout_arr,
            is_front,
            pins,
        });
        id
    }

    /// Appends a front side package named `Package#<n>` with the given pins only.
    pub fn add_unnamed(&mut self, pins: Vec<PackagePin>) -> PackageNo {
        let name = format!("Package#{}", self.packages.len() + 1);
        self.add(
            name,
            pins,
            None,
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            true,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fr_geom::IntVector;

    fn pin(name: &str) -> PackagePin {
        PackagePin::new(name, 1, Vector::from(IntVector::new(0, 0)), 0.0)
    }

    fn add(p: &mut Packages, name: &str, front: bool) -> PackageNo {
        p.add(
            name,
            vec![pin("1"), pin("2")],
            None,
            None,
            None,
            vec![],
            vec![],
            vec![],
            front,
        )
    }

    #[test]
    fn lookup() {
        let mut p = Packages::new();
        let a = add(&mut p, "SOIC8", true);
        let b = add(&mut p, "soic8", false);
        let c = add(&mut p, "R0603", false);
        assert_eq!((a, b, c), (1, 2, 3));
        assert_eq!(p.get_by_name("Soic8", true).unwrap().id, 1);
        assert_eq!(p.get_by_name("SOIC8", false).unwrap().id, 2);
        // only a back side package exists -> returned as fallback
        assert_eq!(p.get_by_name("r0603", true).unwrap().id, 3);
        // instance suffix
        assert_eq!(p.get_by_name("R0603::12", false).unwrap().id, 3);
        assert_eq!(p.get_by_name("SOIC8::1", true).unwrap().id, 1);
        assert!(p.get_by_name("R0603::", false).is_none());
        assert!(p.get_by_name("R0603:1", false).is_none());
        assert!(p.get_by_name("X", true).is_none());
        let d = p.add_unnamed(vec![pin("A")]);
        assert_eq!(p.get(d).name, "Package#4");
        assert!(p.get(d).is_front);
    }

    #[test]
    fn pins() {
        let mut p = Packages::new();
        let a = add(&mut p, "P", true);
        let pk = p.get(a);
        assert_eq!(pk.pin_count(), 2);
        assert_eq!(pk.get_pin_index("2"), 1);
        assert_eq!(pk.get_pin_index("3"), -1);
        assert!(pk.get_pin(2).is_none());
        assert_eq!(pk.get_pin(0).unwrap().name, "1");
    }

    #[test]
    fn suffix() {
        assert_eq!(strip_instance_suffix("A::12"), "A");
        assert_eq!(strip_instance_suffix("A:::12"), "A:");
        assert_eq!(strip_instance_suffix("A::12\n"), "A\n");
        assert_eq!(strip_instance_suffix("A::x12"), "A::x12");
        assert_eq!(strip_instance_suffix("::7"), "");
    }
}
