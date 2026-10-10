# core

`core/` is the kernel of the `openhuman` crate. It defines the controller
contract every domain implements, keeps the one registry of those
controllers, dispatches calls to them in-process, and owns the process-wide
event bus, the RPC bearer token, the CLI, logging and crash reporting, and
the `CoreBuilder` runtime composition API. It holds no business logic: every
controller it exposes is implemented by a domain under
`crates/openhuman-core/src/<domain>/` and only wired in here.

Callers are the hosts above the core. The JSON-RPC server in
[`crates/openhuman-rpc`](../../../openhuman-rpc/), the `openhuman-core` binary in [`crates/openhuman-cli`](../../../openhuman-cli/),
the desktop shell, the TUI and `openhuman-embed` all reach domain behavior
through the entry points in this folder.

## How it works

### The controller contract

A domain exposes an operation by building a `RegisteredController`: a
`ControllerSchema` (namespace, function, description, typed inputs and
outputs) paired with a `ControllerHandler`, which is a plain
`fn(Map<String, Value>) -> ControllerFuture`. The schema types
(`ControllerSchema`, `FieldSchema`, `TypeSchema`) live in [`mod.rs`](./mod.rs).
`TypeSchema` covers scalars, arrays, maps, options, string enums, nested
objects, named refs, and `BoundedU64` for fields backed by a narrower
integer type, so an out-of-range value is refused before dispatch.

