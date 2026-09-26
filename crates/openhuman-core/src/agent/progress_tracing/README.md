# progress_tracing

Turns the agent's real-time [`AgentProgress`](../progress.rs) event stream into
OpenTelemetry/Langfuse-style trace spans (`agent.turn` -> `agent.iteration` ->
`tool.*`/`subagent.*`), correlated by session id, for offline inspection and
debugging long multi-agent runs (issue #3886). The module doc on
`agent/progress_tracing.rs` has the full span-tree shape and the
content-capture privacy gate (`observability.agent_tracing.capture_content`,
default on; enforced once, in `SpanCollector`, so no exporter can leak
content).

## Key files

- `agent/progress_tracing.rs` (parent module file) declares the submodules
  below it. `collector/` — `SpanCollector` (pure state machine: feed it
  progress events plus a timestamp, it accumulates finished `TraceSpan`s).
  `types.rs` — `TraceContext`, `RunType`, `SpanKind`/`SpanStatus`.
  `serialize.rs` — `spans_to_ndjson`. `export.rs` — the local file/log
  exporter `export_spans`, and the two run-completion entry points
  `export_run_trace` / `export_run_trace_from_journal`. Each entry point runs
  two independent, best-effort paths: a Langfuse push when
  `observability.share_usage_data` is on (the default), and local NDJSON
  export to `export_path` or the app log when
  `observability.agent_tracing.enabled` is on (opt-in).
- `otlp.rs` — the remote agent-turn exporter. It turns completed live spans
  into OTLP/HTTP JSON, preserves the root turn's input/output, maps
  TinyInference messages into role-labeled conversations, puts usage only on
  model generations, and summarizes repeated internal tool discovery.
  It POSTs through the authenticated backend's
  `/telemetry/langfuse/otel/v1/traces` proxy; the backend supplies project
  keys and authoritative user attribution. Export is on by default through
  `observability.share_usage_data`; local NDJSON export remains optional.
- `langfuse.rs` + `langfuse/` retain the legacy batch projection for
  compatibility tests and the flow-run exporter. Agent turns use `otlp.rs`.
- `journal_projection.rs` — `spans_from_observations` rebuilds spans from the
  durable `AgentObservation` journal instead of the live stream, by folding
  journalled events through the same `SpanCollector`, so a UI/supervisor can
  attach after a run. Its match is exhaustive over
  `tinyagents_harness::events::AgentEvent`; the module doc lists the known
  parity gaps (estimated vs. charged cost, no subagent prompt/output content).

Tests live in this directory as `*_tests.rs` files, e.g.
`progress_tracing_tests.rs`, `progress_tracing_span_tree_tests.rs`,
`progress_tracing_attribution_tests.rs`, `progress_tracing_content_gate_tests.rs`,
`langfuse_tests.rs`, `langfuse_batch_tests.rs`, `langfuse_trace_fields_tests.rs`,
`journal_projection_tests.rs`, `journal_projection_cost_rollup_tests.rs`.

## Called by

- `web_chat/progress_bridge.rs` — the only caller. Builds a `SpanCollector`
  per run, feeds it `AgentProgress`, shadow-compares the live spans against
  `spans_from_observations` over the run journal, then calls
  `export_run_trace_from_journal` (journal available) or `export_run_trace`.

`flows/tinyflows/langfuse_export.rs` is the separate flow-run exporter and
currently still uses the legacy batch proxy.

## Related docs

- [gitbooks/developing/agent-observability.md](../../../../../gitbooks/developing/agent-observability.md)
