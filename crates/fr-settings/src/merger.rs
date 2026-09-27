//! `SettingsSource` / `SettingsMerger`: priority-ordered merge of partial settings.

use crate::settings::RouterSettings;

/// Kind of a settings source. `add_or_replace_sources` replaces a source of
/// the same kind (Java: same class).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceKind {
    Default,
    JsonFile,
    DsnFile,
    SesFile,
    RulesFile,
    Gui,
    EnvironmentVariables,
    Cli,
    Api,
}

impl SourceKind {
    /// `SettingsSource.getPriority()` of the corresponding Java source.
    pub fn priority(self) -> i32 {
        match self {
            SourceKind::Default => 0,
            SourceKind::JsonFile => 10,
            SourceKind::DsnFile => 20,
            SourceKind::SesFile => 30,
            SourceKind::RulesFile => 40,
            SourceKind::EnvironmentVariables => 55,
            SourceKind::Cli => 60,
            SourceKind::Gui => 65,
            SourceKind::Api => 70,
        }
    }
}

/// One settings source (`SettingsSource`).
#[derive(Debug, Clone)]
pub struct SettingsSource {
    pub kind: SourceKind,
    pub name: String,
    pub priority: i32,
    /// `None` = the source provides nothing (skipped by the merge).
    pub settings: Option<RouterSettings>,
}

impl SettingsSource {
    /// A source with the kind's standard priority.
    pub fn new(kind: SourceKind, name: impl Into<String>, settings: RouterSettings) -> Self {
        SettingsSource {
            kind,
            name: name.into(),
            priority: kind.priority(),
            settings: Some(settings),
        }
    }
}

/// `SettingsMerger`.
#[derive(Debug, Clone, Default)]
pub struct SettingsMerger {
    pub sources: Vec<SettingsSource>,
}

impl SettingsMerger {
    pub fn new(sources: impl IntoIterator<Item = SettingsSource>) -> Self {
        let mut m = SettingsMerger::default();
        m.add_or_replace_sources(sources);
        m
    }

    /// Replaces the first source of the same kind, else appends.
    pub fn add_or_replace_sources(&mut self, sources: impl IntoIterator<Item = SettingsSource>) {
        for s in sources {
            match self.sources.iter_mut().find(|e| e.kind == s.kind) {
                Some(e) => *e = s,
                None => self.sources.push(s),
            }
        }
    }

    /// `merge()`: stable sort by priority, the first non-null source is the
    /// base (`clone()`, which drops `resultJsonPath`), the others are applied
    /// with `applyNewValuesFrom`, then `validate()`.
    pub fn merge(&self, available_processors: i32) -> RouterSettings {
        let mut sorted: Vec<&SettingsSource> = self.sources.iter().collect();
        sorted.sort_by_key(|s| s.priority);
        let mut merged: Option<RouterSettings> = None;
        for source in sorted {
            let Some(s) = &source.settings else { continue };
            match &mut merged {
                None => merged = Some(s.java_clone()),
                Some(m) => {
                    m.apply_new_values_from(s);
                }
            }
        }
        let mut merged = merged.unwrap_or_default();
        merged.validate(available_processors);
        merged
    }
}
