# OpenHuman Embed

A typed Rust facade for running the OpenHuman core inside your application: one Runtime per process, with independently configured Agents.

Read the [embedding documentation](https://github.com/tinyhumansai/openhuman/blob/main/docs/gitbooks/en/developing/embed/README.md), [installation](https://github.com/tinyhumansai/openhuman/blob/main/docs/gitbooks/en/developing/embed/installation.md), and [quickstart](https://github.com/tinyhumansai/openhuman/blob/main/docs/gitbooks/en/developing/embed/quickstart.md).

The [cookbook](https://github.com/tinyhumansai/openhuman/blob/main/docs/gitbooks/en/developing/embed/cookbook.md) is generated from executable examples. Generate complete local API docs with `cargo doc -p openhuman-embed --all-features --no-deps --open`; this workspace crate is currently unpublished.

Hosts supply transport, credentials and application resources. Runtime settings establish shared defaults; agents narrow provider, access, prompt and tool behavior. Use ProfileRuntime when users require separate credentials and workspaces.

## Cancelling one turn

Acquire `Turn::cancellation_handle()` before sending a turn. The handle is
cloneable and `cancel().await` waits for the turn to stop and for tracked
commands to be reaped. The agent remains available for later turns:



Cancellation is scoped to this turn, including while waiting for inference.
Turns with a cancellation handle use owned interpreter subprocesses rather
than the Node/Python pool, which has no acknowledged per-job abort API. This
trades warm-worker reuse for awaited cleanup; ordinary turns retain pooling.
The `meter` callback fires once on cancellation or a dropped send future,
with `None` when dispatch has not supplied usage yet.

Before send, cancellation prevents dispatch; after completion it is a no-op.
On Unix, the built-in shell, Node, Python and npm commands kill their process
group, including descendants. Other platforms stop the direct command. Host
tools that spawn independent tasks or processes must provide their own cleanup;
MCP server lifecycles remain owned by the agent. Keep polling `send()` while
awaiting cancellation, for example in a spawned task.

## Observing provider dispatches

Attach `Completer::physical_observer` to receive one content-free terminal event
for each admitted provider `ChatModel` invocation. Structured JSON repairs
produce separate events, while request and response content remains private.
Transport-level compatibility retries performed inside one provider invocation
remain within that event, and its latency includes those retries. With a model
budget, the explicit and turn-scoped observers run inside admission and after
gateway charge normalization, so they see the same authoritative amount the
ledger settles. A budget refusal before provider invocation creates no event.
The existing `CompletionObserver` remains a once-per-logical-call callback.

## Steering one turn

Acquire `Turn::steering_handle()` before sending a turn to add messages to it
while it runs. The handle is cloneable and `Send + Sync`; `steer(text)` queues
a user message that the agent loop applies at its next safe checkpoint:
before the first model call, after a tool batch has settled, or when the model
has just written its final answer. In that last case the draft stays on the
transcript, the steer follows it, and the model is called once more, so a
message that arrives during a plain chat turn's only model call still shapes
that turn's reply. Nothing interrupts a model call or a tool already running.
The model sees each steer as `[User steering message]: <text>`.

Nothing is dropped silently. Steers the turn ended without applying come back
on `TurnOutcome::unapplied_steers`, or from `TurnSteering::take_unapplied()`
when the turn failed or was cancelled. A steer sent after the turn ended is
refused with `SteerRefused`, which carries its text (`into_text()`). The host
decides whether to run what came back as the next turn.

## Scoped worker hooks

Hooks can be supplied at three levels: `RuntimeBuilder::tool_hook` /
`post_turn_hook` for all agents, `AgentSpec::tool_hook` / `post_turn_hook`
for one agent, and `Turn::tool_hook` / `post_turn_hook` for one dispatch.
Tool callbacks run in that order. Named agent updates replace only that agent’s callback; per-turn callbacks are additional.
`ToolHookContext` carries the agent/session identity when known, and `cwd`
follows the execution workspace descriptor (including `Turn::cwd`), falling
back to the embedding context's configured action root.
The agent and turn hooks are never installed in the global registry, so
concurrent workers and later turns do not pick up one another's callbacks.
Post-turn callbacks run asynchronously with an owned session snapshot.
Independently spawned tasks that build sessions must explicitly inherit
`tinyagents_hooks::scope::HookScope` to carry scoped hooks.

Gateway attribution headers can be attached to `Route::header(name, value)`
and used with `Turn::route`, or with `Provider::routed(route)` on an agent.
They follow only that route's endpoint and are never saved to configuration,
sent to background providers, or included as values in `Route`'s `Debug`.

### Inline permission and usage policy

`AgentSpec::can_use_tool` and `Turn::can_use_tool` await a host callback before
executing each tool. The callback can wait for an approval UI and return
`ToolHookDecision::Proceed`, `Deny`, or `ProceedWith`. It owns that wait;
returning `Ask` denies execution. These callbacks add to existing tool policies,
and a turn callback cannot override an agent denial.

`AgentSpec::stop_hook` and `Turn::stop_hook` receive cumulative usage after each
completed model call. Return `StopDecision::Continue` to observe usage, or
`Stop` to prevent subsequent calls. Completed tool rounds may still execute;
this is an after-call budget boundary, so hosts must refuse an already exhausted
budget before sending a turn. Provider-reported charges remain authoritative,
including known zero; missing charges remain unknown unless pricing is known.
A policy stop uses a deterministic partial summary rather than spending on
final-answer repair calls.

### Per-turn tools and subprocess environment

`Turn::tools` replaces this turn's host tool belt, including attached sources.
An empty belt revokes host tools; the next turn returns to the agent's belt.
Builtin tools still follow the agent definition. This supplies dynamic tools for
in-process hosts; statically declared MCP servers retain their creation-time
configuration.

`Turn::tool_env` supplies the base environment of owned builtin subprocesses.
Variables absent from it are not inherited from the daemon. The builtin command
builders retain their own security/runtime additions, including Git restrictions,
managed interpreter paths and scratch directories. Scoped turns bypass Node and
Python pools, which cannot acknowledge per-job cancellation or swap a job's
process environment. A host tool that spawns a separate Tokio task must explicitly
carry the command environment and cleanup scopes into that task.

Standalone exact source pins and generated Cargo patches: [consumer setup](CONSUMERS.md).
Ordered fallbacks and required exploration: [routing](ROUTING.md).
Host telemetry and the existing exporter: [observers](OBSERVERS.md).
