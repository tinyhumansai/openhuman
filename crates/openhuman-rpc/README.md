# openhuman-rpc

JSON-RPC 2.0 for OpenHuman, on both sides of the wire, and the shared host
boot. The core (`openhuman-core`, package `openhuman`) defines what a
controller is: its schema, the `core::Outcome` it returns, the structured
error envelope, the params rules and in-process dispatch
(`core::invoke::invoke_method`). This crate decides how that is exposed: the
JSON-RPC envelopes, an HTTP client, the HTTP and Socket.IO server, the
`http_host` static file server, the on-disk session store, and `host`, the
boot sequence each host shape runs. The desktop app, the `openhuman-core`
binary and the TUI depend on it; the core, `openhuman-embed` and
`openhuman-tinyhumans` do not.

It is the top of the library chain:

```text
openhuman-core -> openhuman-embed -> openhuman-tinyhumans -> openhuman-rpc -> app / cli / tui
```

Its only openhuman dependency is `openhuman-tinyhumans`. Core internals the
server needs come through embed's doc-hidden `__host` list (aliased
as a private binding, `core_host`, in `src/lib.rs`); nothing here re-exports the
core. Hosts get `openhuman_rpc::tinyhumans` and `openhuman_rpc::embed` (curated
`pub use` lists, not the crates) as their facade, and this crate is
the only OpenHuman crate they depend on (`scripts/ci/check-crate-chain.mjs`).

## How it works

A request to a running core takes this path:

```text
 client side
 app/src coreRpcClient (fetch)
 openhuman-app core_rpc.rs
   request_body + post_json_rpc
        |  POST /rpc
        v
 core process
 axum router (build_core_http_router)
   CoreContext::scope         added by serve
   socket.io layer            if enabled
   cors_middleware            origin allowlist
   rpc_auth_middleware        bearer check
   request log
        |
        v
 rpc_handler
   invoke_method(state, method, params)
        |   core: registry lookup, params rules,
        |         DomainSet gate, controller handler
        v
   Ok(value)   -> RpcSuccess { result }
   Err(string) -> StructuredRpcError::decode
                  classify_failure (how loudly to report)
                  RpcFailure { error.code = -32000 }
        |
        |  HTTP 200 for success and failure alike
        v
 decode_response(status, body) -> Result<Value, String>
```

The server never interprets a method. `rpc_handler` hands the method name and
params to the core's `invoke_method`, which looks up the controller in the
registry (`core/all.rs` plus any registered extensions), validates params,
applies `DomainSet` gating and runs the handler inside the runtime's
`CoreContext`. A failure comes back as a string; the handler decodes the
controller's `StructuredRpcError` envelope if there is one, decides how
loudly to report it (`server/classify.rs`), and answers with a JSON-RPC error
whose code is always `SERVER_ERROR_CODE` (`-32000`). Every controller
failure is sent with HTTP 200, so `decode_response` lets an `error` member win
over the status.

Socket.IO clients take a parallel path: an authenticated socket sends
`rpc:request`, the server calls the same `invoke_method`, and replies with
`rpc:response` or `rpc:error`. The same socket carries live domain events the
other way. See [`src/server/README.md`](src/server/README.md).

The core does not depend on this crate, so the server is wired in from the
host side. `host` has one entry per host shape; the older entry points stay
for the hosts that still call them:

```text
 host::cli(args)        tinyhumans cli preset, connected
                        + server launcher + http_host controllers
                        -> run_from_args (core CLI run/serve -> run_server*)
 host::desktop(opts, shutdown, ready)
                        tinyhumans desktop preset, connected
                        + bearer, listener, services, launcher, http_host
                        -> build_and_serve
 host::tui()            tinyhumans tui preset, connected + session store
                        -> embed Runtime (no server)

 bare:   run_server / run_server_headless / run_server_saas (embed cli
         preset, not connected; the caller installs the transport itself)

 servers end in: session_store::install(); RuntimeBuilder::build(); serve(..)
```

## Layout

