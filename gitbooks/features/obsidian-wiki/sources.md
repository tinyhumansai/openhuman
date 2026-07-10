---
description: >-
  The typed registry of connectors that feed your Memory Tree (local folders,
  GitHub repos, RSS feeds, web pages, and Composio OAuth integrations), plus
  per-agent source scoping for privacy and focus.
icon: database
---

# Memory Sources & Scoping

A **memory source** is a configured connector that feeds the [Memory Tree](memory-tree.md). Where the tree owns _"how do I store and summarize?"_, the `memory_sources` domain (`src/openhuman/memory_sources/`) owns the upstream question: **"what feeds my memory?"** It is a typed registry of connectors, persisted in `config.toml` under `[[memory_sources]]`, with CRUD at runtime, a uniform reader abstraction, per-source sync status, and the `openhuman.memory_sources_*` RPC surface.

The domain only _defines connectors and reads from them_. The ingestion engine and sync scheduling live in `memory` / `memory_sync`; sources dispatch work to the right backend.

***

## Source kinds

Every source is a single flat `MemorySourceEntry` (`src/openhuman/memory_sources/types.rs`) whose `kind` discriminator (the `SourceKind` enum) decides which fields are required. Validation is enforced at add/update time by `validate()`, not the type system. The kinds:

| Kind             | `SourceKind`   | What it ingests                                                                              |
| ---------------- | -------------- | -------------------------------------------------------------------------------------------- |
| **Composio**     | `Composio`     | An OAuth-connected SaaS integration (Gmail, Slack, Notion, …); sync is provider-driven.     |
| **Conversation** | `Conversation` | The agent's own conversation transcripts.                                                    |
| **Folder**       | `Folder`       | A local directory, globbed (default `**/*.md`, 10 MB/file cap) with a path-traversal guard.  |
| **GitHub repo**  | `GithubRepo`   | Project activity (commits, issues, PRs) via the `gh` CLI or a public REST fallback.          |
| **RSS feed**     | `RssFeed`      | RSS/Atom feed items.                                                                         |
| **Web page**     | `WebPage`      | A fetched web page, optionally narrowed by a CSS `selector`.                                 |
| **Twitter query**| `TwitterQuery` | A saved Twitter query. The reader is scaffolded; sync is intentionally unimplemented pending creds. |

Each entry also carries optional per-sync budgets (`max_tokens_per_sync`, `max_cost_per_sync_usd`, `sync_depth_days`) so a chatty source can't blow up your token spend on one run.

***

## Adding and configuring sources

Sources are CRUD-ed through the `memory_sources` controllers (`src/openhuman/memory_sources/schemas.rs` → `rpc.rs`), namespace `openhuman.memory_sources_*`:

| RPC           | Purpose                                                             |
| ------------- | ------------------------------------------------------------------ |
| `list`        | List configured sources (lazily reconciles Composio first).        |
| `get`         | Fetch one source by `id`.                                          |
| `add`         | Add a source; kind-specific fields are flat on the request.        |
| `update`      | Partial update via `MemorySourcePatch`.                           |
| `remove`      | Delete a source by `id`.                                          |
| `list_items`  | List readable items from a source via its reader.                  |
| `read_item`   | Read one item's content.                                          |
| `sync`        | Queue a manual sync (returns immediately; progress via events).    |
| `status_list` | Per-source sync status.                                            |

All mutations reload the live `Config`, apply the change, and `config.save()` atomically (`registry.rs`). In the desktop app these surface in the Intelligence / Memory tab alongside the [Auto-fetch](auto-fetch.md) cadence.

***

## The reader abstraction

Every kind implements one async trait, `SourceReader` (`src/openhuman/memory_sources/readers/mod.rs`):

```rust
#[async_trait]
pub trait SourceReader: Send + Sync {
    fn kind(&self) -> SourceKind;
    async fn list_items(&self, source, config) -> Result<Vec<SourceItem>, String>;
    async fn read_item(&self, source, item_id, config) -> Result<SourceContent, String>;
}
```

A `reader_for(kind)` dispatcher hands back the right implementation (`FolderReader`, `GithubReader`, `RssReader`, `WebPageReader`, etc.). On a manual `sync`, reader-backed kinds walk `list_items` and ingest each item through `memory::ingest_pipeline::ingest_document` (`sync.rs`); Composio sources delegate wholesale to `memory_sync::composio::run_connection_sync` rather than reading item-by-item, so `ComposioReader::read_item` is an explanatory placeholder.

***

## Sync status & freshness

`status.rs` computes a `SourceStatus` per source by querying `mem_tree_chunks` (chunks synced/pending, last-chunk timestamp) using a `source_id LIKE` prefix: `mem_src:{id}:%` for reader kinds, `{toolkit}:%` for Composio. Each source gets a `FreshnessLabel`:

- **Active**: last chunk ≤ 30 s ago.
- **Recent**: last chunk ≤ 5 min ago.
- **Idle**: older, or no chunks yet.

Sync progress streams as `MemorySyncStageChanged` events (Requested → Fetching → Stored → Ingesting → Completed/Failed), tagged with `connection_id = Some(source.id)`, so the UI can show live progress without polling. `status_list` degrades a per-source query failure to an `Idle` zero-row entry rather than failing the whole call.

**Composio auto-upsert.** When an OAuth connection is created, `memory_sync::composio::bus` calls `upsert_composio_source`, so freshly-connected integrations appear as sources with no restart. `list_rpc` also performs a lazy reconciliation (`reconcile::ensure_composio_sources`) on every list, catching connections made before this hook existed.

***

## Source scoping for agent profiles

By default an agent recalls from **every** source. Source scoping lets an agent profile restrict recall to a whitelist of source ids, so a customer-support flavour never surfaces your personal Gmail, and a research flavour stays focused on the repos and feeds that matter. This is a privacy and focus control, not just a relevance tweak.

The mechanism lives in `src/openhuman/memory/source_scope.rs`. Threading an allowlist through every memory tool and the deep `select_trees` retrieval layer would touch dozens of call sites. So, mirroring `thread_context`, the channel sets a `tokio::task_local!` around the agent turn and the retrieval layer reads it ambiently, with no explicit plumbing:

- **`None`** (outside any scope, or `with_source_scope(None, …)`) means **unrestricted**. This is the default for cron, sub-agents, the CLI, and any profile that left `memory_sources` unset.
- **`Some(set)`** restricts recall to source scopes in the set. An **empty** set surfaces nothing (the profile selected no sources).

The gate is **tag-discriminated and fail-open** for everything that is not a memory-source chunk. Every source-ingested chunk carries the `memory_sources` tag; the gate (`chunk_source_allowed`) only touches tagged chunks:

- A chunk **without** the `memory_sources` tag (working memory, conversation transcripts, internal chunks) **always passes**, even under an empty allowlist.
- A **tagged** memory-source chunk passes only if its source id is allowed. The id is matched against either the raw `source_id` (Composio / channel scopes like `slack:#eng`) or the registry id extracted from a `mem_src:<id>:<item>` composite (reader-based sources).

So tightening a profile's scope hides its connected sources without ever starving it of its own conversation context.

***

## See also

- [Auto-fetch](auto-fetch.md): the 20-minute cadence that keeps active sources fresh.
- [Memory Trees](memory-tree.md): the pipeline every source feeds into.
- [Obsidian Wiki](README.md): the Markdown vault sources land in.
- [Integrations](../integrations/README.md): connecting the OAuth providers behind Composio sources.
