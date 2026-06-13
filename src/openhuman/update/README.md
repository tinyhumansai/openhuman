# update

Self-update domain for the `openhuman-core` binary. Checks GitHub Releases (`tinyhumansai/openhuman`, "latest" endpoint) for a newer build of the platform-appropriate core binary, downloads + atomically stages it next to the running executable, and (depending on the configured restart strategy) publishes a self-restart so the Tauri shell/supervisor can swap it in. Also exposes a cheap no-network version probe and a periodic background checker. Network failures are classified so transient transport/HTTP problems don't spam Sentry.

## Responsibilities
- Query the GitHub Releases "latest" API and compare semver-ish tags against the compiled `CARGO_PKG_VERSION` (`is_newer`).
- Select the release asset matching this platform's target triple (`openhuman-core-{triple}`, `.exe` on Windows).
- Download the asset to a temp file, set `0o755` on Unix, and atomically rename it into the staging dir (current-exe dir by default).
- Orchestrate the full `check → apply → restart` flow (`update_run`), publishing a service restart for the `SelfReplace` strategy or staging-only for `Supervisor`.
- Run a periodic background checker (default 1h, floor 10 min) that logs availability and emits health events.
- Enforce a fail-closed mutation policy (`config.update.rpc_mutations_enabled`) and validate download URLs / asset names at the RPC boundary.
- Filter transient network/HTTP failures away from Sentry while keeping local `warn` diagnostics.

## Key files
| File | Role |
| --- | --- |
| `src/openhuman/update/mod.rs` | Export-focused. `pub use core::*`, `pub use ops as rpc`, `pub use types::*`, and the `all_update_controller_schemas` / `all_update_registered_controllers` re-exports. |
| `src/openhuman/update/core.rs` | Core logic: `current_version`, `platform_triple`, `check_available` (GitHub fetch + parse), `download_and_stage[_with_version]` (atomic staging), asset selection, semver compare, transport-failure classifier. |
| `src/openhuman/update/ops.rs` | RPC handlers (`update_version`/`update_check`/`update_apply`/`update_run`), mutation-policy enforcement, URL/asset-name validation, and `UpdateRunResult` builders per restart strategy. Aliased as `update::rpc`. |
| `src/openhuman/update/scheduler.rs` | `run(UpdateConfig)` periodic checker loop + `tick()`; publishes startup/health events. Floor `MIN_INTERVAL_MINUTES = 10`. |
| `src/openhuman/update/schemas.rs` | Controller registry: `all_controller_schemas`, `all_registered_controllers`, `schemas(fn)`, and `handle_*` thunks delegating to `ops`. |
| `src/openhuman/update/types.rs` | Serde types: `UpdateInfo`, `VersionInfo`, `UpdateRunResult`, `UpdateApplyResult`, `GitHubRelease`, `GitHubAsset`. |
| `src/openhuman/update/ops_tests.rs` | Sibling test suite for `ops.rs` (via `#[path]`). |

## Public surface
- Types (`types.rs`): `UpdateInfo`, `VersionInfo`, `UpdateRunResult`, `UpdateApplyResult`, `GitHubRelease`, `GitHubAsset`.
- Core fns (`core.rs`, re-exported via `core::*`): `current_version() -> &'static str`, `platform_triple() -> &'static str`, `check_available() -> Result<UpdateInfo, String>`, `download_and_stage(...)`, `download_and_stage_with_version(...)`.
- `update::rpc` (alias of `ops`): `update_version`, `update_check`, `update_apply`, `update_run` — all returning `RpcOutcome<Value>`.
- `update::scheduler::run(UpdateConfig)` — background loop entry point.
- `all_update_controller_schemas()` / `all_update_registered_controllers()`.

## RPC / controllers
All under namespace `update` (i.e. `openhuman.update_*`):
| Method | Inputs | Output |
| --- | --- | --- |
| `update.version` | none | `version_info` (`VersionInfo`) — cheap, no network. |
| `update.check` | none | `update_info` (`UpdateInfo`). |
| `update.apply` | `download_url` (req), `asset_name` (req), `staging_dir` (optional, **ignored** — always default dir) | `apply_result` (`UpdateApplyResult`). |
| `update.run` | none | `run_result` (`UpdateRunResult`) — orchestrated check→stage→restart. |

