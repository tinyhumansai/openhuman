# agent

The agent domain is where a user message becomes model calls, tool calls and
a reply. It composes OpenHuman's product policy (agent definitions, prompts,
tool visibility, approvals, budgets, progress events) around the vendored
TinyAgents loop, and it owns sub-agent delegation, trigger triage, and the
bundled prompt and archetype assets. Web chat, channel runtimes, cron jobs,
flows, MCP, voice and the `agent.chat` RPC all reach the model through this
folder.

It does not implement the agent loop itself. The model/tool iteration,
sessions, transcripts, run queues and orchestration state machines live in
[`vendor/tinyagents`](../../../../vendor/tinyagents/). Most of the code here
is adapters that hand TinyAgents the things only OpenHuman knows.

## How it works

### Layering

```text
  callers: web_chat, channels, cron, flows, mcp, voice, agent.chat RPC
     |                                   |
     | OpenHumanSessionHost              | BUS.native() "agent.run_turn"
     | (stateful, per thread)            | (stateless, caller owns history)
     v                                   v
  +-----------------------------------------------------------------+
  | agent/session_host/            agent/bus.rs + harness/graph.rs  |
  |   builder, hooks, codec, driver                                 |
  +-----------------------------------------------------------------+
     |                                   |
     v                                   v
  +-----------------------------------------------------------------+
  | agent/tinyagents/   the adapter seam                            |
  |   run_root_turn_via_hosted_agent / run_turn_via_tinyagents_shared|
  |   TurnModelSource, middleware stack, host/ capability adapters, |
  |   OpenhumanEventBridge, journal                                 |
  +-----------------------------------------------------------------+
     |
     v
  +-----------------------------------------------------------------+
  | vendor/tinyagents                                               |
  |   tinyagents-runtime   Session: history, resume, commit         |
  |   tinyagents-session   SessionRef, transcripts, run ledger      |
  |   tinyagents-harness   AgentHarness loop, middleware, RunQueue  |
  |   tinyagents-graph / -orchestration   delegation, sub-agents    |
  |   tinytools, tinyinference   Tool trait, ChatModel, providers   |
  +-----------------------------------------------------------------+
     |                         |
     v                         v
  crate::tools (Tool impls)   crate::inference (providers, models)
```

There are two ways into a turn. The stateful one is `OpenHumanSessionHost`
(re-exported from `crate::agent`), which owns one conversation, resumes it
from its transcript, and is what web chat, cron, flows, MCP and voice use.
The stateless one is the `agent.run_turn` native bus handler in [`bus.rs`](./bus.rs):
the caller passes the whole history, the tool registry and a
`TurnModelSource` as owned Rust values in an `AgentTurnRequest` and gets an
`AgentTurnResponse` back. Channel runtimes
([`channels/runtime/dispatch/processor/turn.rs`](../channels/runtime/dispatch/processor/turn.rs)) and the triage evaluator use
the bus path because they keep their own history.

### A chat turn, end to end

```text
 web_chat::run_task (or cron, flows, mcp, ...)
   | build or reuse OpenHumanSessionHost::from_config_for_agent(cfg, id)
   v
 run_single_with_origin(msg, origin)          session_host/runtime/run_loop.rs
   | prompt-injection guard, publish AgentTurnStarted
   v
 turn_with_origin                     session_host/runtime_session_turn.rs
   | stage attachments, build OpenHumanRunContext (origin, progress,
   | thread, workspace, cancellation), resume transcript if bound
   v
 tinyagents_runtime::Session::turn
   |-- OpenHumanSessionHooks::before_turn     prompt, memory pack, tools
   |-- OpenHumanSessionDriver::execute        session_host/driver.rs
   |     v
   |   run_chat_turn_graph                    session_host/turn/graph.rs
   |     v
   |   run_root_turn_via_hosted_agent         tinyagents/turn_runner.rs
   |     | assemble_turn_harness: models + tools + middleware
   |     v
   |   AgentHarness loop  <---------------------------------+
   |     model call -> tool calls -> middleware -> results --+
   |     |  events -> OpenhumanEventBridge -> AgentProgress -> on_progress
   |     v
   |   grounded close if the loop hit its cap
   |-- OpenHumanTranscriptCodec              durable rows, commit
   |-- after_commit                          memory, BUS, dual-write
   v
 publish AgentTurnCompleted, return final text
```