Inside the handler, the domain's `ops.rs` returns an `Outcome<T>`
([`outcome.rs`](./outcome.rs)): a value plus log lines. `Outcome::into_cli_compatible_json`
serializes it through `apply_log_envelope`, which is the single definition of
the wire shape. With no logs the value goes out bare; with logs it is wrapped
as `{ "result": value, "logs": [...] }`. That means a response's shape depends
on its log vector rather than its schema (issue #6080). The rule is kept
byte-for-byte on purpose, because changing it is a wire change for every
client. `unwrap_rpc` peels `result` and `data` envelopes back off.

Errors travel as `Err(String)`. When a controller needs a typed error, it
encodes a `StructuredRpcError` ([`structured_error.rs`](./structured_error.rs)) into that string behind
the `STRUCTURED_RPC_ERROR_SENTINEL` prefix, and the transport decodes it. A
few surfaces (`threads`, desktop provider surfaces) wrap results in the
`{data, error, meta}` envelope built by [`envelope.rs`](./envelope.rs).

### Registration

[`all.rs`](./all.rs) is the registry. `build_registered_controllers` calls every domain's
`all_*_registered_controllers()` and tags each batch with a `DomainGroup`
through the private `push` helper, so domain modules never see groups.
`build_internal_only_controllers` does the same for methods that are callable
over RPC but hidden from schema discovery and agent tool lists (desktop
control, the MCP write audit, the `modules` namespace). Both registries are
built lazily on first access, and `validate_registry` panics at that moment
on duplicate controllers, duplicate RPC method names, empty names, or
repeated required inputs.

Crates above the core add controllers at runtime through
`register_controller_extension(ControllerExtension { group, controllers,
namespaces })`. `openhuman-tinyhumans` uses it for the hosted backend proxies
(`DomainGroup::Hosted`) and `openhuman-rpc` uses it for the `http_host`
static file server. Extensions are validated against the union of the
built-in, internal and previously extended sets, re-registering an identical
set is a no-op, and the only ordering rule is to install before the first
dispatch of one of the extension's methods.

```text
 domain/schemas.rs                    host crates above core
 all_*_registered_controllers()       register_controller_extension()
          |                                     |
          v                                     v
 +-----------------+  +------------------+  +------------+
 | REGISTRY        |  | INTERNAL_REGISTRY|  | EXTENSIONS |
 | (agent-visible) |  | (RPC only)       |  |            |
 +-----------------+  +------------------+  +------------+
          \                  |                  /
           +---- filtered by the ambient DomainSet ----+
                             |
                 /schema, dispatch, agent tools
```

Every entry carries a `DomainGroup`: `Agent`, `Memory`, `Threads`, `Config`,
`Security`, `Flows`, `Skills`, `Mcp`, `Channels`, `Web3`, `Voice`, `Media`,
`Inference`, `Integrations`, `Automation`, `Runtimes`, `Desktop`, `Hosted`,
`Modules` and `Platform`. The variants track the family directories under
[`crates/openhuman-core/src/`](../). The live surface (schema listing, dispatch,
agent tools, stores, subscribers) is filtered by whether the `DomainSet` of
the ambient `runtime::context::CoreContext` allows the group. With no context
yet (some unit tests), nothing is filtered. Adding a family directory means
adding a `DomainGroup` variant, a `DomainSet` field, an arm in
`DomainSet::allows()` and an entry in each preset; the compiler enforces all
four, and `DomainGroup::ALL`, `COUNT` and `index()` are pinned by a test.

Names come in two forms that must not be confused. `ControllerSchema::method_name`
gives the dotted key (`memory.doc_put`); `rpc_method_name` gives the wire
name (`openhuman.memory_doc_put`). RPC namespace strings are wire contracts
and do not follow directory renames.

### Dispatch

Every transport calls `invoke::invoke_method(state, method, params)`: the
`/rpc` handler and Socket.IO in `openhuman-rpc`, the CLI's `call` and
namespace commands, the device tunnel, and `CoreRuntime::invoke`.

```text
 invoke_method(method, params)
   |
   +-- schema_for_rpc_method(method)  (built-in, internal, extensions)
   |     found: params_to_object -> validate_params -> try_invoke_registered_rpc
   |               |  group disabled under DomainSet -> treated as unknown
   |               |  else run handler inside CoreContext::scope
   |               v
   |             result
   |
   +-- not found (or no handler): dispatch::dispatch
   |      tier 0  legacy_aliases::resolve_legacy rewrites retired names
   |      tier 1  core.ping, core.version, core.events_subscribe_token
   |      tier 2  registry lookup again (catches rewritten names)
   |      tier 3  Err("unknown method: <name>")
   |
   +-- on Err: is_session_expired_error?
          yes, backend credential -> BUS.publish(SessionExpired)
          yes, local credential   -> BACKEND_UNAVAILABLE: error instead
          401 that is not confirmed -> logged, session left alone
```

`validate_params` (in `all.rs`, with message builders and the matcher
`is_param_validation_error` in [`params.rs`](./params.rs)) rejects missing required params,
unknown params and values that do not match their declared `TypeSchema`, so
handlers only deserialize input that already passed the schema.

The `DomainSet` gate is enforced in `try_invoke_registered_rpc`, not in
schema lookup: a gated method answers exactly like an unregistered one. The
handler future is wrapped in `CoreContext::scope` and re-boxed as a
`ControllerFuture`; that type erasure keeps the `Send` solver from
overflowing on the largest handler futures, and together with the deep axum
and harness futures it is also why `lib.rs` sets
`#![recursion_limit = "256"]`.

Unknown methods always fail with the `UNKNOWN_METHOD_PREFIX` error. The
transport decides severity: names in `KNOWN_PROBE_METHODS` (generic probes
such as `rpc.discover`, and retired calls) stay debug-only, and anything else
is reported to Sentry at warn.

Session expiry is classified narrowly by [`session_expiry.rs`](./session_expiry.rs). Only OpenHuman
backend failures (explicit "Session expired" text, the scheduler-gate
`SESSION_EXPIRED` sentinel, missing session guards, and 401s formatted as
`"{METHOD} /path failed (401 ...)"`) count. A provider's 401 does not, so a
broken BYOK key or Discord token never signs the user out.

### Event bus

[`bus.rs`](./bus.rs) declares `BUS: OnceBus<DomainEvent>`, the process-wide `tinybus`
singleton. The bus implementation lives in the `tinybus` submodule; the core
keeps only the static, because `OnceBus<E>` is generic over the event type
and the host is the one that knows `E`. [`events.rs`](./events.rs) is the event catalog:
`DomainEvent`, a `#[non_exhaustive]` enum whose `domain()` value is appended
to `EVENTS_ROOT` (`/ai/tinyhumans/openhuman/events`) as the routing key.
The catalog is published under `EVENTS_INTERFACE`
(`ai.tinyhumans.openhuman.Events`) at `EVENTS_VERSION`, currently `1.10.0`.

`bus::init()` starts the in-process broker and announces a peer manifest
(advisory; failure only logs). Every accessor is safe before `init`:
publishing is a no-op logged at trace and subscribing returns `None`.
`TracingSubscriber` (`core::bus::tracing`) logs every event at debug.

Pick the surface by what the call carries:

| Need | Use |
| --- | --- |
| tell anyone who cares that something happened | `BUS.publish` |
| call another module with a payload that is plain data | `BUS.publish` plus a reply event, or a native request |
| call another module passing `Arc`s, trait objects, or channels | `BUS.native()` |
| call an out-of-process integration | a `tinybus::Proxy` off `BUS.get()?.connection()` |

The native registry is in-process by design. Types such as
`AgentTurnRequest` carry `Arc<Vec<Box<dyn Tool>>>` and live progress channels
that cannot cross a serialized transport. Native request and response types
must be `Send + 'static` and need no serialization.

Adding an event takes four steps: add the variant to `DomainEvent`, extend
the `domain()` match, register its subscriber at startup, and bump
`EVENTS_VERSION`. Bump the minor for an added variant or field; for a change
an older subscriber cannot parse, bump the major and rename the interface.
Each subscribing domain owns a `bus.rs`, and subscriber names use
`<domain>::<purpose>`.

### Auth

[`auth.rs`](./auth.rs) seeds the per-process RPC bearer into a `OnceLock`, choosing the
first available source:

1. an in-memory handoff from the desktop shell (`init_rpc_token_with_value`),
   which never touches the process environment;
2. the `OPENHUMAN_CORE_TOKEN` env var (`CORE_TOKEN_ENV_VAR`), for Docker and
   cloud operators;
3. a freshly generated token written owner-read-only to
   `{workspace_dir}/core.token` by `init_rpc_token`, for standalone CLI
   clients.

Once set, that value is what every transport checks: the HTTP middleware in
`openhuman_rpc::server::auth`, Socket.IO, the SSE query-token fallback, and
the approval-gate session id. `verify_bearer_token` and `bearer_matches` do
the comparison.

Browser `EventSource` cannot send an `Authorization` header, so `/events`
uses separate bind tokens from [`event_bind_tokens.rs`](./event_bind_tokens.rs). An authenticated
caller invokes `core.events_subscribe_token { client_id }` and then opens
`/events?client_id=<id>&token=<bind>`. A bind token is 256 random bits, tied
to one `client_id`, consumed on first use, and expires after 60 seconds by
default (30 minutes at most). The store caps at 4096 live tokens.

This is the RPC transport bearer only. User credentials (session JWT, API
key, offline local token) arrive through `auth.set_credential` in
`security::credentials`, not here.

### CLI

`cli::run_from_cli_args` is the entry point for the `openhuman-core` binary.
It loads `.env` (`load_dotenv_for_cli`), applies the process-local
`--provider` / `--model` overrides, prints the banner to stderr (except for
`mcp`, `mcp-server`, `tui` and `chat`, which keep stdout clean), and matches
the first argument:

| Subcommand | Behavior |
| --- | --- |
| `run`, `serve` | start the JSON-RPC server through the installed `ServerLauncher` |
| `mcp`, `mcp-server` | run the stdio MCP server (`mcp::server::run_stdio_from_cli`) |
| `tui`, `chat` | print a pointer to the separate `openhuman-tui` binary |
| `call --method <m> --params <json>` | one raw RPC call through `invoke_method` (`--params-stdin` reads params from stdin) |
| `agent dump-prompt / dump-all / prompt-size / list` | prompt inspection tools in [`agent_cli.rs`](./agent_cli.rs) |
| `sentry-test` | send a test event to Sentry |
| `<namespace> <function> [--flags]` | generic dispatcher over the registry schemas |

The generic dispatcher groups registered schemas by namespace, prints help
from their descriptions (`namespace_description` in `all.rs`), and turns
flags into params. A namespace with no function can fall through to a
standalone `RegisteredCliAdapter`: today `voice` (which stays registered with
the `voice` feature off and reports that voice is disabled) and `subsystems`
([`subsystems_cli.rs`](./subsystems_cli.rs), which prints the subsystem table). Wire new CLI
behavior through the registry; do not add namespace branches to [`cli.rs`](./cli.rs) or
the JSON-RPC server.

`run` and `serve` need a server, which the core does not contain.
[`server_launcher.rs`](./server_launcher.rs) holds a `ServerLauncher` port; the host installs one
(`openhuman_rpc::host::cli` for the `openhuman-core` binary, the desktop
builder for the app) before calling `run_from_cli_args`. Without it those
subcommands fail with an explanation.

### Logging, redaction and crash reporting

[`logging.rs`](./logging.rs) sets up the logger per host: `init_for_cli_run`,
`init_for_embedded` and `init_for_tui` (which also buffers lines for the TUI
through `tui_log_lines`). Two redaction passes keep secrets out of logs.
`rpc_log::redact_params_for_log` strips sensitive keys (`api_key`, `token`,
`authorization`, `password`, `client_secret` and similar) from structured
params before the `[rpc:dispatch]` trace line. `log_redaction::scrub_secrets`
pattern-matches secrets in free text and is shared by the Sentry path and the
always-on log path, so the two cannot drift.

[`observability.rs`](./observability.rs) is the error-reporting policy: `report_error`,
`report_error_or_expected` and `report_warning_message`, the expected-error
classifier (`expected_error_kind`), and the Sentry `before_send` predicates
that drop deterministic noise (transient provider HTTP statuses and transport
failures, updater blips, session expiry, max-iteration events). It also
defines the shared `BACKEND_UNAVAILABLE_PREFIX`. [`sentry_transport.rs`](./sentry_transport.rs) is a
bounded, nonblocking Sentry transport over the core's reqwest stack, compiled
only with the `crash-reporting` feature.

[`shutdown.rs`](./shutdown.rs) listens for SIGINT and SIGTERM (`signal`) and runs cleanup
hooks that domains `register`, sequentially in registration order.

### Runtime composition and subsystems

`runtime/` is the embeddable composition API. `CoreBuilder::build()` does
initialization only (controllers, master key, RPC bearer, workspace stores,
event subscribers) and returns a `CoreRuntime` that can already `invoke`
methods and run agent turns. Serving and background services come second,
through `openhuman_rpc::server::serve` or `CoreRuntime::start_services`.
`ServiceSet`, `DomainSet` and `ToolGroups` narrow what runs. See
[runtime/README.md](runtime/README.md) for the full reference.
[`runtime/mod.rs`](./runtime/mod.rs) also holds `AGENT_WORKER_STACK_BYTES` (20 MiB) and
`MAX_BLOCKING_THREADS` (64). One agent turn is a very large async state
machine and a sub-agent nests another, which overflows tokio's default 2 MiB
worker stack, so every multi-thread runtime that can host a turn (the desktop
shell, `openhuman-core run`, `agent_cli`) sets both.

`subsystem/` serves the `subsystems` RPC namespace and its CLI table: one
`SubsystemStatus` row per kernel capability slot (class, health, contract
version, advertised capabilities). It is the one controller registered from
`core/` itself, tagged `DomainGroup::Platform`, because the subsystem table is
a kernel binding table like the controller registry. Memory is the only
occupant today; `crate::memory::status` builds its row, and a new subsystem
appends its adapter call in `subsystem::schemas::subsystems_status`.

## Layout

Contract and registry:

| Path | What it does |
| --- | --- |
| `mod.rs` | `ControllerSchema`, `FieldSchema`, `TypeSchema`; module declarations; re-exports `Outcome` and `StructuredRpcError`. |
| `outcome.rs` | `Outcome<T>`, `apply_log_envelope` (the one wire-shape rule) and `unwrap_rpc`. |
| `structured_error.rs` | `StructuredRpcError`, a typed error sentinel-encoded into a controller's `Err(String)`. |
| `envelope.rs` | `ApiEnvelope`, `ApiError`, `ApiMeta`, `PaginationMeta`: the `{data, error, meta}` shape some namespaces use. |
| `params.rs` | Params shape (`params_to_object`, `parse_json_params`), validation messages and `is_param_validation_error`. |
| `all.rs` | The registry: `RegisteredController`, `RegisteredCliAdapter`, `DomainGroup`, `ControllerExtension`, `register_controller_extension`, `validate_params`, `schema_for_rpc_method`, `try_invoke_registered_rpc`, `all_http_method_schemas` (adds `core.ping` and `core.version` for `/schema`), `namespace_description`. |
| [`types.rs`](./types.rs) | `AppState`, `HostKind`, `InvocationResult`, `approval_gate_boot_decision`. |

Dispatch and auth:

| Path | What it does |
| --- | --- |
| [`invoke.rs`](./invoke.rs) | `invoke_method` and `default_state`: the in-process entry point and the session-expiry hook. |
| [`dispatch.rs`](./dispatch.rs) | `dispatch`: legacy rewrite, `core.*` methods, registry fallback, unknown-method handling, `KNOWN_PROBE_METHODS`. |
| [`legacy_aliases.rs`](./legacy_aliases.rs) | `resolve_legacy`: retired method names to canonical ones; mirrors `LEGACY_METHOD_ALIASES` in [`app/src/services/rpcMethods.ts`](../../../../app/src/services/rpcMethods.ts). |
| `session_expiry.rs` | `is_session_expired_error` and `is_unconfirmed_unauthorized_error`. |
| `auth.rs` | The per-process RPC bearer: init paths, `get_rpc_token`, `verify_bearer_token`, `bearer_matches`. |
| `event_bind_tokens.rs` | Single-shot bind tokens for the `/events` SSE stream (`issue`, `consume`). |
| `server_launcher.rs` | `ServerLauncher` port that `run` / `serve` start a server through. |
| `http_server_status.rs` | `HTTP_SERVER_COMPILED_IN`, an ungated marker the desktop shell asserts so a core built without `http-server` fails the build. |

Bus:

| Path | What it does |
| --- | --- |
| `bus.rs` | `BUS`, `EVENTS_ROOT`, `EVENTS_INTERFACE`, `EVENTS_VERSION`, `init`, `manifest`, `TracingSubscriber`. |
| `events.rs` | `DomainEvent`, the full event catalog, and its `domain()` routing. |
| [`bus_testing.rs`](./bus_testing.rs) | Test helpers: `isolated_bus`, `BUS_HANDLER_LOCK`, `mock_bus_stub` and the RAII `MockBusGuard` for native stubs. |

CLI:

| Path | What it does |
| --- | --- |
| `cli.rs` | `run_from_cli_args`, launch flags, help output, `call` and the generic namespace dispatcher. |
| `agent_cli.rs` | `openhuman-core agent ...`: `dump-prompt`, `dump-all`, `prompt-size`, `list`. |
| `subsystems_cli.rs` | `run_subsystems_command`, the table printed by bare `openhuman-core subsystems`. |

Process plumbing:

| Path | What it does |
| --- | --- |
| `logging.rs` | Logger setup per host kind, log directory, file-guard shutdown. |
| [`rpc_log.rs`](./rpc_log.rs) | `redact_params_for_log`, key-name redaction for dispatch trace logs. |
| [`log_redaction.rs`](./log_redaction.rs) | `scrub_secrets`, pattern-based secret scrubbing for free-text log and Sentry messages. |
| `observability.rs` | Error reporting, expected-error classification, Sentry `before_send` filters, shared error prefixes. |
| `sentry_transport.rs` | Sentry transport over reqwest (`crash-reporting` feature). |
| `shutdown.rs` | SIGINT/SIGTERM signal and the registered shutdown hooks. |
| `runtime/` | `CoreBuilder`, `CoreRuntime`, `CoreContext`, `ServiceSet`, `DomainSet`, bootstrap and subscribers. See [runtime/README.md](runtime/README.md). |
| `subsystem/` | `SubsystemStatus` and the `subsystems` controller. |

## Key types and entry points

- `ControllerSchema` / `TypeSchema` (`mod.rs`): what a domain declares for each operation; drives validation, `/schema`, CLI help and agent tool schemas.
- `RegisteredController` (`all.rs`): schema plus handler, the unit every domain's `all_*_registered_controllers()` returns.
- `Outcome<T>` (`outcome.rs`): the return type of every domain operation.
- `StructuredRpcError` (`structured_error.rs`): a typed error through the string error channel.
- `invoke::invoke_method`: call any method in-process with validation and session-expiry handling.
- `all::register_controller_extension`: how a crate above the core adds a namespace.
- `BUS` and `DomainEvent` (`bus.rs`, `events.rs`): publish, subscribe and native requests.
- `auth::init_rpc_token` / `init_rpc_token_with_value`: seed the transport bearer.
- `cli::run_from_cli_args`: the CLI entry point a host binary calls.
- `HostKind` (`types.rs`): which host booted the core; affects approval-gate policy and log tags.
- `runtime::CoreBuilder`: compose a runtime for an embedding host.

## RPC / CLI surface

Methods handled directly in `dispatch.rs`, with no registry entry:

- `core.ping`: liveness, returns `{ "ok": true }`.
- `core.version`: the running core's version.
- `core.events_subscribe_token`: mint a bind token for `/events`.

Registered from `core/`:

- `openhuman.subsystems_status`: one `SubsystemStatus` row per kernel slot (CLI: `openhuman-core subsystems`).

Every other method belongs to a domain and is listed in that domain's README.
`GET /schema` on a running server and `openhuman-core --help` list the live
surface.

## Boundaries

- Business logic belongs in `crates/openhuman-core/src/<domain>/`. Do not add
  domain behavior or new namespaces to `core/`; register them in `all.rs`.
- The JSON-RPC 2.0 server (axum router, `/rpc`, `/health`, `/schema`,
  `/events`, `/oauth/mcp/callback`, auth middleware, CORS, Socket.IO,
  `/dev/connect`, listener bind, `run_server*`) lives in `crates/openhuman-rpc`.
  Core has no server and does not depend on that crate. Domain-owned HTTP
  handlers the router mounts (`inference::http`, the dictation WebSocket)
  stay in their domains behind core's `http-server` feature.
- The bus runtime, broker, native registry and proxies belong to the
  `tinybus` submodule ([`vendor/tinybus`](../../../../vendor/tinybus/)). Only the `BUS` static and the event
  catalog live here.
- Hosted backend proxies (`billing`, `team`, `referral`, `announcements`,
  `webhooks`, `channel_link`, `oauth`) are registered by
  [`crates/openhuman-tinyhumans`](../../../openhuman-tinyhumans/) as an extension, not built into the core.
- Login, token exchange and `/auth/me` belong to the host's session owner
  (`openhuman_tinyhumans::session`, `openhuman_embed::Auth`). Credential
  storage and `auth.set_credential` are in `security::credentials`.
- The CLI binary itself and its developer bins are in `crates/openhuman-cli`;
  this folder only provides the argument handling it calls.

## Gotchas

- `bus::init()` binds the broker's tokio tasks to whichever runtime calls it
  first. Under `cargo test` each `#[tokio::test]` has its own runtime, so the
  singleton can end up attached to a dead reactor. Tests that observe events
  should use `bus_testing::isolated_bus()`, and tests that stub native
  handlers should use `mock_bus_stub` under `BUS_HANDLER_LOCK`.
- `HostKind::TauriShell` always installs the approval gate and ignores
  `OPENHUMAN_APPROVAL_GATE=0`; `Cli`, `Docker` and `Library` honor it
  (`approval_gate_boot_decision`). `HostKind::detect_standalone` picks
  `Docker` from `/.dockerenv` or `OPENHUMAN_DOCKER=1`; the shell must pass
  `TauriShell` explicitly.
- `openhuman.auth_clear_session` is a legacy alias whose params get
  `kind: "session"` filled in during dispatch, preserving its session-only
  contract while the canonical method clears every kind by default.
- Registry lookups for validation and CLI routing search the full,
  unfiltered registry; only dispatch applies the `DomainSet` gate. A gated
  method therefore passes validation and then reports as unknown.
- The log-dependent response shape from `apply_log_envelope` is deliberate.
  Adding a log line to a handler that had none changes its wire shape.
- `http_server_status::HTTP_SERVER_COMPILED_IN` is ungated on purpose: the
  desktop shell asserts it, so a core built without `http-server` fails at
  compile time instead of shipping without a listener.

## Tests

Tests live beside their modules as `<module>_tests.rs` ([`core_mod_tests.rs`](./core_mod_tests.rs)
for `mod.rs`; the registry, observability and event catalog have several
split files). Run them with:

```bash
cargo test -p openhuman core::
pnpm debug rust core::
```

## Related docs

- [runtime/README.md](runtime/README.md)
- [../security/README.md](../security/README.md)
- [Architecture overview](../../../../gitbooks/developing/architecture.md)
- [Parent module README](../../README.md)
- [Building the Rust core](../../../../gitbooks/developing/building-rust-core.md)
- [Embedding OpenHuman](../../../../gitbooks/developing/embedding.md)
- [openhuman-rpc crate](../../../openhuman-rpc/README.md)
