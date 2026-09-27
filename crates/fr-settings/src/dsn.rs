//! DSN / RULES file settings: `io.specctra.parser.AutorouteSettings.readScope`,
//! `settings.sources.DsnFileSettings` (priority 20) and
//! `settings.sources.RulesFileSettings` (priority 40).
//!
//! ## When Java reads `(autoroute_settings ...)` in a DSN
//!
//! `Structure.readScope` only reads the scope while its parser
//! `LayerStructure` is still null; it is created by the first `keepout`,
//! `via_keepout`, `place_keepout`, `plane` or `autoroute_settings` scope, using
//! the `(layer ...)` scopes read *so far*. Otherwise the scope is not consumed
//! at all and its closing bracket ends the structure scope early (a Java
//! desync, not emulated here). Freerouting writes `autoroute_settings` after
//! layers/boundary/via/rule/control and before planes and keepouts, so in
//! practice it is read with all layers.
//!
//! * [`DsnFileSettings::from_structure_scope`] reproduces that ordering rule
//!   exactly on the raw `(structure ...)` list.
//! * [`DsnFileSettings::from_dsn`] uses the typed model, which keeps the
//!   *last* `autoroute_settings` scope and no ordering; it assumes the
//!   Freerouting layout (reads it with all valid layers). Differs from Java
//!   only for hand-edited files.
//!
//! During the full board load Java additionally applies the parsed scope onto
//! the job's already-merged settings (`Structure.java`,
//! `currentJob.routerSettings.applyNewValuesFrom(autorouteSettings)`), so the
//! DSN values (including the always-set `autorouter.enabled` /
//! `optimizer.enabled`) then override CLI/env values. The pipeline must
//! reproduce that with [`RouterSettings::apply_new_values_from`] using
//! [`DsnFileSettings::autoroute_settings`].

use fr_dsn::model::Dsn;
use fr_dsn::sexpr::{self, Atom, List, Sexpr};

use crate::settings::RouterSettings;

pub const DSN_PRIORITY: i32 = 20;
pub const RULES_PRIORITY: i32 = 40;

fn first_atom(l: &List) -> Option<&Atom> {
    l.args().first().and_then(Sexpr::as_atom)
}

fn atom_is(a: Option<&Atom>, kw: &str) -> bool {
    a.is_some_and(|a| !a.quoted && a.text.eq_ignore_ascii_case(kw))
}

/// `DsnFile.readOnOffScope`: true only for `on`.
fn read_on_off(l: &List) -> bool {
    atom_is(first_atom(l), "on")
}

/// `DsnFile.readIntegerScope`: the single integer argument, else 0.
fn read_integer(l: &List) -> i32 {
    match l.args() {
        [Sexpr::Atom(a)] => a.as_int().unwrap_or(0),
        _ => 0,
    }
}

/// `DsnFile.readFloatScope`: the single numeric argument, else 0.
fn read_float(l: &List) -> f64 {
    match l.args() {
        [Sexpr::Atom(a)] => a.as_num().unwrap_or(0.0),
        _ => 0.0,
    }
}

/// Parser `LayerStructure.getNo`: exact name match, else Electra's
/// "Top" -> 0 / "Bottom" -> last layer.
fn layer_no(layer_names: &[&str], name: &str) -> Option<usize> {
    if let Some(i) = layer_names.iter().position(|n| *n == name) {
        return Some(i);
    }
    if name.contains("Top") {
        return Some(0);
    }
    if name.contains("Bottom") {
        return layer_names.len().checked_sub(1);
    }
    None
}

/// `AutorouteSettings.readScope(scanner, layerStructure)` on a raw
/// `(autoroute_settings ...)` list. `layer_names` is the parser layer
/// structure (DSN layer order). Returns `None` where Java returns `null`
/// (bad `layer_rule` name or `preferred_direction` value).
///
/// The result has `set_layer_count(layer_names.len())` applied and always
/// sets `autorouter.enabled` / `optimizer.enabled` (default `on`).
pub fn read_autoroute_settings(scope: &List, layer_names: &[&str]) -> Option<RouterSettings> {
    let mut result = RouterSettings::new();
    result.set_layer_count(layer_names.len());
    let mut with_autoroute = true;
    let mut with_postroute = true;
    for sub in scope.sublists() {
        let Some(head) = sub.head() else { continue };
        match head.to_ascii_lowercase().as_str() {
            "fanout" => {}
            "autoroute" => with_autoroute = read_on_off(sub),
            "postroute" => with_postroute = read_on_off(sub),
            "vias" => result.vias_allowed = Some(read_on_off(sub)),
            "via_costs" => result.set_via_costs(read_integer(sub)),
            "plane_via_costs" => result.set_plane_via_costs(read_integer(sub)),
            "start_ripup_costs" => result.set_start_ripup_costs(read_integer(sub)),
            "layer_rule" => read_layer_rule(sub, layer_names, &mut result)?,
            _ => {}
        }
    }
    result.set_run_router(with_autoroute);
    result.set_run_optimizer(with_postroute);
    Some(result)
}

