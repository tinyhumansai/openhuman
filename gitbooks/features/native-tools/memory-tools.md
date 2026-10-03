---
description: How the agent recalls, fetches, learns and forgets with its single memory tool.
icon: brain
---

# Memory Tools

[Memory](../memory.md) is OpenHuman's knowledge base. The agent talks to it through one tool, `memory`, whose `action` is one of:

| Action | What it does |
| --- | --- |
| `recall` | Ask a question; returns an answer and the citations it rests on. |
| `fetch` | Raw search (hybrid) over stored items, optionally filtered by metadata. Returns hits. |
| `learn` | Save one durable learning (a preference, fact, procedure or correction). |
| `forget` | Remove items by id. |

The tool is only registered when a memory engine is usable. With none selected, memory is off and the agent never sees it. Each learning is tagged with the workspace, thread, agent and tool call that produced it.

## Why a tool, not implicit context

Memory is too large to put in every prompt. Each new chat starts with a short compiled brief ([context.md](../memory.md#contextmd)); for everything else the model asks ("what do I know about the Stripe webhook?") and gets back just the relevant answer with citations.

External MCP clients get the same abilities from OpenHuman's [MCP server](../../developing/mcp-server.md) as `memory.recall`, `memory.fetch`, `memory.list`, `memory.learn` and `memory.forget`.

## See also

- [Memory](../memory.md)
