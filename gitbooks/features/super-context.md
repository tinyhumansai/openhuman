---
description: >-
  Deterministic, harness-driven context preparation on the first turn of every
  new conversation - so the agent never starts cold.
icon: sparkles
---

# SuperContext

A fresh chat shouldn't start cold. **SuperContext** makes the agent gather relevant background _before_ it reads your first message. It happens automatically, on every new thread, without you asking and without waiting on a tool call.

Most agents start a conversation blank and only fetch context if the model decides to call a "look things up" tool. That adds a round-trip, costs tokens, and depends on the model choosing well. SuperContext flips it: the harness itself prepares context up front, deterministically, so the very first reply already knows the relevant memories, files, and connected data.

***

## How it works

On the **first turn of a new thread**, if SuperContext is enabled, the harness:

1. Spawns a read-only `context_scout` sub-agent.
2. The scout sweeps your available data (the [Memory Tree](obsidian-wiki/memory-tree.md), workspace files, and connected integrations) and assembles a bounded **context bundle**.
3. The bundle is validated, then prepended to your message under a `Prepared context (super context)` header before the orchestrator model ever sees the turn.
4. The model answers your message already grounded in that context.

```text
New thread, first message
        │
        ▼
┌──────────────────────────────┐
│  Harness gate (deterministic) │
│  super_context_enabled?       │
└───────────────┬──────────────┘
                │ yes
                ▼
        context_scout (read-only)
        sweeps memory + files + data
                │
                ▼
        [context_bundle] … [/context_bundle]
                │  validated & extracted
                ▼
   Prepended to your message → orchestrator
```

Because the scout is **read-only**, it can never take an action on a fresh thread. It only reads and summarizes. And because it runs in the harness rather than as an optional tool, the redundant `agent_prepare_context` tool is suppressed for that turn, so the agent doesn't do the work twice.

The scout runs on the **`burst` tier** (`hint = "burst"` → `burst-v1` on the managed backend), a cheap, high-throughput, non-reasoning model. The sweep is a latency-tolerant pre-flight pass, so raw throughput on a fast model is a better fit than the pricier agentic/reasoning tiers. See [Automatic Model Routing](model-routing/README.md).

***

## Safety and robustness

The scout returns its findings wrapped in `[context_bundle] … [/context_bundle]` tags. Only the bracketed envelope is ever injected. Any surrounding prose the model emits ("sure, here's what I found…") is stripped out. If the bundle is missing, malformed (unterminated, reversed, or duplicated tags), or empty, the turn proceeds **gracefully without augmentation** rather than injecting garbage. A cold start is always preferable to a broken one.

***

## Turning it on or off

SuperContext is **on by default**.

* **From the composer.** A **Super Context** toggle appears below the chat input on a fresh thread. The flag is read when a thread is constructed, so toggling it affects **newly started threads**, not the one you're already in.
* **Config.** `context.super_context_enabled` (boolean, default `true`).
* **Environment.** `OPENHUMAN_SUPER_CONTEXT` (or `OPENHUMAN_CONTEXT_SUPER_CONTEXT_ENABLED`).
* **RPC.** `get_super_context_enabled()` reads the flag; `set_super_context_enabled(value)` sets and persists it.

***

## See also

* [Memory Tree](obsidian-wiki/memory-tree.md): the primary source the scout reads from.
* [Auto-fetch from Integrations](obsidian-wiki/auto-fetch.md): keeps that source fresh between conversations.
* [Subconscious Loop](subconscious.md): the other side of "keeps thinking when you've stopped typing."
