//! The set of one-time initialization steps run eagerly at core startup.
//!
//! Each [`HarnessInitStep`] is a thin descriptor of function pointers that
//! delegate to the existing, already-idempotent provisioning code — this module
//! orchestrates and reports; it does not reimplement any download logic.
//!
//! Current steps (all non-required — failure degrades to a fallback):
//!   1. `python_runtime` — managed CPython. Only provisions eagerly when a
//!      Python backend is actually enabled (`enabled_backends` non-empty);
//!      otherwise it is a no-op and lazy consumers (Python tools / skills)
//!      resolve the interpreter on first use (#5056).
//!   2. `spacy`          — spaCy venv + `en_core_web_sm` model. Opt-in
//!      (`memory_tree.spacy_enabled`, default OFF) so a fresh install does not
//!      provision it — nor spawn the runtime Python server — on launch.
//!   3. `runtime_python_server` — long-running Python backend host. Derived:
//!      launches only when `enabled_backends` is non-empty.
//!   4. `node_runtime`   — managed Node.js (skills / MCP).
//!
//! Voice models (Piper) and Ollama stay lazy/opt-in and are
//! intentionally NOT registered here; they can be added later as steps.

use std::future::Future;
use std::pin::Pin;

use crate::openhuman::config::Config;

/// Future returned by a step's probe/run hooks. Borrows the `Config` for the
/// duration of the call.
pub type StepFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// A single startup provisioning step.
pub struct HarnessInitStep {
    /// Stable identifier used as the dedupe key and the UI i18n key.
    pub id: &'static str,
    /// Default human-readable label.
    pub label: &'static str,
    /// When true, a failure blocks the app. All steps are non-required today.
    pub required: bool,
    /// When true this step may **download/install/repair** — the only kind of
    /// work that justifies the blocking first-run overlay. When false the step
    /// is routine startup (e.g. launching an already-provisioned local server)
    /// that must run silently in the background on every launch and must NOT
    /// surface the overlay. Drives the warm-start visibility decision in
    /// [`super::ops`].
    pub provisioning: bool,
    /// Cheap, network-free probe: is this step already satisfied on this host?
    /// When true the orchestrator marks it `Done` without invoking `run`.
    ///
    /// **Must be durable across process restarts** for provisioning steps — a
    /// process-local memo (e.g. `try_cached`) reads empty on every launch and
    /// would wrongly re-enter a visible `running` state (GH-5047). Probe the
    /// on-disk install instead.
    pub is_done: for<'a> fn(&'a Config) -> StepFuture<'a, bool>,
    /// Perform the work. `Ok(())` → done; `Err(msg)` → failed/skipped.
    pub run: for<'a> fn(&'a Config) -> StepFuture<'a, Result<(), String>>,
}

/// The ordered list of mandatory eager steps.
pub fn all_steps() -> Vec<HarnessInitStep> {
    vec![
        python_runtime_step(),
        spacy_step(),
        kompress_step(),
        runtime_python_server_step(),
        // Registration-site gate: no managed toolchain to provision when
        // `runtime-node` is compiled out, so the step is absent.
        #[cfg(feature = "runtime-node")]
        node_runtime_step(),
    ]
}

// ── python_runtime ──────────────────────────────────────────────────────────

fn python_runtime_step() -> HarnessInitStep {
    HarnessInitStep {
        id: "python_runtime",
        label: "Python runtime",
        required: false,
        provisioning: true,
        is_done: |config| Box::pin(python_is_done(config)),
        run: |config| Box::pin(python_run(config)),
    }
}

/// Whether the managed interpreter must be provisioned **eagerly at boot**.
/// True only when Python is enabled AND a Python backend (spaCy / Kompress)
/// actually needs it. When no backend is enabled we skip the speculative
/// managed-CPython download entirely (#5056) — lazy consumers (Python tools /
/// skills, Python MCP servers) still resolve the interpreter on first use.
fn python_needed_eagerly(config: &Config) -> bool {
    config.runtime_python.enabled
        && !crate::openhuman::runtime::python_server::enabled_backends(config).is_empty()
}

async fn python_is_done(config: &Config) -> bool {
    if !python_needed_eagerly(config) {
        // Nothing at boot needs the interpreter → treat as satisfied. Never
        // downloads CPython speculatively.
        return true;
    }
    // Durable on-disk probe: survives restarts (unlike the process-local
    // `try_cached`), so an already-installed interpreter is detected without
    // entering a user-visible provisioning run (GH-5047). Never downloads.
    use crate::openhuman::runtime::python::PythonBootstrap;
    PythonBootstrap::new(std::sync::Arc::new(config.clone()))
        .probe_installed()
        .await
        .is_some()
}

async fn python_run(config: &Config) -> Result<(), String> {
    if !python_needed_eagerly(config) {
        return Ok(());
    }
    use crate::openhuman::runtime::python::PythonBootstrap;
    PythonBootstrap::new(std::sync::Arc::new(config.clone()))
        .resolve()
        .await
        .map(|resolved| {
            log::info!(
                "[harness_init] python runtime ready version={} source={:?}",
                resolved.version,
                resolved.source
            );
        })
        .map_err(|e| format!("{e:#}"))
}

