# Agent / Subagent / Tool Flow

This document explains the current runtime flow around the agent harness, with emphasis on:

- how the main agent turn executes
- how tools are exposed and executed
- how `spawn_subagent` works
- how typed vs fork subagents differ
- where to look when debugging harness and delegation issues

Scope: current Rust implementation under `src/openhuman/agent/` and `src/openhuman/tools/`.

## Why This Exists

The code path is split across several layers:

- built-in agent definitions in `src/openhuman/agent/agents/`
- harness data + task-local plumbing in `src/openhuman/agent/harness/`
- main session lifecycle in `src/openhuman/agent/harness/session/`
- delegation tools in `src/openhuman/tools/impl/agent/`
- synthesised `delegate_*` tools in `src/openhuman/tools/orchestrator_tools.rs`

If you only read one file, the system looks simpler than it is. The actual runtime path crosses all of them.

## File Map

### Registry and definitions

- `src/openhuman/agent/agents/loader.rs`
  Loads built-in agents from `agent.toml` plus dynamic `prompt.rs` builders.
- `src/openhuman/agent/harness/definition.rs`
  Defines `AgentDefinition`, `ToolScope`, `SubagentEntry`, `PromptSource`, and registry-facing data.
- `src/openhuman/agent/harness/mod.rs`
  Re-exports the harness entrypoints.

### Main agent session

- `src/openhuman/agent/harness/session/builder.rs`
  Builds an `Agent`, chooses dispatcher, applies visible-tool filtering, synthesises delegation tools.
- `src/openhuman/agent/harness/session/turn.rs`
  Main turn lifecycle, tool execution, parent/fork context setup, transcript persistence, post-turn hooks.

### Subagent path

- `src/openhuman/tools/impl/agent/spawn_subagent.rs`
  Runtime tool entrypoint for explicit subagent spawns.
- `src/openhuman/agent/harness/fork_context.rs`
  Task-local parent and fork context.
- `src/openhuman/agent/harness/subagent_runner.rs`
  Typed/fork subagent execution, inner loop, tool filtering, transcript writes, large-result handoff.

### Generic tool loop / bus path

- `src/openhuman/agent/harness/tool_loop.rs`
  Shared LLM -> tool -> tool result -> LLM loop used by the bus handler and legacy call sites.
- `src/openhuman/agent/bus.rs`
  Native event-bus entrypoint `agent.run_turn`.

## High-Level Model

There are two related but distinct execution tiers:

1. `Agent::turn`
   This is the stateful session runtime. It owns conversation history, system prompt reuse, memory loading, hooks, transcript resume, and the parent context needed for subagents.

2. `run_subagent`
   This is an isolated delegated run. It does not become a nested full `Agent` session. It runs a smaller inner loop and returns a single compact text result to the parent as a normal tool result.

That distinction matters when debugging. A subagent is not a second copy of the full session runtime.

## Flow Diagram

### Full parent -> tool -> subagent flow

```text
User message
    |
    v
+---------------------------+
| Agent::turn               |
| session/turn.rs           |
+---------------------------+
    |
    | 1. resume transcript if present
    | 2. build/reuse system prompt
    | 3. load memory context
    | 4. install ParentExecutionContext task-local
    v
+---------------------------+
| Parent iteration loop     |
| provider call             |
+---------------------------+
    |
    | provider response
    v
+---------------------------+
| Parse tool calls          |
| dispatcher + parser       |
+---------------------------+
    |
    +-------------------------------+
    | no tool calls                 |
    |                               |
    v                               |
+---------------------------+       |
| Final assistant text      |       |
| appended to parent history|       |
+---------------------------+       |
    |                               |
    v                               |
Return to caller                    |
                                    |
                                    | has tool calls
                                    v
                          +---------------------------+
                          | Execute tool calls        |
                          | parent tool runtime       |
                          +---------------------------+
                                    |
                +-------------------+-------------------+
                |                                       |
                | regular tool                          | spawn_subagent
                v                                       v
      +---------------------------+         +---------------------------+
      | Tool::execute(...)        |         | SpawnSubagentTool         |
      +---------------------------+         | impl/agent/               |
                |                           | spawn_subagent.rs         |
                | result                    +---------------------------+
                v                                       |
      +---------------------------+                     | validate args
      | append tool result        |                     | lookup AgentDefinition
      | to parent history         |                     | publish spawn event
      +---------------------------+                     v
                |                           +---------------------------+
                +-------------------------->| run_subagent(...)        |
                                            | subagent_runner.rs       |
                                            +---------------------------+
                                                        |
                              +-------------------------+-------------------------+
                              |                                                   |
                              | typed mode                                        | fork mode
                              v                                                   v
                    +---------------------------+                     +---------------------------+
                    | run_typed_mode            |                     | run_fork_mode             |
                    | - resolve model           |                     | - require ForkContext     |
                    | - filter tools            |                     | - replay parent prefix    |
                    | - build narrow prompt     |                     | - reuse parent tool specs |
                    +---------------------------+                     +---------------------------+
                              |                                                   |
                              +-------------------------+-------------------------+
                                                        |
                                                        v
                                            +---------------------------+
                                            | run_inner_loop            |
                                            | subagent private loop     |
                                            +---------------------------+
                                                        |
                            +---------------------------+---------------------------+
                            |                                                       |
                            | no tool calls                                         | tool calls
                            v                                                       v
                  +---------------------------+                         +---------------------------+
                  | final child text          |                         | child executes allowed    |
                  | returned to parent tool   |                         | tools, appends results,   |
                  +---------------------------+                         | loops again               |
                                                                        +---------------------------+
                                                        |
                                                        v
                                            +---------------------------+
                                            | SpawnSubagentTool returns |
                                            | ToolResult(output)        |
                                            +---------------------------+
                                                        |
                                                        v
                                            +---------------------------+
                                            | parent appends tool       |
                                            | result to history         |
                                            +---------------------------+
                                                        |
                                                        v
                                            +---------------------------+
                                            | next parent iteration     |
                                            | synthesizes final answer  |
                                            +---------------------------+
```

