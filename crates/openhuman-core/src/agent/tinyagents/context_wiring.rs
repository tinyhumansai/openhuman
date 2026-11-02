//! Host wiring for the context ladder. The ladder's order and sizing rules live
//! in `tinyagents_harness::middleware::ContextLadder`; this file supplies only
//! what the host owns: the summarizer behind compression (task-state or free
//! form, with memory recall), the wrap-up prompts and deliverable tools, and
//! the outcome sink the wrap-up restores from.

use std::sync::Arc;

use async_trait::async_trait;
use tinyagents_harness::middleware::{
    compression_policy, legacy_max_input_tokens, microcompact_for_window, split_input_allowance,
    ArtifactIndexTocMiddleware, CapturedOutcomes, ContextCompressionMiddleware, ContextLadder,
    FinalCallWrapUpMiddleware, ImageAwareMessageTrimMiddleware, OutcomesUnavailable,
};
use tinyagents_harness::runtime::AgentHarness;
use tinyagents_harness::summarization::{
    FaultTolerantCachingSummarizer, ModelSummarizer, SummarizationPolicy, Summarizer,
    SummaryRecord, SummaryRequest, TaskStateSummarizer, DEFAULT_TASK_STATE_CHUNK_TOKENS,
};
use tinyinference_llm::message::Message;

use crate::agent::tinyagents::host::OpenHumanRunContext;
use crate::agent::tinyagents::memory_summarizer::MemoryRecallSummarizer;
use tinyagents_harness::turn_outcome::ToolOutcomeSink;

/// Fractions of a capped turn's model-call budget at which the model is told
/// how many calls are left.
const BUDGET_NOTICE_THRESHOLDS: [f64; 2] = [0.5, 0.8];
/// Longest verbatim tail of a task-state compaction (tokens).
const TASK_STATE_KEEP_RECENT_TOKENS: u64 = 20_000;
/// Store the artifact index is registered under on the run context.
const ARTIFACT_INDEX_STORE: &str =
    crate::agent::harness::tool_result_artifacts::TINYAGENTS_TOOL_RESULT_ARTIFACT_STORE;

/// The model a compaction's summarizer calls, and how it calls it.
pub(crate) struct CompactionSummarizer {
    /// The model the summary request goes to.
    pub(crate) model: Arc<dyn tinyinference_llm::model::ChatModel<()>>,
    /// The id `model` calls, for the summarizer's logs and summary provenance.
    pub(crate) model_id: String,
    /// `[context] summarizer_tool_rendering` / `summarizer_max_output_tokens`;
    /// the free-form (`Summary`) strategy applies them.
    pub(crate) settings: crate::config::SummarizerSettings,
}

/// Called when the turn starts summarizing earlier conversation.
pub(super) type CompactionNotice = Arc<dyn Fn() + Send + Sync>;

/// A summarizer that reports each summarization before it runs, so the client
/// can say what a turn is doing while a long compaction holds its next model
/// call. It sits under the cache, so a cached summary is not reported.
struct AnnouncingSummarizer {
    inner: Box<dyn Summarizer>,
    notice: CompactionNotice,
}

#[async_trait]
impl Summarizer for AnnouncingSummarizer {
    async fn summarize(
        &self,
        messages: &[Message],
    ) -> tinyagents_harness::error::Result<SummaryRecord> {
        (self.notice)();
        self.inner.summarize(messages).await
    }

    async fn summarize_request(
        &self,
        request: &SummaryRequest,
    ) -> tinyagents_harness::error::Result<SummaryRecord> {
        (self.notice)();
        self.inner.summarize_request(request).await
    }

    async fn merge(
        &self,
        summaries: &[SummaryRecord],
    ) -> tinyagents_harness::error::Result<SummaryRecord> {
        self.inner.merge(summaries).await
    }
}

