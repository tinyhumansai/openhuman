---
description: >-
  How OpenHuman reads back from the memory tree. A handful of deterministic
  retrieval primitives, canonical entity resolution, a co-occurrence graph, and
  a specialist memory sub-agent that combines them.
icon: search
---

# Memory Retrieval & Recall

The [Memory Tree](memory-tree.md) is the write path: it folds the stream of your day into chunks, scores, and hierarchical summary trees on disk. **Retrieval** is the read path - how the agent finds the right node, hydrates the right raw chunk, and resolves "Alice" to a stable id before answering you.

There is deliberately **no classifier, gate, or composer** in the retrieval layer. The primitives are deterministic and scope-specific; deciding *which* primitive to call and *how* to combine results is left to the calling agent (or, for the deterministic `walk`, to a pure routing algorithm). Source: `src/openhuman/memory_tree/retrieval/mod.rs`.

***

## The `memory_tree` tool: one mode dispatcher

The agent-facing surface is a single multi-mode tool named `memory_tree` (`src/openhuman/memory/query/mod.rs`). Its `mode` field routes to one underlying implementation. Every retrieval mode returns the same `RetrievalHit` shape so the model sees a uniform schema regardless of which mode ran.

| Mode | What it's for | Typical use |
| --- | --- | --- |
| `search_entities` | Fuzzy `LIKE` lookup over the canonical entity index; resolves a surface name to a canonical id. | Call **first** when the user mentions someone by name ("what did Alice say?"). |
| `query_source` | Per-source summary retrieval filtered by source kind + time window, with optional semantic rerank. | "Summarise my Slack #eng from last week." |
| `drill_down` | BFS walk of a summary node's `child_ids`, one or more levels down, with optional rerank. | Expand a coarse summary into its finer-grained children. |
| `cover_window` | Minimum set of nodes covering a `[since_ms, until_ms]` time window. | "Last 24h" recaps and other time-bounded catch-ups. |
| `fetch_leaves` | Batch hydration of raw leaf chunks by id (cap 20). | Pull exact source text for citation after a summary hit. |
| `ingest_document` | Write a document into the tree for future retrieval (the one **write** mode). | Persist a fetched web page / GitHub file; re-ingesting the same `source_id` replaces old chunks. |
| `walk` / `smart_walk` | Deterministic E2GraphRAG retrieval - extracts query entities, routes between the entity graph and dense summaries with **no LLM**, returns ranked evidence. | Answer a natural-language question in one shot without an agent loop. |

The historical `query_global` and `query_topic` modes were **removed**: source trees hold all the content, and walking the source hierarchy plus the entity index reconstructs both the time and topic projections (`mod.rs` dispatcher tests assert their absence).

***

## The `RetrievalHit` shape

Every primitive emits `RetrievalHit` (`src/openhuman/memory_tree/retrieval/types.rs`). The important fields:

- `node_id`, `node_kind` - `leaf` (a raw `mem_tree_chunks` row) or `summary` (a sealed `mem_tree_summaries` row). Consumers branch on this (e.g. "only `drill_down` on summaries").
- `tree_id` / `tree_kind` / `tree_scope` / `level` - provenance, so a UI can say "from Slack #eng".
- `content` - the snippet (summary text or raw chunk body).
- `entities` / `topics` - canonical ids and tags carried on the node.
- `time_range_start` / `time_range_end` - RFC3339, so hits from different tools sort on a common axis.
- `score` - relevance.
- `child_ids` - next level down (empty on leaves); the cursor for `drill_down`.
- `source_ref` - back-pointer to the original source (populated on leaves).

Query-style modes wrap hits in a `QueryResponse { hits, total, truncated }` where `total` is the pre-truncation match count, so the agent can tell whether a higher-limit follow-up would surface more.

***

## Entity resolution and canonical ids

Names are messy; ids are not. Before answering a question about a person, the agent resolves the surface form to a **canonical id** like `person:alice` or `email:alice@example.com`.

