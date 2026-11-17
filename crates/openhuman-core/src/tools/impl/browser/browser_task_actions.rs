//! Argument parsing and host approval for browser tool actions.

use super::Pending;
use crate::security::approval::{ApprovalGate, GateOutcome};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use tinycomputer_bus::agent::{TaskStatus, TaskView};
use tinycomputer_bus::browser::tool::ToolArgs;
use tinycomputer_bus::browser::Action;

/// What the host adds to a task report the agent cannot act on alone: a
/// task that failed because Chrome was not found, or would not start from
/// the path the user set, says where the user sets that path, rather than
/// leaving the agent to install a browser.
pub(super) fn host_hint(view: &TaskView) -> Option<&'static str> {
    match &view.status {
        TaskStatus::Failed { reason, hint, .. } => crate::modules::browser::chrome_hint(reason)
            .or_else(|| crate::modules::browser::chrome_hint(hint)),
        _ => None,
    }
}

/// A task report as the tool's outcome: a task that failed, or was cancelled
/// under the caller, is an error carrying its reason, hint and full report,
/// so the turn, the failure breaker and the tool status all see the task did
/// not finish. Every other state (running, paused for a person, input or
/// approval, a checkpoint, done) is a success the model acts on.
pub(super) fn task_outcome(view: &TaskView, output: Value) -> anyhow::Result<Value> {
    let marker = tinytools::status::TASK_FAILED_MARKER;
    let headline = match &view.status {
        TaskStatus::Failed {
            step, reason, hint, ..
        } => {
            let at = step.map_or_else(
                || " before any step ran".to_owned(),
                |step| format!(" at step {step}"),
            );
            let mut line = format!("{marker} Browser task failed{at}: {}", reason.trim());
            if !hint.trim().is_empty() {
                line.push_str(&format!(" Hint: {}", hint.trim()));
            }
            if let Some(host) = output["host_hint"].as_str() {
                line.push_str(&format!(" {host}"));
            }
            line
        }
        TaskStatus::Cancelled => format!(
            "{marker} Browser task was cancelled before it finished: {}",
            view.summary.trim()
        ),
        _ => return Ok(output),
    };
    tracing::debug!(
        task_id = %view.id,
        "[tool.browser] task ended without finishing; reporting a tool error"
    );
    let report = serde_json::to_string_pretty(&output).unwrap_or_default();
    Err(anyhow::anyhow!("{headline}\n\n{report}"))
}

pub(super) fn task_inputs(args: &Value) -> anyhow::Result<BTreeMap<String, String>> {
    args["inputs"].as_object().map_or_else(
        || Ok(BTreeMap::new()),
        |inputs| {
            inputs
                .iter()
                .map(|(k, v)| {
                    Ok((
                        k.clone(),
                        v.as_str()
                            .ok_or_else(|| anyhow::anyhow!("Task input '{k}' must be text"))?
                            .to_owned(),
                    ))
                })
                .collect()
        },
    )
}

/// Ask the host approval gate about a paused task's exact action. A missing
/// gate denies: a task never takes an irreversible step unapproved, unless a
/// trusted unattended turn has `task_step` in `[browser] unattended_actions`.
pub(super) async fn approve_task_action(
    pending: &Pending,
    browser: &crate::config::BrowserConfig,
    origin: Option<&crate::core::runtime::turn_origin::AgentTurnOrigin>,
) -> anyhow::Result<bool> {
    let clean = |raw: &str| {
        let cleaned = raw.chars().filter(|c| !c.is_control()).collect::<String>();
        let mut short = cleaned.chars().take(160).collect::<String>();
        if cleaned.chars().count() > 160 {
            short.push('…');
        }
        short
    };
    let digest = Sha256::digest(serde_json::to_vec(&json!({
        "task": pending.task, "action": pending.action, "target": pending.target
    }))?);
    let digest_hex = format!("{digest:x}");
    if super::unattended::allow(origin, browser, "task_step", &digest_hex) {
        return Ok(true);
    }
    let gate = ApprovalGate::try_global().ok_or_else(|| {
        anyhow::anyhow!("[policy-denied] Browser action needs an interactive host approval gate")
    })?;
    let summary = format!(
        "Browser task: {} — {} [action {}]",
        clean(&pending.action),
        clean(&pending.target),
        &digest_hex[..12]
    );
    let args = json!({"action": "task_step", "target": summary, "exact_action_sha256": digest_hex});
    Ok(
        match gate.intercept_forced("browser", &summary, args).await {
            GateOutcome::Allow => true,
            GateOutcome::Deny { reason } => {
                tracing::debug!(%reason, "[browser] task action denied by host");
                false
            }
        },
    )
}

pub(super) fn required<'a>(args: &'a Value, key: &str) -> anyhow::Result<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("Missing '{key}' parameter"))
}

/// Decodes the model's arguments into the contract's [`ToolArgs`] and maps
/// them to the module's page [`Action`]; the schema and mapping live in
/// `tinycomputer_bus::browser::tool`.
pub(super) fn direct_action(args: &Value) -> anyhow::Result<Action> {
    let parsed: ToolArgs = serde_json::from_value(args.clone())
        .map_err(|e| anyhow::anyhow!("Invalid browser arguments: {e}"))?;
    Ok(parsed.to_action()?)
}