### Context wiring for subagents

```text
Agent::turn
    |
    +--> build ParentExecutionContext
    |      - provider
    |      - all_tools / all_tool_specs
    |      - model / temperature
    |      - memory / memory_context
    |      - connected_integrations
    |      - composio_client
    |      - tool_call_format
    |      - session lineage
    |
    +--> with_parent_context(...)
              |
              +--> any tool call inside this turn can read current_parent()
                        |
                        +--> SpawnSubagentTool
                                  |
                                  +--> run_subagent(...)
                                            |
                                            +--> typed mode uses ParentExecutionContext directly
                                            |
                                            +--> fork mode also requires current_fork()
                                                      |
                                                      +--> exact parent prompt + prefix replay
```

## Startup and Registry Loading

Built-in agents live under `src/openhuman/agent/agents/*/` as:

- `agent.toml`
- `prompt.rs`
- optional `prompt.md` kept as nearby reference material

`loader.rs` parses each `agent.toml`, stamps the source as builtin, and installs the `prompt.rs` builder as `PromptSource::Dynamic`.

The global `AgentDefinitionRegistry` is initialized at startup. `spawn_subagent` depends on it. If the registry is missing, the tool returns a clear error instead of trying to run.

Important consequence: agent delegation is data-driven. The runtime does not hardcode an enum of built-in agents.

## How a Main Agent Session Is Built

`AgentBuilder::build` in `session/builder.rs` assembles:

- provider
- full tool registry
- visible tool specs
- memory backend
- prompt builder
- dispatcher
- context manager

Two tool sets exist at build time:

- full tool registry: what the runtime can execute
- visible tool set: what the model can see in its schema/prompt

That split is intentional. The parent may have access to more runtime tools than it exposes directly to the model.

### Synthesised delegation tools

For agents with `subagents = [...]` in their definition, the builder synthesises `delegate_*` tools using `collect_orchestrator_tools()`:

- `SubagentEntry::AgentId("researcher")` becomes an `ArchetypeDelegationTool`
- `SubagentEntry::Skills({ skills = "*" })` expands to one `SkillDelegationTool` per connected integration

These tools are added to the model-visible surface at build time. They are wrappers around delegation, not standalone business logic.

## Main Turn Flow

`Agent::turn` in `session/turn.rs` is the main harness path.

### 1. Transcript resume and prompt bootstrap

On a fresh session:

- it tries to resume a previous transcript for KV-cache reuse
- fetches connected integrations
- fetches learned context
- builds the system prompt once
- stores that system prompt as the first message

On later turns it deliberately does not rebuild the system prompt. Byte stability is treated as a runtime invariant for backend prefix caching.

### 2. Memory context injection

Per turn, it asks the memory loader for relevant context and prepends that context to the user message. This is parent-session behavior. Subagents do not run the same memory lookup path.

### 3. Parent execution context is captured

Before the loop starts, `Agent::turn` snapshots a `ParentExecutionContext` and installs it on the task-local via `with_parent_context(...)`.

That context carries the data subagents need:

- provider
- all tools and tool specs
- model / temperature
- memory handle
- loaded memory context
- connected integrations
- composio client
- tool call format
- session / transcript lineage

Without this task-local, `spawn_subagent` cannot work.

### 4. Iterative provider loop

For each iteration:

- context reduction runs first
- the dispatcher converts history into provider messages
- the provider is called
- response text and tool calls are parsed
- tool calls are executed
- tool results are appended to history
- the loop repeats until no tool calls remain

This is the full parent loop. It also emits progress events and drives post-turn hooks.