`apply` and `run` are gated by `enforce_update_mutation_policy` (fail-closed if config can't load) and re-validate the URL (must be HTTPS GitHub host) and asset name (must start `openhuman-core-`, no path separators / `..`).

## Agent tools
Not owned here — the domain has no `tools.rs`. Two cross-cutting system tools wrap it: `src/openhuman/tools/impl/system/update_check.rs` (read-only, calls `update::rpc::update_check`) and `src/openhuman/tools/impl/system/update_apply.rs` (calls `update::rpc::update_run`).

## Events
No `bus.rs`. The scheduler *publishes* (via `core::event_bus::publish_global`):
- `DomainEvent::SystemStartup { component: "update_checker" }` at startup.
- `DomainEvent::HealthChanged { component: "update_checker", healthy, message }` after each tick.

It also calls `health::bus::register_health_subscriber()` and `event_bus::init_global(...)` on start.

## Persistence
None. No `store.rs` — staged binaries are written to the filesystem (current-exe dir by default), but the domain holds no persisted state of its own. Configuration is read from `config.update`.

## Dependencies
- `crate::openhuman::config` — reads `UpdateConfig` (`enabled`, `interval_minutes`, `rpc_mutations_enabled`, `restart_strategy`) and `UpdateRestartStrategy`; `ops` loads it via `config::rpc::load_config_with_timeout`.
- `crate::openhuman::service` — `service::rpc::service_restart` to publish the self-restart for `SelfReplace`.
- `crate::openhuman::health` — `health::bus::register_health_subscriber` in the scheduler.
- `crate::openhuman::util` — `utf8_safe_prefix_at_byte_boundary` for safe error-body truncation.
- `crate::core::event_bus` — `publish_global`, `DomainEvent`, `init_global`, `DEFAULT_CAPACITY`.
- `crate::core::observability` — Sentry reporting + transient-failure classifiers (`report_error`, `is_updater_transient_message`, `is_updater_transient_http_status`).
- `crate::core::all` — `ControllerFuture`, `RegisteredController` (schemas wiring); `crate::core::{ControllerSchema, FieldSchema, TypeSchema}`.
- `crate::rpc::RpcOutcome` — RPC return contract.
- External crates: `reqwest` (HTTP), `url` (URL validation), `serde`/`serde_json`, `tokio`.

## Used by
- `src/core/all.rs` — registers `all_update_registered_controllers()` / `all_update_controller_schemas()` into the controller registry.
- `src/core/jsonrpc.rs` — spawns `update::scheduler::run(config.update)` at server start.
- `src/openhuman/tools/impl/system/update_check.rs` and `update_apply.rs` — agent tools wrapping the RPC layer.

## Notes / gotchas
- **`staging_dir` is intentionally ignored** by `update_apply` — it always uses the safe default (current-exe parent dir) regardless of caller input, for security.
- **Fail-closed policy**: if `config.update.rpc_mutations_enabled` is false (or config fails to load) `apply`/`run` are rejected; `check`/`version` remain available.
- **Restart strategies** (`UpdateRestartStrategy`): `SelfReplace` publishes a `service_restart` (process exits shortly after the RPC returns; `restart_requested` reflects whether the publish succeeded); `Supervisor` stages only and expects an external supervisor to restart.
- **Scheduler interval floor**: requested `interval_minutes` is clamped up to `MIN_INTERVAL_MINUTES = 10` to avoid GitHub unauthenticated rate-limits; the first check runs immediately.
- **Version compare** (`is_newer`) is dot-split numeric with `v`-prefix stripping — not full semver (no pre-release/build metadata handling).
- **Sentry hygiene**: transport-level reqwest failures (`is_connect`/`is_timeout`/`is_request`) and transient HTTP statuses are logged at `warn` and skipped from `report_error`; a regression guard test hits an unroutable TEST-NET-1 host to lock the classifier.
- **Test env locking**: tests touching `update_apply` take `config::TEST_ENV_LOCK` because the mutation policy is resolved through the process-global `OPENHUMAN_WORKSPACE` env var and would otherwise race.
- Network-hitting paths (`update_check` success, `update_apply` success, scheduler `tick`) are deferred to integration tests, not unit-tested.
