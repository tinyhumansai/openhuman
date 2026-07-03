//! First-run readiness aggregator logic (issue #4252).
//!
//! [`evaluate`] is a **pure** function from [`ReadinessInputs`] to a
//! [`ReadinessReport`]; it owns all the plain-language / rollup / gating logic
//! and is exhaustively unit-tested without any live service. [`gather_inputs`]
//! is the async side-effecting half that runs the real probes
//! (`health`, memory vault, accessibility TCC, Ollama, model providers) and
//! maps their outputs into [`ReadinessInputs`].

use serde_json::Value;

use crate::openhuman::config::Config;
use crate::rpc::RpcOutcome;

use super::types::{
    CheckStatus, ModelConnProbe, OllamaProbe, PermState, PermissionProbe, ReadinessCheck,
    ReadinessInputs, ReadinessReport, StorageProbe,
};

// ── Pure evaluation ─────────────────────────────────────────────────────────

/// Build a plain-language [`ReadinessReport`] from already-gathered probe
/// inputs. Pure and deterministic — the unit-test surface for #4252.
pub fn evaluate(inputs: &ReadinessInputs) -> ReadinessReport {
    let os = inputs.host_os.clone();
    let mut checks = Vec::with_capacity(6);

    checks.push(core_check(inputs, &os));
    checks.push(storage_check(&inputs.storage, &os));
    checks.push(file_access_check(&inputs.storage, &os));
    checks.push(permissions_check(&inputs.permissions, &os));
    checks.push(model_check(&inputs.model_connection, &os));
    checks.push(ollama_check(&inputs.ollama, &os));

    let overall = rollup(&checks);
    ReadinessReport {
        checks,
        overall,
        model_connection_ok: inputs.model_connection.ok,
        host_os: os,
    }
}

/// Worst status across all checks. `Skipped` never worsens the rollup.
fn rollup(checks: &[ReadinessCheck]) -> CheckStatus {
    let mut worst = CheckStatus::Ok;
    for c in checks {
        if severity(c.status) > severity(worst) {
            worst = c.status;
        }
    }
    worst
}

fn severity(status: CheckStatus) -> u8 {
    match status {
        CheckStatus::Ok | CheckStatus::Skipped => 0,
        CheckStatus::Warn => 1,
        CheckStatus::Fail => 2,
    }
}

fn core_check(inputs: &ReadinessInputs, os: &str) -> ReadinessCheck {
    let (status, remediation) = if inputs.core_healthy {
        (CheckStatus::Ok, String::new())
    } else {
        (
            CheckStatus::Fail,
            "The assistant's core service reported an unhealthy component. Restart the app; if it \
             persists, open Settings → Diagnostics and share the report."
                .to_string(),
        )
    };
    ReadinessCheck {
        id: "core_health".into(),
        category: "core".into(),
        status,
        title: "Core service".into(),
        detail: inputs.core_detail.clone(),
        remediation,
        platform: os.to_string(),
        required: true,
    }
}

fn storage_check(storage: &StorageProbe, os: &str) -> ReadinessCheck {
    let (status, detail, remediation) = if let Some(err) = &storage.probe_error {
        // Probe itself failed (timeout / internal error) — do NOT claim the
        // folder is missing; that would send users to recreate a folder that is
        // probably fine (CR).
        (
            CheckStatus::Fail,
            format!("Couldn't verify memory storage: {err}"),
            "Retry in a moment. If it keeps failing, check your memory folder in \
             Settings → Memory."
                .to_string(),
        )
    } else if !storage.exists {
        (
            CheckStatus::Fail,
            "Memory folder does not exist yet.".to_string(),
            "Finish onboarding to create your memory folder, or pick a folder you can write to in \
             Settings → Memory."
                .to_string(),
        )
    } else if !storage.readable {
        (
            CheckStatus::Fail,
            "Memory folder exists but cannot be read.".to_string(),
            "Grant read access to your memory folder, or choose a different folder in \
             Settings → Memory."
                .to_string(),
        )
    } else if !storage.writable {
        (
            CheckStatus::Fail,
            "Memory folder exists but is not writable.".to_string(),
            "Grant write access to your memory folder, or choose a different folder in \
             Settings → Memory."
                .to_string(),
        )
    } else if !storage.pipeline_healthy {
        (
            CheckStatus::Warn,
            "Memory folder is writable but the memory pipeline is still warming up.".to_string(),
            "This usually clears on its own. Re-run the check in a moment.".to_string(),
        )
    } else {
        (
            CheckStatus::Ok,
            "Memory storage is readable, writable, and healthy.".to_string(),
            String::new(),
        )
    };
    ReadinessCheck {
        id: "memory_storage".into(),
        category: "storage".into(),
        status,
        title: "Memory storage".into(),
        detail,
        remediation,
        platform: os.to_string(),
        required: true,
    }
}