## Tool Execution in the Parent Loop

The parent loop special-cases delegation but otherwise treats tools generically.

Core behaviors:

- unknown or filtered-out tools become structured error results
- `CliRpcOnly` tools are blocked in the autonomous loop
- approval-gated tools can be denied before execution
- successful outputs may be scrubbed / compacted / summarized

The parent’s history preserves:

- assistant tool call intent
- tool results
- final assistant response

That history format is what the next iteration reasons from.

## Where `spawn_subagent` Enters

The explicit delegation tool lives in `src/openhuman/tools/impl/agent/spawn_subagent.rs`.

Its flow is:

1. parse `agent_id`, `prompt`, optional `context`, optional `toolkit`, optional `mode`
2. require the global `AgentDefinitionRegistry`
3. resolve the target definition
4. run pre-flight validation for `integrations_agent`
5. publish `DomainEvent::SubagentSpawned`
6. call `run_subagent(...)`
7. publish completed or failed event
8. return the subagent’s final text as a normal `ToolResult`

Important: the parent model never sees the subagent’s internal transcript. It only sees the final tool result string returned by `spawn_subagent`.

## Typed vs Fork Subagents

`run_subagent` chooses one of two modes.

### Typed mode

Default path. Implemented by `run_typed_mode(...)`.

Behavior:

- resolves model from the definition
- filters the parent’s tools down to what the child is allowed to use
- builds a fresh narrow system prompt
- optionally injects inherited memory context
- runs an isolated inner tool loop

This is the normal specialist-agent path.

### Fork mode

Optimization path. Implemented by `run_fork_mode(...)`.

Behavior:

- requires a `ForkContext` task-local
- replays the parent’s exact rendered prompt and exact message prefix
- reuses the parent’s tool schema snapshot
- appends only the new fork task prompt
- runs the same inner loop

This is for prefix-cache reuse, not for stricter isolation. It is deliberately byte-stable and closely coupled to the parent request shape.

## How Tool Filtering Works for Subagents

Typed subagents do not get a cloned tool registry. Instead the runner filters the parent’s tool list by index.

Filtering inputs:

- `definition.tools`
- `definition.disallowed_tools`
- `definition.skill_filter`
- `SubagentRunOptions.skill_filter_override`
- `definition.extra_tools`

Additional runtime rules:

- non-`welcome` subagents lose `complete_onboarding`
- `tools_agent` strips Composio skill tools
- `integrations_agent` with a bound toolkit may inject dynamic per-action Composio tools

The allowed tool names become both:

- the execution allowlist
- the prompt-visible tool catalog

If the model emits a tool call outside that allowlist, the runner feeds back an error result and continues.

## Prompt Construction for Typed Subagents

Typed mode creates a `PromptContext` and then does one of:

- `PromptSource::Dynamic`: call the Rust prompt builder directly
- `PromptSource::Inline` or `PromptSource::File`: load raw body, then wrap it with `render_subagent_system_prompt(...)`

Definition flags control which standard sections are omitted:

- `omit_identity`
- `omit_memory_context`
- `omit_safety_preamble`
- `omit_skills_catalog`
- `omit_profile`
- `omit_memory_md`

This is one of the main token-saving levers in the harness.

## The Subagent Inner Loop

The actual delegated execution happens in `run_inner_loop(...)`.

It is a slimmed-down tool loop:

- call provider
- parse tool calls
- persist transcript after provider response
- execute tools
- append results
- persist transcript again
- stop on final text or max iterations

It returns:

- final output text
- iteration count
- aggregated usage

Unlike the parent `Agent::turn`, it does not own the broader session lifecycle.

## Integrations Agent Special Cases

`integrations_agent` is the trickiest subagent path.

### Toolkit gate in `spawn_subagent`

If `agent_id == "integrations_agent"`:

- `toolkit` is mandatory
- the toolkit must exist in the allowlist
- if it exists but is not connected, the tool returns a success message explaining that authorization is required

This is intentionally not always treated as a hard tool failure, because disconnected integrations are a user-facing state, not necessarily a runtime error.

### Text-mode override

In `run_inner_loop`, `integrations_agent` with tool specs forces text mode instead of native tool calling.

Why:

- large Composio JSON schemas can blow provider grammar/context limits

What changes:

- tool specs are omitted from the API payload
- XML-style tool instructions are injected into the system prompt
- the runner parses `<tool_call>...</tool_call>` blocks out of plain text
- tool results in text mode are fed back as a user message containing `<tool_result>` tags

If a delegated integration run looks different from native-tool runs, this is usually why.

### Large result handoff cache

For toolkit-scoped `integrations_agent` runs, oversized tool results may be replaced by placeholders and stashed in an in-memory `ResultHandoffCache`.

The child can then call `extract_from_result(result_id, query)` to ask targeted follow-up questions against the cached payload.

