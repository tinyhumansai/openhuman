# memory

Memory v2: Recall, Fetch and Store over a pluggable engine, plus the compiled
`context.md`. The spec is [`docs/specs/memory-v2.md`](../../../../docs/specs/memory-v2.md);
the engine contract and engines live in `vendor/tinymemory`
(`tinymemory-api`, `-cortex`, `-documents`, `-sources`, `-safety`, `-context`,
`-import`). This directory is the host side: it binds an engine, adapts it to
OpenHuman's controllers, tool and event bus, and applies OpenHuman policy
(credentials, consent, scheduling).

Chat thread persistence is not memory: see [`threads/store`](../threads/store/README.md).

## Responsibilities

- Bind the configured engine (`tinyhumans` over the host backend credential,
  or `cortexdb` with the key stored as `memory-cortexdb`); memory is **off**
  when neither is usable, and then the tool is not registered, ingestion is a
  no-op and RPCs answer `MEMORY_OFF`.
- Serve recall, fetch, learn, forget and list; scrub every item before store.
- Buffer committed conversation turns per thread and store one `Conversation`
  item at `batch_turns` or after `idle_secs`; flush on exit.
- Keep the `[[memory.sources]]` registry and sync it on demand and on a cron job.
- Compile, read and inject `context.md`.
- Import a previous (v1) store into the selected engine after explicit consent.

## Key files

| File | Role |
| --- | --- |
| `engine.rs` | `resolve` binds the `[memory]` engine or says why memory is off; engines are cached per config fingerprint. |
| `ops.rs` | Select engine, recall, fetch, learn, forget, list. `store_item` scrubs first. |
| `tools.rs` | The single `memory` agent tool (`recall`, `fetch`, `learn`, `forget`). |
| `conversations/` | Per-thread turn buffer and idle flusher. Tool calls keep name and id only, never arguments. |
| `sources/` | Source registry, state and sync (folder, file, link, github, rss, composio). |
| `context.rs` | Compile `<workspace>/memory/context.md` and build the `<memory-context>` injection for new sessions. |
| `import.rs` | Consent-gated, resumable v1 import; state in `<workspace>/memory/import_state.json`. |
| `exit.rs` | Stores buffered turns on quit within `EXIT_BUDGET` (2 s). |
| `bus.rs` | Turn-commit subscriber and the `memory_context_refresh` / `memory_sources_sync` cron jobs. |
| `status.rs` | The memory row of the subsystem status table. |
| `schemas.rs`, `schemas/` | The `openhuman.memory_*` controllers. |
| `error.rs`, `types.rs` | `MemoryError` (`MEMORY_OFF`, `UNSUPPORTED`, `INVALID_REQUEST`, `UNAUTHORIZED`, `ENGINE`) and shared types. |
| `*_tests.rs`, `test_fixtures.rs` | Sibling unit tests. |

## RPC

`openhuman.memory_engines_list`, `_engine_get`, `_engine_set`, `_recall`,
`_fetch`, `_learn`, `_forget`, `_items_list`, `_conversations_get/set`,
`_sources_list/add/remove/sync`, `_context_get/refresh/set`,
`_import_scan/start/status`. Params and results are in the spec. The MCP server
exposes `memory.recall`, `memory.fetch`, `memory.list`, `memory.learn` and
`memory.forget` over the same handlers (`mcp/server/tools/specs.rs`).

## Persistence

No local database. Items live in the engine. Local files: `<workspace>/memory/context.md`,
`context_state.json`, `import_state.json`; the sources registry is `[[memory.sources]]`
in `config.toml`; the CortexDB key is in the OS keychain.

## Tests

`tests/memory_v2_e2e.rs` (JSON-RPC against the mock backend),
`app/test/playwright/specs/memory-v2.spec.ts` (UI), and the sibling `*_tests.rs`.

Against a real CortexDB server, `scripts/test-memory-cortexdb-live.sh` boots the
pinned harness in Docker (`vendor/tinymemory/integration/cortexdb/`, v0.10.4 by
default, `CORTEXDB_VERSION=v0.9.9` for the older release) and runs
`tests/memory_cortexdb_live.rs`. That test spawns the real `openhuman-core`
binary with the `cortexdb` engine and checks learnings, a synced folder source,
web-chat conversation ingestion, recall, and `context.md` (compiled by its cron
job, written to disk, and injected into a new thread's first message). It skips
unless `OPENHUMAN_LIVE_CORTEXDB_URL` is set, so plain test runs never need
Docker.
