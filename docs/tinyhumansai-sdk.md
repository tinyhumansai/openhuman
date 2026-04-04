# TinyHumans AI SDK — Reference & Project Integration

**Crate:** [`tinyhumansai`](https://crates.io/crates/tinyhumansai)
**Version:** 0.1.6
**License:** MIT
**Repository:** https://github.com/tinyhumansai/neocortex/tree/main/packages/sdk-rust

The `tinyhumansai` Rust SDK is a typed async client for the TinyHumans Neocortex memory API. It supports inserting, querying, recalling, and deleting memory — plus ingestion job tracking, document management, and skill-data sync.

---

## Client Setup

### `TinyHumanConfig`

```rust
let config = TinyHumanConfig::new("your-api-token");

// Override the base URL (optional)
let config = config.with_base_url("https://staging-api.alphahuman.xyz");
```

Base URL resolution order:

1. `with_base_url(...)` call
2. `TINYHUMANS_BASE_URL` env var
3. `NEOCORTEX_BASE_URL` env var
4. Default: `https://api.tinyhumans.ai`

### `TinyHumansMemoryClient::new`

```rust
let client = TinyHumansMemoryClient::new(config)?;
```

Validates that the token is non-empty. Returns `TinyHumansError::Validation` if it is.
Sets a 30-second HTTP timeout on all requests. Uses `rustls` for TLS.

---

## SDK Functions

### `insert_memory`

**Endpoint:** `POST /memory/insert`

Ingests a document into the memory store. Returns a job ID — ingestion is asynchronous.

```rust
let res = client.insert_memory(InsertMemoryParams {
    title: "Sprint Dataset - Team Velocity".to_string(),
    content: "...".to_string(),
    namespace: "sdk-rust-e2e".to_string(),
    document_id: "my-doc-id".to_string(),
    metadata: Some(serde_json::json!({ "source": "example.rs" })),
    ..Default::default()
}).await?;

let job_id = res.data.job_id; // Option<String>
```

**`InsertMemoryParams`**

| Field         | Type                        | Required | Description                         |
| ------------- | --------------------------- | -------- | ----------------------------------- |
| `title`       | `String`                    | Yes      | Document title                      |
| `content`     | `String`                    | Yes      | Document text content               |
| `namespace`   | `String`                    | Yes      | Logical partition for the document  |
| `document_id` | `String`                    | Yes      | Caller-supplied unique ID           |
| `source_type` | `Option<SourceType>`        | No       | `Doc` (default), `Chat`, or `Email` |
| `metadata`    | `Option<serde_json::Value>` | No       | Arbitrary JSON metadata             |
| `priority`    | `Option<Priority>`          | No       | `High`, `Medium`, or `Low`          |
| `created_at`  | `Option<f64>`               | No       | Unix timestamp (ms)                 |
| `updated_at`  | `Option<f64>`               | No       | Unix timestamp (ms)                 |

**Serialised request body** (fields with `None` values are omitted):

```json
{
  "title": "...",
  "content": "...",
  "namespace": "...",
  "sourceType": "doc",
  "metadata": { "source": "example.rs" },
  "documentId": "my-doc-id"
}
```

**`InsertMemoryResponse`**

```rust
InsertMemoryResponse {
    success: true,
    data: InsertMemoryData {
        job_id: Some("a2a1396c-..."),
        state: Some("pending"),
        status: None,
        stats: None,
        usage: None,
    }
}
```

---

### `get_ingestion_job`

**Endpoint:** `GET /memory/ingestion/jobs/{jobId}`

Fetches the current status of an ingestion job.

```rust
let res = client.get_ingestion_job("a2a1396c-bcf5-4552-afc0-6c822bafd7c6").await?;
println!("{:?}", res.data.state); // Some("processing") | Some("completed") | ...
```

**`IngestionJobStatusResponse`**

| Field          | Type                        | Description                                                 |
| -------------- | --------------------------- | ----------------------------------------------------------- |
| `job_id`       | `Option<String>`            | The job ID                                                  |
| `state`        | `Option<String>`            | `pending`, `processing`, `completed`, `failed`, etc.        |
| `endpoint`     | `Option<String>`            | API endpoint that created the job                           |
| `attempts`     | `Option<f64>`               | Number of execution attempts                                |
| `error`        | `Option<String>`            | Error message if failed                                     |
| `response`     | `Option<serde_json::Value>` | Full ingestion result (stats, timings, usage) on completion |
| `created_at`   | `Option<String>`            | ISO 8601 timestamp                                          |
| `started_at`   | `Option<String>`            | ISO 8601 timestamp                                          |
| `completed_at` | `Option<String>`            | ISO 8601 timestamp                                          |

Completed response includes ingestion stats (chunk count, entity count, relation count, timings, embedding token usage, and cost in USD).

---

### `wait_for_ingestion_job`

Polls `get_ingestion_job` until the job reaches a terminal state.

```rust
let res = client.wait_for_ingestion_job(
    "a2a1396c-...",
    Some(30_000),  // timeout_ms
    Some(1_000),   // poll_interval_ms
).await?;
```

| Parameter          | Type          | Default | Description              |
| ------------------ | ------------- | ------- | ------------------------ |
| `job_id`           | `&str`        | —       | Job to poll              |
| `timeout_ms`       | `Option<u64>` | 30 000  | Max wait in milliseconds |
| `poll_interval_ms` | `Option<u64>` | 1 000   | Polling interval         |

**Terminal states:** `completed`, `done`, `succeeded`, `success` → returns `Ok`.
**Failure states:** `failed`, `error`, `cancelled` → returns `Err`.
**Timeout:** returns `TinyHumansError::Api { status: 408 }`.

---

### `query_memory`

**Endpoint:** `POST /memory/query`

Semantic (RAG) query over stored memory. Returns ranked chunks relevant to the query.

```rust
let res = client.query_memory(QueryMemoryParams {
    query: "Which team has the highest velocity?".to_string(),
    namespace: Some("sdk-rust-e2e".to_string()),
    include_references: Some(true),
    max_chunks: Some(5.0),
    ..Default::default()
}).await?;
```

**`QueryMemoryParams`** (`#[serde(rename_all = "camelCase")]`)

| Field                | Type                  | Serialised as         | Description                   |
| -------------------- | --------------------- | --------------------- | ----------------------------- |
| `query`              | `String`              | `"query"`             | The search query              |
| `namespace`          | `Option<String>`      | `"namespace"`         | Filter by namespace           |
| `include_references` | `Option<bool>`        | `"includeReferences"` | Include source chunk metadata |
| `max_chunks`         | `Option<f64>`         | `"maxChunks"`         | Max chunks to retrieve        |
| `document_ids`       | `Option<Vec<String>>` | `"documentIds"`       | Filter to specific documents  |
| `llm_query`          | `Option<String>`      | `"llmQuery"`          | Override query sent to LLM    |

**`QueryMemoryResponse`**

```rust
QueryMemoryResponse {
    success: true,
    data: QueryMemoryData {
        context: Some(QueryContextOut {
            entities: [],
            relations: [],
            chunks: [ /* matched chunks with scores and entity_mentions */ ],
        }),
        usage: Some(Usage {
            embedding_tokens: 20,
            cost_usd: 0.0000004,
            llm_input_tokens: 0,
            llm_output_tokens: 0,
        }),
        cached: false,
        llm_context_message: Some("## Sources\n\n[1] ..."),
        response: None,  // populated if LLM response was requested
    }
}
```

`llm_context_message` is a pre-formatted string ready for injection into an LLM prompt.

---

### `recall_memory`

**Endpoint:** `POST /memory/recall`

Recalls synthesised context from the Master memory node for a namespace — no query required. Returns the most relevant accumulated context.

```rust
let res = client.recall_memory(RecallMemoryParams {
    namespace: Some("sdk-rust-e2e".to_string()),
    max_chunks: Some(5.0),
}).await?;
```

**`RecallMemoryParams`** (`#[serde(rename_all = "camelCase")]`)

| Field        | Type             | Serialised as | Description              |
| ------------ | ---------------- | ------------- | ------------------------ |
| `namespace`  | `Option<String>` | `"namespace"` | Namespace to recall from |
| `max_chunks` | `Option<f64>`    | `"maxChunks"` | Max chunks to return     |

**`RecallMemoryResponse`**

```rust
RecallMemoryResponse {
    success: true,
    data: RecallMemoryData {
        context: Some(/* raw JSON object with chunks, entities, relations */),
        llm_context_message: Some("## Sources\n\n[1] ..."),
        response: None,
        cached: false,
        latency_seconds: Some(2.8183),
        counts: Some(RecallCounts {
            num_chunks: 1,
            num_entities: 0,
            num_relations: 0,
        }),
        usage: Some(/* cost/token breakdown */),
    }
}
```

Recall is embedding-free (0 tokens, $0 cost) unlike `query_memory`.

---

### `delete_memory`

**Endpoint:** `POST /memory/admin/delete`

Deletes all memory for a namespace (or all memory if namespace is omitted).

```rust
client.delete_memory(DeleteMemoryParams {
    namespace: Some("skill:gmail:user@example.com".to_string()),
}).await?;
```

**`DeleteMemoryResponse`**

```rust
DeleteMemoryData {
    status: "ok",
    user_id: "...",
    namespace: Some("skill:gmail:user@example.com"),
    nodes_deleted: 42,
    message: "...",
}
```

---

### `list_documents`

**Endpoint:** `GET /memory/documents?namespace=...&limit=...&offset=...`

Lists ingested documents with optional namespace filtering and pagination.

```rust
let res = client.list_documents(ListDocumentsParams {
    namespace: Some("sdk-rust-e2e".to_string()),
    limit: Some(10.0),
    offset: Some(0.0),
}).await?;
```

**`ListDocumentsParams`**

| Field       | Type             | Description           |
| ----------- | ---------------- | --------------------- |
| `namespace` | `Option<String>` | Filter by namespace   |
| `limit`     | `Option<f64>`    | Max results to return |
| `offset`    | `Option<f64>`    | Pagination offset     |

Returns `serde_json::Value` with a `data.documents` array. Each document includes `document_id`, `namespace`, `title`, `chunk_count`, `created_at`, `updated_at`, `user_id`.

---

### `get_document`

**Endpoint:** `GET /memory/documents/{documentId}?namespace={namespace}`

Fetches metadata for a single document by ID.

```rust
let res = client.get_document("my-doc-id", Some("my-namespace")).await?;
```

Returns `serde_json::Value` with `document_id`, `namespace`, `title`, `chunk_count`, `chunk_ids`, timestamps, and `user_id`.

---

### `delete_document`

**Endpoint:** `DELETE /memory/documents/{documentId}?namespace={namespace}`

Deletes a specific document from a namespace.

```rust
client.delete_document("my-doc-id", "my-namespace").await?;
```

Both `document_id` and `namespace` are required (validated before the request is sent).

---

### Other SDK Methods (Available, Not Used in Project)

| Method                    | Endpoint                           | Description                   |
| ------------------------- | ---------------------------------- | ----------------------------- |
| `insert_document`         | `POST /memory/documents`           | Insert via documents route    |
| `insert_documents_batch`  | `POST /memory/documents/batch`     | Batch insert documents        |
| `recall_memories`         | `POST /memory/memories/recall`     | Recall from Ebbinghaus bank   |
| `recall_memories_context` | `POST /memory/memories/context`    | Recall context from memories  |
| `recall_thoughts`         | `POST /memory/memories/thoughts`   | Reflective thought generation |
| `interact_memory`         | `POST /memory/interact`            | Record entity interactions    |
| `record_interactions`     | `POST /memory/interactions`        | Record interaction signals    |
| `query_memory_context`    | `POST /memory/queries`             | Query alias route             |
| `chat_memory_context`     | `POST /memory/conversations`       | Chat with memory context      |
| `chat_memory`             | `POST /memory/chat`                | Chat via DeltaNet cache       |
| `sync_memory`             | `POST /memory/sync`                | Sync OpenClaw workspace files |
| `memory_health`           | `GET /memory/health`               | Health check                  |
| `get_graph_snapshot`      | `GET /memory/admin/graph-snapshot` | Admin graph data              |

---

## Error Types

**`TinyHumansError`**

| Variant                         | When                                                           |
| ------------------------------- | -------------------------------------------------------------- |
| `Validation(String)`            | Client-side validation failed (empty token, empty title, etc.) |
| `Http(String)`                  | Network/transport error from `reqwest`                         |
| `Api { message, status, body }` | Non-2xx response from the API                                  |
| `Decode(String)`                | Failed to deserialise response JSON                            |

---

## Project Integration (`openhuman`)

### How the SDK is Initialised

**File:** `src-tauri/src/memory/mod.rs`

The project wraps memory operations in a `MemoryClient` struct backed by local SQLite
(via `UnifiedMemory`). Construction happens at runtime via the `openhuman.memory_init`
RPC method.  Memory is **local-only** — the `jwt_token` parameter in the init request
is accepted for backward compatibility but ignored.  Remote/cloud memory sync is a
future consideration.

```rust
// Local-only — no remote sync.
pub fn new_local() -> Result<Self, String> { /* ... */ }
pub fn from_workspace_dir(workspace_dir: PathBuf) -> Result<Self, String> { /* ... */ }
```

The client is stored as `Arc<MemoryClient>` inside a `Mutex<Option<MemoryClientRef>>` (`MemoryState`), shared across RPC handlers.

---

### `MemoryClient` Wrapper Methods

The `MemoryClient` wrapper exposes higher-level methods used throughout the app:

#### `store_skill_sync`

Calls `insert_memory` then polls `ingestion_job_status` every **30 seconds** until the job is `completed` or `failed`. Used after skill OAuth completion and periodic skill syncs.

```rust
client.store_skill_sync(
    skill_id,          // becomes the namespace
    integration_id,    // e.g. "user@example.com"
    title,
    content,
    source_type,       // Option<SourceType>
    metadata,          // Option<serde_json::Value>
    priority,          // Option<Priority>
    created_at,        // Option<f64>
    updated_at,        // Option<f64>
    document_id,       // Option<String> — auto-generated UUID if None
).await?;
```

> Note: polling interval is 30 s (fire-and-forget background task). The E2E test uses `wait_for_ingestion_job` with 1 s polling instead.

#### `query_skill_context`

Calls `query_memory` for a skill's namespace. Returns the `response` string from the API (LLM-synthesised answer).

```rust
let context: String = client.query_skill_context(
    skill_id,        // namespace
    integration_id,  // unused currently
    "What emails were recently synced?",
    10,              // max_chunks
).await?;
```

#### `recall_skill_context`

Calls `recall_memory` for a namespace. Returns `Option<serde_json::Value>` (the raw `context` field).

```rust
let ctx: Option<serde_json::Value> = client.recall_skill_context(
    skill_id,
    integration_id,
    10,  // max_chunks
).await?;
```

#### `clear_skill_memory`

Calls `delete_memory` with `namespace = skill_id`. Used on OAuth revoke / skill disconnect.

```rust
client.clear_skill_memory("gmail", "user@example.com").await?;
// → DELETE namespace "gmail"
```

#### `query_namespace_context` / `recall_namespace_context`

Direct namespace versions of query/recall — bypass the `skill:{id}:{id}` namespace convention.

#### `list_documents` / `delete_document`

Thin pass-through wrappers over the SDK methods.

---

### Memory in the Chat Agentic Loop

**File:** `src-tauri/src/commands/chat.rs`

Every `chat_send` call (desktop) performs these memory operations before hitting the inference API:

**Step 2 — Conversation recall**

```rust
mem.recall_skill_context("conversations", thread_id, 10).await
```

Recalls context from the `conversations` namespace, keyed by `thread_id`. The result is injected into the user message as:

```
[MEMORY_CONTEXT]
{recalled context}
[/MEMORY_CONTEXT]

{user message}
```

**Step 2b — Skill context recall**

For every skill with registered tools, recalls its memory:

```rust
mem.recall_skill_context(skill_id, skill_id, 10).await
```

Each result is injected as:

```
[{SKILL_ID}_CONTEXT]
{recalled context}
[/{SKILL_ID}_CONTEXT]
```

The full assembled user message sent to the inference API looks like:

```
## Project Context
{openclaw_context — SOUL.md, IDENTITY.md, TOOLS.md, etc.}

User message: {original message}

[MEMORY_CONTEXT]
{conversation memory}
[/MEMORY_CONTEXT]

[GMAIL_CONTEXT]
{gmail skill memory}
[/GMAIL_CONTEXT]

{notion_context if present}
```

---

### Namespace Conventions

| Context                        | Namespace pattern | Set by                                                     |
| ------------------------------ | ----------------- | ---------------------------------------------------------- |
| Skill sync (OAuth / periodic)  | `{skill_id}`      | `store_skill_sync` — uses `skill_id` directly as namespace |
| Skill memory clear             | `{skill_id}`      | `clear_skill_memory`                                       |
| Conversation recall            | `conversations`   | `chat_send_inner` hardcoded                                |
| Skill context recall (in chat) | `{skill_id}`      | `chat_send_inner` per-skill loop                           |
| E2E test                       | `sdk-rust-e2e`    | `example_e2e.rs`                                           |

---

### All SDK Calls in the Project

| Location                                  | SDK method called                  | Purpose                                 |
| ----------------------------------------- | ---------------------------------- | --------------------------------------- |
| `memory/mod.rs::store_skill_sync`         | `insert_memory`                    | Write skill sync data                   |
| `memory/mod.rs::store_skill_sync`         | `ingestion_job_status` (poll loop) | Wait for ingestion to complete          |
| `memory/mod.rs::query_skill_context`      | `query_memory`                     | RAG query for skill context             |
| `memory/mod.rs::recall_skill_context`     | `recall_memory`                    | Recall synthesised context              |
| `memory/mod.rs::recall_namespace_context` | `recall_memory`                    | Direct namespace recall                 |
| `memory/mod.rs::query_namespace_context`  | `query_memory`                     | Direct namespace query                  |
| `memory/mod.rs::list_documents`           | `list_documents`                   | List ingested documents                 |
| `memory/mod.rs::delete_document`          | `delete_document`                  | Remove a document                       |
| `memory/mod.rs::clear_skill_memory`       | `delete_memory`                    | Wipe skill namespace on disconnect      |
| `commands/chat.rs` (step 2)               | via `recall_skill_context`         | Inject conversation history into prompt |
| `commands/chat.rs` (step 2b)              | via `recall_skill_context`         | Inject per-skill context into prompt    |
