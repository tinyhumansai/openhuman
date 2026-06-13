# startup

Generic OpenHuman process-startup helpers. Currently a thin, stateless module whose sole job is to run one-shot workspace migrations during core boot. It centralizes "do this once when the process comes up" logic so the transport layer (`src/core/jsonrpc.rs`) can fire it without owning migration details. Failures are logged and never abort startup — individual migration helpers own their own idempotency markers.

## Responsibilities

- Run workspace migrations at process startup via `run_workspace_migrations(workspace_dir)`.
- Drive the **session-layout** migration (`agent::harness::session::migrate_session_layout_if_needed`) and log its outcome (jsonl/md moved, pruned legacy dirs, warnings).
- Drive the **welcome-to-orchestrator** thread/artifact migration (`threads::migrate_welcome_agent_artifacts`) and log its outcome (threads/transcripts updated, files renamed).
- Swallow migration errors (log `warn`) and fall back to in-place legacy reads so boot always proceeds.

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/startup/mod.rs` | Export-only: module docstring + `pub mod ops;` + re-export of `run_workspace_migrations`. |
| `src/openhuman/startup/ops.rs` | Implementation of `run_workspace_migrations` — orchestrates the two workspace migrations and logging. |

## Public surface

- `run_workspace_migrations(workspace_dir: &Path)` (re-exported from `ops`) — the only public entry point. Returns nothing; all failure handling is internal/logged.

## RPC / controllers

None. The module exposes no controller schemas or `handle_*` functions; it is invoked directly by the transport boot path, not via the controller registry.

## Agent tools

None (no `tools.rs`).

## Events

None (no `bus.rs`); does not publish or subscribe to `DomainEvent`s.

## Persistence

No own state/store. It triggers migrations that mutate on-disk workspace artifacts (session layout files, thread/transcript artifacts) under `workspace_dir`, but the actual persistence and idempotency markers live in the called migration helpers (`agent::harness::session`, `threads`).

## Dependencies

- `crate::openhuman::agent::harness::session::migrate_session_layout_if_needed` — performs the session-layout migration (moves jsonl/md, prunes legacy dirs).
- `crate::openhuman::threads::migrate_welcome_agent_artifacts` — performs the welcome-agent → orchestrator artifact migration.

## Used by

- `src/core/jsonrpc.rs` (core boot path) — the only caller; invokes `run_workspace_migrations(&workspace_dir)` during core startup, after approval-gate wiring and before MCP registry boot-spawn.

## Notes / gotchas

- **Non-fatal by design**: every migration branch logs and continues; a failed migration must never block startup.
- **Idempotency is delegated**: this module does not track whether a migration already ran — it relies on each helper's own `already_done` markers. Re-running is safe.
- **Grep-friendly log prefixes**: `[runtime]` for session-layout, `[migration::welcome-to-orchestrator]` for the thread/artifact migration.
- Module is intentionally minimal — no `types.rs`/`store.rs`/`schemas.rs` because it holds no domain types, no persisted state, and no RPC surface.
