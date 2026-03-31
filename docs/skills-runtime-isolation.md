# Skills Runtime Isolation (QuickJS)

This document defines OpenHuman's skill isolation contract for the QuickJS runtime.

## Goals

- Keep each running skill isolated in JavaScript execution state.
- Prevent lateral movement where one skill invokes another skill's tools.
- Preserve external orchestration from trusted host surfaces (RPC/UI/socket MCP).

## Runtime Architecture

### Per-skill components

- `QjsSkillInstance` (`src/openhuman/skills/qjs_skill_instance`) per started skill.
- One dedicated `AsyncRuntime` + `AsyncContext` created during skill spawn.
- Skill-local JS globals (`globalThis`), module state, and in-memory variables.
- Skill-local data path (`skills_data/<skill_id>/...`) and namespaced memory writes.
- Skill-local message loop handling `SkillMessage` commands.

### Shared infrastructure

- `RuntimeEngine` as lifecycle orchestrator.
- `SkillRegistry` for tracking/routing running skills.
- `SocketManager` for external MCP transport.
- Schedulers (`CronScheduler`, `PingScheduler`) and preferences store.

## Lifecycle and Reset Semantics

1. `start_skill(skill_id)` creates a fresh skill instance and QuickJS context.
2. The instance initializes (`init/start`) and registers tools/state.
3. During runtime, only that skill's event loop mutates its JS state.
4. `stop_skill(skill_id)` stops the loop and transitions status.
5. Restart creates a new instance/context; previous JS globals are not reused.

Failure modes:

- Tool dispatch to non-running skill returns a status error.
- Reply channel drop returns a deterministic runtime error.
- Policy denials return explicit "cross-skill forbidden" errors.

## Tool Invocation Policy

Policy is enforced at host boundary (Rust), not in JS:

- `External` origin (JSON-RPC/UI/socket MCP): may call any target skill tool.
- `SkillSelf { skill_id }` origin: may call only tools owned by the same `skill_id`.
- Cross-skill attempt from `SkillSelf` is denied before dispatch.

This policy is implemented in `SkillRegistry::call_tool_scoped`.

## Forbidden Paths

- A running skill may not invoke another skill's tools.
- Inter-skill bridge helpers must not bypass host policy.
- `skills_call` generic cross-skill agent tool is not part of the default tool registry.

## Testing Requirements

Runtime policy tests must cover:

- external origin allowed,
- same-skill origin allowed,
- cross-skill origin denied with clear error.

Regression checks should also ensure the default agent tool registry does not include
legacy cross-skill helper surfaces.
