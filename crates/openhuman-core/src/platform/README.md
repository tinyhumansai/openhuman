# platform

Services about the core process itself rather than about what the agent is
doing: is it running, is it healthy, what did it cost, is it connected to the
backend, does it need an update, how does it restart. The desktop shell, the
CLI, and the frontend reach these through ordinary JSON-RPC controllers, and
the boot path in `core/runtime/` starts the background pieces.

Nothing here is feature-gated. A build whose only inference driver is an
external backend still has to start, stay healthy, report cost, and be
reachable, so this family ships in every build. Its controllers are tagged
`DomainGroup::Platform`, the catch-all group that `DomainSet::platform`
switches at runtime.

## How it works

Most members are small request and response domains. A few take part in
startup, and two are driven by events rather than calls. In boot order:

```text
 core/runtime/bootstrap.rs
   |- platform::cost::init_global(cfg.cost, workspace)    cost ledger singleton
   |- platform::startup::run_workspace_migrations(ws)     one-shot migrations
   |- platform::socket::set_global_socket_manager(..)    SocketManager singleton
 core/runtime/subscribers.rs
   |- platform::health::bus::register_health_subscriber()
   |- platform::service::bus::register_restart_subscriber()
   |- platform::service::bus::register_shutdown_subscriber()
 core/runtime/services.rs
   |- platform::update::scheduler::run(config.update)     periodic update check
   |- spawn_socket_auto_connect(..)                       if socketio service on
        token_provider_from_config(config)                and a transport is
                                                          installed
 core/all.rs
   '- about_app, health, doctor, cost, connectivity,
      service, socket, update controllers  (DomainGroup::Platform)
```

The event-driven pieces:

- [`health`](./health) keeps an in-memory component registry. Its subscriber listens to
  `system` and `channel` domain events and marks components ok or in error.
  `GET /health` on the RPC server reads the verdict: 503 only when a critical
  component (`core`) is unhealthy, 200 with a `degraded` flag otherwise.
- [`service`](./service) turns a restart or shutdown request into
  `DomainEvent::SystemRestartRequested` or `SystemShutdownRequested`. Its
  subscribers respawn the process or exit after a short flush window.
  [`update`](./update) uses the same restart path after staging a new binary.
- [`socket`](./socket) goes the other way. It is the core's own Socket.IO client to the
  hosted backend, and it republishes inbound server events onto the bus
  (`WebhookIncomingRequest`, `ComposioTriggerReceived`,
  `ChannelInboundMessage`, device tunnel events) for other domains to handle.

Some members read state other domains own instead of keeping their own.
[`doctor`](./doctor) reads the daemon state file `service` writes and the memory engine
status from `memory`. [`connectivity`](./connectivity) reads the `SocketManager` state to tell
the frontend which of its connection channels is broken.

## Layout

| Path | What it does |
| --- | --- |
| [`about_app/`](about_app/README.md) | The user-facing capability catalog: what each feature does, where it is in the UI, its maturity, and its privacy disclosure. A compile-time static table with list, lookup, and search. Update it when user-visible capabilities change. |
| [`connectivity/`](connectivity/README.md) | `connectivity_diag` (backend socket state, loop liveness, core PID and port, a loopback port probe) and the listen-port picker the core uses to bind its HTTP listener, including stale-listener takeover and the Windows excluded-port fallback. |
| [`cost/`](cost/README.md) | Token usage and USD cost: an append-only `costs.jsonl` ledger, day and month aggregates, the 7-day dashboard, the static pricing and context-window catalog (`catalog.rs`), and managed vs BYOK route classification (`route.rs`). |
| [`doctor/`](doctor/README.md) | Self-checks over config, workspace, the daemon state file, the environment, the memory engine, and the embedding model, aggregated into a severity-tagged `DoctorReport`. Powers `openhuman doctor` and the Settings health view. |
| [`health/`](health/README.md) | The in-process component health registry, the `HealthVerdict` behind `GET /health`, and static system info. |
| [`proc_metrics/`](proc_metrics/README.md) | Cross-platform RSS and peak-RSS sampling for this process and its tree. Used by the `rss-bench` and `library-profile` bins in openhuman-benchmarks (`profile/`). No RPC. |
| [`service/`](service/README.md) | Installs the core as a per-user OS service (LaunchAgent, systemd user unit, Windows scheduled task), self-restart and graceful shutdown over the bus, daemon-host tray preferences, and a file-backed mock for E2E (`OPENHUMAN_SERVICE_MOCK`). |
| [`socket/`](socket/README.md) | The persistent Socket.IO client to the backend: hand-rolled Engine.IO and Socket.IO handshakes over a WebSocket, reconnect with backoff, a token provider re-read on every reconnect, redirect following, and inbound event dispatch. |
| [`startup/`](startup/README.md) | One-shot workspace migrations run during boot (session layout, welcome-agent artifacts). Errors are logged and never abort startup. |
| [`update/`](update/README.md) | Self-update from GitHub Releases: check, download and atomically stage the platform binary, then restart (`SelfReplace`) or leave it for a supervisor. Plus the periodic background checker. |

