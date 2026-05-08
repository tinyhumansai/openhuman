# WhatsApp data flow — scanner, store, agent

**Issue:** [#1341](https://github.com/tinyhumansai/openhuman/issues/1341)

This document describes how WhatsApp Web data captured by the desktop scanner becomes available to the agent. It exists to clear up the most common confusion: there are **two** local storage paths and they are intentional, not duplicates — each backs a different agent capability.

## Pipeline at a glance

```text
┌────────────────────────┐
│ WhatsApp Web (CEF view)│
└────────────┬───────────┘
             │  CDP scan tick
             ▼
┌────────────────────────────────────┐
│ app/src-tauri/src/whatsapp_scanner │
│ (DOM + IndexedDB merge)            │
└─────┬───────────────────────┬──────┘
      │ exact rows            │ canonicalised transcript
      ▼                       ▼
┌──────────────────────┐ ┌──────────────────────────┐
│ openhuman.whatsapp_  │ │ openhuman.memory_doc_    │
│ data_ingest          │ │ ingest                   │
│ (internal-only RPC)  │ │ (internal-only RPC)      │
└──────────┬───────────┘ └─────────────┬────────────┘
           ▼                           ▼
┌──────────────────────┐ ┌──────────────────────────┐
│ whatsapp_data.db     │ │ memory tree              │
│ (SQLite, per-account)│ │ (per-source summaries +  │
│  - wa_chats          │ │  embeddings)             │
│  - wa_messages       │ │                          │
└──────────┬───────────┘ └─────────────┬────────────┘
           ▼                           ▼
┌──────────────────────┐ ┌──────────────────────────┐
│ Agent tools          │ │ Agent tools              │
│  whatsapp_data_*     │ │  memory_tree_*           │
│  (exact lookup)      │ │  (semantic / cross-src)  │
└──────────────────────┘ └──────────────────────────┘
```

Both ingest endpoints fire on every scan tick; both are `tokio::spawn` fire-and-forget so the scanner never blocks on either HTTP call.

## Why two paths

| Path | Backing store | Strength | Use it for |
|------|---------------|----------|------------|
| **Direct** | `whatsapp_data.db` (SQLite) | Exact, structured, paginated | "List my WhatsApp chats", "show the last 50 messages with Alice", "search for `invoice` across WhatsApp" |
| **Memory tree** | Per-source memory tree + embeddings | Semantic, cross-source | "Summarise this week of WhatsApp", "find action items across email and WhatsApp", "what did the team agree on?" |

The same scan tick populates both stores. Idempotency keys make the dual-write safe to retry:

- `whatsapp_data_ingest` keys on `(account_id, chat_id, message_id)` — UPSERT.
- `memory_doc_ingest` keys on `(namespace, key)` where namespace is `whatsapp-web:<account_id>` and key is `<chat_id>:<day>` — also UPSERT.

If one path fails (network blip, store init race), the other still progresses. The next scan tick converges both stores.

## Read-only boundary

The scanner write-path RPCs are registered as **internal-only** in [`src/core/all.rs`](../src/core/all.rs) under `build_internal_only_controllers`. They are reachable over JSON-RPC but invisible to the agent's tool catalog and to schema discovery (`all_controller_schemas`). The agent has **no** way to call `whatsapp_data_ingest` or `memory_doc_ingest` — accidentally or otherwise.

The agent surfaces are exclusively read-only:

- [`src/openhuman/tools/impl/whatsapp_data/`](../src/openhuman/tools/impl/whatsapp_data/) — `whatsapp_data_list_chats`, `whatsapp_data_list_messages`, `whatsapp_data_search_messages`. All three wrap their RPC counterparts and emit a `"provider": "whatsapp"` tag in the response so the agent can cite WhatsApp as the source.
- [`src/openhuman/tools/impl/memory/tree/`](../src/openhuman/tools/impl/memory/tree/) — generic `memory_tree_*` tools. Filter by `source_kind: "chat"` or query directly; WhatsApp chat-day transcripts are tagged `whatsapp` so they surface in cross-source flows.

## Why the orchestrator only lists three of these

The orchestrator's `agent.toml` exposes the three direct WhatsApp tools alongside the generic `memory_tree_*` family. That choice is deliberate — adding more provider-specific tools would compete with the memory-tree tools for the same intents and fragment routing. The combination satisfies the three real shapes of WhatsApp request:

1. **Exact lookup** ("what was my last message with Bob") → `whatsapp_data_list_messages` after `whatsapp_data_list_chats`.
2. **Keyword search** ("did anyone mention `Q3` on WhatsApp") → `whatsapp_data_search_messages`.
3. **Summarisation / action items / cross-source** ("what came up across WhatsApp and email this week") → `memory_tree_query_source { source_kind: "chat" }` or `memory_tree_query_global`.

If a future intent doesn't fit any of these, the right move is usually a new memory-tree retrieval primitive, not a new provider-specific tool.

## What this fix changed (#1341)

Prior to #1341 the read-only RPC controllers existed and were callable over JSON-RPC, but no `Tool` impl wrapped them and the orchestrator didn't list them — so the agent could see WhatsApp data only through the memory tree. That worked for summaries but failed on exact-lookup intents because the memory tree's per-day transcript granularity loses the structure the user asks about (sender JID, exact `chat_id`, per-message timestamp). Adding the three direct tools closed that gap without adding any new ingest path.
