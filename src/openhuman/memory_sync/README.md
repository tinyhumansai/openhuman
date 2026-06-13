# memory_sync

Every "pull data from upstream → land it in memory_store" pipeline in
one place, organised by the kind of upstream they talk to.

## Three pipeline kinds

| Kind | Submodule | Owns |
| --- | --- | --- |
| **Composio**  | [`composio/`](composio/)  | Per-provider sync via the Composio Edge API: gmail, slack, github, notion, linear, clickup, … |
| **Workspace** | [`workspace/`](workspace/) | Vault file watch, harness turn capture, dictation transcripts — anything local. |
| **MCP**       | [`mcp/`](mcp/)            | Third-party MCP servers via `mcp_clients/` transport. |

## Trait

Every pipeline implements [`SyncPipeline`]:

```rust
async fn init(&self, &Config)  -> anyhow::Result<()>;
async fn tick(&self, &Config)  -> anyhow::Result<SyncOutcome>;
fn id(&self) -> &str;
fn kind(&self) -> SyncPipelineKind;
```

`SyncOutcome { records_ingested, more_pending, note }` is the
orchestrator-facing result; pipelines own their own pagination cursors
and retry policy behind that.

## Layout

| Path | Role |
| --- | --- |
| [`mod.rs`](mod.rs) | Module root + re-exports. |
| [`traits.rs`](traits.rs) | `SyncPipeline`, `SyncOutcome`, `SyncPipelineKind`. |
| [`composio/`](composio/) | Per-provider pipelines (gmail, slack, github, notion, linear, clickup). |
| [`workspace/`](workspace/) | Vault, harness, dictation pipelines. |
| [`mcp/`](mcp/) | MCP-server pipelines (one per connected server). |

## Schedulers & self-healing

Three background loops keep memory synced and compressed without manual
"Sync now" clicks:

| Loop | Module | Cadence | Covers |
| --- | --- | --- | --- |
| Composio periodic | [`composio/periodic.rs`](composio/periodic.rs) | 20-min tick, per-connection interval | Composio connections (gmail, slack, …) |
| Workspace periodic | [`workspace/periodic.rs`](workspace/periodic.rs) | 20-min tick, `memory_sync_interval_secs` (24h default, `0` = manual only) | `github_repo`, `folder`, `rss_feed`, `web_page` sources |
| Queue scheduler | `memory_queue::scheduler` | 3 h | flush stale L0 buffers + requeue transient-failed jobs |

**Raw-archive coverage** ([`sources/rebuild.rs`](sources/rebuild.rs)): every
summary batch records its raw files in `mem_tree_ingested_sources`
(`source_kind = "raw_file"`). After each sync, `check_and_rebuild_tree`
diffs disk vs gate and summarises only the uncovered remainder — so an
interrupted sync self-heals on the next pass instead of stranding raw
files forever. Inspect/trigger over RPC with
`openhuman.memory_sources_reconcile`.

⚠️ A source's **tree scope** and **raw-archive id** slugify to different
directories for GitHub (`github:owner/repo` vs `github.com/owner/repo`) —
always carry both (`memory_sources::sync::SourceScope`).

## Status

**Pipelines: scaffold only.** Today's sync code still lives in:

- `composio/providers/<provider>/ingest.rs` + `bin/{slack_backfill,gmail_backfill_3d}.rs`
- `vault/sync.rs`, `agent_experience/`, `dictation_hotkeys/`
- `mcp_clients/` (transport only; no drain loop yet)

Each migrates here as its own per-pipeline PR. The job-queue orchestration
in `memory::jobs` stays put — it just gains the ability to iterate over a
registered `Vec<Box<dyn SyncPipeline>>`.

## Layer rules

- Sync writes go through `memory::ingest_pipeline` so every record
  lands as raw md → chunks → tree leaves like any other ingest.
- No direct writes into trees or unified. No upstream-specific data
  models leak past the pipeline boundary.
- One pipeline per upstream service. Composio's GitHub and MCP's GitHub
  are distinct pipelines because they hit different surfaces with
  different cadence and auth.
