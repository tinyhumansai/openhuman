# tinyagents

The **adapter seam** between OpenHuman and the vendored [`tinyagents`](../../../../../vendor/tinyagents/) crate family (issue #4249). Every agent turn runs on the crate's `AgentHarness` loop; this module bridges OpenHuman's `Provider`, `Tool`, and `ChatMessage` types onto the crate's `ChatModel`, `Tool`, and `Message` traits, assembles the per-turn harness, and enforces the OpenHuman-specific policy (approval, tool scope, budgets, credential scrubbing, compaction) as harness middleware on the way in and out. The chat, channel/CLI, and sub-agent routes all enter through one function, `run_turn_via_tinyagents_shared`, so they cannot drift from each other.

## Responsibilities

- Assemble a per-turn harness (`assemble_turn_harness` in `harness_assembly.rs`): register the turn's `ChatModel`s, every shared tool, and the full middleware stack, then drive it via `AgentHarness::invoke_stream_in_context` (`turn_runner.rs`).
- Convert between OpenHuman and crate types: tiered `ChatModel` bundles from `(role, config)` (`turn_models.rs`, `model.rs`), direct TinyTools/TinyInference tool declarations at their consumers, and `ChatMessage`/`ConversationMessage` ↔ crate `Message` via `crate::agent::message_convert`.
- Enforce cross-cutting policy as harness middleware: approval/security gating, tool policy and CLI/RPC-only denial, cost budgets, context compaction/summarization, credential scrubbing, malformed-argument recovery, and the repeated-tool-failure circuit breaker (`middleware*.rs`).
- Route workloads to model tiers and record the resolved provider/model for audit (`routes.rs`, canonical TinyInference response metadata).
- Make turns durable and replayable: a JSONL event journal plus status store (`journal.rs`), a startup sweep for orphaned runs (`reaper.rs`), and a read-only RPC surface over both (`replay/`).
- Provide graph-layer helpers for multi-stage sub-agent orchestration (`orchestration.rs`, `delegation.rs`) and expose graph structure for debugging (`topology.rs`).
- Host adapters (`host/`) for the crate's ten host-capability traits. Not yet wired into the live turn path; see [Host adapters](#host-adapters) below.

## Key files

| File / group | Role |
| --- | --- |
| `mod.rs` | Module root: declares submodules and re-exports (`TurnContextMiddleware`, `HandoffConfig`, `TranscriptSnapshotSink`, `SubagentScope`, `ToolPolicyEnforcement`, `TinyagentsTurnOutcome`, `TurnModelSource`/`TurnModels`, `run_turn_via_tinyagents_shared`). |
| `turn_policy.rs` | `ToolPolicyEnforcement`, run-policy construction, and wall-clock-ceiling helpers (`agent_turn_wall_clock_ms` and friends). |
| `turn_runner.rs` | `run_turn_via_tinyagents_shared`, the single entry point every production caller uses, plus the `#[cfg(test)]`-only `run_turn_via_tinyagents`. |
| `turn_models.rs` | `TurnModels` (primary + tier routes + summarizer) and `TurnModelSource`, whose `build`/`build_summarizer` construct crate-native models from `(role, config)` via `build_turn_models_crate`. |
| `harness_assembly.rs` | `assemble_turn_harness`, which registers models, tools, and middleware in order. |
| `harness_tool_registration.rs` | Tool + agent capability-registry projection consumed by `assemble_turn_harness`. |
| `turn_outcome.rs` | `TinyagentsTurnOutcome`, `HaltSummarySlot`, `ToolOutcomeSink`, and `record_unobserved_turn_usage`, the cost-tracker fallback for turns with no `on_progress` observer. |
| `turn_run_error.rs` | Maps a failed harness run onto the typed OpenHuman error `run_turn_via_tinyagents_shared` returns. |
| `turn_run_finalize.rs` | Turns a completed harness run into a `TinyagentsTurnOutcome` on the success path (journal completion, compaction diagnostics, the terminal `TurnCompleted` event). |
| `host/` | OpenHuman's implementations of the crate's ten host-capability traits: `agent_memory`, `budget_gate`, `context_composer`, `definition_registry`, `experience_store`, `learning_sink`, `model_resolver`, `progress_sink`, `security_gate`, `tool_outcome_classifier`. Each file adapts one trait onto the OpenHuman domain that implements it. |
| `replay/` | Read-only agent-run replay/status RPC (`mod.rs`, `ops.rs`, `schemas.rs`): three `agent`-namespace controllers over `journal.rs`. |
| `journal.rs` | `TurnJournal` and `FileStatusStore`: a crate `StoreEventJournal` over a JSONL append store plus a `HarnessStatusStore` writer under `{workspace}/tinyagents_store`. Attached alongside the live `observability` bridge as an independent `EventSink` subscriber, wrapped in `RedactingSink`; best-effort and non-fatal. |
| `reaper.rs` | `reap_orphaned_runs`: startup sweep that marks non-terminal runs `Cancelled` in the durable status store. The only writer over the status seam that `replay/` reads. |
| `middleware.rs` + `middleware/` | The named OpenHuman middleware stack. `turn_context.rs`: `TurnContextMiddleware` (bundles config and installs the enabled hooks), `TranscriptSnapshotMiddleware`/`TranscriptSnapshotSink`, `HandoffConfig`/`HandoffMiddleware` (oversized sub-agent results stashed for `extract_from_result`). `prompt_cache.rs`: `PromptCacheSegmentMiddleware`. `tool_output.rs`: `ToolOutputMiddleware` (per-result byte cap and optional payload summarizer). `approval.rs`: `ApprovalSecurityMiddleware`. `cli_rpc_only.rs`: `CliRpcOnlyMiddleware`. `tool_policy.rs`: the `ToolPolicyMiddleware` struct and impl (records blocks into `crate::tools::registry::denials`). `tool_outcome_capture.rs`: `ToolOutcomeCaptureMiddleware`. `embedder_hooks.rs`: `EmbedderToolHooksMiddleware`. `arg_recovery.rs`: `ArgRecoveryMiddleware`. `memory_protocol.rs`: `MemoryProtocolMiddleware`. `cost_budget.rs`: `CostBudgetMiddleware`. `repeated_failure.rs`: `RepeatedToolFailureMiddleware`. `loop_guards.rs`: `TerminalInferenceFailure`. `repeat_progress.rs`: `RepeatProgressMiddleware` (adjacent-repeat streaks plus a run-wide `(call, result)` recurrence ledger) and `RepeatEvictionObserver` (registered last; resets the ledger when compaction evicts a recorded result, #6275). `message_trim.rs`: `ImageAwareMessageTrimMiddleware`. `final_call_wrap_up.rs`: `FinalCallWrapUpMiddleware`. `artifact_index_toc.rs`: `ArtifactIndexTocMiddleware` (issue #6014). `credential_scrub.rs`: `CredentialScrubMiddleware` (issue #4453). |
| `model.rs` | Helpers and wrappers over native crate `ChatModel` values: message/response translation, usage merging, stream-delta forwarding, `MaxTokensModel`, `ProfileOverrideModel`. |
| `tools.rs` | `CanonicalSharedToolAdapter`: resolves an OpenHuman shared tool registry and forwards the canonical `tinytools::Tool` contract; `EarlyExitHook` pauses successful `ask_user_clarification`-style calls. |
| `routes.rs` | `WORKLOAD_ROUTE_TIERS` (the tier inventory projected into the model registry), `RequiredCapabilitiesMiddleware`, `FallbackObserverMiddleware`, `UsageCarryMiddleware`, and the per-model fallback policy. |
| `topology.rs` | `all_graph_topologies`: behaviour-free `GraphTopology` exports of every custom OpenHuman graph for debug/inspection. |
| `observability.rs` + `observability/` | `event_bridge.rs`: `OpenhumanEventBridge`, translating crate `AgentEvent`s into `AgentProgress` and feeding per-call usage into `crate::platform::cost`. `event_projection.rs`: its `EventListener` impl. `cap_pauser.rs`: `CapPauser` and `SubagentScope`. `graph_tracing.rs`: `GraphTracingSink`. |
| `host/steering.rs` | OpenHuman's steering allowlist and the shared `SteeringRegistry` for detached sub-agents. Task primitives are imported directly from `tinyagents_graph::orchestration`. |
| `host/delegation.rs` | Explicit tracing-bound execution helpers over `tinyagents_graph::delegation`'s plan → execute ⇄ review → finalize graph. |
| `payload_summarizer.rs` | `PayloadSummarizer` trait, `SummarizeOutcome`/`UnavailableReason`, and the default `SubagentPayloadSummarizer` that compresses oversized tool results through the `summarizer` sub-agent instead of hard-truncating them. |
| `policy_denial.rs` | `maybe_enrich_policy_block`: rewrites `[policy-blocked]` tool results into structured what/why/workaround messages that tell the model to relay the denial rather than fabricate output. Called from `ToolOutcomeCaptureMiddleware`. |
| `abort_guard.rs` | `AbortOnDrop`: ties a detached streaming-producer task's lifetime to its consumer stream so a dropped turn aborts the in-flight provider call (issue #4460). |
| `host/run_context.rs` + typed tool dispatch | `OpenHumanRunContext` owns one root `CancellationToken`; child contexts share it and recursive tools receive it from their typed parent `RunContext`. |
| `steering_forwarder.rs` | `SteeringForwarderGuard`: forwards `RunQueue` steer/collect messages into the harness `SteeringHandle`; abort-on-drop so drop-based cancellation always deregisters it (issue #4456). |
| `stop_hooks.rs` | `StopHookMiddleware`: evaluates OpenHuman `StopHook`s after each model call and pauses the run via steering on the first `Stop` decision. |
| `summarize.rs` | `ModelSummarizer` / `FaultTolerantCachingSummarizer` plus a context-window-aware `SummarizationPolicy` driving the crate's `ContextCompressionMiddleware`. |
| `embeddings.rs` | `ProviderEmbeddingModel`: adapts `crate::inference::embedding_host::EmbeddingProvider` onto the crate's `EmbeddingModel` trait. |
| `retriever.rs` | `recall_through_facade` / `build_retriever`: wraps `Memory::recall`, projects onto the crate's `ScoredDoc`, applies the `path_scope` dedupe rule, emits `MemoryLoaded`. |
| `host/run_context.rs` | Explicit run-owned `thread_id` inherited by children and supplied to managed backend construction. |
| `todos.rs` | `todos_store` / `scratch_todos_store` (the crate `Store` behind per-thread agent todos, `tinyagents_graph::todos`). |
| `config.rs` | Maps OpenHuman's `Config` (including model pins) onto `tinyagents_harness::config` structs. |
| `*_tests.rs` | Sibling test suites for each file/part group above. |

## Public surface

Most of the module is `pub(crate)` or private. The `pub` items, per `mod.rs`:

- `config::*`: the host half of the generic-harness config seam.
- `payload_summarizer::*`: `pub` since issue #6014 so an embedder can pass its own `Arc<dyn PayloadSummarizer>` to `AgentBuilder::payload_summarizer` (the default dispatches a sub-agent, which some embedders cannot do).
- `routes::ResolvedRouteMiddleware`: records canonical response metadata in the explicit run context; `TinyagentsTurnOutcome` carries it to the bus.
- `todos::*`, `host::*` (all ten `OpenHuman*` adapter structs), and `TurnModelSource`.

Within the crate the entry point is `run_turn_via_tinyagents_shared` (`pub(crate)`); `run_turn_via_tinyagents` is `#[cfg(test)]` only.

## RPC (`replay/`)

Namespace `agent`, three read-only controllers registered in `core/all.rs` through `all_agent_replay_registered_controllers`, all over the durable stores in `journal.rs`:

| Method | Purpose |
| --- | --- |
| `openhuman.agent_run_events` | Paged late-attach replay of a run's durable event stream (`run_id`, `offset`, `limit`). |
| `openhuman.agent_run_status` | Latest `HarnessRunStatus` for a run, or `null` if unknown. |
| `openhuman.agent_runs_active` | Active runs, optionally filtered by `thread_id` and/or `root_run_id`. |

Responses project the crate's own `AgentObservation` and `HarnessRunStatus` serde shapes directly, with no bespoke DTO, since both are already what the store persists as JSON. Neither carries prompt text, tool arguments, or provider payloads.

## Dependencies

- The vendored crates under `vendor/tinyagents/`: `tinyagents-harness`, `tinyagents-graph`, `tinyagents-registry`, plus `tinyinference` and `tinytools` from `vendor/tinyagents/vendor/`, all declared as path dependencies in `crates/openhuman-core/Cargo.toml`. Per AGENTS.md, use this vendored copy; a second path to the same crates creates incompatible Rust types.
- `crate::agent::message_convert` for `ChatMessage` ↔ crate `Message` conversion.
- `crate::agent::harness::{run_queue, tool_result_artifacts, subagent_runner}` and `crate::agent::{messages, progress, stop_hooks, cost, hooks}`: the OpenHuman-side turn plumbing this seam plugs into.
- `crate::tools`: the canonical `tinytools::Tool` trait resolved by `CanonicalSharedToolAdapter`, and `tools::registry::denials` for recording policy blocks.
- `crate::platform::cost`: the global cost tracker fed by `observability/event_bridge.rs` and `turn_outcome.rs`.
- `crate::config` and `crate::inference`: tier constants, `Config`, providers, and embedding providers.

## Used by

- `agent/session_host/turn/graph.rs`: the chat-turn route into `run_turn_via_tinyagents_shared`.
- `agent/harness/graph.rs`: the channel/CLI bus turn route.
- `agent/harness/subagent_runner/ops/graph.rs`: the sub-agent spawn route. It and the session route both build their context-window summarizer through `TurnModelSource::build_summarizer`.
- `agent/session_host/builder/setters.rs`, `agent/harness/subagent_runner/ops/{provider,runner}.rs`, `channels/`: construct `TurnModelSource`.
- `agent/bus.rs`: reads `TinyagentsTurnOutcome::resolved_route` after a turn.
- `core/all.rs` (replay controllers) and `core/runtime/builder.rs` (`reaper::reap_orphaned_runs`).
- `agent/todos/`: `todos::*` stores.
- `memory/tools/{recall,store}.rs` and `memory/auto_recall/`: `host::agent_memory::DEFAULT_AGENT_MEMORY_NAMESPACE`.
- `flows/tinyflows/caps/{llm,prompt}.rs`: the message-conversion re-exports and `model::usage_info_from_response`.
- `tools/README.md` names `CanonicalSharedToolAdapter` and `ToolPolicyMiddleware` as the primary tinyagents-side tool consumers.

## Host adapters

`host/` implements the crate's ten host-capability traits and exposes one
`OpenHumanHostBundleFactory`. It constructs the complete
`HostCapabilities<()>` bundle from a single session/runtime input set, while
`OpenHumanRunContext` carries the explicit per-turn values (origin, progress,
attachments, artifacts, dispatch, cancellation, thread, workspace and stop
hooks) across the OpenHuman turn boundary. The shared runner builds
`RunContext<OpenHumanRunContext>` with `into_tinyagents`, and the OpenHuman
assembly plus middleware stack are specialized to that context. Model and
transcript consumers receive the owned thread through that carrier; no thread
task-local surrounds the drive. Cancellation reaches recursive tools solely
through the typed parent run.

## Notes

- The taint/scope/redaction/approval/budget guarantees the crate deliberately does not know about are enforced in `host/` and `middleware*.rs`. Widening a permission or dropping a scope filter to make an adapter signature fit would silently disable a guarantee the rest of the system assumes.
- `journal.rs` is best-effort: store opening and every write swallow errors behind a `[journal]` log line rather than failing a turn. `reaper.rs` logs its sweep under `[agent] startup run sweep`.
- `defer_turn_completed_to_caller` (a `run_turn_via_tinyagents_shared` parameter, issue #4457) exists because the chat/session path emits its own `TurnCompleted` after streaming a post-run checkpoint; callers without that step (channel/CLI) pass `false` and rely on this seam's emit.
- [gitbooks/developing/architecture/agent-harness.md](../../../../../gitbooks/developing/architecture/agent-harness.md) is the narrative overview of the turn lifecycle and where this seam sits in it.
