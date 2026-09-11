//! Node.js toolchain resolution, delegated to the `tinyruntime` module.
//!
//! This used to be the orchestrator for a download-and-install pipeline: probe
//! the host, fetch a distribution, verify its digest, unpack it, promote it into
//! a cache, and remember it. All of that now lives in the `tinyruntime` module,
//! which does it identically for every language, so what is left here is the
//! adapter that turns a module answer into the [`ResolvedNode`] this core's
//! callers already name.
//!
//! # Why the type survived the move
//!
//! `ShellTool` holds an `Option<Arc<NodeBootstrap>>` as a field and is kernel —
//! always compiled. Keeping the type and its three methods meant the migration
//! did not have to touch the shell, the two exec tools, or the harness
//! initialiser, which is what made it reviewable.
//!
//! # The cache is still here, and still earns its place
//!
//! The module memoises resolution too, so this looks redundant. It is not:
//! [`try_cached`](NodeBootstrap::try_cached) must answer *without awaiting*,
//! because the shell consults it on every command to decide whether to prepend a
//! managed `bin/` directory to `PATH`. A blocking call there would make every
//! unrelated shell command wait on a bus round trip, and a download on the first
//! one.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Result};
use tinyruntime_bus::{Language, ResolvedRuntime, RuntimeSource};

use crate::openhuman::config::Config;
use crate::openhuman::runtime::client as runtime;

/// Origin of the resolved toolchain — feeds into logging and lets the caller
/// decide whether to surface a "Node was downloaded to …" message in the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeSource {
    /// Reused a compatible `node` already on the host.
    System,
    /// A managed distribution the module downloaded and installed.
    Managed,
}

impl From<RuntimeSource> for NodeSource {
    fn from(source: RuntimeSource) -> Self {
        match source {
            RuntimeSource::System => Self::System,
            // A source this build does not know is a module from a newer
            // contract. Managed is the safe reading: it is the one that makes a
            // caller treat the toolchain as something the core provisioned.
            _ => Self::Managed,
        }
    }
}

/// Fully-resolved Node.js toolchain.
#[derive(Debug, Clone)]
pub struct ResolvedNode {
    /// Directory to prepend to `PATH` for child processes so `node`, `npm`,
    /// `npx`, and `corepack` resolve to this toolchain's binaries.
    pub bin_dir: PathBuf,
    /// Absolute path to the `node` binary.
    pub node_bin: PathBuf,
    /// Absolute path to the `npm` launcher.
    ///
    /// The launcher, not the script it points at: the Unix distributions ship
    /// `bin/npm` as a symlink into a JavaScript file under `lib/`, and invoking
    /// that file directly is not the supported contract.
    pub npm_bin: PathBuf,
    /// Version string without the leading `v`, e.g. `22.11.0`.
    pub version: String,
    /// Where the toolchain came from.
    pub source: NodeSource,
}

impl ResolvedNode {
    /// Adapt a module resolution, or say what it was missing.
    ///
    /// `npm` is derived when the provider does not report it rather than being
    /// required: a toolchain without `npm` is unusual but perfectly able to run
    /// `node`, and refusing the whole resolution would take `node_exec` down
    /// with `npm_exec`.
    fn from_module(resolved: &ResolvedRuntime) -> Result<Self> {
        let bin_dir = PathBuf::from(&resolved.bin_dir);
        let node_bin = resolved
            .executable("node")
            .map(PathBuf::from)
            .ok_or_else(|| anyhow!("the resolved node toolchain reports no `node` binary"))?;
        let npm_bin = resolved.executable("npm").map_or_else(
            || bin_dir.join(if cfg!(windows) { "npm.cmd" } else { "npm" }),
            PathBuf::from,
        );

        Ok(Self {
            bin_dir,
            node_bin,
            npm_bin,
            version: resolved
                .version
                .trim_start_matches(['v', 'V'])
                .trim()
                .to_string(),
            source: resolved.source.into(),
        })
    }
}