/// `AutorouteSettings.readLayerRule`.
fn read_layer_rule(l: &List, layer_names: &[&str], settings: &mut RouterSettings) -> Option<()> {
    let name = first_atom(l)?;
    let layer = layer_no(layer_names, &name.text)?;
    for sub in l.sublists() {
        let Some(head) = sub.head() else { continue };
        match head.to_ascii_lowercase().as_str() {
            "active" => settings.set_layer_active(layer, read_on_off(sub)),
            "preferred_direction" => {
                let horizontal = match sub.args() {
                    [Sexpr::Atom(a)] if atom_is(Some(a), "vertical") => false,
                    [Sexpr::Atom(a)] if atom_is(Some(a), "horizontal") => true,
                    _ => return None,
                };
                settings.set_preferred_direction_is_horizontal(layer, horizontal);
            }
            "preferred_direction_trace_costs" => {
                settings.set_preferred_direction_trace_costs(layer, read_float(sub))
            }
            "against_preferred_direction_trace_costs" => {
                settings.set_against_preferred_direction_trace_costs(layer, read_float(sub))
            }
            _ => {}
        }
    }
    Some(())
}

/// Settings extracted from a DSN file.
#[derive(Debug, Clone, Default)]
pub struct DsnFileSettings {
    /// The source settings: the parsed scope (or blank), with the layer
    /// arrays seeded from the DSN layer count when the scope did not set them.
    pub settings: RouterSettings,
    /// The parsed `(autoroute_settings ...)` alone (`scopeParameter.autorouteSettings`).
    pub autoroute_settings: Option<RouterSettings>,
    pub layer_count: usize,
}

impl DsnFileSettings {
    fn finish(autoroute: Option<RouterSettings>, layer_count: usize) -> Self {
        let mut rs = autoroute.clone().unwrap_or_default();
        if layer_count > 0 && rs.get_layer_count() == 0 {
            rs.set_layer_count(layer_count);
        }
        DsnFileSettings {
            settings: rs,
            autoroute_settings: autoroute,
            layer_count,
        }
    }

    /// From the typed model (see module docs for the ordering approximation).
    /// Layers with an unknown type are skipped, as Java drops them.
    pub fn from_dsn(dsn: &Dsn) -> Self {
        let names: Vec<&str> = dsn
            .structure
            .layers
            .iter()
            .filter(|l| l.type_ok)
            .map(|l| l.name.as_str())
            .collect();
        let autoroute = dsn
            .structure
            .autoroute_settings
            .as_ref()
            .and_then(|scope| read_autoroute_settings(scope, &names));
        Self::finish(autoroute, names.len())
    }

    /// Exact Java ordering rule on the raw `(structure ...)` list.
    /// `layer_count` is the final DSN layer count (`DsnReader.readMetadata`).
    pub fn from_structure_scope(structure: &List, layer_count: usize) -> Self {
        let mut names: Vec<&str> = Vec::new();
        let mut layer_structure_created = false;
        let mut autoroute = None;
        for sub in structure.sublists() {
            let Some(head) = sub.head() else { continue };
            match head.to_ascii_lowercase().as_str() {
                "layer" => {
                    if let Some(a) = first_atom(sub) {
                        names.push(&a.text);
                    }
                }
                "keepout" | "via_keepout" | "place_keepout" | "plane" => {
                    layer_structure_created = true;
                }
                "autoroute_settings" => {
                    if layer_structure_created {
                        // Java does not consume the scope and desyncs; stop here.
                        break;
                    }
                    layer_structure_created = true;
                    autoroute = read_autoroute_settings(sub, &names);
                }
                _ => {}
            }
        }
        Self::finish(autoroute, layer_count)
    }
}

/// `RulesReader.readRouterSettings`: the first top-level
/// `(autoroute_settings ...)` of a `(rules pcb NAME ...)` file, read against
/// the layer names discovered anywhere in the file (`(layer X` / `(layer_rule X`,
/// first occurrence order; default `F.Cu`, `B.Cu`). `None` if absent/invalid;
/// the `RulesFileSettings` source then uses blank settings.
pub fn read_rules_router_settings(src: &[u8]) -> Option<RouterSettings> {
    let top = sexpr::parse(src).ok()?;
    let rules = top.first()?.as_list()?;
    if !rules.is("rules") || !atom_is(first_atom(rules), "pcb") {
        return None;
    }
    let mut names: Vec<&str> = Vec::new();
    for item in &top {
        if let Sexpr::List(l) = item {
            discover_layers(l, &mut names);
        }
    }
    if names.is_empty() {
        names = vec!["F.Cu", "B.Cu"];
    }
    let scope = rules.find("autoroute_settings")?;
    read_autoroute_settings(scope, &names)
}