| Path | What it does |
| --- | --- |
| [`src/lib.rs`](src/lib.rs) | Module wiring and curated re-exports (`tinyhumans` and `embed` item lists, the client helpers, `unwrap_rpc`). |
| [`src/host.rs`](src/host.rs) | `server` / `session-store` features: the shared host boot — `cli`, `desktop` / `serve_desktop`, `tui`, and the `*_builder` each starts from. |
| [`src/envelope.rs`](src/envelope.rs) | `RpcRequest`, `RpcSuccess`, `RpcFailure`, `RpcError`, `JSONRPC_VERSION`, `SERVER_ERROR_CODE`, and the client half: `request_body`, `decode_response`. |
| [`src/origin.rs`](src/origin.rs) | `is_origin_allowed_with_extra` and `ALLOWED_ORIGINS_ENV`: the browser-origin allowlist. Pure; the caller reads the environment. |
| [`src/client.rs`](src/client.rs) | `http-client` feature: `post_json_rpc`, `bearer_header`, `redact_url_for_log`, `HttpRpcResponse`. |
| [`src/server/`](src/server/README.md) | `server` feature: the axum router, auth and CORS middleware, the `/rpc` handler, SSE and WebSocket routes, Socket.IO, `/dev/connect`, the listener (`serve`) and the `run_server*` entry points. |
| [`src/http_host/`](src/http_host/README.md) | `server` feature: ad-hoc Basic-auth static directory servers and their `http_host.*` controllers. |
| [`src/session_store/`](src/session_store/README.md) | `session-store` feature: `SqliteSessionStores`, the classic on-disk session layout behind TinyAgents' session store port. |

## Key types and entry points

- `RpcRequest` / `RpcSuccess` / `RpcFailure` (`envelope.rs`): the wire
  shapes the server reads and writes.
- `request_body(id, method, params)` and `decode_response(status, body)`
  (`envelope.rs`): what a client uses to build a request and get back
  `Result<Value, String>`.
- `host::cli(args)`, `host::desktop(options, shutdown, ready_tx)`,
  `host::tui()` (`host.rs`): the boot each host shape runs, built on
  `openhuman_tinyhumans::RuntimeBuilder` presets.
- `unwrap_rpc`: re-exported from the core's `core` module; reaches a handler's
  value through its `result` / `data` envelopes. The TUI decodes with it.
- `post_json_rpc(url, token, body)` (`client.rs`): POSTs a body with an
  optional bearer and returns status and body verbatim.
- `server::serve(&CoreRuntime, ready_tx, shutdown)` (`server/serve.rs`):
  bind and serve an already-built runtime.
- `server::run_server`, `run_server_headless`, `run_server_saas`
  (`server/shims.rs`): build a runtime and serve it: `run_server` and `run_server_headless` from the embed `cli` preset, `run_server_saas` from the SaaS config through `core::runtime::saas::build`. They do not connect the
  TinyHumans backend; `host::desktop` does.
- `server::build_core_http_router(socketio_enabled)` (`server/http/mod.rs`):
  the router on its own; root `tests/*.rs` suites use it to make real HTTP
  calls in-process.
- `session_store::install()` (`session_store/mod.rs`): install the on-disk
  session store before the core boots.

## Feature flags

| Feature | Pulls in | Used by |
| --- | --- | --- |
| `http-client` (default) | `reqwest` with `rustls-tls`, for `client.rs`. | `openhuman-app` |
| `server` (default) | `axum`, `socketioxide`, the tokio stack; turns on the `http-server` gate (the `/v1` inference router and dictation WebSocket the router mounts) and `session-store`. | `openhuman-app`, `openhuman-cli` |
| `session-store` | `tinyagents-session`, `tinyagents-harness`. | `openhuman-tui` on its own; implied by `server` |
| `jev` | `openhuman-tinyhumans/jev`: the Jev `tool_search` ranker `host` wires in. | hosts |
| product gates (`channels`, `voice`, `mcp`, `crash-reporting`, ...) | Forwarded 1:1 to `openhuman-tinyhumans`, which forwards them to embed and the core. `crash-reporting` also runs the Sentry-routing tests. | hosts, tests |

`http-client`, `server` and `session-store` are this crate's own gates
(`CHAIN_LOCAL_GATES` in `scripts/lib/feature-forwarding.mjs`); every other
gate must forward to the same gate on `openhuman-tinyhumans`, which
`scripts/ci/check-feature-forwarding.mjs` enforces.

The root workspace declares this crate with `default-features = false`, so
each consumer names what it needs. `openhuman-cli` enables `server`,
`openhuman-tui` enables `session-store` only (it runs the core without a
server), and [`crates/openhuman-app/Cargo.toml`](../openhuman-app/Cargo.toml), outside the workspace,
enables `http-client`, `server`, `jev` and the product gates. Each host also
forwards its own feature gates here, 1:1, so `e2e-test-support`, the
`storage-*` drivers and every product gate reach the core through this crate.

