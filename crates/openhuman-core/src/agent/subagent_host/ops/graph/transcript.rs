//! Persisting a sub-agent turn's (or a failed run's) raw transcript to
//! `session_raw`, mirroring the removed `SubagentObserver::persist_transcript`.

use crate::agent::subagent_host::types::SubagentRunError;
use tinyagents_session::transcript::TranscriptMessage;

use super::dispatch::AggregatedUsage;
use super::worker_mirror::mirror_worker_thread_from_history;

/// The history a failed sub-agent run persists, plus the unanswered steps as
/// text (#6281).
///
/// Starts from the caller's original `history`, not the snapshot's seeded
/// prefix: that prefix is the provider-bound `dispatch_history`, which carries
/// rehydrated image data where the durable history keeps its placeholders. Only
/// the rounds a provider accepted follow it as structured messages. An empty
/// snapshot (the run failed before its first model call) recovers nothing, as
/// before.
pub(super) fn failed_run_history(
    original: &[TranscriptMessage],
    snapshot: &crate::agent::tinyagents::TranscriptSnapshot,
) -> (Vec<TranscriptMessage>, Option<String>) {
    if snapshot.messages.is_empty() {
        return (Vec::new(), None);
    }
    let base = snapshot.request_base_len.min(snapshot.messages.len());
    let end = snapshot.accepted_end();
    let mut recovered = original.to_vec();
    recovered.extend(
        snapshot.messages[base..end]
            .iter()
            .filter_map(tinyagents_runtime::transcript::message_to_native_transcript),
    );
    (
        recovered,
        crate::agent::tinyagents::render_unanswered_steps(&snapshot.messages[end..]),
    )
}

/// Persist a sub-agent turn's raw transcript to `session_raw`, mirroring the
/// removed `SubagentObserver::persist_transcript`: `agent_type:"subagent"`, the
/// `task_id`, and the provider/model + usage carried on the last assistant
/// message so per-thread usage reads price the sub-agent at its own model.
#[allow(clippy::too_many_arguments)]
pub(super) fn persist_subagent_transcript(
    workspace_dir: &std::path::Path,
    transcript_stem: &str,
    agent_id: &str,
    task_id: &str,
    provider_label: &str,
    model: &str,
    history: &[TranscriptMessage],
    usage: &AggregatedUsage,
    thread_id: Option<&str>,
    context_window: u64,
    dispatcher: &str,
    iteration: u32,
) {
    use tinyagents_session::transcript;

    let root = crate::agent::session_store::transcript_root(workspace_dir);
    let path = match transcript::resolve_keyed_transcript_path(&root, transcript_stem) {
        Ok(p) => p,
        Err(err) => {
            tracing::debug!(
                agent_id,
                error = %err,
                "[subagent_host:graph] failed to resolve child transcript path"
            );
            return;
        }
    };
    let now = chrono::Utc::now().to_rfc3339();
    let turn_usage = transcript::TurnUsage {
        provider: provider_label.to_string(),
        model: model.to_string(),
        usage: transcript::MessageUsage {
            input: usage.input_tokens,
            output: usage.output_tokens,
            cached_input: usage.cached_input_tokens,
            context_window,
            cost_usd: usage.cost.known_usd,
            cost_source: Some(match usage.cost.source {
                crate::agent::cost::CostSource::Charged => transcript::UsageCostSource::Charged,
                crate::agent::cost::CostSource::Estimated => transcript::UsageCostSource::Estimated,
                crate::agent::cost::CostSource::Unknown => transcript::UsageCostSource::Unknown,
            }),
            // No per-call figure reaches here; readers fall back to the mean
            // request size (`spend::context_tokens_of`).
            ..Default::default()
        },
        ts: now.clone(),
        reasoning_content: None,
        tool_calls: Vec::new(),
        iteration,
    };
    let meta = transcript::TranscriptMeta {
        session_id: None,
        parent_session_id: None,
        agent_name: agent_id.to_string(),
        agent_id: Some(agent_id.to_string()),
        agent_type: Some("subagent".to_string()),
        dispatcher: dispatcher.into(),
        provider: Some(turn_usage.provider.clone()),
        model: Some(turn_usage.model.clone()),
        created: now.clone(),
        updated: now,
        turn_count: 1,
        prefix_message_count: None,
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        cached_input_tokens: usage.cached_input_tokens,
        charged_amount_usd: usage.cost.known_usd,
        thread_id: thread_id.map(str::to_owned),
        task_id: Some(task_id.to_string()),
    };
    if let Err(err) = transcript::write_transcript(&path, history, &meta, Some(&turn_usage)) {
        tracing::debug!(
            agent_id,
            error = %err,
            "[subagent_host:graph] failed to write child transcript"
        );
    }
}

/// Persist a **failed** sub-agent run (#4466): write whatever rounds the live
/// transcript-snapshot middleware captured before the harness error to
/// `session_raw` (so `learning/transcript_ingest` can still ingest a failed run,
/// not skip an absent file), mirror those rounds onto the worker thread, and
/// append a trailing failure marker so the record is self-describing. `recovered`
/// holds only what a provider accepted; steps only the failing request carried
/// arrive as `unanswered_steps` text and are appended to the marker (#6281).
/// `usage` is what the answered calls reported, and `completed_rounds` is the
/// number of model calls a provider answered.
#[allow(clippy::too_many_arguments)]
pub(super) fn persist_failed_run(
    workspace_dir: &std::path::Path,
    transcript_stem: &str,
    agent_id: &str,
    task_id: &str,
    provider_label: &str,
    model: &str,
    recovered: &[TranscriptMessage],
    usage: &AggregatedUsage,
    thread_id: Option<&str>,
    unanswered_steps: Option<&str>,
    completed_rounds: u32,
    context_window: u64,
    dispatcher: &str,
    worker_thread_id: Option<&str>,
    error: &SubagentRunError,
) {
    let marker = match unanswered_steps {
        Some(steps) => format!("[subagent run failed before completion: {error}]\n\n{steps}"),
        None => format!("[subagent run failed before completion: {error}]"),
    };
    let mut history = recovered.to_vec();
    history.push(TranscriptMessage::assistant(marker.clone()));

    persist_subagent_transcript(
        workspace_dir,
        transcript_stem,
        agent_id,
        task_id,
        provider_label,
        model,
        &history,
        usage,
        thread_id,
        context_window,
        dispatcher,
        completed_rounds,
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
