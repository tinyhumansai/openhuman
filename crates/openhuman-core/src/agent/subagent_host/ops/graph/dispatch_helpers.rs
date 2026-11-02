use super::*;

pub(super) fn spawn_subagent_trace_export(
    config: Option<&crate::config::Config>,
    dispatch: &crate::core::runtime::DispatchCtx,
    journal_run_id: Option<&str>,
    thread_id: Option<&str>,
    agent_id: &str,
    task_id: &str,
) {
    let (Some(config), Some(journal_run_id)) = (
        config.filter(|config| config.observability.share_usage_data),
        journal_run_id,
    ) else {
        return;
    };
    let config = config.clone();
    let dispatch = dispatch.clone();
    let task_dispatch = dispatch.clone();
    let journal_run_id = journal_run_id.to_owned();
    let thread_id = thread_id.map(str::to_owned);
    let agent_id = agent_id.to_owned();
    let task_id = task_id.to_owned();
    dispatch.spawn(async move {
        crate::core::observability::progress_tracing::export_subagent_journal_trace(
            &task_dispatch,
            &config,
            &journal_run_id,
            thread_id.as_deref(),
            &agent_id,
            &task_id,
        )
        .await;
    });
}

/// A task-specific thread explicitly routes a worker; an absent task thread
/// leaves the parent run's transcript affinity intact.
pub(crate) fn inherited_thread_id(
    parent_thread_id: Option<String>,
    task_thread_id: Option<String>,
) -> Option<String> {
    task_thread_id.or(parent_thread_id)
}

/// Build the sub-agent turn's [`TurnContextMiddleware`] from the live
/// `[context]` config and the agent's TokenJuice profile (#4466), mirroring the
/// chat path (`session/turn/core.rs`). Falls back to
/// [`TurnContextMiddleware::defaults`] when the config can't be loaded so a
/// config glitch degrades to the safe (byte-cap-only) behavior rather than
/// erroring the run.
pub(super) fn build_subagent_context_mw(
    tokenjuice_compression: AgentTokenjuiceCompression,
    config: Option<&crate::config::Config>,
) -> crate::agent::tinyagents::middleware::turn_context::TurnContextMiddleware {
    let mut mw =
        crate::agent::tinyagents::middleware::turn_context::TurnContextMiddleware::defaults();
    // Always thread the agent's compression profile — even on the config-default
    // path — so the definition's TokenJuice choice is honored.
    mw.tokenjuice_compression = tokenjuice_compression;
    match config {
        Some(config) => {
            mw.runtime_config = Some(Arc::new(config.clone()));
            let ctx = &config.context;
            // TokenJuice content-aware compaction gates on the same master
            // `[context].compaction_enabled` the chat path reads
            // (`ContextConfig::compaction_enabled`).
            mw.tokenjuice_compaction_enabled =
                ctx.compaction_enabled && crate::inference::tokenjuice::enabled(config);
            mw.tool_result_budget_bytes = ctx.tool_result_budget_bytes;
            // Microcompact keep-recent is `0` (disabled) unless microcompact is on.
            mw.microcompact_keep_recent = if ctx.microcompact_enabled {
                ctx.microcompact_keep_recent
            } else {
                0
            };
            // Summarization step honors the `[context].enabled` + autocompact
            // opt-outs, same as `ContextConfig::enabled && autocompact_enabled`.
            mw.autocompact_enabled = ctx.enabled && ctx.autocompact_enabled;
            mw.compaction = ctx.compaction_settings();
            tracing::debug!(
                tokenjuice_compaction_enabled = mw.tokenjuice_compaction_enabled,
                compression = ?mw.tokenjuice_compression,
                tool_result_budget_bytes = mw.tool_result_budget_bytes,
                microcompact_keep_recent = mw.microcompact_keep_recent,
                autocompact_enabled = mw.autocompact_enabled,
                "[subagent_host:graph] built sub-agent context middleware from config (#4466)"
            );
        }
        None => {
            tracing::debug!(
                "[subagent_host:graph] no config available building sub-agent context mw; using defaults + compression profile"
            );
        }
    }
    mw
}

pub(super) fn map_tinyagents_subagent_error(err: anyhow::Error) -> SubagentRunError {
    match err.downcast::<SubagentRunError>() {
        Ok(run_err) => run_err,
        Err(err) => SubagentRunError::Provider(err),
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
pub(super) fn cap_tool_results<'a>(
    conversation: &'a [TranscriptEntry],
    tool_outcomes: &[tinyagents_harness::turn_outcome::ToolCallOutcome],
) -> Vec<tinyagents_harness::summarization::CapToolResult<'a>> {
    use std::collections::HashMap;
    use tinyagents_harness::summarization::CapToolResult;

    // call_id -> tool name, from this turn's assistant tool-call rounds.
    let mut names: HashMap<&str, &str> = HashMap::new();
    for msg in conversation {
        if let TranscriptEntry::AssistantToolCalls { tool_calls, .. } = msg {
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

    let results: Vec<_> = conversation
        .iter()
        .filter_map(|entry| match entry {
            TranscriptEntry::ToolResults(results) => Some(results),
            _ => None,
        })
        .flatten()
        .map(|result| CapToolResult {
            call_id: &result.tool_call_id,
            name: names
                .get(result.tool_call_id.as_str())
                .copied()
                .unwrap_or("tool"),
            output: &result.content,
            // Missing outcome means the tool was never run successfully.
            success: success_by_id
                .get(result.tool_call_id.as_str())
                .copied()
                .unwrap_or(false),
        })
        .collect();
    results
}
