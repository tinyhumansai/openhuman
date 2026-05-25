# memory_archivist

Bridge from chat conversation to memory tree. One responsibility: take a
sequence of turns, drop tool-call noise, and append the cleaned blob to
a tree as a single leaf. The tree handles everything downstream
(summarisation, retrieval, vector embedding).

## Flow

```text
Vec<Turn>                                (raw conversation, tool calls inline)
   │
   ▼
clip::clean_conversation()               (drop tool_calls_json, drop "tool" turns)
   │
   ▼
compose::compose_conversation_md()       (one md blob: ## role\n<content>\n... per turn)
   │
   ▼
tree_writer::archive_to_tree()           (append_leaf to memory_tree)
   │
   ▼
memory_store::trees                      (cascade seal, summary nodes)
```

## API

| Function | Purpose |
| --- | --- |
| `clean_conversation(&[Turn]) -> Vec<Turn>` | Pure transform — strips `tool_calls_json` and drops `role == "tool"` turns. |
| `compose_conversation_md(&[Turn]) -> String` | Pure transform — yields the markdown blob that becomes one tree leaf. |
| `archive_to_tree(config, &Tree, session_id, &[Turn]) -> TreeWriteOutcome` | End-to-end: clean → compose → `append_leaf`. Returns sealed-summary ids from any cascade. |

## Layout

| Path | Role |
| --- | --- |
| [`mod.rs`](mod.rs) | Module root + re-exports. |
| [`types.rs`](types.rs) | `Turn { role, content, tool_calls_json, timestamp }`. |
| [`clip.rs`](clip.rs) | `clean_conversation` + tests. |
| [`compose.rs`](compose.rs) | `compose_conversation_md` + tests. |
| [`tree_writer.rs`](tree_writer.rs) | `archive_to_tree` — the end-to-end orchestration. Chunk id = `sha256(session_id ‖ md)[..32]`. |

## Why "clip"?

Tool-call JSON is verbose, model-specific, and rarely meaningful out of
context. Tool-result turns are noisy (stdout dumps, JSON responses) and
distort vector embeddings of the surrounding human conversation.
Stripping both before the conversation lands in the tree keeps
summaries focused on natural-language content.

## Layer rules

- Depends only on `memory_store::trees` (the `Tree` type) and
  `memory_tree::tree::bucket_seal::append_leaf` (the write contract).
- No SQLite. No on-disk md storage of its own — the tree owns
  persistence.
- Replaces the legacy `unified::fts5` per-turn capture path. Callers
  migrating from the archivist hook should batch turns and call
  `archive_to_tree` at conversation boundaries.
