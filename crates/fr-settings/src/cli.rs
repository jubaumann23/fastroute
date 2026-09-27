//! Command-line (`settings.sources.CliSettings`, priority 60) and environment
//! (`EnvironmentVariablesSource`, priority 55) settings sources, plus
//! `LegacyRouterSettingsBridge` path canonicalization.
//!
//! Headless-path notes (Java behaviour, reproduced as is):
//! * `-mp N` maps to `router.max_passes` -> `autorouter.max_passes` (no clamping).
//! * `-mt N` maps to the flat `router.max_threads` field, which the routing
//!   pipeline does not read (it uses `optimizer.max_threads` /
//!   `autorouter.max_threads`, both set by the defaults), so it has no effect.
//! * `-us`, `-is` (and the documented `-hr`, which no code handles) only touch
//!   `GlobalSettings.routerSettings`, which never reaches a routing job; they
//!   are ignored here.
//! * `--router.via_costs=...` (as in the docs) fails: the field is
//!   `router.scoring.via_costs`.
//! * `-de F -do F` without an explicit `--router.enabled` / `--router.autorouter.enabled`
//!   forces `autorouter.enabled = true`.

use crate::jutil::{java_parse_float, java_trim};
use crate::reflect::{set_field_value, SetFieldError};
use crate::settings::RouterSettings;

/// Flat autorouter keys that move to `autorouter.*` (`LegacyRouterSettingsBridge`).
const FLAT_AUTOROUTER_KEYS: &[&str] = &[
    "enabled",
    "algorithm",
    "max_passes",
    "max_items",
    "save_intermediate_stages",
    "ignore_net_classes",
];

fn normalize_relative_path(p: &str) -> String {
    p.strip_prefix("router.").unwrap_or(p).to_lowercase()
}

/// `LegacyRouterSettingsBridge.canonicalCliPath` (result is lower-cased).
pub fn canonical_cli_path(router_relative_path: &str) -> String {
    let path = normalize_relative_path(router_relative_path);
    if FLAT_AUTOROUTER_KEYS.contains(&path.as_str()) {
        format!("autorouter.{path}")
    } else {
        path
    }
}

/// `LegacyRouterSettingsBridge.isDeprecatedFlatAutorouterPath`.
pub fn is_deprecated_flat_autorouter_path(router_relative_path: &str) -> bool {
    FLAT_AUTOROUTER_KEYS.contains(&normalize_relative_path(router_relative_path).as_str())
}

/// Router settings parsed from command-line arguments.
#[derive(Debug, Clone, Default)]
pub struct CliSettings {
    /// Only fields given on the command line are set.
    pub settings: RouterSettings,
    /// Successfully applied (property, value) pairs, in order.
    pub parsed_arguments: Vec<(String, String)>,
    /// `--key=value` arguments outside `router.*` / `optimizer.*` (e.g.
    /// `gui.enabled`, `debug.*`); not router settings, kept for the caller.
    pub other_properties: Vec<(String, String)>,
    /// Deprecation notices and failed assignments (Java logs these).
    pub warnings: Vec<String>,
}

pub const CLI_PRIORITY: i32 = 60;
pub const ENV_PRIORITY: i32 = 55;

impl CliSettings {
    /// `new CliSettings(args)` (args without the program name).
    pub fn parse<S: AsRef<str>>(args: &[S]) -> CliSettings {
        let mut cli = CliSettings::default();
        let mut has_design_input = false;
        let mut has_design_output = false;
        let mut has_explicit_router_enabled = false;
        let mut i = 0;
        while i < args.len() {
            let arg = args[i].as_ref();
            if let Some(body) = arg.strip_prefix("--") {
                if let Some((property, value)) = body.split_once('=') {
                    if property == "router.enabled" || property == "router.autorouter.enabled" {
                        has_explicit_router_enabled = true;
                    }
                    if property == "scoring-version" {
                        cli.apply("router.scoring.version", value);
                        cli.apply("optimizer.scoring.version", value);
                    } else if property.starts_with("router.")
                        || property.starts_with("optimizer.")
                        || property == "router-scoring-version"
                        || property == "optimizer-scoring-version"
                    {
                        let normalized = match property {
                            "router-scoring-version" => "router.scoring.version",
                            "optimizer-scoring-version" => "optimizer.scoring.version",
                            p => p,
                        };
                        cli.apply(normalized, value);
                    } else {
                        cli.other_properties
                            .push((property.to_string(), value.to_string()));
                    }
                }
            } else if let Some(flag) = arg.strip_prefix('-') {
                let value = match args.get(i + 1).map(AsRef::as_ref) {
                    Some(next) if !next.starts_with('-') => {
                        i += 1;
                        next
                    }
                    _ => "",
                };
                match flag {
                    "de" => has_design_input = true,
                    "do" => has_design_output = true,
                    "oit" => cli.warnings.push(
                        "The '-oit' command-line flag is deprecated; use '--router.optimizer.improvement_threshold' instead."
                            .into(),
                    ),
                    "inc" => cli.warnings.push(
                        "The '-inc' command-line flag is deprecated; use '--router.autorouter.ignore_net_classes' instead."
                            .into(),
                    ),
                    _ => {}
                }
                if let Some(property) = map_flag_to_property(flag) {
                    cli.apply(property, value);
                }
            }
            i += 1;
        }
        if has_design_input && has_design_output && !has_explicit_router_enabled {
            cli.settings.autorouter.enabled = Some(true);
        }
        cli
    }

