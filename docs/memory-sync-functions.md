# Memory Module — Consumer Reference

How to use the memory layer from services outside `src/openhuman/memory/`.

**Rule**: Always use `MemoryClient` (`src/openhuman/memory/store/client.rs`). Never call `UnifiedMemory` directly — it's internal to the memory module.

---

## Which Function to Use

### Decision Tree

```text
Need to store data?
  |
  +-- Structured key-value pair (config, state, counters)?
  |     -> kv_set()
  |
  +-- Full document (text, sync payload, user content)?
  |     |
  |     +-- Ephemeral / high-frequency (screen captures, ticks)?
  |     |     -> put_doc_light()    (no embedding, no graph extraction)
  |     |
  |     +-- Important content that should be semantically searchable?
  |     |     -> put_doc()           (embeds + background graph extraction)
  |     |
  |     +-- Rich content needing entity/relation extraction NOW?
  |           -> ingest_doc()        (full synchronous pipeline, slower)
  |
  +-- Knowledge graph fact (entity-relation-entity)?
        -> graph_upsert()

Need to read data?
  |
  +-- Have a user query / search string?
  |     -> query_namespace()              (returns text for LLM prompt)
  |     -> query_namespace_context_data() (returns structured data)
  |
  +-- Need recent context, no specific query?
  |     -> recall_namespace()              (returns text)
  |     -> recall_namespace_context_data() (returns structured data)
  |     -> recall_namespace_memories()     (returns individual hits)
  |
  +-- Looking up a specific key?
  |     -> kv_get()
  |
  +-- Listing what exists?
  |     -> list_documents(), list_namespaces(), kv_list_namespace()
  |
  +-- Querying entity relationships?
        -> graph_query()

Need to delete data?
  |
  +-- Single document?       -> delete_document()
  +-- Single KV entry?       -> kv_delete()
  +-- All data for a skill (e.g. on disconnect/revoke)?  -> clear_skill_memory()
  +-- All data in namespace? -> clear_namespace()
```

---

### Writing Data

#### `put_doc()` — General-purpose document storage

Use when content should be **semantically searchable** later. Embeds the content and enqueues background graph extraction.

```rust
client.put_doc(NamespaceDocumentInput {
    namespace: "autocomplete-memory".into(),
    key: Some(format!("completion:{timestamp:018}")),
    title: "Accepted completion".into(),
    content: format!("{context}\n---\n{suggestion}"),
    source_type: Some("autocomplete".into()),
    metadata: Some(json!({ "app_name": app, "timestamp_ms": ts })),
    ..Default::default()
}).await?;
```

**Used by**: Skills JS `memory.insert()`, Autocomplete (searchable completions), Subconscious (working-memory docs).

#### `put_doc_light()` — High-frequency / ephemeral storage

Use when data is **written often** and doesn't need embedding or graph extraction. Much faster than `put_doc()`.

```rust
client.put_doc_light(NamespaceDocumentInput {
    namespace: "vision".into(),
    key: Some(format!("screen_intelligence_{id}")),
    title: format!("Screen: {app_name} — {window_title}"),
    content: yaml_frontmatter_and_text,
    ..Default::default()
}).await?;
```

**Used by**: Screen Intelligence (vision summaries every few seconds).

#### `ingest_doc()` — Full synchronous ingestion

Use when you need the complete pipeline (chunking, embedding, entity extraction, relation extraction) to **finish before proceeding**. Blocks until done. Prefer `put_doc()` unless you need synchronous guarantees.

**Used by**: Skills event loop (ingesting sync content into vector graph).

#### `kv_set()` — Structured key-value storage

Use for **small, structured data** you'll look up by exact key. Not semantically searchable.

```rust
client.kv_set(
    Some("autocomplete"),             // namespace (None = global)
    &format!("accepted:{ts:018}"),    // key
    &json!({ "context": ctx, "suggestion": s }),
).await?;
```

**Used by**: Autocomplete (completion records keyed by timestamp).

#### `graph_upsert()` — Knowledge graph facts

