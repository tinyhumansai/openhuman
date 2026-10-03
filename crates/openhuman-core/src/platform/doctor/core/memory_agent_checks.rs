//! Memory-engine, embedding-model, and Claude Agent SDK checks.

use crate::config::Config;

use super::daemon_env_checks::{truncate_for_display, COMMAND_VERSION_PREVIEW_CHARS};
use super::run::MemoryEngineCheck;
use super::types::DiagnosticItem;

/// Report the memory engine: which one is bound and whether it answers, or
/// why memory is off. Off is a warning, not an error: a signed-out user with
/// no CortexDB key has memory off by design.
pub(super) fn check_memory_engine(memory: &MemoryEngineCheck, items: &mut Vec<DiagnosticItem>) {
    let cat = "memory_engine";
    log::debug!(
        "[doctor] check_memory_engine: engine={:?} status={}",
        memory.engine,
        memory.status
    );
    let engine = memory.engine.as_deref().unwrap_or("none");
    let reason = memory.reason.as_deref().unwrap_or("no reason given");
    items.push(match memory.status.as_str() {
        "ok" => DiagnosticItem::ok(cat, format!("memory engine '{engine}' is serving")),
        "degraded" => DiagnosticItem::warn(
            cat,
            format!("memory engine '{engine}' is degraded: {reason}"),
        ),
        "off" => DiagnosticItem::warn(cat, format!("memory is off: {reason}")),
        _ => DiagnosticItem::error(cat, format!("memory engine '{engine}' is down: {reason}")),
    });
}

// ── Embedding model health ───────────────────────────────────────

/// Probe the configured embedding provider and model.
///
/// - If the intended provider is not `"ollama"` (e.g. cloud): `Ok` — no
///   local daemon is involved and nothing to diagnose here.
/// - If Ollama is configured but the daemon at `<base_url>/api/tags` is
///   unreachable: `Error` with the pull command as the fix hint.
/// - If the daemon is reachable but the configured embedding model is not
///   listed in `/api/tags`: `Error` with `ollama pull <model>` guidance.
/// - If both daemon and model are healthy: `Ok`.
///
/// This check is synchronous (uses a small blocking HTTP call) so it fits
/// the existing `run()` contract. The timeout is capped at 3 s to avoid
/// stalling `openhuman doctor` on a very slow Ollama daemon.
pub(super) fn check_embedding_model_health(config: &Config, items: &mut Vec<DiagnosticItem>) {
    let cat = "embedding_model";

    // Resolve the effective (intended, non-probed) embedding settings.
    let local_embedding_model = config.workload_local_model("embeddings");
    let (provider, model, _dims) = tinyinference_embeddings::catalog::effective_embedding_settings(
        &config.memory.embedding_provider,
        &config.memory.embedding_model,
        config.memory.embedding_dimensions,
        local_embedding_model.as_deref(),
    );

    log::debug!("[doctor] check_embedding_model_health: provider={provider} model={model}");

    if provider != "ollama" {
        // Cloud or custom provider — no local daemon to probe.
        items.push(DiagnosticItem::ok(
            cat,
            format!("embedding provider: {provider} (model: {model}) — no local daemon required"),
        ));
        return;
    }

    // Ollama path: probe reachability then model availability.
    let base_url = tinyinference_local::ollama::ollama_base_url();
    let tags_url = format!("{}/api/tags", base_url.trim_end_matches('/'));

    log::debug!("[doctor] probing ollama at {tags_url} for embedding model {model}");

    let client = match reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            items.push(DiagnosticItem::warn(
                cat,
                format!("could not build HTTP client for Ollama probe: {e}"),
            ));
            return;
        }
    };

    let resp = match client.get(&tags_url).send() {
        Ok(r) => r,
        Err(e) => {
            items.push(DiagnosticItem::error(
                cat,
                format!(
                    "Ollama daemon unreachable at {base_url} — embedding model `{model}` cannot be used. \
                     Start Ollama, then run: ollama pull {model}  (error: {e})"
                ),
            ));
            return;
        }
    };

    if !resp.status().is_success() {
        items.push(DiagnosticItem::error(
            cat,
            format!(
                "Ollama /api/tags returned {} at {base_url} — cannot verify embedding model `{model}`. \
                 Start Ollama and run: ollama pull {model}",
                resp.status()
            ),
        ));
        return;
    }

    // Parse the tags response and look for the configured model.
    let body = match resp.text() {
        Ok(t) => t,
        Err(e) => {
            items.push(DiagnosticItem::warn(
                cat,
                format!("Ollama /api/tags response could not be read: {e}"),
            ));
            return;
        }
    };

    // Parse the JSON and extract the `models` array.  If the response is
    // malformed or the schema changed (missing `models` key), report that
    // explicitly instead of falling through to "model NOT installed".
    let models_array = match serde_json::from_str::<serde_json::Value>(&body) {
        Ok(v) => match v.get("models").and_then(|m| m.as_array()) {
            Some(arr) => arr.clone(),
            None => {
                items.push(DiagnosticItem::warn(
                    cat,
                    format!(
                        "Ollama /api/tags response is missing the `models` key — \
                         cannot verify embedding model `{model}`. Ollama API may have changed."
                    ),
                ));
                return;
            }
        },
        Err(e) => {
            items.push(DiagnosticItem::warn(
                cat,
                format!(
                    "Ollama /api/tags returned invalid JSON — \
                     cannot verify embedding model `{model}`: {e}"
                ),
            ));
            return;
        }
    };

    let model_found = models_array.iter().any(|entry| {
        entry
            .get("name")
            .and_then(serde_json::Value::as_str)
            .map(|name| model_matches(name, &model))
            .unwrap_or(false)
    });

    if model_found {
        items.push(DiagnosticItem::ok(
            cat,
            format!("embedding model `{model}` is installed and reachable at {base_url}"),
        ));
    } else {
        items.push(DiagnosticItem::error(
            cat,
            format!(
                "embedding model `{model}` is NOT installed on Ollama at {base_url}. \
                 Run: ollama pull {model}  (OpenHuman does not download models; pull it \
                 in your own Ollama)"
            ),
        ));
    }
}

