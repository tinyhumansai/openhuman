//! JSON-RPC / CLI controller surface for inline autocomplete.

use crate::openhuman::autocomplete::{
    self, AcceptedCompletion, AutocompleteAcceptParams, AutocompleteAcceptResult,
    AutocompleteCurrentParams, AutocompleteCurrentResult, AutocompleteDebugFocusResult,
    AutocompleteSetStyleParams, AutocompleteSetStyleResult, AutocompleteStartParams,
    AutocompleteStartResult, AutocompleteStatus, AutocompleteStopParams, AutocompleteStopResult,
};
use crate::rpc::RpcOutcome;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::process::Stdio;
use tokio::time::{self, Duration};

#[derive(Debug, Clone)]
pub struct AutocompleteStartCliOptions {
    pub debounce_ms: Option<u64>,
    pub serve: bool,
    pub spawn: bool,
}

pub async fn autocomplete_status() -> Result<RpcOutcome<AutocompleteStatus>, String> {
    let result = autocomplete::global_engine().status().await;
    let app = result.app_name.as_deref().unwrap_or("n/a");
    let suggestion_chars = result
        .suggestion
        .as_ref()
        .map(|s| s.value.chars().count())
        .unwrap_or(0);
    let last_error = result.last_error.as_deref().unwrap_or("none");
    let status_log = format!(
        "[autocomplete] status running={} enabled={} phase={} debounce={}ms app={} suggestion_chars={} last_error={}",
        if result.running { "yes" } else { "no" },
        if result.enabled { "yes" } else { "no" },
        result.phase,
        result.debounce_ms,
        app,
        suggestion_chars,
        last_error
    );
    Ok(RpcOutcome::new(
        result,
        vec!["autocomplete status fetched".to_string(), status_log],
    ))
}

pub async fn autocomplete_start(
    payload: AutocompleteStartParams,
) -> Result<RpcOutcome<AutocompleteStartResult>, String> {
    let requested_debounce = payload.debounce_ms;
    let result = autocomplete::global_engine().start(payload).await?;
    let status = autocomplete::global_engine().status().await;
    let start_log = format!(
        "[autocomplete] start started={} requested_debounce_ms={} running={} phase={} effective_debounce_ms={}",
        if result.started { "yes" } else { "no" },
        requested_debounce
            .map(|v| v.to_string())
            .unwrap_or_else(|| "default".to_string()),
        if status.running { "yes" } else { "no" },
        status.phase,
        status.debounce_ms
    );
    Ok(RpcOutcome::new(
        result,
        vec!["autocomplete started".to_string(), start_log],
    ))
}

pub async fn autocomplete_stop(
    payload: Option<AutocompleteStopParams>,
) -> Result<RpcOutcome<AutocompleteStopResult>, String> {
    let reason = payload
        .as_ref()
        .and_then(|value| value.reason.clone())
        .unwrap_or_else(|| "none".to_string());
    let result = autocomplete::global_engine().stop(payload).await;
    let status = autocomplete::global_engine().status().await;
    let stop_log = format!(
        "[autocomplete] stop stopped={} reason={} running={} phase={}",
        if result.stopped { "yes" } else { "no" },
        reason,
        if status.running { "yes" } else { "no" },
        status.phase
    );
    Ok(RpcOutcome::new(
        result,
        vec!["autocomplete stopped".to_string(), stop_log],
    ))
}

pub async fn autocomplete_current(
    payload: Option<AutocompleteCurrentParams>,
) -> Result<RpcOutcome<AutocompleteCurrentResult>, String> {
    let override_chars = payload
        .as_ref()
        .and_then(|params| params.context.as_ref())
        .map(|text| text.chars().count())
        .unwrap_or(0);
    let result = autocomplete::global_engine().current(payload).await?;
    let suggestion_chars = result
        .suggestion
        .as_ref()
        .map(|s| s.value.chars().count())
        .unwrap_or(0);
    let current_log = format!(
        "[autocomplete] current app={} context_chars={} override_chars={} suggestion_chars={}",
        result.app_name.as_deref().unwrap_or("n/a"),
        result.context.chars().count(),
        override_chars,
        suggestion_chars
    );
    Ok(RpcOutcome::new(
        result,
        vec!["autocomplete suggestion fetched".to_string(), current_log],
    ))
}

