//! Persisting a sub-agent turn's (or a failed run's) raw transcript to the
//! turn tree's transcript store, mirroring the removed
//! `SubagentObserver::persist_transcript`.

use std::sync::Arc;

use crate::agent::subagent_host::types::SubagentRunError;
use crate::agent::tinyagents::host::OpenHumanRunContext;
use tinyagents_session::transcript::{TranscriptLocator, TranscriptMessage, TranscriptTurn};

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
    snapshot: &crate::agent::tinyagents::middleware::turn_context::TranscriptSnapshot,
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
        crate::agent::tinyagents::middleware::turn_context::render_unanswered_steps(
            &snapshot.messages[end..],
        ),
    )
}

/// The store a sub-agent's transcript goes to: the turn tree's own
/// ([`OpenHumanRunContext::transcripts`], set by the root session), so it lands
/// beside its parent's under the same key. A run no session started falls
/// back to the store its own carried context selects, keyed like the parent
/// session's (its definition id, or the default agent). That context's store
/// is used explicitly: a detached run is a bare `tokio::spawn` with no ambient
/// one.
///
/// A SaaS process whose carried context is gone has no tenant to scope to, so
/// the run gets an isolated in-memory store instead of the operator's.
pub(super) fn subagent_transcripts(
    run_context: &OpenHumanRunContext,
    workspace_dir: &std::path::Path,
) -> Arc<dyn TranscriptLocator> {
    subagent_transcripts_in(run_context, workspace_dir, crate::core::runtime::is_saas())
}

pub(super) fn subagent_transcripts_in(
    run_context: &OpenHumanRunContext,
    workspace_dir: &std::path::Path,
    saas: bool,
) -> Arc<dyn TranscriptLocator> {
    if let Some(transcripts) = run_context.transcripts.clone() {
        return transcripts;
    }
    let core = run_context.core.as_ref().and_then(std::sync::Weak::upgrade);
    let fallback = run_context
        .parent
        .as_ref()
        .map_or(crate::agent::session_store::DEFAULT_AGENT, |parent| {
            parent.agent_definition_id.as_str()
        });
    let Some(core) = core else {
        if saas {
            tracing::warn!(
                "[subagent_host:graph] no turn-tree store and no carried context in SaaS mode; \
                 transcript kept in memory only"
            );
            return tinyagents_session::port::SessionStoreProvider::for_agent(
                &tinyagents_session::port::InMemorySessionStores::new(),
                fallback,
            )
            .transcripts;
        }
        let key = crate::agent::session_store::agent_key_for(
            &crate::core::runtime::Tenant::default(),
            fallback,
        );
        return crate::agent::session_store::transcripts_or_files(&key, workspace_dir);
    };
    let tenant = crate::core::runtime::Tenant::of(&core);
    let key = crate::agent::session_store::agent_key_for(&tenant, fallback);
    tracing::debug!(
        profile = tenant.profile.is_some(),
        "[subagent_host:graph] no turn-tree transcript store; using the carried context's"
    );
    crate::agent::session_store::transcripts_or_files_in(&core, &key, workspace_dir)
}

/// Persist a sub-agent turn's raw transcript to `transcripts` under
/// `transcript_stem`, mirroring the removed `SubagentObserver::persist_transcript`: `agent_type:"subagent"`, the
/// `task_id`, and the provider/model + usage carried on the last assistant
/// message so per-thread usage reads price the sub-agent at its own model.
#[allow(clippy::too_many_arguments)]
pub(super) fn persist_subagent_transcript(
    transcripts: &dyn TranscriptLocator,
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
                tinyagents_harness::cost::CostSource::Charged => {
                    transcript::UsageCostSource::Charged
                }
                tinyagents_harness::cost::CostSource::Estimated => {
                    transcript::UsageCostSource::Estimated
                }
                tinyagents_harness::cost::CostSource::Unknown => {
                    transcript::UsageCostSource::Unknown
                }
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
    let handle = match transcripts.open_stem(transcript_stem, meta.clone()) {
        Ok(handle) => handle,
        Err(err) => {
            tracing::debug!(
                agent_id,
                error = %err,
                "[subagent_host:graph] failed to open child transcript"
            );
            return;
        }
    };
    // What the stem already holds (a resumed sub-agent session) is the
    // baseline: an extension appends, anything else is recorded as a
    // compaction, so nothing written before is erased.
    let prev = match handle.messages() {
        Ok(prev) => prev,
        Err(err) => {
            tracing::debug!(
                agent_id,
                error = %err,
                "[subagent_host:graph] failed to read child transcript"
            );
            return;
        }
    };
    if let Err(err) = handle.append_turn(TranscriptTurn {
        prev: &prev,
        next: history,
        meta: &meta,
        turn_usage: Some(&turn_usage),
        request_id: None,
        tools: None,
    }) {
        tracing::debug!(
            agent_id,
            error = %err,
            "[subagent_host:graph] failed to write child transcript"
        );
    }
}

/// Persist a **failed** sub-agent run (#4466): write whatever rounds the live
/// transcript-snapshot middleware captured before the harness error to the
/// turn tree's transcript store (so `learning/transcript_ingest` can still ingest a failed run,
/// not skip an absent file), mirror those rounds onto the worker thread, and
/// append a trailing failure marker so the record is self-describing. `recovered`
/// holds only what a provider accepted; steps only the failing request carried
/// arrive as `unanswered_steps` text and are appended to the marker (#6281).
/// `usage` is what the answered calls reported, and `completed_rounds` is the
/// number of model calls a provider answered.
#[allow(clippy::too_many_arguments)]
pub(super) fn persist_failed_run(
    ctx: &crate::core::runtime::DispatchCtx,
    transcripts: &dyn TranscriptLocator,
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
        transcripts,
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
            ctx,
            workspace_dir,
            thread_id,
            agent_id,
            task_id,
            recovered,
            Some(marker.as_str()),
        );
    }
}
