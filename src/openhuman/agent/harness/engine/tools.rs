//! Shared per-call tool executor.
//!
//! [`run_one_tool`] runs the full lifecycle of a single parsed tool call:
//!
//! 1. emit `ToolCallStarted` (for *every* call, including ones rejected below,
//!    so a client row created from streamed args always gets a terminal event);
//! 2. evaluate the pluggable [`ToolPolicy`] (deny short-circuits everything,
//!    including approval side-effects);
//! 3. guard `CliRpcOnly` scope (such tools can't run in the autonomous loop);
//! 4. route external-effect tools through the process-global `ApprovalGate`;
//! 5. execute with the configured timeout, then scrub credentials, apply
//!    tokenjuice, the per-tool size cap, and the optional payload summarizer;
//! 6. stamp the approval audit "after" row (#2135);
//! 7. emit `ToolCallCompleted`.
//!
//! It returns a [`ToolRunResult`] (`text` + `success`). The caller owns history
//! shaping (native `role:tool` messages vs XML `<tool_result>` blocks) and the
//! repeated-failure circuit breaker, both of which it drives uniformly from the
//! returned `success`/`text` regardless of which branch produced them.
//!
//! This body was lifted verbatim (behavior-preserving) from the canonical
//! `run_tool_call_loop` in `tool_loop.rs`; the three loops now call it instead
//! of each carrying their own copy.

use super::super::payload_summarizer::PayloadSummarizer;
use super::progress::ProgressReporter;
use crate::openhuman::agent::harness::parse::ParsedToolCall;
use crate::openhuman::tools::policy::{PolicyDecision, ToolPolicy};
use crate::openhuman::tools::traits::ToolScope;
use crate::openhuman::tools::Tool;

use super::super::credentials::scrub_credentials;

/// Outcome of a single tool call. `text` is what should be fed back to the
/// model (a result body, an error, or a denial reason); `success` is `false`
/// for any non-OK outcome (policy/approval denial, scope rejection, timeout,
/// tool error, unknown tool) so the caller's circuit breaker and history
/// formatting can treat every failure mode uniformly.
pub(crate) struct ToolRunResult {
    pub text: String,
    pub success: bool,
}

