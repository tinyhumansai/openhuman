# core

The controller contract, in-process dispatch, the controller registry, the
event bus, auth, the CLI, and runtime composition. `core/` is not a domain: it holds no business
logic. Every controller it exposes is implemented by a domain module under
`crates/openhuman-core/src/<domain>/` and wired in here.

## Responsibilities

- Define the transport-agnostic controller contract that both RPC and CLI
  invoke against: `ControllerSchema`, `FieldSchema`, `TypeSchema` (`mod.rs`),
  the [`Outcome`](outcome.rs) every operation returns, the structured error
  envelope (`structured_error.rs`) and the params rules (`params.rs`).
- Own the single registry of every controller (`all.rs`) and the coarse
  `DomainGroup` tagging that lets a runtime narrow its live surface.
- Dispatch every call in-process through `invoke::invoke_method`, which
  validates params, runs the tiered router (`dispatch.rs`) and publishes
  `SessionExpired` on a confirmed expiry (`session_expiry.rs`).
- Own the process-wide typed event bus (`bus.rs`, `events.rs`).
- Provide the CLI surface (`cli.rs` and friends). Serving JSON-RPC and
  Socket.IO over HTTP is `openhuman-rpc`'s job; the CLI's `run` / `serve`
  start it through the launcher a host installs (`server_launcher.rs`).
- Seed and check the per-process RPC bearer token (`auth.rs`,
  `event_bind_tokens.rs`).
- Compose an embeddable `CoreRuntime` from domain modules (`runtime/`) and
  track subsystem driver health (`subsystem/`).
- Redact secrets before anything reaches a log line or Sentry
  (`log_redaction.rs`, `rpc_log.rs`, `observability.rs`).

## Key files

| File | Role |
| --- | --- |
| `mod.rs` | `ControllerSchema`/`FieldSchema`/`TypeSchema`: the controller contract; module declarations. |
| `all.rs` | The controller registry: `RegisteredController`, `RegisteredCliAdapter`, `DomainGroup`, `all_registered_controllers()`, `all_controller_schemas()`, `rpc_method_name()`, `validate_params()`, `all_http_method_schemas()`. |
| `dispatch.rs` | `dispatch()`: the 4-tier RPC router. |
| `bus.rs` | The `BUS: OnceBus<DomainEvent>` singleton, `EVENTS_ROOT`/`EVENTS_INTERFACE`/`EVENTS_VERSION`, `init`. |
| `events.rs` | `DomainEvent`: the full event catalog, `domain()` routing. |
| `outcome.rs` | `Outcome<T>`, `apply_log_envelope`, `unwrap_rpc`: the controller result and its wire shape. |
| `structured_error.rs` | `StructuredRpcError`: typed error envelope sentinel-encoded into a controller's `Err(String)`. |
| `params.rs` | Params shape (`params_to_object`) and the validation messages `all::validate_params` emits, with their matcher. |
| `invoke.rs` | `invoke_method` / `default_state`: in-process dispatch every transport and the CLI use. |
| `session_expiry.rs` | `is_session_expired_error`: which failures mean the OpenHuman session expired. |
| `server_launcher.rs` | `ServerLauncher` port the CLI `run` / `serve` subcommands start a server through. |
| `auth.rs` | Per-process RPC bearer token: init paths, `get_rpc_token`, `verify_bearer_token`, `bearer_matches`. The HTTP route policy is `openhuman_rpc::server::auth`. |
| `event_bind_tokens.rs` | Single-shot bind tokens for the `/events` SSE stream. |
| `cli.rs`, `agent_cli.rs`, `memory_cli.rs`, `subsystems_cli.rs`, `cli_capability.rs` | CLI argument parsing and dispatch, routed through the same registry as RPC. |
| `types.rs` | `AppState`, `HostKind`, `InvocationResult`, `approval_gate_boot_decision`. |
| `legacy_aliases.rs` | `resolve_legacy`: rewrites retired method names before dispatch; mirrors `app/src/services/rpcMethods.ts`'s `LEGACY_METHOD_ALIASES`. |
| `observability.rs` | `report_error` + Sentry `before_send` filters that drop deterministic provider/updater noise. |
| `log_redaction.rs` | `scrub_secrets`: regex secret scrubbing shared by the Sentry path and always-on log path. |
| `rpc_log.rs` | `redact_params_for_log` (key-name redaction for the `[rpc:dispatch]` trace log in `dispatch.rs`) |
| `logging.rs` | `init_for_cli_run` / `init_for_embedded`: logger setup for each host kind. |
| `shutdown.rs` | Graceful shutdown signal plumbing. |
| `sentry_transport.rs` | Sentry client setup, gated by the `crash-reporting` feature. |
| `http_server_status.rs` | `HTTP_SERVER_COMPILED_IN`: ungated compile-time marker so a listener-less core fails the build instead of shipping silently. |
| `bus_testing.rs` | `isolated_bus()`: a private bus for tests that must observe events without racing the process-global singleton. |
| `runtime/` | `CoreBuilder` → `CoreRuntime` composition API: see [runtime/README.md](runtime/README.md). |
| `subsystem/` | The `subsystems` status namespace (`SubsystemStatus` row, RPC and CLI table); memory fills its row. |