pub async fn autocomplete_debug_focus() -> Result<RpcOutcome<AutocompleteDebugFocusResult>, String>
{
    let result = autocomplete::global_engine().debug_focus().await?;
    let focus_log = format!(
        "[autocomplete] debug_focus app={} role={} context_chars={} has_raw_error={}",
        result.app_name.as_deref().unwrap_or("n/a"),
        result.role.as_deref().unwrap_or("n/a"),
        result.context.chars().count(),
        if result.raw_error.is_some() {
            "yes"
        } else {
            "no"
        }
    );
    Ok(RpcOutcome::new(
        result,
        vec!["autocomplete focus debug fetched".to_string(), focus_log],
    ))
}

pub async fn autocomplete_accept(
    payload: AutocompleteAcceptParams,
) -> Result<RpcOutcome<AutocompleteAcceptResult>, String> {
    let explicit_chars = payload
        .suggestion
        .as_ref()
        .map(|text| text.chars().count())
        .unwrap_or(0);
    let skip_apply = payload.skip_apply.unwrap_or(false);
    let result = autocomplete::global_engine().accept(payload).await?;
    let accept_log = format!(
        "[autocomplete] accept accepted={} applied={} explicit_chars={} value_chars={} skip_apply={} reason={}",
        if result.accepted { "yes" } else { "no" },
        if result.applied { "yes" } else { "no" },
        explicit_chars,
        result
            .value
            .as_deref()
            .map(|text| text.chars().count())
            .unwrap_or(0),
        if skip_apply { "yes" } else { "no" },
        result.reason.as_deref().unwrap_or("none")
    );
    Ok(RpcOutcome::new(
        result,
        vec!["autocomplete suggestion accepted".to_string(), accept_log],
    ))
}

