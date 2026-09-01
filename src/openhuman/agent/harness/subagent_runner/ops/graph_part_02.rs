/// Persist a **failed** sub-agent run (#4466): write whatever rounds the live
/// transcript-snapshot middleware captured before the harness error to
/// `session_raw` (so `learning/transcript_ingest` can still ingest a failed run,
/// not skip an absent file), mirror those rounds onto the worker thread, and
/// append a trailing failure marker so the record is self-describing. Usage is
/// zeroed — the harness reported no totals on the error path — and the iteration
/// count is the number of completed rounds recovered.
#[allow(clippy::too_many_arguments)]
fn persist_failed_run(
    workspace_dir: &std::path::Path,
    transcript_stem: &str,
    agent_id: &str,
    task_id: &str,
    provider_label: &str,
    model: &str,
    recovered: &[ChatMessage],
    context_window: u64,
    dispatcher: &str,
    worker_thread_id: Option<&str>,
    error: &SubagentRunError,
) {
    let marker = format!("[subagent run failed before completion: {error}]");
    let mut history = recovered.to_vec();
    history.push(ChatMessage::assistant(marker.clone()));

    // A failed run has no usage totals; record zeros so the transcript is still a
    // valid, ingestable `session_raw` record with the failure surfaced.
    let usage = AggregatedUsage::default();
    persist_subagent_transcript(
        workspace_dir,
        transcript_stem,
        agent_id,
        task_id,
        provider_label,
        model,
        &history,
        &usage,
        context_window,
        dispatcher,
        recovered.len() as u32,
    );

    if let Some(thread_id) = worker_thread_id {
        mirror_worker_thread_from_history(
            workspace_dir,
            thread_id,
            agent_id,
            task_id,
            recovered,
            Some(marker.as_str()),
        );
    }
}

