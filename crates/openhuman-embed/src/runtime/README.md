# runtime

The first step of the two-step library API: initialize one `Runtime` per
process, then put any number of independently configured agents on it with
`Runtime::agent`. This folder owns the builder that boots the embedded core,
the `Runtime` handle, and the guard that tears the core down when the last
handle goes away. See
[`gitbooks/developing/embedding.md`](../../../../gitbooks/developing/embedding.md)
for the walkthrough and
[`gitbooks/developing/performance.md`](../../../../gitbooks/developing/performance.md)
for what many agents on one runtime cost in memory.

## How it works

`RuntimeBuilder::new()` starts from safe defaults: an ephemeral workspace,
`Provider::inherit()`, `Access::supervised()`, `HostKind::Library`, only the
no background services by default, and `DomainSet::embedded()` plus `mcp` and
`skills` when those features are compiled in. `build()` then runs these steps
in order:

```text
 RuntimeBuilder::build
   |
   |- claim RUNTIME_LIVE (AtomicBool)  -> AlreadyRunning if already set
   |- refuse a blank API key, or Stateless without a session store
   |- install the host memory engine, if given
   |- ResolvedWorkspace::resolve       temp dir / Dir / scratch / Inherit
   |- base Config: Inherit loads the operator's; otherwise supplied or
   |  default, with workspace_dir, action_dir and config_path overridden
   |- backend_url, Access::apply, provider model
   |- store the API key beside config_path (before boot: the scheduler
   |  gate reads the credential store once)
   |- effective_host_kind          Library on an inherited install without
   |                               a host credential becomes Cli
   |- install the session store provider (before boot: recovery runs)
   |- CoreBuilder::new(host_kind).domains.tool_groups.services
   |      .token(EnvOrFile).config(..)[.backend_transport(..)].build()
   |- store the Session through auth, if given
   v
 Runtime { guard: Arc<CoreGuard>, base_config, domains, tool_groups, ... }
```

Any failure after the slot is claimed releases it again, so a failed build
does not poison the process against a retry. The builder never mutates the
process environment.

The resulting `Runtime` keeps the base `Config` every agent starts from, the
registered `DomainSet` and `ToolGroups` (agents may only narrow them), the
default `Provider` and `Access`, and a registry of live agents keyed by id
(held as `Weak` references, so an id frees up when its last clone drops).

`CoreGuard` holds the `Core`, the resolved workspace (and its `TempDir` for
an ephemeral one), and the installed session store. Both `Runtime` and every
`AgentInner` hold an `Arc<CoreGuard>`, so the core stays alive while any
agent or in-flight turn exists, even after the host drops its `Runtime`. When
the last owner drops, the guard drops the core while the process slot is
still claimed, restores the previous session store if it still owns the
slot, removes an ephemeral workspace with a short retry loop (background
writers can recreate subdirectories just after a turn returns), and only then
releases `RUNTIME_LIVE`.

## Layout

