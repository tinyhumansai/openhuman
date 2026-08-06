//! Host adapter for [`tinyagents::harness::host::LearningSink`] — the seam the
//! generic agent runtime uses to hand a finished turn to OpenHuman's
//! self-learning subsystem.
//!
//! This is `docs/specs/plan-agents.md` Phase 4. OpenHuman already has a
//! post-turn learning fan-out: [`crate::openhuman::agent::hooks::PostTurnHook`]
//! implementations (`UserProfileHook`, `ToolTrackerHook`, `ReflectionHook`,
//! `ToolMemoryCaptureHook`, `AgentExperienceCaptureHook`, `ArchivistHook`, …)
//! dispatched by [`crate::openhuman::agent::hooks::fire_hooks`]. That function
//! is already exactly the shape the crate's trait doc asks for — it
//! `tokio::spawn`s every hook and returns immediately, logging failures rather
//! than propagating them. So this adapter is deliberately thin: translate
//! [`TurnSummary`] into a [`TurnContext`] and enqueue.
//!
//! # Contract mismatches, and how each is resolved
//!
//! **1. `tools_invoked` is names-only; `TurnContext.tool_calls` is outcomes.**
//! This is the load-bearing mismatch. [`TurnSummary::tools_invoked`] carries a
//! deduplicated list of tool *names* by design — the crate doc is explicit that
//! arguments and results must never travel this path because the record is
//! built to be persisted. OpenHuman's
//! [`crate::openhuman::agent::hooks::ToolCallRecord`] is the opposite: it exists
//! to carry `success`, `duration_ms`, and a sanitized `output_summary`, and
//! every outcome-mining hook (`ToolTrackerHook`, `AgentExperienceCaptureHook`)
//! reads precisely those fields.
//!
//! There is no honest projection from one to the other. Synthesizing records
//! with `success: true, duration_ms: 0` would make `ToolTrackerHook` write
//! fabricated success rates and a corrupted running average into the
//! `tool_effectiveness` namespace, and would make
//! `AgentExperienceCaptureHook` mine "successful multi-tool experience"
//! candidates from a turn whose tools may all have failed. Persisting an
//! invented outcome is worse than persisting none, so **`tool_calls` is left
//! empty** and the names are emitted to the log only. The outcome-driven hooks
//! then self-disable (both early-return on an empty `tool_calls`), which is the
//! correct degradation: silent no-op, not silent lies.
//!
//! The hooks that read the *text* of the turn — `UserProfileHook`'s preference
//! extraction and `ReflectionHook`'s heuristic cue fast-path, which runs before
//! and independently of the `min_turn_complexity` gate — are unaffected and
//! keep working from a names-only summary.
//!
//! **2. `thread_id` vs `session_id`.** `TurnContext.session_id` is populated
//! host-side from the harness' `event_session_id`; a [`TurnSummary`] only
//! carries a [`tinyagents::harness::ids::ThreadId`]. The thread id is the
//! closest available correlation key, so it is mapped through. The visible
//! consequence is that `ReflectionHook`'s `max_reflections_per_session` throttle
//! becomes per-thread rather than per-session on this path — a slightly
//! different, but not incorrect, bucketing.
//!
//! **3. Missing fields.** [`TurnSummary`] has no wall-clock duration and no
//! model-call count, so `turn_duration_ms` is `0` and `iteration_count` is `1`.
//! Neither is read by any gate; they are reporting fields only. `entrypoint` is
//! `None` because the crate summary has no notion of a channel.
//!
//! **4. Errors are advisory.** The trait doc is emphatic that the turn is
//! already committed and an `Err` must not roll it back. `fire_hooks` is
//! infallible and non-blocking, so [`OpenHumanLearningSink::on_turn_complete`]
//! always returns `Ok(())`; hook failures surface in the host log where they
//! belong.
//!
//! # Domains deliberately not wired here
//!
//! - [`crate::openhuman::agent::learning::transcript_ingest`] is *transcript-file*
//!   driven (`ingest_transcript_path` / `ingest_session_transcript` take a
//!   session `.jsonl` on disk) and runs on session close, not per turn. A sink
//!   invocation has no transcript to point at.
//! - [`crate::openhuman::subconscious`] is scheduler-driven: its public entry
//!   point is `SubconsciousInstance::tick`, polled by the heartbeat engine.
//!   There is no per-turn ingestion API to call.
//!
//! Both are reachable through the same `Memory` those hooks write to, so
//! nothing is lost by leaving them on their own cadence.

use std::sync::Arc;