// ── Claude Agent SDK check ───────────────────────────────────────

pub(super) fn check_claude_agent_sdk(config: &Config, items: &mut Vec<DiagnosticItem>) {
    let sdk = &config.claude_agent_sdk;
    if !sdk.enabled {
        return;
    }

    tracing::debug!("probe:claude_agent_sdk:entry binary={}", sdk.binary);

    // Probe the configured binary by running `<binary> --version`.
    let mut cmd = std::process::Command::new(&sdk.binary);
    cmd.arg("--version")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }

    tracing::debug!(
        "probe:claude_agent_sdk:exec binary={} cmd=--version",
        sdk.binary
    );

    match cmd.output() {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout)
                .lines()
                .next()
                .unwrap_or("(unknown version)")
                .to_string();
            tracing::info!(
                "probe:claude_agent_sdk:ok binary={} version={}",
                sdk.binary,
                version
            );
            items.push(DiagnosticItem::ok(
                "claude_agent_sdk",
                format!("claude CLI found (binary='{}'): {version}", sdk.binary),
            ));
            tracing::debug!(
                "probe:claude_agent_sdk:exit binary={} result=ok",
                sdk.binary
            );
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let preview = stderr.lines().next().unwrap_or("(no stderr)");
            tracing::warn!(
                "probe:claude_agent_sdk:warn binary={} status={:?} stderr={}",
                sdk.binary,
                output.status,
                truncate_for_display(preview, COMMAND_VERSION_PREVIEW_CHARS)
            );
            items.push(DiagnosticItem::warn(
                "claude_agent_sdk",
                format!(
                    "claude CLI execution failed (binary='{}', status={}). {}",
                    sdk.binary,
                    output.status,
                    truncate_for_display(preview, COMMAND_VERSION_PREVIEW_CHARS)
                ),
            ));
            tracing::debug!(
                "probe:claude_agent_sdk:exit binary={} result=warn",
                sdk.binary
            );
        }
        Err(err) => {
            tracing::warn!(
                "probe:claude_agent_sdk:warn binary={} err={}",
                sdk.binary,
                err
            );
            items.push(DiagnosticItem::warn(
                "claude_agent_sdk",
                format!(
                    "claude CLI not found or not executable (configured binary='{}'): {}. \
                     Install from https://claude.ai/code or set claude_agent_sdk.binary in config.",
                    sdk.binary, err
                ),
            ));
            tracing::debug!(
                "probe:claude_agent_sdk:exit binary={} result=warn",
                sdk.binary
            );
        }
    }
}

// ── Helpers ──────────────────────────────────────────────────────

pub(super) fn model_matches(installed: &str, configured: &str) -> bool {
    if installed == configured {
        return true;
    }

    if installed.contains(':') && configured.contains(':') {
        return false;
    }

    model_base(installed) == model_base(configured)
}

fn model_base(model: &str) -> &str {
    model.split(':').next().unwrap()
}
