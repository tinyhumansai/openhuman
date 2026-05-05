# conversations

Workspace-backed conversation thread/message storage. Lives at
`<workspace>/memory/conversations/` as plain JSONL — easy to inspect,
recover, and back up. Used by the desktop UI for chat threads and by
non-web channel adapters (Slack, Telegram, …) so all surfaces share one
persistence path.

## Files

- **`mod.rs`** — re-exports the public surface
  (`ConversationStore`, `ConversationThread`, `ConversationMessage`,
  `CreateConversationThread`, `ConversationMessagePatch`,
  `ConversationPurgeStats`, free-function shims, and
  `register_conversation_persistence_subscriber`).
- **`types.rs`** — wire/storage structs: thread metadata, message
  records, create requests, partial-update patches.
- **`store.rs`** — `ConversationStore` plus free-function shims.
  Thread metadata is appended to `threads.jsonl` (upsert/delete log);
  messages live in `threads/<thread_id>.jsonl`. A process-wide mutex
  serialises every on-disk mutation.
- **`bus.rs`** — `EventHandler` that mirrors inbound `DomainEvent`
  channel messages into the store, so non-web providers persist
  alongside UI-driven threads.
- **`store_tests.rs`** — unit tests covering upsert, append, label/
  title updates, deletion, and purge.

## Where it fits

Sits next to the unified memory store but is intentionally separate:
the conversation log is append-only chat history with no embeddings or
graph relations. Ingestion into the searchable memory tree happens via
`tree/` and the per-provider ingestion modules (e.g. `slack_ingestion/`)
— this folder only owns durable transcript storage.
