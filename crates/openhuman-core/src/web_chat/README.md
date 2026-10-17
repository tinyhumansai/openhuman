# Web chat

The turn runner for the in-app chat (desktop and web). It takes a raw user
message, runs it through the agent harness on the thread's own session, and
ends with a reply that is stored durably and announced to the client exactly
once. It owns the `channel.web_*` RPC namespace and the business logic behind
the Socket.IO `chat:start` and `chat:cancel` handlers.

External messaging providers (Telegram, Discord and so on) live in
`channels/`, but their inbound messages are dispatched through this module's
`start_chat` as well (`channels/bus/subscriber.rs`). So this is the single
turn runner behind both surfaces, and its `WebChannelEvent` broadcast bus is
what Socket.IO, the JSON-RPC `/events` SSE stream, the TUI and the channel
subscriber all listen to.

## How it works

### The request path

```text
 Socket.IO chat:start         channel.web_chat RPC      channels/ subscriber
 (openhuman-rpc socketio.rs)  (schemas.rs)              threads edit/regenerate
            \                      |                        /
             v                     v                       v
        +-----------------------------------------------------+
        | start_chat (ops/start_chat.rs)                      |
        |  attachments -> prompt guard -> approval reply?     |
        |  -> beforeSubmitPrompt hook -> queue mode           |
        +-----------------------------------------------------+
             | Interrupt (default)          | Steer/Followup/Collect -> RunQueue
             | Parallel -> spawn_parallel_turn (ops/parallel_turn.rs)
             v
        tokio::spawn( run_turn_under_cancel_and_deadline(        )
                        cancel token + wall-clock backstop
                        + WebChat origin + chat approval context
                        run_chat_task (run_task.rs) )
             |
             |   checkout_session_agent (session.rs)
             |   spawn_progress_bridge  (progress_bridge.rs) ---> WebChannelEvent
             |   agent.run_single_with_origin                     (deltas, tools,
             |   checkin_session_agent                             heartbeat, ...)
             v
     Ok(reply) --> presentation::deliver_response
                     persist_reply (reply_persistence.rs) -> conversation store
                     announce_reply -> one chat_done
                     spawn_follow_up_suggestions -> chat_suggestions (later)
     Err(e)    --> web_errors::classify_inference_error -> one chat_error
```

1. A `chat:start` socket event arrives in `openhuman-rpc/src/server/socketio.rs`
   (or a `channel.web_chat` RPC call, or an inbound provider message) and
   calls `start_chat` with the raw message, the thread and client ids, and any
   model, temperature, locale or queue-mode overrides.

2. `start_chat` stages `[FILE:...]` and `[IMAGE:...]` markers through
   `agent::attachments::stage` first. A multi-megabyte base64 blob must never
   reach prompt-injection scanning or persistence. It then runs
   `security::prompt_injection::enforce_prompt_input`; a block comes back as
   `StartChatError::Guardrail` with a verdict, score and reasons so the
   frontend can classify on `chat_error.error_type == "guardrail"` instead of
   matching copy. Next it checks whether the thread has a parked chat-native
   approval and the message parses as an approval reply
   (`security::approval::parse_approval_reply`); if so the reply goes to the
   approval gate and no new turn starts. Only after that does a configured
   `beforeSubmitPrompt` hook see the message, so a bare "yes" answering an
   approval can never be blocked by a hook.

3. The queue mode decides what happens to a turn already in flight on the
   thread. `QueueMode` ([`types.rs`](./types.rs)) has five values. `Interrupt` (the default)
   cancels the current turn and starts this one. `Steer`, `Followup` and
   `Collect` map to TinyAgents run-queue lanes (`QueueMode::queue_lane`) and
   are pushed onto the running turn's `RunQueue` instead of starting a turn;
   with nothing in flight they start a normal turn.
   `Parallel` starts an isolated fork alongside whatever is running
   ([`ops/parallel_turn.rs`](./ops/parallel_turn.rs)), tracked in its own `parallel_in_flight()` table
   keyed by request id so it never touches interrupt or queue semantics.