/// Resolves the Node.js toolchain through the `tinyruntime` module.
///
/// Hold one per session so every tool that needs Node shares the same memoised
/// answer rather than each asking the bus.
pub struct NodeBootstrap {
    config: Arc<Config>,
    /// The last resolution, for the non-awaiting [`NodeBootstrap::try_cached`].
    cached: Mutex<Option<ResolvedNode>>,
}

impl std::fmt::Debug for NodeBootstrap {
    /// `Config` is large and full of secrets; what identifies a bootstrap is
    /// whether it has resolved yet.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NodeBootstrap")
            .field("resolved", &self.try_cached().is_some())
            .finish_non_exhaustive()
    }
}

impl NodeBootstrap {
    /// Build a bootstrap over this host's configuration.
    #[must_use]
    pub fn new(config: Arc<Config>) -> Self {
        Self {
            config,
            cached: Mutex::new(None),
        }
    }

    /// The configuration this bootstrap resolves under.
    ///
    /// Exposed because the pooled-execution path needs the same configuration to
    /// build its request, and threading a second copy through every tool would
    /// give two answers to "which version".
    #[must_use]
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// The memoised toolchain, without awaiting anything.
    ///
    /// Returns `None` when nothing has resolved yet. Callers use this where a
    /// blocking wait would change the meaning of an unrelated operation — the
    /// shell's `PATH` injection being the one that matters.
    #[must_use]
    pub fn try_cached(&self) -> Option<ResolvedNode> {
        self.cached.lock().ok().and_then(|guard| guard.clone())
    }

    /// Report an already-provisioned toolchain, without downloading one.
    ///
    /// Asks the module to resolve without installing, so a warm start detects an
    /// existing install and a cold one reports nothing rather than spending a
    /// user's first minute on a download they did not ask for.
    pub async fn probe_installed(&self) -> Option<ResolvedNode> {
        if let Some(existing) = self.try_cached() {
            return Some(existing);
        }
        if !self.config.node.enabled {
            return None;
        }
        match runtime::resolve(&self.config, &Language::nodejs(), false).await {
            Ok(Some(resolved)) => self.adopt(&resolved).ok(),
            Ok(None) => {
                tracing::debug!(
                    "[runtime::node] no node toolchain is provisioned yet (provisioning required)"
                );
                None
            }
            Err(error) => {
                tracing::debug!("[runtime::node] probing for a node toolchain failed: {error}");
                None
            }
        }
    }

    /// Resolve the toolchain, installing a managed one if the host has none.
    ///
    /// # Errors
    ///
    /// When Node is disabled for this host, when the module or its provider
    /// cannot be loaded, or when provisioning fails.
    pub async fn resolve(&self) -> Result<ResolvedNode> {
        if let Some(existing) = self.try_cached() {
            return Ok(existing);
        }
        if !self.config.node.enabled {
            return Err(anyhow!(
                "the node runtime is disabled (set node.enabled = true to use tools that need node or npm)"
            ));
        }

        let resolved = runtime::resolve(&self.config, &Language::nodejs(), true)
            .await
            .map_err(|error| anyhow!("{error}"))?
            .ok_or_else(|| {
                anyhow!("the node runtime module reported no toolchain and did not say why")
            })?;
        self.adopt(&resolved)
    }

    /// Adapt a module resolution and remember it.
    fn adopt(&self, resolved: &ResolvedRuntime) -> Result<ResolvedNode> {
        let adapted = ResolvedNode::from_module(resolved)?;
        tracing::info!(
            version = %adapted.version,
            source = ?adapted.source,
            "[runtime::node] node toolchain ready"
        );
        if let Ok(mut cached) = self.cached.lock() {
            *cached = Some(adapted.clone());
        }
        Ok(adapted)
    }
}

#[cfg(test)]
#[path = "bootstrap_tests.rs"]
mod tests;
