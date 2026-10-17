# harness

`Harness` is the one-call front door over the library API: one `Runtime`
plus exactly one `Agent` with the id `harness`, built from a single set of
inputs. This folder also defines the input types that `Runtime` and
`AgentSpec` share with it: `Access`, `Provider`, `Workspace` and `McpServer`.
Reach for `Runtime` and `AgentSpec` directly once a host needs more than one
agent. See
[`docs/gitbooks/en/developing/embedding.md`](../../../../docs/gitbooks/en/developing/embedding.md)
for the walkthrough.

## How it works

`HarnessBuilder::build` is a thin composition over the other two layers:

```text
 HarnessBuilder
   |
   |- refuse skills_dir with Workspace::Inherit   (before claiming the slot)
   |- pick domains: DomainSet::embedded(), plus mcp only if servers were
   |  given and skills only if a skills_dir was given
   |
   |- RuntimeBuilder::new()
   |     .workspace .host_kind .provider .access .domains
   |     [.config .backend_url .services .tool_groups .session]
   |     .build()                                   -> Runtime
   |
   |- AgentSpec::new("harness")
   |     .provider .access [.tools]
   |     .action_dir(the workspace's own action/ dir, unless Inherit)
   |     [skills copied into <workspace>/skills] [.mcp per server]
   |  runtime.agent(spec)                           -> Agent
   v
 Harness { agent, runtime }
```

Every `Harness` method delegates. `run` and `turn` go to the agent,
`workspace_dir` and `core` go to the runtime, and `runtime()` and `agent()`
hand out the two halves so a host can add more agents with
`Runtime::agent`. On drop the agent is released first, then the runtime, so
the core and an ephemeral workspace are torn down with nothing still holding
them.

`Harness::core()` returns `HarnessCore`, a deliberately narrow facade with
only `config()` and `auth()`. It exposes neither the raw `CoreRuntime` nor
the orchestrator facade, because either could start a turn that skips the
agent's provider route and access tier. `Runtime::core()` returns the same
type.

## Layout

| File | What it does |
| --- | --- |
| [`mod.rs`](mod.rs) | `Harness` (`builder`, `run`, `turn`, `runtime`, `agent`, `core`, `workspace_dir`, `action_dir`) and `HarnessCore`. |
| [`builder.rs`](builder.rs) | `HarnessBuilder` and `HARNESS_AGENT_ID`. Inputs: `workspace`, `action_dir`, `provider`, `access`, `skills_dir`, `mcp`, `tools`, `services`, `domains`, `tool_groups`, `host_kind`, `backend_url`, `session`, `config`. |
| [`access.rs`](access.rs) | `Access`: the autonomy tier and the turn origin, set together. |
| [`provider.rs`](provider.rs) | `Provider`: which model answers and where the request goes. |
| [`workspace.rs`](workspace.rs) | `Workspace` and the crate-private `ResolvedWorkspace` that materializes directories. |
| [`mcp.rs`](mcp.rs) | `McpServer`, `HttpHeader`, `McpAuthConfig` (`mcp` feature). |
| [`skills.rs`](skills.rs) | Copies skill bundles into a skills root (`skills` feature). |
| [`error.rs`](error.rs) | `HarnessError`: `Workspace`, `Invalid`, `Build`, `Call`, `AlreadyRunning`. |

## The shared input types

`Access` closes a trap. Access is governed by two independent mechanisms:
the autonomy tier (`config.autonomy.level`) drives `SecurityPolicy`, and the
turn origin (a task-local `AgentTurnOrigin`) is what the fail-closed approval
gate reads. Setting only the tier produces an agent whose `shell`, `edit` and
`apply_patch` calls refuse while the model narrates around the refusals.
`Access` always carries both halves:

| Preset | Tier | Origin | Approval gate |
| --- | --- | --- | --- |
| `Access::readonly()` | `ReadOnly` | none | on |
| `Access::supervised()` (default) | `Supervised` | none | on |
| `Access::full()` | `Full` | `TrustedAutomation` (workflow, no approval) | off |

`trust(path, TrustedAccess)` adds a trusted root outside the action
directory (credential stores such as `~/.ssh` stay blocked whatever is
granted), `allow_tool_install(true)` permits the `install_tool` tool (off in
every preset), and `origin(..)` overrides the turn origin for a caller with a
narrower authority. `full()` does not set `auto_approve_all`; the origin is
the instrument the gate reasons about.

`Provider` is a per-turn route, never a config write. Writing
`inference_url` or `api_key` into the config would persist it and repoint the
operator's install, so `Provider::openai_compatible(base_url, key)` compiles
to the core's `EphemeralRoute`, a field that is never serialized. The route
pins the chat, reasoning, agentic and coding roles and leaves background
roles (memory, embeddings, learning) alone. Pass the API root
(`https://host/v1`); `/chat/completions` is appended. `Provider::model` is
advisory (an unknown model falls back to the default), and a route with no
model resolved registers nothing. `Provider::inherit()` uses whatever the
machine's configuration resolves to.

