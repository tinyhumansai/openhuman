# Turn observers

Hosts implement `observe::TurnObserver` to receive model and tool observations
and a terminal `TurnTrace`. Callbacks run synchronously: enqueue export work in
the host and return promptly. Model observations contain the answering model,
finish reason, duration, and provider-reported usage/cost. Tool observations
pair starts and completions by call ID and report failure without an error body.
A completed tool observation also carries `input_bytes` (the serialized
arguments) and `output_bytes` (the result text the model is shown) whatever the
capture mode, so tool cost can be attributed without capturing content.
The terminal trace reports dispatch success, latency and final metadata.

`TraceContent::MetadataOnly` is the default. It excludes model messages, replies,
tool arguments/results, arbitrary tool metadata, raw error strings, streamed
deltas, and provider options. `TraceContent::Include` opts into messages and tool
payloads; it still excludes raw errors, arbitrary metadata and provider options.
Hosts own consent, redaction, retention, and export destinations for captured
content. Metadata such as session IDs, tool names and model IDs also needs an
appropriate retention policy.

```rust,no_run
use std::sync::Arc;
use openhuman_embed::observe::{TurnObserver, TurnObservation, TurnTrace};

struct Metrics;
impl TurnObserver for Metrics {
    fn on_event(&self, event: &TurnObservation) {
        // Enqueue metadata to your process's metrics collector.
    }
    fn on_turn(&self, trace: &TurnTrace<'_>) {
        // Record latency and success/failure without formatting payloads.
    }
}
```

An observer scope is captured before dispatch crosses to the owned core runtime
and restored inside its worker. A task-local alone would silently lose model and
tool observations across that boundary. Each root harness captures its own
scope; spawned child harnesses require their own observer scopes.

Model callbacks run around the model pipeline, before terminal schema/tool
validation. A premature answer rejected after a successful model response still
reports that response's model, finish reason and spend. Pipeline callbacks cover
logical model invocations; internal provider retry attempts are not separate
callback records. Failed pipeline invocations report `failed=true`, without
including raw provider errors. Unknown per-model charges stay `None`. Terminal
`LastTurnUsage` retains the core cost-source classification, which can distinguish
provider charges from catalogue estimates.

`TurnObservation::Setup` reports a step the turn ran before its first model
request, in the order the steps ran: `SetupPhase::AgentOpen`,
`TranscriptResume`, `PromptRefresh` and `MemoryBlock`, each with its duration
in milliseconds and whether it failed or was skipped (a memory block that
missed its `memory_block_deadline_ms`). A step the turn did not need (no
resume on a new thread, no refresh on a prompt that stays) reports nothing.
Setup events carry no text or identifiers, and `run_id` is usually `None`
because the run has not started; correlate by order within the observed turn.

The terminal callback runs after the supplied dispatch future completes,
including its cleanup, and handles failures that have no `TurnOutcome`. Dropping
the entire future cannot promise a terminal callback; use the turn cancellation
API and await its cleanup when completion telemetry is required.

## Existing Langfuse exporter

The optional `langfuse` feature forwards the existing transport through
`observe::langfuse`: `LangfuseClient`, `LangfuseAuth`, score types and trace
configuration, plus the OTLP span builder (`TraceSpan`, `SpanKind`,
`SpanStatus`, `ExportBrand`, `otlp_requests`). Hosts build their own spans
from curated `TurnObservation` events, turn them into OTLP requests with
`otlp_requests`, and queue `LangfuseClient::send_otlp` outside synchronous
callbacks. OTLP is Langfuse's supported trace path; the legacy batch
ingestion API is sunset on Langfuse Cloud on 2026-11-16, though scores still
go through `create_score`. The adapter adds
no second HTTP transport. Harness events, journal records, runtime identifiers
and native inference usage types are not part of this public facade.

The architecture gate inventories two exact SDK forwarding statements in
`scripts/lib/agent-sdk-contracts.mjs`: this telemetry transport contract and the
native atomic budget ledger primitives. Reusing the ledger preserves one
admission owner below physical retries; wrapping it with a second host ledger
would split reservations. These inventories are permanent public contracts,
not temporary violation baseline entries. Each matches its full statement and
owning file; adding a runtime symbol, alias, wildcard or moving the forwarding
requires explicit review and fails the anti-leak regressions.
See [CONSUMERS.md](CONSUMERS.md) for the current dependency footprint: gating the
public exporter does not make enabled Embed an HTTP-free build.

Loopback regressions live in `tests/turn_observers.rs` and
`tests/observed_turns.rs`, covering sanitized terminal errors and actual model /
host-tool events with metadata-only and explicit content capture.
