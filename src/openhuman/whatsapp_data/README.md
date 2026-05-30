# whatsapp_data

Local-only, structured persistence and agent-facing query API for WhatsApp Web data. The Tauri `whatsapp_scanner` scrapes chats and messages from WhatsApp Web via CDP and pushes them into this domain over an internal JSON-RPC write path; the data lands in a dedicated on-device SQLite database (`whatsapp_data.db`) and is exposed to the agent through read-only RPC controllers and LLM-callable tools. **All data stays on-device — nothing is transmitted to any external service.**

## Responsibilities

- Persist scraped WhatsApp chats (`wa_chats`) and messages (`wa_messages`) in a local SQLite DB.
- Upsert scanner snapshots idempotently (ingest path), refreshing per-chat aggregates (`message_count`, `last_message_ts`) after writes.
- Auto-prune messages older than 90 days on every ingest, then refresh affected chat stats.
- Serve read-only queries to the agent: list chats, list messages (with time-range + pagination), and substring search over message bodies + sender names.
- Keep the write (ingest) path off the agent controller registry — only the scanner may write.
- Tolerate concurrent SQLite contention via a write mutex + busy-handler + application-level retry with backoff.

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/whatsapp_data/mod.rs` | Module docstring + exports; re-exports the schema/controller entry points. |
| `src/openhuman/whatsapp_data/types.rs` | Serde domain types: `WhatsAppChat`, `WhatsAppMessage`, ingest/list/search request + result structs. |
| `src/openhuman/whatsapp_data/store.rs` | `WhatsAppDataStore` — SQLite persistence: schema init, upsert chats/messages, prune, list, search. |
| `src/openhuman/whatsapp_data/ops.rs` | Business logic over `&WhatsAppDataStore`: `ingest` (with 90-day prune), `list_chats`, `list_messages`, `search_messages`. |
| `src/openhuman/whatsapp_data/rpc.rs` | Async RPC handlers wrapping ops in `RpcOutcome<T>`, acquiring the global store. |
| `src/openhuman/whatsapp_data/schemas.rs` | Controller schemas + `handle_*` dispatch; splits agent-facing (read-only) vs internal (ingest) controller sets. |
| `src/openhuman/whatsapp_data/global.rs` | Process-global `WhatsAppDataStore` singleton (`init` / `store` / `store_if_ready` / `reset_for_tests`). |
| `src/openhuman/whatsapp_data/sqlite_retry.rs` | `is_sqlite_busy` detection + `retry_on_sqlite_busy` backoff helper; `BUSY_TIMEOUT` constant. |
| `src/openhuman/whatsapp_data/tools.rs` | Re-exports the three LLM-callable read-only tool structs from `tools/`. |
| `src/openhuman/whatsapp_data/tools/list_chats.rs` | `WhatsAppDataListChatsTool` — `Tool` impl over the list_chats RPC. |
| `src/openhuman/whatsapp_data/tools/list_messages.rs` | `WhatsAppDataListMessagesTool`. |
| `src/openhuman/whatsapp_data/tools/search_messages.rs` | `WhatsAppDataSearchMessagesTool`. |
| `src/openhuman/whatsapp_data/store_tests.rs` | Sibling test suite for the store. |
| `src/openhuman/whatsapp_data/schemas_tests.rs` | Sibling test suite for the schemas. |

## Public surface

From `mod.rs` re-exports:
- `all_whatsapp_data_controller_schemas()` — read-only schemas advertised to the agent.
- `all_whatsapp_data_registered_controllers()` — read-only registered controllers.
- `all_whatsapp_data_internal_controllers()` — read-only controllers **plus** the internal `ingest` handler.

Other notable public items: `global::{init, store, store_if_ready}`, `store::WhatsAppDataStore`, the `types::*` records, `tools::{WhatsAppDataListChatsTool, WhatsAppDataListMessagesTool, WhatsAppDataSearchMessagesTool}`, and the `rpc::whatsapp_data_*` handler functions.

## RPC / controllers

Namespace `whatsapp_data` (method form `openhuman.whatsapp_data_<function>`):

| Method | Exposure | Purpose |
| --- | --- | --- |
| `openhuman.whatsapp_data_list_chats` | Agent (read) | List chats ordered by `last_message_ts` DESC; optional `account_id`, `limit` (50), `offset`. |
| `openhuman.whatsapp_data_list_messages` | Agent (read) | List messages for a `chat_id` ordered by timestamp ASC; optional `account_id`, `since_ts`, `until_ts`, `limit` (100), `offset`. |
| `openhuman.whatsapp_data_search_messages` | Agent (read) | Case-insensitive `LIKE` substring search over `body` **and** `sender`; optional `chat_id`, `account_id`, `limit` (20). |
| `openhuman.whatsapp_data_ingest` | Internal (write) | Scanner-only upsert + prune. Deliberately excluded from agent schemas/registry; wired via `all_internal_controllers()` in `src/core/all.rs`. |

Handlers return a "not connected"-style error when the global store has not been initialised yet, instead of panicking.

## Agent tools

Three read-only tools (issue #1341), re-exported into `src/openhuman/tools/mod.rs`. Each is a thin shim over the corresponding read RPC handler, unwraps the `RpcOutcome`, and emits a compact JSON object tagged `"provider": "whatsapp"` for provenance/citation. All are `PermissionLevel::ReadOnly`, `ToolScope::All`, and concurrency-safe:

- `whatsapp_data_list_chats`
- `whatsapp_data_list_messages`
- `whatsapp_data_search_messages`

There is intentionally **no** tool wrapping `whatsapp_data_ingest` — preserving the read-only boundary is load-bearing (noted in `tools.rs`).

## Persistence

SQLite DB at `<workspace_dir>/whatsapp_data/whatsapp_data.db` (WAL mode, `foreign_keys = ON`). Two tables:

- `wa_chats` — PK `(account_id, chat_id)`; columns include `display_name`, `is_group`, `last_message_ts`, `message_count`, `updated_at`. `is_group` is derived from `@g.us` suffix on upsert.
- `wa_messages` — PK `(account_id, chat_id, message_id)`; columns include `sender`, `sender_jid`, `from_me`, `body`, `timestamp`, `message_type`, `source` (`cdp-dom` / `cdp-indexeddb`), `ingested_at`. Indexed on `(account_id, chat_id, timestamp)` and `(account_id, body)`.

Writes (upsert/prune) are serialized by an in-process `Mutex<()>` write lock and protected by `retry_on_sqlite_busy`. Non-text messages (stickers/images/system events) are persisted so chat aggregates don't skew toward text-only rows.

## Dependencies

- `crate::core::all` — `ControllerFuture`, `RegisteredController` (controller registration).
- `crate::core::{ControllerSchema, FieldSchema, TypeSchema}` — schema definitions.
- `crate::rpc::RpcOutcome` — RPC return envelope.
- `crate::openhuman::tools::traits` — `Tool`, `ToolResult`, `PermissionLevel`, `ToolScope` for the agent tools.
- External crates: `rusqlite` (SQLite), `anyhow`, `serde` / `serde_json`, `async_trait`.

## Used by

- `src/core/all.rs` — registers the read-only controllers and the internal ingest controller; routes the `whatsapp_data` namespace.
- `src/openhuman/tools/mod.rs` — re-exports the three agent tools into the global tool surface.
- `src/core/jsonrpc.rs` — calls `whatsapp_data::global::init(workspace_dir)` during the core boot sequence (alongside `memory::global`).
- `src/openhuman/subconscious/store.rs` — references `whatsapp_data::sqlite_retry` as the pattern model for its own busy/retry logic.

## Notes / gotchas

- The global store uses `RwLock<Option<…>>` (not `OnceLock`) specifically so tests can swap workspaces via `reset_for_tests()`. `init` is idempotent and race-resolving in production. `reset_for_tests` is compiled only under `cfg(any(test, debug_assertions))` and must never be called at runtime — it would release the SQLite connection mid-call.
- `search_messages` matches both `body` and `sender` so person-name queries ("what did Alice say") surface even though the sender name rarely appears in message bodies. `%`/`_` in the query are escaped (`ESCAPE '\'`); an empty/whitespace query returns no rows.
- Ingest auto-prunes messages older than 90 days (`PRUNE_HORIZON_SECS`) every call; pruning only touches rows with `timestamp > 0`.
- RPC error logging uses `{e:#}` (full anyhow chain) deliberately — plain `{e}` dropped the underlying SQLite cause before it reached Sentry (see `rpc.rs` comment referencing OPENHUMAN-TAURI-6B / TAURI-B2).
- The `ingest` write path is kept off the agent registry by design so an agent cannot mutate/poison the local store; only the scanner calls it over JSON-RPC.
