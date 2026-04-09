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