use async_trait::async_trait;
use tinyagents::error::Result;
use tinyagents::harness::host::{LearningSink, TurnSummary};

use crate::openhuman::agent::hooks::{self, PostTurnHook, TurnContext};
use crate::openhuman::agent::learning::{ToolTrackerHook, UserProfileHook};
use crate::openhuman::config::LearningConfig;
use crate::openhuman::memory::Memory;

/// Adapts OpenHuman's [`PostTurnHook`] fan-out to the crate's
/// [`LearningSink`] capability.
///
/// Holds the hook list rather than building it, because *which* hooks are
/// installed is a composition decision the session builder already makes (see
/// `agent/harness/session/builder/factory.rs`); duplicating that policy here
/// would give the generic runtime a second, silently divergent hook set.
/// [`OpenHumanLearningSink::from_learning_config`] is a convenience for the
/// hooks that need nothing but config and memory.
pub struct OpenHumanLearningSink {
    /// Hooks fired, in parallel, for every completed turn.
    hooks: Vec<Arc<dyn PostTurnHook>>,
}

impl OpenHumanLearningSink {
    /// Wraps an already-composed hook list.
    ///
    /// An empty list is legal and turns the sink into a no-op. It is *not* the
    /// way to express "this host has no learning pipeline" — the crate doc asks
    /// hosts to pass `None` for the capability in that case, so absence stays
    /// distinguishable from a sink that ran and did nothing.
    pub fn new(hooks: Vec<Arc<dyn PostTurnHook>>) -> Self {
        Self { hooks }
    }

    /// Builds a sink over the two hooks that need only `[learning]` config and
    /// a [`Memory`] handle: [`UserProfileHook`] and [`ToolTrackerHook`].
    ///
    /// Both hooks re-check `LearningConfig::enabled` plus their own sub-flag
    /// inside `on_turn_complete`, so they are always installed and gate
    /// themselves — that keeps a config toggle live without rebuilding the sink.
    ///
    /// `ReflectionHook` is *not* included: its constructor also needs an
    /// `Arc<Config>` and an optional `ChatModel` provider, which are session
    /// composition concerns. Add it with [`OpenHumanLearningSink::with_hook`].
    pub fn from_learning_config(config: LearningConfig, memory: Arc<dyn Memory>) -> Self {
        Self::new(vec![
            Arc::new(UserProfileHook::new(config.clone(), Arc::clone(&memory))),
            Arc::new(ToolTrackerHook::new(config, memory)),
        ])
    }

    /// Appends one more hook, for hooks whose construction needs more than
    /// config + memory (`ReflectionHook`, `ArchivistHook`, …).
    pub fn with_hook(mut self, hook: Arc<dyn PostTurnHook>) -> Self {
        self.hooks.push(hook);
        self
    }

    /// Number of installed hooks. Exposed for assertions and for a caller
    /// deciding whether to supply the capability at all.
    pub fn hook_count(&self) -> usize {
        self.hooks.len()
    }

    /// Projects a crate [`TurnSummary`] onto OpenHuman's [`TurnContext`].
    ///
    /// See the module doc for why `tool_calls` comes out empty. Kept associated
    /// and pure so the projection is testable without a runtime.
    fn turn_context_from(summary: &TurnSummary) -> TurnContext {
        TurnContext {
            user_message: summary.input.clone(),
            assistant_response: summary.output.clone(),
            // Intentionally empty — a names-only summary cannot supply the
            // `success` / `duration_ms` / `output_summary` fields that give a
            // `ToolCallRecord` its meaning, and fabricating them would poison
            // the `tool_effectiveness` tallies. See the module doc.
            //
            // TODO(phase4): if outcome-driven learning is wanted over the
            // generic runtime, the fix is upstream — the crate would need a
            // richer per-tool record (name + outcome class + duration, still no
            // arguments or results), most likely alongside
            // `tinyagents::harness::host::ToolOutcomeClassifier`, which already
            // owns the "did this tool call succeed" judgement host-side. It is
            // not something this adapter can synthesize.
            tool_calls: Vec::new(),
            // Not carried by `TurnSummary`; reporting-only fields, read by no
            // gate in any installed hook.
            turn_duration_ms: 0,
            iteration_count: 1,
            // `TurnSummary` has no session id; the thread id is the nearest
            // correlation key. Blank ids are normalized to `None` so hooks that
            // key on the session (reflection throttling) fall back to their
            // global bucket rather than keying on "".
            session_id: Some(summary.thread_id.as_str().to_string())
                .filter(|id| !id.trim().is_empty()),
            agent_id: Some(summary.agent_id.clone()).filter(|id| !id.trim().is_empty()),
            // No channel/entrypoint concept exists on the crate side.
            entrypoint: None,
        }
    }
}