// ── spacy ─────────────────────────────────────────────────────────────────

fn runtime_python_server_step() -> HarnessInitStep {
    HarnessInitStep {
        id: "runtime_python_server",
        label: "Runtime Python server",
        required: false,
        // Routine service startup, not an install: launching an already
        // provisioned server must stay silent (its venv is provisioned by the
        // `spacy` / `kompress` steps, which are the ones that surface progress).
        provisioning: false,
        is_done: |config| Box::pin(runtime_python_server_is_done(config)),
        run: |config| Box::pin(runtime_python_server_run(config)),
    }
}

async fn runtime_python_server_is_done(config: &Config) -> bool {
    if crate::openhuman::runtime::python_server::enabled_backends(config).is_empty() {
        return true;
    }
    let status = crate::openhuman::runtime::python_server::status().await;
    status.running
}

async fn runtime_python_server_run(config: &Config) -> Result<(), String> {
    if crate::openhuman::runtime::python_server::enabled_backends(config).is_empty() {
        return Ok(());
    }
    crate::openhuman::runtime::python_server::ensure_started(config)
        .await
        .map(|_| {
            log::info!("[harness_init] runtime Python server ready");
        })
        .map_err(|e| format!("{e:#}"))
}

fn spacy_step() -> HarnessInitStep {
    HarnessInitStep {
        id: "spacy",
        label: "spaCy language model",
        required: false,
        provisioning: true,
        is_done: |config| Box::pin(spacy_is_done(config)),
        run: |config| Box::pin(spacy_run(config)),
    }
}

async fn spacy_is_done(config: &Config) -> bool {
    if !config.runtime_python.enabled || !config.memory_tree.spacy_enabled {
        return true;
    }
    crate::openhuman::runtime::python_server::spacy_provisioned(config)
}

async fn spacy_run(config: &Config) -> Result<(), String> {
    if !config.runtime_python.enabled || !config.memory_tree.spacy_enabled {
        return Ok(());
    }
    crate::openhuman::runtime::python_server::ensure_spacy(config)
        .await
        .map(|_| {
            log::info!("[harness_init] spaCy provisioned");
        })
        .map_err(|e| format!("{e:#}"))
}

// ── node_runtime ────────────────────────────────────────────────────────────

fn kompress_step() -> HarnessInitStep {
    HarnessInitStep {
        id: "kompress",
        label: "TokenJuice ML compressor (torch)",
        required: false,
        provisioning: true,
        is_done: |config| Box::pin(kompress_is_done(config)),
        run: |config| Box::pin(kompress_run(config)),
    }
}

/// Whether this step should provision a *dedicated* Kompress venv. When spaCy is
/// also enabled, the single runtime-python server must share one interpreter, so
/// `runtime_python_server_step` installs torch into the spaCy venv instead — a
/// dedicated venv here would be unused and double the (heavy) provisioning work.
fn kompress_needs_dedicated_venv(config: &Config) -> bool {
    config.runtime_python.enabled
        && config.tokenjuice.ml_compression_enabled
        && !config.memory_tree.spacy_enabled
}

async fn kompress_is_done(config: &Config) -> bool {
    if !kompress_needs_dedicated_venv(config) {
        return true;
    }
    crate::openhuman::runtime::python_server::kompress_provisioned(config)
}

async fn kompress_run(config: &Config) -> Result<(), String> {
    if !kompress_needs_dedicated_venv(config) {
        // Shared-venv case (spaCy on) is provisioned by the server launch step.
        return Ok(());
    }
    crate::openhuman::runtime::python_server::ensure_kompress(config)
        .await
        .map(|_| {
            log::info!("[harness_init] Kompress (torch) provisioned");
        })
        .map_err(|e| format!("{e:#}"))
}

#[cfg(feature = "runtime-node")]
fn node_runtime_step() -> HarnessInitStep {
    HarnessInitStep {
        id: "node_runtime",
        label: "Node.js runtime",
        required: false,
        provisioning: true,
        is_done: |config| Box::pin(node_is_done(config)),
        run: |config| Box::pin(node_run(config)),
    }
}

#[cfg(feature = "runtime-node")]
fn build_node_bootstrap(config: &Config) -> crate::openhuman::runtime::node::NodeBootstrap {
    crate::openhuman::runtime::node::NodeBootstrap::new(std::sync::Arc::new(config.clone()))
}

#[cfg(feature = "runtime-node")]
async fn node_is_done(config: &Config) -> bool {
    if !config.node.enabled {
        return true;
    }
    // Durable on-disk probe: survives restarts (unlike the process-local
    // `try_cached`), so an already-installed toolchain is detected without
    // entering a user-visible provisioning run (GH-5047). Never downloads.
    build_node_bootstrap(config)
        .probe_installed()
        .await
        .is_some()
}

#[cfg(feature = "runtime-node")]
async fn node_run(config: &Config) -> Result<(), String> {
    if !config.node.enabled {
        return Ok(());
    }
    build_node_bootstrap(config)
        .resolve()
        .await
        .map(|resolved| {
            log::info!(
                "[harness_init] node runtime ready version={} source={:?}",
                resolved.version,
                resolved.source
            );
        })
        .map_err(|e| format!("{e:#}"))
}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;
