# connectivity

Diagnostics for the local core's reachability and the live backend Socket.IO state, plus the listen-port selection logic the core uses when it boots its embedded HTTP listener. The frontend has three independent connectivity channels — browser internet, backend Socket.IO websocket, and the local core HTTP — and issue #1527 split them in the UI so users see *which* channel is broken instead of one conflated "Disconnected" pill. This module exposes a cheap `openhuman.connectivity_diag` RPC that snapshots in-memory backend-socket state plus the local core's PID and listening port (no I/O beyond a single TCP probe), suitable for poll-based health checks.

## Responsibilities

- Answer `openhuman.connectivity_diag` with a flat snapshot: backend socket state, last websocket error, sidecar PID, configured listen port, and whether that port currently has a listener bound.
- Resolve the configured core RPC port from the environment (`OPENHUMAN_CORE_RPC_URL` then `OPENHUMAN_CORE_PORT`, defaulting to `7788`).
- Snapshot the backend Socket.IO connection state from the global `SocketManager` (reports `"uninitialized"` when the manager singleton isn't registered yet).
- Probe whether a TCP port on loopback is already bound (`is_port_in_use`).
- Pick a listen port for the embedded core HTTP listener (`pick_listen_port` / `pick_listen_port_for_host`): try preferred, retry transient `AddrInUse` races, request stale-listener takeover when another OpenHuman core owns the port (#1130), otherwise fall back to a port pool.
- Handle Windows OS-excluded port ranges (`WSAEACCES` / os error 10013, Sentry OPENHUMAN-TAURI-500) by routing straight to fallback ports instead of failing.

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/connectivity/mod.rs` | Module docstring + exports; re-exports schema entry points as `all_connectivity_controller_schemas` / `all_connectivity_registered_controllers`. |
| `src/openhuman/connectivity/ops.rs` | Pure helper `is_port_in_use(port)` — binds an ephemeral loopback listener to detect an occupied port; unit-tested in isolation. |
| `src/openhuman/connectivity/rpc.rs` | Core logic: `ConnectivityDiagResponse`, `snapshot()`, `diag()` RPC, env port resolution, socket-state snapshot, and the `pick_listen_port*` family with fingerprinting (`identify_listener`) and OS-exclusion handling. |
| `src/openhuman/connectivity/schemas.rs` | Controller schema for the single `connectivity_diag` controller + `handle_diag` delegating to `rpc::diag()`. |

## Public surface

- `connectivity::all_connectivity_controller_schemas()` / `all_connectivity_registered_controllers()` — registry entry points (re-exported from `schemas.rs`).
- `connectivity::ops::is_port_in_use(port: u16) -> bool`.
- `connectivity::rpc::ConnectivityDiagResponse` — serialized diag payload (`socket_state`, `last_ws_error`, `sidecar_pid`, `listen_port`, `listen_port_in_use`).
- `connectivity::rpc::snapshot() -> ConnectivityDiagResponse` and `connectivity::rpc::diag() -> Result<RpcOutcome<Value>, String>`.
- `connectivity::rpc::pick_listen_port(preferred)` / `pick_listen_port_for_host(host, preferred)` → `Result<PickListenPortResult, PickListenPortError>`.
- `connectivity::rpc::PickListenPortResult` (`listener`, `port`, `fallback_from`) and `PickListenPortError` (`WouldTakeOver` / `NoAvailablePort` / `BindFailed`).

## RPC / controllers

- **`openhuman.connectivity_diag`** (namespace `connectivity`, function `diag`) — read-only, no inputs. Returns a single output field `diag` (JSON) containing the `ConnectivityDiagResponse` snapshot. Described as cheap and safe to poll. Registered via `all_registered_controllers` → `handle_diag` → `rpc::diag()`.

Restart/mutate operations are intentionally **not** here — they live in the Tauri shell (`restart_core_process` in `app/src-tauri/src/lib.rs`) because they touch the host process tree and can't be answered from inside the core itself.

## Persistence

None — the module holds no state. The diag snapshot reads only the environment, the in-memory `SocketManager`, and a live TCP probe.

## Dependencies

- `crate::openhuman::socket::manager::global_socket_manager` — read the live backend Socket.IO `ConnectionStatus` and last error for the diag snapshot.
- `crate::core::all::{ControllerFuture, RegisteredController}` — controller registration types.
- `crate::core::{ControllerSchema, FieldSchema, TypeSchema}` — controller schema shape.
- `crate::rpc::RpcOutcome` — RPC return envelope (`RpcOutcome::single_log`).
- External crates: `reqwest` (HTTP fingerprint probe of a listener's `GET /` root), `tokio` (async `TcpListener`, retry backoff), `serde`/`serde_json`, `url`, `tracing`/`log`.

## Used by

- `src/core/all.rs` — registers the controller (`all_connectivity_registered_controllers`) and schema (`all_connectivity_controller_schemas`), and routes the `"connectivity"` namespace.
- `src/core/jsonrpc.rs` — calls `connectivity::rpc::pick_listen_port_for_host(...)` during core bind to select the embedded HTTP listener port; afterward syncs `OPENHUMAN_CORE_RPC_URL` to the actual bound port so `resolve_listen_port()` (and thus `connectivity_diag`) reports the live listener after a fallback.
- `src/openhuman/mod.rs` — declares the module.

## Notes / gotchas

- The `pick_listen_port` flow is the real workhorse despite the module's "diag" framing; it owns the core's startup port selection and the stale-listener takeover decision (#1130).
- Port resolution priority is `OPENHUMAN_CORE_RPC_URL` (port component) > `OPENHUMAN_CORE_PORT` > default `7788`. Invalid `OPENHUMAN_CORE_PORT` logs a warning and falls back to the default rather than failing.
- Fallback pool: when preferred is the default `7788`, fallbacks are `7789..=7798`; otherwise `preferred+1..=preferred+10` (saturating-checked).
- `WouldTakeOver` is only returned when something is actually listening (`AddrInUse`) **and** the listener fingerprints as an OpenHuman core (its `GET /` returns JSON with `"name":"openhuman"`). OS-excluded ports (Windows `WSAEACCES` / os error 10013) skip the takeover probe and route directly to fallbacks; `is_port_excluded_bind_error` matches on the raw OS code (10013) because Rust's `ErrorKind` mapping for it isn't stable across releases.
- IPv6 probe hosts are bracketed per RFC 3986 before building the fingerprint URL so live cores on IPv6 aren't misclassified as `Other`.
- `socket_state` is funneled through `serde_json` (not `Debug`) so the lowercased wire shape stays stable; `"uninitialized"` is reported when the `SocketManager` singleton isn't installed (early startup / tests).
- `is_port_in_use` returns `false` on non-`AddrInUse` bind errors (e.g. permission denied) so a port isn't misreported as occupied.
- No agent tools and no event-bus subscribers in this module.
