# Delegation Policy

## When to delegate vs. act directly

The orchestrator follows a direct-first policy. This document codifies the four-tier decision tree the orchestrator applies to every user message.

## Tier 1 — Reply directly (no tools)
Apply when: small talk, simple factual Q&A, acknowledgements, clarification requests, context already in the system prompt.
Cost: 0 tokens (output only).
Rule: if you can answer without calling any tool, do so.

## Tier 2 — Use a direct tool
Apply when: the task needs a tool but not specialised execution (time lookup, memory read/write, cron scheduling, workspace state, listing connections).
Cost: 1 tool call + parse overhead (~200-400 tokens).
Rule: prefer `current_time`, `cron_*`, `memory_*`, `memory_tree`, `read_workspace_state`, `composio_list_connections`, `ask_user_clarification`.

## Tier 3 — Delegate to a reusable async sub-agent
Apply when: the task requires specialised execution (writing code, crawling docs, running shell, calling an external integration) that the orchestrator cannot do directly.
Cost: full sub-agent turn (~1-5k tokens depending on archetype).
Rule: spawn the narrowest archetype that can complete the task. `spawn_subagent` is reusable and asynchronous by default: it first looks for a compatible durable worker for the same parent thread, agent id, toolkit/model/sandbox/action-root shape, and deterministic task key. If one is running, the new instruction is steered into it; if one is idle, it resumes from saved history; otherwise a new durable worker session is created.

## Tier 4 — Explicit worker lifecycle control
Apply when: the task is long (>5 turns estimated), produces a large transcript, needs manual inspection, or the user explicitly wants it tracked/closed separately.
Cost: same as Tier 3 but the parent thread is not flooded.
Rule: use `list_subagents`, `steer_subagent`, `wait_subagent`, and `close_subagent` with `subagent_session_id` for durable control. Use `fresh: true` only when the prior worker is materially incompatible or the user asks for a clean worker. Use `blocking: true` only when the parent must synchronously wait for the child result before replying. Do not chain workers (workers cannot spawn workers).

## Anti-patterns to avoid
- Spawning a sub-agent to answer a question the orchestrator already has context for.
- Delegating a tool call to a sub-agent when `current_tier <= 2` applies.
- Repeatedly forcing fresh sub-agents for the same logical job, which loses worker-local context and cache affinity.
- Passing the entire parent conversation as context to a sub-agent — pass only the task-relevant slice.