fn file_access_check(storage: &StorageProbe, os: &str) -> ReadinessCheck {
    let (status, detail, remediation) = if storage.action_dir_writable {
        (
            CheckStatus::Ok,
            "The assistant can read and write its working folder.".to_string(),
            String::new(),
        )
    } else {
        (
            CheckStatus::Fail,
            "The assistant's working folder is not writable.".to_string(),
            "Grant the app access to its working folder, or pick a writable location in \
             Settings → Agent access."
                .to_string(),
        )
    };
    ReadinessCheck {
        id: "file_access".into(),
        category: "storage".into(),
        status,
        title: "File access".into(),
        detail,
        remediation,
        platform: os.to_string(),
        required: true,
    }
}

fn permissions_check(perm: &PermissionProbe, os: &str) -> ReadinessCheck {
    // On platforms with no OS permission gate for desktop automation (Windows,
    // Linux), we do NOT claim the capability is working — we mark it Skipped
    // with an honest note (acceptance criterion 6).
    if !perm.os_gated {
        return ReadinessCheck {
            id: "desktop_automation_perm".into(),
            category: "permissions".into(),
            status: CheckStatus::Skipped,
            title: "Desktop control permission".into(),
            detail: "This system does not require a separate OS permission for desktop control."
                .to_string(),
            remediation: String::new(),
            platform: os.to_string(),
            required: true,
        };
    }

    // macOS: Accessibility is the primary blocker for desktop control; screen
    // recording and input monitoring are strong signals too.
    let accessibility_ok = matches!(perm.accessibility, PermState::Granted);
    let (status, detail, remediation) = if accessibility_ok {
        (
            CheckStatus::Ok,
            "Desktop control (Accessibility) permission is granted.".to_string(),
            String::new(),
        )
    } else {
        (
            CheckStatus::Fail,
            format!(
                "Desktop control needs macOS Accessibility permission (currently {}).",
                perm_word(perm.accessibility)
            ),
            "Open System Settings → Privacy & Security → Accessibility, enable OpenHuman, then use \
             \"Refresh permissions & restart core\"."
                .to_string(),
        )
    };
    ReadinessCheck {
        id: "desktop_automation_perm".into(),
        category: "permissions".into(),
        status,
        title: "Desktop control permission".into(),
        detail,
        remediation,
        platform: os.to_string(),
        required: true,
    }
}

fn model_check(model: &ModelConnProbe, os: &str) -> ReadinessCheck {
    let (status, detail, remediation) = if model.ok {
        (
            CheckStatus::Ok,
            format!(
                "Validated a working model connection ({}).",
                provider_label(model)
            ),
            String::new(),
        )
    } else if model.provider_id.is_empty() {
        (
            CheckStatus::Fail,
            "No model provider is configured yet.".to_string(),
            "Choose a local or cloud model in the previous step, then re-run this check."
                .to_string(),
        )
    } else {
        (
            CheckStatus::Fail,
            format!(
                "Could not validate a model connection ({}){}.",
                provider_label(model),
                if model.error.is_empty() {
                    String::new()
                } else {
                    format!(": {}", model.error)
                }
            ),
            "Check your API key or local model server, confirm you are online, then retry."
                .to_string(),
        )
    };
    ReadinessCheck {
        id: "model_connection".into(),
        category: "model".into(),
        status,
        title: "Model connection".into(),
        detail,
        remediation,
        platform: os.to_string(),
        required: true,
    }
}

fn ollama_check(ollama: &OllamaProbe, os: &str) -> ReadinessCheck {
    // Ollama is an OPTIONAL local path — never a hard failure.
    let (status, detail, remediation) = if !ollama.reachable {
        (
            CheckStatus::Warn,
            "No local Ollama server detected.".to_string(),
            "Optional: install Ollama and run it if you want fully-local models. Cloud models work \
             without it."
                .to_string(),
        )
    } else if ollama.models.is_empty() {
        (
            CheckStatus::Warn,
            "Ollama is running but no local models are installed.".to_string(),
            "Optional: run `ollama pull <model>` (e.g. `ollama pull llama3`) to add a local model."
                .to_string(),
        )
    } else {
        (
            CheckStatus::Ok,
            format!(
                "Ollama is running with {} local model{} available.",
                ollama.models.len(),
                if ollama.models.len() == 1 { "" } else { "s" }
            ),
            String::new(),
        )
    };
    ReadinessCheck {
        id: "ollama_local".into(),
        category: "local".into(),
        status,
        title: "Local Ollama models".into(),
        detail,
        remediation,
        platform: os.to_string(),
        required: false,
    }
}

