# core/runtime

The embeddable composition surface: `CoreBuilder` → `CoreRuntime`. This is
what `openhuman-embed`'s `Harness` and the standalone `openhuman-core`
binary both build on to host the core without duplicating
`run_server_inner`.

## Two-phase model

1. **`CoreBuilder::build()`: initialization only.** Registers controllers,
   loads the master key, seeds the RPC bearer, initializes the
   workspace-bound stores, and runs the pure-registration part of
   `bootstrap_core_runtime`. No port is bound. After `build()` returns,
   `CoreRuntime::invoke` can dispatch any RPC method in-process and agent
   turns can run: a harness-only embedder using `ServiceSet::none()` needs
   nothing more.
2. **`openhuman_rpc::server::serve(&runtime, …)`: transport + background
   services.** Binds the HTTP listener (when `ServiceSet::rpc_http` is set),
   mounts the router, fires the readiness signal, calls
   `CoreRuntime::start_services`, and serves until shutdown, then runs
   `CoreRuntime::exit_cleanup`. When `rpc_http` is unset it just starts the
   selected background services and returns: the caller owns the process
   lifetime. The server lives in `openhuman-rpc`, above this crate; a
   runtime that needs no transport calls `start_services` directly.

`CoreRuntime::invoke(method, params)` dispatches an RPC method in-process
through `jsonrpc::invoke_method`: the same path the HTTP `/rpc` handler and
the CLI use, wrapped in `CoreContext::scope` so the ambient context resolves
correctly. `openhuman_embed::Harness` goes through this call, never through
domain operations directly.

## `ServiceSet`: which background services and transports run

Each flag is independent. Presets:

| Preset | Shape |
| --- | --- |
| `ServiceSet::desktop()` | Everything on: the Tauri shell and standalone `openhuman-core run`. |
| `ServiceSet::headless_api()` | HTTP JSON-RPC only (`rpc_http`): no Socket.IO, cron, channels, or login-gated services; a single-core cloud/server deployment. |
| `ServiceSet::none()` | No transport, no background services: a library/harness embedder driving only `CoreRuntime::invoke`. |
| `ServiceSet::embedded()` | No transport (`rpc_http: false`), but the background work a long-lived embedded session expects: cron, login-gated services, memory queue, harness init, skill catalog refresh, memory sync. `socketio`/`channels` stay off because such a host reads state through the facade and owns its own networking. |

Individual services still honor their own runtime gates inside
`runtime/services.rs` regardless of `ServiceSet` selection: `cron` checks
`config.cron.enabled`; `channels` returns early on
`OPENHUMAN_DISABLE_CHANNEL_LISTENERS=1` (also folded into `[components].channels`) or when
`config.channels_config.has_listening_integrations()` is false; `login_gated`
(`spawn_login_gated_services`) defers the login-gated services until a user
session exists on disk. `ServiceSet` picks *whether a service is spawned at
all*; the gate picks *whether it runs for this user*.

## `DomainSet`: which domain families exist at runtime

Sibling of `ServiceSet`: where `ServiceSet` selects background services and
transports, `DomainSet` selects which controller/tool/store/subscriber
surfaces are live, one flag per `DomainGroup` (see
[`../all.rs`](../all.rs)'s `DomainGroup` for the full family list and the
harness/gate/platform split). `DomainSet::allows(group)` is the query the
registry filters on.

| Preset | Shape |
| --- | --- |
| `DomainSet::full()` | Every family on: today's default, byte-identical to registration with no runtime narrowing. |
| `DomainSet::harness()` | `agent` + `memory` + `threads` + `config` + `security` only; every gate family and `platform` off. The embeddable agent core; see the [lean headless example](../../../../openhuman-embed/examples/lean_headless.rs). |
| `DomainSet::embedded()` | The harness families plus `flows` (boot reconciliation keys off `ctx.domains().flows`), `skills`, `channels` (`channel.web_chat` is tagged `Channels` and is how an embedded host drives chat turns), `inference`, `integrations`, `automation`, `runtimes`, and `platform`. `mcp`/`web3`/`voice`/`media`/`desktop`/`hosted`/`modules` stay off: an embedded host supplies its own routing and presentation. |
| `DomainSet::kernel()` | The floor: `threads` + `config` + `security` only. Distinct from `none()`: this is "opt a subsystem back in from nothing," so `agent` and `memory` (the two largest, most replaceable subsystems) are deliberately off. See the [capability report example](../../../../openhuman-embed/examples/capability_report.rs) for inspecting runtime composition. |
| `DomainSet::none()` | Every family off. |

## `TokenSource`: how the RPC bearer is seeded

