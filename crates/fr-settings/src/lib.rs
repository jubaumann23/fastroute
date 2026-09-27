//! Router settings read by the headless routing pipeline (porting unit U10).
//!
//! Java sources: `app.freerouting.settings.{RouterSettings, AutorouterSettings,
//! FanoutSettings, OptimizerSettings, RoutingCostSettings, RouterScoreSettings,
//! OptimizerScoreSettings, LayerSettings, SettingsMerger, LegacyRouterSettingsBridge}`,
//! `settings.sources.{DefaultSettings, DsnFileSettings, RulesFileSettings,
//! CliSettings, EnvironmentVariablesSource}`, `io.specctra.parser.AutorouteSettings`
//! (reader half), `util.ReflectionUtil` (settings subset) and
//! `util.TextManager.parseTimespanString`.
//!
//! Not ported: `freerouting.json` (`JsonFileSettings`; the file Java writes has
//! an empty `router` object, so it contributes nothing by default), GUI/API
//! sources, SES settings, property-change plumbing and logging.
//!
//! Headless merge order (`Freerouting.initializeCli` + `RoutingJobScheduler`):
//! Default(0) < Json(10) < Dsn(20) < Rules(40) < Env(55) < Cli(60), then the
//! scheduler re-merges with the job settings as Api(70). See [`dsn`] for how
//! the DSN `(autoroute_settings ...)` scope is applied a second time during
//! board loading.

mod jutil;

pub mod cli;
pub mod defaults;
pub mod dsn;
pub mod merger;
pub mod reflect;
pub mod settings;
pub mod timespan;

pub use cli::{CliSettings, EnvironmentSettings};
pub use defaults::{available_processors, default_settings};
pub use dsn::{read_autoroute_settings, read_rules_router_settings, DsnFileSettings};
pub use merger::{SettingsMerger, SettingsSource, SourceKind};
pub use reflect::{set_field_value, SetFieldError};
pub use settings::*;
pub use timespan::parse_timespan_string;

/// Builds the headless merger: defaults, env, CLI, and optionally the DSN
/// and RULES file sources (`Freerouting.initializeCli`).
pub fn headless_merger(
    cli: &CliSettings,
    env: &EnvironmentSettings,
    dsn: Option<&DsnFileSettings>,
    rules: Option<RouterSettings>,
    available_processors: i32,
) -> SettingsMerger {
    let mut m = SettingsMerger::new([
        SettingsSource::new(
            SourceKind::Default,
            "Default Settings",
            default_settings(available_processors),
        ),
        SettingsSource::new(SourceKind::Cli, "CLI Arguments", cli.settings.clone()),
        SettingsSource::new(
            SourceKind::EnvironmentVariables,
            "Environment Variables",
            env.settings.clone(),
        ),
    ]);
    if let Some(d) = dsn {
        m.add_or_replace_sources([SettingsSource::new(
            SourceKind::DsnFile,
            "DSN file",
            d.settings.clone(),
        )]);
    }
    if let Some(r) = rules {
        m.add_or_replace_sources([SettingsSource::new(SourceKind::RulesFile, "RULES file", r)]);
    }
    m
}
