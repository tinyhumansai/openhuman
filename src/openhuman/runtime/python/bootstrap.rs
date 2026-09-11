//! Python interpreter resolution, delegated to the `tinyruntime` module.
//!
//! This used to own interpreter discovery and a managed-CPython install
//! pipeline. Both now live in the `tinyruntime` module, which does the same work
//! for every language, so what is left is the adapter that turns a module answer
//! into the [`ResolvedPython`] this core's callers already name.
//!
//! # What still happens here
//!
//! [`spawn_stdio`](PythonBootstrap::spawn_stdio) launches a long-lived Python
//! child of this process — the runtime Python server, and the stdio MCP servers.
//! That is deliberately *not* the module's pooled execution: those children
//! outlive a single job, speak their own protocols, and are owned by the
//! subsystem that started them. The module resolves the interpreter; this core
//! decides what to run with it.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Result};
use tinyruntime_bus::{Language, ResolvedRuntime, RuntimeSource};

use crate::openhuman::config::Config;
use crate::openhuman::runtime::client as runtime;

/// Origin of the resolved interpreter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PythonSource {
    /// Reused a compatible interpreter already on the host.
    System,
    /// A managed standalone distribution the module downloaded and installed.
    Managed,
}

impl From<RuntimeSource> for PythonSource {
    fn from(source: RuntimeSource) -> Self {
        match source {
            RuntimeSource::System => Self::System,
            // A source this build does not know is a module from a newer
            // contract; Managed is the reading that treats it as provisioned.
            _ => Self::Managed,
        }
    }
}

/// Fully-resolved Python interpreter.
#[derive(Debug, Clone)]
pub struct ResolvedPython {
    /// Directory to prepend to `PATH` for child processes so `python`,
    /// `python3`, and `pip` resolve to the same toolchain.
    pub bin_dir: PathBuf,
    /// Absolute path to the Python executable.
    pub python_bin: PathBuf,
    /// Normalised interpreter version, e.g. `3.12.4`.
    pub version: String,
    /// Where the interpreter came from.
    pub source: PythonSource,
}

impl ResolvedPython {
    /// Adapt a module resolution, or say what it was missing.
    fn from_module(resolved: &ResolvedRuntime) -> Result<Self> {
        let python_bin = resolved
            .executable("python")
            .map(PathBuf::from)
            .ok_or_else(|| anyhow!("the resolved python toolchain reports no interpreter"))?;

        Ok(Self {
            bin_dir: PathBuf::from(&resolved.bin_dir),
            python_bin,
            version: resolved.version.clone(),
            source: resolved.source.into(),
        })
    }
}

/// Resolves the Python interpreter through the `tinyruntime` module.
pub struct PythonBootstrap {
    config: Arc<Config>,
    /// The last resolution, for the non-awaiting
    /// [`PythonBootstrap::try_cached`].
    cached: Mutex<Option<ResolvedPython>>,
}

impl std::fmt::Debug for PythonBootstrap {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PythonBootstrap")
            .field("resolved", &self.try_cached().is_some())
            .finish_non_exhaustive()
    }
}

impl PythonBootstrap {
    /// Build a bootstrap over this host's configuration.
    #[must_use]
    pub fn new(config: Arc<Config>) -> Self {
        Self {
            config,
            cached: Mutex::new(None),
        }
    }

    /// The configuration this bootstrap resolves under.
    #[must_use]
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// The memoised interpreter, without awaiting anything.
    #[must_use]
    pub fn try_cached(&self) -> Option<ResolvedPython> {
        self.cached.lock().ok().and_then(|guard| guard.clone())
    }

    /// Report an already-provisioned interpreter, without downloading one.
    pub async fn probe_installed(&self) -> Option<ResolvedPython> {
        if let Some(existing) = self.try_cached() {
            return Some(existing);
        }
        if !self.config.runtime_python.enabled {
            return None;
        }
        match runtime::resolve(&self.config, &Language::python(), false).await {
            Ok(Some(resolved)) => self.adopt(&resolved).ok(),
            Ok(None) => {
                tracing::debug!(
                    "[runtime::python] no python interpreter is provisioned yet (provisioning required)"
                );
                None
            }
            Err(error) => {
                tracing::debug!("[runtime::python] probing for an interpreter failed: {error}");
                None
            }
        }
    }

    /// Resolve the interpreter, installing a managed one if the host has none.
    ///
    /// # Errors
    ///
    /// When Python is disabled for this host, when the module or its provider
    /// cannot be loaded, or when provisioning fails.
    pub async fn resolve(&self) -> Result<ResolvedPython> {
        if let Some(existing) = self.try_cached() {
            return Ok(existing);
        }
        if !self.config.runtime_python.enabled {
            return Err(anyhow!(
                "the python runtime is disabled (set runtime_python.enabled = true to use \
                 python-backed integrations)"
            ));
        }

        let resolved = runtime::resolve(&self.config, &Language::python(), true)
            .await
            .map_err(|error| anyhow!("{error}"))?
            .ok_or_else(|| {
                anyhow!("the python runtime module reported no interpreter and did not say why")
            })?;
        self.adopt(&resolved)
    }

    /// Launch a long-lived stdio Python child.
    ///
    /// # Errors
    ///
    /// When the interpreter cannot be resolved, or the child cannot be spawned.
    pub async fn spawn_stdio(
        &self,
        spec: &super::process::PythonLaunchSpec,
    ) -> Result<tokio::process::Child> {
        let resolved = self.resolve().await?;
        super::process::spawn_stdio_process(&resolved, spec)
    }

    /// Adapt a module resolution and remember it.
    fn adopt(&self, resolved: &ResolvedRuntime) -> Result<ResolvedPython> {
        let adapted = ResolvedPython::from_module(resolved)?;
        tracing::info!(
            version = %adapted.version,
            source = ?adapted.source,
            "[runtime::python] python interpreter ready"
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
