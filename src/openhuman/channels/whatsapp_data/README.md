# whatsapp_data (core: shared DTOs + agent tools)

Local-only, structured WhatsApp Web data for the agent. **The SQLite store, the
scanner ingest write path, and the list/search business logic live in the Tauri
shell** (`app/src-tauri/src/whatsapp_data/`) — the core keeps only the shared
serde DTOs and the three read-only agent query tools. **All data stays
on-device — nothing is transmitted to any external service.**

## Architecture (in-process bridge)

The core runs in-process inside the Tauri shell (sidecar removed). The agent
tools reach the shell-owned store over the **native request bus**
(`core::event_bus::{register_native_global, request_native_global}`) — a typed,
zero-serialization, in-process request/response registry keyed by a method
string. No HTTP, no JSON-RPC controller, no WebSocket loopback.

```
agent tool (core)                          shell store (app/src-tauri)
  request_native_global(method, req)  ──▶   register_native_global(method, handler)
    "whatsapp_data.list_chats"                 -> ops::list_chats(&store, req)
    "whatsapp_data.list_messages"              -> ops::list_messages(&store, req)
    "whatsapp_data.search_messages"            -> ops::search_messages(&store, req)
  scanner:
    "whatsapp_data.ingest"                     -> ops::ingest(&store, req)  (90-day prune)
```

Both sides share **one** DTO definition (`types.rs`), so the native-request
`TypeId` checks line up. Method-name constants live in `mod.rs` (`methods::*`).

**Graceful degradation.** In a headless / CLI / docker build there is no shell,
so no handler is registered. The tools then return an empty, well-formed result
with a `"note": "WhatsApp data unavailable (desktop only)"` rather than erroring.

## Key files (core)

| File | Role |
| --- | --- |
| `mod.rs` | Module docstring + `methods::*` native-request keys; re-exports `tools` + `types`. |
| `types.rs` | Shared serde DTOs: `WhatsAppChat`, `WhatsAppMessage`, ingest/list/search request + result structs. Consumed by both the core tools and the shell store. |
| `tools.rs` | Re-exports the three tools + the degradation helpers (`UNAVAILABLE_NOTE`, `is_handler_absent`). |
| `tools/list_chats.rs` | `WhatsAppDataListChatsTool` — dispatches `whatsapp_data.list_chats`. |
| `tools/list_messages.rs` | `WhatsAppDataListMessagesTool` — dispatches `whatsapp_data.list_messages`. |
| `tools/search_messages.rs` | `WhatsAppDataSearchMessagesTool` — dispatches `whatsapp_data.search_messages`. |

## Shell side (`app/src-tauri/src/whatsapp_data/`)

`store.rs` (SQLite persistence + corruption quarantine/rebuild), `ops.rs`
(ingest + 90-day prune + list/search), `sqlite_retry.rs` (busy/corrupt
detection + backoff), `global.rs` (process-global store singleton), and `mod.rs`
(native-handler registration + `ensure_store` + the `whatsapp_data_list_chats` /
`_list_messages` / `_search_messages` Tauri commands the frontend invokes). The
DB lives at `<workspace_dir>/whatsapp_data/whatsapp_data.db` — the same
workspace the core resolves, so it stays on the agent-write denylist
(`security::policy` `WORKSPACE_INTERNAL_DIRS`).
