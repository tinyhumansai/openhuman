# desktop

Core-side logic that exists to serve the desktop client: the app-state
snapshot the shell polls, the notification center, overlay attention bubbles,
the model-health dashboard, the provider respond queue, and opt-in native
desktop control. A headless or embedded host has little use for any of it,
which is why these domains are grouped here and tagged `DomainGroup::Desktop`.

This is not the Tauri shell. The shell lives in [`crates/openhuman-app/`](../../../openhuman-app/) and
owns windows, the tray, and native OS integration. This folder is the
business logic those windows reach over JSON-RPC and Socket.IO. The browser
SPA uses the same RPC surface; the TUI ([`crates/openhuman-tui`](../../../openhuman-tui/)) uses little of
it.

## How it works

The members are independent. They share a host-facing role, not a data model.
Each one meets the frontend in one of two ways:

```text
             React shell / notch window / browser SPA
               |                         ^
     JSON-RPC  |                         |  Socket.IO
  (openhuman.*)|                         |  (openhuman-rpc server/socketio.rs)
               v                         |
 +--------------------------+   +-------------------------------+
 | controllers (core/all.rs)|   | in-process broadcast channels |
 |  app_state_*             |   |  overlay::subscribe_attention |
 |  notification_*          |   |    _events                    |
 |  dashboard_*             |   |  notifications::subscribe_    |
 |  provider_surfaces_*     |   |    core_notifications         |
 |  desktop_* (internal)    |   +-------------------------------+
 +--------------------------+               ^
               |                            | publish
               v                            |
      peer domains (config, auth,   DomainEvent bus -> notifications::bridge
      local AI, service, keyring)   any caller -> overlay::publish_attention
```

Request and response flows (the app-state snapshot, the notification list,
model health, the respond queue, desktop status) are ordinary controllers
registered under `DomainGroup::Desktop` in [`core/all.rs`](../core/all.rs). Push flows use
process-global `tokio::sync::broadcast` channels that the Socket.IO server in
[`crates/openhuman-rpc`](../../../openhuman-rpc/) subscribes to and forwards. Notifications get onto
their channel through `NotificationBridgeSubscriber` (`notifications::bridge`),
which translates selected `DomainEvent`s; it is registered at startup from
[`core/runtime/subscribers.rs`](../core/runtime/subscribers.rs). Overlay attention events are published
directly by any caller.

### Desktop control

`control/` is different from the rest: it lets the agent drive native desktop
apps through the `tinycomputer` module's accessibility layer, with Jev picking
actions. It is opt-in per computer. The enabled flag lives in
`<workspace>/state/desktop-control.json` and fails closed when the file is
missing or corrupt. Control is only supported on macOS and Windows, and only
when the core's HTTP listener is bound to loopback
(`set_listener_is_loopback`, set from [`core/runtime/builder.rs`](../core/runtime/builder.rs) once the
address is known). The tools refuse to run when the listener is not loopback
or control is disabled.

The agent tools are `desktop_list_apps`, `desktop_list_windows`,
`desktop_launch`, `desktop_snapshot`, `desktop_find`, `desktop_goal`, and
`desktop_continue_goal` (`DesktopTool` with a `DesktopToolKind`, registered in
[`tools/ops.rs`](../tools/ops.rs)). A `desktop_goal` call hands one bounded task to the module,
which runs and verifies several actions in that call. When the operator turns
desktop approvals on (`config.desktop.approvals_enabled`), the module can stop
on a pending action. `confirmation.rs` keeps those one-use decisions for ten
minutes and exposes them through `desktop_pending` and `desktop_confirm`. With
approvals off (the default), `approvals_disabled_for` lets these seven tools
skip only the approval park; permission caps, denies, OS grants, and the
action budget still apply. The agent-facing instructions are in
[`control/WORKFLOW.md`](./control/WORKFLOW.md).

## Layout

| Path | What it does |
| --- | --- |
| [`mod.rs`](./mod.rs) | Declares the members. Only `control` is feature-gated. |
| [`app_state/`](app_state/README.md) | The snapshot the shell polls (`app_state_snapshot`): stored credential and user, local-AI and service status, onboarding tasks, keyring status, config-recovery notice. Owns `state/app-state.json` and `update_local_state`. |
| `control/` | Opt-in native desktop control: enablement and status (`ops.rs`), pending-action decisions (`confirmation.rs`), the `desktop_*` agent tools (`tools.rs`), and internal-only controllers (`schemas.rs`). `modules` feature only. Agent guidance in `WORKFLOW.md`. |
| [`dashboard/`](dashboard/README.md) | Read-only per-model health table for Settings, built from `model_registry` and the `dashboard.model_health` thresholds. Also the `dashboard_model_health` agent tool. |
| [`notifications/`](notifications/README.md) | Two pipelines: the event bridge that turns `DomainEvent`s into `CoreNotificationEvent`s, and the integration-notification store (SQLite) with background triage and per-provider settings. |
| [`overlay/`](overlay/README.md) | One broadcast channel for short attention messages shown in the notch or overlay window. No RPC, no persistence. |
| [`provider_surfaces/`](provider_surfaces/README.md) | An early scaffold: a normalized `ProviderEvent` model and an in-memory respond queue (soft cap 500) over embedded provider webviews. |