## Controller registration and `DomainGroup`

Every controller is registered exactly once, in `all.rs`, tagged with a
`DomainGroup`: the coarse family (`Agent`, `Memory`, `Security`, `Flows`,
`Inference`, `Platform`, …) it belongs to. `DomainGroup` variants track the
`crates/openhuman-core/src/` family directories 1:1. The live surface
(schema dump, dispatch, agent tools, stores, subscribers) is filtered by
whether the active `runtime::context::CoreContext`'s `DomainSet` allows that
group; `DomainSet::full()` allows every group, so registration stays
byte-identical to a build with no runtime narrowing. Adding a new family
directory means adding the matching `DomainGroup` variant, a `DomainSet`
field, an arm in `DomainSet::allows()`, and an entry in each preset: the
compiler enforces all four.

`rpc_method_name(schema)` turns a controller's dotted key (`memory.doc_put`)
into its wire method name (`openhuman.memory_doc_put`); the two are
intentionally different strings and callers must not conflate them.
`validate_params()` checks required/unknown params and each present value's
declared `TypeSchema` before a handler ever runs. `all_http_method_schemas()`
adds the two Tier-1 internal methods (`core.ping`, `core.version`) to the
registered set for `/schema`.

Wire controllers only through this registry: do not add namespace branches
to `cli.rs` or the JSON-RPC server. RPC namespace strings are wire contracts and do
not follow directory renames.

## Dispatch

`dispatch::dispatch()` is the single entry point every transport (`/rpc`,
Socket.IO, the CLI, `CoreRuntime::invoke`) calls through. It resolves in four
tiers:

1. **Tier 0: legacy alias rewrite.** `legacy_aliases::resolve_legacy`
   rewrites a retired method name to its canonical form before any lookup,
   symmetric with the frontend's `normalizeRpcMethod`.
2. **Tier 1: internal core methods.** `core.ping`, `core.version`, and
   `core.events_subscribe_token` (mints a bind token for `/events`) are
   handled directly, with no controller registration.
3. **Tier 2: registered controllers.** Looked up by RPC method name in the
   `all.rs` registry, validated with `validate_params`, and invoked.
4. **Tier 3: unknown method.** Always a method-not-found error; only the
   Sentry severity differs by whether the method is a known non-actionable
   probe (`is_known_probe_method`).

## Event bus

`bus.rs` declares the process-wide `BUS: OnceBus<DomainEvent>`: the one
thing a generic bus crate (`tinybus`) cannot own, since it is generic over
the event type. `events.rs` is the vocabulary half: `DomainEvent`, a
`#[non_exhaustive]` enum whose `domain()` method is the routing key appended
to `EVENTS_ROOT` (`/ai/tinyhumans/openhuman/events`). `EVENTS_VERSION` is
currently `1.4.0`; bump the minor for an added variant or field, the major
(and rename `EVENTS_INTERFACE`) for a breaking change.

Two surfaces, pick by what the call needs to carry:

| Need | Use |
| --- | --- |
| tell anyone who cares that something happened | `BUS.publish` |
| call another module with a payload that is plain data | `BUS.publish` + a reply event, or a native request |
| call another module passing `Arc`s, trait objects, or channels | `BUS.native()` |
| call an out-of-process integration | a `tinybus::Proxy` off `BUS.get()?.connection()` |

Adding an event requires four steps: add the variant to `DomainEvent`,
extend the `domain()` match, register its subscriber at startup, and bump
`EVENTS_VERSION` in `bus.rs`. Native request/response types must be
`Send + 'static` and never need serialization: they are the sanctioned
escape hatch for values (live channels, `Arc<dyn Tool>`) that cannot cross a
serialized transport.

Each subscribing domain owns a `bus.rs`; subscriber names use
`<domain>::<purpose>` (e.g. `health::registry`).

## RPC and HTTP transport

Every transport resolves a method through `invoke::invoke_method`, which checks
the registry schema, validates params
(`all::validate_params`, messages from `params.rs`), dispatches, and on a
confirmed session expiry publishes `DomainEvent::SessionExpired`.
`CoreRuntime::invoke` wraps it for embedders; the CLI's `call` subcommand
uses it directly.

The HTTP and Socket.IO server that exposes these methods lives in
`crates/openhuman-rpc` (`openhuman_rpc::server`): the axum router, `/rpc`,
`/health`, `/schema`, `/events`, `/oauth/mcp/callback`, the SSE streams, auth
middleware, CORS, Socket.IO and the listener bind. Host-owned `/auth` and
`/auth/telegram` callbacks are not served by this router. It mounts the
domain-owned HTTP handlers that stay here behind the `http-server` feature
(`inference::http`'s `/v1` router, the
dictation WebSocket in `voice::streaming`). `CoreRuntime` exposes the hooks
the server needs around a listener (`start_services`, `listener_bound`,
`serving_started`, `exit_cleanup`).

