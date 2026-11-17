---
description: >-
  How OpenHuman learns your preferences from everyday use: learnings in memory,
  the memory pack recalled for each turn, and your persona files.
icon: brain
---

# Personalization and self-learning

OpenHuman gets to know you through [memory](memory.md), not a settings form. Three things shape how it treats you, and you can see and edit all of them.

## Learnings

A learning is one durable statement: a preference, fact, procedure or correction, such as "prefers pnpm" or "never schedule meetings before 10". The agent saves one with its `memory` tool (the `learn` action) when you state or correct something worth keeping. You can add your own on **Connections → Memory → Learnings**. Each learning records where it came from (workspace, thread, agent and tool call). Delete any learning there, or ask the agent to forget it.

Learnings live in your selected memory engine, so memory has to be on (see [Engines](memory.md#engines)). Secrets and personal identifiers are scrubbed before storing.

## The memory pack

Before every turn, OpenHuman recalls a short, token-budgeted memory pack. It holds relevant learnings (and beliefs the engine built from them), documents from your brain, this agent's earlier conversations and, briefly, other agents' turns.

The pack goes into that turn's model request only. It is never written into the chat. So the agent follows your preferences without being asked, and the prompt cache is unaffected. Preview the pack on **Connections → Memory → Ask**. See [How the agent uses memory](memory.md#how-the-agent-uses-memory).

## Persona files

How the agent presents itself is separate from memory. It comes from `SOUL.md`, `IDENTITY.md` and `ROLE.md` in your workspace. These are plain Markdown files you edit directly.

## See also

- [Memory](memory.md): the engines, sources and tabs behind all of this.
- [Memory tools](native-tools/memory-tools.md): how the agent recalls and learns.
