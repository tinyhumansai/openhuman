# agent_memory

Memory agent domain — owns the retrieval-focused memory agent, its prompt, and performance instrumentation for memory tree walking and chunk retrieval.

## Purpose

The memory agent is a specialist sub-agent that navigates the user's memory tree to answer questions. It combines multiple retrieval strategies:

1. **Vector search** — semantic similarity across all stored embeddings
2. **Keyword search** — pattern matching across raw content files on disk
3. **Entity search** — canonical entity lookup and relationship following
4. **Tree browse** — hierarchical navigation of time-based summary trees
5. **Content read** — direct file reads from raw/wiki/episodic/document stores
6. **Source listing** — discovery of available sources and content types

## Module layout

| File | Role |
|------|------|
| `mod.rs` | Module declarations and re-exports |
| `types.rs` | Benchmark and performance tracking types |
| `ops.rs` | Benchmarking harness for memory walk performance |
| `tools.rs` | `call_memory_agent` tool implementation |

## Memory tree structure

The memory tree lives at `{workspace}/memory_tree/content/` with this layout:

```text
content/
├── chat/              # Conversation chunks (by source)
│   └── conversations-agent/
│       └── {hash}.md
├── episodic/          # Session/subconscious episode chunks
│   └── {session_id}/
│       └── {hash}.md
├── raw/               # Raw ingested documents (GitHub, Gmail, etc.)
│   └── {source-slug}/
│       └── {hash}.md
└── wiki/              # Summary tree (hierarchical)
    └── summaries/
        └── {namespace}/
            └── {level}/{node_id}.md
```

## Benchmarking

Use the benchmark script to measure retrieval performance:

```bash
# Run default benchmark queries against the staging memory tree
./scripts/bench-memory-walk.sh

# Custom queries
./scripts/bench-memory-walk.sh --query "what did I discuss about OpenHuman?" --max-turns 15

# Custom content root
./scripts/bench-memory-walk.sh --content-root /path/to/memory_tree/content
```

## Agent definition

The built-in agent is registered at `src/openhuman/agent_memory/agent/`:
- `agent.toml` — tool allowlist, model hint, iteration cap
- `prompt.rs` — dynamic prompt builder
- `prompt.md` — system prompt archetype

The agent has access to the full memory retrieval tool surface: `memory_tree` (with deterministic E2GraphRAG `walk`/`smart_walk` modes plus `search_entities`/`query_source`/`cover_window`/`drill_down`/`fetch_leaves`), `memory_recall`, and `query_memory`.
