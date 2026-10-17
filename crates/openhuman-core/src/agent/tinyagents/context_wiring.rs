//! Host wiring for the context ladder. The ladder's order and sizing rules live
//! in `tinyagents_harness::middleware::ContextLadder`; this file supplies only
//! what the host owns: the summarizer behind compression (task-state or free
//! form, with memory recall), the wrap-up prompts and deliverable tools, and
//! the outcome sink the wrap-up restores from.

use std::sync::Arc;

use tinyagents_harness::middleware::{
    compression_policy, legacy_max_input_tokens, microcompact_for_window, split_input_allowance,
    ArtifactIndexTocMiddleware, CapturedOutcomes, ContextCompressionMiddleware, ContextLadder,
    FinalCallWrapUpMiddleware, ImageAwareMessageTrimMiddleware, OutcomesUnavailable,
};
use tinyagents_harness::runtime::AgentHarness;
use tinyagents_harness::summarization::{
    FaultTolerantCachingSummarizer, ModelSummarizer, SummarizationPolicy, TaskStateSummarizer,
    DEFAULT_TASK_STATE_CHUNK_TOKENS,
};

use crate::agent::tinyagents::host::OpenHumanRunContext;
use crate::agent::tinyagents::memory_summarizer::MemoryRecallSummarizer;
use crate::agent::tinyagents::model::TurnChatModel;
use tinyagents_harness::turn_outcome::ToolOutcomeSink;

/// Fractions of a capped turn's model-call budget at which the model is told
/// how many calls are left.
const BUDGET_NOTICE_THRESHOLDS: [f64; 2] = [0.5, 0.8];
/// Longest verbatim tail of a task-state compaction (tokens).
const TASK_STATE_KEEP_RECENT_TOKENS: u64 = 20_000;
/// Store the artifact index is registered under on the run context.
const ARTIFACT_INDEX_STORE: &str =
    crate::agent::harness::tool_result_artifacts::TINYAGENTS_TOOL_RESULT_ARTIFACT_STORE;

/// Read side of the tool-outcome capture sink, for the wrap-up's restoration.
pub(super) struct OutcomeSinkSource(pub(super) ToolOutcomeSink);

impl CapturedOutcomes for OutcomeSinkSource {
    fn content_for(&self, call_id: &str) -> Result<Option<String>, OutcomesUnavailable> {
        let outcomes = self.0.lock().map_err(|_| OutcomesUnavailable)?;
        Ok(outcomes
            .iter()
            .find(|o| o.call_id == call_id)
            .map(|o| o.content.clone()))
    }
}

/// Install the ladder; returns the compression and wrap-up handles the run loop
/// reads after the drive future returns. `wrap_up_at_cap` is true only for a
/// top-level turn that pauses at its model-call cap.
#[allow(clippy::too_many_arguments)]
pub(super) fn install_context_ladder(
    harness: &mut AgentHarness<(), OpenHumanRunContext>,
    model: &str,
    context_window: Option<u64>,
    autocompact_enabled: bool,
    compaction_trigger_tokens: Option<u64>,
    compaction_strategy: crate::config::CompactionStrategy,
    microcompact_keep_recent: usize,
    summarizer_model: TurnChatModel,
    wrap_up_at_cap: bool,
    tool_outcome_sink: &ToolOutcomeSink,
    memory_turn: Option<Arc<crate::memory::lifecycle::hooks::MemoryTurn>>,
) -> (
    Option<Arc<ContextCompressionMiddleware>>,
    Option<Arc<FinalCallWrapUpMiddleware>>,
) {
    let window = context_window.filter(|w| *w > 0);
    let compression = autocompact_enabled
        .then(|| compression_policy(context_window, compaction_trigger_tokens))
        .flatten()
        .map(|policy| {
            Arc::new(compression_middleware(
                policy,
                compaction_strategy,
                context_window,
                summarizer_model,
                model,
                memory_turn,
            ))
        });
    // One allowance split between the contents list and wrap-up restoration.
    let trim_allowance = window
        .map(|w| legacy_max_input_tokens(w).max(1))
        .unwrap_or(0);
    let (toc_allowance, restore_allowance) = split_input_allowance(trim_allowance);
    let wrap_up = wrap_up_at_cap.then(|| {
        Arc::new(
            FinalCallWrapUpMiddleware::new(
                crate::agent::session_host::turn_checkpoint::MAX_ITER_CHECKPOINT_INSTRUCTION,
                crate::agent::session_host::turn_checkpoint::FINAL_WRITE_INSTRUCTION,
                Arc::new(OutcomeSinkSource(tool_outcome_sink.clone())),
                restore_allowance,
            )
            .with_deliverable_tools(["file_write", "apply_patch"])
            .with_cleared_placeholder(crate::agent::context::CLEARED_PLACEHOLDER)
            .with_budget_notice(BUDGET_NOTICE_THRESHOLDS),
        )
    });
    ContextLadder {
        compression,
        microcompact: (microcompact_keep_recent > 0).then(|| {
            microcompact_for_window(
                microcompact_keep_recent,
                crate::agent::context::CLEARED_PLACEHOLDER,
                context_window,
            )
        }),
        wrap_up,
        artifact_toc: Some(ArtifactIndexTocMiddleware::new(
            toc_allowance,
            ARTIFACT_INDEX_STORE,
        )),
        trim: window.map(ImageAwareMessageTrimMiddleware::for_context_window),
    }
    .install(harness)
}

/// The compression middleware for `strategy`: `TaskState` writes a typed
/// checkpoint over a token-budgeted tail, `Summary` is the free-form summarizer.
fn compression_middleware(
    policy: SummarizationPolicy,
    strategy: crate::config::CompactionStrategy,
    context_window: Option<u64>,
    summarizer_model: TurnChatModel,
    model: &str,
    memory_turn: Option<Arc<crate::memory::lifecycle::hooks::MemoryTurn>>,
) -> ContextCompressionMiddleware {
    let window = context_window.filter(|w| *w > 0);
    match strategy {
        crate::config::CompactionStrategy::TaskState => {
            let keep_recent = window.map_or(TASK_STATE_KEEP_RECENT_TOKENS, |w| {
                (w / 5).min(TASK_STATE_KEEP_RECENT_TOKENS)
            });
            let chunk = window.map_or(DEFAULT_TASK_STATE_CHUNK_TOKENS, |w| {
                (w * 2 / 5).clamp(4_000, DEFAULT_TASK_STATE_CHUNK_TOKENS)
            });
            tracing::info!(model, keep_recent_tokens = keep_recent, max_chunk_tokens = chunk, context_window = ?window, "[context_compression] strategy=task_state");
            let summarizer = FaultTolerantCachingSummarizer::new(
                Box::new(
                    TaskStateSummarizer::new(summarizer_model, model).with_max_chunk_tokens(chunk),
                ),
                &policy,
            );
            ContextCompressionMiddleware::with_summarizer(
                policy,
                MemoryRecallSummarizer::wrap(Box::new(summarizer), memory_turn),
            )
            .with_keep_recent_tokens(keep_recent)
        }
        crate::config::CompactionStrategy::Summary => {
            tracing::info!(model, "[context_compression] strategy=summary");
            let summarizer = FaultTolerantCachingSummarizer::new(
                Box::new(ModelSummarizer::new(summarizer_model, model)),
                &policy,
            );
            ContextCompressionMiddleware::with_summarizer(
                policy,
                MemoryRecallSummarizer::wrap(Box::new(summarizer), memory_turn),
            )
        }
    }
}

#[cfg(test)]
#[path = "context_wiring_tests.rs"]
mod tests;