fn perm_word(p: PermState) -> &'static str {
    match p {
        PermState::Granted => "granted",
        PermState::Denied => "denied",
        PermState::Unknown => "unknown",
        PermState::Unsupported => "unsupported",
    }
}

fn provider_label(model: &ModelConnProbe) -> String {
    if model.provider_id.is_empty() {
        "no provider".to_string()
    } else {
        model.provider_id.clone()
    }
}

// ── Async gathering (side-effecting) ────────────────────────────────────────

/// Run all live readiness probes and assemble a [`ReadinessInputs`]. Never
/// fails as a whole — a failing probe degrades to a failing/warn check via its
/// mapped field rather than aborting the report.
pub async fn gather_inputs(config: &Config) -> ReadinessInputs {
    let host_os = std::env::consts::OS.to_string();
    let (core_healthy, core_detail) = probe_core_health();
    let storage = probe_storage(config).await;
    let permissions = probe_permissions();
    let ollama = probe_ollama().await;
    let model_connection = probe_model_connection(config).await;

    ReadinessInputs {
        host_os,
        core_healthy,
        core_detail,
        storage,
        permissions,
        ollama,
        model_connection,
    }
}

/// `readiness.check_all` — gather + evaluate.
pub async fn readiness_check_all(config: &Config) -> Result<RpcOutcome<ReadinessReport>, String> {
    let inputs = gather_inputs(config).await;
    let report = evaluate(&inputs);
    log::debug!(
        "[readiness] check_all: overall={:?} model_ok={} checks={}",
        report.overall,
        report.model_connection_ok,
        report.checks.len()
    );
    Ok(RpcOutcome::single_log(report, "readiness check completed"))
}

/// `readiness.validate_model_connection` — re-probe just the model gate.
pub async fn validate_model_connection(
    config: &Config,
) -> Result<RpcOutcome<ModelConnProbe>, String> {
    let probe = probe_model_connection(config).await;
    Ok(RpcOutcome::single_log(probe, "model connection validated"))
}

/// `readiness.ollama_models` — installed local-model discovery.
pub async fn ollama_models() -> Result<RpcOutcome<OllamaProbe>, String> {
    let probe = probe_ollama().await;
    Ok(RpcOutcome::single_log(probe, "ollama models probed"))
}

fn probe_core_health() -> (bool, String) {
    core_health_from_snapshot(&crate::openhuman::health::rpc::health_snapshot().value)
}

/// Pure derivation of core health from a `health.snapshot` payload. Extracted
/// so the healthy / degraded / starting-up branches are unit-testable.
pub(crate) fn core_health_from_snapshot(snapshot: &Value) -> (bool, String) {
    let components = match snapshot.get("components").and_then(Value::as_object) {
        Some(map) => map,
        None => return (false, "Core health snapshot unavailable.".to_string()),
    };
    if components.is_empty() {
        // No components registered yet — core is still coming up. Treat as
        // not-yet-verified rather than healthy.
        return (false, "Core is still starting up.".to_string());
    }
    let total = components.len();
    let mut ok = 0usize;
    let mut errored: Vec<String> = Vec::new();
    for (name, comp) in components {
        let status = comp
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        match status {
            "ok" => ok += 1,
            "error" => errored.push(name.clone()),
            _ => {}
        }
    }
    if errored.is_empty() {
        (true, format!("{ok}/{total} core components healthy."))
    } else {
        (
            false,
            format!(
                "{ok}/{total} components healthy; unhealthy: {}.",
                errored.join(", ")
            ),
        )
    }
}

async fn probe_storage(config: &Config) -> StorageProbe {
    let (exists, readable, writable, pipeline_healthy, probe_error) =
        match crate::openhuman::memory::read_rpc::vault_health_check_rpc(config, None).await {
            Ok(outcome) => {
                let v = outcome.value;
                (v.exists, v.readable, v.writable, v.pipeline_healthy, None)
            }
            Err(e) => {
                // Probe failed — carry the error rather than masquerading as a
                // missing/unwritable folder (CR).
                log::debug!("[readiness] vault_health_check failed: {e}");
                (false, false, false, false, Some(e))
            }
        };
    let action_dir_writable = probe_dir_writable(&config.action_dir);
    StorageProbe {
        exists,
        readable,
        writable,
        pipeline_healthy,
        action_dir_writable,
        probe_error,
    }
}

