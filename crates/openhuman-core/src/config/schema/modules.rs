//! `[modules]` — loadable native modules.
//!
//! A module is trusted in-process code, so this block deliberately cannot add
//! one. The set of loadable modules is compiled into
//! `crate::modules::registry`; what config controls is whether they load at
//! all, whether this host may fetch them, and where a developer's own build
//! lives. A config file that could name a new artifact to `dlopen` would be a
//! remote-code-execution surface with a download step.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::defaults;

/// Configuration for the module host.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ModulesConfig {
    /// Whether modules may be loaded at all.
    ///
    /// Off means the features they provide are unavailable and say so, rather
    /// than failing at the point of use.
    pub enabled: bool,

    /// Whether a missing module may be downloaded from its pinned release.
    ///
    /// Off pins this host to artifacts that are already installed, which is what
    /// an air-gapped or strictly-reproducible deployment wants. Nothing is
    /// downloaded silently either way: the digest comes from the compiled-in
    /// registry, not from the release.
    pub allow_download: bool,

    /// Where downloaded artifacts are installed.
    ///
    /// Defaults to the user cache directory, falling back to the workspace when
    /// the host has none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub install_dir: Option<String>,

    /// Local artifacts to load instead of the pinned release.
    ///
    /// For developing a module against a live core. An override bypasses the
    /// digest check — the artifact is whatever is at that path — which is
    /// acceptable only because it is the operator's own file, named by the
    /// operator's own config.
    pub overrides: Vec<ModuleOverride>,

    /// Where modules come from when no per-module override says otherwise.
    ///
    /// `release` (the default, and the only mode CI and shipped builds use) is
    /// the pinned, digest-checked GitHub release. `linked` and `source` are for
    /// local development and local tests. Overridden by
    /// `OPENHUMAN_MODULE_SOURCE`.
    pub source: ModuleSourceMode,
}

impl Default for ModulesConfig {
    fn default() -> Self {
        Self {
            enabled: defaults::default_true(),
            allow_download: defaults::default_true(),
            install_dir: None,
            overrides: Vec::new(),
            source: ModuleSourceMode::default(),
        }
    }
}

/// Name of the global module-source environment variable.
pub const MODULE_SOURCE_ENV: &str = "OPENHUMAN_MODULE_SOURCE";

/// Name of the per-module source variable for `id`: the id upper-cased, with
/// non-alphanumerics as `_`.
#[must_use]
pub fn module_source_env_for(id: &str) -> String {
    let suffix: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    format!("{MODULE_SOURCE_ENV}_{suffix}")
}

/// Where a module is resolved from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ModuleSourceMode {
    /// The registry-pinned, digest-checked GitHub release. The default.
    #[default]
    Release,
    /// The in-process factory registered by `openhuman-modules-linked`. Local
    /// development only; falls back to `release` when no factory is registered.
    Linked,
    /// A locally built artifact. Local development only; falls back to
    /// `release` when none is available.
    Source,
}

impl ModuleSourceMode {
    /// Parse a user-supplied value (`release`, `linked`, `source`).
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "release" => Some(Self::Release),
            "linked" => Some(Self::Linked),
            "source" => Some(Self::Source),
            _ => None,
        }
    }

    /// Stable lowercase name, as written in config and logs.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Release => "release",
            Self::Linked => "linked",
            Self::Source => "source",
        }
    }
}

/// A local artifact, or a source mode, standing in for one registry entry.
///
/// Exactly one of `path` and `source` is set; an entry with both or neither is
/// ignored with a warning (see [`ModuleOverride::validate`]). `{ id, path }`
/// entries written before `source` existed keep parsing unchanged.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModuleOverride {
    /// Registry id this override replaces, e.g. `tinydocs`.
    pub id: String,
    /// Absolute path to a platform library (`.so` / `.dylib` / `.dll`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Source mode for just this module, beating `[modules] source`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<ModuleSourceMode>,
}

impl ModuleOverride {
    /// An override pointing at a local library.
    #[must_use]
    pub fn with_path(id: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            path: Some(path.into()),
            source: None,
        }
    }

    /// An override selecting a source mode.
    #[must_use]
    pub fn with_source(id: impl Into<String>, source: ModuleSourceMode) -> Self {
        Self {
            id: id.into(),
            path: None,
            source: Some(source),
        }
    }

    /// `Ok` when exactly one of `path` and `source` is set.
    ///
    /// # Errors
    ///
    /// A static description of what is wrong; it never echoes the path.
    pub fn validate(&self) -> Result<(), &'static str> {
        match (&self.path, &self.source) {
            (Some(_), None) | (None, Some(_)) => Ok(()),
            (Some(_), Some(_)) => Err("set either `path` or `source`, not both"),
            (None, None) => Err("set one of `path` or `source`"),
        }
    }
}

#[cfg(test)]
#[path = "modules_tests.rs"]
mod tests;
