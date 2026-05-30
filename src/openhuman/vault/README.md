# vault

Knowledge vault domain — a NotebookLM-style "folder of files" mirrored into the memory-tree backend. A `Vault` points at a local directory; on sync the module walks that directory, routes supported files to a plain-UTF-8 extractor by extension, and feeds them through the memory ingestion pipeline under a vault-derived namespace. Per-file dedup uses `(rel_path, mtime, content hash)` so re-syncs only touch what actually changed, and vanished files have their memory rows deleted to keep retrieval in sync with disk.

## Responsibilities

- Register / list / fetch / remove user-owned local folders as "vaults" backed by SQLite.
- Walk a vault's root directory, prune built-in noise dirs (`.git`, `node_modules`, `target`, …), apply user include/exclude substring patterns, and filter by supported extension + max file size (5 MiB).
- Ingest new/changed files into the **memory-tree** backend (`mem_tree_chunks` / `mem_tree_ingested_sources`) via the ingest pipeline, keyed by a stable `source_id = vault:{vault_id}:{rel_path}`.
- Two-tier dedup: fast-path mtime skip during discovery, secondary content-hash skip during concurrent ingestion.
- For content updates, delete prior chunks before re-ingest (the pipeline's `already_ingested` gate is content-blind on a stable source_id).
- Delete memory rows for files that vanished since the last sync; record per-file outcome in a ledger.
- Run sync as a background tokio task; expose live progress + final outcome via an in-process state registry, polled by the frontend.
- On vault removal, optionally purge all memory rows for the vault (memory-tree prefix delete + best-effort legacy `clear_namespace`).
- Hide vaults that belong to an incompatible host OS / path shape (cross-machine config sharing safety).

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/vault/mod.rs` | Export-focused module root: docstring, `mod`/`pub mod` decls, re-exports of types + controller-schema pair. |
| `src/openhuman/vault/types.rs` | Serde domain types: `Vault`, `VaultFile`, `VaultFileStatus`, `VaultSyncState`, `VaultSyncStatus`, `VaultSyncReport`. |
| `src/openhuman/vault/ops.rs` | RPC-facing business logic: `vault_create/list/get/files/remove/sync/sync_status`, namespace derivation, background-task spawn with panic guard, memory purge on remove. |
| `src/openhuman/vault/schemas.rs` | Controller schemas + `handle_*` fns delegating to `ops`; `all_controller_schemas` / `all_registered_controllers`. |
| `src/openhuman/vault/store.rs` | SQLite persistence (`vault.db`): vault + per-file ledger tables, migrations, host-OS compatibility filtering. |
| `src/openhuman/vault/sync.rs` | The directory-walk + 4-phase ingest engine (discovery → concurrent ingest → ledger writes → deletions); supported-extension list; memory-tree source_id helper. Inline `sync_tests`. |
| `src/openhuman/vault/state.rs` | Process-global in-memory sync-progress registry (`once_cell::Lazy` + `parking_lot::RwLock`), keyed by `vault_id`. |
| `src/openhuman/vault/tests.rs` | `#[cfg(test)]` module (additional tests beyond the inline `sync.rs` suite). |

## Public surface

Re-exported from `mod.rs`:

- Types: `Vault`, `VaultFile`, `VaultFileStatus`, `VaultSyncReport`, `VaultSyncState`, `VaultSyncStatus`.
- `all_vault_controller_schemas` / `all_vault_registered_controllers` (the controller-registry pair, wired in `src/core/all.rs`).
- `ops` is `pub mod` — `vault_create`, `vault_list`, `vault_get`, `vault_files`, `vault_remove`, `vault_sync`, `vault_sync_status`.

## RPC / controllers

Namespace `vault` (invoked as `openhuman.vault_<function>`):

| Method | Inputs | Output |
| --- | --- | --- |
| `vault.create` | `name`, `root_path` (absolute dir), `include_globs?`, `exclude_globs?` | `Vault` |
| `vault.list` | — | `Vec<Vault>` (current-host only) |
| `vault.get` | `vault_id` | `Vault` |
| `vault.files` | `vault_id` | `Vec<VaultFile>` (per-file ledger) |
| `vault.remove` | `vault_id`, `purge_memory?` | `{ vault_id, removed, purged, memory_tree_chunks_deleted, purge_error? }` |
| `vault.sync` | `vault_id` | `{ status: "started", vault_id }` (background; rejects if already running) |
| `vault.sync_status` | `vault_id` | `VaultSyncState` (Idle / Running / Completed / Failed) |

`include_globs` / `exclude_globs` are substring matches (case-insensitive), not true globs despite the name.

## Agent tools

None — the module owns no `tools.rs`.

## Events

None — no `bus.rs`; the module publishes/subscribes to no `DomainEvent`s. Sync progress is surfaced via the in-memory `state` registry + `vault.sync_status` polling rather than the event bus.

## Persistence

- **SQLite** at `{workspace_dir}/vault/vault.db` (`store.rs`):
  - `vaults` — id, name, root_path, host_os, namespace (UNIQUE), include/exclude globs (JSON), created_at, last_synced_at.
  - `vault_files` — per-file ledger keyed by `(vault_id, rel_path)`, FK→`vaults` with `ON DELETE CASCADE`; stores `document_id` (= memory-tree source_id post-#2705), `content_hash`, `mtime_ms`, `bytes`, `ingested_at`, `status`. The dedup ledger.
  - Lazy schema init + an additive `host_os` column migration (cached per DB path).
- **Process-global in-memory** (`state.rs`): `VaultSyncState` per vault, retained after completion until the next sync overwrites it. Lost on process restart.
- **Memory-tree** (owned by the `memory` domain, written via the ingest pipeline): chunks under `source_id = vault:{vault_id}:{rel_path}`.

## Dependencies

- `crate::openhuman::config` (`Config`, `config::rpc::load_config_with_timeout`) — workspace dir for the DB, config passed into ingest/delete calls.
- `crate::openhuman::memory::ingest_pipeline` — `ingest_document` / `IngestResult`: the canonical memory-tree ingest path.
- `crate::openhuman::memory::ops` — `clear_namespace` (legacy purge on remove) and `doc_delete` (best-effort legacy UnifiedMemory cleanup for pre-#2705 ledger rows).
- `crate::openhuman::memory_store::chunks` — `delete_chunks_by_source` / `delete_chunks_by_source_prefix` (chunk cleanup on re-ingest, file deletion, and vault purge); `SourceKind::Document`.
- `crate::openhuman::memory_sync::canonicalize::document::DocumentInput` — the document shape handed to the ingest pipeline.
- `crate::core::all` (`ControllerFuture`, `RegisteredController`) + `crate::core::{ControllerSchema, FieldSchema, TypeSchema}` — controller registration plumbing.
- `crate::rpc::RpcOutcome` — RPC return contract.
- External crates: `rusqlite`, `walkdir`, `sha2`, `uuid`, `chrono`, `futures` (`buffer_unordered`), `tokio`, `once_cell`, `parking_lot`.

## Used by

- `src/core/all.rs` — registers vault controllers + schemas into the global controller registry (the only wiring point; vault is controller-only, no CLI/JSON-RPC branches).
- `src/core/observability.rs` (+ tests) — references `vault_create` / `openhuman.vault_create` as an example in path-boundary / autonomy-policy diagnostics, not a runtime dependency.

## Notes / gotchas

- **Memory-tree, not `memory_docs` (#2705).** Sync ingests via the memory-tree pipeline; the pre-#2705 path wrote to legacy `UnifiedMemory` (`memory_docs`), which the UI reported as "synced" but retrieval never saw. The ledger now stores the memory-tree `source_id` (`vault:{id}:{rel_path}`); deletion/purge logic keys off `document_id.starts_with("vault:")` to distinguish post- vs pre-#2705 rows and run the right cleanup.
- **Namespace is a hashed digest, not the raw UUID.** `vault_namespace_for_id` derives an alphabet-only SHA-256 suffix (`vault-<24 a-z chars>`) because memory writes reject namespace/key values that resemble PII / strict identifier patterns.
- **`include_globs`/`exclude_globs` are substring matches**, lowercased, not real glob patterns — the field name is misleading.
- **Background sync + panic guard.** `vault_sync` spawns a tokio task and returns immediately; the work is wrapped in `catch_unwind` so a panic marks state `Failed` instead of leaving it stuck in `Running` (which would reject every future sync until restart). Concurrency is bounded to 4 (`buffer_unordered`).
- **Host-OS gating.** `list_vaults`/`get_vault` hide vaults whose stamped `host_os` (or, for legacy rows, whose `root_path` shape) doesn't match the current machine — prevents surfacing another machine's folders from shared config.
- **Ledger ↔ memory_tree desync is logged loudly.** If ingest returns `already_ingested && chunks_written == 0` after the delete-first guard, the code emits a `warn!` because it means new content never reached retrieval (the exact false-success class #2705 was meant to kill).
- `vault_create` canonicalizes `root_path` (falling back to the trimmed input if canonicalization fails), so the stored id/path may differ from the literal input.
