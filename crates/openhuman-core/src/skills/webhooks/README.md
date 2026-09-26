# webhooks

Client-side webhook **tunnel routing** for OpenHuman. The backend provisions and hosts the actual tunnels (ngrok / cloudflare / etc.) and forwards incoming HTTP requests to the app over Socket.IO; this module maps each backend tunnel UUID to its owning target (a skill, the built-in echo responder, or the agent triage pipeline), dispatches incoming requests, builds responses, captures debug logs, and exposes the local routing RPCs. The backend tunnel-management RPCs (`webhooks.{list,create,get,update,delete}_tunnel`, `webhooks.get_bandwidth`) share this namespace but live in `crates/openhuman-tinyhumans/src/hosted/webhooks/` on the TinyHumans SDK; they are registered only when `openhuman_tinyhumans::install` runs.

`webhooks` is nested under `skills/` for historical reasons but is **not** gated by the `skills` Cargo feature — it has always-compiled callers in `crates/openhuman-core/src/core/` and stays outside the `skills` feature gate (see `crates/openhuman-core/src/skills/mod.rs`).

## Responsibilities

- Maintain an in-memory, ownership-enforced map of `tunnel_uuid → TunnelRegistration` (`WebhookRouter`), persisted to disk as JSON.
- Enforce isolation: a skill can only register/unregister/list its own tunnels; cross-skill takeover and silent agent rebinds are rejected.
- Route incoming `WebhookIncomingRequest` events to the correct target by `target_kind` (`echo`, `agent`, or `skill`) and emit the response back over the socket (`webhook:response`).
- Build echo responses; route `agent` tunnels into the agent triage pipeline (spawned, non-blocking, returns `202 Accepted`).
- Capture per-request debug logs (request + response + lifecycle stage) in a bounded ring buffer for developer tooling, and broadcast debug events.
- Expose local routing RPCs (registrations, logs, echo/agent registration, manual triage trigger).

## Key files

| File | Role |
| --- | --- |
| `crates/openhuman-core/src/skills/webhooks/mod.rs` | Export-only: module docstring, `pub mod` decls, re-exports of `WebhookRouter`, types, and the `all_webhooks_*` controller pair. |
| `crates/openhuman-core/src/skills/webhooks/types.rs` | Serde domain types: `WebhookRequest`, `WebhookResponseData`, `TunnelRegistration`, `WebhookActivityEntry`, `WebhookDebugLogEntry`, debug result wrappers, `WebhookDebugEvent`. |
| `crates/openhuman-core/src/skills/webhooks/router.rs` | `WebhookRouter` — route map + ownership rules, disk persistence (generation-counter, spawn_blocking offload), bounded debug log ring (`MAX_DEBUG_LOG_ENTRIES = 250`), debug-event broadcast channel. |
| `crates/openhuman-core/src/skills/webhooks/ops.rs` | RPC handler logic returning `RpcOutcome<T>`: local routing ops (`list_registrations`, `list_logs`, `clear_logs`, `register_echo`, `unregister_echo`, `register_agent`, `trigger_agent`), and `build_echo_response`. |
| `crates/openhuman-core/src/skills/webhooks/schemas.rs` | Controller schemas + `handle_*` fns + `all_controller_schemas` / `all_registered_controllers`; deserializes params, delegates to `ops.rs`. |
| `crates/openhuman-core/src/skills/webhooks/bus.rs` | `WebhookRequestSubscriber` (`EventHandler`) — the incoming-request routing flow; helpers `decode_webhook_body`, `run_agent_trigger`, `build_agent_response`. |
| `crates/openhuman-core/src/skills/webhooks/{webhooks_tests,bus_tests,ops_tests,router_tests,schemas_tests,types_tests}.rs` | Test suites, each pulled into its sibling source file via `#[cfg(test)] #[path = "..."] mod tests;` (no inline test modules). |

## Public surface

Re-exported from `mod.rs`:

- `WebhookRouter` (from `router`).
- `all_webhooks_controller_schemas`, `all_webhooks_registered_controllers` (from `schemas`).
- Types: `TunnelRegistration`, `WebhookActivityEntry`, `WebhookDebugEvent`, `WebhookDebugLogEntry`, `WebhookDebugLogListResult`, `WebhookDebugLogsClearedResult`, `WebhookDebugRegistrationsResult`, `WebhookRequest`, `WebhookResponseData`.

Key `WebhookRouter` methods: `new(persist_path)`, `register` / `register_echo` / `register_agent`, `unregister`, `unregister_skill`, `route`, `registration`, `list_for_skill`, `list_all`, `record_request` / `record_parse_error` / `record_response`, `list_logs`, `clear_logs`, `subscribe_debug_events`. `ops::build_echo_response` is also public for the bus.

## RPC / controllers

Registered via `all_webhooks_registered_controllers()` (wired in `crates/openhuman-core/src/core/all.rs`). Methods (`webhooks` namespace):

| Method | Backing | Purpose |
| --- | --- | --- |
| `webhooks.list_registrations` | local router | All in-app tunnel registrations. |
| `webhooks.list_logs` | local router | Captured request/response debug logs (`limit`). |
| `webhooks.clear_logs` | local router | Clear debug logs; returns count cleared. |
| `webhooks.register_echo` | local router | Register echo target for a `tunnel_uuid`. |
| `webhooks.unregister_echo` | local router | Remove echo target. |
| `webhooks.register_agent` | local router | Register an agent-backed tunnel (routes to triage). |
| `webhooks.trigger_agent` | triage | Fire triage/agent pipeline directly (source `webhook`/`cron`/`external`); 60s timeouts on triage + apply. |