## Key types and entry points

There is no type shared across the family. The ones contributors touch most:

- `HealthSnapshot`, `mark_component_ok`, `mark_component_error` ([`health/`](./health/))
  for reporting component state.
- `CostTracker` and `cost::init_global` / `try_global` / `rebind_global`
  ([`cost/global.rs`](./cost/global.rs)). The tracker is bound to one workspace, and signing in or
  out rebinds it so usage lands in the right user's ledger.
- `DoctorReport` ([`doctor/core/types.rs`](./doctor/core/types.rs)).
- `SocketManager` and `token_provider_from_config` ([`socket/`](./socket/)).
- `pick_listen_port_for_host` and its variants ([`connectivity/`](./connectivity/)), used by the
  RPC host when binding.
- `ServiceStatus` and the restart and shutdown publishers ([`service/`](./service/)).

## RPC / CLI surface

Method names are `openhuman.<namespace>_<function>`.

| Namespace | Functions |
| --- | --- |
| [`about_app`](./about_app) | `list`, `lookup`, `search` |
| `connectivity` | `diag` |
| [`cost`](./cost) | `get_dashboard`, `get_daily_history`, `get_summary`, `get_usage_log` |
| [`doctor`](./doctor) | `report`, `models` |
| `health` | `snapshot`, `system_info` |
| `service` | `install`, `start`, `stop`, `status`, `uninstall`, `restart`, `shutdown`, `daemon_host_get`, `daemon_host_set` |
| `socket` | `connect`, `connect_with_session`, `disconnect`, `emit`, `state` |
| [`update`](./update) | `version`, `check`, `apply`, `run` |

Agent tools from this family (registered in `tools/ops.rs`):
`health_snapshot`, `health_system_info`, `doctor_health`, `doctor_models`,
`cost_get_dashboard`, `cost_get_daily_history`, `cost_get_summary`,
`service_restart`, `service_shutdown`, and `daemon_host_prefs_set`. The
lifecycle mutators ship off by default through the user tool filter.

## Boundaries

- The JSON-RPC server, `/health` route, and Socket.IO server the frontend
  connects to belong to `crates/openhuman-rpc`. `platform::socket` is the
  core's outbound client to the backend, not that server.
- The backend URL belongs to the host crate that registers the config enricher
  (`crates/openhuman-tinyhumans`). `socket` reads `Config.resolved_endpoints`;
  with none resolved it does not auto-connect.
- The session credential belongs to the host's session owner. `socket` reads
  whatever token the core was given through `auth.set_credential`.
- How the desktop shell supervises the core belongs to
  `crates/openhuman-app/src/core_process.rs`.

## Gotchas

- `update` mutations are fail-closed behind
  `config.update.rpc_mutations_enabled`, and download URLs and asset names are
  validated at the RPC boundary.
- Cost history is recorded even when `cost.enabled` is off
  (`record_usage_unconditional`), and the local limits apply only to records
  classified as managed. BYOK and local calls are recorded but never gated.
- `socket` reports `Reconnecting` between attempts and `Disconnected` only
  when the loop is not running. The frontend's connectivity chip depends on
  that distinction.

## Tests

Each member keeps sibling `*_tests.rs` files. The `service` mock backend makes
lifecycle tests deterministic.

```bash
cargo test -p openhuman platform::
pnpm debug rust platform::
```

## Further reading

- [Platform and availability](../../../../docs/gitbooks/en/features/platform.md)
- [Tauri shell architecture](../../../../docs/gitbooks/en/developing/architecture/tauri-shell.md)
- [Architecture overview](../../../../docs/gitbooks/en/developing/architecture.md)