`Workspace` chooses where state lives:

| Variant | Meaning |
| --- | --- |
| `Ephemeral` (default) | A temp directory (`openhuman-harness-*`), removed when the last handle drops. |
| `Dir(path)` | A caller-owned directory, created if absent. `config.toml` and `action/` sit beside it. |
| `Stateless` | No durable state; requires `RuntimeBuilder::session_store`. A scratch temp dir (`openhuman-scratch-*`) holds process-local caches. |
| `Inherit` | The machine's own OpenHuman workspace, resolved by `Config::load_or_init`. |

For the runtime-owned variants `ResolvedWorkspace::resolve` lays out a
`workspace/` directory, a sibling `action/` directory and a sibling
`config.toml`. The sibling layout matters twice: the core refuses agent
writes beneath the workspace, and the credential store, auth profiles and
keyring file resolve against `config.toml`'s parent, which is what keeps an
ephemeral harness from touching `~/.openhuman`.

`McpServer::stdio(name, command, args)` and `McpServer::http(name, endpoint)`
compile to the same `McpServerConfig` a `[[mcp_client.servers]]` block parses
into, with `env`, `cwd`, `auth`, `allow_tools`, `deny_tools`, `timeout_secs`
and `description` refinements. The config is the registration API for the
core's static MCP registry, so servers are fixed when the agent is built.

## Running on your own endpoint

A harness identifies as `HostKind::Library` by default, so a `Provider` is
enough for inference: the library host brings its own endpoint and
credentials, without an OpenHuman app login. Managed TinyHumans inference
needs the runtime's API key instead (`RuntimeBuilder::api_key`; see
[`docs/gitbooks/en/developing/tinyhumans-api-key.md`](../../../../docs/gitbooks/en/developing/tinyhumans-api-key.md)).
The core can still make non-inference backend calls; `backend_url` points
them at the embedding product's backend, and `session` installs a backend
identity when one is required.

Neither applies to `Provider::inherit()` with `Workspace::Inherit`, which
runs exactly as the installed app does, session included. In that case a
requested `HostKind::Library` is downgraded to `HostKind::Cli` unless the
host supplied a usable route or an API key, so borrowing an install does not
bypass its session gate.

## Boundaries

- The runtime and agent mechanics live in [`../runtime/`](../runtime/README.md)
  and [`../agent/`](../agent/README.md); this folder only composes them.
- Security enforcement (`SecurityPolicy`, the approval gate, path checks)
  is the core's (`openhuman_core::security`). `Access` only chooses inputs.
- MCP transport and the server registry belong to the core's MCP domain and
  [`vendor/tinymcp`](../../../../vendor/tinymcp/).

## Gotchas

- The tokio runtime is the host's, and the default 2 MiB worker stack
  overflows on a turn with a nested sub-agent. Build it with
  `AGENT_WORKER_STACK_BYTES` and `MAX_BLOCKING_THREADS`
  (`openhuman_core::core::runtime`).
- One runtime per process. A harness owns one, so a second
  `HarnessBuilder::build` returns `HarnessError::AlreadyRunning`. Add agents
  to the existing runtime instead.
- `skills_dir` with `Workspace::Inherit` is refused with
  `HarnessError::Invalid`: the harness copies into `<workspace>/skills`,
  which there is the operator's own. `AgentSpec::skills_dir` copies into the
  agent's own `agents/<id>/skills/` and has no such restriction.
- Skill bundles are copied, not linked. Discovery skips symlinked bundles
  and symlinked manifests as a security control, so a link would leave the
  skills silently absent. A bundle is a directory holding `WORKFLOW.md`,
  `SKILL.md` or `skill.json`; `skills_dir` accepts either one bundle or a
  directory of them.
- `Access::supervised()` keeps the approval gate on. An unattended agent
  parks risky calls and they deny after ten minutes. Use `Access::full()` for
  automation.

## Tests

Unit tests sit beside each module ([`access_tests.rs`](access_tests.rs), [`builder_tests.rs`](builder_tests.rs),
[`provider_tests.rs`](provider_tests.rs), [`workspace_tests.rs`](workspace_tests.rs), and others). The end-to-end proof
that a harness runs a real turn against a `wiremock` provider is
[`tests/harness_embed.rs`](../../tests/harness_embed.rs).

```bash
cargo test -p openhuman-embed --features inference,mcp,skills harness::
cargo test -p openhuman-embed --features inference,mcp,skills --test harness_embed
```

## Further reading

- [`docs/gitbooks/en/developing/embedding.md`](../../../../docs/gitbooks/en/developing/embedding.md): embedding the core in another product.
- [`docs/gitbooks/en/developing/architecture/agent-harness.md`](../../../../docs/gitbooks/en/developing/architecture/agent-harness.md): the agent harness.
- [`crates/openhuman-embed/README.md`](../../README.md): the openhuman-embed crate README.
