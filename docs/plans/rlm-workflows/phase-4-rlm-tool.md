# Phase 4 — The first-class `rlm` tool

## 4.1 Tool definition (`src/openhuman/rlm/tools.rs`)

`RlmTool` implementing `crate::openhuman::tools::Tool`:

- `name()` → `"rlm"`.
- `description()` — teaches the model the surface in one screen: what a
  cell is, the built-ins (`tool_call`, `agent_query`, `model_query`,
  `*_batched`, `emit`, `answer`), that `let` bindings persist within a
  `session_id`, the limits, and 2 worked examples (parallel fan-out over
  subagents; batched tool calls + reduce loop).
- `parameters_schema()`:

```json
{
  "type": "object",
  "properties": {
    "script":       { "type": "string",  "description": "Rhai workflow cell to evaluate" },
    "session_id":   { "type": "string",  "description": "Continue a prior RLM session's namespace; omit for a fresh session" },
    "timeout_secs": { "type": "integer", "minimum": 1, "maximum": 3600 },
    "limits": {
      "type": "object",
      "properties": {
        "max_tool_calls":  { "type": "integer" },
        "max_agent_calls": { "type": "integer" },
        "max_model_calls": { "type": "integer" },
        "max_concurrency": { "type": "integer" }
      }
    },
    "close_session": { "type": "boolean", "description": "Close the session after this cell" }
  },
  "required": ["script"]
}
```

- `permission_level_with_args` → `Execute` (matches `spawn_subagent`).
- `external_effect_with_args` → `false` for the tool itself: every effectful
  operation a script performs goes through the bridged inner tools, each of
  which carries its **own** `external_effect` and hits the `ApprovalGate`
  middleware inside `ToolAdapter`'s call path. (Verify in Phase 5 that the
  approval middleware is actually on the bridged path; if the gate lives
  only in the harness middleware stack and not in `ToolAdapter`, invoke
  `ApprovalGate::intercept_audited` inside the bridge — fail closed.)
- `timeout_policy(args)` → `ToolTimeout::Secs(clamped timeout_secs)`,
  default `Secs(300)` — never `Inherit` (a fan-out legitimately outlives the
  default inherit budget), never `Unbounded`.
- `scope()` → `ToolScope::AgentOnly` in v1 (orchestrator surface, not
  CLI/RPC).
- `display_label` → "running RLM workflow"; `display_detail` → first line of
  the script, elided at 80 chars.

## 4.2 Registration

- Re-export from `src/openhuman/rlm/mod.rs`; glob re-export via
  `src/openhuman/tools/mod.rs` like other domains.
- Add to `all_tools_with_runtime` (`src/openhuman/tools/ops.rs`), gated:
  **not registered** when the autonomy tier is `readonly`, or when
  `OPENHUMAN_RLM=0`. Env/config default: **on** for `supervised`/`full`.
- The tool needs the turn's tool list + provider to build its bridge, so it
  is constructed with the same runtime handles `all_tools_with_runtime`
  already passes (security policy, config) plus a late-bound
  `RlmRuntimeContext` resolved per-call from the fork/turn context (the
  pattern `SpawnSubagentTool` uses via `current_parent()`).

## 4.3 Prompt & docs surfacing

- Native tool-call format carries `description()` + schema automatically —
  the primary model documentation.
- Add a "Language workflows (rlm)" section to
  `src/openhuman/agent_registry/agents/orchestrator/prompt.md`: when to
  prefer `rlm` over `spawn_parallel_agents` (ad-hoc control flow, loops,
  dedup/verify pipelines), the one-cell-per-call model, and session reuse.
- Update `src/openhuman/about_app/` feature inventory (new user-facing
  capability).
- `docs`/gitbooks: extend
  `gitbooks/developing/architecture/agent-harness.md` with the RLM section.
