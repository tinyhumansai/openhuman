# webview_apis

Core-side **client half** of the webview-APIs bridge. It exposes typed `openhuman.webview_apis_*` JSON-RPC/CLI methods that proxy to the Tauri host over a loopback WebSocket, so the live-webview connectors (currently Gmail) are reachable from curl and the agent without the shell-only Tauri IPC channel. The actual scraping/automation runs natively (CDP) inside the Tauri shell's mirror module (`app/src-tauri/src/webview_apis/`); this module only validates params, forwards a request envelope, and decodes the typed response.

## Responsibilities

- Register `webview_apis` JSON-RPC controllers (Gmail labels/messages/search/get/send/trash/add-label) into the core controller registry.
- Validate incoming params (`account_id`, `limit`, `message_id`, `request`, …) before dispatching, with a tightened `u32` numeric guard for `limit`.
- Maintain one lazy, long-lived WebSocket connection to the Tauri host's local bridge server; multiplex requests by generated id and resolve responses via a pending `oneshot` map.
- Reconnect transparently on connection drop and fail in-flight requests so callers never hang; bound each request to a 15s timeout.
- Surface an actionable error when the bridge port env var is missing (i.e. the shell isn't running / didn't spawn core).
- Keep Gmail wire types wire-compatible with the Tauri-side `app/src-tauri/src/gmail/types.rs`.

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/webview_apis/mod.rs` | Export-only. Re-exports schema registry fns (`all_webview_apis_controller_schemas`, `all_webview_apis_registered_controllers`, `webview_apis_schemas`) and the Gmail wire types. |
| `src/openhuman/webview_apis/types.rs` | Plain serde wire types mirroring the bridge's Gmail shapes: `GmailLabel`, `GmailMessage`, `GmailSendRequest`, `SendAck`, `Ack`. No domain logic. |
| `src/openhuman/webview_apis/schemas.rs` | Registry-only. `ControllerSchema`s + `all_controller_schemas` / `all_registered_controllers`; maps each function to its `rpc.rs` handler. |
| `src/openhuman/webview_apis/rpc.rs` | Handler bodies (`handle_gmail_*`). Validate params, call `client::request(<bridge method>, params)`, wrap in `RpcOutcome`. Param helpers `require_string` / `require_u32` / `read_required` live here. |
| `src/openhuman/webview_apis/client.rs` | Lazy `OnceLock` WebSocket client to `ws://127.0.0.1:$OPENHUMAN_WEBVIEW_APIS_PORT/`. Request/response envelope multiplexing, writer + reader tokio tasks, reconnect, 15s timeout. |

## Public surface

- `all_webview_apis_controller_schemas()` / `all_webview_apis_registered_controllers()` — wired into `src/core/all.rs`.
- `webview_apis_schemas(function: &str) -> ControllerSchema`.
- Types: `Ack`, `GmailLabel`, `GmailMessage`, `GmailSendRequest`, `SendAck`.
- `client::request<T>(method, params)` (module `pub`) and `client::PORT_ENV` (`OPENHUMAN_WEBVIEW_APIS_PORT`).

## RPC / controllers

Namespace `webview_apis`; methods exposed as `openhuman.webview_apis_<fn>`:

| Method | Bridge method | Inputs | Output |
| --- | --- | --- | --- |
| `gmail_list_labels` | `gmail.list_labels` | `account_id` | `Vec<GmailLabel>` |
| `gmail_list_messages` | `gmail.list_messages` | `account_id`, `limit`, `label?` | `Vec<GmailMessage>` |
| `gmail_search` | `gmail.search` | `account_id`, `query`, `limit` | `Vec<GmailMessage>` |
| `gmail_get_message` | `gmail.get_message` | `account_id`, `message_id` | `GmailMessage` |
| `gmail_send` | `gmail.send` | `account_id`, `request: GmailSendRequest` | `SendAck` |
| `gmail_trash` | `gmail.trash` | `account_id`, `message_id` | `Ack` |
| `gmail_add_label` | `gmail.add_label` | `account_id`, `message_id`, `label` | `Ack` |

## Agent tools

None owned directly. The controllers above are reachable to the agent via the generic JSON-RPC/CLI surface; this module has no `tools.rs`.

## Events

None. No `bus.rs`; no `DomainEvent` publish/subscribe.

## Persistence

None. No `store.rs`. The only state is the in-memory WebSocket client singleton (`OnceLock<Client>`); types are stateless wire shapes.

## Dependencies

- `crate::core::all::{ControllerFuture, RegisteredController}` — controller registry types/futures for handler signatures and registration.
- `crate::core::{ControllerSchema, FieldSchema, TypeSchema}` — schema description types.
- `crate::rpc::RpcOutcome` — response wrapping (`single_log`, `into_cli_compatible_json`).
- External crates: `tokio` (sync/time/spawn), `tokio-tungstenite` + `futures-util` (WebSocket), `serde` / `serde_json` (envelopes & wire types), `tracing` (diagnostics).

No dependency on other `openhuman/*` domains.

## Used by

- `src/core/all.rs` — extends the registered controllers (`:124`), the schema list (`:299`), and provides the `"webview_apis"` namespace description (`:472`).

## Notes / gotchas

- **Port discovery is mandatory.** The client reads `OPENHUMAN_WEBVIEW_APIS_PORT` (set by the Tauri host's `webview_apis::server::PORT_ENV` before spawning core). Missing var → every request fails with an explicit "shell must be running" error; this module cannot start the bridge itself.
- **Lazy + reconnecting.** First `request` opens the socket and spawns writer/reader tasks. On drop, the cached sender is cleared and all pending requests are failed (`"connection dropped"`), so the next request reconnects rather than hanging.
- **Coarse error surface by design** — every failure (timeout, connect, deserialize, `ok=false`) collapses to a single `String` so the JSON-RPC handler can propagate it verbatim.
- **`limit` is declared `TypeSchema::U64`** in the schema but the Tauri router casts to `u32`; `require_u32` rejects negatives, fractions, and u32 overflow up front to avoid confusing downstream errors.
- **Wire-compat contract**: `types.rs` must stay in sync with `app/src-tauri/src/gmail/types.rs`; the schemas describe them via `TypeSchema::Ref(...)`.
- Follows the canonical module shape: `schemas.rs` is registry-only and delegates handler bodies to `rpc.rs`.
