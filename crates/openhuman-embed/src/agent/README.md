# agent

One fully described, independently configured participant on a `Runtime`.
See [`gitbooks/developing/embedding.md`](../../../../gitbooks/developing/embedding.md)
for how agents fit into the two-step library API.

## Contents

- `mod.rs`: `Agent`, the cheap-to-clone handle a host holds: `run()` /
  `turn()` to talk to it, `action_dir()` / `workspace_dir()` / `home_dir()` /
  `skills_dir()` / `transcripts_dir()` for its paths, `provider()` /
  `access()` / `config()` for inspection. `AgentInner` is the shared state
  behind every clone, and `AgentError` covers the ways building or driving an
  agent can fail (duplicate id, invalid id, widening the runtime's surface, a
  workspace I/O failure).
- `spec.rs`: `AgentSpec`, the pure-data description a host builds before an
  agent exists: id, `AgentDefinitionSpec`, provider, access, tool groups,
  domains, MCP servers (`mcp` feature), a skills directory (`skills`
  feature), `action_dir`, trusted paths, an escape-hatch config closure, and
  an optional per-turn tool factory (`tools()`). `Runtime::agent` applies
  these, in order, onto a clone of the runtime's base config.
- `definition.rs`: `AgentDefinitionSpec`, `ToolScopeSpec` and
  `SandboxModeSpec`: a thin re-spelling of the core's `AgentDefinition` so
  the library surface does not track that struct field by field. The default
  is the built-in orchestrator's definition under the agent's own id, with
  every registered tool visible.
- `layout.rs`: `AgentLayout`: where one agent's files live under the
  runtime's workspace (`agents/<id>/` for its home and skills,
  `session_raw/` for transcripts, plus its resolved `action_dir`).
- `build.rs`: `instantiate`, the function that turns an `AgentSpec` plus a
  `Runtime` into the assembled `AgentInner` (lays out directories, copies
  skills, builds the `Config`, derives the `CoreContext`).

## Isolation between agents

Two agents on one runtime never read each other's settings. Each turn
dispatches under its own derived `CoreContext`, so the core's config loader,
`DomainSet` gate, tool-group filter and skill discovery all read the agent
that turn belongs to, not any other agent sharing the runtime.

## Tool factories

`AgentSpec::tools` supplies a host's own in-process tools when an agent is
created. `Agent::attach_tools` adds permanent tools to an existing agent. The factory runs once per session build,
which in practice means once per turn, because a session is rebuilt from the
spec's data every time; see the doc comment on `AgentSpec::tools` for why
this is forced (an `Agent` is `Clone`, `Box<dyn Tool>` is not) and useful (a
belt bound to something shorter-lived than the agent can vary per turn).

## Where to look next

- [`../runtime/README.md`](../runtime/README.md): what instantiates an
  agent and what stays runtime-wide instead of per-agent.
- [`../harness/README.md`](../harness/README.md): the one-runtime,
  one-agent shorthand.

## Attach tools to an existing agent

`Agent::attach_tools(key, factory)` permanently adds a named `HostTools`
source without rebuilding the agent. All clones share the attachment. Reusing
both the source key and the same factory `Arc` is idempotent; another factory
under the same key or a colliding tool name returns `ToolAttachmentError`.
The factory is sampled without a session during registration, then runs per
turn. Keep its tool names stable and support the registration occasion.

Attached tools always have direct provider schemas and a separate system
catalogue, including tools originally declared deferred. Adding a source to
an existing conversation updates only that managed section on the next turn:
the configured prompt, skills, memory, and other frozen system sections stay
intact. TinyAgents seals the prior transcript generation and writes a successor
with the conversation preserved. Identical catalogues do not create generations.

Attachments do not replace the original host's tool gate. Each source's policy
applies to its own tool names; absent a source policy, those callbacks are
admitted and must perform their own operation authorization. Other tools keep
the agent's original policy, including its denials.

Use `runtime_id()` to check that supplied agents belong to one runtime and
`same_agent()` to distinguish an existing handle from a conflicting agent ID.
Runtime identities are opaque and valid for that runtime's lifetime; they are
not persistence keys.

```rust,no_run
use std::sync::Arc;
use openhuman_embed::{Agent, HostTools, HostTurnTools, Tool};
# fn connect(agent: &Agent, make_tools: impl Fn() -> Vec<Box<dyn Tool>> + Send + Sync + 'static) -> Result<(), openhuman_embed::ToolAttachmentError> {
let source: HostTools = Arc::new(move |_| HostTurnTools::advertised(make_tools()));
agent.attach_tools("tinyhivemind", source.clone())?;
agent.clone().attach_tools("tinyhivemind", source)?;
# Ok(())
# }
```
