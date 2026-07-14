# Phase 1 — `CoreBuilder` / `CoreRuntime` / `ServiceSet`

**Status:** planned.
**Goal:** split bootstrap (context init) from transport (HTTP is just another
service) behind a public builder API; port all existing entry points onto it.
This phase alone delivers the headline goal: embeddable library, programmatic
harness entry, thin CLI/Tauri.

## New modules

```text
src/core/runtime/
├── mod.rs        # pub use CoreBuilder, CoreRuntime, ServiceSet, TokenSource
├── builder.rs    # CoreBuilder — validation + build()
├── context.rs    # CoreContext (Stage A facade — owns init order; see phase 2)
└── services.rs   # from phase 0; gains spawn_rpc_http_service, spawn_socketio wiring
```

Public API per README §2.1. Additional decisions:

- **`TokenSource`**: `Fixed(Arc<String>)` (Tauri in-memory handoff),
  `EnvOrFile` (read `OPENHUMAN_CORE_TOKEN` when present, otherwise generate
  and write the standalone `{root}/core.token` fallback 0600). `build()` seeds
  `auth::init_rpc_token*` exactly once, same precedence as today
  (`src/core/auth.rs`).
- **`build()` is init-only**: no sockets and no detached jobs for
  `ServiceSet::none()` / `ServiceSet::headless_api()`. It runs, in
  order: controller registration (`all::all_registered_controllers`), master
  key init, token seeding, `Config::load_or_init`, `init_stores` (phase 0),
  `bootstrap_core_runtime(host_kind, services)` for pure registration plus
  ServiceSet-gated legacy bootstrap jobs. Init-order regressions are the top
  risk; add a startup-sequence integration test asserting the order via log
  markers.
- **`serve()`** spawns only the services selected by `ServiceSet`. The HTTP
  listener (bind, port fallback, router from `build_core_http_router`,
  `axum::serve`) moves into `spawn_rpc_http_service`; **the
  `set_var OPENHUMAN_CORE_RPC_URL` call keeps its exact timing** (post-bind,
  `jsonrpc.rs:2010`) inside that service — child tools depend on it. It is
  flagged in the drift ledger as single-runtime-only.
- **Ready signal**: `EmbeddedReadySignal` (port-fallback reporting) is kept
  type-identical, relocated; the readiness sender is supplied to
  `CoreRuntime::serve` and fires after bind (when `rpc_http` selected).
- **`invoke()`** delegates to the existing `invoke_method`
  (`jsonrpc.rs:230`) — same tiered dispatch, no second path.

## `AgentRuntime` (harness-as-library)

`CoreRuntime::agent_runtime()` returns the slice needed to run turns with
zero ports bound:

```rust
pub struct AgentRuntime { ctx: Arc<CoreContext> }
impl AgentRuntime {
    pub fn agent(&self, sel: AgentSelector) -> anyhow::Result<Agent>;      // Agent::from_config
    pub async fn run_turn(&self, agent: &Agent, input: TurnInput) -> …;    // tinyagents seam
    pub fn events(&self) -> broadcast::Receiver<CoreEvent>;
}
```

Its required bootstrap subset _defines_ what `ServiceSet::none()` must still
initialize: config, workspace, master key, event bus,
`AgentDefinitionRegistry`, memory/tinycortex stores, cost/x402 ledgers,
security live-policy + approval gate, tool registry. Explicitly not required:
HTTP, socket.io, cron, channels, heartbeat, update scheduler.

**Acceptance test for the whole phase:** an integration test (and
`examples/run_turn.rs`) that builds with `ServiceSet::none()`, runs one agent
turn through `run_turn_via_tinyagents_shared`, and asserts no listener was
bound.

## Porting the consumers

| Consumer                                                       | Change                                                                                                                                                                                                                     |
| -------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `run_server` / `run_server_embedded*` (`jsonrpc.rs:1682-1737`) | become `#[deprecated]` shims over the builder; deleted one release later                                                                                                                                                   |
| Tauri (`app/src-tauri/src/core_process.rs:289`)                | `CoreProcessHandle::ensure_running` builds `CoreBuilder::new(HostKind::TauriShell).token(TokenSource::Fixed(..)).services(ServiceSet::desktop())`, then passes readiness to `CoreRuntime::serve`; `CancellationToken` / restart / port-takeover logic unchanged |
| CLI `run`/`serve` (`src/core/cli.rs:66`)                       | maps flags → `ServiceSet` (`--jsonrpc-only` → `socketio: false`)                                                                                                                                                           |
| CLI `call` + namespace dispatch (`cli.rs:354,435`)             | `ServiceSet::none()` build → `runtime.invoke()`; one-shot calls stop constructing server state                                                                                                                             |
| MCP stdio (`cli.rs` `mcp`)                                     | adapter over `runtime.invoke()`                                                                                                                                                                                            |
| `src/lib.rs`                                                   | re-export the new surface; keep `run_core_from_args`                                                                                                                                                                       |

Plus `examples/embed_headless.rs` (build + `serve()` with
`ServiceSet::headless_api()`, call `openhuman.ping` over HTTP) and
`examples/run_turn.rs` (above).

## Risks & mitigations

| Risk                                                                          | Mitigation                                                                               |
| ----------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------- |
| Init-order regression (master key before stores; policy before approval gate) | single `CoreContext::init` encoding the order, line-ref comments, startup-sequence test  |
| Tauri port-fallback / ready semantics drift                                   | `EmbeddedReadySignal` unchanged; e2e boot test in CI Full already covers                 |
| `set_var` timing change breaks child tools                                    | timing preserved inside `spawn_rpc_http_service`; grep test for env presence after ready |
| Double-bootstrap when shims + builder both run in tests                       | builder guards with the same idempotent `Once`s during this phase (removed in phase 3)   |

## Verification

- `pnpm test:rust`, `bash scripts/test-rust-with-mock.sh --test json_rpc_e2e`.
- Both examples run green in CI.
- Desktop app boots via ported `core_process.rs` (CI Full e2e matrix).
