---
description: Durable, tool-scoped rules for safety-critical guidance and learnings.
icon: shield-check
---

# Tool-Scoped Memory

The tool-scoped memory layer captures **actionable guidance** about how the agent should use specific tools - separate from the [Memory Tools](memory-tools.md) general-purpose recall and from the `tool_effectiveness` statistics namespace. It is the surface that turns "never email Sarah" into a hard constraint the agent has to obey on every subsequent turn.

It implements [issue #1400](https://github.com/tinyhumansai/openhuman/issues/1400) - a first-class storage and retrieval system for durable learnings and high-priority rules.

## What it stores

Every tool gets its own namespace, **`tool-{tool_name}`**, distinct from `global`, `skill-{id}`, and the statistics-only `tool_effectiveness` namespace. Inside it, each entry is a `ToolMemoryRule`:

| Field        | Purpose |
| ------------ | ------- |
| `id`         | Stable per-rule UUID. Upserts replay the same id. |
| `tool_name`  | The tool the rule applies to (e.g. `send_email`, `shell`). |
| `rule`       | Natural-language guidance the agent must follow. |
| `priority`   | `critical`, `high`, or `normal`. Drives retrieval + compression. |
| `source`     | `user_explicit`, `post_turn`, or `programmatic` - provenance. |
| `tags`       | Free-form labels (`safety`, `permission`, ...). |
| `created_at` / `updated_at` | RFC3339 timestamps. |

Statistics (`tool_effectiveness/tool/{name}`) and rules (`tool-{name}/rule/{id}`) live in *different* namespaces by design - one tracks "what happened", the other tracks "what to do about it".

## Priority levels

| Priority   | Where it lives | Compression-resistant? |
| ---------- | -------------- | ---------------------- |
| `critical` | Pinned into the **system prompt** via `ToolMemoryRulesSection`. | **Yes** - the system prompt is frozen per-session and never rewritten by the mid-session compactor. |
| `high`     | Same system prompt block, ranked below critical. | **Yes** - same mechanism. |
| `normal`   | Stored in the namespace; retrieved on demand via `memory_recall`. | No - eligible for compression like any other namespaced memory. |

The compression-resistance property is structural: critical and high rules ride in the *system prompt*, which the inference backend's prefix cache keeps frozen for the entire session. There is no way for token compression to silently drop a `critical` rule.

## Capture pipeline

Two automatic capture paths fire after every turn (via `ToolMemoryCaptureHook`):

1. **User edicts** - sentences like `never <verb> <noun>`, `don't <verb> ...`, `do not <verb> ...`, or `stop <verb>ing ...` in the user message are promoted to a **Critical** rule on the matching tool. Common-noun aliases map `"email"` to a tool named `send_email`, `"shell"` to `bash`/`exec`, etc.; when no alias matches, the rule lands on the first tool that ran in the turn so it stays adjacent to the relevant call site.
2. **Repeated tool failures** - a tool that fails twice or more in a single turn earns a **Normal**-priority observation, with the failure class summarized inline so the agent has context next time it considers that tool.

The hook is enabled by default whenever the learning subsystem is on. Disable selectively with `OPENHUMAN_LEARNING_TOOL_MEMORY_CAPTURE_ENABLED=0`.

## Retrieval at tool-selection time

At session start the harness pre-fetches every Critical and High rule via `ToolMemoryStore::rules_for_prompt`, renders them into the `## Tool-scoped rules` block, and pins the block into the system prompt. Because the prompt is frozen for the lifetime of the session, the rules are visible on every turn at tool-selection time and before any actual tool execution.

Lower-priority guidance stays out of the prompt budget; the agent reaches it on demand by calling `memory_recall` against the `tool-{name}` namespace.

## RPC surface

Six methods are exposed under the `memory` namespace:

| Method                              | Purpose |
| ----------------------------------- | ------- |
| `memory.tool_rule_put`              | Upsert a rule. Use `priority='critical'` for safety-critical entries. |
| `memory.tool_rule_get`              | Fetch a rule by `(tool_name, id)`. |
| `memory.tool_rule_list`             | List all rules for a tool, sorted by priority + freshness. |
| `memory.tool_rule_delete`           | Delete a rule. |
| `memory.tool_rules_for_prompt`      | Return the rendered Markdown block + structured snapshot - what the session builder pins. |
| `memory.tool_rules_json`            | Raw JSON list (for envelope consumers). |

JSON payloads use snake_case (`priority: "critical"`, `source: "user_explicit"`). Every method goes through the same `active_memory_client` plumbing as the rest of the memory RPCs.

## End-to-end safety case

The "never email Sarah" path is covered as a regression test:

1. User says *"Never email Sarah at sarah@example.com."* during a turn that called `send_email`.
2. `ToolMemoryCaptureHook` extracts the edict, maps the `email` alias to the `send_email` tool, and writes a Critical rule under `tool-send_email/rule/{uuid}`.
3. On the next session, `prefetch_tool_memory_rules_blocking` pulls every Critical and High rule and the session builder appends a `ToolMemoryRulesSection` to the system prompt.
4. The agent sees `### \`send_email\`` followed by `- **[critical]** Never email Sarah at sarah@example.com.` before ever choosing a tool, and the rule survives any mid-session token compression.

Coverage and the integration test live in `src/openhuman/memory/tool_memory/`.

## See also

- [Memory Tools](memory-tools.md) - general-purpose `recall`, `store`, `forget`.
- [Smart Token Compression](../token-compression.md) - what the system prompt is protected against.