In order:

1. The caller builds a session host. `from_config_for_agent` resolves the
   `AgentDefinition` for the agent id, picks its model and provider role,
   assembles the tool registry, and wires the `ContextManager`,
   `ToolPolicy`, run queue and progress sender. Web chat caches one host per
   thread ([`web_chat/session.rs`](../web_chat/session.rs)); cron builds one per job.
2. The caller scopes an `AgentTurnOrigin` ([`turn_origin.rs`](./turn_origin.rs)) and calls
   `run_single_with_origin`. The origin (`WebChat`, `ExternalChannel`,
   `TrustedAutomation`, `Cli`, `DirectChat`, `Unknown`) is what the approval
   gate reads to decide trust. `run_single_with_origin` runs the
   prompt-injection guard from `security::prompt_injection`, publishes
   `DomainEvent::AgentTurnStarted`, and calls `turn_with_origin`.
3. `turn_with_origin` stages any attachments, builds the explicit per-turn
   carrier `OpenHumanRunContext` ([`tinyagents/host/run_context.rs`](./tinyagents/host/run_context.rs)), and
   resumes the conversation if it is bound to a `SessionRef` with
   `ResumeMode::Session`. On resume it adopts the recorded tool list so
   Composio actions the thread used before are rebuilt as deferred executors
   ([`session_host/recorded_tools.rs`](./session_host/recorded_tools.rs)).
4. TinyAgents' `Session::turn` runs the lifecycle. Its `before_turn` hook
   (`OpenHumanSessionHooks`) prepares the request: system prompt (frozen
   for a resumed thread), per-turn memory pack, integration, MCP and skill
   announcements, and the tool surface. The tool surface is rebuilt as one
   unit (executable tools, visible specs, deferred names, policy) so the
   schema the model sees and the authority the driver enforces cannot drift.
5. The session calls `OpenHumanSessionDriver::execute` with the full
   history and the tool snapshot. The driver builds this turn's tiered
   `ChatModel` set from `TurnModelSource`, resolves the context window, and
   calls `run_chat_turn_graph`, which calls
   `run_root_turn_via_hosted_agent`.
6. `assemble_turn_harness` ([`tinyagents/harness_assembly.rs`](./tinyagents/harness_assembly.rs)) registers the
   models, the tools (through `CanonicalSharedToolAdapter`) and the
   middleware stack: approval and security gating, tool policy, cost
   budget, tool-output capping and summarization, credential scrubbing,
   repeated-failure circuit breaker, research budget, stop hooks, and the
   generic context ladder from `tinyagents-harness`. Then the harness loop
   runs: model call, parse tool calls, run them through middleware, append
   results, repeat until the model answers or a cap is hit.
7. Every harness `AgentEvent` goes through `OpenhumanEventBridge`
   ([`tinyagents/observability/event_bridge.rs`](./tinyagents/observability/event_bridge.rs)), which turns it into an
   `AgentProgress` value ([`progress.rs`](./progress.rs)) on the session's `on_progress`
   channel and feeds usage to `platform::cost`. Web chat forwards that
   stream to the socket in [`web_chat/progress_bridge.rs`](../web_chat/progress_bridge.rs). A `TurnJournal`
   ([`tinyagents/journal.rs`](./tinyagents/journal.rs)) writes the same events to a durable JSONL
   store that the replay RPCs read.
8. If the loop paused at its model-call cap without a conclusion, the driver
   runs a tools-disabled grounded close
   ([`session_host/driver/grounded_close.rs`](./session_host/driver/grounded_close.rs)).
9. The runtime commits the turn. `OpenHumanTranscriptCodec`
   ([`session_host/codec.rs`](./session_host/codec.rs)) converts between durable `TranscriptMessage`
   rows and model `Message`s. After the durable commit, the host publishes
   `ConversationTurnCommitted` for memory ingest, optionally dual-writes
   the legacy session format (`session_import::live`), and accounts usage
   against any active goal.
