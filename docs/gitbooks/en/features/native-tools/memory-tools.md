---
description: How the agent recalls, fetches, learns and forgets with its single memory tool.
icon: brain
---

# Memory tools

[Memory](../memory.md) is OpenHuman's knowledge base. The agent uses it through one tool, `memory`, whose `action` is one of these:

| Action | What it does |
| --- | --- |
| `recall` | Ask a question. Returns an answer and the citations it rests on. |
| `fetch` | Raw hybrid search over stored items, optionally filtered by metadata. Returns hits. |
| `learn` | Queue one durable learning: a preference, fact, procedure or correction. |
| `forget` | Queue removal of items by id. |

The tool is registered only when a memory engine is usable. With none selected, memory is off and the agent never sees the tool. Every learning is shared with all agents under the same memory root. It is tagged with the workspace, thread, agent and tool call that produced it.

## The tool and the per-turn pack

A turn can receive a completed cached [memory pack](../memory.md#how-the-agent-uses-memory) (`<memory-context>`). A cold cache supplies none. The pack is labelled as context from an earlier background lookup and can be incomplete or unrelated to the current question. Automatic refresh and turn logging never wait on the remote engine before the model runs.

Use explicit `recall` or `fetch` when a current answer is needed, or to go further than the pack. The agent can ask a question the pack does not cover ("what do I know about the Stripe webhook?"), search with metadata filters, or save something worth remembering next time.

## Deadlines and queued writes

Agent memory calls have a 15-second per-call deadline, a shared 30-second budget and at most eight attempts per tracked run. Concurrent calls reserve from the same budget. If a read times out or the budget is exhausted, the tool tells the agent to continue without memory and not retry it in that run. A new run has a fresh budget.

`learn` and `forget` persist a validated, scrubbed request in a private local outbox and start background delivery. The statuses "queued; not yet saved to memory" and "queued; not yet removed from memory" confirm local acceptance only. The agent must not describe them as completed remote saves or deletions, or repeat them to force indexing. Pending delivery can resume after a restart, sign-in or background tick. Queue admission can fail when its limits are reached.

These agent-tool limits do not replace the behavior of explicit Memory-page or RPC workflows.

External MCP clients get the same abilities from OpenHuman's [MCP server](../../developing/mcp-server.md) as `memory.recall`, `memory.fetch`, `memory.list`, `memory.learn` and `memory.forget`.

## See also

- [Memory](../memory.md)
