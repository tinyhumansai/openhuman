# service

Service-management domain for the OpenHuman core daemon. It installs/uninstalls the core binary as a per-user OS service (macOS LaunchAgent, Linux systemd user unit, Windows scheduled task), drives its install/start/stop/status lifecycle, and orchestrates **self-restart** and **graceful shutdown** of the currently running core process via the event bus. It also persists machine-local daemon-host UI preferences (tray visibility). All of this is exposed transport-agnostically through the `service.*` RPC/CLI controller registry.

The OS managers themselves (launchd, systemd, schtasks and the E2E mock) live in the leaf crate [`openhuman-platform`](../../../../openhuman-platform/README.md) (`openhuman_platform::service`), driven by a `ServiceSpec`. [`spec.rs`](./spec.rs) builds that spec from `Config` and owns the product identity: labels, daemon executable lookup, arguments and log directory.

## Responsibilities

- Install / uninstall the core daemon as a native per-user service and report its `ServiceStatus`.
- Start / stop / query the installed service per OS (`launchctl`, `systemctl --user`, `schtasks`).
- Accept restart/shutdown requests, publish them to the event bus, and (via subscribers) respawn or `process::exit(0)` the running core with a 150ms flush grace window.
- Resolve the daemon executable to launch (env override `OPENHUMAN_CORE_BIN`, else sibling `openhuman-core[-*]` next to the current exe; macOS also searches `../Resources`).
- Persist and read daemon-host UI preferences (`show_tray`) next to the main config.
- Provide a deterministic, file-backed **mock** service backend for E2E tests (`OPENHUMAN_SERVICE_MOCK`).

## Key files

| File | Role |
| --- | --- |
| `crates/openhuman-core/src/platform/service/mod.rs` | Export-focused module root; declares submodules and re-exports the public surface + controller-schema pair. |
| `crates/openhuman-core/src/platform/service/spec.rs` | The host seam: `service_spec(&Config)` (labels `com.openhuman.core` + legacy `com.openhuman.daemon`/`com.openhuman.app` on macOS, `openhuman.service` on Linux, `OpenHuman Core` on Windows; `resolve_daemon_executable`; args `["run"]`; `<config dir>/logs`), and `install`/`start`/`stop`/`status`/`uninstall(&Config)` over `openhuman_platform::service`. |
| `crates/openhuman-core/src/platform/service/ops.rs` | RPC handler layer (`service_install`, `service_start`, `service_stop`, `service_status`, `service_restart`, `service_shutdown`, `service_uninstall`, `daemon_host_get`/`set`) returning `Outcome<T>`. Re-exported as `rpc`. |
| `crates/openhuman-core/src/platform/service/schemas.rs` | Controller schemas + `handle_*` adapters for the `service` namespace; `all_controller_schemas` / `all_registered_controllers`. |
| `crates/openhuman-core/src/platform/service/restart.rs` | Self-restart orchestration: `service_restart` (publishes event), `trigger_self_restart_now` (respawns current exe with original args), `apply_startup_restart_delay_from_env`, `RestartStatus`. |
| `crates/openhuman-core/src/platform/service/shutdown.rs` | Graceful-shutdown orchestration: `service_shutdown` (publishes event), `ShutdownStatus`. |
| `crates/openhuman-core/src/platform/service/bus.rs` | Event-bus subscribers `RestartSubscriber` / `ShutdownSubscriber` (filter domain `system`) and idempotent `register_*_subscriber` helpers; one-shot atomic gates so only the first request acts. |
| `crates/openhuman-platform/src/service/` | The managers (in `openhuman-platform`): `ServiceState`/`ServiceStatus`, the dispatchers, `macos.rs` (LaunchAgent via `launchctl`, removes legacy labels), `linux.rs` (systemd user unit via `systemctl --user`), `windows.rs` (scheduled task via `schtasks`), `mock.rs`, and `common.rs` (command runners, Windows `CREATE_NO_WINDOW`, `xml_escape`). |
| `crates/openhuman-core/src/platform/service/daemon_host.rs` | `DaemonHostConfig { show_tray }` + async `load_for_config_dir` / `save_for_config_dir` (JSON next to config, `daemon_host_config.json`). |
| `crates/openhuman-core/src/platform/service/tools.rs` | LLM-callable wrappers over the domain (`service_status` / `daemon_host_prefs_get` default-on; lifecycle mutators default-off via the `service_lifecycle` user-filter toggle). |