10. `run_single_with_origin` publishes `AgentTurnCompleted` (or
    `AgentError`) and returns the final text.

The bus path is shorter. `bus.rs` scopes the origin, sandbox mode and file
state agent id, then calls `harness::run_channel_turn_via_graph`
([`harness/graph.rs`](./harness/graph.rs)), which calls `run_turn_via_tinyagents_shared`. There is
no TinyAgents `Session`: the caller owns the history vector, and
`ask_user_clarification` is an early-exit tool whose question becomes the
reply.

### Sessions and transcripts

A conversation's durable identity is
`tinyagents_session::transcript::SessionRef`, derived from the thread id and
the agent id with no timestamp, so one thread maps to one transcript file in
every process. The session host binds it and resumes with
`ResumeMode::Session`. A resumed session reuses its persisted system prompt
and tool declarations verbatim, which keeps the provider's prefix cache warm
across restarts; the cost is that prompt edits and new skills do not reach
an existing thread. Compaction seals the current generation and opens a new
one with the sealed one as parent, so nothing is erased.

By default transcripts, journals, goals and todos are files under the
workspace. A host that keeps them in its own database installs a
`SessionStoreProvider` once through `session_store/` (re-exported from
`tinyagents_session::port`), and every agent then reads and writes through
its `AgentStores`. See [session_host/README.md](session_host/README.md).

### Sub-agents and orchestration

A parent agent delegates by calling a tool. `spawn_subagent`
([`orchestration/tools/spawn_subagent.rs`](./orchestration/tools/spawn_subagent.rs)) and its siblings
(`spawn_async_subagent`, `spawn_parallel_agents`, `continue_subagent`,
`steer_subagent`, `wait_subagent`, `close_subagent`, `delegate`) look up the
target `AgentDefinition` and call `subagent_host::run_subagent_with_parent`.

```text
 parent harness loop
   | tool call: spawn_subagent { agent_id, prompt }
   v
 orchestration/tools/spawn_subagent.rs
   | read ParentExecutionContext (task-local, harness/fork_context.rs)
   v
 subagent_host::run_subagent_with_parent
   | resolve definition, model, narrowed tools, narrowed prompt,
   | workspace/sandbox, SubagentScope for progress
   v
 tinyagents-orchestration::subagent lifecycle
   v
 run_turn_via_tinyagents_shared   (subagent_host/ops/graph/dispatch.rs)
   | child events -> Subagent* AgentProgress variants on parent channel
   v
 one compact tool_result back to the parent
```

The child gets a filtered tool list (the definition's `tools`,
`disallowed_tools`, `skill_filter`), its own system prompt, and often a
cheaper model. Its progress shows up on the parent's channel as
`SubagentSpawned`, `SubagentToolCallStarted`, `SubagentTextDelta` and the
other `Subagent*` variants. Spawn depth is capped by
`harness::MAX_SPAWN_DEPTH`. Detached runs are tracked through TinyAgents'
`DetachedTaskRegistry` and can be steered through the shared
`SteeringRegistry` ([`tinyagents/host/steering.rs`](./tinyagents/host/steering.rs)).

`orchestration/` is the larger control plane on top of that: agent teams,
the command center, declarative workflow runs, git worktrees for parallel
coding workers, and background completion delivery. See
[orchestration/README.md](orchestration/README.md) and
[subagent_host/README.md](subagent_host/README.md).

### Agent definitions, registry and prompts

An `AgentDefinition` ([`harness/definition/`](./harness/definition/)) is an archetype: id, tier,
prompt source, model spec, `ToolScope`, `SandboxMode`, iteration policy and
allowed sub-agents. Built-ins live in `registry/agents/<id>/` as an
`agent.toml` plus `prompt.md` (orchestrator, planner, critic, summarizer,
trigger_triage, trigger_reactor, vision_agent and others). Custom
definitions are TOML files under `<workspace>/agents/*.toml`, with
`~/.openhuman/agents/*.toml` as a fallback ([`harness/definition_loader.rs`](./harness/definition_loader.rs)).
`AgentDefinitionRegistry` holds the merged set; `agent.reload_definitions`
rebuilds it.

