# slack_ingestion

Slack-specific plumbing that feeds the memory tree. Auth and scheduling
live elsewhere — OAuth via Composio, periodic ticks via
`composio::periodic` — this folder converts Slack messages into
`tree::ingest::ingest_chat` calls.

## Files

- **`mod.rs`** — module re-exports
  (`all_slack_ingestion_controller_schemas`,
  `all_slack_ingestion_registered_controllers`).
- **`types.rs`** — internal `SlackMessage`, `SlackChannel`, `Bucket`.
  Decoupled from raw Slack Web API payloads; the Composio provider
  parses into these.
- **`bucketer.rs`** — 6-hour UTC-aligned windowing
  (`bucket_start_for`, `bucket_end_for`), grace-period extraction
  (`split_closed`), and stable `source_id_for` generation. Bucket
  width is a schema constant — changing it invalidates all existing
  source IDs.
- **`ops.rs`** — bucket-to-`ChatBatch` canonicalisation
  (`bucket_to_chat_batch`) and the `ingest_bucket` wrapper around
  `tree::ingest::ingest_chat`. Free of HTTP and timers so it is
  easy to unit-test.
- **`rpc.rs`** — handler bodies for `slack_memory_sync_trigger`
  (manual sync run) and `slack_memory_sync_status` (per-connection
  cursor + budget snapshot).
- **`schemas.rs`** — controller schemas + dispatch wiring under the
  `slack_memory` JSON-RPC namespace.

## Where it fits

The Composio-backed `SlackProvider` (in `composio/providers/slack/`)
calls into `bucketer` + `ops` to produce closed-bucket
`ChatBatch`es; those flow into `memory::tree::ingest::ingest_chat` and
become source-tree leaves. State (cursors, dedup set, daily budget)
lives in the Composio sync-state KV — not here.
