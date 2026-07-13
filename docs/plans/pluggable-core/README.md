# Pluggable Core — `openhuman_core` as an Embeddable Library

**Status:** In progress — `CoreBuilder`, `CoreRuntime`, `CoreContext`, the
first per-context store plumbing, and the `openhuman-fleet` MVP are implemented
in this branch. Remaining production work is tracked in the phase docs.

**Goal:** make the Rust core pluggable into arbitrary hosts — the Tauri shell
(today), a plain CLI, a stdio MCP server, cloud/team servers managing many
users, and other programs consuming it as a library — via a first-class
`CoreBuilder` → `CoreRuntime` API instead of the current "library entry is a
CLI arg vector" surface.

**Why:** teams and programmatic use both need the same thing: the ability to
compose the core's pieces (dispatcher, stores, background services,
transports) differently per host. Today that composition is fixed inside one
700-line function. The client side already solved this problem with the
`CoreTransport` strategy layer (`app/src/services/transport/`); the core side
has no equivalent seam.

**Anchor precedents:** [`docs/tinyagents-port-plan.md`](../../tinyagents-port-plan.md)
and [`docs/tinycortex-migration-spec.md`](../../tinycortex-migration-spec.md) —
the established "seam + staged migration + drift ledger" doctrine. This plan
restructures the _host_, not the engines: `tinyagents` / `tinycortex` seams
are untouched.

---

## 1. Where we are

### 1.1 What is already right

The hard parts of pluggability are, surprisingly, already done:

| Asset                       | Where                                                                                                                                                                                | Why it matters                                                                                                            |
| --------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------- |
| Transport-agnostic dispatch | `ControllerSchema` + `invoke_method` (`src/core/jsonrpc.rs:230`) shared by HTTP `/rpc`, CLI `call`, generic namespace dispatch, and MCP stdio                                        | The _contract_ is host-neutral; only the bootstrap is not                                                                 |
| Pluggable client            | `app/src/services/transport/` — `CoreTransport` interface, `LocalTransport` / `LanHttpTransport` / `TunnelTransport` / `CloudHttpTransport`, `ConnectionProfile`, `TransportManager` | Any backend that answers `POST /rpc` JSON-RPC with a Bearer token is already reachable from every client, including cloud |
| Agent loop as a crate       | `tinyagents` (vendored, `vendor/tinyagents`) via the seam `src/openhuman/tinyagents/` (`run_turn_via_tinyagents_shared`)                                                             | Programmatic harness use does not require extracting the loop — it's extracted                                            |
| Memory engine as a crate    | `tinycortex` via `src/openhuman/tinycortex/`                                                                                                                                         | Same                                                                                                                      |
| Headless server mode        | `openhuman-core run/serve` (`src/core/cli.rs:66`), `--jsonrpc-only`, Bearer token via `OPENHUMAN_CORE_TOKEN` (`src/core/auth.rs:160`)                                                | Cloud deployment of a _single_ core already works                                                                         |
| Host discrimination         | `HostKind { TauriShell, Cli, Docker }` (`src/core/types.rs:167`) threaded into `bootstrap_core_runtime`                                                                              | The natural seam for the refactor already exists                                                                          |
| Tools as trait objects      | `Box<dyn Tool>` on `Agent` (`src/openhuman/agent/harness/session/types.rs:31`)                                                                                                       | No handler-style fn-pointer problem in the tool layer                                                                     |

### 1.2 The coupling points (what blocks embedding)