Use to store **entity-relation-entity triples**. You rarely call this directly — `put_doc()` and `ingest_doc()` extract graph relations automatically. Only call `graph_upsert()` when you have an explicit fact without an associated document.

---

### Reading Data

#### `query_namespace()` — Semantic search (text)

Use when you have a **search string** and want relevant content. Returns formatted text ready for LLM prompt injection.

```rust
let context = client.query_namespace(
    "autocomplete-memory",   // namespace
    &last_80_chars,          // query
    10,                      // max chunks
).await?;
```

**Used by**: Autocomplete (matching past completions to current context), Frontend (user-initiated search in MemoryWorkspace).

#### `query_namespace_context_data()` — Semantic search (structured)

Same query, but returns `NamespaceRetrievalContext` with individual hits, entities, relations, and score breakdowns. Use when you need to process results programmatically.

#### `recall_namespace()` — Recent context (text)

Use when you need **recent memory** but don't have a query. Returns most recent docs as formatted text.

```rust
if let Ok(Some(text)) = client.recall_namespace(&ns, 3).await {
    // top 3 chunks of recent context
}
```

**Used by**: Subconscious (fetching context per namespace for situation report).

#### `recall_namespace_memories()` — Recent context (individual hits)

Returns `Vec<NamespaceMemoryHit>` instead of text. Use when you need to iterate hits and inspect scores.

#### `recall_namespace_context_data()` — Recent context (structured)

Returns `NamespaceRetrievalContext`. Use when you need the full retrieval object (hits + entities + relations).

#### `kv_get()` — Exact key lookup

```rust
let val = client.kv_get(Some("autocomplete"), "accepted:000001719000000").await?;
```

#### `kv_list_namespace()` — List all KV entries

Enumerate all key-value pairs in a namespace. Useful for trimming, history display, or bulk operations.

**Used by**: Autocomplete (trimming beyond 50 entries, settings UI, bulk clear).

#### `list_documents()` — List document metadata

Returns JSON with document metadata (not full content). Useful for delta analysis or trimming.

**Used by**: Subconscious (finding docs updated since last tick), Autocomplete (trimming beyond 200 docs).

#### `graph_query()` — Query knowledge graph

Find entity relationships. Filter by optional namespace, subject, and/or predicate.

```rust
let relations = client.graph_query(None, None, None).await?;
```

**Used by**: Subconscious (relations for situation report), Frontend (knowledge graph display).

---

### Deleting Data