4. For a primary turn, `start_chat` records an `InFlightEntry` in `in_flight()`
   and spawns a task (under `CoreContext::propagate`) that drives
   `run_task::run_chat_task` through `run_turn_under_cancel_and_deadline`
   ([`ops/turn_guards.rs`](./ops/turn_guards.rs)). That wrapper puts four things around the same
   future: a cooperative `CancellationToken`, the wall-clock backstop, the
   `AgentTurnOrigin::WebChat` scope, and the turn's `ApprovalTurnContext`
   (handed to `run_chat_task`, which gives it to the session turn).

5. `run_chat_task` checks the session agent out of the per-thread cache
   (`session::checkout_session_agent`). The entry is removed from
   `thread_sessions()` for the duration of the turn so two turns can never
   drive one agent. It is reused when its `SessionCacheFingerprint` still
   matches; otherwise a new agent is built. A new agent needs no history
   seeding: `set_thread_id` binds the session's durable identity
   (`SessionRef`) and the turn resumes the thread's one transcript by it. The
   task then spawns the progress bridge, awaits
   `agent.run_single_with_origin`, and checks the agent back in with
   `checkin_session_agent` unless the turn poisoned it. A fork never takes or
   returns the cached agent.

6. While the turn runs, `spawn_progress_bridge` turns each `AgentProgress`
   event into `WebChannelEvent`s (`text_delta`, `thinking_delta`,
   `tool_call`, `tool_result`, `chat_interim`, `turn_cost`, sub-agent events
   and so on), mirrors turn state into the `TurnStates` port,
   and emits an `inference_heartbeat` every `INFERENCE_HEARTBEAT_SECS` (20 s)
   so a long silent prefill does not trip the frontend's roughly 120 s
   silence timeout (#4270). Sub-agent arms live in
   [`progress_bridge_subagent_events.rs`](./progress_bridge_subagent_events.rs); time to first visible output is
   stamped by [`turn_timing.rs`](./turn_timing.rs); at the end of a turn
   [`journal_shadow.rs`](./journal_shadow.rs) compares the live trace spans with the spans
   reprojected from the durable journal and logs what differs.

7. On success the spawned task calls `presentation::deliver_response`. It
   first writes the reply to the thread's conversation store
   (`persist_reply`, which uses `reply_persistence::persist_delivered_reply`)
   under the deterministic id `run_reply_message_id(request_id)`, so the
   answer survives a client reconnect or reload (#6034) and the client's own
   append collapses onto the same row. Then `announce_reply` emits exactly one
   `chat_done` carrying the model's unmodified text. For the main single-user
   turn it also spawns `suggestions::spawn_follow_up_suggestions`, a cheap
   `summarization`-role call that may emit `chat_suggestions` after
   `chat_done`. It never delays the reply and is gated on
   `web_chat.suggestions_enabled`.

8. On failure, `run_chat_task` first consults the per-thread budget signal
   ([`ops/budget_correlation.rs`](./ops/budget_correlation.rs)). An empty 200 on the same provider binding as
   a recent budget-exhausted failure is reclassified as out-of-credits (#3386),
   because the managed route closes the stream cleanly when credits run out.
   The spawned task then runs the error string through
   `web_errors::classify_inference_error` and emits one `chat_error`, and
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
([`ops/system_turn.rs`](./ops/system_turn.rs)) instead of `start_chat`. They skip ingress, `in_flight()`
and the progress bridge, run with `SYSTEM_CLIENT_ID` ("system"), and return
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

### Cancellation and the backstop

`cancel_chat` and `cancel_chat_scoped` ([`ops/channel_ops.rs`](./ops/channel_ops.rs)) cancel by
thread, or by request id when one is given. A stale cancel for a superseded
request is ignored (`cancel_should_target`) so the newer turn survives. Without
a request id, a cancel also stops the thread's parallel forks and detached
background sub-agents. `state::cancel_in_flight_gracefully` cancels the token
first and only hard-aborts the task if it has not unwound after a short grace
period.

Two guards make sure a turn always ends in a terminal event instead of an
endless heartbeat stream (#4746):

- The **idle guard** ([`turn_activity.rs`](./turn_activity.rs)) ends a turn
  that has emitted no progress event, with no model or tool call in flight, for
  900 s (`OPENHUMAN_WEB_TURN_IDLE_SECS`, `0` disables it). `run_chat_task` taps
  the turn's progress stream into a per-turn activity record on its way to the
  bridge. Every event restarts the clock, and an open model call
  (`IterationStarted` → `ModelCallCompleted`) or tool call (`ToolCallStarted`
  → `ToolCallCompleted`) holds it, so a silent provider retry or a long shell
  build is never idle. What it catches is silence nothing else bounds: session
  checkout, the persistence plumbing, or a harness that stopped with no call
  open.
- The **wall-clock backstop** is the fixed total ceiling, 3600 s by default
  (`agent::turn_deadline`, `OPENHUMAN_WEB_TURN_TIMEOUT_SECS`, `0` disables it
  and the derived deadlines). The harness winds the turn down at 3480 s and
  writes the grounded close, and its own `max_wall_clock_ms` is clamped to
  3540 s, so a busy turn still delivers an answer before the backstop drops it.

The backstop used to be 900 s and did both jobs, so a turn that was working
steadily was wound down at 780 s like a wedged one.

### The event bus

[`event_bus.rs`](./event_bus.rs) holds an in-process `tokio::sync::broadcast` channel of
`WebChannelEvent` (the type is defined in [`channel_event.rs`](./channel_event.rs)). Any domain can
publish to it with `publish_web_channel_event`; consumers call
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
| [`ops.rs`](./ops.rs) | Thin shell over [`ops/`](./ops/); re-exports the request surface and state. |
| [`ops/start_chat.rs`](./ops/start_chat.rs) | `start_chat` and `StartChatError`: ingress, guardrail, approval-reply routing, queue dispatch, and the spawned turn body that delivers or classifies. |
| [`ops/channel_ops.rs`](./ops/channel_ops.rs) | `cancel_chat`, `cancel_chat_scoped`, and the `channel_web_*` RPC handlers. |
| [`ops/parallel_turn.rs`](./ops/parallel_turn.rs) | Spawns and cancels `QueueMode::Parallel` forks. |
| [`ops/system_turn.rs`](./ops/system_turn.rs) | `run_system_turn_on_thread` for host-authored turns. |
| [`ops/state.rs`](./ops/state.rs) | `thread_sessions()`, `in_flight()`, `parallel_in_flight()`, keying helpers, `invalidate_thread_sessions`, `cancel_should_target`. Each table is a slot of the ambient agent context, so two embedded agents never share an entry; outside an agent context it is the process default. |
| [`ops/turn_guards.rs`](./ops/turn_guards.rs) | `run_turn_under_cancel_and_deadline`, the wall-clock backstop, Sentry suppression and timeout tagging. |
| [`ops/budget_correlation.rs`](./ops/budget_correlation.rs) | `thread_budget_signals()` and `classify_budget_correlation` for the empty-200-after-budget case. |
| [`ops/test_hooks.rs`](./ops/test_hooks.rs) | Debug/test hooks that force or block `run_chat_task`. |
| [`run_task.rs`](./run_task.rs) | `run_chat_task`: checkout, progress bridge, run, budget correlation on error, checkin. |
| [`session.rs`](./session.rs) | Session checkout and checkin, fingerprinting, `CheckoutPolicy`, target agent id, locale reply directive, per-turn provider and model routing. |
| [`progress_bridge.rs`](./progress_bridge.rs) | `spawn_progress_bridge`: `AgentProgress` to `WebChannelEvent`, turn-state mirror, heartbeat. |
| [`progress_bridge_subagent_events.rs`](./progress_bridge_subagent_events.rs) | The bridge's `AgentProgress::Subagent*` handlers. |
| [`turn_timing.rs`](./turn_timing.rs) | Time to first visible output, and rate limiting for live `turn_cost` events. |
| [`journal_shadow.rs`](./journal_shadow.rs) | Compares live trace spans with spans reprojected from the journal and logs divergences. |
| [`presentation.rs`](./presentation.rs) | `deliver_response` (persist, then one `chat_done`), `deliver_response_single_bubble` for core-initiated turns, and legacy `chat_segment` helpers. |
| [`reply_persistence.rs`](./reply_persistence.rs) | `persist_delivered_reply`: durable reply row under the deterministic reply id. |
| [`suggestions.rs`](./suggestions.rs) | Post-turn follow-up suggestions (`chat_suggestions`). |
| [`channel_event.rs`](./channel_event.rs) | `WebChannelEvent` and its payload types (`TurnUsagePayload`, `GuardrailPayload`, `QueueItemPayload`, `ChatSuggestion`, ...). |
| [`event_bus.rs`](./event_bus.rs) | The broadcast channel and the `DomainEvent` surface subscribers. |
| [`egress_surface.rs`](./egress_surface.rs) | The `external_transfer_pending` bridge. |
| [`web_errors.rs`](./web_errors.rs) | Thin shell over [`web_errors/`](./web_errors/): `classify.rs` (the classification ladder and `ClassifiedError`), `backend_error_code.rs`, `budget.rs`, `retry.rs`, `timeout.rs`. The class to copy table is in `inference/failure_copy/`. |
| [`schemas.rs`](./schemas.rs) | Controller schemas and thin handlers for the `channel` namespace. |
| `types.rs` | `QueueMode`, `SessionEntry`, `SessionCacheFingerprint`, `InFlightEntry`, `ParallelEntry`, `WebChatTaskResult`, `ChatRequestMetadata`, RPC param structs. |

## Key types and entry points

- `start_chat` ([`ops/start_chat.rs`](./ops/start_chat.rs)) is the entry point for every user turn.
  It returns the new request id, or a `StartChatError`.
- `run_system_turn_on_thread` (`ops/system_turn.rs`) runs a host-authored turn
  on a thread's own session and returns the reply text.
- `cancel_chat` / `cancel_chat_scoped` (`ops/channel_ops.rs`) stop a turn by
  thread or request id.
- `invalidate_thread_sessions` ([`ops/state.rs`](./ops/state.rs)) drops a thread's cached agent.
  Thread edit and regenerate (`threads/ops/edit.rs`) and channel remote
  control (`channels/host/remote_control.rs`) call it.
- `ChatRequestMetadata` (`types.rs`) is per-request metadata (`speak_reply`,
  `source`, `session_id`, `agent_id` for trace attribution) passed by every
  caller of `start_chat` and `spawn_progress_bridge`.
- `WebChannelEvent` (`channel_event.rs`) is the event payload sent to clients.
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

- The agent loop, run queue, sessions and transcripts belong to TinyAgents
  (`vendor/tinyagents`); this module calls `run_single_with_origin` and uses
  `run_queue::{RunQueue, QueueLane}`. Segmentation helpers come from
  `tinychannels_bus::delivery` (`vendor/tinychannels`).
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
- Approval-reply routing runs before the `beforeSubmitPrompt` hook on
  purpose. Reordering them lets a hook strand a turn waiting on an approval.
- Attachment staging must stay ahead of prompt scanning and persistence.
- A reply persistence error is non-fatal: the reply is announced anyway, since
  the client's own append is still a working fallback.
- Host-authored turns are not in `in_flight()`, so they neither interrupt nor
  get interrupted by user messages. Callers gate on idleness themselves.
- Debug/test hooks (`set_test_forced_run_chat_task_error`,
  `RUN_CHAT_TASK_TEST_LOCK`, `set_test_run_chat_task_block`,
  `TestRunChatTaskBlock`, `parallel_in_flight_entries_for_test`,
  `fresh_approval_surface_subscription`) are compiled only under
  `cfg(any(test, debug_assertions))` or `cfg(debug_assertions)`.
  `in_flight_entries_for_test` is exported unconditionally because
  `test_support/introspect.rs` uses it.

## Tests

Tests live in sibling `*_tests.rs` files. [`web_tests.rs`](./web_tests.rs) and its
`web_tests_*_tests.rs` siblings cover `start_chat`, cancellation, queueing,
session concurrency and error classification end to end;
[`mod_test_support_tests.rs`](./mod_test_support_tests.rs) is the debug/test `test_support` module
(`classify_error_for_test`). The rest are per-file unit tests.

```bash
cargo test -p openhuman web_chat
pnpm debug rust web_chat
```

## Further reading

- [Chat](../../../../docs/gitbooks/en/features/chat.md)
- [Agent harness architecture](../../../../docs/gitbooks/en/developing/architecture/agent-harness.md)
- [Frontend architecture](../../../../docs/gitbooks/en/developing/architecture/frontend.md)
