# server

The core's JSON-RPC server, compiled with the crate's `server` feature. It
binds the HTTP listener for a `CoreRuntime`, serves `POST /rpc` and the
surrounding routes (health, schema, SSE, WebSockets, the OpenAI-compatible
`/v1` router, OAuth callbacks, `/dev/connect`), and bridges live domain
events onto Socket.IO for the desktop webviews. The desktop app's embedded
core and `openhuman-core run` / `serve` both run it. Dispatch itself is the
core's: every transport here resolves a method through
`openhuman::core::invoke::invoke_method`.

## How it works

### Starting a server

There are two ways in, and both end in `serve`. (`crate::host::cli` and
`host::desktop` are the same two paths with the TinyHumans backend
connected on the builder; `host::cli` hands that builder to the core CLI and
`host::desktop` enters at `build_and_serve`.)

```text
 openhuman-core run|serve                 desktop shell (core_process.rs)
   RuntimeBuilder::run_from_args(args)      host::desktop(options, cancel, ready_tx)
     -> core::run_core_from_args_with         desktop_builder(options)
        (args, HostBoot(builder))             -> serve_desktop(builder, ..)
   -> core::server_launcher                     |
   launch(ServeRequest)                         |
     --mode saas? run_server_saas (own boot)    |
     else base = HostBoot builder               |
                 or RuntimeBuilder::cli()       |
          flagged_builder(base, flags)          |
             \                                  /
              v                                v
          (desktop path: desktop_builder in host.rs)
            preset: RuntimeBuilder::desktop() (HostKind::TauriShell)
                    both presets: DomainSet::full, discovered config,
                    OPENHUMAN_E2E set -> ToolGroups::advertised()
            services, TokenSource::Fixed(bearer) if handed in (else
                    EnvOrFile), listen host/port if given
          build_and_serve(builder, ready_tx, shutdown_token)
            session_store::install_for_host()   (before boot: recovery; skipped when
                                                 the builder carries its own store)
            RuntimeBuilder::build()             (claims the embed runtime slot)
              |
              v
            serve(runtime.core_runtime(), ready_tx, shutdown_token)
            drop(runtime)                       (releases the slot)
```

The CLI path boots from the builder the host handed `run_from_args`, or the
`cli` preset when there is none. Precedence, highest first: the operator's
explicit flags (`--host`, `--port`, `--jsonrpc-only`, `--headless-api`), the
host's builder, the preset. `--headless-api` replaces the services with
`ServiceSet::headless_api()` (request/response only, no detached background
jobs); `--jsonrpc-only` only clears Socket.IO on the builder's services.
`--mode saas` ignores the builder and boots the operator's config.
`run_server` (a bare shim, not connected to the backend) uses
`ServiceSet::desktop()` with Socket.IO set by the caller. An embedder that
built its own `CoreRuntime` can call `serve` directly.

`serve` then:

1. Returns right after `start_services()` if the runtime did not select
   `ServiceSet::rpc_http` (a harness-only embedder has no transport).
2. Resolves host and port: the builder's values, else `OPENHUMAN_CORE_HOST`
   / `OPENHUMAN_CORE_PORT`, else `127.0.0.1:7788`.
3. Refuses a public bind without an operator-supplied RPC token
   (`OPENHUMAN_CORE_TOKEN` or an in-memory bearer). The generated
   `core.token` file does not count.
4. Picks a port with `pick_listen_port_for_host_with`. With `ready_tx` (the
   desktop shell) another core already on the port is taken over
   (`OccupiedByCore::Takeover`); a headless serve moves to a free port
   instead (`Fallback`).
5. Sets `OPENHUMAN_CORE_RPC_URL` to the bound address.
6. Builds the router and wraps it in a layer that runs every request inside
   `CoreContext::scope` for this runtime.
7. Awaits `start_services()` (startup migrations finish before readiness),
   then sends `EmbeddedReadySignal { port, fallback_from }` on `ready_tx`.
8. Serves until the cancellation token fires (embedded) or the process
   shutdown signal arrives, runs `exit_cleanup()` even if serving failed,
   then returns the serve result.

### The router

`http/mod.rs::build_core_http_router(socketio_enabled)` registers the
`http_host` controllers and assembles the routes. Request middleware runs
outermost to innermost:

```text
 CoreContext::scope (added by serve)
   socket.io layer          only when socketio_enabled
     cors_middleware        preflight + CORS headers, origin allowlist
       rpc_auth_middleware  bearer on protected paths
         request log        non-/rpc requests with timing
           route handler
```

