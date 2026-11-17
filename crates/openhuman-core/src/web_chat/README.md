# Web chat

The in-app chat (desktop and web) is a thin host. It accepts user text and
typed media references, runs them through the agent harness on the thread's
own session, and ends with a reply that is stored durably and announced to the
client exactly once. It owns the `channel.web_*` RPC namespace and the business
logic behind the Socket.IO `chat:start` and `chat:cancel` handlers.

Two shared engines do the generic work, and this module is only what OpenHuman
alone can supply:

| Owner | Owns | OpenHuman's host seam |
| --- | --- | --- |
| TinyAgents harness | Generic turn admission, run-queue mechanics, and work tracking through `tinyagents_harness::turn_runner` | [`QueuedPayload`](../agent/queued_turn.rs) carries product metadata; [`runner.rs`](./runner.rs) and [`turn_host.rs`](./turn_host.rs) supply web-chat policy and host ports. |
| TinyChannels v2 module and bus | Canonical typed web-event wire contract, generated/shared event types, and projection of output events with per-turn `seq` | [`surface.rs`](./surface.rs) adapts the host to the compiled module through `tinychannels-bus`; OpenHuman retains routing stamps and process-wide event publication. |

OpenHuman owns the product boundary around those engines: the `channel.web_*`
RPC and socket handlers, input validation and attachment trust, approvals,
session checkout, reply persistence, terminal policy, and host event delivery.
It does not own a second turn queue or a web-output event mapper. The host
adapter translates the module's canonical event type into the process bus
envelope without changing its wire payload.

Hosted channel domain events are dispatched through this module's `start_chat`
via `channels/bus/subscriber.rs`; those turns share the web-chat runner. Relay
progressive output is projected through the same compiled web projection used
by web chat and TUI, while the gateway keeps its separate provider-delivery
terminal event. Native listeners retain their channel-specific ingress and
turn dispatch, and use the shared projection when a scoped web target is
configured. The web event bus is consumed by Socket.IO, the JSON-RPC `/events`
SSE stream, the TUI and the hosted channel subscriber.

## How it works

### The request path

```text
 Socket.IO chat:start         channel.web_chat RPC      hosted channels subscriber
 (openhuman-rpc socketio.rs)  (schemas.rs)              threads edit/regenerate
            \                      |                        /
             v                     v                       v
        +-----------------------------------------------------+
        | start_chat (ops/start_chat.rs)                      |
        |  validate -> TurnRequest<QueuedPayload> -> runner   |
        |  answers: started | queued | approval reply | refused|
        +-----------------------------------------------------+
             v   tokio::spawn( TurnRunner::run )            (runner.rs)
        +-----------------------------------------------------+
        | ingress    (ingress.rs)  attachments -> guardrail   |
        |                          -> approval reply -> hook  |
        | queue mode  Interrupt | Steer/Followup/Collect | Parallel
        | checkout   (turn_host.rs, session.rs)               |
        | run        agent.run_single_with_approval           |
        |   progress  --tap--> runner --> ProgressProjection  |
        |                       --> compiled module --> bus  |
        | persist    (turn_events.rs WebReplies)              |
        | terminal   (turn_events.rs WebEvents)               |
        +-----------------------------------------------------+
     Completed -> one chat_done (+ chat_suggestions later)
     Failed    -> web_errors::classify_inference_error -> one chat_error
     Cancelled -> chat_error{cancelled} + chat_cancelled
```

1. A `chat:start` socket event arrives in `openhuman-rpc/src/server/socketio.rs`
   (or a `channel.web_chat` RPC call, or a hosted channel domain event) and
   calls `start_chat` with the raw message, the thread and client ids, and any
   model, temperature, locale or queue-mode overrides.

2. `start_chat` builds a `TurnRequest<QueuedPayload>` (the payload,
   `agent/queued_turn.rs`, carries the context, client id, model, temperature,
   locale and `ChatRequestMetadata`) and spawns `TurnRunner::run` under
   `CoreContext::propagate`. It then waits for the first answer on the
   request's `Admission` and returns it: the new request id (`Started`), the
   queued JSON (`Queued`), the approval request id (`Consumed`) or the refusal
   (`Rejected`). The generic `TurnRunner` runs one request at a time per
   `(agent, profile, thread)`; `WebRuntime` (`runner.rs`) holds one runner per
   core context, so two embedded agents never share a turn table.

