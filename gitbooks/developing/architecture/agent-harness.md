---
description: >-
  How an agent turn actually runs - the tool-call loop, sub-agent dispatch,
  archetypes, triage, hooks, and the cost/budget machinery around them.
icon: layer-group
---

# Agent Harness

> **Status (issue #4249, tinyagents migration):** the agent turn no longer runs
> on the in-tree `run_turn_engine` loop. **All three entry points (`Agent::turn`,
> the channel/CLI bus path, and `run_subagent`) now drive every turn through the
> published [`tinyagents`](https://crates.io/crates/tinyagents) 1.3 agent-loop
> harness** via the adapter seam in [`src/openhuman/tinyagents/`](../../../src/openhuman/tinyagents/)
> (`run_turn_via_tinyagents_shared`). The legacy `run_turn_engine`, the three
> hand-rolled loops, `turn_engine_adapter`, and the custom `agent_graph/` engine
> described later in this page have been **removed**; the surviving shared seams
> (`CheckpointStrategy`, `TurnProgress`) live in `agent/harness/engine/`. The dead
> `token_budget.rs` (context trimming is now `MessageTrimMiddleware`) and the
> vestigial `interrupt.rs` fence (cancellation is the tinyagents steering channel)
> are gone; policy **stop hooks** (budget / thread-goal / iteration caps) now fire
> through a `StopHookMiddleware` ([`tinyagents/stop_hooks.rs`](../../../src/openhuman/tinyagents/stop_hooks.rs))
> that pauses the run on the first stop vote, and the channel route forwards live
> `AgentProgress` like the chat route.
>
> Multi-agent **orchestration** is expressed on tinyagents' **graph layer** via
> `graph::parallel::map_reduce`, the `spawn_parallel_graph` scaffold, and the
> shared `graph::orchestration` `TaskStore` lifecycle primitives re-exported from
> [`tinyagents/orchestration.rs`](../../../src/openhuman/tinyagents/orchestration.rs):
>
> - the model-council member fan-out runs on a real `StateGraph`
>   ([`model_council/graph.rs`](../../../src/openhuman/model_council/graph.rs));
> - [`tinyagents/delegation.rs`](../../../src/openhuman/tinyagents/delegation.rs)
>   is a `plan → execute ⇄ review → finalize` `CompiledGraph` (conditional routing,
>   `RecursionPolicy`, durable `FileCheckpointer`, `CancellationToken`, `GraphTracingSink`);
> - the **workflow phase engine** fans each phase's agents out on the graph
>   (`with_max_concurrency`), keeping the durable `WorkflowRun` ledger as the resume
>   source of truth;
> - `spawn_parallel_agents` runs its fan-out through `spawn_parallel_graph` +
>   `graph::parallel::map_reduce`;
> - the **agent-teams** member runtime is a conditional-routing graph
>   (`execute → complete | fail → done`, [`agent_teams/graph.rs`](../../../src/openhuman/agent_orchestration/agent_teams/graph.rs));
> - the **detached-sub-agent** registry is backed by a typed `TaskStore` lifecycle
>   ledger (Pending → Running → Completed/Failed/Cancelled).
>
> The sections below describing a bespoke `agent_graph/` module + per-agent
> `GraphBlueprint`s are **historical** (the pre-migration design) and are retained
> only for context.

## TinyAgents crate: features & compatibility

OpenHuman pins `tinyagents = { version = "1.5.0", features = ["sqlite", "repl"] }` (see [`Cargo.toml`](../../../Cargo.toml)). The rationale, so future upgrades don't silently regress it:

- **OpenHuman-owned providers only.** We do **not** enable any bundled provider feature. OpenHuman owns provider transport, credentials, OAuth, and billing classification, so the live model is always OpenHuman's `Provider` wrapped as [`ProviderModel`](../../../src/openhuman/tinyagents/model.rs), never an SDK-owned provider client. The `ChatModel` adapter is the seam that replaces feature-gated SDK providers.
- **`sqlite` feature enabled with one native sqlite chain.** OpenHuman's root and Tauri Cargo worlds pin `rusqlite = "=0.40.0"` and patch `rusqlite` / `libsqlite3-sys` locally to avoid the upstream `cfg_select!` build break on the current toolchain. Both worlds resolve to a single `libsqlite3-sys v0.38.0` chain. Durable graph checkpoints still run through [`SqlRunLedgerCheckpointer`](../../../src/openhuman/tinyagents/checkpoint.rs) until the migration re-points those rows to the crate checkpointer.
- **WhatsApp Web storage bridge.** `whatsapp-rust`'s Diesel-backed `sqlite-storage` feature links sqlite separately from rusqlite 0.40, so the optional `whatsapp-web` feature currently builds against `wacore::store::InMemoryBackend` and logs that sessions are not durable. A rusqlite-backed durable WhatsApp store is required before treating Web sessions as persistent again.
- **`repl` feature enabled for language workflows; `.rag` expressive language unused.** OpenHuman still drives *graphs* from Rust (`GraphBuilder`), not the declarative `.rag` language. But the `repl` feature (the imperative Rhai `.ragsh` session runtime) is enabled to power the `rhai_workflows` language-workflow tool ([`openhuman::rhai_workflows`](../../../src/openhuman/rhai_workflows/README.md), see "Language workflows (Rhai)" below).
- **Adapter map (feature-gated SDK piece → OpenHuman replacement):** provider clients → `ProviderModel`; crate SQLite checkpointer rows not yet adopted → `SqlRunLedgerCheckpointer`; task/status stores not yet controller-canonical → OpenHuman SQL/JSON run ledgers (`running_subagents`, `workflow_runs`, `agent_teams`, `command_center`). The generic harness/graph/middleware/event primitives are used as-is.

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
* Local action budget - a rolling hourly cap for side-effecting tool actions, read from `config.autonomy.max_actions_per_hour`.

`Agent::turn(user_message)` is the hot path. In one turn it:

1. **Resumes the session transcript** if this is a fresh process - re-loading the exact provider messages from disk so the inference backend's KV-cache prefix still hits.
2. **Builds the system prompt** (only on the first turn). This pulls in identity, soul, profile, memory, connected integrations, available tools, safety preamble - assembled by the prompt section builder.
3. **Injects memory context** for the new user message via the memory loader: relevant chunks from the [Memory Tree](../../features/obsidian-wiki/memory-tree.md), with citations attached so the UI can show provenance.
4. **Enters the tool-call loop** (next section).
5. **Spawns post-turn hooks** in the background - the user gets their answer before archivist / learning / cost logging finishes.

The system prompt is **not** rebuilt on subsequent turns. Even cosmetic byte changes invalidate the KV-cache prefix and force a full re-prefill, so dynamic per-turn context (memory recall, freshly-learned snippets) is appended as user-visible message content rather than spliced into the system prompt.

## The tool-call loop

Inside `Agent::turn`, the tool-call loop is the inner engine. Since issue #4249 it is the published **tinyagents** crate's `AgentHarness` loop, assembled per turn by `run_turn_via_tinyagents_shared` ([`src/openhuman/tinyagents/mod.rs`](../../../src/openhuman/tinyagents/mod.rs)). It runs up to `max_tool_iterations` rounds (default 10):

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

**One engine, three entry points.** The loop lives in one place (the tinyagents `AgentHarness`, entered via `run_turn_via_tinyagents_shared` in `src/openhuman/tinyagents/mod.rs`) and every caller drives it: the chat turn (`harness/session/turn/core.rs` → `session/turn/graph.rs`), the channel/CLI bus turn (`harness/graph.rs`), and spawned sub-agents (`harness/subagent_runner/ops/graph.rs`). What varies per caller is supplied through the adapter seam: OpenHuman's provider wrapped as a `ChatModel` (`tinyagents/model.rs`), tools wrapped as tinyagents `Tool`s (`tinyagents/tools.rs`), an event bridge that projects harness `AgentEvent`s into `AgentProgress` + cost telemetry (`tinyagents/observability.rs`), `RunPolicy::unknown_tool` for hallucinated tool recovery, and a named middleware stack (`tinyagents/middleware.rs`) carrying the OpenHuman cross-cuts: approval/security gating (`ApprovalSecurityMiddleware`), tool policy and CLI/RPC-only denial (`ToolPolicyMiddleware`, `CliRpcOnlyMiddleware`), malformed-argument recovery (`ArgRecoveryMiddleware`), cost budget pre-checks (`CostBudgetMiddleware`), the repeated-tool-failure circuit breaker (`RepeatedToolFailureMiddleware`), and context trimming/compression. Policy stop hooks fire through `StopHookMiddleware` (`tinyagents/stop_hooks.rs`). The surviving OpenHuman-owned seams, `CheckpointStrategy` (error vs. summarize at the model-call cap) and `TurnProgress`, live in `harness/engine/`. Because all three entry points assemble the same harness, they can't drift.

### Tool dispatch and tool-call dialects

Live turns speak **native tool calling**: the tinyagents harness sends structured tool specs through the `ChatModel` adapter and gets structured tool calls back, for every provider (Claude, GPT, Gemini, local Ollama alike).

The older `ToolDispatcher` trait (`src/openhuman/agent/dispatcher.rs`) with its three dialects still exists, but as a **transcript-compatibility layer**, not a live routing choice:

* **Native** - structured tool-call fields, the shape live turns produce today.
* **XML** - `<tool_call>{...}</tool_call>` tags in assistant text, produced by older sessions.
* **P-Format** - a compact text format some earlier local models used.

Persisted session histories can contain suffixes in any of the three shapes, so the session shell keeps the dispatcher around to parse and replay them faithfully when a transcript is resumed.

### Context management mid-loop

Long tool-calling chains can blow past the context window. Two layers handle that:

* **Tool-result budget** - every tool result is checked against a per-call byte budget, enforced as tinyagents tool middleware. Anything over is hard-truncated with an explanatory marker so the model knows it didn't see the full output.
* **Microcompact / autocompact** - when total history is creeping toward the context window, tinyagents middleware (message trimming + the compression hooks in `tinyagents/summarize.rs`) compacts older turns into summaries before the next provider call. The compacted history keeps the system prompt and the most recent turns intact (KV-cache stability) and rewrites the middle.

### Oversized tool results - the summarizer detour

Some tool calls return enormous payloads - a Composio action dumping 200 KB of JSON, a web scrape returning 50 KB of markdown, a `file_read` over a multi-thousand-line log. Hard-truncating mid-payload drops whatever happens to land past the cut.

When a tool result exceeds the summarizer's threshold, it gets routed through a dedicated `summarizer` sub-agent before entering the parent's history. The summarizer compresses the payload per an extraction contract that preserves identifiers and key facts, and the parent agent only sees the compressed summary. Hard truncation remains the backstop downstream when summarization fails or the payload is so absurdly large that paying for an LLM call on it makes no economic sense.

### TokenJuice - content-aware tool-output compaction (Stage 1a)

Before a fresh tool result enters history (and ahead of the byte-budget backstop), it passes through the **TokenJuice content router** in the vendored TinyJuice crate (`vendor/tinyjuice`), with OpenHuman adapters in `src/openhuman/tokenjuice/`. Inspired by Headroom, the router *detects the content kind* (JSON, code, log, search, diff, HTML, plain text) from the bytes and/or a hint derived from the tool name and arguments, then dispatches to a specialised compressor:

* **JSON** → SmartCrusher: array-of-objects → table (each key once), preserving rows that carry errors or numeric outliers.
* **Code** → tree-sitter (Rust/TS/JS/Python) signature keeper that collapses function bodies; brace-depth heuristic fallback.
* **Log** → the 100-rule engine for *command* output (git/cargo/npm/…), signal-based keep-failures otherwise.
* **Search** → relevance-ranked top-K matches per file with a `+N more` tally.
* **Diff** → keep changed hunks, collapse unchanged context, summarise lockfile hunks.
* **HTML** → strip markup to readable text.
* **Plain text** → the opt-in Python/ML "Kompress" compressor (ModernBERT), or pass-through.

Every lossy compression offloads the original to the **CCR (Compress-Cache-Retrieve)** store behind a `⟦tj:<hash>⟧` marker, so compaction is effectively lossless: the agent calls `tokenjuice_retrieve` (token + optional byte/line range) to fetch the full original on demand. The same engine is exposed as a universal `compress_content(content, hint, opts)` for any large payload (file reads, web fetches), and as read-only `tokenjuice.*` debug RPCs. Configured via the `[tokenjuice]` block / `OPENHUMAN_TOKENJUICE_*` env. Agent definitions can override tool-result compression with `tokenjuice_compression = "auto" | "full" | "light" | "off"`; `auto` resolves coding-model agents (`[model] hint = "coding"`) to `light`, which disables CCR-backed lossy compression so coding agents keep raw build/test/diff/search text unless a reduction is truly lossless. Other agents default to `full`. The ML (Kompress) path runs as a `kompress` backend of the shared [`runtime_python_server`](../../../src/openhuman/runtime_python_server/) (torch + ModernBERT pip-installed at runtime), gated by the `ml_compression_enabled` flag and degrading gracefully to a native compressor when the Python runtime is unavailable.

### The `tool_maker` archetype

The `tool_maker` archetype exists for writing polyfill scripts and small helper tools when a capability is missing. It is spawned explicitly (by the orchestrator or another agent) like any other sub-agent. The old automatic "command not found → spawn ToolMaker → retry" interceptor was removed with the in-house loop; there is no implicit self-healing retry on shell failures today.

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

### Running a reusable sub-agent

When the orchestrator calls `spawn_subagent`, the default contract is durable and asynchronous. The tool builds a deterministic compatibility selector from the parent session/thread, agent id, toolkit scope, model override, sandbox mode, action root, and normalized task key/title. It then checks `agent_orchestration::subagent_sessions` before spawning:

* If a compatible worker is already running, the instruction is injected through its `RunQueue` and the parent gets a quick `subagent_session_id` / `task_id` reference.
* If a compatible worker is idle or paused with reusable history, the harness starts a new transient run for the same durable `subagent_session_id` and passes the saved child history through `SubagentRunOptions.initial_history`, with the new instruction appended as a user-visible follow-up.
* If the shape is incompatible, the worker was closed, `fresh: true` was passed, or no session exists, the harness creates a new durable session and worker thread.

The child run itself still uses the same runner:

1. Reads the parent's execution context from a task-local - the parent's provider, sandbox mode, cancellation fence, transcript root.
2. Resolves the sub-agent's model - inline `model` override first, then config-level pins (`[orchestrator].model`, `[teams.*].lead_model`, `[teams.*].agent_model`), then the archetype hint or inherited parent model.
3. Filters the parent's tool registry per the definition's `tools`, `disallowed_tools`, and `skill_filter`. In `fork` mode, the parent's full registry is inherited verbatim.
4. Builds a narrow system prompt, omitting the sections the definition asks to strip.
5. Runs an inner tool-call loop using the same machinery as the parent.
6. Persists the child history and worker thread pointer under the durable `subagent_session_id` so later turns can resume or inspect it.

`wait_subagent` and `steer_subagent` accept either the durable `subagent_session_id` or the transient `task_id`; durable ids are preferred across turns. `list_subagents` shows reusable children for the current parent thread, and `close_subagent` marks a worker non-reusable and cancels it if it is still running. Inline blocking is explicit via `blocking: true`; it is no longer the default.

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

[^1]: Skill-wildcard entries (`{ skills = "*" }`) are exempt because they collapse to a single `delegate_to_integrations_agent` tool whose target is a worker; they're a fan-out delegation surface, not a recursive spawn.

**Why the rules.**
- *Chat → chat is meaningless.* The chat tier exists for snappy UX. A chat agent spawning another chat agent just doubles TTFT and burns tokens without buying any new capability.
- *Reasoning → reasoning blows up depth.* The reasoning tier is expensive. Chains of reasoning agents tend to re-decompose the same problem and create runaway hierarchies.
- *Worker → anything mixes execution and orchestration.* Workers are leaves so the parent always sees one compact result, not a transcript of nested delegations.

**Enforcement.** Two layers:

1. **Loader-time (static).** [`agents::loader::validate_tier_hierarchy`](../../../src/openhuman/agent/agents/loader.rs) runs over the merged registry (built-ins + workspace TOMLs) and refuses to boot a registry that lists a same-tier or worker-with-subagents entry. Built-in archetypes are checked at compile-test time; user-shipped TOMLs are checked at workspace load.
2. **Runtime depth gate (dynamic).** Independent of tier, the sub-agent runner caps total spawn chain depth at `MAX_SPAWN_DEPTH = 3` via a task-local counter incremented across `run_subagent`, surfaced as a `SpawnDepthExceeded` agent error. This makes a user-shipped TOML that drops the tier annotation still unable to recurse past three hops.

> **Status:** the loader-time tier check, `agent_tier` field, and runtime depth-counter task-local are live. Depth is bounded by both the static loader contract and the runtime `MAX_SPAWN_DEPTH = 3` guard.

### Toolkit-specific specialists

For Composio toolkits with hundreds of actions (GitHub alone has 500+), loading every action into the sub-agent's tool set balloons prompt size. The harness ranks the toolkit's actions against the parent-refined task prompt with a cheap CPU-only filter (verb detection, token overlap, verb-alignment boost) and only loads the top-ranked subset into the sub-agent. No model call, pure heuristic - fast and explainable.

## Language workflows (Rhai)

The fixed delegation primitives (`spawn_subagent`, `spawn_parallel_agents`, `run_workflow`) can't express *ad-hoc control flow* — "spawn N readers, dedupe their findings, verify each survivor with 3 refuters, loop until dry". The **`rhai_workflows` tool** closes that gap: it exposes TinyAgents' Rhai-backed `.ragsh` REPL (the `repl` cargo feature) so the orchestrator can author and run its own workflow scripts.

**One tool call = one `eval_cell`.** The orchestrator's normal tool-call loop *is* the CodeAct driver loop: the model writes a Rhai cell, the cell runs against a persistent per-session namespace (top-level `let` bindings survive into the next cell via an optional `session_id`), and the structured result flows back as the tool result. Scripts reach the host only through capability functions — `tool_call`, `agent_query`, `model_query`, their `*_batched` fan-out variants, `emit`, and `answer`.

The domain lives in [`src/openhuman/rhai_workflows/`](../../../src/openhuman/rhai_workflows/README.md):

- **`policy.rs`** maps the autonomy tier + `tool_timeout` clamps onto a `tinyagents::ReplPolicy` (always bounded, never unbounded; `readonly` refused; `full` may raise call-count limits to a hard 2× ceiling).
- **`bridge.rs`** builds the `CapabilityRegistry`: the parent's visible tools (each re-wrapped so the **approval gate runs in the bridge** — it is *not* on the repl path, which bypasses the harness `wrap_tool` middleware), the turn's provider model, and a sub-agent capability per `allowed_subagent_ids`. Recursion/duplication hazards (`rhai`, legacy `rlm`, `spawn_*`, workflow tools, `CliRpcOnly`-scoped tools) are excluded. Because `eval_cell` runs on `spawn_blocking` + `block_on`, the `agent_query` adapter re-installs the `PARENT_CONTEXT` task-local that `run_subagent` resolves.
- **`sessions.rs`** is a bounded (LRU + idle-TTL) manager of persistent sessions, one cell at a time (a concurrent call on a busy session returns a typed "busy" error).
- **`ops.rs`** runs the cell on `spawn_blocking` under a layered time bound (rhai `on_progress` deadline → `bridge_block_on` timer race → outer `tokio::timeout` backstop → harness `ToolTimeout`), wires the run-cancellation token to a fresh per-cell `ReplCancelFlag`, and maps every failure mode to a model-consumable result.

The tool is registered for the orchestrator on `supervised`/`full` tiers only, behind the `OPENHUMAN_RHAI_WORKFLOWS=0` kill switch; `OPENHUMAN_RHAI=0` and `OPENHUMAN_RLM=0` remain legacy aliases. The TinyAgents-side host-embedding support (external cancellation, live capability events) landed in that crate's `repl` feature.

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
* **Action budget policy** - `SecurityPolicy` enforces `config.autonomy.max_actions_per_hour` for side-effecting tool operations. Users can tune it in Settings -> Advanced -> Agent autonomy, or operators can override it with `OPENHUMAN_MAX_ACTIONS_PER_HOUR`.

A hook returning `Stop` aborts the loop with a clear reason the caller can surface to the user. Stop hooks are distinct from interrupts (next section): they're policy-driven, not user-driven.

### Post-turn hooks

Post-turn hooks fire **after** the turn completes, in the background. They get a `TurnContext` snapshot - user message, assistant response, every tool call with arguments and outcome, total wall-clock, iteration count, session ID. Built-in consumers:

* **Archivist** - distills which facts from the turn are worth persisting to long-term memory.
* **Learning** - feeds reflection, tool-tracker, and user-profile updates.
* **Cost log** - final per-turn cost line.
* **Episodic memory indexing** - writes the turn into the [Memory Tree](../../features/obsidian-wiki/memory-tree.md) as a chunk for future recall.

Hooks run via `tokio::spawn`, so the user gets their answer before any of them finish.

## Interrupts - graceful cancellation

Cancellation is the tinyagents **steering channel**. The old in-house `InterruptFence` (`harness/interrupt.rs`) is gone; when the user hits Ctrl+C or sends `/stop`, the runner forwards the request into the harness's steering/cancellation seam, which stops the loop at the same safe points the fence used to guard - before each tool execution, before each sub-agent spawn, before each provider call:

* Every running sub-agent shares the cancellation scope and bails at its next checkpoint.
* In-flight provider streams are dropped.
* The archivist still fires with whatever partial context exists, so the conversation isn't lost.

Interrupts are user-driven; stop hooks are policy-driven. Both enter the same harness pause/stop plumbing, but from different sides.

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

* **Payload summarizer circuit-breaker** - three consecutive sub-agent failures in a session disable summarization, falling back to truncation.
* **Triage local-vs-remote retry** - local LLM first; remote fallback on parse failure.
* **Unknown-tool and malformed-argument recovery** - middleware rewrites an invalid model tool call into a recoverable result instead of aborting the run.

None of these change the loop's shape - they just make the common failure modes recoverable without the user having to intervene.

## Where to look in the code

The harness shell lives under `src/openhuman/agent/`, with the tinyagents adapter seam in `src/openhuman/tinyagents/` and archetype definitions in `src/openhuman/agent_registry/`. The README in `src/openhuman/agent/` enumerates the public surface; the most load-bearing files (paths relative to `src/openhuman/agent/` unless prefixed) are:

| File / dir                    | What lives there                                                  |
| ----------------------------- | ----------------------------------------------------------------- |
| `harness/session/turn/core.rs` | `Agent::turn` - the lifecycle described above; routes into the tinyagents runner via `session/turn/graph.rs`. |
| `../tinyagents/mod.rs`        | `run_turn_via_tinyagents_shared` - the shared tinyagents harness assembly (the live loop). |
| `../tinyagents/middleware.rs` | The named OpenHuman middleware stack (approval/security, tool policy, recovery, budgets, circuit breaker). |
| `harness/graph.rs`            | The channel/CLI bus turn route into the tinyagents runner.        |
| `harness/subagent_runner/`    | `run_subagent`, history replay, fork-mode, oversized-result handoff; `ops/graph.rs` is its tinyagents route. |
| `agent_orchestration/subagent_sessions/` | Durable reusable sub-agent identity, compatibility matching, persisted status/history. |
| `harness/definition.rs`       | `AgentDefinition` - what an archetype declares.                   |
| `harness/tool_filter.rs`      | Toolkit-action ranking for integrations sub-agents.               |
| `../tinyagents/payload_summarizer.rs` | Oversized-tool-result detour.                             |
| `harness/engine/`             | Surviving OpenHuman seams: `CheckpointStrategy`, `TurnProgress`.  |
| `dispatcher.rs`               | Tool-call dialect abstraction (persisted-transcript compatibility). |
| `triage/`                     | External-trigger classification + escalation.                     |
| `../agent_registry/agents/`   | Built-in archetypes - one subdirectory per agent.                 |
| `hooks.rs` / `stop_hooks.rs`  | Post-turn and mid-turn hook surfaces.                             |
| `cost.rs`                     | Per-turn USD/token accounting.                                    |
| `progress.rs`                 | Real-time progress events to the UI.                              |
| `memory_loader.rs`            | Memory-Tree context injection per user message.                   |

## Agent state graphs (`agent_graph`): HISTORICAL (removed)

> **⚠️ This section describes a design that was never shipped and has been removed.**
> The bespoke `src/openhuman/agent_graph/` engine, `GraphBlueprint`, and the
> `SqliteCheckpointer` described below **do not exist**. The live system runs on
> the published **tinyagents** crate; see the status banner at the top of this
> page and "Agent engine + orchestration on tinyagents (live)" below. Graphs are
> built with `tinyagents::graph::GraphBuilder` (`model_council/graph.rs`,
> `agent_orchestration/*/graph.rs`, `tinyagents/delegation.rs`), durable
> checkpoints use `SqlRunLedgerCheckpointer`, and per-agent graph selection is
> `AgentGraph` (`agent/harness/agent_graph.rs`) with each agent's
> `agent_registry/agents/<id>/graph.rs`. The text below is retained only as
> pre-migration design history.

Alongside the linear tool-call loop, the harness ships a **LangGraph-style state-machine engine** under [`src/openhuman/agent_graph/`](../../../src/openhuman/agent_graph/) (issue #4249). Where the loop is an implicit "prompt → tool → result → next prompt" cycle, a graph models agent execution as an explicit directed graph of **nodes** (states) and **edges** (transitions), with typed working state that survives across transitions, parallel branches, and checkpoints.

```
StateGraph::new(name)
  .add_node(id, node)            // a unit of work: async fn(State) -> (State, Command)
  .add_edge(from, to)            // static transition
  .add_conditional_edges(...)    // route by inspecting state
  .add_fork(from, [a, b])        // fan out in parallel; merge via State::merge
  .set_entry_point(id) / .set_finish_point(id)
  .compile()? -> CompiledGraph   // validated; .invoke(state) / .resume_with(...)
```

| Subfolder         | Role                                                                                              |
| ----------------- | ------------------------------------------------------------------------------------------------- |
| `graph/`          | The engine: `GraphState` (merge reducer), `Node` trait, builder + `compile()` validation, Pregel super-step `executor` with cycle / cancel / step-cap guards, `invoke`/`resume`. |
| `checkpoint/`     | `Checkpointer` trait (type-erased JSON state) → `InMemoryCheckpointer` (tests) + `SqliteCheckpointer` at `{workspace}/.openhuman/agent_graph/checkpoints.db`. Durable pause/resume. |
| `hitl/`           | Human-in-the-loop: `approval`/`clarification` interrupt builders + `ApplyResume` (folds the human's answer into state on resume). A node returns `Command::Interrupt` to pause. |
| `observability/`  | `EventBusSink` (a `ProgressSink`) emits `tracing` spans + publishes the `GraphRun*`/`GraphNode*` `DomainEvent` family (new `agent_graph` event domain). |
| `summarization/`  | Node-boundary wrapper over `context::summarize_chat_history`.                                      |
| `memory/`         | Pre-node wrapper over `DefaultMemoryLoader::load_context`.                                         |
| `definitions/`    | Built-in graphs over a shared `ProductState`: `canonical_turn` (the agent turn as a `dispatch → parse → stop_check → tools → compact → loop / finalize` graph) and `plan_execute_review` (composes the `planner` + `code_executor` archetypes around a HITL review gate), plus a deterministic `demo_review` twin for tests. A registry (`list_definitions`/`build_definition`) + `runner` (`run_graph`/`resume_graph`) persist runs to the checkpointer and emit bus events. |
| `blueprint/`      | The per-agent chain type. Every built-in agent declares its LangGraph-compatible chain in a `graph.rs` next to `prompt.rs` (`pub fn graph() -> GraphBlueprint`), wired into `BuiltinAgent.graph_fn`. `GraphBlueprint` is serializable (typed `NodeKind`/`EdgeSpec`), structurally validated, and `compile()`s to a real `CompiledGraph`. Reusable shapes: `canonical_turn` (most agents), `single_shot`, `orchestrator`, `plan_execute_review`. Inspect via `openhuman.agent_graph_{agent_list,agent_graph}`. |

### Per-agent graphs (`graph.rs`)

Each agent folder under `src/openhuman/agent_registry/agents/<name>/` (and the four agents that live in their own domains) now contains, alongside `agent.toml` + `prompt.rs`:

- **`graph.rs`**: `pub fn graph() -> GraphBlueprint`. `prompt.rs` defines what the agent *says*; `graph.rs` defines how it *runs*, meaning its node/edge chain. A loader test asserts **every** built-in agent's chain validates and compiles, so a malformed chain fails CI.

Most agents reuse `blueprint::canonical_turn(id)` (the standard tool-calling loop); one-pass agents use `single_shot`, the orchestrator uses the delegation chain, and the planner uses `plan_execute_review`.

**RPC surface** (`schemas.rs` + `ops.rs`, registered in `src/core/all.rs`): `openhuman.agent_graph_definition_list`, `_run`, `_run_list`, `_run_get`, `_checkpoint_list`, `_resume`.

> **Status (issue #4249, superseded by the published `tinyagents` crate):** the in-house `agent_graph` engine described in this section **no longer exists**. openhuman's agent engine + orchestration now run on the published [`tinyagents`](https://crates.io/crates/tinyagents) **1.3** crate (the same LangGraph-style harness + durable graph runtime), via the adapter seam in `src/openhuman/tinyagents/`. The sections above are retained as design history; the subsection below describes the live architecture.

## Agent engine + orchestration on tinyagents (live)

Every agent turn (chat via `harness/session/turn/core.rs`, channel/CLI via `harness/graph.rs`, and sub-agent via `harness/subagent_runner/ops/graph.rs`) drives through `crate::openhuman::tinyagents::run_turn_via_tinyagents_shared`, which runs the crate's `AgentHarness`. There is no in-house turn engine, tool loop, or routing gate left; dispatch is unconditional. The seam:

| File (`src/openhuman/tinyagents/`) | Role |
| --- | --- |
| `mod.rs` | The runner (`run_turn_via_tinyagents_shared`): registers openhuman's `Provider`/`Tool` on an `AgentHarness`, runs one turn, caps output via `ProviderModel::with_max_tokens`, mirrors progress, forwards steering, and pauses gracefully at the model-call cap. |
| `mod.rs` / `model.rs` / `tools.rs` / `convert.rs` | `RunPolicy` / `ChatModel` / `Tool` / message adapters (incl. unknown-tool policy and out-of-band reasoning forwarding). |
| `observability.rs` | Harness `AgentEvent` → `AgentProgress` + cost; `GraphTracingSink` for graph events. |
| `orchestration.rs` | Re-exported `graph::orchestration` task-store types; map-reduce fanout now uses the TinyAgents SDK surface directly. |
| `checkpoint.rs` | `SqlRunLedgerCheckpointer`: a `Checkpointer` over openhuman's SQLite (`graph_checkpoints` table). TinyAgents 1.3+ ships `SqliteCheckpointer`; OpenHuman keeps this adapter until existing checkpoint rows are migrated or expired and schema ownership is settled. |
| `delegation.rs` | The durable `plan → execute ⇄ review → finalize` delegation graph (production worker wired in `agent_orchestration::delegation`). |

**Orchestration on graphs** (`src/openhuman/agent_orchestration/`):

- **Workflow phase DAG** (`workflow_runs/engine.rs`) runs on a `dispatch ⇄ run_phase → done` conditional-routing graph; each phase fans its agents out via `graph::parallel::map_reduce`. The durable `workflow_runs` row stays the source of truth (controllers + resume read it).
- **Team member runtime** (`agent_teams/graph.rs`) is a conditional-routing graph (`execute → complete|fail → done`).
- **Multi-stage delegation** (`agent_orchestration::delegation` + the `delegate` tool) runs `delegation.rs`, checkpointed to the session DB.
- **Detached sub-agents** (`running_subagents.rs`) track lifecycle through the crate task-store seam while OpenHuman keeps the executor (abort/steer/await) bespoke for controller compatibility, message injection, and user-facing hard-abort semantics.

**Deliberately kept off the crate's primitives** (documented engineering decisions, not gaps):

- **Sub-agent build pipeline** (`subagent_runner/`) stays openhuman-owned: definition resolution, archetype tool filtering, provider resolution, narrow prompt building, memory context, worker-thread mirror, handoff cache, checkpoint/resume. Sub-agents already *execute* on the harness; the crate's generic `SubAgentTool` would discard this pipeline for marginal crate-native depth tracking (openhuman's `spawn_depth_context` already bounds recursion).
- **Durable run ledgers** (`workflow_runs`, `agent_teams`, `command_center`, `subagent_sessions`) stay on openhuman SQLite/JSON until their controller projections and restart semantics are mapped onto TinyAgents task/status/journal records. The `agent_teams` race-safe SQL compare-and-swap task claim remains OpenHuman-owned.

> **Note:** TinyAgents 1.3+ ships harness store/cache/session primitives (`harness::store` with JSONL append stores, `harness::cache`, `harness::subagent`, lineage-aware status) plus graph task stores and conformance contracts. The session shell, sub-agent pipeline, and detached-task lifecycle are still being migrated onto those primitives.

## Reliability: breakers, handback, and classified failures

Three cooperating mechanisms keep runs from wandering or dying silently:

**No-progress circuit breaker** (`RepeatedToolFailureMiddleware`, `src/openhuman/tinyagents/middleware.rs`) is a thin driver over the crate's `NoProgressTracker`. It fingerprints each tool call's arguments and feeds outcomes into an escalation ladder: `Continue` → `Nudge` (a structured "no progress since step X" corrective injected via `SteeringCommand::InjectMessage`, which is safe inside interactive turns) → `Halt` (record a root-cause summary into the `HaltSummarySlot`, pause via the steering handle). Identical arguments retried count toward the trip (threshold 3 consecutive identical failures); *recoverable* failures (timeouts, connection resets, rate limits, 5xx) get an extended headroom ladder instead of the fixed crate thresholds.

**Sub-agent handback** (`subagent_runner/ops/runner.rs`): a sub-agent run resolves to one of three statuses:

* `Completed`: clean final response.
* `AwaitingUser { question, options }`: the child called `ask_user_clarification`; a full checkpoint (history, question, options, overrides) is written to `{workspace}/.openhuman/subagent_checkpoints/{task_id}.json`, and the run resumes from it when the user answers.
* `Incomplete { reason }`: the child was halted by the breaker or hit its model-call cap. The delegating parent **relays the blocker** instead of treating a halted child as a finished answer or re-spinning the identical delegation.

A breaker halt at the top level is likewise never a silent finish: the turn's final text is overridden with the breaker's root-cause summary, and `hit_cap` / `breaker_halt` are surfaced on the turn result.

**Classified tool failures** (`src/openhuman/tool_status/`): every failed tool call is classified into a transport-agnostic `ClassifiedFailure { class, category, cause_plain, next_action, recoverable }`. Classes cover `MissingPermission`, `MissingApp`, `ServiceUnavailable`, `BadCredentials`, `BlockedByPolicy`, `ModelConnection`, `Timeout`, `Denied`, `ApprovalExpired`; categories map 1:1 to UI states: *recoverable* (safe auto-retry), *blocked by policy* (change settings), *needs user confirmation* (sign in / install / grant), *user declined* (never auto-retried). The classification rides `AgentProgress::ToolCallCompleted.failure` (including for sub-agent calls) into the chat timeline.

## Journals, replay, and migration shadows

Every run appends to a durable **event journal** (`tinyagents/journal.rs`): a `StoreEventJournal` over a JSONL append store at `{workspace}/tinyagents_store/journal`, composed as `FanOutSink` (live bridge + journal) → `RedactingSink` (credential masking before persistence), with restart-stable event ids (`{run_id}-evt-{offset}`). Even an unobserved background turn is reconstructable after the fact. Three read-only RPCs expose it: `agent_run_events` (paged, late-attach replay by `run_id`/`offset`/`limit`), `agent_run_status` (latest harness status), and `agent_runs_active` (active runs, filterable by thread or root run).

The remaining store cutover runs on **shadow scaffolding** (product behavior unchanged; divergences logged):

* **Session dual-write / shadow read** (`session/turn/session_io.rs`): session messages dual-write into the TinyAgents store (default-ON flag `config.session_dual_write`); loads shadow-read for parity while the legacy file store stays authoritative.
* **Task-board shadow** (`todos/graph_shadow.rs`): mirrors the board into the crate `graph.todos` `TaskBoard` and shadow-runs its `claim_card` CAS.
* **Goals shadow** (`thread_goals/crate_adapter.rs`): faithful copy into the crate `graph.goals` KV store, keyed by thread id.

## Workload routes and the burst tier

`tinyagents/routes.rs` projects OpenHuman's workload tiers into the crate `ProviderModel` registry: `chat`, `reasoning`, `agentic`, `coding`, `burst`, `summarization`, `vision`. The **`burst-v1`** tier serves low-context, high-fanout workers (e.g. the SuperContext scout) on a fast/cheap model, while `inference/provider/router.rs` remains the product source of truth for which provider+model backs each tier. Each registry entry carries a `ModelProfile` (vision/reasoning capability, context window) enabling SDK-owned fallback and the model catalog.

## See also

* [Architecture overview](README.md) - where the harness sits in the bigger picture.
* [Memory Tree](../../features/obsidian-wiki/memory-tree.md) - what the memory loader reads from and post-turn hooks write to.
* [Automatic Model Routing](../../features/model-routing/) - how `model: "hint:reasoning"` resolves to a concrete provider+model.
* [Native Tools - Agent Coordination](../../features/native-tools/agent-coordination.md) - the user-facing surface for `spawn_subagent`, `delegate_*`, `todo_write`.
