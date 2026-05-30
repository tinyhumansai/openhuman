# health

In-process health registry for the OpenHuman core. Tracks per-component liveness (status, last-ok / last-error timestamps, restart counts) plus process metadata (PID, uptime), exposes a snapshot over JSON-RPC/CLI, and keeps itself current by subscribing to `system`/`channel` domain events on the global event bus. State is purely in-memory (a process-global `OnceLock` registry) — nothing is persisted to disk. It also serves static system info (version/OS/arch/PID).

## Responsibilities

- Maintain a process-global registry of named components, each with `status`, `updated_at`, `last_ok`, `last_error`, `restart_count`.
- Provide mutators: `mark_component_ok`, `mark_component_error`, `bump_component_restart`.
- Produce a point-in-time `HealthSnapshot` (PID, uptime seconds, components map) and its JSON form.
- Drive component state automatically from `DomainEvent`s via an event-bus subscriber.
- Expose `health.snapshot` and `health.system_info` RPC/CLI controllers.

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/health/mod.rs` | Export-only: declares modules, re-exports `core::*`, `ops::*`, `pub use ops as rpc`, and the controller-schema pair (`all_health_controller_schemas` / `all_health_registered_controllers`). |
| `src/openhuman/health/core.rs` | The registry itself: `ComponentHealth`/`HealthSnapshot` types, the `OnceLock<HealthRegistry>` singleton (`started_at` + `Mutex<BTreeMap>`), and the mutators/`snapshot`/`snapshot_json` functions. |
| `src/openhuman/health/ops.rs` | RPC handler logic returning `RpcOutcome<T>`: `health_snapshot()` and `system_info()` (+ `SystemInfo` type). |
| `src/openhuman/health/schemas.rs` | Controller schemas + `handle_snapshot`/`handle_system_info` async handlers that delegate to `ops` and serialize via `into_cli_compatible_json`. |
| `src/openhuman/health/bus.rs` | `HealthSubscriber` (`EventHandler`) and `register_health_subscriber()`; maps domain events to registry mutations. |

## Public surface

From `core.rs` (re-exported via `pub use core::*`):
- Types: `ComponentHealth`, `HealthSnapshot`.
- Functions: `mark_component_ok(component)`, `mark_component_error(component, error)`, `bump_component_restart(component)`, `snapshot() -> HealthSnapshot`, `snapshot_json() -> serde_json::Value`.

From `ops.rs` (re-exported via `pub use ops::*`, also aliased `pub use ops as rpc`):
- `health_snapshot() -> RpcOutcome<serde_json::Value>`, `system_info() -> RpcOutcome<SystemInfo>`, and the `SystemInfo` struct.

From `bus.rs`: `HealthSubscriber`, `register_health_subscriber()`.

From `schemas.rs` (re-exported under aliased names): `all_health_controller_schemas`, `all_health_registered_controllers`.

## RPC / controllers

Two controllers in the `health` namespace:

| Method | Inputs | Outputs |
| --- | --- | --- |
| `openhuman.health_snapshot` | none | `snapshot` (JSON): full serialized `HealthSnapshot`. |
| `openhuman.health_system_info` | none | `version`, `os`, `arch`, `pid`. |

`system_info`'s `version` is `CARGO_PKG_VERSION`; `os`/`arch` come from `std::env::consts`. Legacy callers may send `openhuman.system_info`, which the alias table rewrites to `health_system_info` before dispatch.

## Events

Subscriber `HealthSubscriber` (`name = "health::registry"`) registered via `register_health_subscriber()` on the global event bus, filtered to the `system` and `channel` domains. It reacts to:

| DomainEvent | Action |
| --- | --- |
| `SystemStartup { component }` | `mark_component_ok(component)` |
| `HealthChanged { component, healthy, message }` | OK if `healthy`, else `mark_component_error` with `message` (default `"unknown health error"`) |
| `HealthRestarted { component }` | `bump_component_restart(component)` |
| `ChannelConnected { channel }` | `mark_component_ok("channel:<channel>")` |
| `ChannelDisconnected { channel, reason }` | `mark_component_error("channel:<channel>", reason)` |

It only subscribes; it does not publish events.

## Persistence

None on disk. State lives in a process-global `OnceLock<HealthRegistry>` (lazy-initialized) holding a `Mutex<BTreeMap<String, ComponentHealth>>` and an `Instant` start time. Cleared on process exit; uptime resets each launch.

## Dependencies

- `crate::core::event_bus` (`DomainEvent`, `EventHandler`, `SubscriptionHandle`, `subscribe_global`) — to receive system/channel events.
- `crate::core::all` (`ControllerFuture`, `RegisteredController`) and `crate::core::{ControllerSchema, FieldSchema, TypeSchema}` — controller registry wiring.
- `crate::rpc::RpcOutcome` — RPC handler return contract.
- External crates: `chrono` (RFC3339 timestamps), `parking_lot::Mutex`, `serde`/`serde_json`, `async_trait`.

## Used by

- `src/core/all.rs` — registers `all_health_*` controllers into the registry.
- `src/core/jsonrpc.rs` — references health (snapshot/system_info surface).
- `src/openhuman/channels/runtime/{startup,supervision}.rs` and `src/openhuman/channels/tests/health.rs` — channel runtime updates component health.
- `src/openhuman/cron/scheduler.rs`, `src/openhuman/update/scheduler.rs` — emit/consume health signals.

## Notes / gotchas

- The registry is a global singleton; tests use UUID-suffixed component names to avoid cross-test contention rather than resetting shared state.
- `upsert_component` creates entries lazily with initial status `"starting"` and always refreshes `updated_at` after the update closure.
- `mark_component_ok` clears `last_error`; `mark_component_error` leaves `last_ok` intact (so the last-known-good time survives a failure).
- `restart_count` uses `saturating_add` (won't overflow).
- The `system_info` schema declares `pid` as a `String` (`TypeSchema::String`) even though the `SystemInfo` struct serializes `pid` as a numeric `u32` — schema type vs. wire type differ here.
- `bus.rs` short-circuits double registration via a `OnceLock` and warns (does not panic) if the bus isn't initialized.