This is not the same as generic payload summarization. It is a progressive-disclosure path specific to oversized delegated tool outputs.

## Parent -> Subagent -> Parent Result Shape

Conceptually the data flow is:

1. parent model emits `spawn_subagent(...)`
2. tool runtime executes the delegated subagent loop
3. subagent finishes with one final text output
4. `spawn_subagent` returns that text as its tool result
5. parent history receives the tool result
6. parent model gets another iteration and synthesizes the user-facing answer

The parent does not absorb the child’s internal reasoning trace or full message history. Only the compact final output crosses the boundary.

## Bus Path vs Session Path

There are two outer entrypoints to keep straight.

### `Agent::turn`

Used for full stateful sessions. This is the richer harness.

### `agent.run_turn` via `src/openhuman/agent/bus.rs`

This native event-bus handler calls `run_tool_call_loop(...)` directly using owned Rust payloads.

It supports:

- provider reuse
- tool filtering
- per-turn extra tools
- progress streaming

But it does not create a full `Agent` session object. If you are debugging channel-dispatch behavior, this distinction matters.

## Debugging Checklist

### 1. Confirm which execution tier you are in

Ask first:

- full `Agent::turn` session?
- bus `agent.run_turn` path?
- explicit `spawn_subagent` tool?
- synthesised `delegate_*` tool leading into `spawn_subagent`?

If you confuse these, logs will look contradictory.

### 2. Check registry state

If delegation fails very early, confirm:

- `AgentDefinitionRegistry::init_global(...)` ran at startup
- the target agent id exists
- workspace overrides did not shadow the expected built-in definition

### 3. Check task-local availability

If `run_subagent` errors with missing context:

- `NoParentContext` means the tool ran outside a parent turn
- `NoForkContext` means fork mode was requested but the fork snapshot was never installed

These are wiring issues, not prompt issues.

### 4. Check tool visibility vs tool execution

A tool can exist in the parent registry but still be invisible to a child due to:

- named `ToolScope`
- `disallowed_tools`
- `skill_filter`
- welcome-only stripping
- toolkit narrowing

If the model says “Unknown tool” or “not available to this sub-agent”, inspect filtering first.

### 5. Check transcript artifacts

Subagents persist transcripts per iteration using the parent session lineage plus a child session key. This is useful for debugging partial runs and crashes during tool execution.

Parent sessions and subagents do not write identical transcript shapes, so compare like with like.

### 6. Check the provider mode

If tool calling is malformed, verify whether the run used:

- native tools
- p-format / xml instructions
- integrations-agent text mode

The parser and message shape differ.

## Useful Log Prefixes

These prefixes are the most useful grep anchors:

- `[agent_loop]`
- `[agent]`
- `[tool-loop]`
- `[spawn_subagent]`
- `[subagent_runner]`
- `[subagent_runner:typed]`
- `[subagent_runner:fork]`
- `[subagent_runner:text-mode]`
- `[subagent_runner:handoff]`
- `[orchestrator_tools]`
- `[agent::bus]`
- `[transcript]`

## Best Existing Tests to Read First

For end-to-end harness behavior:

- `src/openhuman/agent/harness/session/tests.rs`
  - `turn_dispatches_spawn_subagent_through_full_path`
  - `turn_dispatches_spawn_subagent_in_fork_mode`

For runner behavior in isolation:

- `src/openhuman/agent/harness/subagent_runner.rs` tests
  - typed mode returns text
  - memory-context inclusion/omission
  - tool filtering
  - one-tool execution
  - blocked tool recovery
  - fork prefix replay
  - missing parent/fork context errors

For orchestration-tool synthesis:

- `src/openhuman/tools/orchestrator_tools.rs` tests

For generic parent loop behavior:

- `src/openhuman/agent/tests.rs`

## Common Failure Modes

### Subagent never starts

Usually one of:

- registry not initialized
- invalid `agent_id`
- missing parent context
- missing fork context

### Subagent starts but cannot call expected tools

Usually one of:

- tool filtered out by definition scope
- `skill_filter` or toolkit override narrowed too aggressively
- tool is `CliRpcOnly`
- dynamic integration tools were not injected because the toolkit/client state was missing

### Integrations agent behaves unlike other agents

Usually expected. It may be in text mode and may be using the oversized-result handoff cache.

### Parent seems to “lose” child reasoning

Expected. Only the child’s final output is returned to the parent. Internal child history stays isolated.

## Practical Mental Model

The safest mental model is:

- the parent session is the durable conversation runtime
- tools are the execution boundary
- subagents are tool implementations that happen to run their own mini LLM loop
- fork mode is a cache-optimization path, not a different product feature
- `integrations_agent` is a special delegated runtime with extra provider and payload safeguards

If you debug from that model, the current codebase makes much more sense.
