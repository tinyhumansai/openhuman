//! The set of one-time initialization steps run eagerly at core startup.
//!
//! Each [`HarnessInitStep`] is a thin descriptor of function pointers that
//! delegate to the existing, already-idempotent provisioning code — this module
//! orchestrates and reports; it does not reimplement any download logic.
//!
//! Current steps (all non-required — failure degrades to a fallback):
//!   1. `python_runtime` — managed CPython (prerequisite for spaCy).
//!   2. `spacy`          — spaCy venv + `en_core_web_sm` model.
//!   3. `runtime_python_server` — long-running Python backend host.
//!   4. `node_runtime`   — managed Node.js (skills / MCP).
//!
//! Voice models (Whisper, Piper) and Ollama stay lazy/opt-in and are
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
    /// Cheap, network-free probe: is this step already satisfied on this host?
    /// When true the orchestrator marks it `Done` without invoking `run`.
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
        node_runtime_step(),
    ]
}

// ── python_runtime ──────────────────────────────────────────────────────────

fn python_runtime_step() -> HarnessInitStep {
    HarnessInitStep {
        id: "python_runtime",
        label: "Python runtime",
        required: false,
        is_done: |config| Box::pin(python_is_done(config)),
        run: |config| Box::pin(python_run(config)),
    }
}

async fn python_is_done(config: &Config) -> bool {
    if !config.runtime_python.enabled {
        // Disabled → nothing to provision; treat as satisfied.
        return true;
    }
    // Memoised within this process. Across restarts this is None at boot and
    // the (fast) `resolve()` probe in `run` settles it quickly.
    use crate::openhuman::runtime_python::PythonBootstrap;
    PythonBootstrap::new(config.runtime_python.clone())
        .try_cached()
        .is_some()
}

async fn python_run(config: &Config) -> Result<(), String> {
    if !config.runtime_python.enabled {
        return Ok(());
    }
    use crate::openhuman::runtime_python::PythonBootstrap;
    PythonBootstrap::new(config.runtime_python.clone())
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
        is_done: |config| Box::pin(runtime_python_server_is_done(config)),
        run: |config| Box::pin(runtime_python_server_run(config)),
    }
}

async fn runtime_python_server_is_done(config: &Config) -> bool {
    if crate::openhuman::runtime_python_server::enabled_backends(config).is_empty() {
        return true;
    }
    let status = crate::openhuman::runtime_python_server::status().await;
    status.running
}

async fn runtime_python_server_run(config: &Config) -> Result<(), String> {
    if crate::openhuman::runtime_python_server::enabled_backends(config).is_empty() {
        return Ok(());
    }
    crate::openhuman::runtime_python_server::ensure_started(config)
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
        is_done: |config| Box::pin(spacy_is_done(config)),
        run: |config| Box::pin(spacy_run(config)),
    }
}

async fn spacy_is_done(config: &Config) -> bool {
    if !config.runtime_python.enabled || !config.memory_tree.spacy_enabled {
        return true;
    }
    crate::openhuman::runtime_python_server::spacy_provisioned(config)
}

async fn spacy_run(config: &Config) -> Result<(), String> {
    if !config.runtime_python.enabled || !config.memory_tree.spacy_enabled {
        return Ok(());
    }
    crate::openhuman::runtime_python_server::ensure_spacy(config)
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
    crate::openhuman::runtime_python_server::kompress_provisioned(config)
}

async fn kompress_run(config: &Config) -> Result<(), String> {
    if !kompress_needs_dedicated_venv(config) {
        // Shared-venv case (spaCy on) is provisioned by the server launch step.
        return Ok(());
    }
    crate::openhuman::runtime_python_server::ensure_kompress(config)
        .await
        .map(|_| {
            log::info!("[harness_init] Kompress (torch) provisioned");
        })
        .map_err(|e| format!("{e:#}"))
}

fn node_runtime_step() -> HarnessInitStep {
    HarnessInitStep {
        id: "node_runtime",
        label: "Node.js runtime",
        required: false,
        is_done: |config| Box::pin(node_is_done(config)),
        run: |config| Box::pin(node_run(config)),
    }
}

fn build_node_bootstrap(config: &Config) -> crate::openhuman::runtime_node::NodeBootstrap {
    crate::openhuman::runtime_node::NodeBootstrap::new(
        config.node.clone(),
        config.workspace_dir.clone(),
        reqwest::Client::new(),
    )
}

async fn node_is_done(config: &Config) -> bool {
    if !config.node.enabled {
        return true;
    }
    build_node_bootstrap(config).try_cached().is_some()
}

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
mod tests {
    use super::*;

    #[test]
    fn all_steps_have_stable_ids_and_are_non_required() {
        let steps = all_steps();
        let ids: Vec<_> = steps.iter().map(|s| s.id).collect();
        assert_eq!(
            ids,
            vec![
                "python_runtime",
                "spacy",
                "kompress",
                "runtime_python_server",
                "node_runtime"
            ]
        );
        assert!(steps.iter().all(|s| !s.required));
        assert!(steps.iter().all(|s| !s.label.is_empty()));
    }

    #[tokio::test]
    async fn disabled_runtimes_report_done_without_work() {
        let mut config = Config::default();
        config.runtime_python.enabled = false;
        config.node.enabled = false;
        for step in all_steps() {
            assert!(
                (step.is_done)(&config).await,
                "step {} should be done when its runtime is disabled",
                step.id
            );
            assert!(
                (step.run)(&config).await.is_ok(),
                "step {} run should no-op when disabled",
                step.id
            );
        }
    }
}