pub async fn autocomplete_set_style(
    payload: AutocompleteSetStyleParams,
) -> Result<RpcOutcome<AutocompleteSetStyleResult>, String> {
    let requested_enabled = payload.enabled;
    let requested_debounce = payload.debounce_ms;
    let requested_max_chars = payload.max_chars;
    let requested_accept_with_tab = payload.accept_with_tab;
    let result = autocomplete::global_engine().set_style(payload).await?;
    let set_style_log = format!(
        "[autocomplete] set_style requested_enabled={} requested_debounce_ms={} requested_max_chars={} requested_accept_with_tab={} effective_enabled={} effective_debounce_ms={} effective_max_chars={} effective_accept_with_tab={} disabled_apps={}",
        requested_enabled
            .map(|v| v.to_string())
            .unwrap_or_else(|| "unchanged".to_string()),
        requested_debounce
            .map(|v| v.to_string())
            .unwrap_or_else(|| "unchanged".to_string()),
        requested_max_chars
            .map(|v| v.to_string())
            .unwrap_or_else(|| "unchanged".to_string()),
        requested_accept_with_tab
            .map(|v| v.to_string())
            .unwrap_or_else(|| "unchanged".to_string()),
        result.config.enabled,
        result.config.debounce_ms,
        result.config.max_chars,
        result.config.accept_with_tab,
        result.config.disabled_apps.len()
    );
    let mut logs = vec![
        "autocomplete style settings updated".to_string(),
        set_style_log,
    ];
    if requested_enabled == Some(true) {
        match autocomplete::global_engine()
            .start(AutocompleteStartParams {
                debounce_ms: Some(result.config.debounce_ms),
            })
            .await
        {
            Ok(start_result) => {
                let status = autocomplete::global_engine().status().await;
                logs.push(format!(
                    "[autocomplete] auto_start enabled=true started={} running={} phase={} debounce={}ms",
                    if start_result.started { "yes" } else { "no" },
                    if status.running { "yes" } else { "no" },
                    status.phase,
                    status.debounce_ms
                ));
            }
            Err(err) => {
                logs.push(format!(
                    "[autocomplete] auto_start enabled=true failed={err}"
                ));
            }
        }
    }
    Ok(RpcOutcome::new(result, logs))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutocompleteHistoryParams {
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutocompleteHistoryResult {
    pub entries: Vec<AcceptedCompletion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutocompleteClearHistoryResult {
    pub cleared: usize,
}

pub async fn autocomplete_history(
    payload: AutocompleteHistoryParams,
) -> Result<RpcOutcome<AutocompleteHistoryResult>, String> {
    let requested_limit = payload.limit.unwrap_or(20);
    let entries = crate::openhuman::autocomplete::history::list_history(requested_limit).await?;
    let entry_count = entries.len();
    Ok(RpcOutcome::new(
        AutocompleteHistoryResult { entries },
        vec![
            "autocomplete history listed".to_string(),
            format!(
                "[autocomplete] history requested_limit={} entries={}",
                requested_limit, entry_count
            ),
        ],
    ))
}

pub async fn autocomplete_clear_history(
) -> Result<RpcOutcome<AutocompleteClearHistoryResult>, String> {
    let cleared = crate::openhuman::autocomplete::history::clear_history().await?;
    Ok(RpcOutcome::new(
        AutocompleteClearHistoryResult { cleared },
        vec![
            "autocomplete history cleared".to_string(),
            format!("[autocomplete] clear_history cleared_entries={cleared}"),
        ],
    ))
}

pub async fn autocomplete_start_cli(
    options: AutocompleteStartCliOptions,
) -> Result<serde_json::Value, String> {
    if options.spawn {
        let exe = std::env::current_exe()
            .map_err(|e| format!("failed to resolve current executable: {e}"))?;
        let mut child_cmd = std::process::Command::new(exe);
        child_cmd.arg("autocomplete").arg("start").arg("--serve");
        if let Some(debounce_ms) = options.debounce_ms {
            child_cmd.arg("--debounce-ms").arg(debounce_ms.to_string());
        }
        let child = child_cmd
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("failed to spawn autocomplete service: {e}"))?;
        return Ok(json!({
            "logs": [
                "autocomplete background service spawned"
            ],
            "result": {
                "spawned": true,
                "pid": child.id(),
            }
        }));
    }

    if options.serve {
        let start = autocomplete_start(AutocompleteStartParams {
            debounce_ms: options.debounce_ms,
        })
        .await?;
        if !start.value.started {
            return Ok(json!({
                "logs": start.logs,
                "result": {
                    "started": false,
                    "running": false,
                }
            }));
        }
        eprintln!(
            "autocomplete service running in foreground (pid={}); press Ctrl-C to stop",
            std::process::id()
        );
        let mut serve_logs: Vec<String> = Vec::new();
        let mut poll = time::interval(Duration::from_millis(300));
        poll.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
        let mut prev_phase = String::new();
        let mut prev_app: Option<String> = None;
        let mut prev_error: Option<String> = None;
        let mut prev_suggestion: Option<String> = None;

        loop {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {
                    break;
                }
                _ = poll.tick() => {
                    let status = autocomplete::global_engine().status().await;
                    if status.phase != prev_phase {
                        let msg = format!("phase={} (debounce={}ms)", status.phase, status.debounce_ms);
                        eprintln!("{msg}");
                        serve_logs.push(msg);
                        prev_phase = status.phase.clone();
                    }
                    if status.app_name != prev_app {
                        if let Some(app_name) = status.app_name.clone() {
                            let msg = format!("app={app_name}");
                            eprintln!("{msg}");
                            serve_logs.push(msg);
                        }
                        prev_app = status.app_name.clone();
                    }
                    let next_suggestion = status.suggestion.as_ref().map(|s| s.value.clone());
                    if next_suggestion != prev_suggestion {
                        if let Some(suggestion) = next_suggestion.clone() {
                            let msg = format!("suggestion=\"{}\"", suggestion);
                            eprintln!("{msg}");
                            serve_logs.push(msg);
                        }
                        prev_suggestion = next_suggestion;
                    }
                    if status.last_error != prev_error {
                        if let Some(error) = status.last_error.clone() {
                            let msg = format!("error={error}");
                            eprintln!("{msg}");
                            serve_logs.push(msg);
                        }
                        prev_error = status.last_error.clone();
                    }
                }
            }
        }

        let stop = autocomplete_stop(Some(AutocompleteStopParams {
            reason: Some("interrupt".to_string()),
        }))
        .await?;
        let mut logs = start.logs;
        logs.extend(serve_logs);
        logs.push("autocomplete service received interrupt signal".to_string());
        logs.extend(stop.logs);
        return Ok(json!({
            "logs": logs,
            "result": {
                "started": true,
                "stopped": stop.value.stopped,
            }
        }));
    }

    let start = autocomplete_start(AutocompleteStartParams {
        debounce_ms: options.debounce_ms,
    })
    .await?;
    Ok(json!({
        "logs": start.logs,
        "result": start.value,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── autocomplete_status ────────────────────────────────────────────────────
    //
    // TODO: These tests share the process-global autocomplete::global_engine()
    // singleton (via autocomplete_status / autocomplete_stop / autocomplete_start).
    // They are currently stable because start() always errors on non-macOS, keeping
    // the engine in an idle state. Once macOS support lands -- or if concurrent tests
    // transition the engine -- races on the global state will cause flakiness.
    //
    // Fix when that happens: serialize engine-touching tests with a
    // process-wide tokio::sync::Mutex guard (or the `serial_test` crate), or
    // refactor to accept an injected engine instance instead of going through
    // global_engine().

    /// Happy path: `autocomplete_status` always succeeds and produces exactly
    /// two log lines with the expected key tokens.
    #[tokio::test]
    async fn status_returns_outcome_with_two_log_lines() {
        let outcome = autocomplete_status()
            .await
            .expect("autocomplete_status must not return Err");

        assert_eq!(
            outcome.logs.len(),
            2,
            "expected exactly 2 log lines, got: {:?}",
            outcome.logs
        );
        assert!(
            outcome.logs[0].contains("autocomplete status fetched"),
            "first log should confirm fetch: {:?}",
            outcome.logs[0]
        );
        assert!(
            outcome.logs[1].contains("[autocomplete] status"),
            "second log should contain the structured prefix: {:?}",
            outcome.logs[1]
        );
    }

    /// The status payload has the expected boolean/string fields and a non-empty phase.
    #[tokio::test]
    async fn status_payload_has_expected_fields() {
        let outcome = autocomplete_status()
            .await
            .expect("autocomplete_status must not return Err");

        let status = &outcome.value;
        // Phase must be a non-empty string (default is "idle").
        assert!(
            !status.phase.is_empty(),
            "phase must not be empty, got: {:?}",
            status.phase
        );
        // debounce_ms is always set to a positive value by the engine default (120 ms).
        assert!(
            status.debounce_ms > 0,
            "debounce_ms must be positive, got {}",
            status.debounce_ms
        );
    }

    // ── autocomplete_stop ──────────────────────────────────────────────────────

    /// Happy path: stopping a not-yet-running engine reports `stopped: true`
    /// and produces two log lines.
    #[tokio::test]
    async fn stop_without_reason_returns_stopped_true_and_two_logs() {
        let outcome = autocomplete_stop(None)
            .await
            .expect("autocomplete_stop must not return Err");

        assert!(
            outcome.value.stopped,
            "stopped must be true even when engine was already idle"
        );
        assert_eq!(
            outcome.logs.len(),
            2,
            "expected 2 log lines, got: {:?}",
            outcome.logs
        );
        assert!(
            outcome.logs[0].contains("autocomplete stopped"),
            "first log should confirm stop: {:?}",
            outcome.logs[0]
        );
    }

    /// When a `reason` is supplied, the structured log line must include it.
    #[tokio::test]
    async fn stop_with_reason_includes_reason_in_log() {
        let payload = Some(AutocompleteStopParams {
            reason: Some("test-shutdown".to_string()),
        });

        let outcome = autocomplete_stop(payload)
            .await
            .expect("autocomplete_stop must not return Err");

        let structured_log = &outcome.logs[1];
        assert!(
            structured_log.contains("test-shutdown"),
            "structured log must contain the supplied reason; got: {:?}",
            structured_log
        );
    }

    /// When no reason is supplied, the structured log line must record "none".
    #[tokio::test]
    async fn stop_without_reason_logs_none_as_reason() {
        let outcome = autocomplete_stop(None)
            .await
            .expect("autocomplete_stop must not return Err");

        let structured_log = &outcome.logs[1];
        assert!(
            structured_log.contains("reason=none"),
            "structured log must record reason=none when no reason is given; got: {:?}",
            structured_log
        );
    }

    // ── autocomplete_start (non-macOS) ─────────────────────────────────────────

    /// On Linux/Windows `autocomplete_start` must return an `Err` because
    /// the engine only supports macOS. This exercises the error path of the
    /// ops wrapper without needing OS accessibility permissions.
    #[cfg(not(target_os = "macos"))]
    #[tokio::test]
    async fn start_returns_err_on_non_macos() {
        let result = autocomplete_start(AutocompleteStartParams { debounce_ms: None }).await;
        assert!(
            result.is_err(),
            "autocomplete_start must fail on non-macOS; got Ok"
        );
        let msg = result.unwrap_err();
        assert!(
            msg.contains("macOS"),
            "error message must mention macOS; got: {msg:?}"
        );
    }

    // ── autocomplete_start_cli (non-spawn, non-serve path, non-macOS) ──────────

    /// The plain `autocomplete_start_cli` path (neither --spawn nor --serve)
    /// propagates the engine's start error on non-macOS platforms.
    #[cfg(not(target_os = "macos"))]
    #[tokio::test]
    async fn start_cli_plain_path_returns_err_on_non_macos() {
        let opts = AutocompleteStartCliOptions {
            debounce_ms: None,
            serve: false,
            spawn: false,
        };
        let result = autocomplete_start_cli(opts).await;
        assert!(
            result.is_err(),
            "start_cli plain path must propagate start failure on non-macOS; got Ok"
        );
    }

    // ── AutocompleteHistoryParams struct ──────────────────────────────────────

    /// `AutocompleteHistoryParams` with an explicit limit round-trips through
    /// JSON correctly — field name and value are preserved.
    #[test]
    fn history_params_serialise_round_trip() {
        let params = AutocompleteHistoryParams { limit: Some(7) };
        let json = serde_json::to_value(&params).expect("serialise ok");
        assert_eq!(json["limit"], 7);

        let back: AutocompleteHistoryParams = serde_json::from_value(json).expect("deserialise ok");
        assert_eq!(back.limit, Some(7));
    }

    /// `AutocompleteHistoryParams` with no limit serialises to JSON `null` for
    /// the `limit` field.
    #[test]
    fn history_params_none_limit_serialises_to_null() {
        let params = AutocompleteHistoryParams { limit: None };
        let json = serde_json::to_value(&params).expect("serialise ok");
        assert!(json["limit"].is_null());
    }

    // ── AutocompleteClearHistoryResult struct ─────────────────────────────────

    /// `AutocompleteClearHistoryResult` round-trips through JSON and the
    /// `cleared` field is preserved.
    #[test]
    fn clear_history_result_serialise_round_trip() {
        let result = AutocompleteClearHistoryResult { cleared: 42 };
        let json = serde_json::to_value(&result).expect("serialise ok");
        assert_eq!(json["cleared"], 42);

        let back: AutocompleteClearHistoryResult =
            serde_json::from_value(json).expect("deserialise ok");
        assert_eq!(back.cleared, 42);
    }

    // ── autocomplete_history (integration) ───────────────────────────────────
    //
    // NOTE: These tests operate against the real on-disk KV store via
    // MemoryClient::new_local() (resolves to default_root_openhuman_dir()).
    // They are marked #[ignore] to prevent wiping a contributor's autocomplete
    // history on every `cargo test` run and to avoid non-deterministic results.
    // Run explicitly with: cargo test -- --ignored

    /// `autocomplete_history` against a fresh (possibly empty) local KV store
    /// must succeed and produce exactly two log lines — one confirmation and
    /// one structured log.  The result entries count may be 0 or more.
    #[tokio::test]
    #[ignore = "operates on real on-disk KV store; run with --ignored to opt in"]
    async fn history_returns_outcome_with_two_log_lines() {
        let payload = AutocompleteHistoryParams { limit: Some(5) };
        let outcome = autocomplete_history(payload)
            .await
            .expect("autocomplete_history must not return Err");

        assert_eq!(
            outcome.logs.len(),
            2,
            "expected exactly 2 log lines; got: {:?}",
            outcome.logs
        );
        assert!(
            outcome.logs[0].contains("autocomplete history listed"),
            "first log must confirm listing; got: {:?}",
            outcome.logs[0]
        );
        assert!(
            outcome.logs[1].contains("requested_limit=5"),
            "structured log must record requested_limit; got: {:?}",
            outcome.logs[1]
        );
        // entries must be a valid (possibly empty) vec
        let _ = &outcome.value.entries;
    }

    /// When `limit` is `None`, the default of 20 is applied and appears in the log.
    #[tokio::test]
    #[ignore = "operates on real on-disk KV store; run with --ignored to opt in"]
    async fn history_default_limit_appears_in_log() {
        let payload = AutocompleteHistoryParams { limit: None };
        let outcome = autocomplete_history(payload)
            .await
            .expect("autocomplete_history must not return Err");

        assert!(
            outcome.logs[1].contains("requested_limit=20"),
            "default limit of 20 must appear in log; got: {:?}",
            outcome.logs[1]
        );
    }

    // ── autocomplete_clear_history (integration) ──────────────────────────────

    /// `autocomplete_clear_history` on an already-empty or populated store must
    /// succeed, return a non-negative cleared count, and emit exactly two log lines.
    #[tokio::test]
    #[ignore = "operates on real on-disk KV store; run with --ignored to opt in"]
    async fn clear_history_returns_outcome_with_two_log_lines() {
        let outcome = autocomplete_clear_history()
            .await
            .expect("autocomplete_clear_history must not return Err");

        assert_eq!(
            outcome.logs.len(),
            2,
            "expected exactly 2 log lines; got: {:?}",
            outcome.logs
        );
        assert!(
            outcome.logs[0].contains("autocomplete history cleared"),
            "first log must confirm clear; got: {:?}",
            outcome.logs[0]
        );
        assert!(
            outcome.logs[1].contains("cleared_entries="),
            "structured log must contain cleared_entries; got: {:?}",
            outcome.logs[1]
        );
        // cleared is a usize — always non-negative by type
        let _ = outcome.value.cleared;
    }
}