    /// `applyRouterSetting`.
    fn apply(&mut self, property_name: &str, value: &str) {
        if property_name == "scoring-version" {
            self.apply("router.scoring.version", value);
            self.apply("optimizer.scoring.version", value);
            return;
        }
        let mut value = value.to_string();
        let mut field_path = property_name
            .strip_prefix("router.")
            .unwrap_or(property_name)
            .to_string();
        if field_path == "scoring.version" {
            field_path = "routerScoring.version".into();
        } else if property_name == "optimizer.scoring.version" {
            field_path = "optimizerScoring.version".into();
        } else if let Some(relative) = property_name.strip_prefix("router.") {
            let canonical = canonical_cli_path(relative);
            if is_deprecated_flat_autorouter_path(relative) {
                self.warnings.push(format!(
                    "Deprecated settings path 'router.{relative}'; use 'router.{canonical}' instead."
                ));
            }
            field_path = canonical;
        }
        if field_path.ends_with(".version") {
            let v = java_trim(&value).to_lowercase();
            value = match v.as_str() {
                "v1" | "legacy" => "V1_LEGACY".into(),
                "v2" | "continuous" => {
                    if field_path.starts_with("optimizer") {
                        "V2_LOWER_BOUND".into()
                    } else {
                        "V2_CONTINUOUS".into()
                    }
                }
                "lower_bound" | "lower-bound" => "V2_LOWER_BOUND".into(),
                _ => value,
            };
        }
        if property_name == "router.optimizer.improvement_threshold"
            || field_path.ends_with("improvement_threshold")
            || field_path.ends_with("optimizationImprovementThreshold")
        {
            if let Some(parsed) = java_parse_float(java_trim(&value)) {
                if parsed > 0.0 && parsed < 1.0 {
                    // Java: String.valueOf(parsed * 100.0f); round-trips exactly.
                    value = (parsed * 100.0f32).to_string();
                }
            }
        }
        match set_field_value(&mut self.settings, &field_path, &value) {
            Ok(()) => self
                .parsed_arguments
                .push((property_name.to_string(), value)),
            Err(e) => self.warnings.push(format!(
                "Failed to apply CLI router setting: {property_name}: {e}"
            )),
        }
    }
}

/// `CliSettings.mapFlagToProperty`.
fn map_flag_to_property(flag: &str) -> Option<&'static str> {
    Some(match flag {
        "mp" => "router.max_passes",
        "mt" => "router.max_threads",
        "inc" => "router.autorouter.ignore_net_classes",
        "oit" => "router.optimizer.improvement_threshold",
        "router-scoring-version" => "router.scoring.version",
        "optimizer-scoring-version" => "optimizer.scoring.version",
        "scoring-version" => "scoring-version",
        _ => return None,
    })
}

/// Router settings from `FREEROUTING__ROUTER__*` environment variables.
#[derive(Debug, Clone, Default)]
pub struct EnvironmentSettings {
    pub settings: RouterSettings,
    pub parsed_variables: Vec<(String, String)>,
    pub warnings: Vec<String>,
}

