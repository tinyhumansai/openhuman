# artifacts

Metadata domain for agent-generated artifacts (presentations, documents, images, and other files the agent produces). It owns the on-disk layout under `<workspace>/artifacts/`, persists per-artifact `meta.json` records, and exposes read/delete RPC controllers in the `ai` namespace. It does **not** generate artifact content itself — it is a thin, sandboxed metadata store + listing/retrieval/deletion surface over a workspace subdirectory.

## Responsibilities

- Define artifact metadata types (`ArtifactMeta`, `ArtifactKind`, `ArtifactStatus`) with case-insensitive string parsing and lowercase serde.
- Persist artifact metadata to `<workspace>/artifacts/<id>/meta.json` (`save_artifact_meta`).
- List artifacts with pagination, sorted by `created_at` descending, skipping corrupt/unreadable `meta.json` entries (`list_artifacts`).
- Retrieve a single artifact by ID, returning metadata plus a computed `absolute_path` (`get_artifact` / `ai_get_artifact`).
- Delete an artifact directory and all its contents (`delete_artifact`).
- Enforce path-traversal sandboxing on artifact IDs and resolved paths so callers cannot escape the artifacts root.
- Expose `ai.list_artifacts`, `ai.get_artifact`, `ai.delete_artifact` RPC controllers.

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/artifacts/mod.rs` | Export-only: `mod` decls + re-exports of `ArtifactKind`/`ArtifactMeta`/`ArtifactStatus` and the controller-schema/registry pair. |
| `src/openhuman/artifacts/types.rs` | Serde domain types: `ArtifactKind` (presentation/document/image/other), `ArtifactStatus` (pending/ready/failed), `ArtifactMeta`. Each enum has `as_str`/`parse` (case-insensitive, fall back to `Other`/`Pending`). |
| `src/openhuman/artifacts/ops.rs` | Business logic returning `RpcOutcome<Value>`: `ai_list_artifacts`, `ai_get_artifact`, `ai_delete_artifact`. Validates non-empty IDs, computes/guards `absolute_path`. `DEFAULT_LIMIT=50`, `MAX_LIMIT=200`. |
| `src/openhuman/artifacts/store.rs` | Persistence over `tokio::fs`: `artifacts_root`, `save_artifact_meta`, `list_artifacts`, `get_artifact`, `delete_artifact`, plus `validate_artifact_id` / `assert_within_root` sandboxing helpers. |
| `src/openhuman/artifacts/schemas.rs` | Controller schemas (`all_controller_schemas`), registry (`all_registered_controllers`), and `handle_*` fns delegating to `ops.rs`; param-parsing helpers (`read_required`, `read_optional_u64`, `type_name`). |
| `src/openhuman/artifacts/ops_tests.rs` | Sibling test suite for `ops.rs` (via `#[path]`). |
| `src/openhuman/artifacts/store_tests.rs` | Sibling test suite for `store.rs` (via `#[path]`). |

## Public surface

- `ArtifactKind`, `ArtifactMeta`, `ArtifactStatus` (re-exported from `types`).
- `all_artifacts_controller_schemas` / `all_artifacts_registered_controllers` (re-exported from `schemas`, wired into the core registry).
- `ops::{ai_list_artifacts, ai_get_artifact, ai_delete_artifact}` (called by the schema handlers; referenced by fully-qualified path).
- `store::*` functions are `pub(crate)` — usable inside the crate (e.g. a future producer calling `save_artifact_meta`).

## RPC / controllers

All in the `ai` namespace:

| Method | Inputs | Output |
| --- | --- | --- |
| `ai.list_artifacts` | `offset?: u64` (default 0), `limit?: u64` (default 50, cap 200) | `{ artifacts: ArtifactMeta[], total, offset, limit }` |
| `ai.get_artifact` | `artifact_id: string` (required) | flat `ArtifactMeta` fields + `absolute_path` |
| `ai.delete_artifact` | `artifact_id: string` (required) | `{ artifact_id, deleted: bool }` |

`get_artifact` output is intentionally flat (no opaque `artifact` wrapper). Handlers load config via `config::rpc::load_config_with_timeout()` and trim the `artifact_id`.

## Agent tools

None. This module owns no `tools.rs`.

## Events

None. No `bus.rs`; the module publishes/subscribes to no `DomainEvent`s.

## Persistence

- Root: `<workspace_dir>/artifacts/` (auto-created via `create_dir_all` in `artifacts_root`).
- Per artifact: `<workspace>/artifacts/<id>/meta.json` (pretty-printed `ArtifactMeta`).
- `workspace_dir` is read from `Config` (`config.workspace_dir`).
- No database; entirely filesystem-backed via `tokio::fs`. Listing scans subdirectories and skips entries whose `meta.json` is missing or corrupt (logged at `warn`).

## Dependencies

- `crate::openhuman::config::Config` / `config::rpc` — source of `workspace_dir` and config loading in handlers.
- `crate::core::all::{ControllerFuture, RegisteredController}` and `crate::core::{ControllerSchema, FieldSchema, TypeSchema}` — controller registry + schema types.
- `crate::rpc::RpcOutcome` — ops return type / handler JSON conversion.
- External crates: `tokio::fs`, `serde`/`serde_json`, `chrono` (timestamps).

## Used by

- `src/core/all.rs` — registers `all_artifacts_registered_controllers()` (line ~141) and `all_artifacts_controller_schemas()` (line ~307) into the global controller/schema registries. No other in-crate consumer of `save_artifact_meta` was found, so artifact creation is not yet wired from a producer domain.

## Notes / gotchas

- **Path-traversal hardening is layered**: `validate_artifact_id` rejects empty/`.`/`..`, `/` or `\`, absolute paths, and Windows drive-letter paths; `assert_within_root` re-checks the resolved path stays under the artifacts root before any write/read/delete; `ai_get_artifact` independently re-validates that `meta.path` does not escape the root when computing `absolute_path` (defends against a corrupt/adversarial stored `meta.path`).
- `list_artifacts` is **lossy by design** — bad `meta.json` files are skipped (warned), not surfaced as errors, so a single corrupt artifact never breaks the whole listing.
- Enum `parse` never errors: unknown `kind` → `Other`, unknown `status` → `Pending`.
- `store.rs` carries a dead-code `_assert_status_used` helper to keep `ArtifactStatus` referenced outside tests.
- Verbose `[artifacts]` `log::debug!`/`warn!` prefixes throughout, per repo logging conventions.
