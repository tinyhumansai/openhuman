---
description: >-
  How an agent turn actually runs - the tool-call loop, sub-agent dispatch,
  archetypes, triage, hooks, and the cost/budget machinery around them.
icon: layer-group
---

# Agent Harness

The agent harness is the runtime that turns a user message (or a webhook fire, or a cron tick) into a complete, tool-using LLM interaction. It owns the tool-call loop, sub-agent dispatch, the trigger-triage pipeline, and the hook surface around them. It does **not** own provider HTTP transport, tool implementations, prompt-section assembly, or memory storage - those are separate domains the harness composes.

This page walks through what happens in one turn, then zooms in on each of the moving parts.

## The shape of a turn

Every turn - whether the user just typed a message, a Telegram webhook just fired, or a 9am cron just ticked - flows through the same lifecycle:

```
┌─ inbound ─────────────────────────────────────────────────────────┐
│ user message · channel inbound · webhook · cron · composio event │
└──────────────────────────┬────────────────────────────────────────┘
                           │
                           ▼  (external triggers only)
                ┌──────────────────────┐
                │   trigger triage     │  classify → drop / notify /
                │   (small local LLM)  │  spawn reactor / spawn orchestrator
                └──────────┬───────────┘
                           │
                           ▼
            ┌──────────────────────────────┐
            │      Agent::turn()           │
            │  1. resume transcript        │
            │  2. build system prompt*     │
            │  3. inject memory context    │
            │  4. enter tool-call loop ────┼──► provider call
            │  5. dispatch tool calls  ────┼──► tool exec / sub-agent spawn
            │  6. context guard / compact  │
            │  7. stop-hook check          │
            │  8. final assistant text     │
            └──────────┬───────────────────┘
                       │ async, after the user sees the reply
                       ▼
              ┌─────────────────┐
              │  post-turn      │  archivist · learning · cost log ·
              │  hooks          │  episodic memory indexing
              └─────────────────┘

* system prompt is built only on the first turn - subsequent
  turns reuse the rendered prompt verbatim so the inference
  backend's KV-cache prefix stays valid.
```

The rest of this page is the same diagram, expanded.

## Sessions and `Agent::turn`

A **session** is the live conversation an `Agent` instance is running. The `Agent` struct owns:

* The conversation history (system + user + assistant + tool messages).
* The provider client to call (model resolved by the [model router](../../features/model-routing/)).
* The tool registry visible to the model.
* A memory loader that hydrates relevant memories before each user message.
* Per-turn budgets - max tool iterations, max payload size, max USD cost.

`Agent::turn(user_message)` is the hot path. In one turn it:

1. **Resumes the session transcript** if this is a fresh process - re-loading the exact provider messages from disk so the inference backend's KV-cache prefix still hits.
2. **Builds the system prompt** (only on the first turn). This pulls in identity, soul, profile, memory, connected integrations, available tools, safety preamble - assembled by the prompt section builder.
3. **Injects memory context** for the new user message via the memory loader: relevant chunks from the [Memory Tree](../../features/obsidian-wiki/memory-tree.md), with citations attached so the UI can show provenance.
4. **Enters the tool-call loop** (next section).
5. **Spawns post-turn hooks** in the background - the user gets their answer before archivist / learning / cost logging finishes.

The system prompt is **not** rebuilt on subsequent turns. Even cosmetic byte changes invalidate the KV-cache prefix and force a full re-prefill, so dynamic per-turn context (memory recall, freshly-learned snippets) is appended as user-visible message content rather than spliced into the system prompt.

## The tool-call loop

Inside `Agent::turn`, the tool-call loop is the inner engine. It runs up to `max_tool_iterations` rounds (default 10):

