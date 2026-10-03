# Web chat

The web/desktop channel's turn runner. Owns the `channel.web_*` RPC namespace,
the Socket.IO `chat:start`/`chat:cancel` handlers' business logic, and the
whole request lifecycle from a raw message to a delivered, durably stored
reply. `channels/` owns the external messaging providers (Telegram, WhatsApp,
and so on); their inbound messages are dispatched through this module's
`start_chat` too (`channels/bus/subscriber.rs`), so this is the single turn
runner behind both surfaces.

## Request lifecycle

1. `openhuman-rpc/src/server/socketio.rs` receives a `chat:start` socket event and calls
   [`start_chat`] (`ops/start_chat.rs`) with the raw message, thread/client ids,
   and any model/profile/locale/queue-mode overrides.
2. `start_chat` preprocesses `[FILE:...]`/`[IMAGE:...]` attachment markers
   before prompt-injection scanning or persistence (a multi-MB base64 blob
   must never reach those stages), runs `enforce_prompt_input`, and checks
   for a parked chat-native approval reply before treating the message as a
   new turn. It then applies the queue mode (`QueueMode::Interrupt` default,
   `Steer`, `Followup`, `Collect`, `Parallel`) against the thread's
   `InFlightEntry`/`ParallelEntry` (`types.rs`).
3. It spawns a tokio task that runs `run_task::run_chat_task` through
   `run_turn_under_cancel_and_deadline` (`ops/turn_guards.rs`): a cooperative
   `CancellationToken`, the wall-clock backstop (`web_turn_deadline`), the
   `AgentTurnOrigin::WebChat` scope, and the
   `APPROVAL_CHAT_CONTEXT` task-local scope all wrap the same future.
