//! `[modules]` — loadable native modules.
//!
//! A module is trusted in-process code, so this block deliberately cannot add
//! one. The set of loadable modules is compiled into
//! `openhuman::modules::registry`; what config controls is whether they load at
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
}

impl Default for ModulesConfig {
    fn default() -> Self {
        Self {
            enabled: defaults::default_true(),
            allow_download: defaults::default_true(),
            install_dir: None,
            overrides: Vec::new(),
        }
    }
}

/// A local artifact standing in for one registry entry.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModuleOverride {
    /// Registry id this override replaces, e.g. `tinydocs`.
    pub id: String,
    /// Absolute path to a platform library (`.so` / `.dylib` / `.dll`).
    pub path: String,
}
