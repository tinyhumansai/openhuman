# connectivity

Diagnostics for the local core's reachability and the live backend Socket.IO state, plus the listen-port selection logic the core uses when it boots its embedded HTTP listener. The frontend has four independent connectivity channels: browser internet, the local core HTTP, the renderer's Socket.IO link to that core, and (via this module's diag snapshot, #6256) the core's own Socket.IO link to the hosted backend: and issue #1527 split them in the UI so users see *which* channel is broken instead of one conflated "Disconnected" pill. This module exposes a cheap `openhuman.connectivity_diag` RPC that snapshots in-memory backend-socket state plus the local core's PID and listening port (no I/O beyond a single TCP probe), suitable for poll-based health checks.

## Responsibilities

- Answer `openhuman.connectivity_diag` with a flat snapshot: backend socket state, last websocket error, whether the backend reconnect loop is running and whether it stopped on a terminal failure (`socket_loop_active` / `socket_loop_stopped_on_failure`, #6270), sidecar PID, configured listen port, and whether that port currently has a listener bound.
- Resolve the configured core RPC port from the environment (`OPENHUMAN_CORE_RPC_URL` then `OPENHUMAN_CORE_PORT`, defaulting to `7788`).
- Snapshot the backend Socket.IO connection state from the host-installed `SocketPort` (reports `"uninitialized"` before registration).
- Probe whether a TCP port on loopback is already bound (`is_port_in_use`).
- Pick a listen port for the embedded core HTTP listener (`pick_listen_port` / `pick_listen_port_for_host`): try preferred, retry transient `AddrInUse` races, request stale-listener takeover when another OpenHuman core owns the port (#1130): the embedded desktop path; headless `serve` passes `OccupiedByCore::Fallback` to `pick_listen_port_for_host_with` and moves on instead: otherwise fall back to a port pool.
- Handle Windows OS-excluded port ranges (`WSAEACCES` / os error 10013, Sentry OPENHUMAN-TAURI-500) by routing straight to fallback ports instead of failing.

## Key files

| File | Role |
| --- | --- |
| `crates/openhuman-core/src/platform/connectivity/mod.rs` | Module docstring + exports; re-exports schema entry points as `all_connectivity_controller_schemas` / `all_connectivity_registered_controllers`. |
| `crates/openhuman-core/src/platform/connectivity/ops.rs` | Pure helper `is_port_in_use(port)`: binds an ephemeral loopback listener to detect an occupied port; unit-tested in isolation. |
| `crates/openhuman-core/src/platform/connectivity/rpc.rs` | Core logic: `ConnectivityDiagResponse`, `snapshot()`, `diag()` RPC, env port resolution, socket-state snapshot, and the `pick_listen_port*` family with fingerprinting (`identify_listener`) and OS-exclusion handling. |
| `crates/openhuman-core/src/platform/connectivity/schemas.rs` | Controller schema for the single `connectivity_diag` controller + `handle_diag` delegating to `rpc::diag()`. |

## Public surface

- `connectivity::all_connectivity_controller_schemas()` / `all_connectivity_registered_controllers()`: registry entry points (re-exported from [`schemas.rs`](./schemas.rs)).
- `connectivity::ops::is_port_in_use(port: u16) -> bool`.
- `connectivity::rpc::ConnectivityDiagResponse`: serialized diag payload (`socket_state`, `last_ws_error`, `socket_loop_active`, `socket_loop_stopped_on_failure`, `sidecar_pid`, `listen_port`, `listen_port_in_use`). The two booleans are the core's own statement of whether its backend reconnect loop is running and, if not, whether it stopped on a terminal failure (no usable session token); the frontend's connectivity chip keys "no link wanted" vs "down, retrying" vs "stopped, sign in again" off them rather than inferring liveness from `socket_state` (#6256, #6270).
- `connectivity::rpc::snapshot() -> ConnectivityDiagResponse` and `connectivity::rpc::diag() -> Result<Outcome<Value>, String>`.
- `connectivity::rpc::pick_listen_port(preferred)` / `pick_listen_port_for_host(host, preferred)` → `Result<PickListenPortResult, PickListenPortError>`.
- `connectivity::rpc::PickListenPortResult` (`listener`, `port`, `fallback_from`) and `PickListenPortError` (`WouldTakeOver` / `NoAvailablePort` / `BindFailed`).

## RPC / controllers

- **`openhuman.connectivity_diag`** (namespace `connectivity`, function `diag`): read-only, no inputs. Returns a single output field `diag` (JSON) containing the `ConnectivityDiagResponse` snapshot. Described as cheap and safe to poll. Registered via `all_registered_controllers` → `handle_diag` → `rpc::diag()`.

Restart/mutate operations are intentionally **not** here: they live in the Tauri shell (`restart_core_process` in `crates/openhuman-app/src/lib.rs`) because they touch the host process tree and can't be answered from inside the core itself.

## Persistence

None: the module holds no state. The diag snapshot reads only the environment, the in-memory socket port, and a live TCP probe.

## Dependencies

- `crate::platform::socket::global_socket_manager`: read the host-owned backend Socket.IO `ConnectionStatus` and last error for the diag snapshot.
- `crate::core::all::{ControllerFuture, RegisteredController}`: controller registration types.
- `crate::core::{ControllerSchema, FieldSchema, TypeSchema}`: controller schema shape.
- `crate::core::Outcome`: RPC return envelope (`Outcome::single_log`).
- External crates: `reqwest` (HTTP fingerprint probe of a listener's `GET /` root), `tokio` (async `TcpListener`, retry backoff), `serde`/`serde_json`, `url`, `tracing`/`log`.

## Used by

- `crates/openhuman-core/src/core/all.rs`: registers the controller (`all_connectivity_registered_controllers`) and schema (`all_connectivity_controller_schemas`), and routes the `"connectivity"` namespace.
- `crates/openhuman-core/src/core/runtime/builder.rs`: calls `connectivity::rpc::pick_listen_port_for_host(...)` during core bind to select the embedded HTTP listener port; afterward syncs `OPENHUMAN_CORE_RPC_URL` to the actual bound port so `resolve_listen_port()` (and thus `connectivity_diag`) reports the live listener after a fallback.
- `crates/openhuman-core/src/platform/mod.rs`: declares the module.

## Notes / gotchas

- The `pick_listen_port` flow is the real workhorse despite the module's "diag" framing; it owns the core's startup port selection and the stale-listener takeover decision (#1130).
- Port resolution priority is `OPENHUMAN_CORE_RPC_URL` (port component) > `OPENHUMAN_CORE_PORT` > default `7788`. Invalid `OPENHUMAN_CORE_PORT` logs a warning and falls back to the default rather than failing.
- Fallback pool: when preferred is the default `7788`, fallbacks are `7789..=7798`; otherwise `preferred+1..=preferred+10` (saturating-checked).
- `WouldTakeOver` is only returned when something is actually listening (`AddrInUse`) **and** the listener fingerprints as an OpenHuman core (its `GET /` returns JSON with `"name":"openhuman"`). OS-excluded ports (Windows `WSAEACCES` / os error 10013) skip the takeover probe and route directly to fallbacks; `is_port_excluded_bind_error` matches on the raw OS code (10013) because Rust's `ErrorKind` mapping for it isn't stable across releases.
- IPv6 probe hosts are bracketed per RFC 3986 before building the fingerprint URL so live cores on IPv6 aren't misclassified as `Other`.
- `socket_state` is funneled through `serde_json` (not `Debug`) so the lowercased wire shape stays stable; `"uninitialized"` is reported before the host installs a socket port (early startup / tests).
- `is_port_in_use` returns `false` on non-`AddrInUse` bind errors (e.g. permission denied) so a port isn't misreported as occupied.
- No agent tools and no event-bus subscribers in this module.

## Further reading

- [Parent module (`platform`)](../README.md)
- [Platform and availability](../../../../../docs/gitbooks/en/features/platform.md)
- [Tauri shell architecture](../../../../../docs/gitbooks/en/developing/architecture/tauri-shell.md)
- [Architecture overview](../../../../../docs/gitbooks/en/developing/architecture.md)