- `TokenSource::Fixed(Arc<String>)`: an in-memory bearer supplied by the
  embedder (the Tauri shell's `CoreProcessHandle.rpc_token`), seeded via
  `auth::init_rpc_token_with_value`; never crosses the process environment.
- `TokenSource::EnvOrFile`: the standalone fallback: read
  `OPENHUMAN_CORE_TOKEN` from the environment if present, otherwise generate
  a fresh token and write `{root}/core.token` (0o600 on Unix).

## `CoreBuilder` setters

`CoreBuilder::new(host_kind)` defaults to `TokenSource::EnvOrFile`,
`ServiceSet::desktop()`, `DomainSet::full()`, and `ToolGroups::default()`
(every group withheld behind `use_skill`, the desktop app's shape). The
three capability axes only narrow: a group set to `Advertised` whose tools
are compiled out, or whose `DomainGroup` is off, stays absent.

- `.services(ServiceSet)`, `.domains(DomainSet)`, `.tool_groups(ToolGroups)`:
  the three independent narrowing axes. They control which services run,
  which domain families exist, and how the tools of the families that do
  exist are disclosed (advertised on the wire, withheld behind the pack
  proxy, or not registered at all).
- `.token(TokenSource)`, `.host(impl Into<String>)`, `.port(u16)`.
- `.config(Config)`: supply the config outright instead of letting
  `build()` discover one from `config.toml` + environment; used verbatim, no
  env overlay applied automatically.
- `.workspace(dir)`: sugar over `.config()`: roots `workspace_dir` at `dir`
  and sets `config_path` to `dir/config.toml` so credentials and sessions
  resolve beside the workspace rather than a previous config root.
- `.action_dir(dir)`: sugar over `.config()` for the agent's read/write
  root.
- `.backend_url(url)`.

## `CoreContext` and initialization order

`runtime/context.rs`'s `CoreContext` owns the core's initialization
*order*: register controllers, load the master key, seed the RPC bearer,
initialize the workspace-bound stores, and run the pure-registration part of
`bootstrap_core_runtime`. `init_stores` gates each store on its owning
`DomainGroup` via `StoreInitPlan`: the memory driver binding (`Memory`), the
image-attachment sidecar dir (`Agent`), and the legacy-workflow prune
(`Skills`). The keyring-path log and the boot-time Sentry user bind run
unguarded for every `DomainSet`. The WhatsApp store moved to the Tauri
shell and the people store is owned by the bound memory driver, so neither
is seeded here. `CoreContext` also holds the `DomainSet` for its scope and,
when supplied, the embedder's `Config`. RPC handlers otherwise re-resolve
config independently per dispatch, so an embedder-supplied config would be
silently ignored without this seam.

The "wrong-workspace guard" (Sentry OPENHUMAN-CORE-48 / TAURI-RUST-8NM)
lives in `CoreContext::init_with_config`: when no config was supplied and
`Config::load_or_init()` fails, it logs an error and skips `init_stores`
entirely. Memory stays explicitly uninitialised for the run and callers get
a "memory client not ready" error, rather than the core falling back to
`Config::default()` and seeding stores against the wrong workspace.

`CoreContext::scope(ctx, fut)` establishes the ambient context for the
duration of a future (used at the `try_invoke_registered_rpc` chokepoint and
by `CoreRuntime::invoke`); `CoreContext::current` falls back to a
process-wide default context when no scope is active, which is what lets
existing single-tenant call sites keep working unmodified.

## Operating mode: `SingleUser` and `Saas`

`runtime/mode.rs` holds the process mode. `SingleUser` is today's
behaviour and the default: the desktop app, the CLI and embedders. `Saas`
serves many users from one process, one profile per user, behind a trusted
gateway that authenticates them. The mode belongs to the process, not to any
user's `Config`. The first SaaS boot locks it (`lock_mode`), and from then on
`CoreContext::init_with_config` refuses every non-SaaS core. Code asks
`is_saas()`, which reads an unset slot as `SingleUser`.

`openhuman-core run --mode saas --saas-config <file>` (or
`OPENHUMAN_MODE=saas`, which can raise the mode but never lower it) boots
through `runtime/saas.rs`. So does `openhuman_embed::ProfileRuntime`, which
runs the same `saas::build` in-process without serving a port (it writes a
random service token when none exists, since no gateway presents it). Either
way the lock is one-way: once it is built, no `Runtime::builder()` or other
non-SaaS core can boot in that process, and neither can a second SaaS core.

`saas::build`:

- `SaasConfig` is the **operator's** file. It sets `root`,
  `service_token_file` (defaults to `<root>/service.token`), `tool_allowlist`
  (host tool groups, see `profiles/README.md`), `[sandbox]` (the shell
  container), `rpc_allowlist_extra`, `max_profiles_open`, `profile_ids`, `idle_evict_secs`,
  `shared_backend_api_key`, `custom_definitions` and `require_user_signature`
  (default `true`), and the cluster settings: `storage_url` (the shared
  backend; `OPENHUMAN_STORAGE_URL` wins), `node_id` (else
  `OPENHUMAN_NODE_ID`, else random per process), `advertise_url`,
  `lease_ttl_secs` (default 30) and `operator_dir` (default
  `<root>/operator`). Unknown keys are refused.
- `runtime/boot_guard.rs` refuses the boot, listing every problem at once,
  when:
  - the host kind is not `Saas`;
  - services or domains go beyond `ServiceSet::saas()` / `DomainSet::saas()`;
  - single-user or back-door environment variables are set
    (`OPENHUMAN_WORKSPACE`, `OPENHUMAN_DEV_CONNECT`, backend tokens,
    `OPENHUMAN_CORE_TOKEN`, approval gate or sandbox switched off);
  - `tool_allowlist` names something other than a tool group, or allowlists
    `host_shell` without a working Docker, with network `host`, no image, or
    non-positive limits;
  - `rpc_allowlist_extra` is non-empty (the per-user RPC surface is fixed);
  - the root is relative, missing, world-writable or inside `~/.openhuman`;
  - the service token is missing, readable by others, or shorter than 32 bytes;
  - the node is clustered (`advertise_url` set) without a storage URL whose
    driver keeps compare-and-swap atomic across processes (SQLite, MongoDB).
- The operator plane boots with its own config under `operator_dir()`
  (`<root>/operator/` by default), so
  it never resolves `~/.openhuman` or an `active_user.toml`. The gateway
  bearer is the RPC token.

The SaaS presets are closed. `DomainSet::saas()` registers the operator plane
(`DomainGroup::Operator`, the `profiles.*` controllers) and the user
families whose per-user isolation has landed (threads, channels for web chat,
memory). `profiles::surface`
keeps the two planes apart: the operator scope reaches only the operator
plane, and a user's scope only the reviewed `USER_METHODS`.
`saas::build` installs the process's `profiles::ProfileHost`. Each open
profile (`<root>/users/<profile-id>/`, the desktop's user layout) runs under a
context derived from the operator's, with its own forced config, policy,
`profile` (its default agent has no `session_agent`, as on the desktop),
behind a lease that keeps it on one process
(see `profiles/README.md`). `saas::build` also starts the lease heartbeat.

Three guards keep SaaS work from falling back to process-wide state:

- **Config redirect.** In SaaS, `Config::load_or_init()` returns the config the
  current context carries and fails outside any context
  (`config/schema/load/saas_scope.rs`). It never resolves
  `OPENHUMAN_WORKSPACE`, `active_user.toml` or `~/.openhuman`.
- **Scoped spawns.** `runtime/spawn.rs`'s `spawn_scoped` /
  `spawn_blocking_scoped` carry the caller's `CoreContext` into the spawned task
  (the memory identity is explicit and is handed to spawned work as a value). `scripts/ci/check-saas-ambient.mjs` (`pnpm
  saas:ambient`) ratchets the bare spawns, direct `load_or_init` calls,
  environment writes and `home_dir()` lookups that remain.
  `CoreContext::propagate` follows the same rule in SaaS.
- **The tenant key.** `runtime/tenant.rs`: a context derived with
  `ContextOverlay::profile(id)` serves that SaaS profile, and
  `current_tenant()` reads `Tenant { profile, agent }` from the task's own
  scope only (`CoreContext::scoped`), failing closed with `NoTenant`
  rather than falling back to the operator. Per-tenant state keys on it:
  `tenant_key(&tenant, id)` (injective; the bare id on the desktop) for
  in-process tables keyed by caller-chosen thread or session ids,
  `session_key(&tenant)` for the host session store, and
  `storage::scope_for_profile` for the storage backend. A profile gets
  fresh state slots, `slot_for` hands an unscoped SaaS task a throwaway
  slot, and `CoreContext::tenant_in_use()` reports whether turns still run
  on a profile, so a host never evicts it mid-turn. The ratchet's
  `ambient-context` rule baselines the `CoreContext::current()` and
  `.session_agent()` reads that remain outside `core/runtime/`.

`DomainSet` lives in `runtime/domain_set.rs` and `DomainGroup` in
`core/domain_group.rs`. Both are re-exported from their old paths.

## Shared tokio tuning constants

[`runtime/mod.rs`](../../runtime/mod.rs) declares two constants every multi-thread runtime that may
host an agent turn must set:

- `AGENT_WORKER_STACK_BYTES` (20 MiB): a single agent turn is a very large
  async state machine (system prompt + hundreds of tool specs + the nested
  provider/tool loop), and delegating to a sub-agent nests another one. That
  overflows tokio's default 2 MiB worker stack and aborts the process.
- `MAX_BLOCKING_THREADS` (64): tokio defaults this to 512, which combined
  with the raised stack size could reserve up to `64 × 20 MiB` of virtual
  stack space. Physical pages are committed as a thread's stack grows.
  `spawn_blocking` on these runtimes backs SQLite, filesystem grep/glob,
  document parsing, and URL guarding: bounded, bursty concurrency, so 64
  leaves headroom while capping the idle footprint.

## Related docs

- [../README.md](../README.md): the rest of `core/`, covering dispatch,
  registry, event bus, transport, and CLI.
- [Embedding OpenHuman](../../../../../docs/gitbooks/en/developing/embedding.md)
- [Deep architecture reference](../../../../../docs/gitbooks/en/developing/architecture.md)
