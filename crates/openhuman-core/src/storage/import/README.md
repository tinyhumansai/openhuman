# session_import

Host side of the legacy-session import.

The importer is `crate::storage::import::sessions`, run at boot (and when the
desktop rebinds its workspace to a different user). It reads a classic
workspace's `session_db/sessions.db`, `session_raw/*.jsonl` transcripts and
turn-state JSON without changing those sources, then writes the records through
the current TinyAgents storage ports. Database rows use TinyAgents' typed
legacy batch API; the host owns file discovery and replay. Imports preserve
IDs and are only-if-absent. See the storage README for the agent each record
lands in.

What stays in OpenHuman:

- [`schemas.rs`](./schemas.rs): the `session_import.run` controller
  (`openhuman-core session_import run`, JSON-RPC `openhuman.session_import_run`),
  a manual retry of that import for a workspace. `dry_run` reports which legacy
  sources exist and writes nothing.
- [`live.rs`](./live.rs): the agent's key-value store registered on each turn's
  `RunContext.stores` under `openhuman_sessions`.

The file-era dual-write of transcripts into a second layout, its shadow read
and the `OPENHUMAN_SESSION_DUAL_WRITE` / `OPENHUMAN_SESSION_SHADOW_READS` kill
switches are gone: the backend is the only record. The
`AgentConfig::session_dual_write` and `session_shadow_reads` fields remain so
existing `config.toml` files still load; they have no effect.

## Further reading

- [Parent module README](../README.md)
- [Agent harness architecture](../../../../../../docs/gitbooks/en/developing/architecture/agent-harness.md)