## Public surface

From [`mod.rs`](./mod.rs) re-exports:

- `spec::{install, start, stop, status, uninstall}`: the `Config`-bound dispatchers. `ServiceState` / `ServiceStatus` are `openhuman_platform::service`'s.
- `ops::*` (also aliased `rpc`): the async RPC handlers `service_install` … `service_uninstall`, `service_restart`, `service_shutdown`, `daemon_host_get`, `daemon_host_set`.
- `restart::{apply_startup_restart_delay_from_env, RestartStatus}` (and `restart::trigger_self_restart_now` via the `restart` module path).
- `shutdown::ShutdownStatus`.
- `schemas::all_service_controller_schemas` / `all_service_registered_controllers`.
- `daemon_host::{DaemonHostConfig, …}` (via its `pub mod` path).

## RPC / controllers

Namespace `service` (called as `openhuman.service_<fn>` / `service.<fn>`). All nine handlers go through `Outcome` and are wired into the registry by `crate::core::all` via `all_service_registered_controllers`.

| Method | Inputs | Output |
| --- | --- | --- |
| `service.install` |: | `ServiceStatus` |
| `service.start` |: | `ServiceStatus` |
| `service.stop` |: | `ServiceStatus` |
| `service.status` |: | `ServiceStatus` |
| `service.uninstall` |: | `ServiceStatus` |
| `service.restart` | `source?`, `reason?` | restart ack JSON (`RestartStatus`) |
| `service.shutdown` | `source?`, `reason?` | shutdown ack JSON (`ShutdownStatus`) |
| `service.daemon_host_get` |: | `DaemonHostConfig` |
| `service.daemon_host_set` | `show_tray` (required) | `DaemonHostConfig` |

Lifecycle handlers load config via `config::rpc::load_config_with_timeout`; `restart`/`shutdown` are intentionally config-free (they target the running process).

## Events

Publishes (in [`restart.rs`](./restart.rs) / [`shutdown.rs`](./shutdown.rs)) to the global event bus, domain `system`:

- `DomainEvent::SystemRestartRequested { source, reason }`
- `DomainEvent::SystemShutdownRequested { source, reason }`

Subscribes (in [`bus.rs`](./bus.rs)):

- `RestartSubscriber` (`name = "service::restart"`): on `SystemRestartRequested`, atomically claims a one-shot gate, calls `trigger_self_restart_now` to spawn a replacement process, then `process::exit(0)` after 150ms.
- `ShutdownSubscriber` (`name = "service::shutdown"`): on `SystemShutdownRequested`, claims its gate and `process::exit(0)` after 150ms (no respawn).

Both subscribers are registered idempotently from `crates/openhuman-core/src/core/runtime/subscribers.rs` at startup via `register_restart_subscriber` / `register_shutdown_subscriber`; handles are held in process-lifetime `OnceLock`s so they are never dropped.

## Persistence

- Daemon-host prefs: `daemon_host_config.json` next to the main config (`DaemonHostConfig { show_tray }`, default `true`); read/written async by [`daemon_host.rs`](./daemon_host.rs).
- Service unit files: written by the per-OS impls (macOS plist in `~/Library/LaunchAgents`, Linux systemd user unit, Windows scheduled task).
- Mock state: `service-mock-state.json` (overridable via `OPENHUMAN_SERVICE_MOCK_STATE_FILE`) tracking `installed`/`running`/`agent_running`/forced `failures`, only when the mock is enabled.

## Dependencies