/// Write-probe a directory by creating a *uniquely-named* temp file and removing
/// it. Uses `tempfile` (O_CREAT|O_EXCL) so the probe can never follow a planted
/// symlink or truncate a pre-existing fixed-name file, and concurrent readiness
/// runs can't race on the same path (CR).
fn probe_dir_writable(dir: &std::path::Path) -> bool {
    if !dir.is_dir() {
        // Try to create it — the working folder is app-owned and may not exist
        // on a fresh install.
        if std::fs::create_dir_all(dir).is_err() {
            return false;
        }
    }
    match tempfile::Builder::new()
        .prefix(".openhuman-readiness-writecheck-")
        .tempfile_in(dir)
    {
        Ok(mut file) => {
            let ok = std::io::Write::write_all(file.as_file_mut(), b"ok").is_ok();
            // `file` drops here, removing the temp file.
            ok
        }
        Err(e) => {
            log::debug!("[readiness] write-check temp create failed in {dir:?}: {e}");
            false
        }
    }
}

fn probe_permissions() -> PermissionProbe {
    let status = crate::openhuman::accessibility::detect_permissions();
    PermissionProbe {
        os_gated: cfg!(target_os = "macos"),
        accessibility: map_perm(status.accessibility),
        screen_recording: map_perm(status.screen_recording),
        input_monitoring: map_perm(status.input_monitoring),
    }
}

fn map_perm(state: crate::openhuman::accessibility::PermissionState) -> PermState {
    use crate::openhuman::accessibility::PermissionState as S;
    match state {
        S::Granted => PermState::Granted,
        S::Denied => PermState::Denied,
        S::Unknown => PermState::Unknown,
        S::Unsupported => PermState::Unsupported,
    }
}

async fn probe_ollama() -> OllamaProbe {
    let base_url = crate::openhuman::inference::local::ollama_base_url();
    let reachable =
        crate::openhuman::memory_store::factories::probe_ollama_reachable(&base_url).await;
    let models = if reachable {
        ollama_installed_models(&base_url).await
    } else {
        Vec::new()
    };
    OllamaProbe { reachable, models }
}

/// Parse installed model names from Ollama's `/api/tags` endpoint.
async fn ollama_installed_models(base_url: &str) -> Vec<String> {
    let url = format!("{}/api/tags", base_url.trim_end_matches('/'));
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            log::debug!("[readiness] ollama client build failed: {e}");
            return Vec::new();
        }
    };
    let body: Value = match client.get(&url).send().await {
        Ok(resp) if resp.status().is_success() => match resp.json().await {
            Ok(v) => v,
            Err(e) => {
                log::debug!("[readiness] ollama /api/tags bad json: {e}");
                return Vec::new();
            }
        },
        Ok(resp) => {
            log::debug!("[readiness] ollama /api/tags status {}", resp.status());
            return Vec::new();
        }
        Err(e) => {
            log::debug!("[readiness] ollama /api/tags unreachable: {e}");
            return Vec::new();
        }
    };
    parse_ollama_tags(&body)
}

