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

## Tier 3 — Spawn a sub-agent (inline)
Apply when: the task requires specialised execution (writing code, crawling docs, running shell, calling an external integration) that the orchestrator cannot do directly.
Cost: full sub-agent turn (~1-5k tokens depending on archetype).
Rule: spawn the narrowest archetype that can complete the task. Prefer inline spawn (`spawn_worker_thread` with no dedicated thread) for tasks that complete in <5 turns.

## Tier 4 — Spawn a dedicated worker thread
Apply when: the task is long (>5 turns estimated), produces a large transcript, or the user explicitly wants it tracked as a separate thread.
Cost: same as Tier 3 but the parent thread is not flooded.
Rule: use `spawn_worker_thread` and surface a brief summary back to the parent. Do not chain workers (workers cannot spawn workers).

## Anti-patterns to avoid
- Spawning a sub-agent to answer a question the orchestrator already has context for.
- Delegating a tool call to a sub-agent when `current_tier <= 2` applies.
- Using `spawn_subagent` when `delegate_{archetype}` covers the task — `delegate_*` tools carry the full archetype definition and have correct tool filtering pre-configured.
- Passing the entire parent conversation as context to a sub-agent — pass only the task-relevant slice.