4. `run_chat_task` checks the session `Agent` out of the per-thread cache
   (`session.rs::checkout_session_agent`), reusing the `THREAD_SESSIONS` entry
   when its `SessionCacheFingerprint` still matches and otherwise building one
   and cold-boot resuming it from the thread's `session_raw` transcript (or
   the conversation log). A `Parallel` fork always builds a fresh agent and
   never touches the cache. It then spawns [`spawn_progress_bridge`]
   (`progress_bridge.rs`), awaits `agent.run_single`, and checks the agent
   back in (`checkin_session_agent`) unless the turn poisoned it. The bridge
   forwards `AgentProgress` into `WebChannelEvent` socket events, mirrors turn
   state into `crate::threads::turn_state::TurnStateStore`, and emits an
   `inference_heartbeat` beat every `INFERENCE_HEARTBEAT_SECS` (20s) so a long
   silent prefill doesn't trip the frontend's ~120s silence timeout (#4270).
5. On `Ok`, the spawned task calls `presentation::deliver_response`, which
   first calls `reply_persistence::persist_delivered_reply` to write the reply
   to the thread's conversation store under the deterministic
   `run_reply_message_id(request_id)` (so the answer survives a client
   reconnect or reload, #6034) and then emits exactly one `chat_done` carrying
   the model's unmodified text. The segmentation helpers in `presentation.rs`
   are legacy and unused on this path.
6. On `Err`, `run_chat_task`'s error branch first consults the per-thread
   budget signal (`THREAD_BUDGET_SIGNALS`, `classify_budget_correlation` maps
   to a `BudgetCorrelation`) so an empty 200 on the same provider binding as a
   recent budget-exhausted failure is reclassified as out-of-credits (#3386);
   the spawned task then normalizes the error string through
   `web_errors::classify_inference_error` into the user-facing `chat_error`
   (budget-exhausted, non-retryable rate limit, fallback-chain-exhausted, turn
   timeout, and so on) and decides via `sentry_suppression_reason` whether it
   pages. The event carries `message` (finished English copy, unchanged for
   older UIs, the CLI, the TUI and embedders) plus `copy_key`
   (`chat_error.<class>`, one per row of `inference/failure_copy/table.rs`) and
   `copy_params` (`retry_after_secs`, `provider`, `detail`); the app renders
   the key in the user's locale (`app/src/lib/chatErrorCopy.ts`) and falls back
   to `message` for an unknown or missing key. Loop-guard halt summaries are
   not `chat_error` events (they become the turn's reply text), so they carry
   no key.

Host-authored turns, meaning background-delivery notices
(`agent::orchestration::background_delivery`) and goal continuations
(`agent::goals::continuation`), enter through `run_system_turn_on_thread`
(`ops/system_turn.rs`) instead of `start_chat`. They skip ingress, `IN_FLIGHT`
and the progress bridge but go through the same session checkout, so the
model sees the conversation and the turn lands in the thread's transcript. A
turn run on a throwaway host bound to the thread wrote a competing root
transcript that the next cold-boot resume preferred, dropping every earlier
turn. Such a turn checks out with `CheckoutPolicy::AdoptCached` (reuse the
thread's agent under whatever settings the user's last turn chose, rather than
rebuilding on a fingerprint miss) and checks in with
`checkin_session_agent_if_vacant`: a user turn that started meanwhile and
re-cached its own agent wins.

## Public surface

- Event bus (`event_bus.rs`): `subscribe_web_channel_events`,
  `publish_web_channel_event`, `approval_request_event`,
  `register_approval_surface_subscriber`, `register_artifact_surface_subscriber`,
  `register_egress_surface_subscriber`. These bridge `DomainEvent`s onto the
  in-process `WebChannelEvent` broadcast bus consumed by both Socket.IO and
  the JSON-RPC `/events` SSE stream.
- Operations (`ops.rs`, a thin shell over the `ops/` submodule: `start_chat.rs`,
  `channel_ops.rs`, `parallel_turn.rs`, `turn_guards.rs`, `state.rs`,
  `budget_correlation.rs`, `system_turn.rs`, `test_hooks.rs`): `start_chat`,
  `run_system_turn_on_thread` (+ `SYSTEM_CLIENT_ID`,
  `SESSION_CHECKOUT_FAILURE`), `cancel_chat`,
  `cancel_chat_scoped`, `cancel_should_target`, `channel_web_chat`,
  `channel_web_cancel`, `channel_web_queue_status`, `channel_web_queue_clear`,
  `invalidate_thread_sessions`, plus `in_flight_entries_for_test` (exported
  unconditionally; `test_support/introspect.rs` uses it). The turn's
  in-flight/session state lives here (`THREAD_SESSIONS`,
  `THREAD_BUDGET_SIGNALS`, `IN_FLIGHT`, `PARALLEL_IN_FLIGHT`).
- `ChatRequestMetadata` (`types.rs`): per-request metadata passed by every
  caller of `start_chat`/`spawn_progress_bridge`.
- Schemas (`schemas.rs`): `all_web_channel_controller_schemas`,
  `all_web_channel_registered_controllers`, `schemas`. This is the RPC contract
  for the `channel` namespace.
- Debug/test-only hooks: `set_test_forced_run_chat_task_error`,
  `RUN_CHAT_TASK_TEST_LOCK`, `set_test_run_chat_task_block`,
  `TestRunChatTaskBlock`, `parallel_in_flight_entries_for_test`
  (`#[cfg(any(test, debug_assertions))]`) and
  `fresh_approval_surface_subscription` (`#[cfg(debug_assertions)]`), so none
  of them link into a release binary.

## Files

| File | Purpose |
| --- | --- |
| `mod.rs` | Module wiring and re-exports; no business logic |
| `ops.rs` (thin shell over `ops/`: `start_chat.rs`, `channel_ops.rs`, `parallel_turn.rs`, `turn_guards.rs`, `state.rs`, `budget_correlation.rs`, `test_hooks.rs`) | `start_chat`/`cancel_*`/`channel_web_*` operations, session cache, in-flight tracking, budget-signal correlation, `run_turn_under_cancel_and_deadline` |
| `run_task.rs` | `run_chat_task`: resolves/builds the session agent, spawns the progress bridge, runs the turn, applies the budget correlation to its error |
| `session.rs` | Builds/fingerprints the cached session `Agent`, resolves target agent id, locale directive, provider role for a model override |
| `progress_bridge.rs` | Forwards `AgentProgress` into `WebChannelEvent`s and `TurnStateMirror`, emits the `inference_heartbeat` liveness beat |
| `presentation.rs` | `deliver_response` (one unsegmented `chat_done`, persisted first) and `deliver_response_single_bubble` (core-initiated turns); local-model emoji-reaction decision; legacy segmentation helpers |
| `reply_persistence.rs` | Durable write of the reply about to be announced, under a deterministic id shared with the client's own append |
| `event_bus.rs` | The `WebChannelEvent` broadcast channel plus approval/artifact/egress `DomainEvent` surface subscribers |
| `web_errors.rs` (thin shell over `web_errors/`: `backend_error_code.rs`, `budget.rs`, `classify.rs`, `provider_detail.rs`, `retry.rs`, `timeout.rs`; the class -> copy table is `inference/failure_copy/`) | Classifies raw provider error strings into user-facing copy; budget-exhausted / rate-limit / fallback-exhausted / timeout detection |
| `schemas.rs` | `ControllerSchema`/`RegisteredController` definitions for the `channel.web_*` RPC functions |
| `types.rs` | `SessionEntry`, `SessionCacheFingerprint`, `InFlightEntry`, `ParallelEntry`, `WebChatTaskResult`, `ChatRequestMetadata`, `WebChatParams` |

## RPC

Namespace `channel`, registered via
`all_web_channel_registered_controllers()` in `core/all.rs`:

| Function | Handler |
| --- | --- |
| `web_chat` | `channel_web_chat` |
| `web_cancel` | `channel_web_cancel` |
| `web_queue_status` | `channel_web_queue_status` |
| `web_queue_clear` | `channel_web_queue_clear` |

## Events

- Broadcasts `WebChannelEvent` (defined in `openhuman-rpc/src/server/socketio.rs`) over an
  in-process `tokio::sync::broadcast` channel. `openhuman-rpc/src/server/socketio.rs` forwards it
  to the connected Socket.IO client; `core/jsonrpc/http/events.rs` subscribes to that stream and
  serves the JSON-RPC `/events` SSE endpoint; `channels/bus/subscriber.rs`
  subscribes to collect the reply for an inbound provider message.
- Subscribes to `DomainEvent` on `crate::core::bus::BUS` via three
  process-lifetime, `OnceLock`-guarded subscribers registered at startup from
  `core/runtime/bootstrap.rs` and `channels/runtime/startup/start_channels.rs`:
  `register_approval_surface_subscriber`
  (maps `ApprovalRequested`/`PlanReviewRequested` to `approval_request` /
  `plan_review_request`), `register_artifact_surface_subscriber`
  (maps `ArtifactPending`/`ArtifactReady`/`ArtifactFailed` to `artifact_*`), and
  `register_egress_surface_subscriber` (maps `ExternalTransferPending` to
  `external_transfer_pending`, only when the transfer carries chat routing).

## Calls into

- `crate::agent::harness`: `Agent::from_config_for_agent`,
  `run_queue::{RunQueue, QueueMode}`, and the tool-calling loop itself.
- `crate::threads::turn_state::{TurnStateStore, TurnStateMirror}`: the
  progress bridge mirrors turn state here for cross-surface visibility.
- `crate::inference::provider::provider_for_role`: resolves the provider
  binding for `provider_role_for_model_override`, which feeds the session
  fingerprint.
- `crate::security::approval::APPROVAL_CHAT_CONTEXT`: scoped by
  `run_turn_under_cancel_and_deadline` around the `run_chat_task` future.
  `crate::web3::wallet::execution::current_owner()` relies on this task-local
  staying scoped through the inline `.await` chain in `run_chat_task`;
  detaching the tool loop onto a fresh `tokio::spawn` without re-scoping it
  would silently disable the quote-owner gate (see `web3/wallet/README.md`).
- `crate::memory::conversations`: `reply_persistence` appends the durable
  reply row; `crate::memory::agent::memory_loader::MemoryCitation` carries
  citations through to that row's metadata.

## Called by

- `openhuman-rpc/src/server/socketio.rs`: the `chat:start` and `chat:cancel` handlers call
  `start_chat` / `cancel_chat_scoped`, and forward `WebChannelEvent`s to the
  client.
- `core/jsonrpc/http/events.rs`: subscribes to the `/events` SSE stream and forwards web-channel events; `core/runtime/bootstrap.rs` registers the `DomainEvent` surface subscribers.
- `core/all.rs`: registers `all_web_channel_registered_controllers()` under
  `DomainGroup::Channels`, deliberately not behind the `channels` feature
  (the in-app chat is core product surface, #5002).
- `channels/bus/subscriber.rs`: inbound external-provider messages run through
  `start_chat` with a per-sender `client_id`;
  `channels/providers/telegram/remote_control.rs` calls
  `invalidate_thread_sessions`.
- `flows/ops/streaming.rs` and
  `agent/orchestration/background_delivery.rs`: core-initiated turns reuse
  `spawn_progress_bridge` and `presentation::deliver_response*` so they render
  on the same socket surface.
- `publish_web_channel_event` is also called from `cron`, `voice`,
  and `channels/proactive.rs` for surface-level
  notifications.

## Tests

- `web_tests.rs` (+ `web_tests_error_code_classification_tests.rs`,
  `web_tests_rate_limit_classification_tests.rs`,
  `web_tests_session_and_concurrency_tests.rs`,
  `web_tests_start_chat_ingress_tests.rs`): end-to-end coverage
  of `start_chat`/`cancel_*`/queue behavior.
- `mod_test_support_tests.rs`: the `test_support` module
  (`classify_error_for_test`, `ClassifiedErrorSnapshot`) exposed through
  `mod.rs` for debug/test builds.
- Per-file unit tests: `event_bus_tests.rs`, `ops_budget_correlation_tests_tests.rs`,
  `presentation_tests.rs` + `presentation_test_support_tests.rs`,
  `progress_bridge_tests.rs`, `reply_persistence_tests.rs`, `run_task_tests.rs`.