/// Pure parse of an Ollama `/api/tags` body into model names. Extracted for
/// unit testing.
pub(crate) fn parse_ollama_tags(body: &Value) -> Vec<String> {
    body.get("models")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|m| m.get("name").and_then(Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Timeout for the live model-connection ping. Bounded so opening the readiness
/// panel can never hang on a stuck provider.
const MODEL_PROBE_TIMEOUT_SECS: u64 = 12;
/// Minimal prompt used to confirm the configured model actually answers.
const MODEL_PROBE_PROMPT: &str = "ping";

/// Live-probe the configured chat model. The prior implementation delegated to
/// `doctor::run_models`, which is a gutted no-op (every provider `Skipped`,
/// `summary.ok == 0`) — so the gate reported `model_connection_ok == false`
/// even for correctly-configured users, and Finish only unlocked via the skip
/// override. This instead builds the real chat provider and sends a bounded
/// one-shot ping (CR: chatgpt-codex + coderabbitai).
///
/// A build failure means no usable model is configured (kept distinct — empty
/// `provider_id` so [`model_check`] shows the "no provider configured"
/// remediation). A failed / timed-out ping means the provider is configured but
/// unreachable, surfaced with its transport error.
async fn probe_model_connection(config: &Config) -> ModelConnProbe {
    use crate::openhuman::inference::provider::{create_chat_provider, provider_for_role};

    let (provider, model) = match create_chat_provider("chat", config) {
        Ok(pair) => pair,
        Err(e) => {
            log::debug!("[readiness] model provider not configured: {e}");
            return ModelConnProbe {
                ok: false,
                provider_id: String::new(),
                error: String::new(),
            };
        }
    };

    let provider_id = {
        let slug = provider_for_role("chat", config);
        if slug.trim().is_empty() {
            model.clone()
        } else {
            slug
        }
    };

    let ping = tokio::time::timeout(
        std::time::Duration::from_secs(MODEL_PROBE_TIMEOUT_SECS),
        provider.simple_chat(MODEL_PROBE_PROMPT, &model, 0.0),
    )
    .await;
    let result = match ping {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err(format!(
            "Timed out after {MODEL_PROBE_TIMEOUT_SECS}s waiting for the model to respond."
        )),
    };
    log::debug!(
        "[readiness] model probe provider={provider_id} ok={}",
        result.is_ok()
    );
    model_conn_from_probe(provider_id, result)
}

/// Pure mapping of a live model-probe attempt onto a [`ModelConnProbe`].
/// Extracted so the validated / failed branches are unit-testable without a
/// network round-trip.
pub(crate) fn model_conn_from_probe(
    provider_id: String,
    result: Result<(), String>,
) -> ModelConnProbe {
    match result {
        Ok(()) => ModelConnProbe {
            ok: true,
            provider_id,
            error: String::new(),
        },
        Err(error) => ModelConnProbe {
            ok: false,
            provider_id,
            error,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::readiness::types::*;

    fn healthy_inputs() -> ReadinessInputs {
        ReadinessInputs {
            host_os: "macos".into(),
            core_healthy: true,
            core_detail: "3/3 core components healthy.".into(),
            storage: StorageProbe {
                exists: true,
                readable: true,
                writable: true,
                pipeline_healthy: true,
                action_dir_writable: true,
                probe_error: None,
            },
            permissions: PermissionProbe {
                os_gated: true,
                accessibility: PermState::Granted,
                screen_recording: PermState::Granted,
                input_monitoring: PermState::Granted,
            },
            ollama: OllamaProbe {
                reachable: true,
                models: vec!["llama3".into()],
            },
            model_connection: ModelConnProbe {
                ok: true,
                provider_id: "anthropic".into(),
                error: String::new(),
            },
        }
    }

    fn find(r: &ReadinessReport, id: &str) -> ReadinessCheck {
        r.checks
            .iter()
            .find(|c| c.id == id)
            .expect("check present")
            .clone()
    }

    #[test]
    fn all_healthy_rolls_up_ok() {
        let r = evaluate(&healthy_inputs());
        assert_eq!(r.overall, CheckStatus::Ok);
        assert!(r.model_connection_ok);
        assert_eq!(r.checks.len(), 6);
        assert_eq!(r.host_os, "macos");
    }

    #[test]
    fn healthy_checks_have_no_remediation() {
        let r = evaluate(&healthy_inputs());
        for c in &r.checks {
            if c.status == CheckStatus::Ok {
                assert!(
                    c.remediation.is_empty(),
                    "ok check {} had remediation",
                    c.id
                );
            }
        }
    }

    #[test]
    fn core_unhealthy_fails_and_has_remediation() {
        let mut inp = healthy_inputs();
        inp.core_healthy = false;
        inp.core_detail = "1/3 components healthy; unhealthy: memory_tree_db.".into();
        let r = evaluate(&inp);
        assert_eq!(r.overall, CheckStatus::Fail);
        let c = find(&r, "core_health");
        assert_eq!(c.status, CheckStatus::Fail);
        assert!(!c.remediation.is_empty());
        assert!(c.required);
    }

    #[test]
    fn missing_memory_folder_fails() {
        let mut inp = healthy_inputs();
        inp.storage.exists = false;
        let c = evaluate(&inp);
        assert_eq!(find(&c, "memory_storage").status, CheckStatus::Fail);
    }

    #[test]
    fn unwritable_memory_folder_fails() {
        let mut inp = healthy_inputs();
        inp.storage.writable = false;
        assert_eq!(
            find(&evaluate(&inp), "memory_storage").status,
            CheckStatus::Fail
        );
    }

    #[test]
    fn warming_pipeline_warns_not_fails() {
        let mut inp = healthy_inputs();
        inp.storage.pipeline_healthy = false;
        let r = evaluate(&inp);
        assert_eq!(find(&r, "memory_storage").status, CheckStatus::Warn);
        // warn does not fail the overall
        assert_eq!(r.overall, CheckStatus::Warn);
    }

    #[test]
    fn unwritable_action_dir_fails_file_access() {
        let mut inp = healthy_inputs();
        inp.storage.action_dir_writable = false;
        assert_eq!(
            find(&evaluate(&inp), "file_access").status,
            CheckStatus::Fail
        );
    }

    #[test]
    fn macos_missing_accessibility_fails_permissions() {
        let mut inp = healthy_inputs();
        inp.permissions.accessibility = PermState::Denied;
        let c = find(&evaluate(&inp), "desktop_automation_perm");
        assert_eq!(c.status, CheckStatus::Fail);
        assert!(c.detail.contains("denied"));
    }

    #[test]
    fn non_macos_permissions_skipped_not_ok() {
        let mut inp = healthy_inputs();
        inp.host_os = "windows".into();
        inp.permissions.os_gated = false;
        inp.permissions.accessibility = PermState::Unsupported;
        let c = find(&evaluate(&inp), "desktop_automation_perm");
        // Honest: skipped, never implies working (criterion 6).
        assert_eq!(c.status, CheckStatus::Skipped);
        assert!(c.required);
        // skipped must not worsen an otherwise-healthy rollup
        assert_eq!(evaluate(&inp).overall, CheckStatus::Ok);
    }

    #[test]
    fn no_model_connection_fails_and_flags_gate() {
        let mut inp = healthy_inputs();
        inp.model_connection = ModelConnProbe {
            ok: false,
            provider_id: "anthropic".into(),
            error: "401 unauthorized".into(),
        };
        let r = evaluate(&inp);
        assert!(!r.model_connection_ok);
        let c = find(&r, "model_connection");
        assert_eq!(c.status, CheckStatus::Fail);
        assert!(c.detail.contains("401 unauthorized"));
        assert_eq!(r.overall, CheckStatus::Fail);
    }

    #[test]
    fn no_provider_configured_has_pick_a_model_remediation() {
        let mut inp = healthy_inputs();
        inp.model_connection = ModelConnProbe {
            ok: false,
            provider_id: String::new(),
            error: String::new(),
        };
        let c = find(&evaluate(&inp), "model_connection");
        assert_eq!(c.status, CheckStatus::Fail);
        assert!(c.detail.contains("No model provider"));
    }

    #[test]
    fn ollama_absent_warns_never_fails() {
        let mut inp = healthy_inputs();
        inp.ollama = OllamaProbe {
            reachable: false,
            models: vec![],
        };
        let r = evaluate(&inp);
        let c = find(&r, "ollama_local");
        assert_eq!(c.status, CheckStatus::Warn);
        assert!(!c.required);
        // optional warn keeps overall at worst Warn, not Fail
        assert_ne!(r.overall, CheckStatus::Fail);
    }

    #[test]
    fn ollama_reachable_no_models_warns() {
        let mut inp = healthy_inputs();
        inp.ollama = OllamaProbe {
            reachable: true,
            models: vec![],
        };
        assert_eq!(
            find(&evaluate(&inp), "ollama_local").status,
            CheckStatus::Warn
        );
    }

    #[test]
    fn ollama_singular_plural_wording() {
        let mut one = healthy_inputs();
        one.ollama.models = vec!["a".into()];
        assert!(find(&evaluate(&one), "ollama_local")
            .detail
            .contains("1 local model "));
        let mut many = healthy_inputs();
        many.ollama.models = vec!["a".into(), "b".into()];
        assert!(find(&evaluate(&many), "ollama_local")
            .detail
            .contains("2 local models"));
    }

    #[test]
    fn fail_beats_warn_in_rollup() {
        let mut inp = healthy_inputs();
        inp.storage.pipeline_healthy = false; // warn
        inp.core_healthy = false; // fail
        assert_eq!(evaluate(&inp).overall, CheckStatus::Fail);
    }

    #[test]
    fn report_round_trips_through_json() {
        let r = evaluate(&healthy_inputs());
        let json = serde_json::to_value(&r).unwrap();
        let back: ReadinessReport = serde_json::from_value(json).unwrap();
        assert_eq!(r, back);
    }

    #[test]
    fn check_status_serializes_snake_case() {
        assert_eq!(serde_json::to_string(&CheckStatus::Ok).unwrap(), "\"ok\"");
        assert_eq!(
            serde_json::to_string(&CheckStatus::Skipped).unwrap(),
            "\"skipped\""
        );
    }

    #[test]
    fn parse_ollama_tags_extracts_names() {
        let body = serde_json::json!({
            "models": [
                {"name": "llama3:latest", "size": 123},
                {"name": "mistral:7b"},
                {"nope": true}
            ]
        });
        assert_eq!(
            parse_ollama_tags(&body),
            vec!["llama3:latest", "mistral:7b"]
        );
    }

    #[test]
    fn parse_ollama_tags_handles_missing_key() {
        assert!(parse_ollama_tags(&serde_json::json!({})).is_empty());
        assert!(parse_ollama_tags(&serde_json::json!({"models": "nope"})).is_empty());
    }

    #[test]
    fn core_health_all_ok() {
        let snap = serde_json::json!({
            "components": { "core": {"status": "ok"}, "memory_tree_db": {"status": "ok"} }
        });
        let (healthy, detail) = core_health_from_snapshot(&snap);
        assert!(healthy);
        assert!(detail.contains("2/2"));
    }

    #[test]
    fn core_health_reports_errored_component() {
        let snap = serde_json::json!({
            "components": { "core": {"status": "ok"}, "memory_tree_db": {"status": "error"} }
        });
        let (healthy, detail) = core_health_from_snapshot(&snap);
        assert!(!healthy);
        assert!(detail.contains("memory_tree_db"));
    }

    #[test]
    fn core_health_empty_components_is_starting_up() {
        let snap = serde_json::json!({ "components": {} });
        let (healthy, detail) = core_health_from_snapshot(&snap);
        assert!(!healthy);
        assert!(detail.contains("starting up"));
    }

    #[test]
    fn core_health_missing_components_key_unavailable() {
        let (healthy, detail) = core_health_from_snapshot(&serde_json::json!({}));
        assert!(!healthy);
        assert!(detail.contains("unavailable"));
    }

    #[test]
    fn model_conn_from_probe_ok_keeps_provider_no_error() {
        let probe = model_conn_from_probe("anthropic".into(), Ok(()));
        assert!(probe.ok);
        assert_eq!(probe.provider_id, "anthropic");
        assert!(probe.error.is_empty());
    }

    #[test]
    fn model_conn_from_probe_err_surfaces_transport_error() {
        let probe = model_conn_from_probe("anthropic".into(), Err("401 unauthorized".into()));
        assert!(!probe.ok);
        // Provider stays populated so model_check surfaces a transport error,
        // NOT the "no provider configured" branch (CR).
        assert_eq!(probe.provider_id, "anthropic");
        assert_eq!(probe.error, "401 unauthorized");
    }

    #[test]
    fn model_check_maps_configured_transport_error_distinct_from_no_provider() {
        // provider_id empty → "no provider configured"
        let no_provider = model_check(
            &ModelConnProbe {
                ok: false,
                provider_id: String::new(),
                error: String::new(),
            },
            "macos",
        );
        assert_eq!(no_provider.status, CheckStatus::Fail);
        assert!(no_provider.detail.contains("No model provider"));

        // provider_id set + error → transport-failure branch carrying the error
        let unreachable = model_check(
            &ModelConnProbe {
                ok: false,
                provider_id: "anthropic".into(),
                error: "401 unauthorized".into(),
            },
            "macos",
        );
        assert_eq!(unreachable.status, CheckStatus::Fail);
        assert!(unreachable.detail.contains("401 unauthorized"));
    }

    #[test]
    fn storage_check_probe_error_is_not_folder_missing() {
        let storage = StorageProbe {
            exists: false,
            readable: false,
            writable: false,
            pipeline_healthy: false,
            action_dir_writable: true,
            probe_error: Some("vault-health timed out".into()),
        };
        let check = storage_check(&storage, "macos");
        assert_eq!(check.status, CheckStatus::Fail);
        assert!(check.detail.contains("Couldn't verify"));
        assert!(check.detail.contains("vault-health timed out"));
        assert!(!check.detail.contains("does not exist"));
    }

    #[test]
    fn storage_check_unreadable_fails() {
        let storage = StorageProbe {
            exists: true,
            readable: false,
            writable: true,
            pipeline_healthy: true,
            action_dir_writable: true,
            probe_error: None,
        };
        let check = storage_check(&storage, "macos");
        assert_eq!(check.status, CheckStatus::Fail);
        assert!(check.detail.contains("cannot be read"));
    }

    #[test]
    fn map_perm_covers_all_states() {
        use crate::openhuman::accessibility::PermissionState as S;
        assert_eq!(map_perm(S::Granted), PermState::Granted);
        assert_eq!(map_perm(S::Denied), PermState::Denied);
        assert_eq!(map_perm(S::Unknown), PermState::Unknown);
        assert_eq!(map_perm(S::Unsupported), PermState::Unsupported);
    }

    #[test]
    fn probe_dir_writable_true_for_writable_dir() {
        let dir = tempfile::tempdir().unwrap();
        assert!(probe_dir_writable(dir.path()));
    }

    #[test]
    fn probe_dir_writable_creates_missing_dir() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("nested/child");
        assert!(probe_dir_writable(&nested));
        assert!(nested.is_dir());
    }

    /// Minimal always-OK provider so the live model probe is deterministic +
    /// offline (no real network round-trip) in the end-to-end test.
    struct OkProvider;
    #[async_trait::async_trait]
    impl crate::openhuman::inference::provider::traits::Provider for OkProvider {
        async fn chat_with_system(
            &self,
            _system_prompt: Option<&str>,
            _message: &str,
            _model: &str,
            _temperature: f64,
        ) -> anyhow::Result<String> {
            Ok("pong".to_string())
        }
    }

    /// Drives the async probe wrappers + RPC entry points end-to-end against a
    /// temp-scoped default config. A mock chat provider is installed so the live
    /// model probe resolves deterministically offline; there is no local Ollama
    /// in CI, so this executes every gather/probe branch without real network or
    /// panics.
    #[tokio::test]
    async fn gather_inputs_and_rpc_entry_points_execute() {
        // Serialize with other tests that install a global provider override so
        // they don't clobber each other's mock in parallel.
        let _serial = crate::openhuman::config::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _provider =
            crate::openhuman::inference::provider::factory::test_provider_override::install(
                std::sync::Arc::new(OkProvider),
            );

        let tmp = tempfile::tempdir().unwrap();
        let mut config = crate::openhuman::config::Config::default();
        config.action_dir = tmp.path().to_path_buf();

        let inputs = gather_inputs(&config).await;
        assert_eq!(inputs.host_os, std::env::consts::OS);
        // file-access probe wrote into our writable tempdir
        assert!(inputs.storage.action_dir_writable);

        let report = evaluate(&inputs);
        assert_eq!(report.checks.len(), 6);

        // RPC surfaces succeed and carry a report/probe payload.
        let all = readiness_check_all(&config).await.unwrap();
        assert_eq!(all.value.checks.len(), 6);

        // Mock provider answers the ping → model gate validates ok.
        let model = validate_model_connection(&config).await.unwrap();
        assert!(model.value.ok);
        assert!(model.value.error.is_empty());

        // No local Ollama in CI → unreachable, empty models; call still ok.
        let ollama = ollama_models().await.unwrap();
        assert!(ollama.value.models.is_empty() || ollama.value.reachable);
    }

    /// Provider whose ping fails — exercises the transport-error arm of the live
    /// model probe (configured provider that can't be reached).
    struct ErrProvider;
    #[async_trait::async_trait]
    impl crate::openhuman::inference::provider::traits::Provider for ErrProvider {
        async fn chat_with_system(
            &self,
            _system_prompt: Option<&str>,
            _message: &str,
            _model: &str,
            _temperature: f64,
        ) -> anyhow::Result<String> {
            anyhow::bail!("401 unauthorized")
        }
    }

    #[tokio::test]
    async fn model_probe_maps_ping_failure_to_transport_error() {
        let _serial = crate::openhuman::config::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _provider =
            crate::openhuman::inference::provider::factory::test_provider_override::install(
                std::sync::Arc::new(ErrProvider),
            );
        let config = crate::openhuman::config::Config::default();

        let probe = validate_model_connection(&config).await.unwrap().value;
        assert!(!probe.ok);
        // Provider resolved (build succeeded) → provider_id populated and the
        // transport error is carried, NOT reduced to "no provider configured".
        assert!(!probe.provider_id.is_empty());
        assert!(probe.error.contains("401 unauthorized"));
    }
}