#[async_trait]
impl LearningSink for OpenHumanLearningSink {
    /// Enqueues the turn onto OpenHuman's post-turn hook fan-out and returns.
    ///
    /// Always `Ok(())`. `fire_hooks` spawns each hook on the tokio runtime and
    /// logs its failure, so there is nothing fallible left to report — and the
    /// trait doc forbids using an `Err` as a veto in any case, since the turn is
    /// already committed by the time this runs.
    async fn on_turn_complete(&self, summary: &TurnSummary) -> Result<()> {
        if self.hooks.is_empty() {
            return Ok(());
        }

        // Names only. This log line is the sole place the summary's tool list
        // is surfaced, precisely because it must not reach a persistence path
        // as a fabricated outcome.
        log::debug!(
            "[tinyagents][learning] turn complete thread={} agent={} tools=[{}] hooks={}",
            summary.thread_id.as_str(),
            summary.agent_id,
            summary.tools_invoked.join(","),
            self.hooks.len()
        );

        hooks::fire_hooks(&self.hooks, Self::turn_context_from(summary));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tokio::sync::mpsc;

    /// A hook that forwards each `TurnContext` it receives, so tests can await
    /// the spawned fan-out deterministically instead of sleeping.
    struct RecordingHook {
        tx: mpsc::UnboundedSender<TurnContext>,
    }

    #[async_trait]
    impl PostTurnHook for RecordingHook {
        fn name(&self) -> &str {
            "recording"
        }

        async fn on_turn_complete(&self, ctx: &TurnContext) -> anyhow::Result<()> {
            let _ = self.tx.send(ctx.clone());
            Ok(())
        }
    }

    /// A hook that always fails, to pin that a hook error never reaches the
    /// sink's caller.
    struct FailingHook {
        called: Arc<Mutex<bool>>,
    }

    #[async_trait]
    impl PostTurnHook for FailingHook {
        fn name(&self) -> &str {
            "failing"
        }

        async fn on_turn_complete(&self, _ctx: &TurnContext) -> anyhow::Result<()> {
            *self.called.lock().unwrap() = true;
            Err(anyhow::anyhow!("hook blew up"))
        }
    }

    fn sample() -> TurnSummary {
        TurnSummary::new("thread-7", "orchestrator")
            .with_text("I prefer terse answers.", "Understood.")
            .with_tool("shell")
            .with_tool("read_file")
    }

    #[test]
    fn projection_carries_identity_and_text() {
        let ctx = OpenHumanLearningSink::turn_context_from(&sample());
        assert_eq!(ctx.user_message, "I prefer terse answers.");
        assert_eq!(ctx.assistant_response, "Understood.");
        assert_eq!(ctx.session_id.as_deref(), Some("thread-7"));
        assert_eq!(ctx.agent_id.as_deref(), Some("orchestrator"));
        assert!(ctx.entrypoint.is_none());
        assert_eq!(ctx.iteration_count, 1);
        assert_eq!(ctx.turn_duration_ms, 0);
    }

    #[test]
    fn tool_names_are_never_projected_into_tool_call_records() {
        // The regression this whole adapter is shaped around: a names-only
        // summary must not become fabricated `ToolCallRecord` outcomes, or the
        // tool_effectiveness tallies start recording invented successes.
        let summary = sample();
        assert!(summary.used_tools(), "fixture must carry tool names");
        let ctx = OpenHumanLearningSink::turn_context_from(&summary);
        assert!(
            ctx.tool_calls.is_empty(),
            "no outcome data exists to build a ToolCallRecord from"
        );
    }

    #[test]
    fn blank_identifiers_normalize_to_none() {
        // A blank agent id must not become `Some("")`, which would key hook
        // state on an empty string rather than falling back to a global bucket.
        let summary = TurnSummary::new("   ", "  ").with_text("hi", "hello");
        let ctx = OpenHumanLearningSink::turn_context_from(&summary);
        assert!(ctx.session_id.is_none());
        assert!(ctx.agent_id.is_none());
    }

    #[tokio::test]
    async fn on_turn_complete_dispatches_the_projected_context() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let sink = OpenHumanLearningSink::new(vec![Arc::new(RecordingHook { tx })]);
        assert_eq!(sink.hook_count(), 1);

        sink.on_turn_complete(&sample()).await.expect("advisory Ok");

        let ctx = rx.recv().await.expect("hook received the turn");
        assert_eq!(ctx.user_message, "I prefer terse answers.");
        assert_eq!(ctx.session_id.as_deref(), Some("thread-7"));
        assert!(ctx.tool_calls.is_empty());
    }

