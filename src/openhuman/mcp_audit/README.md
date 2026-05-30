# mcp_audit

Persistent audit log for MCP write-tool calls. When the MCP server dispatches a write tool (e.g. `memory.store`, `tree.tag`) on behalf of an external MCP client (Claude Desktop, Cursor, …), this domain records the attempt — successful or failed — into the local workspace SQLite database and exposes a read-only query surface over those records. The audit table lives inside the existing memory-tree chunk database, reusing the same per-workspace persistence rather than opening a new file.

## Responsibilities

- Record every MCP write-tool attempt: timestamp, originating client, tool name, an arg summary, the resulting chunk id (if any), success flag, and error message.
- Truncate over-long error messages at a UTF-8 char boundary (cap 1024 bytes) before persisting.
- Query the audit log newest-first with optional filters: `since_ms`, exact `client_filter`, exact `tool_filter`, `success_only`, plus `limit`/`offset` paging (default limit 50, hard cap 500).
- Expose the query as an internal-only RPC controller (`mcp_audit.list`).

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/mcp_audit/mod.rs` | Export-only module surface; re-exports store fns, types, and the controller-schema pair. |
| `src/openhuman/mcp_audit/types.rs` | Serde types: `NewMcpWriteRecord` (insert), `McpWriteRecord` (row), `McpWriteListQuery` (filters/paging). |
| `src/openhuman/mcp_audit/store.rs` | Persistence: `record_write` (INSERT) and `list_writes` (filtered SELECT), arg/error serialization, limit/offset normalization, row mapping. Includes the test suite. |
| `src/openhuman/mcp_audit/schemas.rs` | Controller schema for `mcp_audit.list` + `handle_list` handler delegating to `store::list_writes`. Internal-controller registration. |

## Public surface

From `mod.rs`:
- Types: `McpWriteRecord`, `NewMcpWriteRecord`, `McpWriteListQuery`.
- Store fns: `record_write(&Config, NewMcpWriteRecord) -> Result<i64>`, `list_writes(&Config, &McpWriteListQuery) -> Result<Vec<McpWriteRecord>>`.
- Schema/controller exports: `all_mcp_audit_controller_schemas`, `all_mcp_audit_registered_controllers`, `all_mcp_audit_internal_controllers`, `mcp_audit_schemas`.

## RPC / controllers

- **`openhuman.mcp_audit_list`** (namespace `mcp_audit`, function `list`) — lists write-attempt audit records (successes and rejected/failed attempts) ordered by `timestamp_ms` descending.
  - Inputs (all optional): `limit` (u64, default 50, max 500), `offset` (u64), `since_ms` (u64), `client_filter` (string, exact), `tool_filter` (string, exact), `success_only` (bool).
  - Output: `records` — array of `McpWriteRecord`.
- **Internal-only.** Registered via `all_internal_controllers` and wired into `src/core/all.rs` through `all_mcp_audit_internal_controllers()`. Per `src/core/all_tests.rs`, the method is routable internally (`schema_for_rpc_method`) but **not** exposed publicly via `rpc_method_from_parts`.

## Persistence

- Table `mcp_writes` in the memory-tree chunk SQLite DB; schema and indexes (`idx_mcp_writes_timestamp`, `idx_mcp_writes_client`, `idx_mcp_writes_tool`) are created in `src/openhuman/memory_store/chunks/store.rs`, not here.
- Columns: `id`, `timestamp_ms`, `client_info`, `tool_name`, `args_summary` (JSON text), `resulting_chunk_id`, `success` (0/1), `error_message`.
- Connections are obtained via `chunk_store::with_connection(config, …)`; this module does not own a DB handle.

## Dependencies

- `crate::openhuman::config::Config` — workspace location used to resolve the DB.
- `crate::openhuman::config::rpc` (`load_config_with_timeout`) — loads config in the RPC handler.
- `crate::openhuman::memory_store::chunks::store` — provides `with_connection`; the audit table is co-located in the chunk DB.
- `crate::core::all` (`ControllerFuture`, `RegisteredController`) and `crate::core` (`ControllerSchema`, `FieldSchema`, `TypeSchema`) — controller/schema plumbing.
- External crates: `rusqlite`, `serde`/`serde_json`, `anyhow`.

## Used by

- `src/openhuman/mcp_server/write_dispatch.rs` — calls `mcp_audit::record_write` after each write attempt and `mcp_audit::list_writes` for its own surfaces/tests.
- `src/openhuman/mcp_server/tools_tests.rs` — reads back audit rows in tests.
- `src/core/all.rs` — registers the internal controller.
- `src/openhuman/tool_registry/ops.rs` — independently `COUNT`s `mcp_writes` rows (queries the shared table directly, not via this module's API).

## Notes / gotchas

- Filters `client_filter`/`tool_filter` are trimmed and dropped when empty (`normalized_filter`); matching is exact, not prefix/substring.
- `limit`/`offset` are `u64` in the query but converted to `i64` for SQLite (`u64_to_i64` errors on overflow); `limit` is clamped to `MAX_LIST_LIMIT` (500), default 50.
- Error messages longer than 1024 bytes are truncated at the nearest preceding UTF-8 char boundary, so multibyte error text stays valid.
- The table itself is created by the memory_store chunk DB migration — this module assumes it already exists and will fail if that migration hasn't run.
- Logging uses the grep-friendly `[mcp_audit]` prefix at `debug`/`trace`/`warn`.
