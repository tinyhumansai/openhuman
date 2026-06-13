# workspace

Owns workspace layout bootstrap and the editable "Persona Pack" prompt files (`SOUL.md`, `IDENTITY.md`) that drive the agent's personality. Two concerns live here: (1) `init_workspace` — the one-shot setup that creates the default directory tree and copies the bundled prompt/skills/heartbeat files into a fresh workspace (backs CLI `init`-style entrypoints); and (2) read/edit/reset RPCs over a tightly allowlisted set of persona files, so the settings UI can round-trip those prompts without ever exposing an arbitrary path under the workspace.

## Responsibilities

- Initialize a fresh workspace: create the `memory`, `sessions`, `state`, `cron` directories, write bundled `SOUL.md` / `IDENTITY.md`, seed the skills dir README, and ensure `HEARTBEAT.md` — reporting created/overwritten/existing entries.
- Define the single source of truth for which workspace files are editable (the `BOOTSTRAP_FILES` allowlist via `bundled_default_contents`).
- Read an editable persona file, falling back to the bundled default (with `is_default = true`) when the on-disk copy is missing.
- Overwrite an editable persona file with user-supplied contents (size-capped, allowlist-enforced).
- Reset an editable persona file back to its bundled default.
- Enforce safety bounds: allowlist-only filenames, a `MAX_WORKSPACE_FILE_BYTES` (256 KiB) cap, TOCTOU-safe bounded reads, and UTF-8 validation.

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/workspace/mod.rs` | Export-focused. Re-exports `ops::*` and the RPC controller-schema pair (`all_workspace_controller_schemas` / `all_workspace_registered_controllers`). |
| `src/openhuman/workspace/ops.rs` | `init_workspace(force)` bootstrap logic, the `BOOTSTRAP_FILES` table (`SOUL.md`, `IDENTITY.md`), `bundled_default_contents` (the editable allowlist + reset source of truth), and `ensure_workspace_file`. |
| `src/openhuman/workspace/rpc.rs` | Pure-domain persona file API: `WorkspaceFile` type, `read_workspace_file` / `write_workspace_file` / `reset_workspace_file`, `MAX_WORKSPACE_FILE_BYTES`, allowlist enforcement (`ensure_editable`). Returns `RpcOutcome<WorkspaceFile>`. |
| `src/openhuman/workspace/schemas.rs` | Controller schemas + `handle_*` fns delegating to `rpc.rs`; loads config to resolve `workspace_dir`. |

## Public surface

From `mod.rs` re-exports (`ops::*` plus the schema pair):

- `init_workspace(force: bool) -> Result<serde_json::Value, String>` — bootstrap entrypoint.
- `bundled_default_contents(filename: &str) -> Option<&'static str>` — editable allowlist / default lookup.
- `all_workspace_controller_schemas()`, `all_workspace_registered_controllers()` — registry wiring.

`rpc.rs` items (`WorkspaceFile`, `read_/write_/reset_workspace_file`, `MAX_WORKSPACE_FILE_BYTES`) are `pub` and reached via `crate::openhuman::workspace::rpc::*`.

## RPC / controllers

Namespace `workspace`; three controllers (defined in `schemas.rs`, all return the `WorkspaceFile` output shape `{ filename, contents, is_default }`):

| RPC method | Inputs | Description |
| --- | --- | --- |
| `openhuman.workspace_file_read` | `filename` | Read an editable persona file; falls back to bundled default when the workspace copy is missing. |
| `openhuman.workspace_file_write` | `filename`, `contents` | Overwrite an editable persona file (size-capped server-side). |
| `openhuman.workspace_file_reset` | `filename` | Restore an editable persona file to its bundled default. |

Handlers resolve `workspace_dir` from config (`config_rpc::load_config_with_timeout`), trim the filename, delegate to `rpc.rs`, and serialize via `RpcOutcome::into_cli_compatible_json`. Unknown function names yield an `unknown` schema. Wired into the global registry in `src/core/all.rs`.

## Persistence

No dedicated `store.rs`. State is plain files under the configured `workspace_dir`:

- Directories: `memory/`, `sessions/`, `state/`, `cron/`.
- Files: `SOUL.md`, `IDENTITY.md` (bundled-prompt copies), `skills/README.md`, `HEARTBEAT.md`.

The editable surface is restricted to the `BOOTSTRAP_FILES` allowlist (`SOUL.md`, `IDENTITY.md`); the workspace dir is created on demand by write/reset.

## Dependencies

- `crate::openhuman::config::rpc` — loads `Config` (timeout-bounded) to resolve `workspace_dir` and `config_path` in both `ops.rs` and `schemas.rs`.
- `crate::openhuman::skills::init_skills_dir` — seeds the `skills/` directory README during `init_workspace`.
- `crate::openhuman::heartbeat::engine::HeartbeatEngine::ensure_heartbeat_file` — ensures `HEARTBEAT.md` during `init_workspace`.
- `crate::core::all` (`ControllerFuture`, `RegisteredController`) and `crate::core::{ControllerSchema, FieldSchema, TypeSchema}` — controller registry types.
- `crate::rpc::RpcOutcome` — uniform RPC return type.
- Bundled prompt assets via `include_str!("../agent/prompts/SOUL.md" | "IDENTITY.md")`.

## Used by

- `src/core/all.rs` — extends the global controller + schema registries with the workspace controllers (the only external consumer found in-tree). The CLI/JSON-RPC surface reaches `init_workspace` and the persona RPCs through that registry rather than direct calls.

## Notes / gotchas

- `bundled_default_contents` is the single allowlist gate: membership there is both "what may be edited from the Persona surface" and "what to restore on reset", so a caller can never read or clobber an arbitrary workspace path (path-traversal names like `../escape.md` and case variants like `soul.md` are rejected).
- `read_workspace_file` deliberately avoids a `metadata().len()` pre-check (TOCTOU-prone) and instead reads through `take(MAX_WORKSPACE_FILE_BYTES + 1)`, capping bytes held regardless of races; an over-cap file is refused, non-UTF-8 is rejected.
- `WorkspaceFile` intentionally omits the absolute on-disk path to avoid leaking host filesystem layout over RPC.
- This is a stateless-handler domain: no `store.rs`, `tools.rs`, `bus.rs`, or `types.rs` — no agent tools, no event-bus subscribers, no persisted in-memory state.
- The module's own `init_workspace` is unrelated to `keyring::init_workspace` (same name, different domain).
