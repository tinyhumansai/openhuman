---
description: >-
  The Memory v2 host layer in crates/openhuman-core/src/memory/ and the
  TinyMemory crates it sits on.
icon: diagram-project
---

# Memory (`crates/openhuman-core/src/memory/`)

Memory v2 is three operations (Recall, Fetch, Store) over a pluggable engine, plus an engine-neutral `context.md`. The contract lives in TinyMemory (`vendor/tinymemory/docs/specs/memory-v2.md`); the OpenHuman side is `docs/specs/memory-v2.md`. The user-facing feature is described in [Memory](../../features/memory.md).

## TinyMemory crates (`vendor/tinymemory/crates/`)

| Crate | Owns |
| --- | --- |
| `tinymemory-api` | `MemoryEngine`, request/response types, `MemoryMeta`, `MetaFilter`, `EngineDescriptor`, `Error`. No I/O. |
| `tinymemory-cortex` | The CortexDB engine, registered as `cortexdb` (direct `/v1/*`) and `tinyhumans` (behind the backend `/memory/*`, host bearer). |
| `tinymemory-documents` | Format sniffing and conversion to markdown. |
| `tinymemory-sources` | Readers for folder, file, link, github, rss and composio payloads, with the SSRF guard. |
| `tinymemory-safety` | Secret and PII scrubbing before every store. |
| `tinymemory-context` | `ContextCompiler` for `context.md`. |
| `tinymemory-import` | Reads a legacy v1 workspace and yields items. |
| `tinymemory-conformance` | Behaviour suite every engine passes, plus a reference engine. |
| `tinymemory` | Facade: engine registry, `MemoryConfig`, `build_engine`. |

## Host modules

| Module | Role |
| --- | --- |
| `engine` | Binds `[memory] engine`: `tinyhumans` over the host's backend credential (resolved per request), or `cortexdb` with the key stored as `memory-cortexdb`. Engines are cached per config fingerprint. Off when neither is usable. |
| `ops` | Select engine, recall, fetch, learn, forget, list. `store_item` scrubs before storing. |
| `tools` | The single `memory` agent tool (`recall`, `fetch`, `learn`, `forget`); not registered when memory is off. |
| `conversations` | Per-thread buffering of committed turns; stores a `Conversation` item at `batch_turns` or `idle_secs`. Tool calls keep name and id only. |
| `sources` | The `[[memory.sources]]` registry, on-demand and scheduled sync. |
| `context` | Compiles, reads and injects `<workspace>/memory/context.md` (cron job `memory_context_refresh`, default every 360 minutes). |
| `import` | Consent-gated, resumable import of a v1 store; state in `<workspace>/memory/import_state.json`. |
| `exit` | Flushes buffered conversation turns on quit within a 2 second budget. |
| `bus` | Turn-commit and cron subscribers, and the idle flusher. |
| `schemas` | The `openhuman.memory_*` controllers (engines, recall, fetch, learn, forget, items, conversations, sources, context, import). |
| `status`, `error`, `types` | Engine status, `MEMORY_OFF` / `UNSUPPORTED` / `INVALID_REQUEST` / `UNAUTHORIZED` / `ENGINE` errors, shared types. |

The session host prepends `context.md`, wrapped in `<memory-context>`, as the first user message of a new session only; resumed sessions keep their frozen transcript.

Chat thread persistence is not memory. The JSONL thread store (formerly `memory::conversations` over `tinymemory-conversations`) is now `tinyagents_session::threads` in `vendor/tinyagents`, wrapped by `crates/openhuman-core/src/threads/store`.

The MCP server exposes `memory.recall`, `memory.fetch`, `memory.list`, `memory.learn` and `memory.forget` (`mcp/server/tools/specs.rs`).

## Tests

`tests/memory_v2_e2e.rs` (JSON-RPC), `app/test/playwright/specs/memory-v2.spec.ts` (UI), and unit tests beside each module (`*_tests.rs`).

Against a real CortexDB server, `scripts/test-memory-cortexdb-live.sh` boots the
pinned harness in Docker (`vendor/tinymemory/integration/cortexdb/`, v0.10.4 by
default, `CORTEXDB_VERSION=v0.9.9` for the older release) and runs
`tests/memory_cortexdb_live.rs`. That test spawns the real `openhuman-core`
binary with the `cortexdb` engine and checks learnings, a synced folder source,
web-chat conversation ingestion, recall, and `context.md` (compiled by its cron
job, written to disk, and injected into a new thread's first message). It skips
unless `OPENHUMAN_LIVE_CORTEXDB_URL` is set, so plain test runs never need
Docker.
