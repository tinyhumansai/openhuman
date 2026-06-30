//! Python runtime configuration.
//!
//! Controls how the core resolves a Python 3.12+ interpreter for features
//! that need to launch Python subprocesses such as MCP servers.
//!
//! Product direction: `runtime_python` should eventually own a managed
//! CPython distribution so OpenHuman does not depend on host Python being
//! installed correctly. The system-interpreter probe is a compatibility and
//! developer override path, not the desired long-term contract.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct RuntimePythonConfig {
    /// Master switch. When `false`, the runtime refuses to resolve a Python
    /// interpreter and callers must skip Python-backed features entirely.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Minimum accepted Python version. Interpreters older than this are
    /// rejected even when present on `PATH`.
    #[serde(default = "default_minimum_version")]
    pub minimum_version: String,
    /// Exclusive upper bound for the managed-runtime selector. The
    /// `python-build-standalone` "latest" release ships every supported series
    /// (incl. pre-release lines like 3.15.x betas, whose `bN` suffix the
    /// version parser drops — so they look like a stable `3.15.0`). Capping
    /// selection below this keeps us on a stable line with prebuilt wheels for
    /// dependencies such as spaCy. Empty string disables the cap. Default
    /// `3.14.0` → selects the latest `3.13.x`.
    #[serde(default = "default_maximum_version")]
    pub maximum_version: String,
    /// Absolute path to a directory reserved for future managed Python
    /// installs. Empty string means "use the runtime default cache dir".
    #[serde(default)]
    pub cache_dir: String,
    /// Optional upstream release tag for managed standalone Python builds.
    /// Empty string means "query the latest release at install time".
    #[serde(default)]
    pub managed_release_tag: String,
    /// When `true`, probe the host `PATH` for a compatible interpreter before
    /// attempting any managed-runtime flow. Useful for development; the
    /// intended shipped path is a managed interpreter owned by OpenHuman.
    #[serde(default = "default_prefer_system")]
    pub prefer_system: bool,
    /// Optional preferred executable name or absolute path. Examples:
    /// `python3.12`, `/opt/homebrew/bin/python3.12`.
    #[serde(default)]
    pub preferred_command: String,
}

fn default_enabled() -> bool {
    true
}

fn default_minimum_version() -> String {
    "3.12.0".to_string()
}

fn default_maximum_version() -> String {
    "3.14.0".to_string()
}

fn default_prefer_system() -> bool {
    false
}

impl Default for RuntimePythonConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            minimum_version: default_minimum_version(),
            maximum_version: default_maximum_version(),
            cache_dir: String::new(),
            managed_release_tag: String::new(),
            prefer_system: default_prefer_system(),
            preferred_command: String::new(),
        }
    }
}