- `search_entities` does the fuzzy lookup over the entity index that the tree summariser maintains.
- The canonical registry lives in `src/openhuman/memory_entities/` - one Markdown file per entity at `<content_root>/entities/<kind>/<canonical_id>.md`, with YAML frontmatter (`id`, `kind`, `display_name`, `aliases`, `emails`, `handles`) plus a free-form notes body the user can edit in Obsidian. `lookup_alias` matches by alias / email / handle / display name, case-insensitively.
- `kind` matches `memory_tree::score::extract::EntityKind`, so the ids the scorer emits round-trip through the registry unchanged. The vault is the source of truth - Obsidian, grep, and vector search all see the same data without a separate store.

***

## The entity graph (read-only, derived)

`src/openhuman/memory_graph/` exposes entity relationships **without** a parallel triple-store table. The premise: *the graph is the tree mapped out*. Two entities that co-occur on the same tree node form an edge; the weight is the count of distinct shared nodes.

- `co_occurring_entities(config, subject, limit)` - `GraphEdge { subject, object, weight }` sorted by weight.
- `neighbors(config, subject, limit)` - neighbour ids only.

It is a pure read-only SELF-JOIN over `mem_tree_entity_index` - no new tables, no new schema. This graph is exactly what powers the deterministic `walk` routing below.

***

## Deterministic walk (`walk` / `smart_walk`, no LLM)

`walk` and `smart_walk` both route through `fast_retrieve` (`src/openhuman/memory_tree/retrieval/fast.rs`), an **E2GraphRAG-style** algorithm that replaces the old agentic turn-by-turn loops. It never invokes an LLM. Routing is decided purely by query entities and co-occurrence-graph hop distance:

1. Extract query entities `Eq` (spaCy NLP, regex fallback).
2. `Eq` empty -> **global**: dense rerank over the summary tree.
3. Otherwise compute related entity pairs within `h` hops:
   - none related -> **global with occurrence ranking**: dense top-2k, re-ranked by how many `Eq` entities each summary mentions.
   - related pairs found -> **local**: intersect the entity-index node sets of each pair, tightening `h` while candidates exceed `k`, then rank survivors by entity coverage and recency.

Tunables (`FastRetrieveOptions`): `limit` (`k`, default 10, cap 100), `max_hops` (`h`, default 2, cap 4), and an optional `time_window_days` look-back on the dense branch. Output is a structured `QueryResponse` of hits - no synthesised prose - for a higher-level context agent to consume.

***

## Time-windowed recall

For "what happened in the last 24h" style questions, `cover_window` computes the **minimum set of nodes** that covers `[since_ms, until_ms]` (epoch-millis). Because summary nodes carry `time_range_start` / `time_range_end`, a single high-level summary can cover a whole window without fanning out to every leaf - the agent only drills down or fetches leaves when it needs detail or a citation.

***

## `memory_recall` - legacy key-value search

Distinct from the tree, `memory_recall` (`src/openhuman/memory/tools/recall.rs`) searches the older namespaced key-value memory: `memory_recall { namespace, query, limit }` over namespaces like `global`, `background`, `autocomplete`, or `skill-{id}`. It returns scored results and is best for exact preference / fact lookups ("does the user prefer dark mode?") that predate the tree.

***

## The memory agent (specialist sub-agent)

`src/openhuman/agent_memory/` owns a specialist retrieval sub-agent invoked via the `call_memory_agent` tool. It navigates the memory tree to answer a question by combining strategies the primitives expose: vector search, keyword search over raw files, entity search and relationship following, hierarchical tree browse, direct content reads, and source listing.

Its tool allowlist (`src/openhuman/agent_memory/agent/agent.toml`) is the full retrieval surface: `memory_tree` (with all the modes above, including deterministic `walk` / `smart_walk`), `memory_recall`, and `query_memory`. The prompt and iteration cap live alongside in `agent/prompt.md` + `agent/prompt.rs`; performance is tracked by the benchmark harness in `ops.rs` (`scripts/bench-memory-walk.sh`).

***

## See also

- [memory-tree.md](memory-tree.md) - the write path that builds the trees retrieval reads.
- [memory-diff.md](memory-diff.md) - how memory changes are tracked over time.
- [README.md](README.md) - feature index for the Obsidian-backed wiki.
- [../subconscious.md](../subconscious.md) - the background loop that consumes recalled context.