`list_registrations`, `list_logs` and `clear_logs` return empty results when the router/socket manager isn't initialized; the `register_*`/`unregister_*` ops return an error in that case.

## Agent tools

None. This domain owns no `tools.rs` agent tools.

## Events

Subscriber (in `bus.rs`): **`WebhookRequestSubscriber`** — `name() = "webhook::request_handler"`, `domains() = ["webhook"]`. Registered in `register_domain_subscribers()` (`crates/openhuman-core/src/core/jsonrpc.rs`, called from `bootstrap_core_runtime()`), gated on the `Skills` domain group being enabled (`plan.skills`) and installed at most once per process; `channels/runtime/startup/start_channels.rs` deliberately does not register it, to avoid double-registration when both startup paths run in the same process.

- **Subscribes**: `DomainEvent::WebhookIncomingRequest` (published by the socket transport in `socket/event_handlers.rs`).
- **Publishes**: `DomainEvent::WebhookRegistered` / `WebhookUnregistered` (from the router on registration changes — `WebhookUnregistered`, the `registration_changed` debug event and the route re-persist all fire **only when a registration was actually removed**; unregistering an absent tunnel is a silent no-op that returns `Ok(false)`, see #6091), `DomainEvent::WebhookReceived` (when routed to a target), `DomainEvent::WebhookProcessed` (always, with status/elapsed/error).

Routing outcomes by `target_kind`: `echo` → `build_echo_response` (200); `agent` → decode body, spawn triage, return `202 Accepted` (spawned task emits the real response later, with 60s timeout → 504); `skill` / unknown → `501` (direct skill dispatch not available); no registration → `404`. Responses are emitted over the socket as `webhook:response`.

The router also runs a separate `tokio::sync::broadcast` channel of `WebhookDebugEvent` (`registration_changed`, `log_updated`, `logs_cleared`) consumed via `subscribe_debug_events()` for frontend/dev tooling.

## Persistence

`WebhookRouter` serializes its registrations to a JSON file (`PersistedRoutes`) at the optional `persist_path` passed to `new()`, and reloads them from that file when constructed. Writes are best-effort and fire-and-forget: offloaded to `spawn_blocking` inside a tokio runtime (inline otherwise), guarded by a monotonic generation counter so stale writes under rapid churn are dropped. Debug logs are **not** persisted — they live only in an in-memory `VecDeque` capped at 250 entries.

Note that nothing in the production startup path currently constructs a `WebhookRouter` or calls `SocketManager::set_webhook_router`; the only caller is `tests/raw_coverage/webhooks_ingress_e2e.rs`. Until a router is attached, the local RPCs return empty results (see above) and the subscriber answers every incoming request with `404`.

## Dependencies

- `crate::core::bus::BUS` and `crate::core::events::DomainEvent` — publishing and subscribing; the `EventHandler` trait comes from `tinybus`.
- `crate::core::all` — `ControllerFuture`, `RegisteredController` for controller registration.
- `crate::core::{ControllerSchema, FieldSchema, TypeSchema}` — RPC schema types.
- `crate::core::observability::report_error` — error reporting for body-decode / agent-trigger failures.
- `crate::platform::socket::global_socket_manager` — obtain the `WebhookRouter` (stored on the socket manager) and `emit` responses over the socket.
- `crate::agent::triage` — `TriggerEnvelope`, `run_triage`, `apply_decision`, `TriageOutcome` for agent-tunnel routing and `trigger_agent`.
- `crate::config::{Config, rpc::load_config_with_timeout}` — config for backend-proxy RPCs.
- `crate::rpc::RpcOutcome` — handler return contract.

## Used by

- `crates/openhuman-core/src/core/all.rs` — registers the controllers/schemas into the RPC registry.
- `crates/openhuman-core/src/platform/socket/manager.rs` — stores the `WebhookRouter` (`set_webhook_router` / `webhook_router`) on the socket manager; ops/bus retrieve it from there.
- `crates/openhuman-core/src/platform/socket/event_handlers.rs` — publishes `WebhookIncomingRequest` from the socket and reads the shared router slot.
- `crates/openhuman-core/src/core/jsonrpc.rs` — registers `WebhookRequestSubscriber` in `register_domain_subscribers()` and provides the RPC transport surface.
- `crates/openhuman-core/src/core/events.rs` — defines the `Webhook*` `DomainEvent` variants this module uses.

## Notes / gotchas

- **Direct skill dispatch is not implemented**: a `skill`-kind tunnel returns `501`. Real handling exists only for `echo` and `agent` kinds (the QuickJS skill runtime was removed; see `gitbooks/developing/architecture.md`).
- `route()` deliberately resolves only `target_kind == "skill"` registrations; echo/agent registrations are matched via `registration()` in the bus, not `route()`.
- Agent tunnels respond `202` immediately and complete asynchronously — the spawned task emits the final `webhook:response` itself to avoid blocking the broadcast dispatch task during LLM calls.
- `register_agent` stores `agent_id` for observability and rebind validation only; per the docstring the triage evaluator selects the target agent dynamically regardless of the pinned value.
- Persistence is fire-and-forget and may not flush before process exit; a lost write only replays the most recent registration change on next startup.
- `decode_webhook_body` returns `{}` for empty bodies and wraps non-JSON-but-valid-UTF-8 bodies under a `"raw"` key; invalid base64 is a hard error (→ 400).
- All request/response bodies are base64-encoded over the wire (`WebhookRequest.body` / `WebhookResponseData.body`).
