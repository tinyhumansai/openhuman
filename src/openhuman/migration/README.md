# migration

Data-migration helpers that import memory from **other AI assistants' workspaces** (OpenClaw, Hermes Agent) into the current OpenHuman workspace's memory backend. It scans a source workspace for SQLite (`brain.db`) and Markdown memory artifacts, normalizes them into `Memory` entries, backs up the target's existing memory, and writes the imported entries — supporting a `dry_run` plan-only mode and idempotent re-runs (unchanged entries are skipped, conflicts are renamed). Exposes two RPC controllers under the `migrate` namespace.

> Not to be confused with `crate::openhuman::migrations` (plural), which handles internal config **schema** version upgrades. This module migrates **user memory data** from foreign vendors.

## Responsibilities

- Resolve a source workspace path (explicit override, else vendor default: `~/.openclaw/workspace`, or `~/.hermes` / `%LOCALAPPDATA%\hermes` on Windows).
- Refuse self-migration when source resolves to the current OpenHuman workspace.
- **OpenClaw**: read memory entries from `memory/brain.db` (SQLite `memories` table, schema-tolerant column detection) plus `MEMORY.md` and `memory/*.md`.
- **Hermes**: read a fixed file mapping — `MEMORY.md` → core, `USER.md` → `Custom("user_profile")`, `SOUL.md` → `Custom("persona")`.
- Normalize keys (non-alphanumeric → `_`), parse/map categories, de-dup exact duplicates for deterministic re-runs.
- Back up the target workspace's existing memory (`MEMORY.md`, `brain.db`, `memory/*.md` → `memory_backup/`) before applying.
- Import into the target memory backend: skip entries whose content is unchanged, rename key on content conflict (`key_1`, `key_2`, …).
- Produce a `MigrationReport` (source/target paths, dry-run flag, `MigrationStats`, warnings).

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/migration/mod.rs` | Export-only: declares `core`/`ops`/`schemas`, re-exports `core::*` and `ops::*`, aliases `ops as rpc`, and exports the `all_migration_controller_schemas` / `all_migration_registered_controllers` pair. |
| `src/openhuman/migration/core.rs` | Core logic. `MigrationStats`, `MigrationReport`, `SourceEntry`; `migrate_openclaw_memory` / `migrate_hermes_memory`; source readers (SQLite + Markdown), workspace resolution, key/category normalization, backup, conflict-rename helpers. Inline `#[cfg(test)]` unit tests. |
| `src/openhuman/migration/ops.rs` | JSON-RPC/CLI adapter (the canonical handler file, re-exported as `rpc`). `migrate_openclaw` / `migrate_hermes` wrap the core fns, map `anyhow::Error` → `String`, and return `RpcOutcome<MigrationReport>` with a `"migration completed"` log. Tests cover dry-run, apply, missing-source, and self-migration. |
| `src/openhuman/migration/schemas.rs` | Controller schemas + handlers. Defines `MigrateOpenClawParams` / `MigrateHermesParams`, `all_controller_schemas`, `all_registered_controllers`, `schemas(function)`, and `handle_migrate_openclaw` / `handle_migrate_hermes` which load config and delegate to `migration::rpc::*`. |

## Public surface

From `mod.rs` re-exports (`core::*` + `ops::*`):

- Types: `MigrationReport`, `MigrationStats`.
- Core fns: `migrate_openclaw_memory(config, source_workspace, dry_run) -> Result<MigrationReport>`, `migrate_hermes_memory(...)`.
- RPC fns (via `ops` / alias `rpc`): `migrate_openclaw(...) -> Result<RpcOutcome<MigrationReport>, String>`, `migrate_hermes(...)`.
- Controller registry exports: `all_migration_controller_schemas`, `all_migration_registered_controllers`.

(`SourceEntry` and the internal helpers in `core.rs` are private.)

## RPC / controllers

Two controllers in the `migrate` namespace (registered via `all_registered_controllers`):

| Method | Description | Inputs | Output |
| --- | --- | --- | --- |
| `migrate.openclaw` | Migrate OpenClaw memory into current workspace. | `source_workspace?: String`, `dry_run?: bool` | `report: MigrationReport` |
| `migrate.hermes` | Migrate Hermes Agent memory into current workspace. | `source_workspace?: String`, `dry_run?: bool` | `report: MigrationReport` |

Both inputs are optional; `dry_run` **defaults to `true`** when omitted (`handle_*` use `unwrap_or(true)`). Handlers load config via `config_rpc::load_config_with_timeout()`. Unknown function names yield an `"unknown"` placeholder schema with an `error` output.

## Agent tools

None. This module owns no `tools.rs`.

## Events

None. No `bus.rs` / event-bus subscribers or publishers.

## Persistence

No own store. It writes imported entries through the **target memory backend** obtained from `memory_store::create_memory_for_migration(&config.memory, &config.workspace_dir)`, using `Memory::store` / `Memory::get`. Before applying (non-dry-run), it copies the target's existing memory artifacts into `<workspace_dir>/memory_backup/`.

## Dependencies

- `crate::openhuman::config::Config` — source of `workspace_dir` and `memory` config; `config::rpc::load_config_with_timeout` in handlers.
- `crate::openhuman::memory` (`Memory`, `MemoryCategory`) — target backend trait + category enum entries are mapped into.
- `crate::openhuman::memory_store` — `create_memory_for_migration` constructs the target memory backend.
- `crate::core::all` (`ControllerFuture`, `RegisteredController`) and `crate::core::{ControllerSchema, FieldSchema, TypeSchema}` — controller registry/schema types.
- `crate::rpc::RpcOutcome` — RPC response envelope.
- External crates: `rusqlite` (read OpenClaw `brain.db`), `directories::UserDirs` (home dir), `anyhow`, `serde`/`serde_json`.

## Used by

- `src/core/all.rs` — registers `all_migration_registered_controllers()` (line ~165) and `all_migration_controller_schemas()` (line ~319) into the global controller/schema registry, exposing both methods over CLI and JSON-RPC.

## Notes / gotchas

- `dry_run` default is `true` at the RPC boundary — callers must explicitly pass `dry_run: false` to actually apply a migration.
- OpenClaw SQLite reading is **schema-tolerant**: it inspects `PRAGMA table_info(memories)` and picks key/content/category columns from candidate name lists (`key`/`id`/`name`, `content`/`value`/`text`/`memory`, `category`/`kind`/`type`), bailing only if no content-like column exists. DB is opened read-only.
- Idempotency: exact-duplicate source entries are de-duped; unchanged target entries are skipped (`skipped_unchanged`); content conflicts get a renamed key (`renamed_conflicts`).
- `migrate_openclaw_apply_imports_markdown_entries` in `ops.rs` documents a regression (#1440): the apply path previously bailed in `create_memory_for_migration` under the unified-namespace memory core; that hard-disable was removed.
- Memory entries are stored under the empty namespace (`""`).
