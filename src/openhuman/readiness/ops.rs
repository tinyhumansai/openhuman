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
    let (status, detail, remediation) = if !storage.exists {
        (
            CheckStatus::Fail,
            "Memory folder does not exist yet.".to_string(),
            "Finish onboarding to create your memory folder, or pick a folder you can write to in \
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
    let snapshot: Value = match crate::openhuman::health::rpc::health_snapshot()
        .value
        .as_object()
        .cloned()
    {
        Some(map) => Value::Object(map),
        None => return (false, "Core health snapshot unavailable.".to_string()),
    };
    let components = snapshot
        .get("components")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if components.is_empty() {
        // No components registered yet — core is still coming up. Treat as
        // not-yet-verified rather than healthy.
        return (false, "Core is still starting up.".to_string());
    }
    let total = components.len();
    let mut ok = 0usize;
    let mut errored: Vec<String> = Vec::new();
    for (name, comp) in &components {
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
    let (exists, readable, writable, pipeline_healthy) =
        match crate::openhuman::memory::read_rpc::vault_health_check_rpc(config, None).await {
            Ok(outcome) => {
                let v = outcome.value;
                (v.exists, v.readable, v.writable, v.pipeline_healthy)
            }
            Err(e) => {
                log::debug!("[readiness] vault_health_check failed: {e}");
                (false, false, false, false)
            }
        };
    let action_dir_writable = probe_dir_writable(&config.action_dir);
    StorageProbe {
        exists,
        readable,
        writable,
        pipeline_healthy,
        action_dir_writable,
    }
}

/// Write-probe a directory by creating, writing, and removing a temp file.
fn probe_dir_writable(dir: &std::path::Path) -> bool {
    if !dir.is_dir() {
        // Try to create it — the working folder is app-owned and may not exist
        // on a fresh install.
        if std::fs::create_dir_all(dir).is_err() {
            return false;
        }
    }
    let probe = dir.join(".openhuman-readiness-writecheck.tmp");
    let ok = std::fs::write(&probe, b"ok").is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
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

async fn probe_model_connection(config: &Config) -> ModelConnProbe {
    use crate::openhuman::doctor::ModelProbeOutcome as O;
    match crate::openhuman::doctor::rpc::doctor_models(config, true).await {
        Ok(outcome) => {
            let report = outcome.value;
            let ok = report.summary.ok >= 1;
            // Label with the first provider that validated (or the first probed
            // target) — more meaningful to the user than the raw config string.
            let provider_id = report
                .entries
                .iter()
                .find(|e| matches!(e.outcome, O::Ok))
                .or_else(|| report.entries.first())
                .map(|e| e.provider.clone())
                .unwrap_or_default();
            let error = if ok {
                String::new()
            } else {
                first_probe_error(&report)
            };
            ModelConnProbe {
                ok,
                provider_id,
                error,
            }
        }
        Err(e) => ModelConnProbe {
            ok: false,
            provider_id: String::new(),
            error: e,
        },
    }
}

/// First human-readable message from a failing model probe entry.
fn first_probe_error(report: &crate::openhuman::doctor::ModelProbeReport) -> String {
    use crate::openhuman::doctor::ModelProbeOutcome as O;
    report
        .entries
        .iter()
        .find(|e| matches!(e.outcome, O::Error | O::AuthOrAccess))
        .and_then(|e| e.message.clone())
        .unwrap_or_else(|| "no reachable model provider".to_string())
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
}