- `openhuman_platform::service`: the OS managers and the mock, driven by the `ServiceSpec` from [`spec.rs`](./spec.rs).
- `crate::config`: `Config` (paths, config dir) for every lifecycle/path operation; `config::rpc::load_config_with_timeout` in the schema handlers.
- `crate::core::bus::BUS` (`tinybus::OnceBus<DomainEvent>`): `publish` / `subscribe` for restart/shutdown orchestration; `crate::core::events::DomainEvent` for the event catalog.
- `crate::core` (`ControllerSchema`, `FieldSchema`, `TypeSchema`) and `crate::core::all` (`ControllerFuture`, `RegisteredController`): controller schema/registration contract.
- `crate::core::Outcome`: standard RPC result envelope.
- External: `anyhow`, `serde`/`serde_json`, `tokio`, `async_trait`, plus OS CLIs (`launchctl`, `systemctl`, `schtasks`).

## Used by

- `crates/openhuman-core/src/core/all.rs`: registers the service controllers (`all_service_registered_controllers`).
- `crates/openhuman-core/src/core/runtime/subscribers.rs`: registers the restart/shutdown event-bus subscribers at startup.
- `crates/openhuman-core/src/platform/update/ops.rs` (`rpc::service_restart` after a self-replace), `crates/openhuman-core/src/desktop/app_state/ops/runtime_snapshot.rs` (`status`): call into `crate::platform::service`. `crates/openhuman-core/src/config/ops/loader/runtime_flags.rs` (`mock_agent_running`) and `desktop/app_state/ops/{types,runtime_snapshot}.rs` (`ServiceState`/`ServiceStatus`) use `openhuman_platform::service` directly.
- `crates/openhuman-core/src/tools/mod.rs`: re-exports `platform::service::tools::*`.
- `lib.rs`: module wiring.

## Notes / gotchas

- Restart/shutdown are **two-phase**: the RPC/CLI call only acknowledges and publishes an event; the actual respawn/exit happens in the subscriber, so RPC, CLI, and internal triggers share one path with consistent logging. The subscriber must be registered or requests are no-ops.
- One-shot atomic gates exist in **both** [`bus.rs`](./bus.rs) and [`restart.rs`](./restart.rs) (`RESTART_IN_PROGRESS`): duplicate restart events are ignored; a failed `trigger_self_restart_now` resets the gate to allow a retry.
- `trigger_self_restart_now` respawns the current exe with the **original argv** (preserving launch mode) and sets `OPENHUMAN_RESTART_DELAY_MS` (default 350) on the child; the child honors it at startup via `apply_startup_restart_delay_from_env` to dodge HTTP-port bind races while the old process releases sockets.
- Self-restart fails if launched with no args (`std::env::args().skip(1)` empty).
- Platform support is compile-gated; on unsupported targets the dispatchers `bail!` with "supported on macOS, Linux, and Windows only".
- `OPENHUMAN_SERVICE_MOCK` short-circuits **every** lifecycle dispatcher to the file-backed mock: used for deterministic E2E; it can also inject forced per-operation failures.
- Windows command spawns set `CREATE_NO_WINDOW` (`openhuman_platform`'s `common::no_window`) so polled `schtasks /Query` calls don't flash a console.
- Every lifecycle call builds the spec, so it resolves the daemon executable (env `OPENHUMAN_CORE_BIN`, else a sibling `openhuman-core[-*]`) even for `status`.
- Lifecycle RPCs are deliberately **not** unit-tested (they mutate real OS state or kill the process); RPC-adapter coverage lives in `tests/json_rpc_e2e.rs`. `daemon_host_get`/`set` and the restart/shutdown publish paths are unit-tested.

## Further reading

- [Parent module (`platform`)](../README.md)
- [Platform and availability](../../../../../docs/gitbooks/en/features/platform.md)
- [Tauri shell architecture](../../../../../docs/gitbooks/en/developing/architecture/tauri-shell.md)
- [Architecture overview](../../../../../docs/gitbooks/en/developing/architecture.md)
