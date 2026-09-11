//! `runtime-node` disabled-build stub.
//!
//! Mirrors the *type* surface that always-compiled callers name, with no-op
//! behaviour. Only what is actually reached from outside the gate lives here.
//!
//! Why a stub rather than a leaf gate: [`NodeBootstrap`] appears in the **field
//! type** of `tools::impl::system::ShellTool` (`Option<Arc<NodeBootstrap>>`, for
//! managed-Node `PATH` injection), and `shell.rs` is kernel — always compiled.
//! Deleting the module would take the shell tool with it. The registration
//! sites (`node_runtime_step`, the `javascript` controllers, `node_exec` /
//! `npm_exec`) are leaf-gated at their call sites instead, because registration
//! sites want absence.
//!
//! Off-state: `try_cached` and `probe_installed` return `None`, so the shell
//! simply never prepends a managed `bin/` directory — the same path taken when
//! `node.enabled = false`. `resolve` is the one erroring method and returns a
//! build fact so a stray caller reports something actionable.
//!
//! Note: this stub carries **only** the Node toolchain type surface.
//! [`super::ops`] and [`super::types`] are not stubbed — they are the generic
//! native-tool dispatcher and its inert serde types, always compiled so the
//! ungated `flows` backend can keep dispatching `oh:*` tools when the managed
//! Node runtime is off.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, Result};

use crate::openhuman::config::Config;

/// Returned by [`NodeBootstrap::resolve`] in a `runtime-node`-less build.
/// Phrased as a build fact, matching the `mcp` / `tui` CLI-arm convention.
pub const RUNTIME_NODE_DISABLED_MESSAGE: &str =
    "runtime-node feature disabled at compile time — rebuild with `--features runtime-node` \
     to use the managed Node.js toolchain";

/// Origin of a resolved toolchain. Never produced here; kept so caller `match`
/// arms and imports still resolve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeSource {
    /// Reused a compatible `node` already on the host.
    System,
    /// A managed distribution.
    Managed,
}

/// Fully-resolved Node toolchain. Never constructed in a disabled build.
#[derive(Debug, Clone)]
pub struct ResolvedNode {
    /// Directory to prepend to `PATH`.
    pub bin_dir: PathBuf,
    /// Absolute path to the `node` binary.
    pub node_bin: PathBuf,
    /// Absolute path to the `npm` launcher.
    pub npm_bin: PathBuf,
    /// Version string without the leading `v`.
    pub version: String,
    /// Where the toolchain came from.
    pub source: NodeSource,
}

/// Inert stand-in for the toolchain client.
pub struct NodeBootstrap {
    config: Arc<Config>,
}

impl std::fmt::Debug for NodeBootstrap {
    /// The real client redacts `Config` because it is full of secrets; the stub
    /// mirrors that so the two render identically in logs, rather than the
    /// disabled build being the one that leaks an `api_key` into a debug line.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NodeBootstrap")
            .field("resolved", &false)
            .finish_non_exhaustive()
    }
}

impl NodeBootstrap {
    /// Build a stub over this host's configuration.
    ///
    /// Takes the same argument as the real client so construction sites do not
    /// need their own gate.
    #[must_use]
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }

    /// The configuration this bootstrap would resolve under.
    #[must_use]
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Always `None`: nothing resolves in a disabled build.
    #[must_use]
    pub fn try_cached(&self) -> Option<ResolvedNode> {
        None
    }

    /// Always `None`: nothing is provisioned in a disabled build.
    pub async fn probe_installed(&self) -> Option<ResolvedNode> {
        None
    }

    /// Always an error naming the missing feature.
    ///
    /// # Errors
    ///
    /// Always, with [`RUNTIME_NODE_DISABLED_MESSAGE`].
    pub async fn resolve(&self) -> Result<ResolvedNode> {
        Err(anyhow!(RUNTIME_NODE_DISABLED_MESSAGE))
    }
}
