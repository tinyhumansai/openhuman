# mcp/audit: write-audit RPC surface

RPC surface over the MCP write-audit log. The log itself (its store and
schema) moved to [`tinymcp`](https://github.com/tinyhumansai/tinymcp). What
is here is the `mcp_audit` controller family and the payload types it
speaks, re-exported from the wire contract.

## Where the rows are

The audit table used to be created inside this application's memory-tree
chunk database, which was precisely what made it unmovable. It has its own
file now, `<workspace>/mcp_audit/mcp_audit.db`, under the same workspace
directory. Rows written before the move stay in the old table: an audit log
is history rather than operational state, and nothing reads that table any
more.

## Key files

| File | Role |
| --- | --- |
| [`mod.rs`](./mod.rs) | Facade: re-exports the payload types (`types` module, from `tinymcp_bus`), `record_write`/`list_writes` (delegating to the `mcp::host` service's `AuditStore`), and the schema re-exports. |
| [`schemas.rs`](./schemas.rs) / [`schemas_tests.rs`](./schemas_tests.rs) | The `mcp_audit.list` controller: schema (`limit`/`offset`/`since_ms`/`client_filter`/`tool_filter`/`success_only` inputs, `records` output) and its handler, which loads the config, deserializes the params into `McpWriteListQuery`, and calls `list_writes`. |
| [`stub.rs`](./stub.rs) | The `mcp`-less mirror: `record_write` is a no-op returning `Ok(0)`, `list_writes` returns `Ok(vec![])`. |

## RPC surface

`mcp_audit.list` is the only controller, registered through
`all_mcp_audit_internal_controllers` in `build_internal_only_controllers`
([`core/all.rs`](../../core/all.rs), ~line 1020): routable over RPC for the desktop UI/CLI, but
not exposed to agents. The handler deserializes its params straight into
`tinymcp_bus::McpWriteListQuery` and hands them to `list_writes`; the
filter and limit semantics (`limit` default 50 / max 500 via
`resolved_limit`, `offset`, `since_ms`, `client_filter`, `tool_filter`,
`success_only`) are the contract's and the store's, not this layer's. The
schema's field comments repeat them for `/schema` readers only.

## Compile-time gate (`mcp` feature)

`pub mod audit;` is always compiled. It is a facade. The RPC surface
(`schemas`) is gated; with the `mcp` feature off, `stub` mirrors the
consumed surface (`record_write`, `list_writes`,
`all_mcp_audit_internal_controllers`) so the audited subsystem simply
appears never to have happened, rather than erroring.

## Dependencies

- `tinymcp` (`AuditStore`): the write-audit store, opened per workspace by
  `crate::mcp::host`.
- `tinymcp_bus`: `McpWriteListQuery`, `McpWriteRecord`, `NewMcpWriteRecord`.
- `crate::mcp::host`: resolves the per-workspace `AuditStore` via
  `for_config`.
- `crate::core::all` (`RegisteredController`, `ControllerFuture`) and
  `crate::core` (`ControllerSchema`, `FieldSchema`, `TypeSchema`): the
  controller registration types. Unlike the other RPC domains the handler
  returns a raw `{ "records": [...] }` object, not an `Outcome`.

## Used by

- [`crates/openhuman-core/src/mcp/server/write_dispatch.rs`](../server/write_dispatch.rs): `audit_write`
  / `audit_write_rejection[_without_config]` call `audit::record_write` off
  the hot path (`spawn_blocking`, or a thread when no runtime is current)
  for every MCP write-tool attempt, success and rejection; `record_write`
  resolves the per-workspace `mcp::host` service and writes through its
  `AuditStore`.
- `crates/openhuman-core/src/core/all.rs`: registers
  `all_mcp_audit_internal_controllers()`.

## Further reading

- [Parent module README](../README.md)
- [MCP registry](../../../../../docs/gitbooks/en/developing/architecture/mcp-registry.md)
- [Security architecture](../../../../../docs/gitbooks/en/developing/architecture/security.md)
