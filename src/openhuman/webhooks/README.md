# webhooks

Client-side webhook **tunnel routing** for OpenHuman. The backend provisions and hosts the actual tunnels (ngrok / cloudflare / etc.) and forwards incoming HTTP requests to the app over Socket.IO; this module maps each backend tunnel UUID to its owning target (a skill, the built-in echo responder, or the agent triage pipeline), dispatches incoming requests, builds responses, captures debug logs, and exposes both local routing RPCs and thin proxies to the backend's tunnel-management API.

## Responsibilities

- Maintain an in-memory, ownership-enforced map of `tunnel_uuid → TunnelRegistration` (`WebhookRouter`), persisted to disk as JSON.
- Enforce isolation: a skill can only register/unregister/list its own tunnels; cross-skill takeover and silent agent rebinds are rejected.
- Route incoming `WebhookIncomingRequest` events to the correct target by `target_kind` (`echo`, `agent`, or `skill`) and emit the response back over the socket (`webhook:response`).
- Build echo responses; route `agent` tunnels into the agent triage pipeline (spawned, non-blocking, returns `202 Accepted`).
- Capture per-request debug logs (request + response + lifecycle stage) in a bounded ring buffer for developer tooling, and broadcast debug events.
- Expose local routing RPCs (registrations, logs, echo/agent registration, manual triage trigger) and proxy RPCs to the backend tunnel CRUD + bandwidth API.

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/webhooks/mod.rs` | Export-only: module docstring, `pub mod` decls, re-exports of `WebhookRouter`, types, and the `all_webhooks_*` controller pair. |
| `src/openhuman/webhooks/types.rs` | Serde domain types: `WebhookRequest`, `WebhookResponseData`, `TunnelRegistration`, `WebhookActivityEntry`, `WebhookDebugLogEntry`, debug result wrappers, `WebhookDebugEvent`. Inline tests. |
| `src/openhuman/webhooks/router.rs` | `WebhookRouter` — route map + ownership rules, disk persistence (generation-counter, spawn_blocking offload), bounded debug log ring (`MAX_DEBUG_LOG_ENTRIES = 250`), debug-event broadcast channel. |
| `src/openhuman/webhooks/ops.rs` | RPC handler logic returning `RpcOutcome<T>`: local routing ops (`list_registrations`, `list_logs`, `clear_logs`, `register_echo`, `unregister_echo`, `register_agent`, `trigger_agent`), `build_echo_response`, and backend-proxied tunnel CRUD (`list/create/get/update/delete_tunnel`, `get_bandwidth`). |
| `src/openhuman/webhooks/schemas.rs` | Controller schemas + `handle_*` fns + `all_controller_schemas` / `all_registered_controllers`; deserializes params, delegates to `ops.rs`. |
| `src/openhuman/webhooks/bus.rs` | `WebhookRequestSubscriber` (`EventHandler`) — the incoming-request routing flow; helpers `decode_webhook_body`, `run_agent_trigger`, `build_agent_response`. Inline tests. |
| `src/openhuman/webhooks/{tests,ops_tests,router_tests,schemas_tests}.rs` | Test suites (module-level + per-file via `#[path]`). |

## Public surface

Re-exported from `mod.rs`:

- `WebhookRouter` (from `router`).
- `all_webhooks_controller_schemas`, `all_webhooks_registered_controllers` (from `schemas`).
- Types: `TunnelRegistration`, `WebhookActivityEntry`, `WebhookDebugEvent`, `WebhookDebugLogEntry`, `WebhookDebugLogListResult`, `WebhookDebugLogsClearedResult`, `WebhookDebugRegistrationsResult`, `WebhookRequest`, `WebhookResponseData`.

Key `WebhookRouter` methods: `new(persist_path)`, `register` / `register_echo` / `register_agent`, `unregister`, `unregister_skill`, `route`, `registration`, `list_for_skill`, `list_all`, `record_request` / `record_parse_error` / `record_response`, `list_logs`, `clear_logs`, `subscribe_debug_events`. `ops::build_echo_response` is also public for the bus.

## RPC / controllers

Registered via `all_webhooks_registered_controllers()` (wired in `src/core/all.rs`). Methods (`webhooks` namespace):

| Method | Backing | Purpose |
| --- | --- | --- |
| `webhooks.list_registrations` | local router | All in-app tunnel registrations. |
| `webhooks.list_logs` | local router | Captured request/response debug logs (`limit`). |
| `webhooks.clear_logs` | local router | Clear debug logs; returns count cleared. |
| `webhooks.register_echo` | local router | Register echo target for a `tunnel_uuid`. |
| `webhooks.unregister_echo` | local router | Remove echo target. |
| `webhooks.register_agent` | local router | Register an agent-backed tunnel (routes to triage). |
| `webhooks.trigger_agent` | triage | Fire triage/agent pipeline directly (source `webhook`/`cron`/`external`); 60s timeouts on triage + apply. |
| `webhooks.list_tunnels` | backend proxy | `GET /webhooks/core`. |
| `webhooks.create_tunnel` | backend proxy | `POST /webhooks/core`. |
| `webhooks.get_tunnel` | backend proxy | `GET /webhooks/core/{id}`. |
| `webhooks.update_tunnel` | backend proxy | `PATCH /webhooks/core/{id}`. |
| `webhooks.delete_tunnel` | backend proxy | `DELETE /webhooks/core/{id}`. |
| `webhooks.get_bandwidth` | backend proxy | `GET /webhooks/core/bandwidth`. |

