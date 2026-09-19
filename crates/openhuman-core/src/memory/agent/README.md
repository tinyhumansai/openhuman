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
| `tools.rs` | `call_memory_agent` tool implementation, re-exported from [`tools/mod.rs`](../../tools/mod.rs) (`pub use crate::memory::agent::tools::*;`) |
| `memory_loader.rs` | What is left of the old per-turn memory loader: `CROSS_CHAT_HEADER` (the `[Cross-chat context]` block header, bound by `agent/harness/memory_context.rs` and the orchestrator prompt so the wording cannot drift), `MemoryCitation`, and `collect_recall_citations` (called from `agent/session_host/turn/core_turn.rs`; the citations reach `web_chat`'s reply presentation). The per-turn `load_context()` block — two full scans of the `global` namespace every turn — was removed from `core_turn.rs`; see [`memory/auto_recall/mod.rs`](../auto_recall/mod.rs) for what replaced it. The `PRIOR_CONVERSATION_*` constants are leftovers of that block and have no caller. |

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

The built-in agent is registered at `crates/openhuman-core/src/memory/agent/agent/`:
- `agent.toml` — tool allowlist, model hint, iteration cap
- `prompt.rs` — dynamic prompt builder
- `prompt.md` — system prompt archetype

`agent.toml`'s `[tools] named` allowlist is `memory_recall`, `memory_tree`, `query_memory`, `memory_doctor`, `memory_flavour`, and `ask_user_clarification`. `memory_tree` is the [`memory/query/`](../query/) dispatcher, so its `walk`/`smart_walk`/`search_entities`/`query_source`/`cover_window`/`drill_down`/`fetch_leaves` modes are all reachable; `prompt.md` steers the model to `walk` first. The allowlist is per tool name, not per mode, so the write mode `ingest_document` is technically callable too — nothing in `memory/query/` checks `current_sandbox_mode`, and `sandbox_mode = "read_only"` is only consulted by the Composio tools and by orchestration's write-capability checks, not by memory writes. The agent is read-only by prompt and by intent (`prompt.md` never mentions ingest), not by enforcement.
