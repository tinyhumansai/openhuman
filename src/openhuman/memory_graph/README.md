# memory_graph

Placeholder over `mem_tree_entity_index`. Derives entity relationships
on demand instead of writing a parallel triple-store table.

**Premise**: the graph IS the tree mapped out. Two entities that
co-occur on the same tree node form an edge; weight is the count of
distinct shared nodes.

## API

| Function | Returns |
| --- | --- |
| `co_occurring_entities(config, subject, limit)` | `Vec<GraphEdge>` sorted by weight DESC, then object ASC. |
| `neighbors(config, subject, limit)` | `Vec<String>` — neighbor entity ids only. |
| `query::group_by_weight(edges)` | `HashMap<u32, Vec<String>>` for UIs that want strong vs weak buckets. |

## Layout

| Path | Role |
| --- | --- |
| [`mod.rs`](mod.rs) | Module root + re-exports. |
| [`types.rs`](types.rs) | `GraphEdge { subject, object, weight }`. |
| [`query.rs`](query.rs) | Co-occurrence SELF-JOIN over `mem_tree_entity_index`. Tests. |

## Layer rules

- Read-only. No new tables, no new schema. Everything derives from the
  entity index that the tree summariser already maintains.
- Reads through `memory_store::chunks::store::with_connection` — same
  SQLite connection used by the rest of memory_store.
- Intentionally does **not** cover the LLM-extracted
  `(subject, predicate, object)` triples that ingestion writes via
  `unified::graph::graph_upsert_namespace`. That surface needs a
  separate decision (drop it, or persist triples as md files).