| File | What it does |
| --- | --- |
| [`mod.rs`](mod.rs) | `Runtime`, `RuntimeError`, `CoreGuard` and the `RUNTIME_LIVE` process slot. |
| [`builder.rs`](builder.rs) | `RuntimeBuilder`, its knobs and `ConfigSource` (`Resolved`: embed hands the core a config; `Discovered`: the core loads the operator's install, as desktop/CLI/TUI do). |
| [`build.rs`](build.rs) | `RuntimeBuilder::build`: validation, config resolution, seam install, `CoreBuilder` boot; `apply_provider`, `effective_host_kind`. |
| [`presets.rs`](presets.rs) | Host presets `library`/`desktop`/`cli`/`tui` and the library defaults `default_domains`, `default_services`. |
| [`seams.rs`](seams.rs) | Process-global seams as builder options (`controller_extension`, `tool_ranker`, `post_turn_hook`, `tool_hook`, `server_launcher`, `live_policy`) and the guard that restores the restorable ones on drop. |
| [`run.rs`](run.rs) | `RuntimeBuilder::run_from_args` / `run_from_args`: install the builder's process-global pieces for the process, then run the core's CLI dispatcher. |
| [`api_key.rs`](api_key.rs) | `ApiKey`, a newtype over the TinyHumans key whose `Debug` does not print it. |

## Key types and entry points

- `RuntimeBuilder` ([`builder.rs`](builder.rs)): `workspace`, `workspace_dir`,
  `action_dir`, `config_source`, `api_key`, `backend_url`,
  `backend_transport`, `memory_engine`, `session_store`, `provider`,
  `access`, `services`, `domains`, `tool_groups`, `host_kind`, `token`,
  `listen` (`listen_host`, `listen_port`), `session`, `config`, the seam
  options ([`seams.rs`](seams.rs)), then `build` or `run_from_args`. Presets
  ([`presets.rs`](presets.rs)): `library()` (= `new()`), `desktop()`, `cli()`, `tui()`.
- `Runtime` ([`mod.rs`](mod.rs)): `agent(spec)` builds an agent; `agent_ids()` lists
  the live ones; `core()` returns the narrow `HarnessCore` (config, auth);
  `memory(root)` returns one tenant's `memory::Memory`; `root_dir()` and
  `workspace_dir()` give paths; `domains()`, `tool_groups()` and `services()`
  report what was registered; `has_api_key()` reads the credential store
  live, so it reflects `Auth::store_api_key` and `Auth::clear_api_key` calls
  made after build; `runtime_id()` is an opaque per-instance id.
- `RuntimeError` (`mod.rs`): `AlreadyRunning`, `BlankApiKey`,
  `NoSessionStore`, `Build` (the core failed to boot), `Workspace` (an I/O
  failure), `Invalid`, and `Call` (storing a session failed).

## What is runtime-wide versus per-agent

The runtime owns everything process-scoped in the core: the event bus, the
keyring and credential store, the RPC bearer, the background `ServiceSet`,
the registered `DomainSet`, and the API key. An agent owns everything the
core reads through its own context: its `Config` (provider route and model,
MCP servers, autonomy tier, `action_dir`), its `AgentDefinition` (system
prompt, tool scope, sandbox mode), its skills root, its narrowed
`DomainSet` and `ToolGroups`, its approval settings and parked approvals,
its sub-agents, its MCP host, its cron jobs and its per-turn state. The
crate README's "Still process-owned" list names what agents share, and its
"Lifecycle" section covers `max_agents` and `Runtime::remove_agent`.

## Boundaries

- Booting the core (`CoreBuilder`, `CoreContext`, service startup) belongs to
  `openhuman_core::core::runtime`. See its
  [README](../../../openhuman-core/src/core/runtime/README.md).
- No backend transport is installed here. `openhuman-tinyhumans`'s
  `RuntimeBuilder` ([`crates/openhuman-tinyhumans/src/runtime.rs`](../../../openhuman-tinyhumans/src/runtime.rs)) wraps this
  builder and installs the SDK transport on `build()`; see
  [`gitbooks/developing/tinyhumans-api-key.md`](../../../../gitbooks/developing/tinyhumans-api-key.md).
- The session store port (`SessionStoreProvider`) is defined by
  [`vendor/tinyagents`](../../../../vendor/tinyagents/) and the core's `agent::session_store`; the classic
  on-disk provider lives in `openhuman-rpc`'s `session_store`.

## Gotchas

- One runtime per process. `CoreContext::init` seeds the process-scoped
  state once, so `build` returns `RuntimeError::AlreadyRunning` rather than
  letting two runtimes share a keyring and event bus while each believes it
  owns a separate workspace. Agents are the unit of multiplicity.
- The tokio runtime is the host's job. Build it with
  `AGENT_WORKER_STACK_BYTES` and `MAX_BLOCKING_THREADS`
  (`openhuman_core::core::runtime`) or a turn with a nested sub-agent
  overflows the default 2 MiB worker stack.
- The provider's route is never written into the config; only its model is
  (`apply_provider`). Config routes persist, and a library caller must not
  repoint the operator's install.
- `memory_engine` and `session_store` are process-wide, like the runtime.
  The session store is removed with the runtime.

## Tests

[`builder_tests.rs`](builder_tests.rs) and [`api_key_tests.rs`](api_key_tests.rs) sit beside their modules. The
end-to-end suites in [`../../tests/`](../../tests/README.md) build real
runtimes.

```bash
cargo test -p openhuman-embed --features inference,mcp,skills runtime::
```

## Further reading

- [`gitbooks/developing/embedding.md`](../../../../gitbooks/developing/embedding.md): embedding the core in another product.
- [`gitbooks/developing/tinyhumans-api-key.md`](../../../../gitbooks/developing/tinyhumans-api-key.md): running on a TinyHumans API key.
- [`crates/openhuman-embed/README.md`](../../README.md): the openhuman-embed crate README.
