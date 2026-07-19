---
description: >-
  OpenHuman is an orchestrator, not a chatbot: durable agent graphs, visual
  workflows, sub-agent fleets, a split-brain always-on layer, and end-to-end
  encrypted agent-to-agent sessions, all in one coherent stack.
icon: sitemap
---

# The Orchestrator

<figure><img src="../.gitbook/assets/orchestration.png" alt=""><figcaption><p>OpenHuman orchestrating a fleet of agents (its own workers and external harnesses alike) over Signal-encrypted agent-to-agent sessions.</p></figcaption></figure>

Most harnesses run one agent in one loop. OpenHuman is built as an **orchestrator**: a stack for coordinating many agents, over long horizons, across machines. It does this durably, observably, and under your control.

Five layers make that real:

## 1. Graphs, not loops

Every agent turn runs on [tinyagents](https://github.com/tinyhumansai/tinyagents), our open-source graph engine. Multi-step work compiles to **state-machine graphs with conditional routing**: `plan → execute ⇄ review → finalize` for delegation, phase DAGs for multi-agent workflow runs, and map-reduce fan-out for parallel workers. All of it has **durable checkpointing**. A graph can pause mid-run (for your answer, for an approval, for a restart) and resume exactly where it stopped.

## 2. Sub-agent fleets that don't get lost

The orchestrator spawns specialized sub-agents (up to 3 levels deep), reuses compatible idle workers instead of re-spawning, and routes each to the right model tier: heavy reasoning for the core, a fast **burst tier** for low-context workers. Reliability is structural: a no-progress circuit breaker stops loops, and stuck children hand back a `question` (pause + resume on your answer) or an `Incomplete` root-cause summary, never silence. See the [Agent Harness](../developing/architecture/agent-harness.md).

## 3. Workflows you can see

[Workflows](workflows.md) lift orchestration out of the chat: the agent *proposes* a typed graph of triggers, agents, tools and conditions; you review it on a canvas and save it. Runs are durable, approval-gated, and fully inspectable step-by-step, powered by open-source [tinyflows](https://github.com/tinyhumansai/tinyflows).

## 4. An always-on split brain

Inbound traffic hits a **fast reflex agent** that triages in seconds and hands a deep **reasoning core** a concise brief; the core does the multi-step work and delegates to workers. The [subconscious loop](subconscious.md) reviews compressed session history and injects steering directives, keeping the always-on layer aligned with your goals, while 20:1 compression keeps week-long sessions bounded.

## 5. Encrypted orchestration across machines

OpenHuman instances collaborate through [tiny.place](tinyplace.md) sessions secured with the **Signal protocol**: real end-to-end encryption, with keys derived on-device and never persisted. Pairing is consent-based and fails closed: an unlinked agent's message is just a message, never an instruction. Your agent can orchestrate other agents (and be orchestrated) without a server ever seeing plaintext.

Because the transport is plain Signal-encrypted messaging, the other side doesn't have to be OpenHuman. Connect **Claude Code, Codex, OpenClaw, Hermes**, or anything that can hold a session, and use OpenHuman as the conductor for all of your agents and tools.

## What's next: RLMs

The direction we're building toward: **Rhai-backed language workflows**. These are agents that express orchestration as small programs in a sandboxed REPL, rather than a fixed graph, so control flow itself becomes something the model writes, inspects, and repairs. The graph engine, checkpointing, and trust model above are the substrate for it.

***

## Why this differentiates

| | Single-agent harnesses (Claude Code, OpenClaw, Hermes) | OpenHuman |
| --- | --- | --- |
| Execution model | One loop, one context | Compiled graphs, conditional routing, checkpoint/resume |
| Parallelism | Manual / plugin | Native sub-agent fleets, map-reduce fan-out, worker reuse |
| Automation | Scripts & cron | Visual, durable, approval-gated workflows |
| Always-on | None | Split-brain reflex + reasoning core, subconscious steering |
| Agent-to-agent | None | Signal-encrypted sessions, consent-based pairing, x402 payments |

## See also

* [Workflows](workflows.md) · [Subconscious Loop](subconscious.md) · [tiny.place Agent Economy](tinyplace.md)
* [Agent Harness](../developing/architecture/agent-harness.md): the developer deep-dive on graphs, breakers, journals.
* [Agent Coordination tools](native-tools/agent-coordination.md): the user-facing spawn/delegate surface.