3. **Ingress** (`ingress.rs`, `WebIngress`) is the runner's `IngressPolicy`,
   called once per stage in a fixed order. Modern media arrives as typed
   `ContentPart` values: uploads are persisted with the user row as scoped
   opaque references, then resolved under the trusted turn context. Queued
   media is revalidated at execution before its row is persisted. The legacy
   textual `[FILE:...]` and `[IMAGE:...]` marker path is retained for older
   callers; it stages through `attachments::stage` before prompt
   scanning. `security::prompt_injection::enforce_prompt_input` follows; a
   block comes back as `StartChatError::Guardrail` with a verdict, score and
   reasons so the frontend can classify on `chat_error.error_type ==
   "guardrail"` instead of matching copy. Next, a parked chat-native approval
   plus a message that parses as an approval reply
   (`security::approval::parse_approval_reply`) goes to the approval gate and
   no turn starts. Only then does a configured `beforeSubmitPrompt` hook see
   the message, so a bare "yes" answering an approval can never be blocked by a
   hook. A refusal or a consumed reply is answered through the `Admission` and
   publishes nothing.

4. **Queue modes.** `QueueMode` (`tinychannels_bus::web`) has five values.
   `Interrupt` (the default) cancels the thread's turn and starts this one;
   the superseded turn is announced as `chat_cancelled{superseded}` naming its
   replacement. `Steer`, `Followup` and `Collect` join the running turn's
   `RunQueue` (the very queue the agent's steering feed drains mid-turn) and
   are answered `queued`; with nothing in flight they start a normal turn.
   What the turn did not absorb is handed back by the runner once it
   completed: steers and follow-ups start as fresh turns, leftover collects
   start as follow-ups (`WebEvents::collected`). A failed or cancelled turn
   drops its queue (those messages are never announced). `Parallel` runs an
   isolated fork (`checkout_fork`) beside the primary turn without touching
   interrupt or queue semantics.

5. **Checkout and run** (`turn_host.rs`, `WebHost`). `checkout` resolves the
   config and the thread's working folder, then takes the session agent out of
   the per-thread cache (`session::checkout_session_agent`). The entry is
   removed from `thread_sessions()` for the duration of the turn so two turns
   can never drive one agent; it is reused when its `SessionCacheFingerprint`
   still matches, otherwise a new agent is built. A new agent needs no history
   seeding: `set_thread_id` binds the session's durable identity
   (`SessionRef`) and the turn resumes the thread's one transcript by it.
   `run` drives `agent.run_single_with_approval` under the `WebChat` turn
   origin and the chat `ApprovalTurnContext`, applies the per-thread budget
   correlation on error, speaks the reply when asked, and keeps the reply for
   the reply store. `checkin` returns a healthy agent to the cache and drops a
   failed one, so the next request cold-boots from the durable transcript.

6. **Progress, projected once.** The agent's `AgentProgress` stream goes
   through a tap (`turn_host::tap_progress`) that folds each event into the
   idle record and tells the runner when model or tool work opens and closes
   (`WorkStarted` / `WorkFinished`, TinyAgents' `turn_runner::WorkTracker`), then
   hands it to the runner, which calls `WebEvents::progress` in order. That is
   `ProgressProjection::on_event` (`progress.rs`), the one host consumer: it
   adapts `AgentProgress` to TinyChannels' typed `ChannelOutputEvent` inputs
   (`progress_map.rs`) and asks the compiled module to project them onto the
   web wire (`surface.rs`). It also does host side effects in the same pass:
   the run ledger (`progress_ledger.rs`), the `TurnStates`
   mirror, the tracing export (`progress_trace.rs`),
   first-token timing and the throttled live `turn_cost`. An
   `inference_heartbeat` every `INFERENCE_HEARTBEAT_SECS` (20 s) keeps a long
   silent prefill from tripping the frontend's roughly 120 s silence timeout
   (#4270). The Flow Canvas copilot drives the same projection from a channel
   through `spawn_progress_bridge`.

7. **Completion.** The runner persists the reply (`WebReplies`, which uses
   `reply_persistence::persist_delivered_reply`) under the deterministic id
   `run_reply_message_id(request_id)`, so the answer survives a client
   reconnect or reload (#6034) and the client's own append collapses onto the
   same row. It then releases the thread and emits the terminal event, so a
   `chat_done` is never what lets the next message interrupt the turn that
   just finished. `WebEvents::terminal` announces exactly one `chat_done`
   carrying the model's unmodified text (`presentation.rs`), and for the main
   single-user turn spawns `suggestions::spawn_follow_up_suggestions`, a cheap
   `summarization`-role call that may emit `chat_suggestions` after
   `chat_done` (gated on `web_chat.suggestions_enabled`).

8. **Failure.** `run` first consults the per-thread budget signal
   (`ops/budget_correlation.rs`): an empty 200 on the same provider binding as
   a recent budget-exhausted failure is reclassified as out-of-credits (#3386),
   because the managed route closes the stream cleanly when credits run out.
   `WebEvents::terminal` runs the error through
   `web_errors::classify_inference_error`, emits one `chat_error`, and
   `sentry_suppression_reason` decides whether it pages.

### The chat_error payload

A `chat_error` carries `message` (finished English copy, kept for older UIs,
the CLI, the TUI and embedders), `copy_key` (`chat_error.<class>`, one per row
of `inference/failure_copy/table.rs`) and `copy_params` (`retry_after_secs`,
`provider`, `detail`). The app renders the key in the user's locale
(`app/src/lib/chatErrorCopy.ts`) and falls back to `message` for an unknown or
missing key. The classifier covers budget exhaustion, non-retryable rate
limits, an exhausted fallback chain, turn timeouts, backend error codes and
generic provider failures. Loop-guard halt summaries are not `chat_error`
events (they become the turn's reply text), so they carry no key.

### Host-authored turns

Background-delivery notices (`agent::orchestration::background_delivery`) and
goal continuations enter through `run_system_turn_on_thread`
([`ops/system_turn.rs`](./ops/system_turn.rs)) instead of `start_chat`. They skip ingress, the turn runner
and the progress projection, run with `SYSTEM_CLIENT_ID` ("system"), and return
the reply text for the caller to deliver. They still go through the same
session checkout, so the model sees the conversation and the turn lands in the
thread's transcript. A turn run on a throwaway session host would write a
competing root transcript that the next resume preferred, dropping every
earlier turn.

Such a turn checks out with `CheckoutPolicy::AdoptCached` (reuse the thread's
agent under whatever settings the user's last turn chose, instead of
rebuilding on a fingerprint miss) and checks in with
`checkin_session_agent_if_vacant`, so a user turn that started meanwhile and
re-cached its own agent wins. A checkout failure returns an error prefixed with
`SESSION_CHECKOUT_FAILURE`, which callers use to tell "no turn ran" from "the
turn failed".

### Session cache fingerprint

`SessionCacheFingerprint` ([`types.rs`](./types.rs)) decides when a cached agent can be
reused. It holds the model override, the resolved effective model (so a
changed managed default rebuilds even without a picker override), temperature,
target agent id, provider binding, an autonomy signature and a model-registry
signature (toggling a model's vision flag keeps the model id but must
rebuild). A miss logs the fields that differ. Adding a dimension that should
force a rebuild means adding a field there and filling it in
`build_session_fingerprint` ([`session.rs`](./session.rs)).

`session::effective_session_config` applies a concrete picker provider and
model to the per-turn config clone. Managed `openrouter/...` defaults restore
the managed route after restart without saving changes or touching sibling
role routes; hints and unqualified legacy model ids keep their configured
provider route. `provider_role_for_model_override` picks the provider role
that feeds the fingerprint's binding.

### Cancellation and the guards

`cancel_chat_scoped` and `channel_web_cancel` (`ops/channel_ops.rs`) cancel by
thread, or by request id when one is given, through the runner's
`cancel_requests`. A stale cancel for a superseded request is ignored, because
the runner cancels only the generation the id names, so the newer turn
survives. Without a request id, a cancel also stops the thread's parallel forks
and its detached background sub-agents. The cancelled turn gets a short grace
(`cancel_grace`, 3 s) to hand back the rounds it completed, with its own
cancellation token already fired. The runner publishes each cancelled turn's
terminal event; `CancelNote` (`runner.rs`) tells it why (a Stop, or which
request superseded it) and which client to address.

Two guards make sure a turn always ends in a terminal event instead of an
endless heartbeat stream (#4746), both owned by the runner:

- The **idle guard** ends a turn that has emitted no progress event, with no
  model or tool call in flight, for 900 s (`OPENHUMAN_WEB_TURN_IDLE_SECS`, `0`
  disables it). The host feeds it: every event is progress, and
  TinyAgents' `turn_runner::WorkTracker` reports open calls, so a silent provider retry
  or a long shell build is never idle. What it catches is silence nothing else
  bounds: session checkout, the persistence plumbing, or a harness that
  stopped with no call open.
- The **wall-clock backstop** is the fixed total ceiling, 3600 s by default
  (`web_chat::turn_guards::backstop`, `OPENHUMAN_WEB_TURN_TIMEOUT_SECS`, `0`
  disables it and the derived deadlines). The harness winds the turn down at
  3480 s and writes the grounded close, and its own `max_wall_clock_ms` is
  clamped to 3540 s, so a busy turn still delivers an answer before the
  backstop drops it.

When either fires, the runner asks `WebSummarizer::grounded_close`
(`turn_close.rs`) for a reply that says what the turn got done and where it got
stuck, built from the turn-state snapshot and `turn_activity::StuckPoint`. That
reply is delivered like any other. A turn that stopped before it resolved its
own config has no workspace to write to and ends in a retryable `turn_timeout`
`chat_error`.

### The event bus

[`event_bus.rs`](./event_bus.rs) holds an in-process `tokio::sync::broadcast` channel of
`ScopedWebEvent` (the host routing envelope in [`channel_event.rs`](./channel_event.rs)). Its
wire payload is TinyChannels' typed `WebChannelEvent`; any domain can publish through
`publish_web_channel_event`, and consumers call
`subscribe_web_channel_events`. It also registers process-lifetime,
`OnceLock`-guarded subscribers on `core::bus::BUS` that turn `DomainEvent`s
into socket events:

| Registration | DomainEvents | Socket events |
| --- | --- | --- |
| `register_approval_surface_subscriber` | `ApprovalRequested`, `ApprovalDecided`, `PlanReviewRequested`, `PlanReviewDecided` | `approval_request`, `approval_decided`, `plan_review_request`, `plan_review_decided` |
| `register_artifact_surface_subscriber` | `ArtifactPending`, `ArtifactReady`, `ArtifactFailed` | `artifact_pending`, `artifact_ready`, `artifact_failed` |
| `register_agent_surface_subscriber` | thread goal, todo and run-mode changes; run-queue queued, delivered, dispatched and interrupted | `thread_goal_updated`, `thread_goal_cleared`, `thread_todos_changed`, `run_mode_changed`, `queue_item_queued`, `queue_item_delivered` |
| `register_memory_activity_surface_subscriber` | `MemoryStored`, `MemoryRecalled` | `memory_activity` |
| `register_egress_surface_subscriber` ([`egress_surface.rs`](./egress_surface.rs)) | `ExternalTransferPending`, only when the transfer carries chat routing | `external_transfer_pending` |

They are registered from `core/runtime/bootstrap.rs` and
`channels/runtime/startup/start_channels.rs`; the TUI registers the approval
and artifact bridges itself (`openhuman-tui/src/runner.rs`).

## Layout

| Path | What it does |
| --- | --- |
| [`mod.rs`](./mod.rs) | Module wiring and re-exports. No business logic. |
| [`runner.rs`](./runner.rs) | `WebRuntime` (the runner plus its shared state, one per core context), `Shared`/`TurnSlot` per-request state, `Admission`, `CancelNote`. |
| [`ingress.rs`](./ingress.rs) | `WebIngress`: the runner's ingress policy (attachments, guardrail, approval reply, hook). |
| [`turn_host.rs`](./turn_host.rs) | `WebHost`: session checkout/checkin and the agent run, the progress tap, the heartbeat. |
| [`turn_events.rs`](./turn_events.rs) | `WebEvents` (progress and the terminal event), `WebFailures` (error copy), `WebReplies` (reply store), `WebSummarizer` (grounded close). Sentry suppression and timeout tagging. |
| [`turn_activity.rs`](./turn_activity.rs) | Where a turn got stuck (`TurnActivity`, `StuckPoint`) and its reply context. Work-liveness folding lives in TinyAgents' `turn_runner::WorkTracker`. |
| [`turn_close.rs`](./turn_close.rs) | The reply that closes a turn a guard stopped. |
| [`turn_guards/backstop.rs`](./turn_guards/backstop.rs) | The web-turn wall-clock backstop: default, env override, `0` opt-out. |
| [`citations.rs`](./citations.rs) | `TurnCitation`: memory hits a turn cited, recorded per thread and attached to `chat_done`. |
| [`progress.rs`](./progress.rs) | `ProgressProjection`, the single consumer of a turn's progress, and `spawn_progress_bridge`. |
| [`progress_map.rs`](./progress_map.rs) | `AgentProgress` to the canonical `ChannelOutputEvent`, with payload caps. |
| [`progress_ledger.rs`](./progress_ledger.rs) | The run-ledger rows and telemetry a turn's progress writes. |
| [`progress_trace.rs`](./progress_trace.rs) | Tracing export for one bridged turn. |
| [`surface.rs`](./surface.rs) | The host adapter for TinyChannels' compiled projection module: `Surface` owns per-turn module handles, stamps host routing, and publishes projected events onto the broadcast bus. |
| [`ops.rs`](./ops.rs) | Thin shell over [`ops/`](./ops/); re-exports the request surface and state. |
| [`ops/start_chat.rs`](./ops/start_chat.rs) | `start_chat`, `StartChatError`, and the backstop resolution. |
| [`ops/channel_ops.rs`](./ops/channel_ops.rs) | `cancel_chat_scoped`, `cancel_all_turns`, and the `channel_web_*` RPC handlers. |
| [`ops/system_turn.rs`](./ops/system_turn.rs) | `run_system_turn_on_thread` for host-authored turns. |
| [`ops/state.rs`](./ops/state.rs) | `thread_sessions()`, keying helpers, `invalidate_thread_sessions`, `cancel_should_target`, the test accessors over the runner. Each table is a slot of the ambient agent context, so two embedded agents never share an entry; outside an agent context it is the process default. |
| [`ops/budget_correlation.rs`](./ops/budget_correlation.rs) | `thread_budget_signals()` and `classify_budget_correlation` for the empty-200-after-budget case. |
| [`ops/test_hooks.rs`](./ops/test_hooks.rs) | Debug/test hooks that force or park a turn. |
| [`session.rs`](./session.rs) | Session checkout and checkin, fingerprinting, `CheckoutPolicy`, target agent id, locale reply directive, per-turn provider and model routing. |
| [`presentation.rs`](./presentation.rs) | The terminal `chat_done` event, `deliver_response` (persist, then one `chat_done`) for the Flow Canvas, and `deliver_response_single_bubble` for core-initiated turns. |
| [`reply_persistence.rs`](./reply_persistence.rs) | `persist_delivered_reply`: durable reply row under the deterministic reply id. |
| [`suggestions.rs`](./suggestions.rs) | Post-turn follow-up suggestions (`chat_suggestions`). |
| [`channel_event.rs`](./channel_event.rs) | The host routing envelope around TinyChannels' canonical typed web event; only routing stamps remain host-private. |
| [`event_bus.rs`](./event_bus.rs) | The broadcast channel and the `DomainEvent` surface subscribers. |
| [`egress_surface.rs`](./egress_surface.rs) | The `external_transfer_pending` bridge. |
| [`web_errors.rs`](./web_errors.rs) | Thin shell over [`web_errors/`](./web_errors/): `classify.rs` (the classification ladder and `ClassifiedError`), `backend_error_code.rs`, `budget.rs`, `retry.rs`, `timeout.rs`. The class to copy table is in `inference/failure_copy/`. |
| [`schemas.rs`](./schemas.rs) | Controller schemas and thin handlers for the `channel` namespace. |
| [`voice_notes.rs`](./voice_notes.rs) | In-app voice-note storage and reply-speech policy. |
| `types.rs` | `SessionEntry`, `SessionCacheFingerprint`, `WebChatTaskResult`, RPC param structs; re-exports `ChatRequestMetadata`. |

## Key types and entry points

- `start_chat` ([`ops/start_chat.rs`](./ops/start_chat.rs)) is the entry point for every user turn.
  It returns the new request id, or a `StartChatError`.
- `run_system_turn_on_thread` (`ops/system_turn.rs`) runs a host-authored turn
  on a thread's own session and returns the reply text.
- `cancel_chat_scoped` (`ops/channel_ops.rs`) stops a turn by thread or
  request id.
- `invalidate_thread_sessions` ([`ops/state.rs`](./ops/state.rs)) drops a thread's cached agent.
  Thread edit and regenerate (`threads/ops/edit.rs`) and channel remote
  control (`channels/host/remote_control.rs`) call it.
- `QueuedTurn` / `QueuedPayload` (`agent/queued_turn.rs`) are the runner's
  `TurnRequest` and the host metadata it carries; the same type is the run
  queue's item in the agent harness.
- `ChatRequestMetadata` (`tinychannels_bus::web`) is per-request metadata
  (`speak_reply`, `source`, `session_id`, `agent_id` for trace attribution)
  passed by every caller of `start_chat` and `spawn_progress_bridge`.
- `tinychannels_bus::web::WebChannelEvent` is the typed event payload sent to
  clients; `ScopedWebEvent` carries host-private routing stamps beside it.
  New fields are additive so older clients keep working.
- `publish_web_channel_event` / `subscribe_web_channel_events`
  (`event_bus.rs`) are the bus.
- `spawn_progress_bridge` and `presentation::deliver_response*` are reused by
  core-initiated turns (`flows/ops/streaming.rs`,
  `agent/orchestration/background_delivery.rs`) so they render on the same
  socket surface.
- `persist_delivered_reply` and `pick_target_agent_id` are also used by the
  cron scheduler's origin delivery (`cron/scheduler/origin_delivery.rs`).
- `classify_inference_error` ([`web_errors/classify.rs`](./web_errors/classify.rs)) maps a flattened
  error string to a `ClassifiedError`.

## RPC surface

Namespace `channel`, registered through
`all_web_channel_registered_controllers()` in `core/all.rs` under
`DomainGroup::Channels`. It is deliberately not behind the `channels` feature,
because the in-app chat is core product surface (#5002).

| Method | Handler | Purpose |
| --- | --- | --- |
| `channel.web_chat` | `channel_web_chat` | Send a message through the agent loop. Takes `client_id`, `thread_id`, `message`, plus optional `model_override`, `temperature`, `locale`, `speak_reply`, `source`, `session_id`, `queue_mode`, `run_mode`, `reasoning_effort`. |
| `channel.web_cancel` | `channel_web_cancel` | Cancel the thread's turn, or one `request_id`. |
| `channel.web_queue_status` | `channel_web_queue_status` | Run-queue status for a thread. |
| `channel.web_queue_clear` | `channel_web_queue_clear` | Clear a thread's run queue. |
| `channel.web_queue_remove` | `channel_web_queue_remove` | Remove one queued item by `item_id`. |

## Boundaries

- The turn lifecycle belongs to TinyAgents (`turn_runner`), the agent loop, run
  queue, sessions and transcripts too (`vendor/tinyagents`). The wire types
  belong to `tinychannels-bus`; canonical output projection belongs to the
  compiled TinyChannels module and is called through TinyBus. The host stamps
  routing from its `DispatchCtx` and owns the process event bus. A change
  to either goes upstream first; this module keeps only the host ports.
- The Socket.IO transport and the `/events` SSE endpoint live in
  `openhuman-rpc` (`src/server/socketio.rs`, `src/server/http/events.rs`).
  This module only produces events.
- External provider adapters (Telegram, Slack, ...) live in `channels/`.
- Prompt-injection policy is `security::prompt_injection`; the approval gate
  and `ApprovalTurnContext` are `security::approval`.
- Failure copy per error class is `inference/failure_copy/`; provider
  resolution is `inference::provider`.
- Turn-state storage is `threads::turn_state`; the conversation store that the
  reply row lands in is `memory::conversations`.

## Gotchas

- The chat approval context is an explicit value on the turn
  (`ApprovalTurnContext`, carried by the turn's run context to the gate and to
  the tools that read it). A detached sub-agent deliberately starts without it
  (`OpenHumanRunContext::detached_child`), so its parks route through the
  origin and are marked detached (#5499). The wallet quote owner is taken from
  that context by the wallet and Web3 tools; a tool dispatched without a run
  context has none and cannot prepare or execute an owned quote.
- Ingress order is the runner's and is fixed: attachments, guardrail, approval
  reply, hook. Approval-reply routing runs before the `beforeSubmitPrompt` hook
  on purpose; reordering them lets a hook strand a turn waiting on an approval.
- A reply persistence error is non-fatal on the delivery path
  (`presentation::persist_reply` logs and carries on): the reply is announced
  anyway, since the client's own append is still a working fallback.
- Host-authored turns are not in the runner, so they neither interrupt nor get
  interrupted by user messages. Callers gate on idleness themselves.
- The terminal event comes last. `WebEvents::terminal` removes the request's
  state, so nothing may read a turn's slot after its terminal event.
- The module's output events carry the round of a `chat_interim` only
  through the host (`Surface::interim`): the canonical `Clarification` output
  has none.
- Debug/test hooks (`set_test_forced_run_chat_task_error`,
  `RUN_CHAT_TASK_TEST_LOCK`, `set_test_run_chat_task_block`,
  `TestRunChatTaskBlock`, `parallel_in_flight_entries_for_test`,
  `fresh_approval_surface_subscription`) are compiled only under
  `cfg(any(test, debug_assertions))` or `cfg(debug_assertions)`.
  `in_flight_entries_for_test` is exported unconditionally because
  `test_support/introspect.rs` uses it. A turn parked or failed by a hook skips
  session checkout.

## Tests

Tests live in sibling `*_tests.rs` files. [`web_tests.rs`](./web_tests.rs) and its
`web_tests_*_tests.rs` siblings cover `start_chat`, cancellation, queueing,
the turn lifecycle (supersede, stop, queued drop), session concurrency and
error classification end to end; [`mod_test_support_tests.rs`](./mod_test_support_tests.rs)
is the debug/test `test_support` module (`classify_error_for_test`). The rest
are per-file unit tests. The runner's own behaviour is tested upstream in
TinyAgents.

```bash
cargo test -p openhuman web_chat
pnpm debug rust web_chat
```

## Further reading

- [Chat](../../../../docs/gitbooks/en/features/chat.md)
- [Agent harness architecture](../../../../docs/gitbooks/en/developing/architecture/agent-harness.md)
- [Frontend architecture](../../../../docs/gitbooks/en/developing/architecture/frontend.md)

## Voice notes

`voice_notes.rs` stores the audio behind in-app voice notes under
`<workspace>/voice_notes/<note_id>.<ext>`; thread rows carry only
`extraMetadata.voiceNote = { noteId, mime, durationMs?, waveform? }`.

- `channel.web_voice_note_store` / `channel.web_voice_note_read` move audio between
  the composer/player and the store (base64 over JSON-RPC).
- A turn that is spoken (`speak_reply`, or `[channel_voice]` mode `always`, or
  `mirror` when the turn's `source` is `voice_note`) stores its synthesized reply as
  `reply-<request_id>`; `persist_delivered_reply` then stamps `voiceNote` on the
  reply row, so the reply is replayable after a reload.
