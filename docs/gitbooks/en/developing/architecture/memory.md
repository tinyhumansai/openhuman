---
description: >-
  The memory host layer in crates/openhuman-core/src/memory/: TinyMemory's
  agent lifecycle bound to OpenHuman's turns, RPC and UI.
icon: diagram-project
---

# Memory

`crates/openhuman-core/src/memory/` is the host layer over TinyMemory. It runs the agent memory lifecycle around every turn. The prelude takes completed cached context immediately and queues user logging and refresh; committed replies are logged in the background. Compaction uses completed context and queues its own refresh without delaying the authoritative summarizer. Belief builds also run in the background.

The behavior is specified in [`docs/specs/memory-v2.md`](https://github.com/tinyhumansai/openhuman/blob/main/docs/specs/memory-v2.md). The user-facing feature is [Memory](../../features/memory.md).

## TinyMemory crates (`vendor/tinymemory/crates/`)

| Crate | Owns |
| --- | --- |
| `tinymemory-api` | The engine contract: `MemoryEngine` (recall, fetch, store, forget, list, consolidate, beliefs), `StoreItem`, `MemoryMeta`, `MetaFilter`, `Namespace`/`Reach`, `EngineDescriptor`, `Error`. No I/O. Its `conformance` feature adds the suite every engine passes and the in-memory `ReferenceEngine` the unit tests use. |
| `tinymemory-tools` | The agent surface over any engine: `AgentMemory` (`start_session`, `pre_turn`, `post_turn`, `recall_for_compaction`, `recall`), `MemoryLayout`, `Brain`, holistic recall (`ContextPack`), `BackgroundJob`/`BackgroundRunner`. |
| `tinymemory-integrations` | Everything that touches the outside world: the CortexDB engine on both wires (`cortexdb` direct, `tinyhumans` behind the backend), the registry, document conversion and the brain filer, source readers (SSRF-guarded), secret/PII scrubbing, the v1 importer. |

## Host modules

| Module | Role |
| --- | --- |
| `engine` | Binds `[memory] engine`: `tinyhumans` over the host's backend credential (resolved per request) with the transport's attribution headers, or `cortexdb` with the key stored as `memory-cortexdb`. Off when neither is usable. |
| `guard` | `ScrubbingEngine`: every write is scrubbed under the host policy, whichever path makes it. |
| `scope` | Projects OpenHuman config into TinyMemory `IdentityPins`; callers use upstream `Identity` / `ResolvedIdentity` for each turn. |
| `lifecycle::prefetch` | Scoped, bounded in-memory cache and background workers for automatic turn and compaction recall. |
| `lifecycle::hooks` | Lifecycle primitives; explicit/manual `pre_turn` and `compaction` retain configured deadlines. |
| `lifecycle::jobs` | The persisted background queue and the `memory_background` cron job. |
| `lifecycle::views` | Policy, pack preview, agents list, job views for the RPCs. |
| `brain` | Brain source mapping for synced items and the `memory_brain_*` ops. |
| `sources` | The `[[memory.sources]]` registry and sync into the brain. |
| `channels` | Which channel each logged thread arrived on, for forgetting a channel. |
| `backfill` | Consent-gated storing of past chats in the lifecycle's shape. |
| `import` | Consent-gated, resumable v1 import. |
| `tools`, `tool_budget`, `tool_writes` | The scoped agent tool: bounded explicit reads and durable local admission of `learn`/`forget`, followed by background delivery. |
| `ops`, `explore` | Engine selection, recall, fetch, learn, forget, listing, the explorer. |
| `bus` | The cron subscriber (`memory_sources_sync`, `memory_background`). |
| `schemas` | The `openhuman.memory_*` controllers. |
| `status`, `error`, `types` | Subsystem status, error codes, shared types. |

## Where the agent loop calls memory

| Hook | File |
| --- | --- |
| Pre-turn (take cached pack, queue logging and refresh) | `memory/lifecycle/session_hooks.rs`, from `before_turn` in `runtime_session.rs` |
| Pack injection, ephemeral, every model request of the turn | `agent/tinyagents/middleware/memory_pack.rs` |
| Post-turn (log the reply, queue builds), after the durable commit | `session_hooks.rs`, from `finalize_after_durable_commit` |
| Compaction enrichment from completed cache, with background refresh | `agent/tinyagents/memory_summarizer.rs`, wrapped around the summarizer in `context_wiring.rs` (ladder order and sizing live in `tinyagents-harness` `ContextLadder`) |

Automatic packs are keyed by CoreContext, workspace, config fingerprint, resolved memory identity and thread. They expire after 60 seconds. The cache holds at most 64 entries per context and 64 KiB per pack; eight workers globally process bounded queues of at most 16 pending operations per entry, with a 120-second deadline per operation. Saturation skips best-effort automatic work rather than waiting. Packs identify themselves as results of an earlier lookup, not authoritative answers to the current question.

Successful content changes clear cached packs and advance a revision, preserving queued conversation logs while rejecting older in-flight results. Engine or credential invalidation clears the caches and pending refreshes. Scoped background tasks retain the CoreContext; the memory identity is an explicit value on the run context, not task-local state.

The agent tool allows 15 seconds per call, 30 seconds of aggregate reserved time and eight attempts per retained run record. The tracker keeps 128 records; idle eviction under churn can reset the budget for that run, while outstanding reservations cannot be evicted. `learn`/`forget` acknowledge durable local admission, not remote completion. Their scrubbed outbox entries contain trusted scope rather than credentials, and retry under the matching owner on background/auth ticks. Direct RPC, UI and import/deletion workflows keep their explicit behavior.

The turn's `MemoryTurn` (config, identity, thread, pack) rides
`OpenHumanRunContext::memory_turn`; a child run has its own.

Chat thread persistence is not memory. It is `tinyagents_session::threads`,
wrapped by `crates/openhuman-core/src/threads/store`.

The MCP server exposes `memory.recall`, `memory.fetch`, `memory.list`,
`memory.learn` and `memory.forget` (`mcp/server/tools/specs.rs`).

## Tests

- Unit tests beside each module (`*_tests.rs`), against `ReferenceEngine`.
- `tests/memory_v2_e2e.rs`: every `memory_*` RPC against the mock backend, and
  a web-chat turn whose inference request carries the pack, whose turns are
  logged with `x-sdk-name`, and whose transcript holds no pack.
- `crates/openhuman-embed/tests/runtime_agents.rs`: `AgentSpec::memory`.
- `app/test/playwright/specs/memory-v2.spec.ts` (UI).

Against a real CortexDB server, `scripts/test-memory-cortexdb-live.sh` boots
the pinned harness in Docker (`vendor/tinymemory/integration/cortexdb/`) and
runs `tests/memory_cortexdb_live.rs`: learnings, a synced folder source,
web-chat turn logging, recall, the pack preview, the pack injected into a
turn's inference request, and belief builds. It skips unless
`OPENHUMAN_LIVE_CORTEXDB_URL` is set.