| #   | Concern                            | Location                                                                                                                                                                                        | Nature                                                                                                                                                                                                                                                                  |
| --- | ---------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | Bootstrap fused to transport       | `run_server_inner` (`src/core/jsonrpc.rs:1738`) + `bootstrap_core_runtime` (`:2428`)                                                                                                            | One function: controller registration, master key, token seeding, `Config::load_or_init`, ~10 global store inits (`:1807-1892`), port bind, env mutation, router, ready signal, inline `tokio::spawn` of every background service. You cannot get "just the dispatcher" |
| 2   | Handlers are bare `fn` pointers    | `ControllerHandler` (`src/core/all.rs:21`)                                                                                                                                                      | No captured state possible; every handler reads process globals                                                                                                                                                                                                         |
| 3   | Three hand-maintained registries   | `src/core/all.rs` — handlers (`:105-344`), schemas (`:375-509`), namespace descriptions (`:530-690`)                                                                                            | Parallel lists in `OnceLock` statics, panic-on-drift validation                                                                                                                                                                                                         |
| 4   | Process singletons                 | `RPC_TOKEN` (`src/core/auth.rs:75`), `GLOBAL_BUS` (`src/core/event_bus/bus.rs:20`), `NativeRegistry` (`src/core/event_bus/native_request.rs:329`), every `*::global::init(workspace_dir)` store | One of each per process                                                                                                                                                                                                                                                 |
| 5   | Single active user per process     | `~/.openhuman/active_user.toml` (`src/openhuman/config/schema/load_user_state.rs:21`), resolution chain in `dirs.rs:299`                                                                        | One workspace resolved once; all global stores bound to it                                                                                                                                                                                                              |
| 6   | Background services spawned inline | cron (`jsonrpc.rs:~2124`), channels (`:~2162`), heartbeat/subconscious (`:~2083`), update scheduler                                                                                             | Flags exist (`config.cron.enabled`, `OPENHUMAN_DISABLE_CHANNEL_LISTENERS`, `--jsonrpc-only`) but the caller cannot compose a service set                                                                                                                                |
| 7   | Runtime env mutation               | `set_var OPENHUMAN_CORE_RPC_URL` (`jsonrpc.rs:2010`)                                                                                                                                            | Process-global side effect; needed by spawned child tools                                                                                                                                                                                                               |
| 8   | `Once`-guarded subscribers         | `register_domain_subscribers` (`jsonrpc.rs:2232`, `std::sync::Once`)                                                                                                                            | Second context in one process cannot re-register                                                                                                                                                                                                                        |
| 9   | Dead dispatch tier                 | `src/rpc/dispatch.rs:10` always returns `None`                                                                                                                                                  | Removable noise                                                                                                                                                                                                                                                         |

### 1.3 Teams today

`src/openhuman/team/` is a **pure thin proxy** to `tinyhumansai/backend`
(session JWT via `crate::api::jwt`, URL via
`effective_backend_api_url`, `src/api/config.rs:154`). All membership, roles,
invites, and authorization are enforced server-side. That is the right
division and this plan keeps it: **we make the core _hostable by_ a team
server; we do not reimplement team logic locally.**

---

## 2. Target architecture

Bootstrap splits into three layers that are fused today:

```text
CoreBuilder::build()                      ── layer 1: context init (pure — no
  └─ CoreContext                             sockets, no spawns): config,
       stores · event bus · registries       workspace, master key, stores,
       security policy · approval gate       subscribers, policy

CoreRuntime::serve()                      ── layer 2: services (each opt-in):
  ├─ rpc_http    (axum /rpc + Bearer)        selected by ServiceSet
  ├─ socketio
  ├─ cron · channels · heartbeat · update

runtime.invoke(method, params)            ── layer 3: transports are thin
CLI `call` · MCP stdio · HTTP handler        adapters over the same dispatch
```

### 2.1 Public API (sketch)

