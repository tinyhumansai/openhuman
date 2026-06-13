# agent_tool_policy

Profiles and enforces the tool boundary for a single agent session, keeping the prompt-visible tool set and runtime execution decisions aligned with the channel's configured permission ceiling. Given the active agent, channel, configured per-channel permission map, the available tool registry, and an optional set of explicitly-visible tool names, it produces a deterministic, immutable `ToolPolicySession` snapshot: per-tool decisions (allow / deny / hide), the allowed/blocked/hidden name sets, and a coarse task risk level. It also renders a compact system-prompt section describing the active boundary. This domain is pure logic — no persistence, no RPC, no events.

## Responsibilities

- Resolve a channel's `PermissionLevel` ceiling from a `channel -> permission` string map (`permission_for_channel`), with fallbacks.
- Classify every tool in the registry against that ceiling and the optional visibility set, producing a `ToolPolicyAction` (Allow / RequireApproval / Deny / HideFromPrompt) per tool.
- Build an immutable `ToolPolicySession` snapshot (profile, capabilities, allowed/blocked/hidden tool-name sets, decision map) attached to an agent session.
- Derive a coarse `TaskRiskLevel` (Low/Medium/High/Critical) from the highest allowed permission.
- Render a bounded `## Tool Policy Boundary` system-prompt section listing the active agent/channel/entrypoint, allowed permission, risk, allowed tools, and a restricted-count summary.
- Provide a fail-closed default decision (`Deny`) for unknown or unlisted tool names at runtime.

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/agent_tool_policy/mod.rs` | Export-only: module docstring + `mod` decls + `pub use` re-exports of the engine, prompt renderer, and types. |
| `src/openhuman/agent_tool_policy/types.rs` | Serde-free domain types: `TaskRiskLevel`, `TaskProfile`, `ToolPolicyAction`, `ToolPolicyDecision`, `ToolCapability`, `ToolPolicySession` (with query helpers). Holds the `NO_TOOLS_ALLOWED_SENTINEL`. |
| `src/openhuman/agent_tool_policy/engine.rs` | `ToolPolicyEngine::build_session` — the classification logic; private `permission_for_channel` / `parse_permission_level` helpers. Includes inline `#[cfg(test)]` suite. |
| `src/openhuman/agent_tool_policy/prompt.rs` | `render_tool_policy_boundary` + `TOOL_POLICY_BOUNDARY_HEADING`; UTF-8-safe `truncate_utf8`. Includes inline `#[cfg(test)]` suite. |

## Public surface

Re-exported from `mod.rs`:

- `ToolPolicyEngine` — `build_session(agent_id, channel, entrypoint, channel_permissions: &HashMap<String,String>, tools: &[Box<dyn Tool>], visible_tool_names: &HashSet<String>) -> ToolPolicySession`.
- `render_tool_policy_boundary(session: &ToolPolicySession, max_bytes: usize) -> Option<String>` — `None` when the session has no restrictions; otherwise a truncated prompt section.
- Types: `TaskProfile`, `TaskRiskLevel`, `ToolCapability`, `ToolPolicyAction`, `ToolPolicyDecision`, `ToolPolicySession`.

`ToolPolicySession` helpers: `is_allowed(name)`, `has_restrictions()`, `restricted_tool_count()`, `visible_tool_names_for_prompt()`, `decision_for(name)` (defaults to `Deny`). `ToolPolicyDecision::is_denied()` is true for anything other than `Allow`.

## Dependencies

- `crate::openhuman::tools` — `PermissionLevel` (the ceiling/ordering, parsed and compared) and the `Tool` trait (`name()`, `permission_level()`). The only openhuman/core dependency.
- stdlib `std::collections` (`BTreeSet`/`HashMap`/`HashSet`) and `log` for grep-friendly `[tool-policy]` diagnostics under target `openhuman::agent_tool_policy`.

## Used by

- `src/openhuman/agent/harness/session/builder.rs` — builds the `ToolPolicySession` (`ToolPolicyEngine`, `ToolPolicySession`).
- `src/openhuman/agent/harness/session/runtime.rs` — uses `ToolPolicyEngine`.
- `src/openhuman/agent/harness/session/turn.rs` — calls `render_tool_policy_boundary` to inject the boundary into the prompt.
- `src/openhuman/agent/harness/session/types.rs` — carries `ToolPolicySession` on the session.

## Notes / gotchas

- **Legacy escape hatch**: an empty `channel_permissions` map yields `PermissionLevel::Dangerous` (fully unrestricted), preserving pre-policy behavior. Once *any* channel policy exists, channels missing from the map (or with an unparseable value) fall back to `PermissionLevel::ReadOnly`, not unrestricted.
- **Permission parsing** (`parse_permission_level`) is lenient: trims, lowercases, strips `-`/`_`, and accepts aliases (`read`/`readonly`, `exec`/`execute`, `danger`/`dangerous`). Unrecognized tokens fall back to read-only.
- **Two independent restriction axes**: a tool can be `Deny`ed (exceeds the permission ceiling → `blocked_tool_names`) or `HideFromPrompt` (not in the non-empty `visible_tool_names` set → `hidden_tool_names`). Hidden takes precedence over deny in the classification order.
- `ToolPolicyAction::RequireApproval` is defined and handled in the match (routed to `blocked_tool_names`) but `build_session` never currently produces it.
- `visible_tool_names_for_prompt()` inserts `NO_TOOLS_ALLOWED_SENTINEL` when restrictions exist but nothing is allowed, so prompt rendering can signal an empty-but-restricted surface rather than an unrestricted one.
- `render_tool_policy_boundary` returns `None` for unrestricted sessions, and `truncate_utf8` guarantees the output stays within `max_bytes` on a char boundary (appending `\n[...truncated]` only when there is room).
- Snapshots are immutable and deterministic per session; there is no mutation API.
