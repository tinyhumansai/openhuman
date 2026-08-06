# memory_sync

OpenHuman orchestration and product policy around the TinyCortex sync engine.

TinyCortex owns generic Composio provider fetch/pagination, canonical memory
records, sync budgets/state, workspace reconciliation, and persistence traits.
The live host path calls it through `src/openhuman/memory/tinycortex/sync.rs` from
`composio::run_connection_sync` and the default provider `sync()` method.

OpenHuman retains:

- periodic scheduling and connection selection;
- credentials and Composio action execution;
- source-scope and redaction policy;
- translation into host `DomainEvent`s;
- JSON-RPC/status/connect surfaces;
- agent-facing action tools and result post-processing;
- product task/profile projections for GitHub, Notion, Linear, and ClickUp;
- local workspace watching and MCP orchestration.

The provider directories therefore are not alternate sync engines. Their
remaining `provider.rs`, `tools.rs`, `normalization.rs`, profile, catalog, and
post-processing files implement host product surfaces over the crate-backed
sync path. New generic parsing or persistence behavior belongs in
`vendor/tinycortex/src/memory/sync/`.

D4.1-D4.4 are closed in `docs/tinycortex-drift-ledger.md`. Gmail's bounded
25-message page is crate-owned and prevents Composio 413 responses.