| Route | Handler | Auth |
| --- | --- | --- |
| `GET /` | info page with endpoint list | public |
| `GET /health` | granular liveness; 503 only for a critical component | public |
| `GET /schema` | controller schema discovery | public |
| `POST /rpc` | `rpc_handler`, 64 MiB body limit | bearer |
| `GET /events` | SSE filtered by `client_id` | bearer header, or a one-shot bind token from `core.events_subscribe_token` |
| `GET /events/webhooks` | webhook debug SSE | bearer header or `?token=` |
| `GET /events/domain` | every `DomainEvent` as SSE, for the live event log | bearer |
| `GET /ws/dictation` | dictation WebSocket | bearer header or `?token=`, plus origin check |
| `GET /ws/live-voice` | live voice agent WebSocket | bearer header or `?token=`, plus origin check |
| `GET /oauth/mcp/callback` | MCP browser OAuth redirect target | public; the one-time `state` is the guard |
| `GET /dev/connect` | debug handoff to a loopback Vite renderer | public; guarded in the handler |
| `/v1/*` | the core's OpenAI-compatible router (`inference::http`) | core bearer or the user-managed external API key |

The bearer is the per-launch RPC token the core owns
(`openhuman::core::auth`); [`auth.rs`](auth.rs) is only the route policy over it.

### SaaS mode

`run_server_saas` (`--mode saas --saas-config <file>`) boots the operator's
config instead of a builder. Before boot it installs the session store for
the operator's storage URL (`OPENHUMAN_STORAGE_URL`, else `storage_url`)
through `session_store::install_for_saas`: one default database per profile
workspace with no URL, else `DriverSessionStores` over that backend with
`recover_on_open(false)`,
because a profile's turns are recovered when its lease is taken over, not
when its stores open. On shutdown it releases the leases of idle profiles.

In place of the single-context layer, [`saas_gateway.rs`](saas_gateway.rs)
picks each request's context (the decision is
`openhuman::profiles::gateway`):

- routes a SaaS core never serves (`/v1`, `/events/*`, `/ws/*`,
  `/socket.io`, `/dev/connect`, `/oauth/*`) answer `404`, and so does
  `/events` without a user;
- no `X-OpenHuman-User`: the operator plane;
- with one: the bearer first (`401`), then a duplicate or unreadable header
  (`400`), the signature (`401`), and the user's profile, which is opened
  and leased: `403` when it is not provisioned, `503` when every slot is
  busy or storage fails.

A profile another core hosts answers **`409 Conflict`**:

```text
X-OpenHuman-Profile-Owner: <owner node id>

{"error": "profile_held", "owner": "<node id>",
 "endpoint": "<owner's advertise_url, or null>", "retry_after_ms": <ms>}
```

`endpoint` is where the owner can be reached (null for a node that
advertises none) and `retry_after_ms` how long its lease lasts unless it is
renewed. A gateway routes the user to the owner, or retries after that
long; a dead owner's lease lapses within `lease_ttl_secs`. The refusal comes
after the bearer check, so an unauthenticated caller learns nothing.

### A failed call

`rpc_handler` decodes the controller's `StructuredRpcError` (if any), then
`classify.rs::classify_failure` picks a `FailureDisposition`:
`ExpectedUserState`, `WalletNotConfigured`, `ParamValidation`,
`SessionExpired`, `UsageProbeBackoff`, `TransientDownstream`,
`UnknownMethod`, or `Unexpected`. Only `Unexpected` is an error-level Sentry
event; the rest are logged at lower levels, with caller-supplied text
redacted or sanitized where it could carry params or tokens. The JSON-RPC
error the caller sees is the same for every class, except that
`UsageProbeBackoff` replaces the internal marker with a client message.

### Socket.IO

`socketio.rs::attach_socketio` returns the layer and the `SocketIo` handle;
`spawn_web_channel_bridge` starts the forwarders. A client authenticates by
passing the bearer in the handshake `auth` map (`io(url, { auth: { token }
})`), since browsers cannot set headers on the upgrade. A connection from a
disallowed origin or with a missing or wrong bearer is disconnected at
connect; the event handlers also check the `AuthedConnection` marker before
dispatching anything.

Client to server events: `rpc:request` (answered with `rpc:response` or
`rpc:error`), `chat:start`, `chat:cancel` and `thread:subscribe`. Each
connection joins its own room and the `system` room and gets a `ready`
event.

