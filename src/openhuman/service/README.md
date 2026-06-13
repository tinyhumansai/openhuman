# service

Service-management domain for the OpenHuman core daemon. It installs/uninstalls the core binary as a per-user OS service (macOS LaunchAgent, Linux systemd user unit, Windows scheduled task), drives its install/start/stop/status lifecycle, and orchestrates **self-restart** and **graceful shutdown** of the currently running core process via the event bus. It also persists machine-local daemon-host UI preferences (tray visibility). All of this is exposed transport-agnostically through the `service.*` RPC/CLI controller registry.

## Responsibilities

- Install / uninstall the core daemon as a native per-user service and report its `ServiceStatus`.
- Start / stop / query the installed service per OS (`launchctl`, `systemctl --user`, `schtasks`).
- Accept restart/shutdown requests, publish them to the event bus, and (via subscribers) respawn or `process::exit(0)` the running core with a 150ms flush grace window.
- Resolve the daemon executable to launch (env override `OPENHUMAN_CORE_BIN`, else sibling `openhuman-core[-*]` next to the current exe; macOS also searches `../Resources`).
- Persist and read daemon-host UI preferences (`show_tray`) next to the main config.
- Provide a deterministic, file-backed **mock** service backend for E2E tests (`OPENHUMAN_SERVICE_MOCK`).
- Expose `daemon_state.json` path helper consumed by doctor/health reporting.

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/service/mod.rs` | Export-focused module root; declares submodules and re-exports the public surface + controller-schema pair. |
| `src/openhuman/service/core.rs` | `ServiceState` / `ServiceStatus` types and the cross-platform `install`/`start`/`stop`/`status`/`uninstall` dispatchers that route to the mock or the per-OS impl via `cfg`. |
| `src/openhuman/service/ops.rs` | RPC handler layer (`service_install`, `service_start`, `service_stop`, `service_status`, `service_restart`, `service_shutdown`, `service_uninstall`, `daemon_host_get`/`set`) returning `RpcOutcome<T>`. Re-exported as `rpc`. |
| `src/openhuman/service/schemas.rs` | Controller schemas + `handle_*` adapters for the `service` namespace; `all_controller_schemas` / `all_registered_controllers`. |
| `src/openhuman/service/restart.rs` | Self-restart orchestration: `service_restart` (publishes event), `trigger_self_restart_now` (respawns current exe with original args), `apply_startup_restart_delay_from_env`, `RestartStatus`. |
| `src/openhuman/service/shutdown.rs` | Graceful-shutdown orchestration: `service_shutdown` (publishes event), `ShutdownStatus`. |
| `src/openhuman/service/bus.rs` | Event-bus subscribers `RestartSubscriber` / `ShutdownSubscriber` (filter domain `system`) and idempotent `register_*_subscriber` helpers; one-shot atomic gates so only the first request acts. |
| `src/openhuman/service/common.rs` | Shared OS helpers: service labels, `resolve_daemon_executable`, `daemon_program_args` (`["run"]`), `xml_escape`, command runners (`run_checked`/`run_capture`/`run_best_effort`/`run_check_silent`), Windows `CREATE_NO_WINDOW` suppression. |
| `src/openhuman/service/macos.rs` | LaunchAgent (`com.openhuman.core.plist`) install/start/stop/status/uninstall via `launchctl`; migrates legacy labels. |
| `src/openhuman/service/linux.rs` | systemd user-unit install/lifecycle via `systemctl --user`. |
| `src/openhuman/service/windows.rs` | Scheduled-task install/lifecycle via `schtasks`. |
| `src/openhuman/service/daemon.rs` | `state_file_path(config)` → `<config_dir>/daemon_state.json`, used by doctor/health. |
| `src/openhuman/service/daemon_host.rs` | `DaemonHostConfig { show_tray }` + async `load_for_config_dir` / `save_for_config_dir` (JSON next to config, `daemon_host_config.json`). |
| `src/openhuman/service/mock.rs` | File-backed deterministic mock backend gated on `OPENHUMAN_SERVICE_MOCK`; supports forced failures and an `agent_running` flag (`mock_agent_running`). |
| `src/openhuman/service/mock_tests.rs` | Sibling test suite for `mock.rs`. |

## Public surface

From `mod.rs` re-exports:

- `core::*` — `ServiceState`, `ServiceStatus`, and the dispatchers `install` / `start` / `stop` / `status` / `uninstall`.
- `ops::*` (also aliased `rpc`) — the async RPC handlers `service_install` … `service_uninstall`, `service_restart`, `service_shutdown`, `daemon_host_get`, `daemon_host_set`.
- `restart::{apply_startup_restart_delay_from_env, RestartStatus}` (and `restart::trigger_self_restart_now` via the `restart` module path).
- `shutdown::ShutdownStatus`.
- `schemas::all_service_controller_schemas` / `all_service_registered_controllers`.
- `daemon::state_file_path`, `daemon_host::{DaemonHostConfig, …}` (via their `pub mod` paths).

## RPC / controllers

Namespace `service` (called as `openhuman.service_<fn>` / `service.<fn>`). All nine handlers go through `RpcOutcome` and are wired into the registry by `crate::core::all` via `all_service_registered_controllers`.

| Method | Inputs | Output |
| --- | --- | --- |
| `service.install` | — | `ServiceStatus` |
| `service.start` | — | `ServiceStatus` |
| `service.stop` | — | `ServiceStatus` |
| `service.status` | — | `ServiceStatus` |
| `service.uninstall` | — | `ServiceStatus` |
| `service.restart` | `source?`, `reason?` | restart ack JSON (`RestartStatus`) |
| `service.shutdown` | `source?`, `reason?` | shutdown ack JSON (`ShutdownStatus`) |
| `service.daemon_host_get` | — | `DaemonHostConfig` |
| `service.daemon_host_set` | `show_tray` (required) | `DaemonHostConfig` |

Lifecycle handlers load config via `config::rpc::load_config_with_timeout`; `restart`/`shutdown` are intentionally config-free (they target the running process).

## Events

Publishes (in `restart.rs` / `shutdown.rs`) to the global event bus, domain `system`:

- `DomainEvent::SystemRestartRequested { source, reason }`
- `DomainEvent::SystemShutdownRequested { source, reason }`

Subscribes (in `bus.rs`):

- `RestartSubscriber` (`name = "service::restart"`) — on `SystemRestartRequested`, atomically claims a one-shot gate, calls `trigger_self_restart_now` to spawn a replacement process, then `process::exit(0)` after 150ms.
- `ShutdownSubscriber` (`name = "service::shutdown"`) — on `SystemShutdownRequested`, claims its gate and `process::exit(0)` after 150ms (no respawn).

Both subscribers are registered idempotently from `src/core/jsonrpc.rs` at startup via `register_restart_subscriber` / `register_shutdown_subscriber`; handles are held in process-lifetime `OnceLock`s so they are never dropped.

## Persistence

- **Daemon-host prefs** — `daemon_host_config.json` next to the main config (`DaemonHostConfig { show_tray }`, default `true`); read/written async by `daemon_host.rs`.
- **Daemon state path** — `daemon_state.json` next to config (`daemon::state_file_path`), consumed by doctor/health (not written here).
- **Service unit files** — written by the per-OS impls (macOS plist in `~/Library/LaunchAgents`, Linux systemd user unit, Windows scheduled task).
- **Mock state** — `service-mock-state.json` (overridable via `OPENHUMAN_SERVICE_MOCK_STATE_FILE`) tracking `installed`/`running`/`agent_running`/forced `failures`, only when the mock is enabled.

## Dependencies

- `crate::openhuman::config` — `Config` (paths, config dir) for every lifecycle/path operation; `config::rpc::load_config_with_timeout` in the schema handlers.
- `crate::core::event_bus` — `DomainEvent`, `EventHandler`, `SubscriptionHandle`, `publish_global` / `subscribe_global` / `init_global` for restart/shutdown orchestration.
- `crate::core` (`ControllerSchema`, `FieldSchema`, `TypeSchema`) and `crate::core::all` (`ControllerFuture`, `RegisteredController`) — controller schema/registration contract.
- `crate::rpc::RpcOutcome` — standard RPC result envelope.
- External: `anyhow`, `serde`/`serde_json`, `tokio`, `async_trait`, plus OS CLIs (`launchctl`, `systemctl`, `schtasks`).

## Used by

- `src/core/all.rs` — registers the service controllers (`all_service_registered_controllers`).
- `src/core/jsonrpc.rs` — registers the restart/shutdown event-bus subscribers at startup.
- `src/openhuman/doctor/core.rs` — reads `service::daemon::state_file_path`.
- `src/openhuman/update/ops.rs`, `src/openhuman/config/ops.rs`, `src/openhuman/app_state/ops.rs` — reference `openhuman::service` (status/lifecycle/restart paths).
- `src/lib.rs` — module wiring.

## Notes / gotchas

- Restart/shutdown are **two-phase**: the RPC/CLI call only acknowledges and publishes an event; the actual respawn/exit happens in the subscriber, so RPC, CLI, and internal triggers share one path with consistent logging. The subscriber must be registered or requests are no-ops.
- One-shot atomic gates exist in **both** `bus.rs` and `restart.rs` (`RESTART_IN_PROGRESS`) — duplicate restart events are ignored; a failed `trigger_self_restart_now` resets the gate to allow a retry.
- `trigger_self_restart_now` respawns the current exe with the **original argv** (preserving launch mode) and sets `OPENHUMAN_RESTART_DELAY_MS` (default 350) on the child; the child honors it at startup via `apply_startup_restart_delay_from_env` to dodge HTTP-port bind races while the old process releases sockets.
- Self-restart fails if launched with no args (`std::env::args().skip(1)` empty).
- Platform support is compile-gated; on unsupported targets the dispatchers `bail!` with "supported on macOS, Linux, and Windows only".
- `OPENHUMAN_SERVICE_MOCK` short-circuits **every** lifecycle dispatcher to the file-backed mock — used for deterministic E2E; it can also inject forced per-operation failures.
- Windows command spawns set `CREATE_NO_WINDOW` (`common::no_window`) so polled `schtasks /Query` calls don't flash a console.
- Lifecycle RPCs are deliberately **not** unit-tested (they mutate real OS state or kill the process); RPC-adapter coverage lives in `tests/json_rpc_e2e.rs`. `daemon_host_get`/`set` and the restart/shutdown publish paths are unit-tested.
- `daemon.rs::state_file_path` and `common.rs::state_file_path` (mock) both compute paths next to config but for different files (`daemon_state.json` vs the mock state file).