`registry/` is the user-facing layer over those definitions: enable and
disable, custom agents, model pins, tool allow and deny lists, and sub-agent
policy, persisted at `Config.agent_registry` and exposed as the
`agent_registry` RPC namespace. `library/` projects definitions into a
display-safe `AgentDefinitionDisplay` for the UI.

`prompts/` holds the bundled identity files (`SOUL.md`, `IDENTITY.md`,
`ROLE.md`, `STYLE.md`, `USER.md`) and `SystemPromptBuilder`, which renders
ordered `PromptSection`s into the system prompt, optionally as a
`TieredPrompt` with cache breakpoints. `context/` keeps per-session
`ContextManager` state and the byte-stable channel prompt builder. `debug/`
renders the exact prompt a live session would send, by building a real
session host and calling `build_system_prompt`.

### Host agents

[`host_agents.rs`](./host_agents.rs) is a process-wide `HostAgentResolver` slot (installed and
cleared like the session store). An embedding host registers its agents there; cron agent
jobs and workflow `agent` nodes ask it first. On a hit the session is built as that agent
(`HostAgent::session_host()` under `CoreContext::sync_scope`: its definition and prompt, host
tools, provider route and own `CoreContext`) and the turn runs inside `HostAgent::scope()`.
A miss falls through to the registries, unchanged.

### Triage

External events (Composio triggers, incoming webhooks, task-source cards,
desktop notifications, the `agent.triage_evaluate` RPC) are not chat turns.
The caller wraps the event in a `TriggerEnvelope`, `run_triage` sends a
zero-tool `trigger_triage` turn over `agent.run_turn` (cloud first, one
retry, then a local model arm, else `Deferred`), and `apply_decision` acts
on the parsed `drop` / `acknowledge` / `react` / `escalate` decision.
`react` and `escalate` pass the approval gate and then run `trigger_reactor`
or `orchestrator` through `subagent_host::run_subagent`. Cron jobs do not go
through triage. See [triage/README.md](triage/README.md).

## Layout

Turn execution:

| Path | What it does |
| --- | --- |
| `session_host/` | `OpenHumanSessionHost`: builder, runtime session, driver, hooks, transcript codec, resume and recorded tools ([README](session_host/README.md)) |
| `tinyagents/` | Adapter seam onto the TinyAgents harness: turn runner, model source, middleware stack, `host/` capability adapters, event bridge, journal, replay RPC ([README](tinyagents/README.md)) |
| `harness/` | Agent definitions and loader, parent/fork context task-locals, sandbox and spawn-depth context, oversized tool-result artifacts, the channel/CLI turn graph ([README](harness/README.md)) |
| `bus.rs` | The `agent.run_turn` native request handler (`AgentTurnRequest` to `AgentTurnResponse`) |
| `progress.rs` | `AgentProgress`, the event enum every turn streams to its observer |
| [`progress_sink.rs`](./progress_sink.rs) | Task-local progress sink for in-process embedders driving an RPC that returns only text |
| [`progress_tracing.rs`](./progress_tracing.rs), `progress_tracing/` | OpenTelemetry/Langfuse-style spans built from the progress stream ([README](progress_tracing/README.md)) |
| `turn_origin.rs` | `AgentTurnOrigin` task-local, the trust and routing label the approval gate reads |
| [`turn_workspace.rs`](./turn_workspace.rs) | Task-local per-turn filesystem root an embedder binds one turn to |
| [`stop_hooks.rs`](./stop_hooks.rs) | `StopHook` mid-turn halts and the scoped `with_tool_call_limit` budget |
| [`tool_policy.rs`](./tool_policy.rs) | `ToolPolicy`, the pre-execution allow/deny hook installed through the builder |
| [`hooks.rs`](./hooks.rs) | `PostTurnHook` background hooks, including ones registered by embedders |
| [`queued_turn.rs`](./queued_turn.rs) | `QueuedTurn`, the payload for a message sent while a turn is running |
| [`error.rs`](./error.rs) | `AgentError`, typed retryable and permanent loop errors |
| [`cost.rs`](./cost.rs) | Per-turn token and cost accounting (`TurnCost`) |
| [`message_convert.rs`](./message_convert.rs), [`messages.rs`](./messages.rs) | Conversions between `TranscriptMessage` rows and TinyAgents `Message`s; the `history_wire` adapter for host-owned files |
| `attachments/`, [`multimodal.rs`](./multimodal.rs) | Durable user uploads and provider-side attachment resolution; legacy marker compatibility ([README](attachments/README.md)) |
| [`host_runtime.rs`](./host_runtime.rs), [`platform_shell.rs`](./platform_shell.rs) | Native and Docker shell `RuntimeAdapter`s and the shared `cmd.exe` vs `sh` selection |
| `session_store/` | Process-wide `SessionStoreProvider` slot for hosts with their own database |

