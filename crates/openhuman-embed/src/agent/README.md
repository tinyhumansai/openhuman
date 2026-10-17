# agent

An agent is one fully described, independently configured participant on a
`Runtime`. This folder holds the pure-data `AgentSpec` a host writes, the
build step that turns it into a live `Agent`, and the handle the host keeps.
`Runtime::agent` is the only caller of the build step; hosts call `Agent::run`
and `Agent::turn`. See
[`docs/gitbooks/en/developing/embedding.md`](../../../../docs/gitbooks/en/developing/embedding.md)
for how agents fit into the two-step library API.

## How it works

`AgentSpec::new(id)` starts with every setting unset, meaning "use the
runtime's default". Builder methods fill in the provider, access, definition,
MCP servers, skills, `action_dir`, trusted paths, Composio credential, memory
binding, a config escape hatch and a tool factory. A spec does nothing on its
own.

`Runtime::agent(spec)` holds the runtime's agent registry lock (so two calls
with the same id cannot both pass the duplicate check), then calls
`build::instantiate`. That function runs in a fixed order:

```text
 AgentSpec
   |
   v
 1. validate id            ^[a-z0-9][a-z0-9_-]{0,63}$
 2. config = runtime base config (clone)
 3. action_dir             spec value, else AgentLayout::default_action_dir
 4. access.apply           tier, trusted roots (replaced, not merged)
 5. provider model         route stays per turn, never written to config
 6. MCP servers            appended to mcp_client.servers   (mcp feature)
 7. Composio credential    pinned for this agent only
 8. memory binding         memory.agent_id and memory.root
 9. config escape hatch    then config_path is reset to the runtime's
10. create dirs            action_dir, agents/<id>/, agents/<id>/skills/
11. copy skills            (skills feature)
12. definition             AgentDefinitionSpec -> core AgentDefinition
13. narrowing checks       DomainSet and ToolGroups may only shrink
14. CoreContext::derive_with(ContextOverlay { config, domains, ... })
   |
   v
 AgentInner  (shared by every Agent clone and every Turn it issues)
```

The `action_dir` is read back from the config after step 9, so a `config`
closure that moves it gets the directory it asked for. When MCP servers are
declared and the definition uses a named tool belt, step 12 adds
`tool_search` to the belt, because MCP tools are registered as deferred
`mcp_<server>_<tool>` entries that a named belt can only reach through
discovery.

The derived `CoreContext` is what makes agents independent. Every turn runs
inside it (`CoreRuntime::run_in`), so the core's config loader, `DomainSet`
gate, tool-group filter and skill discovery read this agent's overlay and no
other. The overlay also carries `session_agent`, which a host session store
uses to keep each agent's conversations apart.

`AgentInner` holds an `Arc<CoreGuard>`, so the core (and an ephemeral
workspace) stays alive while any agent handle or in-flight `Turn` exists,
even after the host drops its `Runtime`. The id is released for reuse once
the last clone of an agent is dropped. Dropping an agent does not delete its
directories; transcripts and memory persist with the workspace.

## Layout

| File | What it does |
| --- | --- |
| [`mod.rs`](mod.rs) | `Agent`, the cheap clone handle (`run`, `turn`, `id`, `action_dir`, `workspace_dir`, `home_dir`, `skills_dir`, `transcripts_dir`, `provider`, `access`, `config`), the shared `AgentInner`, and `AgentError`. |
| [`spec.rs`](spec.rs) | `AgentSpec`, the builder a host fills in, and `MemoryBinding`. |
| [`definition.rs`](definition.rs) | `AgentDefinitionSpec`, `ToolScopeSpec` and `SandboxModeSpec`: a small re-spelling of the core's `AgentDefinition`. |
| [`layout.rs`](layout.rs) | `AgentLayout`: where one agent's home, skills, transcripts and `action_dir` live. |
| [`build.rs`](build.rs) | `instantiate`, plus `check_domains_narrow` and `check_tool_groups_narrow`. |
| [`attachments.rs`](attachments.rs) | `Agent::attach_tools`, `runtime_id`, `same_agent`, `ToolAttachmentError`, and the policy that composes attached sources with the agent's own. |

## Key types

- `AgentSpec` ([`spec.rs`](spec.rs)): the description. `system_prompt` and `definition`
  set what the agent is; `provider`, `model` and `access` set where it runs
  and with what authority; `tool_groups` and `domains` narrow its surface;
  `mcp` and `skills_dir` add servers and skills; `include_user_skills(true)`
  lets it see the operator's `~/.openhuman/skills` (off by default);
  `action_dir` and `trust` set its filesystem reach; `composio` pins its own
  Composio key and entity; `memory` binds its memory; `config` is the escape
  hatch; `tools` is the per-turn tool factory.
- `AgentDefinitionSpec` ([`definition.rs`](definition.rs)): the default is the built-in
  orchestrator's definition under the agent's own id, except that every
  registered tool is visible (`ToolScopeSpec::Wildcard`). Narrow with
  `tools(ToolScopeSpec::Named(..))` or `disallow_tools`. Also sets
  `sandbox` (`None`, `ReadOnly`, `Sandboxed`), `max_iterations`,
  `temperature`, `display_name` and `when_to_use`.
- `MemoryBinding` (`spec.rs`): the memory agent id the turns are logged
  under, and optionally a layout root such as `team:acme`.