## Consumers

- [`crates/openhuman-app`](../openhuman-app/): `core_process.rs` runs the embedded server
  (`host::desktop`) and reads its `EmbeddedReadySignal`;
  `core_rpc.rs` wraps `post_json_rpc` to reach the embedded core and
  self-hosted runtimes (#3865); `session/link.rs` and `local_data_reset.rs`
  build requests with `request_body` and decode with `decode_response`;
  `lib.rs::run_core_from_args` calls `host::cli`; the rest of the shell uses
  the `embed` and `tinyhumans` re-exports.
- [`crates/openhuman-cli`](../openhuman-cli/): `main.rs` calls `host::cli`; root
  `tests/*.rs` suites (for example `json_rpc_e2e.rs`) build the router with
  `build_core_http_router`.
- [`crates/openhuman-tui`](../openhuman-tui/): `unwrap_rpc` is its decode point, and
  `runner.rs` boots with `host::tui()`.

## Boundaries

- No business logic. Controller semantics (results, errors, params,
  dispatch, session expiry) belong to the core. This crate frames them as
  JSON-RPC and decides transport policy: which routes need the bearer, which
  origins may call, and how loudly a failure is reported.
- The core does not depend on this crate, and this crate does not depend on
  the core directly: it reaches core internals only through embed's
  `__host` list, and never re-exports them. Anything a domain needs belongs
  in the core. Domain-owned HTTP handlers the router mounts
  (`inference::http`, the dictation and live-voice WebSocket sessions) stay
  in their core domains behind the `http-server` gate.
- Controllers are registered in the core (`core/all.rs`), never by adding
  method branches to the server. `http_host` joins the registry as a
  controller extension.
- The session store port, transcript format and run ledger belong to
  [`vendor/tinyagents`](../../vendor/tinyagents/) (`tinyagents-session`). `session_store` only arranges
  those building blocks into OpenHuman's layout.
- Backend auth, login-token exchange and `/auth/me` belong to
  `openhuman-tinyhumans`, not here.

## Gotchas

- `/rpc` has a 64 MiB body limit (`MAX_RPC_BODY_BYTES`), sized for a
  `channel_web_chat` turn carrying four base64 images. Other routes keep
  axum's 2 MiB default.
- `serve` refuses to bind a non-loopback address unless the operator
  supplied an RPC token (`OPENHUMAN_CORE_TOKEN` or an in-memory bearer). The
  generated `core.token` file does not count, since remote clients cannot
  read it.
- `serve` sets `OPENHUMAN_CORE_RPC_URL` to the port it actually bound, which
  is process-global state and another reason there is one runtime per
  process. The servers build an embed `Runtime`, which claims the process's
  single runtime slot until it drops (when `serve` returns), so a second
  concurrent server, or an embed runtime alongside one, fails with
  `AlreadyRunning`.
- Two origin checks exist. CORS uses `is_origin_allowed_with_extra`;
  the Socket.IO handshake has its own, slightly wider check. Both read
  `OPENHUMAN_CORE_ALLOWED_ORIGINS`.
- `http_host` responses carry the generated Basic-auth password. Treat
  `http_host.*` output as sensitive.

## Tests

Sibling `*_tests.rs` files cover the envelopes (including the exact
server-failure wire bytes), the origin allowlist, failure classification,
the `/rpc` handler's Sentry routing (with `crash-reporting`), CORS, auth
route policy, the SSE and WebSocket routes, Socket.IO, `/dev/connect`,
`http_host` and the session store. End-to-end JSON-RPC behavior lives in the
root [`tests/json_rpc_e2e.rs`](../../tests/json_rpc_e2e.rs).

```bash
cargo test -p openhuman-rpc --features crash-reporting
cargo test -p openhuman-cli --test json_rpc_e2e
pnpm debug rust openhuman_rpc
```

## Further reading

- [`docs/gitbooks/en/developing/architecture.md`](../../docs/gitbooks/en/developing/architecture.md): architecture overview.
- [`docs/gitbooks/en/developing/architecture/tauri-shell.md`](../../docs/gitbooks/en/developing/architecture/tauri-shell.md): the Tauri shell.
- [`docs/gitbooks/en/developing/embedding.md`](../../docs/gitbooks/en/developing/embedding.md): embedding the core in another product.
- [`docs/gitbooks/en/developing/testing-strategy.md`](../../docs/gitbooks/en/developing/testing-strategy.md): testing strategy.
- [`crates/README.md`](../README.md): crates overview.
