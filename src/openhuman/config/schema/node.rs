//! Node.js managed runtime configuration.
//!
//! Controls whether the core bootstraps a Node.js toolchain for skills that
//! require `node`/`npm` (e.g. agentskills.io packages with build steps).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct NodeConfig {
    /// Master switch. When `false`, the Node runtime is not resolved and
    /// `node_exec` / `npm_exec` tools are not registered. Checked before any
    /// bus call, so a disabled runtime costs nothing.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Target Node.js release line, e.g. `v22.11.0`. Pin to a known LTS for
    /// reproducibility.
    ///
    /// Carried to the `tinyruntime` module on every request; the Node provider
    /// there matches on the **major** component, so a host `v22.8.0` satisfies
    /// `v22.11.0`. Set `prefer_system = false` for an exact toolchain.
    #[serde(default = "default_version")]
    pub version: String,
    /// Absolute path to a directory where managed Node distributions are
    /// installed. Empty string means "use the platform cache directory"
    /// (resolved by the `tinyruntime` module, which owns the install).
    #[serde(default)]
    pub cache_dir: String,
    /// When `true` and a system `node` binary is found on `PATH` whose major
    /// version matches `version`, reuse it instead of installing a managed
    /// toolchain. Disable for reproducible CI / airgapped deployments — that is
    /// also how a caller pins an exact version rather than a major line.
    #[serde(default = "default_prefer_system")]
    pub prefer_system: bool,
}

fn default_enabled() -> bool {
    true
}

fn default_version() -> String {
    "v22.11.0".to_string()
}

fn default_prefer_system() -> bool {
    true
}

impl Default for NodeConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            version: default_version(),
            cache_dir: String::new(),
            prefer_system: default_prefer_system(),
        }
    }
}