```rust
pub struct ServiceSet {
    pub rpc_http: bool,
    pub socketio: bool,
    pub cron: bool,
    pub channels: bool,
    pub heartbeat: bool,
    pub update_scheduler: bool,
}
impl ServiceSet {
    pub fn desktop() -> Self;       // everything on (Tauri today)
    pub fn headless_api() -> Self;  // rpc_http only (cloud single-core)
    pub fn none() -> Self;          // library / harness-only
}

pub struct CoreBuilder { /* config, host_kind, token, bind, services */ }
impl CoreBuilder {
    pub fn new(host_kind: HostKind) -> Self;
    pub fn token(self, t: TokenSource) -> Self;      // Fixed | EnvOrFile
    pub fn services(self, set: ServiceSet) -> Self;
    pub async fn build(self) -> anyhow::Result<CoreRuntime>;  // init only, no spawns
}

pub struct CoreRuntime { /* Arc<CoreContext>, CancellationToken, bound_addr */ }
impl CoreRuntime {
    pub async fn serve(&self, ready: Option<oneshot::Sender<EmbeddedReadySignal>>, shutdown: Option<CancellationToken>)
        -> anyhow::Result<()>;                               // spawn selected services
    pub async fn invoke(&self, method: &str, params: Map<String, Value>)
        -> Result<Value, RpcError>;                           // same path as /rpc
    pub fn events(&self) -> broadcast::Receiver<CoreEvent>;
    pub fn agent_runtime(&self) -> AgentRuntime;              // harness-only slice
    pub async fn shutdown(self) -> anyhow::Result<()>;
}
```

`src/lib.rs` re-exports `CoreBuilder`, `CoreRuntime`, `ServiceSet`,
`HostKind`, `AgentRuntime` as the documented public surface;
`run_core_from_args` remains for the binary.

### 2.2 Existing entry points become thin consumers

| Entry                                               | Today                               | After                                                                                                                           |
| --------------------------------------------------- | ----------------------------------- | ------------------------------------------------------------------------------------------------------------------------------- |
| `run_server` (`jsonrpc.rs:1682`)                    | wraps `run_server_inner`            | shim over `CoreBuilder` (deprecated, kept one release)                                                                          |
| `run_server_embedded_with_ready` (`:1717`)          | same                                | Tauri `core_process.rs` calls `CoreBuilder` directly: `TokenSource::Fixed(in-memory)`, `ServiceSet::desktop()`; readiness is passed to `CoreRuntime::serve` |
| CLI `call` / namespace dispatch (`src/core/cli.rs`) | `invoke_method(default_state(), …)` | `ServiceSet::none()` build → `runtime.invoke()` — no port bound for one-shot calls                                              |
| MCP stdio (`src/core/cli.rs` `mcp`)                 | same funnel                         | transport adapter over `runtime.invoke()`                                                                                       |
| Cloud/team host                                     | n/a                                 | fleet supervisor composing one core per user (phase 4)                                                                          |

### 2.3 Multi-tenancy: fleet of processes first

**Decision: one supervised core process per user/workspace first — not
in-process multi-tenancy.** Rationale:

- It is the shape the architecture already has (Tauri = one embedded core for
  one user); a supervisor reuses it with zero correctness risk.