impl EnvironmentSettings {
    /// `new EnvironmentVariablesSource(Map)`: `__` becomes `.`, the path is
    /// canonicalized; values are assigned raw. Variables are processed in
    /// the given order (Java iterates a `HashMap`).
    pub fn parse<I, K, V>(vars: I) -> EnvironmentSettings
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        const PREFIX: &str = "FREEROUTING__ROUTER__";
        let mut env = EnvironmentSettings::default();
        for (k, v) in vars {
            let (raw_key, value) = (k.as_ref(), v.as_ref());
            let upper = raw_key.to_uppercase();
            let Some(rest) = upper.strip_prefix(PREFIX) else {
                continue;
            };
            let property_path = rest.replace("__", ".");
            let canonical = canonical_cli_path(&property_path);
            if is_deprecated_flat_autorouter_path(&property_path) {
                env.warnings.push(format!(
                    "Deprecated settings path '{raw_key}'; use '{PREFIX}{}' instead.",
                    canonical.replace('.', "__").to_uppercase()
                ));
            }
            match set_field_value(&mut env.settings, &canonical, value) {
                Ok(()) => env
                    .parsed_variables
                    .push((raw_key.to_string(), value.to_string())),
                Err(SetFieldError::NoSuchField(_)) => env.warnings.push(format!(
                    "Unknown router setting in environment variable: {raw_key} (property: {canonical})"
                )),
                Err(e) => env.warnings.push(format!(
                    "Failed to parse environment variable: {raw_key} = {value}: {e}"
                )),
            }
        }
        env
    }

    /// Reads the process environment.
    pub fn from_process_env() -> EnvironmentSettings {
        Self::parse(std::env::vars())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{OptimizerScoringVersion, RouterScoringVersion};

    #[test]
    fn cli_flags() {
        let cli = CliSettings::parse(&[
            "-de",
            "board.dsn",
            "-do",
            "board.ses",
            "-mp",
            "12",
            "-mt",
            "3",
            "-us",
            "global",
            "-is",
            "prioritized",
            "-inc",
            "GND,VCC",
            "--router.optimizer.improvement_threshold=0.05",
            "--scoring-version=v1",
            "--router.via_costs=150",
            "--router.scoring.via_costs=150",
            "--router.layers.routable=false,true",
            "--gui.enabled=false",
        ]);
        let s = &cli.settings;
        assert_eq!(s.autorouter.enabled, Some(true));
        assert_eq!(s.autorouter.max_passes, Some(12));
        assert_eq!(s.max_threads, Some(3));
        assert_eq!(s.optimizer.max_threads, None);
        assert_eq!(s.optimizer.item_selection_strategy, None);
        assert_eq!(
            s.autorouter.ignore_net_classes,
            Some(vec!["GND".to_string(), "VCC".to_string()])
        );
        assert_eq!(s.optimizer.optimization_improvement_threshold, Some(0.05f32 * 100.0));
        assert_eq!(s.router_scoring.version, Some(RouterScoringVersion::V1Legacy));
        assert_eq!(s.optimizer_scoring.version, Some(OptimizerScoringVersion::V1Legacy));
        assert_eq!(s.scoring.via_costs, Some(150));
        assert_eq!(s.layers.as_ref().map(Vec::len), Some(2));
        assert_eq!(
            cli.other_properties,
            vec![("gui.enabled".to_string(), "false".to_string())]
        );
        assert!(cli.warnings.iter().any(|w| w.contains("router.via_costs")));
    }

    #[test]
    fn cli_explicit_enabled_and_versions() {
        let cli = CliSettings::parse(&[
            "-de",
            "a.dsn",
            "-do",
            "a.ses",
            "--router.enabled=false",
            "--optimizer-scoring-version=v2",
            "--router-scoring-version=continuous",
        ]);
        assert_eq!(cli.settings.autorouter.enabled, Some(false));
        assert_eq!(
            cli.settings.optimizer_scoring.version,
            Some(OptimizerScoringVersion::V2LowerBound)
        );
        assert_eq!(
            cli.settings.router_scoring.version,
            Some(RouterScoringVersion::V2Continuous)
        );
        // Missing value: "-mp" followed by a flag parses "" and fails.
        let cli = CliSettings::parse(&["-mp", "-de", "x"]);
        assert_eq!(cli.settings.autorouter.max_passes, None);
        assert!(cli.warnings.iter().any(|w| w.starts_with("Failed")));
    }

    #[test]
    fn env_vars() {
        let env = EnvironmentSettings::parse([
            ("FREEROUTING__ROUTER__AUTOROUTER__MAX_PASSES", "100"),
            ("freerouting__router__optimizer__max_threads", "4"),
            ("FREEROUTING__ROUTER__MAX_ITEMS", "5"),
            ("FREEROUTING__ROUTER__BOGUS", "1"),
            ("FREEROUTING__GUI__ENABLED", "false"),
        ]);
        assert_eq!(env.settings.autorouter.max_passes, Some(100));
        assert_eq!(env.settings.optimizer.max_threads, Some(4));
        assert_eq!(env.settings.autorouter.max_items, Some(5));
        assert_eq!(env.parsed_variables.len(), 3);
        assert_eq!(env.warnings.len(), 2);
    }
}
