# agent_policy

Profiles and enforces the tool boundary for a single agent session, keeping the prompt-visible tool set and runtime execution decisions aligned with the channel's configured permission ceiling. Given the active agent, channel, the `channel_permissions` map from `AgentConfig`, the session's tool set, and an optional set of explicitly-visible tool names, it produces a deterministic, immutable `ToolPolicySession` snapshot: per-tool decisions (allow / deny / hide), the allowed/blocked/hidden name sets, and a coarse task risk level. It also renders a compact system-prompt section describing the active boundary. Pure logic: no persistence, no RPC, no bus events.

Not to be confused with `crate::agent::tool_policy` (the generic `ToolPolicy` pre-execution hook trait). That module has its own `ToolPolicyDecision` type; the two are unrelated.

## Responsibilities

- Resolve a channel's `PermissionLevel` ceiling from the `channel -> permission` string map (`permission_for_channel`), with fallbacks.
- Classify every tool against that ceiling and the optional visibility set, producing a `ToolPolicyAction` (Allow / RequireApproval / Deny / HideFromPrompt) per tool.
- Build an immutable `ToolPolicySession` snapshot (profile, capabilities, allowed/blocked/hidden tool-name sets, decision map).
- Derive a coarse `TaskRiskLevel` (Low/Medium/High/Critical) from the allowed permission.
- Render a bounded `## Tool Policy Boundary` system-prompt section listing the active agent/channel/entrypoint, allowed permission, risk, allowed tools, and a restricted-count summary.
- Provide a fail-closed default decision (`Deny`) for unknown or unlisted tool names at runtime.

## Key files

| File | Role |
| --- | --- |
| `crates/openhuman-core/src/tools/agent_policy/mod.rs` | Export-only: module docstring + `mod` decls + `pub use` re-exports of the engine, prompt renderer, and types. |
| `crates/openhuman-core/src/tools/agent_policy/types.rs` | Serde-free domain types: `TaskRiskLevel`, `TaskProfile`, `ToolPolicyAction`, `ToolPolicyDecision`, `ToolCapability`, `ToolPolicySession` (with query helpers). Holds the private `NO_TOOLS_ALLOWED_SENTINEL`. |
| `crates/openhuman-core/src/tools/agent_policy/engine.rs` | `ToolPolicyEngine::build_session` / `build_session_from_refs` — the classification logic; private `permission_for_channel` / `parse_permission_level` helpers. |
| `crates/openhuman-core/src/tools/agent_policy/engine_tests.rs` | Engine tests, attached via `#[path]` from `engine.rs`. |
| `crates/openhuman-core/src/tools/agent_policy/prompt.rs` | `render_tool_policy_boundary` + `TOOL_POLICY_BOUNDARY_HEADING`; private UTF-8-safe `truncate_utf8`. |
| `crates/openhuman-core/src/tools/agent_policy/prompt_tests.rs` | Prompt-rendering tests, attached via `#[path]` from `prompt.rs`. |

## Public surface

Re-exported from `mod.rs`:

- `ToolPolicyEngine::build_session(agent_id, channel, entrypoint, channel_permissions: &HashMap<String,String>, tools: &[Box<dyn Tool>], visible_tool_names: &HashSet<String>) -> ToolPolicySession`.
- `ToolPolicyEngine::build_session_from_refs(..., tools: &[&dyn Tool], ...)` — same classification over borrowed tools so the harness can classify its durable registry and its synthesized delegate tools together; `build_session` delegates to it.
- `render_tool_policy_boundary(session: &ToolPolicySession, max_bytes: usize) -> Option<String>` — `None` when the session has no restrictions; otherwise a truncated prompt section.
- Types: `TaskProfile`, `TaskRiskLevel`, `ToolCapability`, `ToolPolicyAction`, `ToolPolicyDecision`, `ToolPolicySession`.

`ToolPolicySession` helpers: `is_allowed(name)`, `has_restrictions()`, `restricted_tool_count()`, `visible_tool_names_for_prompt()`, `decision_for(name)` (defaults to `Deny`).

