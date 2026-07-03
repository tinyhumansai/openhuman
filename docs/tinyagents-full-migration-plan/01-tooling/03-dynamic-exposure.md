# 01.3 — Dynamic tool exposure as middleware

Replace scattered pre-filtering with crate selection middleware.

Current OpenHuman surfaces: `agent/harness/tool_filter.rs` (mechanics),
`tools/user_filter.rs`, sub-agent `tool_prep.rs`, dynamic delegation refresh in
`session/turn/tools.rs`, channel permission ceiling checks.

Current status: the shared TinyAgents runner still registers only the
OpenHuman-computed callable tools, preserving hidden-tool execution semantics.
It now tags OpenHuman run contexts and emits a diagnostics-only
`AgentEvent::ToolsFiltered` via `openhuman_tool_visibility` when that existing
allowlist withholds candidate tools. Full middleware-owned selection remains.
Production tool exposure now goes exclusively through
`run_turn_via_tinyagents_shared` + `SharedToolAdapter`; the old non-shared
`ToolAdapter` wrapper is test-only.

`agent/harness/tool_filter.rs` is still live for `integrations_agent` toolkit
spawns: the runner uses it to choose a compact Composio action set before
registering dynamic tools.
`src/openhuman/agent/harness/subagent_runner/tool_prep.rs` is also still live,
and it currently mixes filtering (`filter_tool_indices`, nested-delegation
stripping, denylist checks, toolkit top-K budgets) with non-filter helpers
(`load_prompt_source`, text-mode protocol instructions). Delete it only after
`ContextualToolSelectionMiddleware` owns child/toolkit selection and those
non-filter helpers have moved to narrower modules. The `tool_prep` helper
surface has been narrowed to `subagent_runner` except for the text-mode protocol
renderer used by prompt debug dumps. The turn-local parent-context/progress and
cached integration refresh helpers in `session/turn/tools.rs` are scoped to the
turn module, while the cross-surface integration fetch and delegation refresh
entrypoints remain live.

## Steps

1. Express agent `tool_allowlist`/`tool_denylist`, sub-agent tool scope,
   MCP tool visibility, and the channel permission ceiling as a composed
   `ToolAllowlistMiddleware` + one OpenHuman
   `ContextualToolSelectionMiddleware`. TinyAgents 1.3.0's selection context
   carries run id, depth, tags, and requested model; OpenHuman-specific agent
   id, task kind, security tier, and channel either stay in local middleware
   state or are encoded into tags. Inheritance rule: children can only narrow —
   use `ContextualToolSelectionMiddleware::inheriting(...)`.
2. Fail closed when policy metadata is missing (unclassified → not exposed);
   exposure decisions are event-native via `AgentEvent::ToolsFiltered
   { by, excluded, remaining }` (1.3.0) — projected into the bridge as
   structured diagnostics today.
3. Move the visible-set computation, dynamic delegation refresh, and skill-event
   catalogue reconciliation out of
   `src/openhuman/agent/harness/session/turn/tools.rs` (616 lines) plus
   `subagent_runner/tool_prep.rs` (344 lines) into the middleware; the turn code
   only declares candidate tool sets and parent execution context.
4. Keep product policy sources (registry definitions, security tier tables)
   in OpenHuman — the middleware consumes them.

## Deletions

- `agent/harness/tool_filter.rs` mechanics (299 lines; keep policy tables if
  any inline).
- `subagent_runner/tool_prep.rs` (344 lines).
- Filtering/refresh blocks in `session/turn/tools.rs` (file shrinks to
  parent-context and assembly glue).

## Acceptance

- Sub-agents can never see a tool the parent couldn't grant (test).
- Exposure decision visible in run events.
- CLI/RPC-only denial (`CliRpcOnlyMiddleware`) unaffected or folded in.