## Key types and entry points

- `app_state::snapshot()` ([`app_state/ops/snapshot.rs`](./app_state/ops/snapshot.rs)) assembles
  `AppStateSnapshot` from peer domains. It never calls the backend; the user
  it reports is what the host passed in through `auth.set_credential`.
- `notifications::register_notification_bridge_subscriber(config)` and
  `notifications::publish_core_notification` / `subscribe_core_notifications`
  ([`notifications/bus.rs`](./notifications/bus.rs)) are the push path for the notification center.
- `overlay::publish_attention(OverlayAttentionEvent)` returns how many
  subscribers received the event (zero means it was dropped).
- `control::status`, `control::set_enabled`, and `control::probe`
  ([`control/ops.rs`](./control/ops.rs)) back the Connections page.
- `DesktopTool` and `DesktopToolKind` ([`control/tools.rs`](./control/tools.rs)) are the desktop
  agent tools, re-exported through [`tools/mod.rs`](../tools/mod.rs).

## RPC / CLI surface

Method names are `openhuman.<namespace>_<function>`.

| Namespace | Functions | Notes |
| --- | --- | --- |
| `app_state` | `snapshot`, `update_local_state` | Polled by the shell every few seconds. |
| `notification` | `ingest`, `list`, `mark_read`, `dismiss`, `mark_acted`, `settings_get`, `settings_set`, `stats`, `core_list`, `core_mark_read` | Integration notifications plus persisted core notifications. |
| `dashboard` | `model_health` | Errors when `dashboard.model_health.enabled` is false. |
| `provider_surfaces` | `ingest_event`, `list_queue` | In-memory only. |
| `desktop` | `status`, `set_enabled`, `probe`, `pending`, `confirm` | Internal-only (`build_internal_only_controllers`): callable by the shell, hidden from agent tool listings. `modules` feature only. |

## Boundaries

- Windows, tray, notch window, and OS integration belong to the Tauri shell
  (`crates/openhuman-app/`). The notch window host is
  [`crates/openhuman-app/src/notch_window.rs`](../../../openhuman-app/src/notch_window.rs).
- Socket.IO forwarding belongs to [`crates/openhuman-rpc/src/server/socketio.rs`](../../../openhuman-rpc/src/server/socketio.rs).
- OS accessibility (AX and IOKit FFI, the Swift helper, focus, permissions,
  the Globe key) executes inside the compiled TinyComputer module in
  [`vendor/tinycomputer`](../../../../vendor/tinycomputer/) (upstream `tinyhumansai/tinycomputer`). Desktop
  observation, interaction, and Jev-driven goals belong to the `tinycomputer`
  module; this folder calls it through `tinycomputer-bus`.
- The live current user belongs to the host's session owner
  (`openhuman_tinyhumans::session`), not to `app_state`.
- Dictation overlay activation is driven by `voice::dictation_listener`, not
  by `overlay`.

## Gotchas

- Only `control` is feature-gated (`modules`). Everything else compiles
  unconditionally. The runtime filter is `DomainSet::desktop`
  (`core/runtime/builder.rs`). There is no `desktop` Cargo feature yet; the
  grouping exists so one can drop the family as a unit later. (The `mod.rs`
  doc points at `docs/specs/2026-08-02-core-kernel-domain-reorg.md`, which is
  not in the repository.)
- `control` checks loopback on every tool call and every confirmation, not
  just at enable time. A core bound to a non-loopback address reports
  `supported: false`.
- The overlay and notification channels drop events when no one is
  subscribed. They are notifications, not a queue.
- `provider_surfaces` keeps its queue in process memory, so it is empty after
  a restart.

## Tests

Each member keeps sibling `*_tests.rs` files.

```bash
cargo test -p openhuman desktop::
pnpm debug rust desktop::
```

## Further reading

- [Parent module README](../../README.md)
- [Tauri shell](../../../../docs/gitbooks/en/developing/architecture/tauri-shell.md)
- [Frontend](../../../../docs/gitbooks/en/developing/architecture/frontend.md)
- [openhuman-app crate](../../../openhuman-app/README.md)
