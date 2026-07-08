# C4 — Journal-backed progress projection & `progress_tracing` deletion

Status: execution plan (2026-07-04), written after a ground-truth code map of
both the OpenHuman progress surface and the vendored crate observability
primitives. This is the actionable plan for the C4 workstream's July 2026
continuation notes, and it is the gated prerequisite for **doc 03 (V3) Step
5** — deleting the `ProviderDelta` bridge and `progress_tracing`.

## 1. Corrected architecture (what the map found)

- **There is no crate `SpanCollector`.** The only span state machine is
  OpenHuman's `SpanCollector` in `src/openhuman/agent/progress_tracing.rs`
  (1272 lines). The crate does **not** build spans — it journals raw
  `AgentObservation`s and lets exporters project.
- **The journal/status/persistence stack already exists and is attached to
  every run.** `run_turn_via_tinyagents_shared`
  (`src/openhuman/tinyagents/mod.rs:420`) mints a run id, seeds the `EventSink`
  with it, and `attach_turn_journal` (`src/openhuman/tinyagents/journal.rs:304`)
  installs `StoreEventJournal` (over `JsonlAppendStore`) via a
  `JournalSink → RedactingSink → FanOutSink`, plus a durable `FileStatusStore`.
  Every run already durably records the crate `AgentEvent` stream as
  `AgentObservation`s.
- **Two producers of `AgentProgress`:**
  - Crate path: `OpenhumanEventBridge` (`src/openhuman/tinyagents/observability.rs:464`)
    maps `AgentEvent` → `AgentProgress` live (stateful: iteration cursor,
    subagent `scope`, `tool_names` recovery, display labels, failure class).
  - Legacy path: `session/tool_progress.rs` `TurnProgress` +
    `spawn_delta_forwarder` maps engine callbacks + `ProviderDelta` →
    `AgentProgress` (the **Step-5 deletion target**, 253 lines).
- **`SpanCollector` consumes `AgentProgress`** (the bridge *output*), while the
  journal captures the `AgentEvent` *input*. The web progress bridge
  (`channels/providers/web/progress_bridge.rs:157`) is a side-observer of the
  `AgentProgress` channel: `collector.record(&event, now)` per event, then
  `collector.finish()` + `export_run_trace(config, spans)` at loop exit.
- **Langfuse is exported twice, in two data models.**
  `progress_tracing/langfuse.rs` (825 lines) hand-rolls a `TraceSpan`→Langfuse
  ingestion batch; the crate `observability/langfuse/` already builds the same
  batch from `&[AgentObservation]` (`LangfuseClient::send_observations`,
  `build_ingestion_batch`). Same proxy path, same `207` handling.

## 2a. BLOCKER found during S1 (2026-07-04): the mapping is not journal-replayable

The S1 attempt to extract a pure `event_to_progress` revealed that the live
`OpenhumanEventBridge` mapping **depends on live side-channels absent from the
journal**, so "parity by construction via journal replay" (§2 below) does
**not** hold as written:

- **`ToolCompleted`** (`observability.rs:745`): the crate event
  `ToolCompleted { call_id, tool_name, started_at_ms, input, output }` carries
  **no outcome**. `success` / `elapsed_ms` / `output_chars` / failure class are
  read from `self.failure_map`, a `call_id → (ok, failure, elapsed, chars)` map
  populated by `ToolOutcomeCaptureMiddleware` — not journalled.