fn discover_layers<'a>(l: &'a List, names: &mut Vec<&'a str>) {
    if l.is("layer") || l.is("layer_rule") {
        if let Some(a) = first_atom(l) {
            if !a.text.chars().all(char::is_whitespace) && !names.contains(&a.text.as_str()) {
                names.push(&a.text);
            }
        }
    }
    for sub in l.sublists() {
        discover_layers(sub, names);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(src: &str) -> List {
        sexpr::parse(src.as_bytes()).unwrap()[0]
            .as_list()
            .unwrap()
            .clone()
    }

    const SCOPE: &str = "(autoroute_settings (fanout off) (autoroute on) (postroute off) \
        (vias on) (via_costs 70) (plane_via_costs 0) (start_ripup_costs 150) \
        (layer_rule F.Cu (active on) (preferred_direction vertical) \
          (preferred_direction_trace_costs 1.8) (against_preferred_direction_trace_costs 3.0)) \
        (layer_rule B.Cu (active off) (preferred_direction horizontal)))";

    #[test]
    fn autoroute_scope() {
        let s = read_autoroute_settings(&list(SCOPE), &["F.Cu", "B.Cu"]).unwrap();
        assert_eq!(s.autorouter.enabled, Some(true));
        assert_eq!(s.optimizer.enabled, Some(false));
        assert_eq!(s.vias_allowed, Some(true));
        assert_eq!(s.scoring.via_costs, Some(70));
        assert_eq!(s.scoring.plane_via_costs, Some(1));
        assert_eq!(s.scoring.start_ripup_costs, Some(150));
        assert!(!s.get_preferred_direction_is_horizontal(0));
        assert!(s.get_preferred_direction_is_horizontal(1));
        assert!(!s.get_layer_active(1));
        assert_eq!(s.get_preferred_direction_trace_costs(0), 1.8);
        assert_eq!(s.get_against_preferred_direction_trace_costs(0), 3.0);
        assert_eq!(s.get_preferred_direction_trace_costs(1), 1.0);
        assert_eq!(s.board_specific_trace_costs_applied, Some(true));
        assert_eq!(s.layers.as_ref().unwrap()[1].preferred_direction_trace_cost, None);
    }

    #[test]
    fn bad_layer_makes_whole_scope_null() {
        let l = list("(autoroute_settings (via_costs 70) (layer_rule In5.Cu (active on)))");
        assert!(read_autoroute_settings(&l, &["F.Cu", "B.Cu"]).is_none());
        let l = list("(autoroute_settings (layer_rule \"Bottom\" (active off)))");
        let s = read_autoroute_settings(&l, &["F.Cu", "In1", "B.Cu"]).unwrap();
        assert!(!s.get_layer_active(2));
        let l = list("(autoroute_settings (layer_rule F.Cu (preferred_direction diagonal)))");
        assert!(read_autoroute_settings(&l, &["F.Cu"]).is_none());
    }

    #[test]
    fn structure_ordering() {
        let st = list(&format!(
            "(structure (layer F.Cu (type signal)) (layer B.Cu (type signal)) {SCOPE} (plane GND (rect B.Cu 0 0 1 1)))"
        ));
        let d = DsnFileSettings::from_structure_scope(&st, 2);
        assert_eq!(d.autoroute_settings.as_ref().unwrap().get_layer_count(), 2);
        let st = list(&format!(
            "(structure (layer F.Cu (type signal)) (keepout (rect F.Cu 0 0 1 1)) (layer B.Cu) {SCOPE})"
        ));
        let d = DsnFileSettings::from_structure_scope(&st, 2);
        assert!(d.autoroute_settings.is_none());
        assert_eq!(d.settings.get_layer_count(), 2);
        assert_eq!(d.settings.board_specific_trace_costs_applied, Some(false));
    }

    #[test]
    fn rules_file() {
        let src = format!(
            "(rules PCB board (snap_angle fortyfive_degree) {SCOPE} (rule (width 250)) (layer_rule B.Cu))"
        );
        let s = read_rules_router_settings(src.as_bytes()).unwrap();
        // Discovered layer order: F.Cu, B.Cu (from the layer_rule scopes).
        assert_eq!(s.get_layer_count(), 2);
        assert_eq!(s.scoring.via_costs, Some(70));
        assert!(read_rules_router_settings(b"(rules PCB x (rule (width 1)))").is_none());
    }
}
