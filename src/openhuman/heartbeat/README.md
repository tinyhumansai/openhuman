# heartbeat

Periodic background scheduler that wakes on a configurable interval to do two things: (1) run the **planner** — evaluate upcoming meetings, cron reminders, and urgent notifications and dispatch deduplicated proactive notifications; and (2) optionally drive the **subconscious** engine for task-driven evaluation via local model inference. The loop reloads config before every tick so UI setting changes apply without an app restart, and it sleeps before the first tick so a fresh login never burns budget immediately.

## Responsibilities

- Run a long-lived tick loop (`HeartbeatEngine::run`) gated by `[heartbeat]` config; clamps the interval to a 5-minute floor.
- Reload config each tick and re-emit a settings line only when the relevant settings change.
- On each tick, run the planner (`evaluate_and_dispatch`) to collect → plan → dedupe → persist → notify for three categories: meetings, reminders, important events.
- When `inference_enabled`, fetch the shared global subconscious engine and call `engine.tick()`; otherwise (legacy mode) just count tasks in `HEARTBEAT.md`.
- Collect calendar meetings via Composio (mode-aware: backend vs direct), cron reminders, and unread/urgent integration notifications.
- Pick a delivery stage + message text per event based on lead time (`plan_delivery_for_event`).
- Dedupe deliveries both in-tick (content-based `overlap_key`) and across ticks (durable SQLite store), prune dedupe rows older than 14 days.
- Persist each alert into the notifications store and emit a core notification; optionally emit a `ProactiveMessageRequested` event for external channel delivery.
- Expose RPC to read/update heartbeat settings and to force one immediate planner tick.
- Seed a default `HEARTBEAT.md` in the workspace (`ensure_heartbeat_file`).

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/heartbeat/mod.rs` | Module docstring + exports; re-exports planner/rpc/engine and the controller-schema pair. |
| `src/openhuman/heartbeat/engine.rs` | `HeartbeatEngine` — the tick loop, config reload, planner-tick + subconscious dispatch, `collect_tasks`/`parse_tasks`/`ensure_heartbeat_file`. |
| `src/openhuman/heartbeat/rpc.rs` | RPC handlers `settings_get` / `settings_set` / `tick_now`; `HeartbeatSettingsPatch` / `HeartbeatSettingsView`; clamps and applies settings, bootstraps/stops the loop. |
| `src/openhuman/heartbeat/schemas.rs` | Controller schemas + `handle_*` thunks for the three RPC methods. |
| `src/openhuman/heartbeat/planner/mod.rs` | `evaluate_and_dispatch` — orchestrates collect → plan → dedupe → persist → notify; owns the cross-tick + in-tick dedupe logic. |
| `src/openhuman/heartbeat/planner/types.rs` | `HeartbeatCategory`, `PendingEvent`, `PlannedDelivery`, public `PlannerRunSummary`. |
| `src/openhuman/heartbeat/planner/collectors.rs` | Source collectors: `collect_cron_reminders`, `collect_calendar_meetings` (Composio fan-out + rotation), `collect_relevant_notifications`, calendar payload extraction. |
| `src/openhuman/heartbeat/planner/plan.rs` | `plan_delivery_for_event` — stage selection (`heads_up`/`final_call`/`starting_now`, `soon`/`due`, `important_now`) and message text per category and lead time. |
| `src/openhuman/heartbeat/planner/persistence.rs` | `persist_heartbeat_alert` — writes a durable `IntegrationNotification` (provider `"heartbeat"`, `triage_action="react"`) into the notifications store. |
| `src/openhuman/heartbeat/planner/store.rs` | SQLite dedupe store (`heartbeat_notification_state`): `mark_sent`, `prune_old`, `SentMarker`. |
| `src/openhuman/heartbeat/planner/utils.rs` | Pure helpers: `sanitize_preview`, `stable_key` (SHA-256), `compute_overlap_key` (category + normalized title + 15-min bucket). |

## Public surface

- `engine::HeartbeatEngine` — `new(config, workspace_dir)`, `run()`, `collect_tasks()`, `parse_tasks()`, `ensure_heartbeat_file(dir)`.
- `planner::evaluate_and_dispatch(config, now) -> PlannerRunSummary`.
- `planner::PlannerRunSummary` — `{ source_events, deliveries_attempted, deliveries_sent, deliveries_skipped_dedup }`.
- `rpc::{settings_get, settings_set, tick_now}`, `rpc::{HeartbeatSettingsPatch, HeartbeatSettingsView}`.
- `all_heartbeat_controller_schemas` / `all_heartbeat_registered_controllers` (re-exported from `schemas.rs`).

## RPC / controllers

Namespace `heartbeat` (wired into the registry in `src/core/all.rs`):

| Method | Inputs | Output | Notes |
| --- | --- | --- | --- |
| `heartbeat.settings_get` | — | `settings` (JSON `HeartbeatSettingsView`) | Read current settings. |
| `heartbeat.settings_set` | optional `enabled`, `interval_minutes`, `inference_enabled`, `notify_meetings`, `notify_reminders`, `notify_relevant_events`, `external_delivery_enabled`, `meeting_lookahead_minutes`, `max_calendar_connections_per_tick`, `reminder_lookahead_minutes` | `settings` (updated view) | Clamps (`interval_minutes`≥5, lookaheads/cap≥1), saves config, then bootstraps (`subconscious::global::bootstrap_after_login`) or stops (`stop_heartbeat_loop`) the loop. |
| `heartbeat.tick_now` | — | `summary` (JSON `PlannerRunSummary`) | Runs one immediate planner tick. |

## Events

- **Publishes** `DomainEvent::ProactiveMessageRequested` (via `core::event_bus::publish_global`) when `external_delivery_enabled` and the planned delivery sets `allow_external` — routes the proactive message to active external channels.
- **Publishes** core notifications via `notifications::bus::publish_core_notification` (`CoreNotificationEvent`) for every delivered alert.
- This module defines no `EventHandler`/subscriber of its own (no `bus.rs`).

## Persistence

- **Dedupe store** (`planner/store.rs`): SQLite at `{workspace_dir}/heartbeat/heartbeat_state.db`, table `heartbeat_notification_state` keyed by `dedupe_key` (`INSERT OR IGNORE` for atomic dedupe). Rows older than 14 days are pruned each tick.
- **Durable alerts** (`planner/persistence.rs`): written into the shared notifications store (provider `"heartbeat"`) — not owned by this module's schema.
- The dedupe marker is written **after** the durable notification persists, so a failed persist never permanently suppresses retries.

## Dependencies

- `crate::openhuman::config` — `Config` / `HeartbeatConfig`; reads the `[heartbeat]` block, `load_or_init` / `load_config_with_timeout` / `save`.
- `crate::openhuman::subconscious::global` — `get_or_init_engine` (shared engine for inference ticks), `bootstrap_after_login`, `stop_heartbeat_loop`.
- `crate::openhuman::cron` — `list_jobs`, `CronJob` to surface reminder-like jobs.
- `crate::openhuman::composio` — `client` (mode-aware factory, backend/direct list+execute), `types`, `googlecalendar_args` to poll Google Calendar events.
- `crate::openhuman::notifications` — `store` (read unread/urgent items, `insert_if_not_recent`), `bus::publish_core_notification`, `types` (`IntegrationNotification`, `NotificationStatus`, `CoreNotificationEvent`/`CoreNotificationCategory`).
- `crate::core::event_bus` — `publish_global` / `DomainEvent` for proactive-message dispatch.
- `crate::core::all` / `crate::core::{ControllerSchema, FieldSchema, TypeSchema}` — controller registration.
- `crate::rpc::RpcOutcome` — RPC return contract.
- External crates: `rusqlite` (dedupe store), `sha2` + `hex` (stable keys), `chrono`, `serde`/`serde_json`.

## Used by

- `src/core/all.rs` — registers the heartbeat controllers + schemas into the RPC registry.
- `src/openhuman/subconscious/global.rs` — constructs `HeartbeatEngine` and owns its run lifecycle.
- `src/openhuman/workspace/ops.rs` — calls `HeartbeatEngine::ensure_heartbeat_file` during workspace setup.

## Notes / gotchas

- **5-minute floor** on `interval_minutes` is enforced both at the RPC clamp and at runtime in `run()`.
- **First tick is delayed** — the loop sleeps for the interval before the first evaluation (fresh-login budget safety).
- **Cross-source dedupe** uses `overlap_key` = `category | normalized_title | 15-min bucket`, so the same meeting surfaced by both a cron job and a calendar connection only notifies once; the disk dedupe key folds in the delivery `stage`.
- **All-day calendar events are intentionally skipped** — extraction only accepts `start.dateTime`, never `start.date` (avoids birthdays/OOO/holidays being promoted to meetings).
- **Self-escalation guard**: `collect_relevant_notifications` filters out `provider == "heartbeat"` to avoid a feedback loop where each tick re-escalates its own alerts.
- **Calendar fan-out is rotated** across ticks (`select_calendar_connections_for_tick`) and capped by `max_calendar_connections_per_tick` so a user with many calendar connections doesn't hammer Composio in a single tick.
- **Timezone correctness**: calendar start times are parsed via RFC3339 offset parsing and normalized to UTC (regression-pinned for non-UTC offsets, issue #1714); direct-mode users see their own calendar, not the backend tenant's (#1710).
- Grace windows extend up to ~10 minutes past an anchor so tick alignment doesn't miss a meeting/reminder.
- `mark_sent` returns `false` on an existing key (dedupe hit) rather than erroring.
