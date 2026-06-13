# http_host

Static directory hosting over ad-hoc, in-process HTTP listeners owned by the core. Lets trusted callers (RPC/CLI) start, inspect, list, and stop lightweight file servers that expose a chosen directory on a chosen TCP port. Each server runs as an in-process `axum` task sharing the core's lifetime, and defaults to HTTP Basic authentication using the active user's identity plus a randomly generated password. There is no on-disk persistence — the registry of running servers lives in process memory and is torn down on shutdown.

## Responsibilities

- Start an `axum` static file server bound to a requested `bind_host:port`, serving a canonicalized directory tree.
- Default-on HTTP Basic auth: derive a username from the active session (falling back to env), generate a random password per server.
- Serve files (streamed) and directory listings (auto `index.html`, otherwise a generated HTML listing), with MIME type inference by extension.
- Enforce path-traversal safety: reject `..`, absolute, URL-encoded escape, and out-of-root resolved paths.
- Track running servers in an in-process registry keyed by a UUID `server_id`; prevent duplicate `bind_host:port` registrations and prune finished tasks.
- Gracefully stop individual servers (cancel + join) and register a one-time core shutdown hook that stops all servers on core exit.
- Expose start/stop/get/list as JSON-RPC / CLI controllers under the `http_host` namespace.

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/http_host/mod.rs` | Module docstring + declarations; re-exports controller schema/registry pair; defines `LOG_PREFIX = "[http_host]"`. |
| `src/openhuman/http_host/types.rs` | Serde types: `StartHostedDirParams`, `HostedDirLookupParams`, `HostedDirServerInfo`, `HostedDirAuth`, and the `*Result` response shapes. |
| `src/openhuman/http_host/ops.rs` | In-process server manager: `HostedDirRegistry` (Mutex<HashMap>) + `OnceLock` singleton, `start/list/get/stop/stop_all` ops, shutdown-hook registration, finished-task pruning, collision checks. |
| `src/openhuman/http_host/handlers.rs` | `axum` router + request handlers (`HostedDirState`, `build_router`, root/path/file/directory serving, streamed file responses, generated directory listing HTML). |
| `src/openhuman/http_host/auth.rs` | Basic-auth verification (`ensure_authorized`), default username resolution from session/env, username sanitization, random password generation. |
| `src/openhuman/http_host/path_utils.rs` | Path safety + URL/HTML helpers: directory canonicalization, request-path traversal resolution, bind-host/label sanitization, href builders, `escape_html`, `content_type_for_path`, `redact_path_for_log`. |
| `src/openhuman/http_host/rpc.rs` | RPC adapters wrapping ops into `RpcOutcome<T>` (`start`/`stop`/`get`/`list`). |
| `src/openhuman/http_host/schemas.rs` | `ControllerSchema`s + `handle_*` controller handlers; `all_controller_schemas` / `all_registered_controllers`. |
| `src/openhuman/http_host/tests.rs` | `#[cfg(test)]` test module. |

## Public surface

- `all_http_host_controller_schemas()` / `all_http_host_registered_controllers()` — re-exported from `schemas`; wired into the core controller registry.
- `pub mod ops` — `start_hosted_dir_server`, `list_hosted_dir_servers`, `get_hosted_dir_server`, `stop_hosted_dir_server`, `stop_all_hosted_dir_servers`.
- `pub mod rpc` — async `start`/`stop`/`get`/`list` returning `RpcOutcome<...>`.

(`auth`, `handlers`, `path_utils`, `types` are private to the module.)

## RPC / controllers

Namespace `http_host` (invoked as `openhuman.http_host_<function>`):

| Method | Inputs | Outputs |
| --- | --- | --- |
| `http_host.start` | `directory` (req), `port` (req; `0` = OS-chosen), `bind_host` (default `127.0.0.1`), `server_name`, `disable_auth` (default false), `username` | `server` (JSON: `HostedDirServerInfo` incl. URLs + generated auth credentials) |
| `http_host.stop` | `server_id` (req) | `stopped` (bool), `server` (final snapshot) |
| `http_host.get` | `server_id` (req) | `server` (incl. current auth credentials) |
| `http_host.list` | none | `servers` (array of `HostedDirServerInfo`) |

## Persistence

None on disk. Running servers are held in a process-global `HostedDirRegistry` (`OnceLock<HostedDirRegistry>` wrapping `Mutex<HashMap<server_id, HostedDirRuntime>>`). State is volatile and cleared on shutdown.

## Dependencies

- `crate::openhuman::config` — `load_config_with_timeout` to resolve the active config when deriving the default Basic-auth username (`auth.rs`).
- `crate::openhuman::credentials::session_support` — `build_session_state` to read the active user identity for the default auth username (`auth.rs`).
- `crate::core::shutdown` — `register` a one-time hook so all hosted servers stop when the core shuts down (`ops.rs`).
- `crate::core::all` — `ControllerFuture`, `RegisteredController` for controller registration (`schemas.rs`).
- `crate::core::{ControllerSchema, FieldSchema, TypeSchema}` — controller schema types (`schemas.rs`).
- `crate::rpc::RpcOutcome` — RPC response envelope (`rpc.rs`).
- External crates: `axum` (HTTP server/router), `tokio` (`TcpListener`, tasks), `tokio_util` (`CancellationToken`, `ReaderStream`), `uuid`, `base64`, `rand`, `urlencoding`, `serde`/`serde_json`.

## Used by

- `src/core/all.rs` — registers `all_http_host_registered_controllers()` (line ~145) and `all_http_host_controller_schemas()` (line ~309) into the core controller registry, exposing the RPC/CLI surface.
- `src/openhuman/mod.rs` — declares `pub mod http_host`.
- `src/core/observability.rs` references `http_host::path_utils` paths in error-classification docs/tests (`http_host` directory-not-found maps to a filesystem user-path-invalid class).

## Notes / gotchas

- **Credentials in responses**: `HostedDirServerInfo.auth` carries the generated password in `start`/`get`/`list` responses. Treat RPC output as sensitive.
- **Auth defaults**: when auth is enabled and no username resolves from session/env, the username falls back to `"openhuman"`. Passwords are 18 random bytes, URL-safe base64 (no padding).
- **Path safety**: `resolve_request_path` rejects URL-encoded traversal and verifies the canonicalized target stays under the hosted root; `canonicalize_hosted_directory` resolves and verifies the root is a real directory before binding.
- **Port `0`**: binding with port `0` lets the OS pick a free port; the actual assigned port (from `local_addr`) is what gets stored and reported.
- **No duplicate binds**: `start` rejects another server already registered on the same `bind_host:port`.
- **Logging redaction**: directory paths are logged via `redact_path_for_log` (only the leaf name, prefixed `<redacted>/`) — full host paths are not emitted.
- **Lifetime**: servers do not persist across core restarts; the shutdown hook (`register_shutdown_hook_once`) is installed lazily on the first `start`.
- **IPv6**: `bind_host` containing `:` (and not already bracketed) is wrapped in `[...]` for both the bind target and the URL rendering.