```
loop {
    1. context guard      - if history is too big, microcompact / autocompact
    2. stop-hook check    - budget caps, max-iterations, custom kill switches
    3. provider call      - send messages + tool specs, stream the response
    4. parse response     - split assistant text from tool calls
    5. if no tool calls   - return final text
    6. execute tool calls - dispatch each one (next section)
    7. summarize oversize - route huge tool outputs through the summarizer agent
    8. append results     - push tool results into history, loop again
}
```

Every iteration emits a real-time `AgentProgress` event so the UI can render token-by-token streaming, "calling tool X" status, and per-iteration cost updates.

### Tool dispatch and tool-call dialects

Different LLMs speak different tool-calling dialects. The harness abstracts that with a `ToolDispatcher` trait, which has three concrete implementations:

* **Native** - providers with first-class tool-calling APIs (Anthropic, OpenAI). Tool calls come back as structured fields, not in the text body.
* **XML** - fallback for models that aren't natively trained for tool-calling but can follow instructions. Tools are wrapped in `<tool_call>{...}</tool_call>` tags in the assistant text.
* **P-Format** - a compact text format used by some smaller models.

The dispatcher is selected per provider, which keeps the loop itself dialect-agnostic. The same loop code drives Claude, GPT, Gemini, and a local Ollama model.

### Context management mid-loop

Long tool-calling chains can blow past the context window. Two layers handle that:

* **Tool-result budget** - every tool result is checked against a per-call byte budget. Anything over is hard-truncated with an explanatory marker so the model knows it didn't see the full output.
* **Microcompact / autocompact** - when total history is creeping toward the context window, the harness compacts older turns into summaries before the next provider call. The compacted history keeps the system prompt and the most recent turns intact (KV-cache stability) and rewrites the middle.

### Oversized tool results - the summarizer detour

Some tool calls return enormous payloads - a Composio action dumping 200 KB of JSON, a web scrape returning 50 KB of markdown, a `file_read` over a multi-thousand-line log. Hard-truncating mid-payload drops whatever happens to land past the cut.

When a tool result exceeds the summarizer's threshold, it gets routed through a dedicated `summarizer` sub-agent before entering the parent's history. The summarizer compresses the payload per an extraction contract that preserves identifiers and key facts, and the parent agent only sees the compressed summary. Hard truncation remains the backstop downstream when summarization fails or the payload is so absurdly large that paying for an LLM call on it makes no economic sense.

### Self-healing for missing commands

When the code-executor sub-agent runs a shell command and the runtime answers "command not found", a self-healing interceptor catches the error, spawns a `ToolMaker` sub-agent to write a polyfill script for the missing command, and retries the original call. There's a per-command attempt cap so a genuinely impossible command can't loop forever.

## Sub-agents - the orchestrator pattern

OpenHuman is **multi-agent**. The agent the user is chatting with is the **Orchestrator** - a senior, strategy-level agent that decides when to answer directly, when to use a direct tool, and when to spawn a specialist sub-agent.

### Why multi-agent

A single agent that knows everything also has a system prompt the size of a small book. Splitting work across specialists means:

* Each sub-agent gets a **narrow system prompt** with only the sections it needs (identity / memory / safety preamble can be stripped).
* Each sub-agent gets a **filtered tool registry** - the integrations agent doesn't need filesystem tools, the coder doesn't need the Composio catalog.
* Sub-agent histories never leak back to the parent - the parent sees one compact tool result, not the inner conversation.
* Cheaper models can do the leaf work. The orchestrator is on a strong reasoning model; a research sub-agent might be on a faster, cheaper one.

### The built-in archetypes

Each archetype lives under `agents/<name>/` with an `agent.toml` (metadata, tool scope, model hint) and a prompt:

