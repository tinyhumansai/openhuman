# threads/store

Chat thread and message storage: transcript persistence, not memory. The store
API, indexing and CRUD are `tinyagents_session::threads` in
`vendor/tinyagents`; this directory keeps only the host wiring and re-exports
the store so callers name `crate::threads::store::{...}`. Memory v2
(`crate::memory`) is separate: it only ingests committed turns through its own
bus subscriber (see [`memory/`](../../memory/)).

## Parts

- [`mod.rs`](./mod.rs): re-exports the store API. `ConversationMessage` and
  `ConversationMessagePatch` are host-spelled aliases of the crate's
  `ThreadMessage` / `ThreadMessagePatch`; type names never reach the disk.
- [`blocking.rs`](./blocking.rs): `spawn_blocking` wrappers around every store entry point.
  The store is synchronous and takes `parking_lot` locks across fsync'd file
  I/O, so request paths must go through `blocking` rather than calling it from
  an `async fn` (#5156).
- [`bus.rs`](./bus.rs): the `core::bus` subscriber
  (`register_conversation_persistence_subscriber`) that mirrors inbound and
  processed channel turns into the store, so channel transcripts (Slack,
  Telegram, ...) persist alongside the UI's own threads.
  A caller that persists a channel turn itself (the hosted-channel relay)
  calls `claim_channel_turn(channel, message_id)` first; the subscriber skips
  claimed turns rather than mirroring them under a second thread id. In SaaS
  the subscriber is not registered and a claim is a no-op, so the untenanted
  `(channel, message_id)` key never spans profiles.

## Persistence

The runtime installs `tinyagents_session::threads` with the configured
session store. Local profiles use the workspace-backed driver; shared storage
uses the installed backend. This host module does not choose a filesystem
layout.

## Tests

[`blocking_tests.rs`](./blocking_tests.rs) and [`bus_tests.rs`](./bus_tests.rs) cover the wiring; the store's own tests
live in `vendor/tinyagents/crates/tinyagents-session/src/threads/`.

## Further reading

- [Parent module (`threads`)](../README.md)
- [Chat](../../../../../docs/gitbooks/en/features/chat.md)
- [Agent harness architecture](../../../../../docs/gitbooks/en/developing/architecture/agent-harness.md)
- [Frontend architecture](../../../../../docs/gitbooks/en/developing/architecture/frontend.md)