/// `inner`, reporting each summarization to `notice` when there is one.
fn announced(inner: Box<dyn Summarizer>, notice: Option<CompactionNotice>) -> Box<dyn Summarizer> {
    match notice {
        Some(notice) => Box::new(AnnouncingSummarizer { inner, notice }),
        None => inner,
    }
}

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
    summarizer: CompactionSummarizer,
    wrap_up_at_cap: bool,
    tool_outcome_sink: &ToolOutcomeSink,
    memory_turn: Option<Arc<crate::memory::lifecycle::hooks::MemoryTurn>>,
    compaction_notice: Option<CompactionNotice>,
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
                summarizer,
                model,
                memory_turn,
                compaction_notice,
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
                crate::agent::session_host::product_copy::OPENHUMAN_CLOSE_COPY
                    .max_iteration_instruction,
                crate::agent::session_host::product_copy::OPENHUMAN_CLOSE_COPY
                    .final_write_instruction,
                Arc::new(OutcomeSinkSource(tool_outcome_sink.clone())),
                restore_allowance,
            )
            .with_deliverable_tools(["file_write", "apply_patch"])
            .with_cleared_placeholder(
                tinyagents_harness::middleware::library::DEFAULT_CLEARED_PLACEHOLDER,
            )
            .with_budget_notice(BUDGET_NOTICE_THRESHOLDS),
        )
    });
    ContextLadder {
        compression,
        microcompact: (microcompact_keep_recent > 0).then(|| {
            microcompact_for_window(
                microcompact_keep_recent,
                tinyagents_harness::middleware::library::DEFAULT_CLEARED_PLACEHOLDER,
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
/// checkpoint over a token-budgeted tail, `Summary` is the free-form summarizer,
/// rendering tool activity and capping its output as `summarizer.settings`
/// say. `model` is the turn's.
pub(crate) fn compression_middleware(
    policy: SummarizationPolicy,
    strategy: crate::config::CompactionStrategy,
    context_window: Option<u64>,
    summarizer: CompactionSummarizer,
    model: &str,
    memory_turn: Option<Arc<crate::memory::lifecycle::hooks::MemoryTurn>>,
    compaction_notice: Option<CompactionNotice>,
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
                announced(
                    Box::new(
                        TaskStateSummarizer::new(summarizer.model, summarizer.model_id)
                            .with_max_chunk_tokens(chunk),
                    ),
                    compaction_notice,
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
            let settings = summarizer.settings;
            tracing::info!(
                model,
                tool_rendering = ?settings.tool_rendering,
                max_output_tokens = ?settings.max_output_tokens,
                "[context_compression] strategy=summary"
            );
            let mut model_summarizer = ModelSummarizer::new(summarizer.model, summarizer.model_id)
                .with_tool_rendering(settings.tool_rendering);
            if let Some(cap) = settings.max_output_tokens {
                model_summarizer = model_summarizer.with_max_output_tokens(cap);
            }
            let summarizer = FaultTolerantCachingSummarizer::new(
                announced(Box::new(model_summarizer), compaction_notice),
                &policy,
            );
            ContextCompressionMiddleware::with_summarizer(
                policy,
                MemoryRecallSummarizer::wrap(Box::new(summarizer), memory_turn),
            )
        }
    }
}

/// What a compaction outside a turn folds against.
pub(crate) struct IdleFold<'a> {
    /// The caller's threshold: the conversation is at least this large.
    pub(crate) min_tokens: u64,
    /// The verbatim tail in tokens; `None` sizes it as a turn would against
    /// a `min_tokens` trigger. Either way the policy caps it at half of
    /// `min_tokens`, as it caps a turn's tail at half its trigger.
    pub(crate) keep_recent_tokens: Option<u64>,
    /// `[context] compaction_strategy`.
    pub(crate) strategy: crate::config::CompactionStrategy,
    /// The agent model's context window, when known.
    pub(crate) context_window: Option<u64>,
    /// The agent's model, for logs.
    pub(crate) model: &'a str,
}

/// `history` compacted with no model turn, or `None` when nothing was old
/// enough to fold.
///
/// This is a turn's compaction, driven over one request: the same
/// [`compression_middleware`] runs its `before_model` over `history` and its
/// `after_agent` reports the folded transcript, which becomes a
/// [`CompactionCarry`](tinyagents_harness::summarization::CompactionCarry)
/// re-applied to `history` exactly as the session driver applies a turn's.
/// The summary is placed after the system prompt (the default
/// `SummaryPlacement::User`). The caller has already decided the
/// conversation is at least `min_tokens`; the policy's trigger sits just
/// below that so the middleware compacts it.
///
/// # Errors
///
/// When the middleware fails (its failure policy normally recovers instead).
pub(crate) async fn compact_history(
    history: &[Message],
    fold: IdleFold<'_>,
    summarizer: CompactionSummarizer,
) -> tinyagents_harness::error::Result<Option<Vec<Message>>> {
    use tinyagents_harness::context::{RunConfig, RunContext};
    use tinyagents_harness::middleware::{AgentRun, Middleware};

    let trigger = fold.min_tokens.saturating_sub(1).max(1);
    let Some(policy) = compression_policy(fold.context_window, Some(trigger)) else {
        return Ok(None);
    };
    let mut mw = compression_middleware(
        policy,
        fold.strategy,
        fold.context_window,
        summarizer,
        fold.model,
        None,
        None,
    );
    if let Some(tokens) = fold.keep_recent_tokens {
        mw = mw.with_keep_recent_tokens(tokens);
    }
    let mut ctx = RunContext::new(RunConfig::new("compact_thread"), ());
    let mut request = tinyinference_llm::model::ModelRequest::new(history.to_vec());
    Middleware::<(), ()>::before_model(&mw, &mut ctx, &(), &mut request).await?;
    let mut run = AgentRun {
        messages: history.to_vec(),
        ..Default::default()
    };
    Middleware::<(), ()>::after_agent(&mw, &mut ctx, &(), &mut run).await?;
    let carry = run
        .compacted_history
        .as_deref()
        .and_then(tinyagents_harness::summarization::CompactionCarry::from_compacted_history);
    Ok(carry.map(|carry| carry.apply(history.to_vec())))
}

#[cfg(test)]
#[path = "context_wiring_tests.rs"]
mod tests;