- `AgentLayout` ([`layout.rs`](layout.rs)): `home` is `<workspace>/agents/<id>/`, `skills`
  is `<workspace>/agents/<id>/skills/`, `transcripts` is
  `<workspace>/session_raw/`, and `action_dir` is the resolved acting root.
- `AgentError` ([`mod.rs`](mod.rs)): `DuplicateId`, `InvalidId`, `WidensRuntime`,
  `Workspace` (an I/O failure laying out directories), `Invalid`, and `Call`
  for a failed turn.

## Tool factories

`AgentSpec::tools` supplies a host's own in-process tools when an agent is
created. Import `Tool`, `ToolResult` and `ToolPolicy` from `openhuman_embed`;
`Tool::policy` declares execution requirements using the same vendored contract
as the core. Application-specific executors belong to the embedding host.
Each tool is a real tool with its own schema on the wire, unlike a
tool reached through an MCP server's `mcp_call_tool` envelope. The factory
runs once per session build, which in practice is once per turn: an `Agent`
is `Clone` and `Box<dyn Tool>` is not, so a stored belt could not survive the
rebuild. The factory receives a `TurnContext` with the agent id and the
session id the caller named, so a host whose tools are bound to one episode
or room can return a different belt per conversation.

The prompt's tool catalogue is rendered from the same belt in the same
build. A resumed session reuses its persisted system messages, though, so a
belt that changes under a long-lived thread is still described by the prompt
that thread opened with. Vary a belt only on turns that run in a session of
their own.

## Attach tools to an existing agent

`Agent::attach_tools(key, factory)` permanently adds a named `HostTools`
source without rebuilding the agent. All clones share the attachment.
Attaching the same key with the same factory `Arc` again is a no-op; another
factory under that key returns `ToolAttachmentError::SourceConflict`, and a
tool name that collides with one already registered returns
`NameCollision`. The factory is sampled once without a session at
registration to learn its tool names, then runs per turn. Keep its tool names
stable and make it work for that registration call.

Attached tools always get direct provider schemas and their own system
catalogue section, including tools that declare themselves deferred. Adding a
source to an existing conversation updates only that managed section on the
next turn; the configured prompt, skills, memory and other frozen system
sections stay as they were. TinyAgents seals the prior transcript generation
and writes a successor with the conversation preserved. An identical
catalogue does not create a new generation.

Attachments do not replace the agent's original tool gate. Each source's
policy applies to its own tool names. A source without a policy admits its
tools, so their callbacks must authorize their own operations. Every other
tool keeps the agent's original policy, including its denials.

Use `runtime_id()` to check that supplied agents belong to one runtime and
`same_agent()` to tell an existing handle from a different agent with a
conflicting id. Runtime ids are opaque and valid only for that runtime's
lifetime; they are not persistence keys.

See the runnable [host_tools example](../../examples/host_tools.rs), region `host_tools`. It uses the public facade and asserts its behavior against an offline stub.

## Boundaries

- What a turn does once dispatched (the agent loop, transcripts, tool-call
  parsing) belongs to [`vendor/tinyagents`](../../../../vendor/tinyagents/); the definition types and
  `agent_chat_for` belong to the core (`openhuman_core::agent::harness` and
  `inference::host_runtime`).
- Process-scoped state (keyring, bus, services, the registered domain set)
  belongs to the runtime. See [`../runtime/README.md`](../runtime/README.md)
  and the crate README's list of settings that are still runtime-wide.
- `Access`, `Provider` and `McpServer` are defined in
  [`../harness/`](../harness/README.md) and shared with the one-agent
  `Harness`.

## Gotchas

- `Access::apply` replaces the trusted roots; it does not add to the
  runtime's. A `readonly()` agent on a runtime whose default trusts extra
  paths trusts none of them.
- The `config` closure cannot move `config_path`. It is reset to the
  runtime's after the closure runs, because every agent shares one
  credential store and keyring.
- An agent cannot widen the runtime. Asking for a domain family the runtime
  did not register, or a tool group more exposed than the runtime's
  (`Off` < `Withheld` < `Advertised`), fails with `AgentError::WidensRuntime`.
- Avoid built-in agent ids such as `orchestrator` and `summarizer`; the
  runtime-wide delegation catalogue resolves those to the shipped
  definitions.

## Tests

[`build_tests.rs`](build_tests.rs), [`definition_tests.rs`](definition_tests.rs), [`layout_tests.rs`](layout_tests.rs), [`spec_tests.rs`](spec_tests.rs)
and [`attachments_tests.rs`](attachments_tests.rs) sit beside their modules. The end-to-end proof of
agent isolation is [`tests/runtime_agents.rs`](../../tests/runtime_agents.rs), and attachments are covered by
[`tests/attached_tools.rs`](../../tests/attached_tools.rs).

```bash
cargo test -p openhuman-embed --features inference,mcp,skills agent::
cargo test -p openhuman-embed --features inference,mcp,skills --test attached_tools
```

## Further reading

- [`docs/gitbooks/en/developing/embedding.md`](../../../../docs/gitbooks/en/developing/embedding.md): embedding the core in another product.
- [`docs/gitbooks/en/developing/architecture/agent-harness.md`](../../../../docs/gitbooks/en/developing/architecture/agent-harness.md): the agent harness.
- [`crates/openhuman-embed/README.md`](../../README.md): the openhuman-embed crate README.