    #[tokio::test]
    async fn a_failing_hook_does_not_fail_the_sink() {
        // The turn is already committed when this runs, so the crate contract
        // forbids surfacing a hook failure to the caller.
        let called = Arc::new(Mutex::new(false));
        let (tx, mut rx) = mpsc::unbounded_channel();
        let sink = OpenHumanLearningSink::new(vec![
            Arc::new(FailingHook {
                called: Arc::clone(&called),
            }),
            Arc::new(RecordingHook { tx }),
        ]);

        assert!(sink.on_turn_complete(&sample()).await.is_ok());

        // Awaiting the surviving hook proves the fan-out ran past the failure.
        rx.recv().await.expect("the second hook still fires");
        assert!(*called.lock().unwrap(), "the failing hook was invoked");
    }

    #[tokio::test]
    async fn an_empty_hook_list_is_a_successful_no_op() {
        let sink = OpenHumanLearningSink::new(Vec::new());
        assert_eq!(sink.hook_count(), 0);
        assert!(sink.on_turn_complete(&sample()).await.is_ok());
    }

    #[tokio::test]
    async fn with_hook_appends_and_is_usable_as_a_trait_object() {
        // The runtime stores this capability as `Option<Arc<dyn LearningSink>>`;
        // pin that the adapter is object-safe in that position.
        let (tx, mut rx) = mpsc::unbounded_channel();
        let sink = OpenHumanLearningSink::new(Vec::new())
            .with_hook(Arc::new(RecordingHook { tx }) as Arc<dyn PostTurnHook>);
        assert_eq!(sink.hook_count(), 1);

        let sink: Arc<dyn LearningSink> = Arc::new(sink);
        sink.on_turn_complete(&sample()).await.expect("advisory Ok");
        rx.recv().await.expect("appended hook fires");
    }

    /// Inert [`Memory`] so `from_learning_config` can be exercised without a
    /// store. Signatures mirror the trait impl in `learning/tool_tracker.rs`.
    struct InertMemory;

    #[async_trait]
    impl Memory for InertMemory {
        fn name(&self) -> &str {
            "inert"
        }

        async fn store(
            &self,
            _namespace: &str,
            _key: &str,
            _content: &str,
            _category: crate::openhuman::memory::MemoryCategory,
            _session_id: Option<&str>,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn recall(
            &self,
            _query: &str,
            _limit: usize,
            _opts: crate::openhuman::memory::RecallOpts<'_>,
        ) -> anyhow::Result<Vec<crate::openhuman::memory::MemoryEntry>> {
            Ok(Vec::new())
        }

        async fn get(
            &self,
            _namespace: &str,
            _key: &str,
        ) -> anyhow::Result<Option<crate::openhuman::memory::MemoryEntry>> {
            Ok(None)
        }

        async fn list(
            &self,
            _namespace: Option<&str>,
            _category: Option<&crate::openhuman::memory::MemoryCategory>,
            _session_id: Option<&str>,
        ) -> anyhow::Result<Vec<crate::openhuman::memory::MemoryEntry>> {
            Ok(Vec::new())
        }

        async fn forget(&self, _namespace: &str, _key: &str) -> anyhow::Result<bool> {
            Ok(false)
        }

        async fn namespace_summaries(
            &self,
        ) -> anyhow::Result<Vec<crate::openhuman::memory::NamespaceSummary>> {
            Ok(Vec::new())
        }

        async fn count(&self) -> anyhow::Result<usize> {
            Ok(0)
        }

        async fn health_check(&self) -> bool {
            true
        }
    }

    #[tokio::test]
    async fn from_learning_config_installs_the_config_only_hooks() {
        // The point is the composition, not the hooks' own behaviour (which
        // they test themselves against their own mocks).
        let memory: Arc<dyn Memory> = Arc::new(InertMemory);
        let sink = OpenHumanLearningSink::from_learning_config(LearningConfig::default(), memory);
        assert_eq!(sink.hook_count(), 2, "user profile + tool tracker");
        // Learning is off by default, so both hooks self-gate to a no-op — the
        // call must still succeed.
        assert!(sink.on_turn_complete(&sample()).await.is_ok());
    }
}