`ToolPolicyDecision` has two predicates that answer different questions:

- `is_denied()` — true for anything other than `Allow`, including `HideFromPrompt`. Right for a direct tool call by name.
- `blocks_execution()` — true for `Deny` / `RequireApproval`, or when `required_permission` exceeds `allowed_permission`. `HideFromPrompt` alone does not block, so withheld packed tools stay reachable through `use_skill`. The permission re-check exists because the classifier tests hidden-ness before the ceiling, so a hidden tool over the ceiling is recorded as `HideFromPrompt` only.

## Dependencies

- `crate::tools` — `PermissionLevel` (ordered, parsed and compared) and the `Tool` trait (`name()`, `permission_level()`). The only in-crate dependency.
- `log` for `[tool-policy]` diagnostics; every call sets `target: "openhuman::tools::agent_policy"` explicitly (the lib crate is named `openhuman`).

## Used by

- `crates/openhuman-core/src/agent/session_host/builder/builder_build.rs` — builds two snapshots with `build_session_from_refs`: the session's `tool_policy_session` (with the role visibility filter) and a `channel_policy_session` without it, used to derive the sub-agent tool ceiling.
- `crates/openhuman-core/src/agent/session_host/builder/mod.rs` — `visible_tool_specs_for_policy` filters prompt-visible specs with `decision_for(name).blocks_execution()`.
- `crates/openhuman-core/src/agent/session_host/runtime/accessors.rs` — `rebuild_tool_policy_session` re-runs `build_session_from_refs` after `hide_tools`.
- `crates/openhuman-core/src/agent/session_host/turn/context.rs` — calls `render_tool_policy_boundary(&session, 2048)` and appends the block to the end of the system prompt.
- `crates/openhuman-core/src/agent/session_host/types.rs` — carries `tool_policy_session` on the session.
- `crates/openhuman-core/src/agent/tinyagents/middleware/tool_policy.rs` — `ToolPolicyMiddleware` is the runtime enforcement point: `is_denied()` for direct calls, `blocks_execution()` for the `use_skill` route. The snapshot reaches it through `ToolPolicyEnforcement` in `agent/tinyagents/turn_policy.rs`.
- `crates/openhuman-core/src/agent/tinyagents/host/security_gate.rs` — `SecurityGate::with_tool_policy` accepts an `Arc<ToolPolicySession>` and maps its actions to gate verdicts.
- `crates/openhuman-core/src/config/schema/agent.rs` — the `channel_permissions` field docs describe the engine's fallback semantics.

## Notes / gotchas

- **Legacy escape hatch**: an empty `channel_permissions` map yields `PermissionLevel::Dangerous` (fully unrestricted). `AgentConfig::migrate_channel_permissions_if_legacy` seeds the map on first boot, so this branch is only hit before that migration. Once *any* entry exists, channels missing from the map (or with an unparseable value) fall back to `PermissionLevel::ReadOnly`.
- **Permission parsing** (`parse_permission_level`) is lenient: trims, lowercases, strips `-`/`_`, and accepts aliases (`read`/`readonly`, `exec`/`execute`, `danger`/`dangerous`). Unrecognized tokens fall back to read-only.
- **Two independent restriction axes**: a tool can be `Deny`ed (exceeds the permission ceiling → `blocked_tool_names`) or `HideFromPrompt` (not in the non-empty `visible_tool_names` set → `hidden_tool_names`). Hidden is tested first, so a tool that is both hidden and over the ceiling lands only in `hidden_tool_names`; `blocks_execution()` carries the ceiling check for that case.
- An empty `visible_tool_names` set means "all visible", not "none visible".
- `ToolPolicyAction::RequireApproval` is handled (routed to `blocked_tool_names`) but the engine never produces it.
- `visible_tool_names_for_prompt()` inserts `NO_TOOLS_ALLOWED_SENTINEL` when restrictions exist but nothing is allowed, so callers can distinguish an empty-but-restricted surface from an unrestricted one.
- `truncate_utf8` keeps `render_tool_policy_boundary` output within `max_bytes` on a char boundary, appending `\n[...truncated]` only when there is room.