Backend-proxy methods require a stored session token (`get_session_token`) and call the backend via `BackendOAuthClient`. Local router methods degrade gracefully to empty results when the router/socket manager isn't initialized rather than erroring.

## Agent tools

None. This domain owns no `tools.rs` agent tools.

## Events

Subscriber (in `bus.rs`): **`WebhookRequestSubscriber`** — `name() = "webhook::request_handler"`, `domains() = ["webhook"]`. Registered at startup (see `channels/runtime/startup.rs`).

- **Subscribes**: `DomainEvent::WebhookIncomingRequest` (published by the socket transport in `socket/event_handlers.rs`).
- **Publishes**: `DomainEvent::WebhookRegistered` / `WebhookUnregistered` (from the router on registration changes), `DomainEvent::WebhookReceived` (when routed to a target), `DomainEvent::WebhookProcessed` (always, with status/elapsed/error).

Routing outcomes by `target_kind`: `echo` → `build_echo_response` (200); `agent` → decode body, spawn triage, return `202 Accepted` (spawned task emits the real response later, with 60s timeout → 504); `skill` / unknown → `501` (direct skill dispatch not available); no registration → `404`. Responses are emitted over the socket as `webhook:response`.

The router also runs a separate `tokio::sync::broadcast` channel of `WebhookDebugEvent` (`registration_changed`, `log_updated`, `logs_cleared`) consumed via `subscribe_debug_events()` for frontend/dev tooling.

## Persistence

`WebhookRouter` serializes its registrations to a JSON file (`PersistedRoutes`) at the `persist_path` passed to `new()` (e.g. `~/.openhuman/webhook_routes.json`). Writes are best-effort and fire-and-forget: offloaded to `spawn_blocking` inside a tokio runtime (inline otherwise), guarded by a monotonic generation counter so stale writes under rapid churn are dropped. Routes are reloaded from this file on startup. Debug logs are **not** persisted — they live only in an in-memory `VecDeque` capped at 250 entries.

## Dependencies

- `crate::core::event_bus` — `publish_global`, `DomainEvent`, `EventHandler` for the subscriber and registration events.
- `crate::core::all` — `ControllerFuture`, `RegisteredController` for controller registration.
- `crate::core::{ControllerSchema, FieldSchema, TypeSchema}` — RPC schema types.
- `crate::core::observability::report_error` — error reporting for body-decode / agent-trigger failures.
- `crate::openhuman::socket::global_socket_manager` — obtain the `WebhookRouter` (stored on the socket manager) and `emit` responses over the socket.
- `crate::openhuman::agent::triage` — `TriggerEnvelope`, `run_triage`, `apply_decision`, `TriageOutcome` for agent-tunnel routing and `trigger_agent`.
- `crate::openhuman::config::{Config, rpc::load_config_with_timeout}` — config for backend-proxy RPCs.
- `crate::api::{BackendOAuthClient, config::effective_backend_api_url, jwt::get_session_token}` — authenticated backend tunnel CRUD/bandwidth calls.
- `crate::rpc::RpcOutcome` — handler return contract.

## Used by

- `src/core/all.rs` — registers the controllers/schemas into the RPC registry.
- `src/openhuman/socket/manager.rs` — stores the `WebhookRouter` (`set_webhook_router` / `webhook_router`) on the socket manager; ops/bus retrieve it from there.
- `src/openhuman/socket/event_handlers.rs` — publishes `WebhookIncomingRequest` from the socket and reads the shared router slot.
- `src/openhuman/channels/runtime/startup.rs` — registers `WebhookRequestSubscriber` at startup.
- `src/core/event_bus/events.rs` — defines the `Webhook*` `DomainEvent` variants this module uses.
- `src/core/jsonrpc.rs` — RPC transport surface.

## Notes / gotchas

- **Direct skill dispatch is not implemented**: a `skill`-kind tunnel returns `501`. Real handling exists only for `echo` and `agent` kinds (the skills QuickJS runtime was removed; see CLAUDE.md).
- `route()` deliberately resolves only `target_kind == "skill"` registrations; echo/agent registrations are matched via `registration()` in the bus, not `route()`.
- Agent tunnels respond `202` immediately and complete asynchronously — the spawned task emits the final `webhook:response` itself to avoid blocking the broadcast dispatch task during LLM calls.
- `register_agent` stores `agent_id` for observability and rebind validation only; per the docstring the triage evaluator selects the target agent dynamically regardless of the pinned value.
- Persistence is fire-and-forget and may not flush before process exit; a lost write only replays the most recent registration change on next startup.
- `decode_webhook_body` returns `{}` for empty bodies and wraps non-JSON-but-valid-UTF-8 bodies under a `"raw"` key; invalid base64 is a hard error (→ 400).
- All request/response bodies are base64-encoded over the wire (`WebhookRequest.body` / `WebhookResponseData.body`).
