# TinyCortex Memory Cutover Evaluation (2026-07-28)

## Decision

The memory engine cutover is complete. TinyCortex is the implementation
authority for chunks, content and vector primitives, trees, retrieval, scoring,
queue jobs, ingest, source readers, sync pipelines, diffs, goals, graph
primitives, conversations, and tool memory.

The remaining `src/openhuman/memory*` modules must not be moved wholesale. They
are product adapters or compatibility paths, not a second memory engine:

- RPC schemas and controller registration
- agent tools and `SecurityPolicy` enforcement
- source-scope and redaction policy
- credentials, scheduling, and event-bus bridges
- the host-owned namespace/document store
- wiki-git and Obsidian product surfaces
- process lifecycle and the global memory singleton

Moving those concerns into TinyCortex would reverse the established dependency
boundary by teaching the reusable engine about OpenHuman RPC, policy, secrets,
and runtime composition.

## Audit result

The audit covered `src/openhuman/memory/`, every `memory_*` domain, the
`src/openhuman/memory/tinycortex/` seam, and `vendor/tinycortex/src/memory/`.

| Host area | Disposition |
| --- | --- |
| `memory_store::{chunks,content,vectors,kv,entities,trees,safety}` | TinyCortex-backed compatibility and host glue |
| `memory_tree::{tree,retrieval,score}` | TinyCortex-backed compatibility plus RPC, CLI, health, and bus glue |
| `memory_queue` | TinyCortex queue, driven by the host worker lifecycle |
| `memory_sync` | TinyCortex sync engine plus host credentials, schedules, projections, RPC, and events |
| `memory_diff`, `memory_goals`, `memory_conversations` | TinyCortex engine types/operations plus host tools, RPC, or bus glue |
| `memory_sources` | Host source registry and RPC over TinyCortex readers |
| `memory_tools` | TinyCortex tool-memory store/types plus host prompt hooks and agent tools |
| `memory_store::namespace_store` | Host-owned; intentionally outside TinyCortex |
| `memory`, `tinycortex` | Product orchestration and the engine adapter seam; retained |

No second store, retrieval engine, queue engine, or sync-provider engine remains
on the live host path. The large host line counts are dominated by retained
product surfaces and their tests; line count alone is not evidence of engine
duplication.

## Cleanup executed

The unused `memory_tools::types` facade file was removed. `memory_tools` now
re-exports the crate-owned types and store directly at its existing domain-level
API. Its private `store` module retains only the host constructor so `mod.rs`
remains export-focused; prompt integration, the capture hook, and agent tools
also stay host-owned.

Other small re-export modules remain where they preserve heavily used import
paths such as `memory_queue::types`, `memory_sources::types`,
`memory_tree::retrieval::types`, and `memory_store::trees::types`. Removing
those files would create broad source churn without changing runtime ownership.

## Guardrails

- New generic memory behavior belongs in `vendor/tinycortex`.
- OpenHuman may add adapters, policy, RPC, tools, lifecycle, and product
  projections, but must not fork TinyCortex engine logic.
- Persisted format parity tests in `openhuman::memory::tinycortex::parity` remain the
  cutover guard for existing workspaces.
- `MemoryTaint`, redaction, and source-scope behavior remain security-sensitive
  review points.