Delegation and coordination:

| Path | What it does |
| --- | --- |
| `subagent_host/` | Runs a child agent: definition, model and tool narrowing, prompt, checkpoints, progress ([README](subagent_host/README.md)) |
| `orchestration/` | Spawn/steer/wait tools, teams, command center, workflow runs, worktrees, background delivery ([README](orchestration/README.md)) |
| `triage/` | Classifies `TriggerEnvelope`s and escalates ([README](triage/README.md)) |
| `goals/` | Host adapters around `tinyagents_graph::goals` and the `goal_*` tools ([README](goals/README.md)) |
| `todos/` | Session-scoped todo list over the TinyAgents todo store ([README](todos/README.md)) |
| `plan_review/` | `request_plan_review` gate that parks a web-chat turn until the user decides |
| [`tools.rs`](./tools.rs), `tools/` | Loop control tools: `delegate`, `plan_exit`, `todo`, `run_workflow`/`await_workflow` (with `skills`) ([README](tools/README.md)) |

Definitions, prompts and context:

| Path | What it does |
| --- | --- |
| `registry/` | `agent_registry` RPC; built-in archetypes under [`registry/agents/`](./registry/agents/) ([README](registry/README.md)) |
| `prompts/` | Bundled identity files, `SystemPromptBuilder`, prompt sections ([README](prompts/README.md)) |
| `context/` | `ContextManager` per session and the channel prompt builder ([README](context/README.md)) |
| [`context_breakdown.rs`](./context_breakdown.rs) | `agent.context_breakdown`: system/tools/history split of prompt size for the UI |
| `debug/` | Dumps the exact system prompt and prompt-size report for an agent |
| `library/` | Display-safe projection of definitions (`AgentDefinitionDisplay`) |

Persistence, setup and RPC:

| Path | What it does |
| --- | --- |
| `artifacts/` | Agent-generated artifact storage and the `ai` RPC namespace ([README](artifacts/README.md)) |
| `session_db/` | `run_ledger` RPC over `tinyagents_session::run_ledger` |
| `session_import/` | One-time import of legacy session files and the optional live dual-write ([README](session_import/README.md)) |
| [`schemas.rs`](./schemas.rs) | The `agent` namespace controllers |

## Key types and entry points

- `OpenHumanSessionHost` ([`session_host/types.rs`](./session_host/types.rs)): one conversation.
  Construct with `from_config`, `from_config_for_agent` or
  `from_config_with_definition` ([`session_host/builder/factory.rs`](./session_host/builder/factory.rs)), or the
  `SessionHostBuilder` fluent API. Drive with `run_single` or
  `run_single_with_origin` ([`session_host/runtime/run_loop.rs`](./session_host/runtime/run_loop.rs)).
  `TurnOverrides` adjusts the next turn only.
- `AgentTurnRequest` / `AgentTurnResponse` and `AGENT_RUN_TURN_METHOD`
  (`bus.rs`): the stateless turn over `BUS.native()`.
  `register_agent_handlers` installs it at startup.
