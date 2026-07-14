# RLM — Language-Based Workflows (Rhai/`.ragsh`) Integration Plan

**Goal:** expose TinyAgents' Rhai-backed REPL language (the `.ragsh` / RLM /
CodeAct surface, gated behind the `repl` cargo feature in
`vendor/tinyagents/Cargo.toml`) as a **first-class `rlm` tool** in the
OpenHuman Rust core, so the orchestrator agent can *write its own workflow
scripts* — fan-out over subagents, batched tool/model calls, loops,
conditionals — and execute them deterministically, similar to Claude Code
Workflows and Recursive Language Models (RLMs).

## Why

Today the orchestrator composes work through fixed primitives:
`spawn_subagent`, `spawn_parallel_agents`, `run_workflow` (WORKFLOW.md
bundles), and typed tinyflows graphs. None of these let the model express
*ad-hoc control flow* — "spawn N readers, dedupe their findings, verify each
survivor with 3 refuters, loop until dry". The `.ragsh` session in TinyAgents
is exactly that surface: a sandboxed, policy-bounded scripting engine whose
only host access is capability functions (`model_query`, `tool_call`,
`agent_query`, batched variants), with fail-closed limits on operations,
wall-clock time, output bytes, call counts, and recursion depth.

## Architecture summary

```
Orchestrator model turn
  └─ rlm tool call { script, session_id?, timeout_secs?, limits? }
       └─ src/openhuman/rlm/  (new domain)
            ├─ session manager  (persistent ReplSession per rlm session_id)
            ├─ capability bridge (openhuman Tools/Subagents/Provider →
            │                     tinyagents CapabilityRegistry)
            ├─ policy mapping   (autonomy tier + SecurityPolicy → ReplPolicy)
            ├─ progress bridge  (ReplCallRecord / EventSink → AgentProgress
            │                     + DomainEvent bus)
            └─ tinyagents::ReplSession::eval_cell  (spawn_blocking)
                 └─ rhai engine — model_query / tool_call / agent_query /
                    *_batched / emit / answer  (fail-closed ReplPolicy)
```

Two repos change:

1. **`vendor/tinyagents`** (submodule, separate PR against
   `tinyhumansai/tinyagents`): host-embedding gaps — external cancellation
   flag, live capability-call events on the `EventSink`, async-embedding
   documentation. Branch: `feat/repl-host-embedding`.
2. **`openhuman`** (one gigantic PR against `tinyhumansai/openhuman`): the
   `repl` feature flag, the new `src/openhuman/rlm/` domain, the `rlm` tool,
   prompt/docs surfacing, and tests. Branch: `feat/rlm-language-workflows`,
   including the submodule pointer bump once the tinyagents PR lands.

## Phases

| Phase | File | Deliverable |
| ----- | ---- | ----------- |
| 1 | [phase-1-research.md](phase-1-research.md) | Research findings: what tinyagents `repl` provides, what openhuman provides, the gaps |
| 2 | [phase-2-tinyagents.md](phase-2-tinyagents.md) | TinyAgents-side changes (cancellation, live events) — separate PR |
| 3 | [phase-3-rlm-domain.md](phase-3-rlm-domain.md) | `src/openhuman/rlm/` domain: sessions, capability bridge, policy |
| 4 | [phase-4-rlm-tool.md](phase-4-rlm-tool.md) | First-class `rlm` tool: schema, registration, prompt surfacing |
| 5 | [phase-5-hardening.md](phase-5-hardening.md) | Error handling, timeouts, cancellation, limits, observability |
| 6 | [phase-6-tests.md](phase-6-tests.md) | Tests (written last): unit, timeout/cancel/limit, RPC E2E |
| 7 | [phase-7-delivery.md](phase-7-delivery.md) | PR strategy: tinyagents PR + one gigantic openhuman PR |

Tests are deliberately the **final implementation phase** (per the feature
brief): phases 3–5 land the behavior with verbose debug logging; phase 6
back-fills unit and E2E coverage to the ≥80% changed-line gate before the PR
is opened.
