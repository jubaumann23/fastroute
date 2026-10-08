//! Hello settings (SPEC 5.1). The keys are exactly the CLI's `--router.*` flag names without the
//! dashes, the values strings, numbers or booleans. They are parsed and merged exactly like the
//! CLI does (`main.rs` `CliSettings::parse` then `headless_merger`), so a setting means the same
//! on the stock command line and over the protocol.

use std::collections::BTreeMap;

use fr_settings::CliSettings;
use serde_json::{Map, Value};

use crate::proto::{ProtoError, R};

/// The settings fixed by `hello` for the whole session.
#[derive(Clone, Debug)]
pub struct SessionSettings {
    pub threads: usize,
    /// The CLI-priority settings source (client settings plus the forced thread counts).
    pub cli: CliSettings,
    /// Settings in effect after merging, as canonical strings (sorted by key).
    pub applied: BTreeMap<String, String>,
    /// Keys the router did not recognise (sorted).
    pub unknown: Vec<String>,
}

/// Canonical text of a setting value: integers without a fraction, booleans as `true`/`false`.
fn canonical(key: &str, v: &Value) -> R<String> {
    match v {
        Value::String(s) => Ok(s.clone()),
        Value::Bool(b) => Ok(b.to_string()),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                return Ok(i.to_string());
            }
            match n.as_f64() {
                Some(f) if f.is_finite() && f.fract() == 0.0 && f.abs() < 9e15 => Ok(format!("{}", f as i64)),
                Some(f) if f.is_finite() => Ok(f.to_string()),
                _ => Err(ProtoError::bad_request(format!("hello: setting '{key}' is not a finite number"))),
            }
        }
        _ => Err(ProtoError::bad_request(format!("hello: setting '{key}' must be a string, number or boolean"))),
    }
}

impl SessionSettings {
    pub fn from_hello(threads: usize, settings: Option<&Map<String, Value>>) -> R<SessionSettings> {
        let mut given: BTreeMap<String, String> = BTreeMap::new();
        for (k, v) in settings.into_iter().flatten() {
            if k.is_empty() || k.contains('=') || k.starts_with('-') {
                return Err(ProtoError::bad_request(format!("hello: bad setting key '{k}'")));
            }
            given.insert(k.clone(), canonical(k, v)?);
        }
        let forced = [
            ("router.autorouter.max_threads", threads.to_string()),
            ("router.optimizer.max_threads", threads.to_string()),
        ];
        // The thread count is the session's, whatever the client's settings say.
        let mut args: Vec<String> = given.iter().filter(|(k, _)| !forced.iter().any(|(f, _)| f == k)).map(|(k, v)| format!("--{k}={v}")).collect();
        args.extend(forced.iter().map(|(k, v)| format!("--{k}={v}")));
        let mut cli = CliSettings::parse(&args);
        // `-de F -do F` without an explicit enable forces the autorouter on (cli.rs); the stock command
        // line the protocol mirrors always has both.
        if !given.contains_key("router.enabled") && !given.contains_key("router.autorouter.enabled") {
            cli.settings.autorouter.enabled = Some(true);
        }
        let mut applied = BTreeMap::new();
        for (k, v) in &cli.parsed_arguments {
            applied.insert(k.clone(), v.clone());
        }
        let mut unknown: Vec<String> =
            given.keys().filter(|k| !applied.contains_key(*k) && !forced.iter().any(|(f, _)| f == k)).cloned().collect();
        unknown.sort();
        Ok(SessionSettings { threads, cli, applied, unknown })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn threads_are_forced_and_unknown_keys_listed() {
        let m = json!({
            "router.autorouter.max_passes": 7,
            "router.autorouter.max_threads": 9,
            "router.copper_to_edge_clearance_um": 400.0,
            "router.no_such": 1,
            "gui.enabled": false
        });
        let s = SessionSettings::from_hello(3, m.as_object()).unwrap();
        assert_eq!(s.applied["router.autorouter.max_threads"], "3");
        assert_eq!(s.applied["router.optimizer.max_threads"], "3");
        assert_eq!(s.applied["router.autorouter.max_passes"], "7");
        assert_eq!(s.applied["router.copper_to_edge_clearance_um"], "400");
        assert_eq!(s.unknown, vec!["gui.enabled".to_string(), "router.no_such".to_string()]);
    }

    #[test]
    fn bad_value_type() {
        let m = json!({ "router.x": [1] });
        assert_eq!(SessionSettings::from_hello(1, m.as_object()).unwrap_err().code, "bad_request");
    }
}