- `TurnModelSource` ([`tinyagents/turn_models.rs`](./tinyagents/turn_models.rs)): builds a turn's primary,
  tier-routed and summarizer `ChatModel`s from `(role, config)`.
- `run_turn_via_tinyagents_shared` and `run_root_turn_via_hosted_agent`
  ([`tinyagents/turn_runner.rs`](./tinyagents/turn_runner.rs)): the only places a harness is invoked.
- `OpenHumanRunContext` (`tinyagents/host/run_context.rs`): the explicit
  per-turn carrier (origin, progress, thread, workspace, cancellation, stop
  hooks) passed into TinyAgents and down to recursive tools.
- `AgentProgress` (`progress.rs`): turn, iteration, tool, text and thinking
  deltas, cost updates and the `Subagent*` family.
- `AgentDefinition`, `AgentDefinitionRegistry`, `ToolScope`, `SandboxMode`
  (`harness/definition/`): what an agent is.
- `ParentExecutionContext` ([`harness/fork_context.rs`](./harness/fork_context.rs)): task-local parent
  state a sub-agent reads; `orchestration::parent_context::build_root_parent`
  builds one for runs with no parent turn.
- `run_subagent`, `run_subagent_with_parent`, `SubagentRunOptions`
  ([`subagent_host/lifecycle.rs`](./subagent_host/lifecycle.rs), [`subagent_host/types.rs`](./subagent_host/types.rs)).
- `run_triage`, `apply_decision`, `TriggerEnvelope` (`triage/`).
- `SystemPromptBuilder` ([`prompts/builder.rs`](./prompts/builder.rs)).
- `AgentTurnOrigin` and `with_origin` (`turn_origin.rs`).

## RPC surface

All controllers register under `DomainGroup::Agent` in [`core/all.rs`](../core/all.rs).

| Namespace | Owner | Methods |
| --- | --- | --- |
| `agent` | `schemas.rs` | `chat`, `chat_simple`, `server_status`, `list_definitions`, `get_definition`, `reload_definitions`, `triage_evaluate`, `graph_topologies`, `registry_snapshot`, `context_breakdown` |
| `agent` | [`tinyagents/replay/`](./tinyagents/replay/) | `runs_active`, `run_status`, `run_events` (read-only journal replay) |
| `agent` | [`tinyagents/run_mode.rs`](./tinyagents/run_mode.rs) | `set_run_mode`, `get_run_mode` |
| `agent_registry` | `registry/` | `list`, `available_tools`, `get`, `upsert_custom`, `create_custom`, `update`, `set_enabled`, `remove` |
| `plan_review` | `plan_review/` | `decide` |
| `ai` | `artifacts/` | `list_artifacts`, `get_artifact`, `delete_artifact`, `regenerate` |
| `run_ledger` | `session_db/` | `list`, `get`, `events` |
| `session_import` | `session_import/` | `run` |
| `agent_work` | [`orchestration/command_center`](./orchestration/command_center/) | `list`, `control` |
| `workflow_run` | [`orchestration/workflow_runs`](./orchestration/workflow_runs/) | `list_definitions`, `list`, `get`, `start`, `stop`, `resume` |
| `agent_team` | [`orchestration/agent_teams`](./orchestration/agent_teams/) | `create`, `list`, `get`, `assign_task`, `claim_task`, `message_member`, `list_messages`, `complete_task`, `shutdown_member`, `close`, `start_member` |
| `worktree` | [`orchestration/worktree_schemas.rs`](./orchestration/worktree_schemas.rs) | `list`, `status`, `diff`, `remove` |
| `subagent` | [`orchestration/subagent_control.rs`](./orchestration/subagent_control.rs) | `cancel`, `steer` for running sub-agents |

`agent.chat` and `agent.chat_simple` load config and call
`inference::host_runtime::rpc::agent_chat` / `agent_chat_simple`, which
build an `OpenHumanSessionHost`. `chat_simple` is a bare provider call with
no tools. The JSON-RPC server itself is in [`crates/openhuman-rpc`](../../../openhuman-rpc/).

## Boundaries

What lives elsewhere:

