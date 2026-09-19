# session_import

One-time migration of legacy OpenHuman session transcripts into TinyAgents
`Store`/`AppendStore` records, exposed as the explicit `session_import.run`
controller (`openhuman-core session_import run`, JSON-RPC
`openhuman.session_import_run`). It is never run from a boot hook. The same
module also owns the live dual-write and shadow-read paths that keep new
turns landing in the same store layout (`live.rs`), and `open_session_stores`,
which every other TinyAgents-store consumer in the crate reuses.

> The migration was specified in a design doc for issue #4249 that is not
> checked into this repository; `types.rs` still refers to it when explaining
> why stream names are dot-separated.

## Sources → destination

- Transcript JSONL under `session_raw/`: the current flat layout
  (`session_raw/{stem}.jsonl`) and the legacy date-folder layout
  (`session_raw/{DDMMYYYY}/{stem}.jsonl`).
- Markdown-only sessions under `sessions/{dir}/{stem}.md` that have no JSONL
  twin. A Markdown file next to a JSONL source is recorded as a companion
  pointer only, never read.
- Precedence per session stem: flat JSONL > legacy-dir JSONL > Markdown-only
  (`scan.rs::discover_sources`); a duplicate stem is a warning, not a second
  item.
- `session_db/sessions.db` is opened read-only, when present, to join
  `agent_runs` ids onto the descriptor and cross-check parent lineage. The
  stem chain (`a__b` → parent `a`) wins over a disagreeing ledger.

All writes land under `{workspace}/tinyagents_store/`:

- `kv/sessions/{sanitized stem}.json` — `SessionDescriptor` compatibility
  record mapping the OpenHuman session key to TinyAgents identifiers.
- `kv/migration_items/{sha256(relative source path)}.json` — per-item
  idempotency ledger (`ItemLedgerRecord`).
- `kv/migrations/session_import_v1.json` — global run marker.
- `journal/session.{stem}.messages.jsonl` — message journal, one
  `StoreRecord` per line via TinyAgents `JsonlAppendStore`.

Source files are never mutated or deleted.

## Idempotency

- A full run (no `only`) that is not a dry run writes the global marker
  (`MARKER_KEY`). A later full, non-forced, non-dry run sees the marker and
  returns `already_done` without scanning.
- Independently, every written source gets a ledger entry keyed by the sha256
  of its workspace-relative path holding `IMPORT_VERSION`, size, and mtime.
  A source whose fingerprint still matches is `skipped_unchanged`, even under
  `only` or after the global marker is gone.
- `force` ignores both the marker and the ledger.
- `dry_run` bypasses the marker fast path, reports `would_import` per item,
  and writes nothing.
- Re-import is a full rewrite: the journal stream file is deleted and
  re-appended because `JsonlAppendStore` has no truncate.

## Key files

| File | Role |
| --- | --- |
| `mod.rs` | Module docs and re-exports (`ImportOptions`, `ImportSummary`, controller registration). |
| `types.rs` | Serde types: `ImportOptions`, `ImportSummary`, `ItemReport`, `SessionDescriptor`, `JournalMessage`, `ItemLedgerRecord`, and the store-layout constants (`KV_SUBDIR`, `JOURNAL_SUBDIR`, `NS_*`, `MARKER_KEY`, `IMPORT_VERSION`). |
| `scan.rs` | `discover_sources` — walks `session_raw/` and `sessions/`, dedupes by stem per the precedence order above. |
| `convert.rs` | Pure helpers: `parent_session_key` stem lineage, `sanitize_store_name`, `stream_name`, `effective_thread_id` (synthesizes `imported-{stem}` when `_meta` has none), `build_descriptor`, `journal_messages`. |
| `ops.rs` | `run_import` — scan → read all → plan/write per item → marker; `open_session_stores` opens the shared KV/journal handles over `{workspace}/tinyagents_store/{kv,journal}`. |
| `live.rs` | Dual-write of each persisted turn into the same layout (`write_live_turn`), gated by `AgentConfig::session_dual_write` (default on) with the `OPENHUMAN_SESSION_DUAL_WRITE` env kill switch; store-backed shadow read of the same session (`shadow_read_compare`), gated by `AgentConfig::session_shadow_reads` (default on) with `OPENHUMAN_SESSION_SHADOW_READS`; `session_kv_store` exposes the KV store on `RunContext.stores` as `TINYAGENTS_SESSION_KV_STORE`. Env vars can only force off, never on. Errors are logged and swallowed by callers; the legacy transcript stays authoritative for both reads and writes. |
| `schemas.rs` | `session_import.run` `ControllerSchema` and handler; resolves the workspace from config unless `workspace` is passed. |
| `*_tests.rs` | Sibling test suites for `convert`, `live`, `ops`, `schemas`. |

## RPC

`session_import.run` accepts `dry_run`, `only` (glob over session stems),
`force`, `verbose`, and an optional `workspace` override, and returns an
`ImportSummary`. Registered via `all_session_import_registered_controllers`
in `crates/openhuman-core/src/core/all.rs`.

## Used by

- `crates/openhuman-core/src/core/all.rs` — registers the controller.
- `tinyagents_session::transcript` — the legacy transcript readers
  (`read_transcript`, `read_transcript_legacy_md`) the importer converts from.
- `crates/openhuman-core/src/agent/session_host/turn/session_io/transcript_persist.rs`
  — calls `live::write_live_turn` after each transcript write and
  `live::shadow_read_compare` after each transcript load, both on background tasks.
- `crates/openhuman-core/src/agent/tinyagents/turn_runner.rs` — registers
  `live::session_kv_store` on the per-turn `RunContext`.
- `open_session_stores` is reused by `agent/tinyagents/{journal,reaper,todos,replay/ops}.rs`
  and `threads/goals/migration.rs` so every TinyAgents-store consumer shares
  one layout.