/// Execute one parsed tool call end-to-end. See the module docs for the full
/// lifecycle. `tool_opt` is the (already visibility-filtered) tool the caller
/// resolved by name — `None` means the model requested an unknown/filtered-out
/// tool, which is reported as a structured error the LLM can correct next turn.
///
/// `progress_call_id` is the stable id threaded through the start/complete
/// event pair (and any preceding args-delta events) so consumers can reconcile
/// tool rows by id.
pub(crate) async fn run_one_tool(
    tool_opt: Option<&dyn Tool>,
    call: &ParsedToolCall,
    iteration: usize,
    progress: &dyn ProgressReporter,
    tool_policy: &dyn ToolPolicy,
    payload_summarizer: Option<&dyn PayloadSummarizer>,
    progress_call_id: &str,
) -> ToolRunResult {
    let iteration_u32 = (iteration + 1) as u32;

    // Emit a "tool started" event for every parsed call, even ones that will be
    // rejected below (approval denied, CliRpcOnly, unknown) — the client-side
    // row was created from the streamed args and needs a terminal event.
    progress
        .tool_started(progress_call_id, &call.name, &call.arguments, iteration_u32)
        .await;

    // Helper: emit a failed "tool completed" event for an early-exit path
    // (denied / CliRpcOnly / unknown) so the client row flips to `error`
    // instead of staying running.
    let emit_failed_completion = |message: &str| {
        let output_chars = message.chars().count();
        async move {
            progress
                .tool_completed(
                    progress_call_id,
                    &call.name,
                    false,
                    output_chars,
                    0,
                    iteration_u32,
                )
                .await;
        }
    };

    // ── Tool policy check (#2131) ─────────────────
    // Evaluate the pluggable ToolPolicy before any approval or execution. If
    // the policy denies the call, skip everything (including approval
    // side-effects) and return the denial reason as a tool error to the model.
    if let PolicyDecision::Deny(reason) = tool_policy.evaluate(&call.name, &call.arguments) {
        tracing::debug!(
            iteration,
            tool = call.name.as_str(),
            reason = %reason,
            "[agent_loop] tool policy denied tool call"
        );
        let denied = format!("Tool '{}' denied by policy: {reason}", call.name);
        emit_failed_completion(&denied).await;
        return ToolRunResult {
            text: denied,
            success: false,
        };
    }

    let Some(tool) = tool_opt else {
        tracing::warn!(
            iteration,
            tool = call.name.as_str(),
            "[agent_loop] unknown tool requested"
        );
        let msg = format!("Unknown tool: {}", call.name);
        emit_failed_completion(&msg).await;
        return ToolRunResult {
            text: msg,
            success: false,
        };
    };

    tracing::debug!(
        iteration,
        tool = call.name.as_str(),
        found = true,
        "[agent_loop] executing tool"
    );

    // Scope check: CliRpcOnly tools cannot run in the autonomous agent loop.
    if tool.scope() == ToolScope::CliRpcOnly {
        tracing::warn!(
            iteration,
            tool = call.name.as_str(),
            "[agent_loop] tool scope is CliRpcOnly — denied in agent loop"
        );
        let denied = format!(
            "Tool '{}' is only available via explicit CLI/RPC invocation, not in the autonomous agent loop.",
            call.name
        );
        emit_failed_completion(&denied).await;
        return ToolRunResult {
            text: denied,
            success: false,
        };
    }

    // ── External-effect approval gate (#1339, #2135) ──
    // Tools whose `external_effect()` returns true route through the
    // process-global `ApprovalGate` so the UI can prompt the user before
    // `execute()` runs. The gate is `None` when supervised mode is disabled or
    // in test envs — behavior matches the pre-#1339 path.
    //
    // `approval_request_id` carries the persisted row id forward so we can
    // stamp the terminal execution outcome onto the same `pending_approvals`
    // row after the tool finishes (issue #2135). `None` means the tool was
    // either not gated, was session-allowlist-shortcutted, or was denied —
    // none of which produce an audit row that needs an "after" entry.
    let mut approval_request_id: Option<String> = None;
    let mut approval_gate_for_audit: Option<
        std::sync::Arc<crate::openhuman::approval::ApprovalGate>,
    > = None;
    if tool.external_effect_with_args(&call.arguments) {
        if let Some(gate) = crate::openhuman::approval::ApprovalGate::try_global() {
            let summary = crate::openhuman::approval::summarize_action(&call.name, &call.arguments);
            let redacted = crate::openhuman::approval::redact_args(&call.arguments);
            let (outcome, request_id) =
                gate.intercept_audited(&call.name, &summary, redacted).await;
            match outcome {
                crate::openhuman::approval::GateOutcome::Allow => {
                    approval_request_id = request_id;
                    if approval_request_id.is_some() {
                        approval_gate_for_audit = Some(gate);
                    }
                }
                crate::openhuman::approval::GateOutcome::Deny { reason } => {
                    tracing::warn!(
                        iteration,
                        tool = call.name.as_str(),
                        reason = %reason,
                        "[agent_loop] approval gate denied tool call"
                    );
                    emit_failed_completion(&reason).await;
                    return ToolRunResult {
                        text: reason,
                        success: false,
                    };
                }
            }
        }
    }

    let tool_deadline = crate::openhuman::tool_timeout::tool_execution_timeout_duration();
    let timeout_secs = crate::openhuman::tool_timeout::tool_execution_timeout_secs();
    let tool_started = std::time::Instant::now();
    let outcome = tokio::time::timeout(tool_deadline, tool.execute(call.arguments.clone())).await;
    let elapsed_ms = tool_started.elapsed().as_millis() as u64;
    let (result_text, success) = match outcome {
        Ok(Ok(r)) => {
            let output = r.output();
            let success = !r.is_error;
            if success {
                tracing::debug!(
                    iteration,
                    tool = call.name.as_str(),
                    output_len = output.len(),
                    "[agent_loop] tool succeeded"
                );
                let mut scrubbed = scrub_credentials(&output);
                let (compacted, tj_stats) = crate::openhuman::tokenjuice::compact_tool_output(
                    &call.name,
                    Some(&call.arguments),
                    &scrubbed,
                    Some(0),
                );
                if tj_stats.applied {
                    log::debug!(
                        "[agent_loop] tokenjuice applied tool={} rule={} {}->{} bytes",
                        call.name,
                        tj_stats.rule_id,
                        tj_stats.original_bytes,
                        tj_stats.compacted_bytes
                    );
                    scrubbed = compacted;
                }

                // Per-tool max_result_size_chars cap. When a tool sets it and
                // the (post-tokenjuice) body still exceeds the cap, truncate
                // here and skip the global payload summarizer for this call —
                // the cap is fast and deterministic, the summarizer is the
                // fallback for tools that don't know their own size budget.
                let mut hit_per_tool_cap = false;
                if let Some(cap) = tool.max_result_size_chars() {
                    let char_count = scrubbed.chars().count();
                    if char_count > cap {
                        let truncated: String = scrubbed.chars().take(cap).collect();
                        let dropped = char_count - cap;
                        log::info!(
                            "[agent_loop] per-tool cap applied tool={} cap_chars={} original_chars={} dropped_chars={}",
                            call.name,
                            cap,
                            char_count,
                            dropped,
                        );
                        scrubbed = format!(
                            "{truncated}\n\n[truncated by tool cap: {dropped} more chars not shown]"
                        );
                        hit_per_tool_cap = true;
                    }
                }

                if !hit_per_tool_cap {
                    if let Some(summarizer) = payload_summarizer {
                        log::debug!(
                            "[agent_loop] payload_summarizer intercepting tool={} bytes={}",
                            call.name,
                            scrubbed.len()
                        );
                        match summarizer
                            .maybe_summarize(&call.name, None, &scrubbed)
                            .await
                        {
                            Ok(Some(payload)) => {
                                log::info!(
                                    "[agent_loop] payload_summarizer compressed tool={} {}->{} bytes",
                                    call.name,
                                    payload.original_bytes,
                                    payload.summary_bytes
                                );
                                scrubbed = payload.summary;
                            }
                            Ok(None) => {
                                log::debug!(
                                    "[agent_loop] payload_summarizer pass-through tool={} bytes={}",
                                    call.name,
                                    scrubbed.len()
                                );
                            }
                            Err(e) => {
                                log::warn!(
                                    "[agent_loop] payload_summarizer error tool={} err={} (passing raw payload through)",
                                    call.name,
                                    e
                                );
                            }
                        }
                    }
                }
                (scrubbed, true)
            } else {
                // Scrub before logging — a failing tool payload can carry
                // credentials / PII, so never log the raw output.
                let scrubbed = scrub_credentials(&output);
                tracing::warn!(
                    iteration,
                    tool = call.name.as_str(),
                    "[agent_loop] tool returned error: {scrubbed}"
                );
                let (compacted, _) = crate::openhuman::tokenjuice::compact_tool_output(
                    &call.name,
                    Some(&call.arguments),
                    &scrubbed,
                    Some(1),
                );
                (format!("Error: {compacted}"), false)
            }
        }
        Ok(Err(e)) => {
            // Route through `report_error_or_expected` (not the unconditional
            // `report_error`) so an error a downstream layer already classified
            // as expected user-state isn't re-reported as a hard Sentry event
            // here. The integrations client (`integrations/client.rs`) already
            // demotes its backend 4xx/auth-state failures via
            // `report_error_or_expected`, but it ALSO bubbles the error up; it
            // lands in this `Ok(Err(_))` arm and was being re-reported as a
            // hard `tool`/`execute` event — the double-report behind Sentry
            // TAURI-RUST-84E (`Backend returned 401 Unauthorized for POST
            // .../agent-integrations/parallel/search: Invalid token`, a
            // user-end invalid/expired session token with no openhuman-side
            // lever). Genuine tool failures don't match any classifier arm and
            // still surface as hard errors — only already-classified
            // user-state errors are demoted to a warn/info breadcrumb.
            crate::core::observability::report_error_or_expected(
                format!("{e:#}").as_str(),
                "tool",
                "execute",
                &[
                    ("tool", call.name.as_str()),
                    ("outcome", "failed"),
                    ("iteration", &(iteration + 1).to_string()),
                ],
            );
            (format!("Error executing {}: {e}", call.name), false)
        }
        Err(_) => {
            let msg = format!(
                "tool '{}' timed out after {} seconds",
                call.name, timeout_secs
            );
            crate::core::observability::report_error(
                msg.as_str(),
                "tool",
                "execute",
                &[
                    ("tool", call.name.as_str()),
                    ("outcome", "timeout"),
                    ("timeout_secs", &timeout_secs.to_string()),
                    ("iteration", &(iteration + 1).to_string()),
                ],
            );
            (
                format!(
                    "Error: tool '{}' timed out after {} seconds",
                    call.name, timeout_secs
                ),
                false,
            )
        }
    };
    progress
        .tool_completed(
            progress_call_id,
            &call.name,
            success,
            result_text.chars().count(),
            elapsed_ms,
            iteration_u32,
        )
        .await;
    // ── Approval audit after-action row (#2135) ────
    // Stamp the terminal status onto the same `pending_approvals` row the gate
    // created before execution, so the audit trail carries both the before
    // (approval) and after (executed_at + outcome). Best-effort: a write
    // failure here is logged but not propagated to the agent.
    if let (Some(gate), Some(req_id)) = (
        approval_gate_for_audit.as_ref(),
        approval_request_id.as_ref(),
    ) {
        let exec_outcome = if success {
            crate::openhuman::approval::ExecutionOutcome::Success
        } else {
            crate::openhuman::approval::ExecutionOutcome::Failure
        };
        let err_text = if success {
            None
        } else {
            Some(result_text.as_str())
        };
        gate.record_execution(req_id, exec_outcome, err_text);
    }

    ToolRunResult {
        text: result_text,
        success,
    }
}