| Archetype           | When the orchestrator picks it                                                          |
| ------------------- | --------------------------------------------------------------------------------------- |
| `orchestrator`      | The top-level agent. Never spawned by another orchestrator.                             |
| `planner`           | Multi-step decomposition - break a complex request into ordered sub-tasks.              |
| `researcher`        | Web/doc lookups, citation hunting.                                                      |
| `code_executor`     | Writing, running, and debugging code in the workspace.                                  |
| `critic`            | Code review, quality checks on another agent's output.                                  |
| `summarizer`        | Compressing oversized tool results (called by the harness, not usually the model).      |
| `archivist`         | Memory distillation - what to persist, what to forget.                                  |
| `tool_maker`        | Self-healing - writes polyfills for missing shell commands.                             |
| `tools_agent`       | Generic specialist for arbitrary tool-bound tasks.                                      |
| `integrations_agent`| Bound to a specific Composio toolkit (Gmail, GitHub, Slack…) for that toolkit's actions.|
| `trigger_triage`    | Classifies incoming external events into drop / notify / spawn-reactor / spawn-agent.   |
| `trigger_reactor`   | Lightweight reaction to a triaged trigger that doesn't need a full orchestrator turn.   |
| `morning_briefing`  | Curated daily digest run by cron.                                                       |
| `welcome` / `help`  | Onboarding flows.                                                                       |

Custom archetypes ship as TOML files under `$OPENHUMAN_WORKSPACE/agents/*.toml` (or `~/.openhuman/agents/*.toml` for user-global specialists). Custom definitions override built-ins on id collision.

### Running a sub-agent

When the orchestrator calls `spawn_subagent` (or one of the `delegate_*` convenience tools), the runner:

1. Reads the parent's execution context from a task-local - the parent's provider, sandbox mode, cancellation fence, transcript root.
2. Resolves the sub-agent's model - inherit from parent, follow a hint (`fast` / `reasoning` / `summarization`), or pin an exact model.
3. Filters the parent's tool registry per the definition's `tools`, `disallowed_tools`, and `skill_filter`. In `fork` mode, the parent's full registry is inherited verbatim.
4. Builds a narrow system prompt, omitting the sections the definition asks to strip.
5. Runs an inner tool-call loop using the same machinery as the parent.
6. Returns one compact text result. The intra-sub-agent history is never spliced back into the parent - the orchestrator sees a single tool result and moves on.

For tasks that don't need to block the orchestrator's turn, `spawn_worker_thread` runs the sub-agent in the background and the orchestrator continues immediately.

### Spawn hierarchy and tiers

Not every agent is allowed to spawn every other agent. The harness models a three-tier hierarchy that mirrors the cost / latency / depth-of-thought split between models:

```text
Chat        (fast, UX-focused — e.g. orchestrator on `chat` hint)
  │
  ├─► Worker      ◄─── fast path: one delegation, leaf does the work
  │
  └─► Reasoning   (slow, deep-thinking — e.g. planner on `reasoning` hint)
        │
        └─► Worker  ◄─── deep path: reasoning decomposes, workers execute
```

Each `AgentDefinition` carries an `agent_tier` field (`chat` / `reasoning` / `worker`, default `worker`). The contract:

| Tier         | May spawn         | Must NOT spawn               | Typical members                                          |
| ------------ | ----------------- | ---------------------------- | -------------------------------------------------------- |
| `chat`       | `reasoning`, `worker` | another `chat`               | `orchestrator`                                           |
| `reasoning`  | `worker`          | another `reasoning`, any `chat` | `planner` (today the canonical one)                     |
| `worker`     | nothing[^1]       | anything                     | researcher, code_executor, critic, archivist, tool_maker, integrations_agent, … |

[^1]: Skill-wildcard entries (`{ skills = "*" }`) are exempt because they collapse to a single `delegate_to_integrations_agent` tool whose target is a worker — they're a fan-out delegation surface, not a recursive spawn.

**Why the rules.**
- *Chat → chat is meaningless.* The chat tier exists for snappy UX. A chat agent spawning another chat agent just doubles TTFT and burns tokens without buying any new capability.
- *Reasoning → reasoning blows up depth.* The reasoning tier is expensive. Chains of reasoning agents tend to re-decompose the same problem and create runaway hierarchies.
- *Worker → anything mixes execution and orchestration.* Workers are leaves so the parent always sees one compact result, not a transcript of nested delegations.