- **`record_usage`** (`observability.rs:298`): drains `usage_carry` (the model
  adapter's provider `UsageInfo` FIFO) to restore **charged USD**, cache-creation
  and context-window tokens the crate `Usage` drops. Not journalled.

A journal-only projection therefore yields structurally-correct spans with
**degraded attributes** (assumed-success tools, zero durations/sizes, `$0`
cost) — which cannot pass the S3/S5 parity gate.

### Corrected prerequisite — S0: make the journal self-sufficient (crate work)

Before any journal projection can hit parity, the enriching data must live in
the journalled `AgentEvent`s, not in OpenHuman side-channels:

1. **Enrich `AgentEvent::ToolCompleted`** in the crate with the outcome:
   `success: bool`, `duration_ms`, `output_bytes` (and an optional structured
   failure), populated in `tools.rs::finish_tool_call` from the `ToolResult`
   and the `started_at_ms` it already tracks. OpenHuman's bridge then reads them
   from the event; `failure_map`/`ToolOutcomeCaptureMiddleware` shrink to the
   product-specific `ToolFailureClass` mapping (or that too moves onto the
   event). This is a V-series crate PR (additive event fields).
2. **Usage accounting**: decide whether charged-USD/provider-cost belongs on a
   crate event (e.g. a `provider_cost`/`cache_creation` extension on
   `UsageRecorded`/`Usage`) or stays a documented, accepted OpenHuman-only trace
   attribute filled at export time from the status/cost store rather than the
   live side-channel. (The cost roll-up is already persisted per-run; the trace
   can read it from there instead of `usage_carry`.)
3. Only after S0 does §2's "reuse the mapping" become true by construction.

**Sequencing impact:** S0 (crate event enrichment) is now the first slice and
gates S1→S2. It is separate crate work that should be PR'd upstream like V1/V2.
The remaining S1–S6 below stand, rebased on S0.

## 2. The parity-preserving approach (valid only after S0)

Reproduce the span tree **by construction**, not by re-deriving it:

> Replay journal `AgentObservation`s → `AgentProgress` using the *same* mapping
> the live bridge uses → fold through the *existing* `SpanCollector`.

Concretely:

1. **Extract the bridge mapping into a pure, reusable function**
   `event_to_progress(event: &AgentEvent, state: &mut BridgeState, scope) -> Vec<AgentProgress>`
   (state = iteration cursor + `tool_names` + scope). The live
   `OpenhumanEventBridge::on_event` becomes a thin driver over it (refactor, **no
   behavior change** — guarded by the existing bridge tests). This makes the
   AgentEvent→AgentProgress mapping a single source of truth.
2. **Journal projection**
   `spans_from_observations(ctx: TraceContext, obs: &[AgentObservation]) -> Vec<TraceSpan>`:
   fold each observation through `event_to_progress` into `AgentProgress`, feed a
   fresh `SpanCollector::record(...)` stamped with `obs.ts_ms`, then `finish()`.
   Because it reuses `SpanCollector`, span-shape parity is guaranteed for every
   `AgentProgress` variant the journal can produce.
3. **Parity harness** (`progress_tracing`'s `tests.rs` is the oracle): drive a
   representative synthetic run and assert
   `spans_from_observations(journal)` == `SpanCollector` fed the live
   `AgentProgress`. Cover: multi-iteration turn, tool calls (success + failed +
   unknown-tool recovery), model-call generation spans w/ usage & reasoning &
   cache-creation, nested sub-agents, cost roll-up.

### Parity gaps to resolve (AgentProgress with no journal `AgentEvent` source)

`SpanCollector` ignores streaming/content deltas for spans
(`progress_tracing.rs:1076`), so `TextDelta`/`ThinkingDelta`/`ToolCallArgsDelta`
**do not affect span parity** — safe to drop on replay. The variants that *do*
carry span data but have no direct `AgentEvent`:

| AgentProgress | Span effect | Journal source | Resolution |
| --- | --- | --- | --- |
| `TurnContent{prompt, reply}` | root span `input`/`output` (gated on `capture_content`) | `ModelCompleted.input/output` present in journal but shaped differently | project root i/o from `ModelStarted`/`ModelCompleted` payloads, or emit a journalled content event |
| `TaskBoardUpdated` | none (no span) | n/a | ignore for spans |
| `SubagentAwaitingUser` | subagent span attr | partial (`SubAgentStarted/Completed` only) | accept minor attr gap or add a crate event (V6) |
| `TurnCostUpdated` | root usage roll-up | `UsageRecorded`/`CostRecorded` in journal | project from those (already in bridge) |

Document any accepted gap in the parity test as an explicit, reviewed
exception.

## 3. Langfuse swap (separable, lower-risk slice)

Replace `progress_tracing/langfuse.rs::push_spans` with the crate
`LangfuseClient::proxy(...).send_observations(trace_cfg, &obs)` reading the run's
journal. Parity test: assert the crate batch matches the existing batch shape
for a fixture run (trace-create + per-observation generation/span/event,
`usageDetails`/`costDetails`, `207` handling, content gating from
`agent_tracing.capture_content`). This deletes ~825 lines independently of the
span-projection slice.

## 4. Sequenced slices (each: code + tests + green build; deletions gated)

0. **S0 — enrich crate `AgentEvent::ToolCompleted`** with `duration_ms` /
   `output_bytes` / `error` so the journal is self-sufficient for tool outcomes
   (§2a). **DONE** — tinyagents#18 (branch `feat/tool-completed-outcome`,
   933 tests green). Also enriches the crate Langfuse tool span. Merge + gitlink
   bump precede S1. (Usage/charged-USD accounting — §2a item 2 — is still open:
   fill it at export time from the persisted per-run cost store rather than the
   `usage_carry` side-channel.)
1. **S1 — extract `event_to_progress`** (pure mapping) + make the live bridge a
   driver over it. No behavior change; existing bridge/`observability` tests are
   the gate. *No deletion.*
2. **S2 — `spans_from_observations`** journal projection + parity harness vs
   `SpanCollector`. **IN PROGRESS** — additive projection module now covers the
   single-agent spine, S0 tool outcomes, root `TurnContent` from captured model
   I/O, and sub-agent lifecycle/scoped child tool/model spans. Remaining known
   gap: per-call charged USD/cache-creation is still zero until export-time cost
   store reconciliation lands.
3. **S3 — flip the web progress bridge** to build spans from the journal at run
   end (`spans_from_observations`) instead of the live `AgentProgress`
   side-observer; keep the old path behind a shadow-compare for one release
   (log divergences), matching the C1 `session_shadow_reads` pattern. **STARTED**
   — the web bridge now keeps live export as-is and logs a structural
   journal-projection shadow comparison keyed by the durable journal run id.
4. **S4 — Langfuse swap** to the crate exporter (§3). **STARTED** — when the
   web bridge can read a durable run journal for S3 shadow comparison, the
   remote `share_usage_data` push now sends those `AgentObservation`s through
   the crate `LangfuseClient`; live spans remain the local tracing sink and
   fallback until S3 shadow parity is release-proven. The OpenHuman wrapper also
   injects aggregate `run.total` usage/cost from the durable run ledger so
   journal-backed Langfuse export preserves the top-level cost signal while
   per-call charged USD awaits a journal-native event. Later delete
   `progress_tracing/langfuse.rs` (~825 + tests).
5. **S5 — delete `progress_tracing.rs` + `SpanCollector`** once S3 shadow shows
   no divergence for one release **and** V3 projection parity holds (the doc 07
   gate). Tick the deletion ledger (~2k incl. tests).
6. **S6 (= V3 Step 5)** — with `AgentProgress` now a journal projection, delete
   the legacy `ProviderDelta` bridge in `session/tool_progress.rs` (253) and the
   `spawn_delta_forwarder`, since the crate event path is the only producer.

## 5. Gates (doc 07)

- `progress_tracing` delete: **C4 shadow parity (S3) for one release AND V3
  projection parity.** Langfuse (S4) may land earlier (independent shape parity).
- Approval/security/redaction boundaries unchanged: the journal path already
  runs through `RedactingSink` (`journal.rs:327`); the projection must not
  re-introduce raw prompt/PII into spans beyond what `capture_content` already
  gates.

## 6. Key file references

Producer/bridge: `tinyagents/observability.rs:464` (`OpenhumanEventBridge`),
`tinyagents/journal.rs:304` (`attach_turn_journal`), `tinyagents/mod.rs:420`
(`run_turn_via_tinyagents_shared`). Span machine:
`agent/progress_tracing.rs` (`SpanCollector:307`, `record:686`, `finish:1087`,
`export_run_trace:1254`), driven at
`channels/providers/web/progress_bridge.rs:157`. Crate foundation:
`harness/observability/{mod,types}.rs` (`HarnessEventJournal:156`,
`StoreEventJournal`, `JournalSink:581`, `AgentObservation:40`),
`harness/observability/langfuse/`. Parity oracle:
`agent/progress_tracing/tests.rs` (1169 lines).