| Function | When to use |
|----------|-------------|
| `delete_document(namespace, id)` | Remove a specific document |
| `kv_delete(namespace, key)` | Remove a specific KV entry |
<<<<<<< HEAD
| `clear_skill_memory(skill_id, integration_id)` | Disconnect / revoke: clears skill-scoped memory in the shared `skill-{skill_id}` namespace. Storage is not isolated per integration—multiple integrations share that namespace; `integration_id` identifies the integration in the API contract (see implementation in `MemoryClient::clear_skill_memory`) |
=======
| `clear_skill_memory(skill_id, integration_id)` | Skill OAuth/auth revoked — wipes `skill-{skill_id}` namespace |
>>>>>>> 5b6b1e2 (feat(subconscious): stabilize heartbeat + subconscious loop (#392))
| `clear_namespace(namespace)` | Wipe an arbitrary namespace |

---

## Common Patterns

**Fire-and-forget writes** — Spawn a background task to avoid blocking:

```rust
let client = memory_client.clone();
tokio::spawn(async move {
    if let Err(e) = client.put_doc(input).await {
        log::warn!("memory write failed: {e}");
    }
});
```

**Trim-after-write** — Cap growth after each insert (e.g., max 50 KV entries, max 200 docs). Use `kv_list_namespace()` or `list_documents()` then delete oldest.

**Init on app startup** — Frontend calls `syncMemoryClientToken()` in `CoreStateProvider.tsx` on mount and token refresh. Memory subsystem must be initialized before queries run.

---

## Namespace Convention

| Namespace | Owner | Description |
|-----------|-------|-------------|
| `skill-{skill_id}` | Skills event loop | Raw state blobs + per-page docs (e.g., `skill-notion`, `skill-gmail`) |
| `global` | Skills working_memory.rs | Extracted user facts (preferences, goals, entities) |
| `conversations` | Agent/inference | Conversation context |
| `autocomplete` | Autocomplete | Accepted completion records (KV) |
| `autocomplete-memory` | Autocomplete | Searchable completion documents |
| `vision` | Screen intelligence | Vision summaries from screen captures |

---

## MemoryClient API Reference

**File**: `src/openhuman/memory/store/client.rs`

### Documents

| Function | Line | Type | Description |
|----------|------|------|-------------|
| `put_doc(NamespaceDocumentInput) -> Result<String>` | 105 | WRITE | Stores document; background graph extraction |
| `put_doc_light(NamespaceDocumentInput) -> Result<String>` | 125 | WRITE | Lightweight insert; no embedding or extraction |
| `ingest_doc(MemoryIngestionRequest) -> Result<MemoryIngestionResult>` | 132 | WRITE | Full synchronous ingestion pipeline |
| `store_skill_sync(...)` | 143 | WRITE | Skill-specific upsert into `skill-{skill_id}` namespace |
| `list_documents(Option<&str>) -> Result<Value>` | 184 | READ | Lists documents, optionally by namespace |
| `delete_document(&str, &str) -> Result<Value>` | 197 | DELETE | Deletes by namespace + document ID |

### Namespaces

| Function | Line | Type | Description |
|----------|------|------|-------------|
| `list_namespaces() -> Result<Vec<String>>` | 192 | READ | Lists all namespaces |
| `clear_namespace(&str) -> Result<()>` | 206 | DELETE | Clears all data in a namespace |
| `clear_skill_memory(&str, &str) -> Result<()>` | 211 | DELETE | Clears documents in the shared `skill-{skill_id}` namespace; second arg is the integration identifier passed from disconnect flows |

### Query / Recall

| Function | Line | Type | Description |
|----------|------|------|-------------|
| `query_namespace(&str, &str, u32) -> Result<String>` | 234 | READ | Semantic query; returns text |
| `query_namespace_context_data(&str, &str, u32) -> Result<NamespaceRetrievalContext>` | 246 | READ | Semantic query; returns structured data |
| `recall_namespace(&str, u32) -> Result<Option<String>>` | 258 | READ | Recent context; returns text |
| `recall_namespace_context_data(&str, u32) -> Result<NamespaceRetrievalContext>` | 269 | READ | Recent context; returns structured data |
| `recall_namespace_memories(&str, u32) -> Result<Vec<NamespaceMemoryHit>>` | 280 | READ | Recent context; returns individual hits |

### Key-Value

| Function | Line | Type | Description |
|----------|------|------|-------------|
| `kv_set(Option<&str>, &str, &Value) -> Result<()>` | 289 | WRITE | Sets KV pair (`None` namespace = global) |
| `kv_get(Option<&str>, &str) -> Result<Option<Value>>` | 302 | READ | Gets KV value |
| `kv_delete(Option<&str>, &str) -> Result<bool>` | 314 | DELETE | Deletes KV pair |
| `kv_list_namespace(&str) -> Result<Vec<Value>>` | 322 | READ | Lists all KV pairs in namespace |

### Knowledge Graph

| Function | Line | Type | Description |
|----------|------|------|-------------|
| `graph_upsert(Option<&str>, &str, &str, &str, &Value) -> Result<()>` | 330 | WRITE | Upserts relation triple (`None` namespace = global) |
| `graph_query(Option<&str>, Option<&str>, Option<&str>) -> Result<Vec<Value>>` | 356 | READ | Queries graph with optional filters |

---

## RPC Method Names

For callers going through JSON-RPC (frontend or external). Each maps 1:1 to a `MemoryClient` method above.

| RPC Method | Type | MemoryClient equivalent |
|------------|------|------------------------|
| `openhuman.memory_init` | INIT | — (initializes subsystem) |
| `openhuman.memory_doc_put` | WRITE | `put_doc()` |
| `openhuman.memory_doc_ingest` | WRITE | `ingest_doc()` |
| `openhuman.memory_doc_list` | READ | `list_documents()` |
| `openhuman.memory_doc_delete` | DELETE | `delete_document()` |
| `openhuman.memory_namespace_list` | READ | `list_namespaces()` |
| `openhuman.memory_list_namespaces` | READ | `list_namespaces()` (structured envelope) |
| `openhuman.memory_list_documents` | READ | `list_documents()` (structured envelope) |
| `openhuman.memory_delete_document` | DELETE | `delete_document()` (structured envelope) |
| `openhuman.memory_clear_namespace` | DELETE | `clear_namespace()` |
| `openhuman.memory_context_query` | READ | `query_namespace()` |
| `openhuman.memory_context_recall` | READ | `recall_namespace()` |
| `openhuman.memory_query_namespace` | READ | `query_namespace_context_data()` |
| `openhuman.memory_recall_context` | READ | `recall_namespace_context_data()` |
| `openhuman.memory_recall_memories` | READ | `recall_namespace_memories()` |
| `openhuman.memory_kv_set` | WRITE | `kv_set()` |
| `openhuman.memory_kv_get` | READ | `kv_get()` |
| `openhuman.memory_kv_delete` | DELETE | `kv_delete()` |
| `openhuman.memory_kv_list_namespace` | READ | `kv_list_namespace()` |
| `openhuman.memory_graph_upsert` | WRITE | `graph_upsert()` |
| `openhuman.memory_graph_query` | READ | `graph_query()` |
| `openhuman.ai_list_memory_files` | READ | — (file I/O, not MemoryClient) |
| `openhuman.ai_read_memory_file` | READ | — (file I/O) |
| `openhuman.ai_write_memory_file` | WRITE | — (file I/O) |

---

## Frontend TypeScript Wrappers

**File**: `app/src/utils/tauriCommands/memory.ts`

Each calls the corresponding RPC method above via `core_rpc_relay`.

| Function | Line | RPC Method |
|----------|------|------------|
| `syncMemoryClientToken(token)` | 82 | `openhuman.memory_init` |
| `memoryListDocuments(namespace?)` | 102 | `openhuman.memory_list_documents` |
| `memoryListNamespaces()` | 117 | `openhuman.memory_list_namespaces` |
| `memoryDeleteDocument(id, ns)` | 132 | `openhuman.memory_delete_document` |
| `memoryClearNamespace(ns)` | 145 | `openhuman.memory_clear_namespace` |
| `memoryQueryNamespace(ns, query, max?)` | 158 | `openhuman.memory_query_namespace` |
| `memoryRecallNamespace(ns, max?)` | 173 | `openhuman.memory_recall_context` |
| `memoryGraphQuery(ns?, subj?, pred?)` | 187 | `openhuman.memory_graph_query` |
| `memoryDocIngest(params)` | 210 | `openhuman.memory_doc_ingest` |
| `aiListMemoryFiles(dir?)` | 229 | `openhuman.ai_list_memory_files` |
| `aiReadMemoryFile(path)` | 246 | `openhuman.ai_read_memory_file` |
| `aiWriteMemoryFile(path, content)` | 261 | `openhuman.ai_write_memory_file` |

---

## Key Data Types

| Type | Description |
|------|-------------|
| `NamespaceDocumentInput` | Document upsert payload (namespace, content, metadata, tags, source_type, priority) |
| `MemoryIngestionRequest` | Full ingestion request with config (chunking, embedding, extraction flags) |
| `MemoryIngestionResult` | Ingestion result (document_id, chunk_count, entities, relations) |
| `NamespaceRetrievalContext` | Structured retrieval (hits, entities, relations, chunks) |
| `NamespaceMemoryHit` | Individual memory item with score breakdown |
| `GraphRelationRecord` | Knowledge graph triple (subject, predicate, object, evidence, namespace) |
| `RetrievalScoreBreakdown` | Score components: graph, vector, keyword, episodic, freshness |

All types defined in `src/openhuman/memory/store/types.rs`.