**Enforcement.** Two layers:

1. **Loader-time (static).** [`agents::loader::validate_tier_hierarchy`](../../../src/openhuman/agent/agents/loader.rs) runs over the merged registry (built-ins + workspace TOMLs) and refuses to boot a registry that lists a same-tier or worker-with-subagents entry. Built-in archetypes are checked at compile-test time; user-shipped TOMLs are checked at workspace load.
2. **Runtime depth gate (dynamic, planned).** Independent of tier, the sub-agent runner *will* cap total spawn chain depth at `MAX_SPAWN_DEPTH = 3` via a task-local counter incremented across `run_subagent`, surfaced as a new `SpawnDepthExceeded` agent error. This makes a user-shipped TOML that drops the tier annotation still unable to recurse past three hops. Tracked as the follow-up to the gap noted in `harness_gap_tests.rs`.

> **Status:** the loader-time tier check and `agent_tier` field are live (this section). The runtime depth-counter task-local is *not yet implemented* — it is the planned defence-in-depth layer described above. Until it lands, depth is bounded only by the static loader contract plus the prompt-level rules in the orchestrator and planner agents.

### Toolkit-specific specialists

For Composio toolkits with hundreds of actions (GitHub alone has 500+), loading every action into the sub-agent's tool set balloons prompt size. The harness ranks the toolkit's actions against the parent-refined task prompt with a cheap CPU-only filter (verb detection, token overlap, verb-alignment boost) and only loads the top-ranked subset into the sub-agent. No model call, pure heuristic - fast and explainable.

## Triage - handling external triggers

When a webhook fires, a cron ticks, or a Composio event arrives, the system can't just hand it straight to the orchestrator. Most triggers are noise; some warrant a notification; only a few deserve a full agent turn. The **trigger-triage pipeline** is the gate.

```
TriggerEnvelope ──► run_triage ──► TriageDecision ──► apply_decision
                       │                                     │
                       │                                     ├─► drop (noise)
                       │                                     ├─► notify only
                       │                                     ├─► spawn trigger_reactor
                       │                                     └─► spawn orchestrator
                       │
                       └── small local LLM (with cloud-LLM retry fallback)
```

The evaluator is intentionally cheap - a small local model where available, falling back to a remote model on retry. The decision is cached so identical triggers don't re-classify. Only triggers that escalate to "spawn orchestrator" go through the full `Agent::turn` machinery.

## Hooks - observability and policy levers

Two hook surfaces wrap the loop, on opposite ends:

### Stop hooks (mid-turn)

Stop hooks fire **between** iterations of the tool-call loop. They're the policy lever for budget caps, rate limits, and custom kill switches. Built-in hooks:

* **Budget stop hook** - caps cumulative turn cost in USD using the per-iteration cost accumulator.
* **Max-iterations stop hook** - caps iteration count from outside the agent's persistent config.

A hook returning `Stop` aborts the loop with a clear reason the caller can surface to the user. Stop hooks are distinct from interrupts (next section): they're policy-driven, not user-driven.

### Post-turn hooks

Post-turn hooks fire **after** the turn completes, in the background. They get a `TurnContext` snapshot - user message, assistant response, every tool call with arguments and outcome, total wall-clock, iteration count, session ID. Built-in consumers:

* **Archivist** - distills which facts from the turn are worth persisting to long-term memory.
* **Learning** - feeds reflection, tool-tracker, and user-profile updates.
* **Cost log** - final per-turn cost line.
* **Episodic memory indexing** - writes the turn into the [Memory Tree](../../features/obsidian-wiki/memory-tree.md) as a chunk for future recall.

Hooks run via `tokio::spawn`, so the user gets their answer before any of them finish.

## Interrupts - graceful cancellation

An `InterruptFence` is checked at fixed safe points in the loop - before each tool execution, before each sub-agent spawn, before each provider call. When the user hits Ctrl+C or sends `/stop`:

* The fence flips.
* Every running sub-agent sees the same flag (it's shared via `Arc`) and bails at its next checkpoint.
* In-flight provider streams are dropped.
* The archivist still fires with whatever partial context exists, so the conversation isn't lost.

Interrupts are user-driven; stop hooks are policy-driven. They share the underlying "halt the loop cleanly" plumbing but enter from different sides.

## Cost accounting

Every provider response carries a `UsageInfo` block - input tokens, output tokens, cached input tokens, and an authoritative `charged_amount_usd` populated by the OpenHuman backend. `TurnCost` sums those across every provider call inside one turn so the harness can:

* Emit per-iteration cost telemetry over the progress channel.
* Feed the budget stop hook so a runaway turn cuts itself off mid-loop.
* Log accurate end-of-turn cost lines.

When the backend doesn't surface a charged amount (older builds, providers that don't bill through it), a small per-tier rate table provides a token-rate floor estimate. Direct cost from the backend always wins when available.

## Fork context - KV-cache reuse across the harness

The harness uses a task-local `ParentExecutionContext` to thread parent state into sub-agents without exploding every function signature. The same pattern carries the current sandbox mode, the interrupt fence, and the stop-hook list. Sub-agents that inherit the parent's provider, model, and prompt prefix get to **share the parent's KV-cache prefix** on the inference backend - measurably cheaper than re-prefilling from scratch.

## Self-healing recap

A few small adaptive systems sit on top of the main loop:

* **Self-healing for missing commands** - `ToolMaker` polyfills, capped retry attempts.
* **Payload summarizer circuit-breaker** - three consecutive sub-agent failures in a session disable summarization, falling back to truncation.
* **Triage local-vs-remote retry** - local LLM first; remote fallback on parse failure.

None of these change the loop's shape - they just make the common failure modes recoverable without the user having to intervene.

## Where to look in the code

The harness lives entirely under `src/openhuman/agent/`. The README in that directory enumerates the public surface; the most load-bearing files are:

| File / dir                    | What lives there                                                  |
| ----------------------------- | ----------------------------------------------------------------- |
| `harness/session/turn.rs`     | `Agent::turn` - the lifecycle described above.                    |
| `harness/tool_loop.rs`        | The inner tool-call loop.                                         |
| `harness/subagent_runner/`    | `run_subagent`, fork-mode, oversized-result handoff.              |
| `harness/definition.rs`       | `AgentDefinition` - what an archetype declares.                   |
| `harness/tool_filter.rs`      | Toolkit-action ranking for integrations sub-agents.               |
| `harness/payload_summarizer.rs` | Oversized-tool-result detour.                                   |
| `harness/self_healing.rs`     | Missing-command interceptor.                                      |
| `harness/interrupt.rs`        | The cancellation fence.                                           |
| `dispatcher.rs`               | Tool-call dialect abstraction.                                    |
| `triage/`                     | External-trigger classification + escalation.                     |
| `agents/`                     | Built-in archetypes - one subdirectory per agent.                 |
| `hooks.rs` / `stop_hooks.rs`  | Post-turn and mid-turn hook surfaces.                             |
| `cost.rs`                     | Per-turn USD/token accounting.                                    |
| `progress.rs`                 | Real-time progress events to the UI.                              |
| `memory_loader.rs`            | Memory-Tree context injection per user message.                   |

## See also

* [Architecture overview](README.md) - where the harness sits in the bigger picture.
* [Memory Tree](../../features/obsidian-wiki/memory-tree.md) - what the memory loader reads from and post-turn hooks write to.
* [Automatic Model Routing](../../features/model-routing/) - how `model: "hint:reasoning"` resolves to a concrete provider+model.
* [Native Tools - Agent Coordination](../../features/native-tools/agent-coordination.md) - the user-facing surface for `spawn_subagent`, `delegate_*`, `todo_write`.