/// Append a worker-thread [`StoredMessage`](crate::openhuman::memory::conversations::ConversationMessage)
/// with the restored legacy [`SubagentObserver`] metadata (#4466): `scope`,
/// `agent_id`, `task_id`, plus the per-message `iteration`, `final`, `mode`, and
/// (for assistant tool rounds / tool results) `tool_calls` / `tool_call_id` /
/// `tool_name`. The migrated path had reduced this to `{scope, agent_id,
/// task_id}` only, dropping the fields worker-thread consumers key on.
#[allow(clippy::too_many_arguments)]
fn append_worker_message(
    workspace_dir: &std::path::Path,
    thread_id: &str,
    agent_id: &str,
    task_id: &str,
    content: String,
    sender: &str,
    metadata: serde_json::Value,
) {
    use crate::openhuman::memory::conversations::{
        append_message, ConversationMessage as StoredMessage,
    };
    let mut extra = serde_json::json!({
        "scope": "worker_thread",
        "agent_id": agent_id,
        "task_id": task_id,
        "mode": "typed",
    });
    if let (Some(base), Some(extra_fields)) = (extra.as_object_mut(), metadata.as_object()) {
        for (k, v) in extra_fields {
            base.insert(k.clone(), v.clone());
        }
    }
    let message = StoredMessage {
        id: format!("{sender}:{}", uuid::Uuid::new_v4()),
        content,
        message_type: "text".to_string(),
        extra_metadata: extra,
        sender: sender.to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    if let Err(err) = append_message(workspace_dir.to_path_buf(), thread_id, message) {
        tracing::debug!(
            agent_id,
            thread_id,
            error = %err,
            "[subagent_runner:graph] failed to append worker-thread message"
        );
    }
}

/// Mirror a sub-agent turn's structured conversation to its worker thread,
/// matching the legacy [`SubagentObserver`]: assistant turns (intents + final)
/// become `agent` messages, tool results become `user` messages. `extra_final`,
/// when set, is appended as a trailing `agent` message (the cap checkpoint or
/// clarifying question, which isn't a plain assistant turn in the transcript).
///
/// Each message carries the restored legacy metadata (#4466): a per-round
/// `iteration` counter, `final` on the trailing message, `tool_calls` on an
/// assistant round, and `tool_call_id` / `tool_name` on each tool result.
fn mirror_worker_thread(
    workspace_dir: &std::path::Path,
    thread_id: &str,
    agent_id: &str,
    task_id: &str,
    conversation: &[ConversationMessage],
    extra_final: Option<&str>,
) {
    use std::collections::HashMap;

    // call_id -> tool name, so each tool result records the tool it came from.
    let mut names: HashMap<&str, &str> = HashMap::new();
    for msg in conversation {
        if let ConversationMessage::AssistantToolCalls { tool_calls, .. } = msg {
            for call in tool_calls {
                names.insert(call.id.as_str(), call.name.as_str());
            }
        }
    }

    let mut iteration: u64 = 0;
    for msg in conversation {
        match msg {
            ConversationMessage::AssistantToolCalls {
                text, tool_calls, ..
            } => {
                iteration += 1;
                if let Some(t) = text.as_deref().filter(|t| !t.trim().is_empty()) {
                    let call_names: Vec<&str> =
                        tool_calls.iter().map(|c| c.name.as_str()).collect();
                    append_worker_message(
                        workspace_dir,
                        thread_id,
                        agent_id,
                        task_id,
                        t.to_string(),
                        "agent",
                        serde_json::json!({
                            "iteration": iteration,
                            "final": false,
                            "tool_calls": call_names,
                        }),
                    );
                }
            }
            ConversationMessage::ToolResults(results) => {
                for r in results {
                    let tool_name = names
                        .get(r.tool_call_id.as_str())
                        .copied()
                        .unwrap_or("tool");
                    append_worker_message(
                        workspace_dir,
                        thread_id,
                        agent_id,
                        task_id,
                        r.content.clone(),
                        "user",
                        serde_json::json!({
                            "iteration": iteration,
                            "final": false,
                            "tool_call_id": r.tool_call_id,
                            "tool_name": tool_name,
                        }),
                    );
                }
            }
            ConversationMessage::Chat(c)
                if c.role == "assistant" && !c.content.trim().is_empty() =>
            {
                iteration += 1;
                append_worker_message(
                    workspace_dir,
                    thread_id,
                    agent_id,
                    task_id,
                    c.content.clone(),
                    "agent",
                    serde_json::json!({
                        "iteration": iteration,
                        "final": extra_final.is_none(),
                    }),
                );
            }
            _ => {}
        }
    }

    if let Some(text) = extra_final.filter(|t| !t.trim().is_empty()) {
        append_worker_message(
            workspace_dir,
            thread_id,
            agent_id,
            task_id,
            text.to_string(),
            "agent",
            serde_json::json!({ "iteration": iteration + 1, "final": true }),
        );
    }
}

/// Worker-thread mirror from a flat [`ChatMessage`] history (the error-recovery
/// path, #4466): assistant messages become `agent` rows, tool messages become
/// `user` rows. Used when only the recovered snapshot (not the typed
/// `conversation`) is available. `failure_final`, when set, is appended as a
/// trailing `agent` failure marker.
fn mirror_worker_thread_from_history(
    workspace_dir: &std::path::Path,
    thread_id: &str,
    agent_id: &str,
    task_id: &str,
    history: &[ChatMessage],
    failure_final: Option<&str>,
) {
    let mut iteration: u64 = 0;
    for m in history {
        match m.role.as_str() {
            "assistant" if !m.content.trim().is_empty() => {
                iteration += 1;
                append_worker_message(
                    workspace_dir,
                    thread_id,
                    agent_id,
                    task_id,
                    m.content.clone(),
                    "agent",
                    serde_json::json!({ "iteration": iteration, "final": false }),
                );
            }
            "tool" if !m.content.trim().is_empty() => {
                append_worker_message(
                    workspace_dir,
                    thread_id,
                    agent_id,
                    task_id,
                    m.content.clone(),
                    "user",
                    serde_json::json!({ "iteration": iteration, "final": false }),
                );
            }
            _ => {}
        }
    }
    if let Some(text) = failure_final.filter(|t| !t.trim().is_empty()) {
        append_worker_message(
            workspace_dir,
            thread_id,
            agent_id,
            task_id,
            text.to_string(),
            "agent",
            serde_json::json!({ "iteration": iteration + 1, "final": true }),
        );
    }
}

/// Build the `tool → outcome` digest the cap-hit summary call summarizes, in the
/// legacy `- {name} [{ok|failed}]: {output}` format (engine `run_tool_digest`),
/// pairing each tool result back to its call by id. Per-tool success is derived
/// from the turn's captured [`ToolCallOutcome`]s (#4467, item 7) rather than
/// reported optimistically as `ok`: a result whose call has no captured outcome
/// — e.g. a hallucinated/unknown tool the crate recovered without running
/// `after_tool` — is marked `failed`, so the summary no longer tells the model
/// every call succeeded.
fn build_cap_digest(
    conversation: &[ConversationMessage],
    tool_outcomes: &[crate::openhuman::agent::tinyagents::ToolCallOutcome],
) -> String {
    use std::collections::HashMap;
    use std::fmt::Write as _;

    // call_id -> tool name, from this turn's assistant tool-call rounds.
    let mut names: HashMap<&str, &str> = HashMap::new();
    for msg in conversation {
        if let ConversationMessage::AssistantToolCalls { tool_calls, .. } = msg {
            for call in tool_calls {
                names.insert(call.id.as_str(), call.name.as_str());
            }
        }
    }

    // call_id -> success, from the captured per-call outcomes.
    let success_by_id: HashMap<&str, bool> = tool_outcomes
        .iter()
        .map(|o| (o.call_id.as_str(), o.success))
        .collect();

    let mut out = String::new();
    for msg in conversation {
        if let ConversationMessage::ToolResults(results) = msg {
            for r in results {
                let name = names
                    .get(r.tool_call_id.as_str())
                    .copied()
                    .unwrap_or("tool");
                // Missing outcome → `false` (unknown/hallucinated tool): honest
                // failed status rather than an optimistic `[ok]`.
                let ok = success_by_id
                    .get(r.tool_call_id.as_str())
                    .copied()
                    .unwrap_or(false);
                let tag = if ok { "ok" } else { "failed" };
                let body = crate::openhuman::util::truncate_with_ellipsis(&r.content, 800);
                let _ = writeln!(out, "- {name} [{tag}]: {body}");
            }
        }
    }
    out.trim_end().to_string()
}
