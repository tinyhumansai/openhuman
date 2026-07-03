# 01.2 — Unknown-tool recovery via RunPolicy

Crate `RunPolicy.unknown_tool` now has `UnknownToolPolicy::{Fail,
ReturnToolError, Rewrite { tool_name }}` plus an `AgentEvent::UnknownToolCall`
variant — the sentinel is deletable.

Current status (2026-07-02): `run_policy_for` sets
`UnknownToolPolicy::ReturnToolError`, `OpenhumanEventBridge` projects
`AgentEvent::UnknownToolCall` into grep-friendly diagnostics, and the old
sentinel/middleware symbols are gone from source.

## Steps

1. Done: `run_policy_for` sets `RunPolicy.unknown_tool =
   UnknownToolPolicy::ReturnToolError`, preserving the legacy "model corrects
   itself" behavior with the original requested name).
2. Done: no wording-only sentinel shim remains in `tinyagents/middleware.rs`.
3. Done: `AgentEvent::UnknownToolCall` reaches `OpenhumanEventBridge` and is
   projected distinctly from "tool executed and failed" in debug logs.

## Deletions

- Deleted: `UNKNOWN_TOOL_SENTINEL` + sentinel tool registration in
  `src/openhuman/tinyagents/tools.rs`.
- Deleted: `UnknownToolRewriteMiddleware` in `src/openhuman/tinyagents/middleware.rs`
  (+ its tests, rewritten as RunPolicy behavior tests).

## Acceptance

- Hallucinated tool name -> recoverable tool error, run continues; event
  stream records the original requested name; no sentinel appears in
  transcripts or advertised schemas.
- Sub-agent and top-level tests accept TinyAgents' fixed recoverable-tool-error
  wording while preserving the original requested tool name in events/logs.
