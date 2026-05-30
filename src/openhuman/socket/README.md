# socket

Persistent, Rust-native Socket.IO client to the OpenHuman backend. The `socket` domain owns a single long-lived `SocketManager` that speaks Engine.IO v4 + Socket.IO v4 directly over a WebSocket (`tokio-tungstenite` + `rustls`), maintains the connection with exponential-backoff reconnection, and routes inbound server events onto the in-process event bus for domain-specific handling (webhooks, channel messages, Composio triggers, device tunnel). Outbound `emit`s and connection lifecycle are exposed over JSON-RPC under the `socket` namespace.

## Responsibilities

- Open and maintain a persistent Socket.IO connection to the backend over a raw WebSocket; perform the Engine.IO OPEN and Socket.IO CONNECT handshakes by hand.
- Authenticate the SIO CONNECT with a JWT and reconnect automatically with exponential backoff (1 s → 30 s cap).
- Follow HTTP 3xx redirects during the upgrade (up to 3 hops) so a `http://`-configured `BACKEND_URL` behind a TLS-forcing edge connects cleanly; pin the resolved URL for subsequent reconnects and surface a one-shot "stale BACKEND_URL" warning for permanent redirects.
- Refresh the session token before every reconnect via a `TokenProvider` callback (live re-read of the profile store) instead of caching a single token — fixes the "Invalid token" retry storm (#2892 / TAURI-RUST-9C).
- Fast-fail on a definitively dead token (server "Invalid token" + no fresher token available) rather than burning the whole backoff budget.
- Track connection status / socket id / last user-visible error in shared state and expose it via RPC.
- Parse inbound Socket.IO EVENT frames and publish them as `DomainEvent`s for other domains to consume; emit outbound events.
- Suppress reconnect-storm Sentry noise: route only the 5th consecutive failure through the observability classifier (which demotes offline/transport shapes to a breadcrumb), keep all other retries at `warn`.

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/socket/mod.rs` | Module docstring + exports only. Re-exports `SocketManager`, `global_socket_manager`, `set_global_socket_manager`, and the `all_socket_controller_schemas` / `all_socket_registered_controllers` pair. |
| `src/openhuman/socket/manager.rs` | `SocketManager` handle + `SharedState`; global `OnceLock` accessor; `connect` / `connect_with_provider` / `disconnect` / `emit` / `get_state`; spawns the background `ws_loop`. Holds emit/shutdown channels and the loop join handle. |
| `src/openhuman/socket/ws_loop.rs` | The background reconnection loop and a single connection attempt: Engine.IO/Socket.IO handshake, redirect-following connect, ping-timeout deadline, backoff, invalid-token decision logic, failure-escalation logging. |
| `src/openhuman/socket/event_handlers.rs` | Inbound SIO event dispatch (`handle_sio_event`), SIO frame parsing (`parse_sio_event`), outbound frame helper (`emit_via_channel`). Maps event names → `DomainEvent` publishes. Redacts payload content from logs. |
| `src/openhuman/socket/token_provider.rs` | `TokenProvider` type alias + `static_token_provider`, `token_provider_from_config`, and `is_invalid_token_error` (strict double-anchor matcher). |
| `src/openhuman/socket/schemas.rs` | Controller schemas + RPC handlers for the `socket` namespace. |
| `src/openhuman/socket/types.rs` | `WsStream` alias, `ConnectionOutcome` enum, observability event-name constants; re-exports `ConnectionStatus` / `SocketState` from `crate::api::models::socket`. |
| `src/openhuman/socket/ws_loop_tests.rs` | Out-of-line test suite for `ws_loop.rs` (via `#[path = ...]`). |

## Public surface

- `SocketManager` — the connection handle. Key methods: `new`, `connect(url, token)`, `connect_with_provider(url, provider)`, `disconnect`, `emit(event, data)`, `get_state() -> SocketState`, `is_connected`, `set_webhook_router` / `webhook_router`.
- `global_socket_manager() -> Option<&'static Arc<SocketManager>>` and `set_global_socket_manager(Arc<SocketManager>)` — the process-global singleton (set once at bootstrap).
- `all_socket_controller_schemas` / `all_socket_registered_controllers` — controller-registry exports wired into `src/core/all.rs`.

Internal-only (`pub(crate)` / `pub(super)`): `TokenProvider` and its builders, `SharedState`, `ConnectionOutcome`, `WsStream`, the `ws_loop` and event-handler helpers.

## RPC / controllers

Namespace `socket` (called as `openhuman.socket_<function>`):

| Function | Inputs | Output | Notes |
| --- | --- | --- | --- |
| `connect` | `url` (str, req), `token` (str, req) | `status` | Connect with an explicit static token. |
| `disconnect` | — | `status` | Tear down the loop. |
| `state` | — | `state` (JSON: status, socket_id, error) | |
| `emit` | `event` (str, req), `data` (JSON, opt) | `ok` (bool) | Emit a SIO event to the backend. |
| `connect_with_session` | — | `status` | Loads config, derives the API URL, and connects with a **live-refresh** token provider reading the stored session token from the profile store on every reconnect. |

All handlers go through `require_manager()` and error with `"SocketManager not initialized"` if the global singleton is unset.

## Events

`event_handlers::handle_sio_event` is a thin transport router — it does not run domain logic itself. It mutates connection status for `ready`/`error` and publishes the following `DomainEvent`s via `publish_global` for other domains' bus subscribers:

| Inbound SIO event | Published `DomainEvent` | Consumer domain |
| --- | --- | --- |
| `webhook:request` | `WebhookIncomingRequest { request, raw_data }` | webhooks (also emits a `webhook:response` 400 on parse failure) |
| `composio:trigger` | `ComposioTriggerReceived { toolkit, trigger, metadata_id, metadata_uuid, payload }` | composio |
| `tunnel:peer-status` | `DevicePeerOnline` / `DevicePeerOffline` | devices |
| `tunnel:frame` | `DeviceTunnelFrame` | devices |
| `tunnel:registered` | `DeviceTunnelRegistered` | devices |
| `tunnel:evicted` | `DevicePeerOffline` | devices |
| `*:message` (suffix match) | `ChannelInboundMessage { event_name, channel, message, sender, reply_target, thread_ts, raw_data }` | channels |

This module is a **publisher only** — it owns no `bus.rs` / `EventHandler` impls.

## Persistence

None of its own. State (`status`, `socket_id`, `error`, attached `WebhookRouter`) lives in-memory in `SharedState` behind `parking_lot::RwLock`. The session token is read on demand from the profile store via `crate::api::jwt::get_session_token` (live-refresh path); there is no `store.rs`.

## Dependencies

- `crate::api::models::socket` — `ConnectionStatus`, `SocketState` DTOs.
- `crate::api::socket::websocket_url`, `crate::api::config::effective_backend_api_url`, `crate::api::jwt::get_session_token` — URL derivation and session-token lookup.
- `crate::core::all` — `ControllerFuture`, `RegisteredController` for the controller registry.
- `crate::core::{ControllerSchema, FieldSchema, TypeSchema}` — RPC schema types.
- `crate::core::event_bus` — `publish_global` / `DomainEvent` for routing inbound events.
- `crate::core::observability::report_error_or_expected` — one-shot sustained-outage classification at the failure threshold.
- `crate::openhuman::webhooks` — `WebhookRouter` (attached for parse-error logging / response emission) and `WebhookRequest`.
- `crate::openhuman::composio` — `ComposioTriggerEvent` DTO for `composio:trigger` deserialization.
- `crate::openhuman::devices::tunnel_client` — `TunnelPeerStatus`, `TunnelFrame` DTOs for tunnel events.
- `crate::openhuman::config` — `Config` + `rpc::load_config_with_timeout` for `connect_with_session`.
- `crate::openhuman::util::utf8_safe_prefix_at_byte_boundary` — UTF-8-safe log truncation of raw packets.

## Used by

- `src/core/all.rs` — registers the socket controllers.
- `src/core/jsonrpc.rs`, `src/core/observability.rs` — reference the socket namespace/state.
- `src/openhuman/connectivity/rpc.rs` — connectivity/status surfacing.
- `src/openhuman/webhooks/{ops.rs,bus.rs}` — emit webhook responses back through the global manager.
- `src/openhuman/devices/tunnel_client.rs` — emits tunnel frames/registration over the socket.

## Notes / gotchas

- **Event-name constants in `types.rs` (`runtime:socket-state-changed`, `server:event`) are grep-anchored** — the frontend subscribes to those exact strings; a rename silently breaks the Tauri event bridge (locked by a test).
- **Payload content is never logged** at any level — webhook bodies / channel messages / Composio payloads can carry PII, secrets, or tokens. Only byte-length and structural shape are logged. This also dodged a UTF-8 char-boundary panic that used to slice raw payloads at byte 500 (OPENHUMAN-TAURI-KC / #1814).
- `connect` rejects an empty/whitespace token immediately rather than spawning a doomed retry loop; `connect_with_provider` does the same eager pre-check via the provider.
- The reconnect loop bounds "fresh-token immediate retry" to **one** per cycle so a provider that returns a different non-empty token every call cannot hot-loop without sleeping or escalating (CodeRabbit Major, #2905).
- A genuinely fresh token from the invalid-token decision is **carried forward** (`pending_token`) so the next iteration uses the exact validated value, avoiding a redundant profile-store lock/disk read.
- Sentry escalation fires exactly once, on the 5th consecutive failure (~15 s of accumulated backoff), and is routed through the observability classifier so offline/transport shapes demote to a breadcrumb (OPENHUMAN-TAURI-8M / -BH).
- Redirect following only persists the "update BACKEND_URL" warning for permanent redirects (301/308); temporary (302/307) hops don't (CodeRabbit, #1547).
- No agent tools and no `bus.rs` — this is a transport domain that publishes events for others to handle.