- The agent loop, tool-call parsing and dialects, sessions, transcripts,
  resume, compaction generations, run queues, steering, the run ledger, the
  sub-agent lifecycle, goals and todos stores, and generic middleware (context
  compression, microcompact, final-call wrap-up, arg recovery) belong to
  [`vendor/tinyagents`](../../../../vendor/tinyagents/) (`tinyagents-runtime`, `-session`, `-harness`,
  `-graph`, `-orchestration`). A bug there is fixed upstream, not worked
  around here.
- The `Tool` trait and tool types are in [`vendor/tinyagents/vendor/tinytools`](../../../../vendor/tinyagents/vendor/tinytools/).
  `ChatModel`, providers and message types are in `tinyinference`.
- Tool implementations are in `crate::tools` and the owning domains. This
  folder only owns the loop-control tools in `tools/` and the orchestration
  tools.
- Provider construction and model routing are in `crate::inference::provider`
  (`factory::provider_for_role`, `create_chat_model_with_model_id`).
- The approval gate and prompt-injection guard are in `crate::security`; this
  folder scopes the origin they read and installs the middleware that calls
  them.
- Memory storage is in `crate::memory`; the agent publishes
  `ConversationTurnCommitted` and reads a memory pack, nothing more.
- The web socket protocol and per-thread session cache are in
  `crate::web_chat`; channel adapters are in `crate::channels`.
- The JSON-RPC server is in `crates/openhuman-rpc`.

## Gotchas

- Task-locals do not cross `tokio::spawn`. `AGENT_TURN_ORIGIN`,
  `ParentExecutionContext`, `with_tool_call_limit` and the turn workspace
  must be re-scoped in any spawned task. An unscoped turn reads origin
  `Unknown` and the approval gate fails closed. Recursive execution passes
  `OpenHumanRunContext` explicitly for this reason.
- The tool surface is all or nothing. An empty visible-tool set is deny-all,
  not "every tool"; `run_chat_turn_graph` deliberately keeps `Some(empty)`.
- A resumed thread's prompt and tools are frozen. Testing a prompt change
  needs a new thread.
- `agent.run_turn` must be registered before channels dispatch.
  `register_agent_handlers` runs from [`channels/runtime/startup/start_channels.rs`](../channels/runtime/startup/start_channels.rs)
  and [`core/runtime/subscribers.rs`](../core/runtime/subscribers.rs); tests that replace the handler restore
  it through the guard in `bus.rs`.
- `stop_hooks::with_tool_call_limit(Some(n), turn)` narrows the TinyAgents
  invocation budget for one awaited turn, including parallel calls. Zero
  permits no tool calls. Nested scopes take the smaller limit, `None` keeps
  the enclosing one, and the limit applies per run, not as an aggregate
  across child runs.
- `journal.rs` is best-effort. A journal write failure logs `[journal]` and
  does not fail the turn.

## Tests

Unit tests sit beside their modules as `*_tests.rs` (for example
[`agent_tests.rs`](./agent_tests.rs), [`bus_tests.rs`](./bus_tests.rs), [`schemas_tests.rs`](./schemas_tests.rs) with
`controller_schema_inventory_is_stable`, and the suites under
`session_host/`, `tinyagents/`, `subagent_host/`, `triage/`). Run them with
`cargo test -p openhuman agent::` or `pnpm debug rust agent::`.

Integration coverage is in [`tests/agent_harness_e2e.rs`](../../../../tests/agent_harness_e2e.rs)
(`cargo test -p openhuman-cli --test agent_harness_e2e`) and in
`tests/in_process/agent_*.rs`, aggregated into the `in_process_all` target.

Related docs:
[agent harness architecture](../../../../gitbooks/developing/architecture/agent-harness.md),
[agent observability](../../../../gitbooks/developing/agent-observability.md).

## Further reading

- [Parent module README](../../README.md)
- [Deep architecture reference](../../../../gitbooks/developing/architecture.md)
- [The orchestrator](../../../../gitbooks/features/orchestration.md)
- [tinyagents](../../../../vendor/tinyagents/README.md)
