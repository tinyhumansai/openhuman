# memory_sources

Registry of data connectors that feed memory. This domain owns the **"what feeds my memory"** question: a typed registry of sources (Composio OAuth connections, local folders, GitHub repos, RSS feeds, Twitter queries, web pages) persisted in `config.toml` under `[[memory_sources]]`. It provides CRUD for source entries, a `SourceReader` trait with per-kind reader implementations that list items and read individual item content, manual sync orchestration that ingests reader output into the memory pipeline, per-source sync status, and the `openhuman.memory_sources_*` RPC surface. It does **not** own sync scheduling or the ingestion engine itself — `memory_sync` / `memory` do that; this module only defines connectors, reads from them, and dispatches sync work to the right backend.

## Responsibilities

- CRUD for `MemorySourceEntry` records (add/get/list/update/remove) persisted in `Config.memory_sources`.
- Validate kind-specific required fields at add/update time.
- Provide a uniform `SourceReader` trait with one implementation per `SourceKind`, plus a `reader_for(kind)` dispatcher.
- List readable items and read individual item content from each source.
- Trigger a manual sync per source: Composio sources delegate to `memory_sync::composio`; reader-backed kinds (folder/github/rss/web) walk items and ingest each via `memory::ingest_pipeline::ingest_document`; Twitter is a placeholder.
- Emit sync progress as `MemorySyncStageChanged` events tagged with `connection_id = Some(source.id)`.
- Compute per-source sync status (chunks synced/pending, last-chunk timestamp, freshness label) by querying `mem_tree_chunks`.
- Reconcile active Composio connections into the registry at boot / on list, so freshly-connected integrations appear as sources without a restart.
- Auto-upsert a Composio source on OAuth connection creation (called from `memory_sync::composio::bus`).

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/memory_sources/mod.rs` | Module docstring + `pub mod` decls + re-exports (registry CRUD, schemas, core types). Export-focused. |
| `src/openhuman/memory_sources/types.rs` | Core types: `SourceKind`, `MemorySourceEntry` (flattened kind-specific `Option` fields) with `validate()`, `SourceItem`, `ContentType`, `SourceContent`. |
| `src/openhuman/memory_sources/registry.rs` | CRUD over `Config.memory_sources` via the config load/save cycle; `MemorySourcePatch` partial-update payload; `upsert_composio_source` for auto-registration. |
| `src/openhuman/memory_sources/rpc.rs` | RPC handler impls returning `RpcOutcome<T>` (request/response structs for list/get/add/update/remove/list_items/read_item/sync/status_list). `list_rpc` lazily reconciles Composio sources. |
| `src/openhuman/memory_sources/schemas.rs` | Controller-registry schemas + `handle_*` fns delegating to `rpc.rs`; `all_controller_schemas` / `all_registered_controllers`. |
| `src/openhuman/memory_sources/sync.rs` | Per-source sync orchestration. Spawns background task, dispatches by kind, ingests reader output, emits stage events. |
| `src/openhuman/memory_sources/status.rs` | `SourceStatus`, `FreshnessLabel`, `source_status` / `status_list` — queries `mem_tree_chunks` by source-id prefix. |
| `src/openhuman/memory_sources/reconcile.rs` | `ensure_composio_sources` — scans active Composio sync targets and upserts them as sources. |
| `src/openhuman/memory_sources/readers/mod.rs` | `SourceReader` async trait + `reader_for(kind)` dispatcher. |
| `src/openhuman/memory_sources/readers/composio.rs` | `ComposioReader` — returns the connection as a single item; `read_item` is a non-op placeholder (sync is provider-driven). |
| `src/openhuman/memory_sources/readers/folder.rs` | `FolderReader` — glob over a local dir (default `**/*.md`, 10 MB cap), reads file content with path-traversal guard. |
| `src/openhuman/memory_sources/readers/github.rs` | `GithubReader` — pulls project activity (commits/issues/PRs) via `gh` CLI or public REST fallback. |
| `src/openhuman/memory_sources/readers/rss.rs` | `RssReader` — RSS/Atom feed items. |
| `src/openhuman/memory_sources/readers/twitter.rs` | `TwitterReader` — Twitter query reader (sync placeholder pending credentials). |
| `src/openhuman/memory_sources/readers/web_page.rs` | `WebPageReader` — fetches a web page, optional CSS selector. |

## Public surface

Re-exported from `mod.rs`:

- **registry**: `add_source`, `get_source`, `list_sources`, `list_enabled_by_kind`, `remove_source`, `update_source`, `upsert_composio_source`, `MemorySourcePatch`.
- **schemas**: `all_memory_sources_controller_schemas`, `all_memory_sources_registered_controllers`.
- **types**: `ContentType`, `MemorySourceEntry`, `SourceContent`, `SourceItem`, `SourceKind`.

Reader trait `SourceReader` and `reader_for` are public under `readers`; sync/status/reconcile entry points (`sync::sync_source`, `status::status_list` / `source_status`, `reconcile::ensure_composio_sources`) are public within the module path.

## RPC / controllers

Namespace `memory_sources` (`openhuman.memory_sources_*`). Nine controllers, each schema/handler pair defined in `schemas.rs` and delegating to `rpc.rs`:

| Function | Description |
| --- | --- |
| `list` | List all configured sources (lazily reconciles Composio first). |
| `get` | Get one source by `id`. |
| `add` | Add a source; kind-specific fields are flat on the request. |
| `update` | Partial update of a source. |
| `remove` | Remove a source by `id`. |
| `list_items` | List readable items from a source via its reader. |
| `read_item` | Read one item's content. |
| `sync` | Queue a manual sync (returns immediately; progress via events). |
| `status_list` | Per-source sync status (chunks, freshness, last-chunk ts). |

Wired into the registry via `core/all.rs` (`all_memory_sources_registered_controllers` / `all_memory_sources_controller_schemas`).

## Agent tools

None. This module exposes no agent tools (`tools.rs` does not exist).

## Events

No `bus.rs` / `EventHandler` of its own. It **publishes** `DomainEvent::MemorySyncStageChanged` indirectly via `memory::sync::emit_sync_stage` during `sync_source` (stages: Requested, Fetching, Stored, Ingesting, Completed, Failed), tagged `connection_id = Some(source.id)`. The reverse direction — auto-registering a Composio source on connection-created — is driven by `memory_sync::composio::bus`, which calls this module's `upsert_composio_source`.

## Persistence

- **Source registry**: persisted in `Config.memory_sources` (`config/schema/types.rs`), serialized as `[[memory_sources]]` in `config.toml`. All mutations reload the live config, apply, and `config.save()` atomically.
- **No dedicated `store.rs`.** Sync status is *read* (not written) from the memory store: `status.rs` queries `mem_tree_chunks` (via `memory_store::chunks::store::with_connection`) using a `source_id LIKE` prefix — `mem_src:{source.id}:%` for reader kinds, `{toolkit}:%` for Composio. Chunks themselves are written by the `memory` ingest pipeline, not here.

## Dependencies

- `openhuman::config` (`Config`, `config::rpc::load_config_with_timeout`) — the registry's backing store; readers/sync receive `&Config`.
- `openhuman::memory::ingest_pipeline::ingest_document` — ingests reader-backed source items into memory.
- `openhuman::memory::sync` (`emit_sync_stage`, `MemorySyncStage`, `MemorySyncTrigger`) — sync-progress event emission.
- `openhuman::memory_sync::composio` — Composio sync delegate (`run_connection_sync`, `scan_active_sync_targets`, `SyncReason`).
- `openhuman::memory_sync::canonicalize::document::DocumentInput` — document shape for ingestion.
- `openhuman::memory_store::chunks::store::with_connection` — SQLite access for status queries against `mem_tree_chunks`.
- `core::all` (`ControllerFuture`, `RegisteredController`) and `core` schema types (`ControllerSchema`, `FieldSchema`, `TypeSchema`) — controller registry wiring.
- `rpc::RpcOutcome` — RPC return contract.
- External: `glob`, `async_trait`, `uuid`, `chrono`, `serde`/`serde_json`, `schemars`, `toml`; `gh` CLI (optional) for the GitHub reader.

## Used by

- `core/all.rs`, `core/jsonrpc.rs` — registers the `memory_sources` controllers/schemas.
- `openhuman/mod.rs` — declares the domain module.
- `config/schema/types.rs` — `Config.memory_sources: Vec<MemorySourceEntry>` field (the persisted store).
- `memory_sync::composio::bus` / `memory_sync::composio::mod` — calls `upsert_composio_source` to auto-register a source on connection creation.
- `composio::ops` — references gmail memory-source cleanup targets (`gmail_memory_sources_for_connection`).

## Notes / gotchas

- `MemorySourceEntry` is a single flat struct: all kind-specific fields are `Option`/`Vec` and the `kind` discriminator decides which are required, enforced only by `validate()` (not the type system). RPC `add`/`update` mirror this flat shape.
- `list_rpc` performs a lazy Composio reconciliation on every list call so newly-connected integrations show up immediately (the connection-created hook only fires on OAuth handoff, not on first launch after a prior connect).
- `sync_source` returns `Ok(())` as soon as work is queued; it spawns a nested `tokio::spawn` so a panic in the sync task surfaces as a `tracing::error!` rather than a dropped join handle. Actual completion/failure arrives only via `MemorySyncStageChanged` events.
- Composio sync does not ingest item-by-item — it delegates wholesale to `memory_sync::composio::run_connection_sync`. The `ComposioReader::read_item` body is an explanatory placeholder, never a real fetch.
- Twitter sync is intentionally unimplemented: `sync_source` returns an error for `TwitterQuery` ("Twitter sync not yet configured").
- Status freshness thresholds: ≤30 s → `Active`, ≤5 min → `Recent`, else / no chunk → `Idle`. `status.rs` surfaces real SQL errors (so a broken DB isn't reported as a healthy zero-row state), but `status_list` degrades a per-source failure to an `Idle` zero-row entry rather than failing the whole call.
- Composio chunk-count matching is by `toolkit` prefix only (`{toolkit}:%`), so distinct connections sharing a toolkit (e.g. two Gmail accounts) are not disambiguated in status counts.
- `FolderReader` caps files at 10 MB on both list and read, and canonicalizes paths to deny traversal outside the configured base.