Server to client, `spawn_web_channel_bridge` spawns one forwarding task per
source: web-chat events (`web_chat::subscribe_web_channel_events`, sent to
the initiating client's room and the `thread:<id>` room, not broadcast),
dictation hotkeys and transcription results (`voice::dictation_listener`),
overlay attention bubbles (`desktop::overlay::subscribe_attention_events`),
core notifications (`desktop::notifications`), and companion state. It also
forwards a set of `DomainEvent`s read off `BUS`: session expiry, MCP setup
secret requests, memory sync and tree-build progress, channel listener
health, and active-workspace changes. Everything except web chat is
broadcast to every client, most under both a colon- and an
underscore-separated name (`workspace:changed` and `workspace_changed`).

`COMPANION_STATE_BUS` and `publish_companion_state_changed` are a
transport-only seam for the shell's companion: the native macOS notch
WKWebView has no Tauri IPC bridge and connects to this Socket.IO endpoint
directly.

## Layout

| Path | What it does |
| --- | --- |
| [`mod.rs`](mod.rs) | Module wiring and the public re-exports. |
| [`serve.rs`](serve.rs) | `serve` and `EmbeddedReadySignal`. |
| [`shims.rs`](shims.rs) | `run_server`, `run_server_headless`, `run_server_saas`, `build_and_serve` (shared with `host`), plus host and port defaults. |
| [`cli.rs`](cli.rs) | `launch`, the server launcher `host::cli` and `host::desktop` install, and `flagged_builder`. |
| [`auth.rs`](auth.rs) | `rpc_auth_middleware`: public paths, query-token paths, and the `/v1` external-key check. |
| [`classify.rs`](classify.rs) | `classify_failure` and `FailureDisposition`. Pure. |
| [`socketio.rs`](socketio.rs) | Socket.IO handshake auth, client event handlers, the event bridge, the companion seam, and the event payload types. |
| [`dev_connect.rs`](dev_connect.rs) | `GET /dev/connect`. |
| [`testing.rs`](testing.rs) | Test-only environment lock shared by this crate's tests. |
| [`http/mod.rs`](http/mod.rs) | `build_core_http_router`, the body limit, root and 404 handlers, request log middleware. |
| [`http/rpc_handler.rs`](http/rpc_handler.rs) | `POST /rpc`. |
| [`http/cors.rs`](http/cors.rs) | CORS headers and preflight over the shared origin rule. |
| [`http/health.rs`](http/health.rs) | `GET /health` and `GET /schema`. |
| [`http/events.rs`](http/events.rs) | The three SSE routes. |
| [`http/dictation.rs`](http/dictation.rs), [`http/live_voice.rs`](http/live_voice.rs) | WebSocket upgrade guards; the sessions themselves live in the core's `voice` domain. |
| [`http/oauth_mcp.rs`](http/oauth_mcp.rs) | MCP OAuth callback, handing off to `mcp::registry::oauth::complete`. |
| [`http/pages.rs`](http/pages.rs) | Static HTML pages for OAuth callbacks. |

## Key entry points

- `serve(&CoreRuntime, Option<oneshot::Sender<EmbeddedReadySignal>>,
  Option<CancellationToken>)` (`serve.rs`).
- `host::desktop(DesktopOptions { rpc_token, .. }, ..)` (`host.rs`): pass the
  bearer the shell already holds so the server never reads it from the
  environment.
- `build_core_http_router(socketio_enabled)` and `rpc_handler`
  ([`http/`](http/)): for tests that want the router without a listener.
- `publish_companion_state_changed(payload)` ([`socketio.rs`](socketio.rs)).

## Boundaries

- Controllers, params, errors and session-expiry semantics are the core's.
  Never add method-name branches here; register controllers in
  `openhuman-core/src/core/all.rs` (or as an extension, as `http_host` does).
- The `/v1` inference router, the dictation and live-voice sessions, and the
  MCP OAuth completion are domain code in the core, mounted here behind the
  core's `http-server` gate.
- The token is minted and verified by `openhuman::core::auth`; listener
  port selection is `openhuman::platform::connectivity::rpc`.
- The event payload types (`WebChannelEvent` and friends) are constructed by
  core domains; this module only transports them.

## Gotchas

- `/dev/connect` only exists in debug builds, or in release with
  `OPENHUMAN_DEV_CONNECT=1`. It accepts only a bare `http://` loopback `app`
  origin, requires a loopback `Host`, refuses cross-site navigations, and
  puts the bearer in the URL fragment with `no-store` and
  `Referrer-Policy: no-referrer`.
- `/events` is exempt from the middleware's header check but not
  unauthenticated: the handler requires a bearer or a bind token, which is
  consumed on use so a leaked URL cannot be replayed.
- The Socket.IO origin check is wider than the CORS one (any scheme on a
  loopback or `tauri.localhost` host). Both read
  `OPENHUMAN_CORE_ALLOWED_ORIGINS`.
- `OPENHUMAN_E2E` switches tool groups to `ToolGroups::advertised()` in the
  embed `desktop` / `cli` presets the `run_server*` shims start from, so the browser E2E mock model can call tools. Library
  hosts are unaffected.

## Tests

Sibling `*_tests.rs` files cover auth route policy, failure classification,
the shims, Socket.IO, `/dev/connect`, and each route module under [`http/`](http/)
(including the `/rpc` handler's Sentry routing and the `/v1` mount in
`inference_route_tests.rs`).

```bash
cargo test -p openhuman-rpc --features crash-reporting server::
```

## Further reading

- [`docs/gitbooks/en/developing/architecture.md`](../../../../docs/gitbooks/en/developing/architecture.md): architecture overview.
- [`docs/gitbooks/en/developing/architecture/security.md`](../../../../docs/gitbooks/en/developing/architecture/security.md): security.
- [`crates/openhuman-rpc/README.md`](../../README.md): the openhuman-rpc crate README.