## Auth

`auth.rs` seeds the per-process RPC bearer through one of three paths, in
order of preference: an in-memory handoff from the Tauri shell
(`init_rpc_token_with_value`, never touches the process environment), the
`OPENHUMAN_CORE_TOKEN` env var (operator-supplied for Docker/cloud), or a
freshly generated token written to `{workspace_dir}/core.token`
(owner-read-only) for standalone CLI clients. Once set, the `OnceLock` is
the single source of truth for every transport: the HTTP auth middleware
in `openhuman-rpc`, Socket.IO, the SSE query-token fallback, and the approval-gate session id.
`event_bind_tokens.rs` mints the separate short-lived, single-shot tokens
`/events` needs because browser `EventSource` cannot send an `Authorization`
header.

## CLI

`cli.rs` is the CLI entry point (`run_from_cli_args`): it parses arguments,
prints the banner, resolves controller schemas grouped by namespace, and
dispatches through the same registry RPC uses. `agent_cli.rs`,
`memory_cli.rs`, and `subsystems_cli.rs` are domain-specific CLI subcommand
trees. `cli_capability.rs` is the one CLI-only exception to "degradation is
absence": over `/rpc` and in the agent tool list an unadvertised memory
capability family is simply unregistered, but the CLI keeps its subcommand
arm and reports the build/config fact (`capability_unavailable_message`:
which bound memory driver does not advertise which family) so a human does
not mistake silence for a typo. It resolves the binding itself because plain
CLI invocations never build a `CoreContext`, so the ambient gate would
answer "everything allowed".

`run` / `serve` start the JSON-RPC server through the launcher a host
installs in `server_launcher.rs` (`openhuman_rpc::server::install_cli_server`
in the `openhuman-core` binary and the desktop app); without one they fail
and say so.

## `runtime/` and `subsystem/`

`runtime/` is the embeddable composition API: `CoreBuilder` builds a
`CoreRuntime` in two phases (initialization, then serve); see
[runtime/README.md](runtime/README.md) for the full builder reference.
`AGENT_WORKER_STACK_BYTES` (16 MiB) and `MAX_BLOCKING_THREADS` (64) live in
`runtime/mod.rs`: a single agent turn is a very large async state machine,
and delegating to a sub-agent nests another one, which overflows tokio's
default 2 MiB worker stack; every multi-thread runtime that can host a turn
(the desktop shell, `openhuman-core run`, `agent_cli`) sets both constants.

`subsystem/` owns the generic `SubsystemStatus` wire row behind the `subsystems`
RPC namespace and the `openhuman subsystems` CLI table. Memory is the only
occupant: `crate::memory::status` projects the bound Memory v2 engine into its
row, and a later subsystem appends its own adapter call in
`subsystem::schemas::subsystems_status`.

## The controller contract

`Outcome<T>` (`outcome.rs`) is what every domain operation returns: a value
plus log lines, turned into JSON by `Outcome::into_cli_compatible_json`,
whose shape `apply_log_envelope` decides. `StructuredRpcError`
(`structured_error.rs`) is the typed error a controller can return through
its `Err(String)` channel, and `params.rs` defines the params shape and the
validation messages. They live in core so every domain and host shares one
definition; `crates/openhuman-rpc` depends on core for them.

## A note on `#![recursion_limit = "256"]`

`lib.rs` raises the crate's recursion limit because the RPC dispatch
chokepoint wraps each handler future in an ambient `CoreContext::scope`
(`runtime/context.rs`); combined with the already-deep async type stacks in
the Axum routes that fan out into the tinyagents harness, the extra future
layer pushes the compiler's `Send` auto-trait solver past the default depth.

## Notes / gotchas

- `bus::init()` binds the in-process broker's tokio tasks to whatever
  runtime calls it first: a trap under `cargo test`, where every
  `#[tokio::test]` builds its own runtime. Tests that need to observe events
  should use `bus_testing::isolated_bus()` instead of the shared singleton.
- `HostKind::TauriShell` always installs the approval gate at boot and
  ignores `OPENHUMAN_APPROVAL_GATE=0`; `Cli`, `Docker`, and `Library` honor
  the env override (`types::approval_gate_boot_decision`).
- `KNOWN_PROBE_METHODS` in `dispatch.rs` silences recurring but harmless
  unknown-method noise (generic RPC probes, retired calls) from Sentry.
  `openhuman.harness_init_status` is on that list for client/core surface
  skew only: it is a live, registered method, and a dedicated test pins
  that fact so a real regression cannot go silent behind the allow-list.
- `http_server_status::HTTP_SERVER_COMPILED_IN` exists so a build without the
  `http-server` feature fails loudly at compile time if the desktop shell
  (which always needs a listener) tries to link it, rather than shipping
  silently without one.

## Related docs

- [runtime/README.md](runtime/README.md)
- [../security/README.md](../security/README.md)
- [../../../../gitbooks/developing/architecture.md](../../../../gitbooks/developing/architecture.md)