- The blockers to in-process tenancy are real and slow: env-var mutation for
  child tools (#7), keyring/master-key process scope, `Once`-guarded
  subscribers (#8), Sentry. Separate processes improve blast-radius and
  lifecycle control, but production multi-tenant security still requires
  distinct OS users or containers because agents run arbitrary tools.
- The wire contract is unchanged, so `CloudHttpTransport` works as-is against
  a per-user base URL.

In-process multi-workspace (a `CoreContext` per tenant) is deferred behind
phase 3 and is an optimization, not a prerequisite, for the team story.

### 2.4 Globals strategy: staged, bounded

Full DI through ~110 handler registration sites in one PR is unreviewable.
Three stages (detail in [phase-2](phase-2-corecontext.md) /
[phase-3](phase-3-multi-context.md)):

- **Stage A (facade):** `CoreContext` _owns initialization order_ and hands
  out handles; existing `*::global::init` calls move inside it; handlers keep
  reading globals. Zero behavior change.
- **Stage B (mechanical):** registered RPC dispatch installs an ambient
  task-local `CoreContext` while handlers remain plain `fn` pointers. Domains
  then migrate off globals opportunistically by reading `CoreContext::current()`,
  tracked in a drift ledger.
- **Stage C (bounded):** only what multi-context isolation actually needs.
  **Exit criterion: two `CoreContext`s in one test process serve
  memory/people/config reads without cross-talk — not "zero `OnceLock`s".**
  Keyring, Sentry, `NativeRegistry`, and env vars stay process-scoped and are
  documented as such.

### 2.5 Storage abstraction

Backend-level pluggability also requires the _stores_ to be swappable, not
just the transports. Today every store is constructed from a workspace
directory (`*::global::init(workspace_dir)`) and is concretely
SQLite/JSON-on-disk under `~/.openhuman/users/<id>/workspace`. That is
invisible in the desktop app but load-bearing for other backends:

- **Fleet hosting (phase 4)** works _without_ abstraction — each per-user core
  process gets its own workspace volume. This is why storage abstraction is
  not on the phase-4 critical path.
- **Managed/cloud storage** (Postgres, object store for attachments,
  shared-nothing replicas) and **in-process multi-tenancy** both require
  stores behind traits.

The seam is the `CoreContext` handle layer introduced in Stage A/B: handlers
stop touching `*::global()` and go through `ctx.<store>()`. Those accessors
return **trait objects** (`Arc<dyn MemoryStore>`, `Arc<dyn PeopleStore>`, …)
rather than concrete types, and `CoreBuilder` gains a `StorageBackend`
selector whose only shipped implementation is `WorkspaceFs` (the current
behavior, byte-for-byte). Store traits are carved per domain _as that domain
migrates onto the context_ (phase 2 drift ledger gets a "trait extracted?"
column) — not as one big up-front storage rewrite. Note the engines already
model this: `tinyagents` has `StoreRegistry`/checkpointer abstractions and
`tinycortex` owns its own store interfaces; the traits here cover the
_host-owned_ stores (people, attachments, cost/x402 ledgers, config state,
run ledgers) that sit outside those crates.

Remote implementations (e.g. `Postgres`) are explicitly out of scope for this
plan; the deliverable is that adding one is a new `StorageBackend` impl, not
a refactor.

---

## 3. Non-goals

- **Wire contract**: `POST /rpc` JSON-RPC, `openhuman.<ns>_<fn>` naming,
  Bearer auth — unchanged. Client transports untouched.
- **tinyagents / tinycortex seams**: unchanged; this plan restructures the
  host around them.
- **Security policy semantics**: `security::live_policy`, approval gate,
  autonomy tiers — unchanged (only _who installs them_ moves into
  `CoreBuilder::build`).
- **Local team logic**: membership truth stays in `tinyhumansai/backend`.
- **Not attempted**: replacing axum, changing controller schema format,
  in-process tenant _security_ isolation, `inventory`/linkme distributed
  registration (explicit registration lists fit this repo's ledger culture).

## 4. Phases

| Phase | File                                                 | Deliverable                                                                                                                                 | Depends on  |
| ----- | ---------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------- | ----------- |
| 0     | [phase-0-defusion.md](phase-0-defusion.md)           | Delete dead tier-3 shim; extract inline service spawns + store-init block into named fns. Pure motion                                       | —           |
| 1     | [phase-1-corebuilder.md](phase-1-corebuilder.md)     | `CoreBuilder`/`CoreRuntime`/`ServiceSet`; `run_server*` become shims; Tauri + CLI + MCP ported; embed examples                              | 0           |
| 2     | [phase-2-corecontext.md](phase-2-corecontext.md)     | `CoreContext` Stage A + B; registry collapse to per-domain `DomainRegistration`; store traits + `StorageBackend::WorkspaceFs`; drift ledger | 1           |
| 3     | [phase-3-multi-context.md](phase-3-multi-context.md) | Bounded Stage C; two-context isolation test; process-scoped inventory                                                                       | 2           |
| 4     | [phase-4-fleet-host.md](phase-4-fleet-host.md)       | `openhuman-fleet` supervisor: per-user cores, token minting, `/:user/rpc` proxy, backend membership sync                                    | 1 (not 2/3) |

**Value ordering:** phases 0+1 alone deliver the headline goal — embeddable
builder, programmatic harness, thin CLI/Tauri. Phase 4 can start immediately
after phase 1, in parallel with 2/3, because the fleet model needs
process-per-user, not context threading.
