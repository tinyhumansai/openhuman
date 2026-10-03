# Memory v2 in OpenHuman

Status: accepted. It replaces the whole v1 memory surface (the `memory_tree`,
`memory_goals`, `people`, `tree_summarizer`, `slack_memory` and `memory_sync`
namespaces, and the old `memory.*` and `memory_sources.*` methods). The engine
contract lives in TinyMemory: `vendor/tinymemory/docs/specs/memory-v2.md`.

## Model

| Concept | Meaning |
| --- | --- |
| Engine | Who stores and answers. At launch: `tinyhumans` (hosted CortexDB, needs sign-in) and `cortexdb` (your own CortexDB, endpoint + key). |
| Recall | Ask a question, get an answer with citations (engine-implemented). |
| Fetch | Raw keyword/vector/hybrid search with metadata filters. |
| Store | Documents (synced sources), Conversations (auto, after turns), Learnings (explicit). |
| context.md | Periodically compiled brief, injected as the first user message of a new session. |

With no usable engine (signed out and no CortexDB key), memory is **off**:
- the tools are not registered;
- ingestion is a no-op;
- RPCs answer `MEMORY_OFF`;
- the UI explains why.

## Config (`config.toml`)

```toml
[memory]
engine = "tinyhumans"            # "tinyhumans" | "cortexdb"

[memory.engines.cortexdb]
endpoint = "https://api-v1.cortexdb.ai"   # key in keychain as "memory-cortexdb"

[memory.conversations]
enabled = true
batch_turns = 4      # store after this many committed turns in a thread…
idle_secs = 120      # …or after the thread is idle this long

[memory.context]
enabled = true
interval_mins = 360
budget_tokens = 2000
```

Old `[subsystems.memory]` and v1 `[memory]` keys are ignored.

Out of scope: `memory::conversations` (the chat thread/message JSONL store over
`tinymemory-conversations`) is thread persistence, not memory. The store moves
to TinyAgents as `tinyagents_session::threads`, keeping the same on-disk format.
The host's thread code moves from `memory::conversations` to
`threads::store`, which wraps it, so that `memory/` holds only v2.

## Agent tool: `memory`

There is one tool. Its `action` is `recall` | `fetch` | `learn` | `forget`:
- `recall { question, filter? }` returns `{answer, citations[]}`.
- `fetch { query, mode?, filter?, limit? }` returns `{hits[]}`. `mode` is limited to the engine's `fetch_modes`.
- `learn { text, kind?, confidence? }` returns `{id}`. The host fills `meta` with:
  - `workspace` (the agent's `action_dir`);
  - `thread_id` and `agent_id`;
  - `tool_call` (this call's name and id);
  - `source.kind = "agent"`.
- `forget { ids }` returns `{forgotten}`.

## Automatic ingestion

- **Conversations:** a bus subscriber on turn commit buffers per thread. It stores one `Conversation` item when `batch_turns` is reached or when the thread has been idle for `idle_secs`. Meta carries `thread_id`, `agent_id`, `workspace`, `turns` and `tool_calls` (name and id only; arguments are never stored).
- **Documents:** a registry of sources (`folder`, `file`, `link`, `github`, `rss`, `composio`). Sync runs on demand and on a scheduler job. Each item gets `folder`, `file_path`, `language`, `repo`, `commit` and `url` where they apply.

## context.md

- The cron job `memory_context_refresh` runs every `interval_mins`, plus on demand. It writes `<workspace>/memory/context.md`.
- On a **new** session the session host prepends it, wrapped in `<memory-context>…</memory-context>`, as the first user message, next to the workflows context.
- Resumed sessions keep their frozen transcript.

## RPC (`openhuman.memory_*`)

All methods take and return JSON objects. Errors use the standard structured error; the `code` is one of `MEMORY_OFF`, `UNSUPPORTED`, `INVALID_REQUEST`, `UNAUTHORIZED` or `ENGINE`.

| Method | Params | Result |
| --- | --- | --- |
| `memory_engines_list` | `{}` | `{engines: EngineDescriptor[], active: string\|null}` |
| `memory_engine_get` | `{}` | `{engine: string\|null, endpoint?: string, has_key: boolean, status: "ok"\|"degraded"\|"down"\|"off", reason?: string, fetch_modes: string[]}` |
| `memory_engine_set` | `{engine, endpoint?, api_key?}` | same as `engine_get` |
| `memory_recall` | `{question, filter?, limit?}` | `{answer, citations: Citation[], model?}` |
| `memory_fetch` | `{query, mode?, filter?, limit?, cursor?}` | `{hits: Hit[], next_cursor?}` |
| `memory_learn` | `{text, kind?, confidence?, meta?}` | `{id}` |
| `memory_forget` | `{ids}` | `{forgotten: number}` |
| `memory_items_list` | `{filter?, limit?, cursor?}` | `{items: Hit[], next_cursor?}` |
| `memory_conversations_get` | `{}` | `{enabled, batch_turns, idle_secs, recent: {thread_id, turns, stored_at}[]}` |
| `memory_conversations_set` | `{enabled?, batch_turns?, idle_secs?}` | same as `conversations_get` |
| `memory_sources_list` | `{}` | `{sources: Source[]}` |
| `memory_sources_add` | `{kind, target, label?, schedule_mins?}` | `{source: Source}` |
| `memory_sources_remove` | `{id, forget_items?}` | `{removed: boolean}` |
| `memory_sources_sync` | `{id?}` (all if omitted) | `{started: string[]}` |
| `memory_context_get` | `{}` | `{markdown, tokens, generated_at\|null, interval_mins, budget_tokens, enabled}` |
| `memory_context_refresh` | `{}` | same as `context_get` |
| `memory_context_set` | `{enabled?, interval_mins?, budget_tokens?}` | same as `context_get` |
| `memory_import_scan` | `{}` | `{found: boolean, counts?: {documents, conversations, learnings}}` |
| `memory_import_start` | `{consent: true}` | `{state: ImportState}` |
| `memory_import_status` | `{}` | `{state: ImportState}` |

The types:
- `EngineDescriptor`:
  - Identity: `id`, `label`, `description`.
  - Requirements: `hosted`, `needs_endpoint`, `needs_key`, `default_endpoint`.
  - `fetch_modes` (a subset of `"keyword"`, `"vector"` and `"hybrid"`).
- `Hit`: `{id, kind: "document"|"conversation"|"learning", text, meta: MemoryMeta, score}`.
- `Citation`: `Hit` with `snippet` in place of `text`.
- `MemoryMeta` uses the TinyMemory field names in snake_case. `source` is `{kind, id?}`.
- `MetaFilter` uses the same fields, plus `kinds`, `sources`, `tags_any`, `observed_after` and `observed_before`.
- `Source`:
  - Identity: `id`, `kind`, `target` (path, URL, `owner/repo`, feed URL or Composio toolkit), `label`.
  - Sync state: `schedule_mins`, `last_sync_at`, `status` (`"idle"`, `"syncing"` or `"error"`), `error?`, `items`.
- `ImportState`: `{phase: "idle"|"running"|"done"|"error", imported, total, error?}`.

`memory_import_start` without `consent: true` is refused. It uploads local data to the selected engine.

## UI

The Memory page lives under Connections at `/connections?tab=brain&brain=<chip>`. Its chips are `engine`, `ask`, `learnings`, `conversations`, `documents` and `context`. The default chip is `ask` when an engine is active and `engine` otherwise.

Legacy `?brain=graph|goals|sync|sources` values map to `ask`, `ask`, `documents` and `documents`. `/settings/memory-engine` redirects to the `engine` chip.
