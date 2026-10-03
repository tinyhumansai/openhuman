# threads/store

Workspace-backed chat thread and message storage: transcript persistence, not
memory. The store itself (on-disk format, locking, warm trigram/CJK-bigram
index for cross-thread search, CRUD) is `tinyagents_session::threads` in
`vendor/tinyagents`; this directory keeps only the host wiring and re-exports
the store so callers name `crate::threads::store::{...}`. Memory v2
(`crate::memory`) is separate: it only ingests committed turns through its own
bus subscriber (see [`memory/`](../../memory/)).

## Parts

- `mod.rs`: re-exports the store API. `ConversationMessage` and
  `ConversationMessagePatch` are host-spelled aliases of the crate's
  `ThreadMessage` / `ThreadMessagePatch`; type names never reach the disk.
- `blocking.rs`: `spawn_blocking` wrappers around every store entry point.
  The store is synchronous and takes `parking_lot` locks across fsync'd file
  I/O, so request paths must go through `blocking` rather than calling it from
  an `async fn` (#5156).
- `bus.rs`: the `core::bus` subscriber
  (`register_conversation_persistence_subscriber`) that mirrors inbound and
  processed channel turns into the store, so channel transcripts (Slack,
  Telegram, ...) persist alongside the UI's own threads.

## On-disk layout

Unchanged from before the move:

```text
<workspace>/memory/conversations/
├── threads.jsonl              # append-only upsert/delete log of thread metadata
└── threads/
    └── <hex(thread_id)>.jsonl # one file per thread, its messages in order
```

## Tests

`blocking_tests.rs` and `bus_tests.rs` cover the wiring; the store's own tests
live in `vendor/tinyagents/crates/tinyagents-session/src/threads/`.
