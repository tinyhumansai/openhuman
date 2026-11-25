# http_host

Static directory hosting over ad-hoc, in-process HTTP listeners. Trusted
callers (JSON-RPC or the CLI) can start a small file server that exposes one
directory on a chosen port, inspect or list the running servers, and stop
them. Each server is an `axum` task inside the core process, protected by
HTTP Basic auth by default. It lives in `openhuman-rpc` (feature `server`)
rather than the core because it is HTTP transport; its controllers join the
core's registry as an extension.

## How it works

### Registration

The core does not know about this module. `register_controllers()` hands
the four controllers to the core's registry with
`register_controller_extension`, under `DomainGroup::Platform` (it has no
domain family of its own, so the runtime's `DomainSet` gates it with the
platform surface). The server calls the panicking wrapper
`ensure_registered()` from `build_core_http_router`, and `crate::host` adds the same controllers
(`http_host::extension()`) to its builders, so every host that runs the RPC server (the desktop
app and the `openhuman-core` binary) exposes `http_host.*`. Registration is
idempotent. A host that embeds the core without this crate's server, such as
the TUI or an `openhuman-embed` library host, has no `http_host` surface.

### Starting a server

```text
 openhuman.http_host_start { directory, port, bind_host?, ... }
   |
   schemas::handle_start -> rpc::start -> ops::start_hosted_dir_server
   |
   |- register the core shutdown hook (once per process)
   |- canonicalize_hosted_directory     must exist and be a directory
   |- sanitize_bind_host, sanitize_optional_label(server_name)
   |- auth: off if disable_auth, else
   |     username = caller's, else the active user's, else $USER/$USERNAME,
   |                sanitized, else "openhuman"
   |     password = 18 random bytes, URL-safe base64, no padding
   |- TcpListener::bind(bind_host:port)     port 0 = OS picks
   |- tokio::spawn(axum::serve(listener, build_router(state))
   |               .with_graceful_shutdown(cancel token))
   |- registry: prune finished tasks, refuse a second server on the same
   |            bind_host:port, insert under a new UUID server_id
   v
 { server: HostedDirServerInfo { server_id, base_url, local_url, auth, ... } }
```

The reported port is the one the listener actually bound, so a request for
port `0` comes back with the real number. `base_url` uses the bind host
(IPv6 hosts are bracketed); `local_url` always uses `127.0.0.1`.

### Serving a request

`handlers::build_router` answers `GET` and `HEAD` on `/` and `/{*path}`.
Each request first passes `auth::ensure_authorized` (a `401` with a
`WWW-Authenticate` challenge on failure), then
`path_utils::resolve_request_path`, which rejects `..`, absolute paths and
URL-encoded escapes and checks that the canonicalized target stays under the
hosted root (`400` on failure). A file is streamed with a content type
inferred from its extension. A directory serves its `index.html` if there is
one, and otherwise a generated HTML listing with escaped names.

### Stopping

`http_host.stop` removes the server from the registry, cancels its token and
joins the task. The shutdown hook registered on first start calls
`stop_all_hosted_dir_servers` when the core exits. Nothing is persisted, so
servers never survive a restart.

## Layout

| File | What it does |
| --- | --- |
| [`mod.rs`](mod.rs) | Module wiring, `register_controllers`, `ensure_registered`, the `http_host` namespace description, `LOG_PREFIX = "[http_host]"`. |
| [`types.rs`](types.rs) | Serde types: `StartHostedDirParams`, `HostedDirLookupParams`, `HostedDirServerInfo`, `HostedDirAuth`, and the `*Result` response shapes. |
| [`ops.rs`](ops.rs) | The in-process server manager: the `HostedDirRegistry` singleton (a `Mutex<HashMap>` behind a `OnceLock`), `start`/`list`/`get`/`stop`/`stop_all`, finished-task pruning, collision checks, the shutdown hook. |
| [`handlers.rs`](handlers.rs) | The per-server `axum` router: auth check, path resolution, streamed files, directory listings. |
| [`auth.rs`](auth.rs) | Basic-auth verification, default username resolution from the session or environment, username sanitizing, password generation. |
| [`path_utils.rs`](path_utils.rs) | Directory canonicalization, request-path traversal checks, bind-host and label sanitizing, link builders, `escape_html`, `content_type_for_path`, `redact_path_for_log`. |
| [`rpc.rs`](rpc.rs) | Thin adapters from ops to `Outcome<T>`. |
| [`schemas.rs`](schemas.rs) | `ControllerSchema`s and `handle_*` handlers; `all_controller_schemas` and `all_registered_controllers`. |

## Public surface

- `register_controllers()`: registers the controller extension.
- `all_http_host_controller_schemas()` and
  `all_http_host_registered_controllers()`: re-exported from `schemas`.
- `ops`: `start_hosted_dir_server`, `list_hosted_dir_servers`,
  `get_hosted_dir_server`, `stop_hosted_dir_server`,
  `stop_all_hosted_dir_servers`.
- `rpc`: async `start`, `stop`, `get`, `list` returning `Outcome<..>`.

`auth`, `handlers`, `path_utils` and `types` are private to the module.

## RPC surface

Namespace `http_host`, invoked as `openhuman.http_host_<function>`:

| Method | Inputs | Output |
| --- | --- | --- |
| `http_host.start` | `directory` (required), `port` (required; `0` lets the OS choose), `bind_host` (default `127.0.0.1`), `server_name`, `disable_auth` (default false), `username` | `server`: `HostedDirServerInfo`, including URLs and the generated credentials |
| `http_host.stop` | `server_id` (required) | `stopped` (bool) and `server`, the final snapshot |
| `http_host.get` | `server_id` (required) | `server`, including current credentials |
| `http_host.list` | none | `servers`: every running `HostedDirServerInfo` |

## Boundaries

- The controller contract (`ControllerSchema`, `Outcome`,
  `register_controller_extension`) is the core's (`openhuman::core`).
- The default username comes from the core's config
  (`config::load_config_with_timeout`) and session state
  (`credentials::session_state::build_session_state`); this
  module never talks to the backend.
- Shutdown sequencing is the core's `core::shutdown::register`.
- The core's error classification (`core/observability.rs`) maps a
  directory-not-found from this module to a filesystem user-path class; that
  mapping lives in the core.

## Gotchas

- Responses carry credentials. `HostedDirServerInfo.auth` includes the
  generated password in `start`, `get` and `list` output, so treat it as
  sensitive and keep it out of logs.
- Directory paths are logged through `redact_path_for_log`, which keeps only
  the leaf name behind a `<redacted>/` prefix.
- The duplicate `bind_host:port` check runs after the listener is bound and
  its task spawned. In practice the bind fails first for a port already in
  use, so the check only guards the registry.
- `disable_auth: true` serves the directory to anyone who can reach the
  port. With a non-loopback `bind_host` that means the network.

## Tests

[`http_host_tests.rs`](http_host_tests.rs) (mounted from [`mod.rs`](mod.rs)) covers a start, list and stop
round trip with Basic auth, path traversal rejection, and username
sanitizing and resolution. [`schemas_tests.rs`](schemas_tests.rs) checks schema and handler
parity, required inputs and the unknown-function fallback.

```bash
cargo test -p openhuman-rpc http_host
```

## Further reading

- [`docs/gitbooks/en/developing/architecture.md`](../../../../docs/gitbooks/en/developing/architecture.md): architecture overview.
- [`crates/openhuman-rpc/README.md`](../../README.md): the openhuman-rpc crate README.
