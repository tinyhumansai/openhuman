# notifications

The notifications domain owns two complementary sub-systems. The **core-bridge** translates selected `DomainEvent`s (cron completions, webhook failures, sub-agent results, triaged integration notifications) into compact `CoreNotificationEvent` payloads pushed over a broadcast channel into the Socket.IO bridge, surfacing as in-app notification-center items. The **integration-notification pipeline** ingests notifications captured from embedded webview integrations (Gmail, Slack, WhatsApp, …), persists them to a per-workspace SQLite store, runs each through the triage LLM pipeline in the background to back-fill an importance score/action, and exposes a unified RPC surface for listing, marking, settings, and stats.

## Responsibilities

- Subscribe to cross-domain `DomainEvent`s and republish a curated subset as user-facing `CoreNotificationEvent`s (with title/body/category/deep-link) onto an in-process broadcast bus consumed by the Socket.IO bridge.
- Filter noise at the bridge: only failed webhooks surface; only `routed` triage actions (`escalate`/`react`) surface; `drop`/`acknowledge`/unrouted are silent.
- Ingest integration notifications, dedup against identical content received in the last 60s, and persist them immediately.
- Spawn background triage per ingest; map the triage action to a 0.0–1.0 importance score and back-fill `importance_score`/`triage_action`/`triage_reason`/`scored_at` in place.
- Auto-route high-importance (`escalate`/`react`) notifications to the orchestrator when the provider's settings allow (re-reading settings just before routing).
- Persist and expose per-provider settings (enabled, importance threshold, route-to-orchestrator).
- Track lifecycle state (`unread`/`read`/`acted`/`dismissed`) and aggregate pipeline stats.

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/notifications/mod.rs` | Export-focused module root; docstring + re-exports of bus helpers, schema registries, and types. |
| `src/openhuman/notifications/types.rs` | Serde domain types: `CoreNotificationEvent`/`CoreNotificationCategory` (bridge), `IntegrationNotification`, `NotificationStatus`, `NotificationSettings`, `NotificationStats`, and RPC request types. |
| `src/openhuman/notifications/bus.rs` | `NotificationBridgeSubscriber` (`EventHandler`), the `NOTIFICATION_BUS` broadcast static, `publish_core_notification`/`subscribe_core_notifications`, the pure `event_to_notification` translator, and `register_notification_bridge_subscriber`. |
| `src/openhuman/notifications/rpc.rs` | Async RPC handler fns (`handle_ingest`, `handle_list`, `handle_mark_read`, `handle_dismiss`, `handle_mark_acted`, `handle_settings_get`/`_set`, `handle_stats`) + `triage_action_to_score` heuristic. |
| `src/openhuman/notifications/schemas.rs` | Controller schema defs, the `NOTIFICATION_CONTROLLER_DEFS` table, `all_controller_schemas`/`all_registered_controllers`, and handler wrappers delegating to `rpc.rs`. |
| `src/openhuman/notifications/store.rs` | SQLite persistence (`integration_notifications` + `notification_settings` + `core_notifications` tables) via a per-call `with_connection` helper; insert/list/dedup/triage-update/status/settings/stats queries plus core-notification persistence (#3805). |
| `src/openhuman/notifications/bus_tests.rs` | Sibling test suite for the bridge. |
| `src/openhuman/notifications/store_tests.rs` | Sibling test suite for the store. |

## Public surface

Re-exported from `mod.rs`:

- From `bus`: `publish_core_notification`, `subscribe_core_notifications`, `register_notification_bridge_subscriber`, `NotificationBridgeSubscriber`.
- From `schemas`: `all_notifications_controller_schemas` (alias of `all_controller_schemas`), `all_notifications_registered_controllers` (alias of `all_registered_controllers`).
- From `types` (`pub use types::*`): `CoreNotificationEvent`, `CoreNotificationCategory`, `IntegrationNotification`, `NotificationStatus`, `NotificationSettings`, `NotificationStats`, `NotificationIngestRequest`, `NotificationSettingsUpsertRequest`.

## RPC / controllers

Namespace `notification` (10 controllers, registered via `all_notifications_registered_controllers`):

| Function | Inputs | Output |
| --- | --- | --- |
| `ingest` | `provider`, `title`, `body`, `raw_payload` (req); `account_id` (opt) | `{ id?, skipped, reason? }` — persists then spawns background triage; skips when provider disabled or duplicate. |
| `list` | `provider?`, `limit?` (50), `offset?` (0), `min_score?` | `{ items, unread_count }` — ordered `received_at` DESC; unscored items pass the score filter. |
| `mark_read` | `id` | `{ ok }` |
| `dismiss` | `id` | `{ ok }` (true when a row matched) |
| `mark_acted` | `id` | `{ ok }` (true when a row matched) |
| `settings_get` | `provider` | `{ settings }` (defaulted if absent) |
| `settings_set` | `provider`, `enabled`, `importance_threshold`, `route_to_orchestrator` | `{ ok, settings }` — threshold clamped to 0.0–1.0. |
| `stats` | — | `{ total, unread, unscored, by_provider, by_action }` |
| `core_list` | `only_unread?` (true), `limit?` (100) | `{ items, unread_count }` — persisted core notifications (#3805), newest first; sync-down for events fired while the app was closed. |
| `core_mark_read` | `id` | `{ ok }` (true when a row matched) |

Schemas + handlers are wired into the controller registry in `src/core/all.rs`.

### Core-notification persistence (#3805)

Core notifications are broadcast-only; if no client is connected when the
event fires (app closed / minimised / disconnected) the broadcast reaches zero
receivers and the notification is lost. `NotificationBridgeSubscriber` therefore
**persists** each translated `CoreNotificationEvent` to a `core_notifications`
table (keyed by event id, so re-publishes dedupe) *before* broadcasting, and the
`core_list` / `core_mark_read` controllers let the frontend sync down and
acknowledge anything missed on the next app open.

## Agent tools

None. This domain owns no `tools.rs`.

## Events

**Subscribes** (via `NotificationBridgeSubscriber`, no `domains()` filter — matches on variant): `DomainEvent::CronJobCompleted` (→ Agents), `WebhookProcessed` (failures only → System), `SubagentCompleted`/`SubagentFailed` (→ Agents), and `NotificationTriaged` (only when `routed` and action is `escalate`/`react` → Agents). Each is translated to a `CoreNotificationEvent` and published on the broadcast bus.

**Publishes**: `DomainEvent::NotificationTriaged` from `rpc::handle_ingest`'s background triage task (carries `id`, `provider`, `action`, `importance_score`, `latency_ms`, `routed`).

The bridge bus is a separate `tokio::sync::broadcast` channel (not the global event bus); `core::socketio` subscribes to it and forwards each event as the `core_notification` / `core:notification` Socket.IO message.

## Persistence

SQLite DB at `{workspace_dir}/notifications/notifications.db`, opened per-call via `with_connection` (idempotent schema migration on each open). Two tables:

- `integration_notifications` — one row per ingested notification (id, provider, account_id, title, body, raw_payload JSON, importance_score, triage_action, triage_reason, status, received_at, scored_at). Indexed on provider, status, and a dedup tuple (provider, account_id, title, body, received_at).
- `notification_settings` — per-provider routing prefs (enabled, importance_threshold, route_to_orchestrator), upserted on `provider` PK.

`insert_if_not_recent` runs a `BEGIN IMMEDIATE` transaction so concurrent duplicate ingests collapse to a single insert.

## Dependencies

- `crate::core::event_bus` — `DomainEvent`, `EventHandler`, `publish_global`, `subscribe_global` for bridge subscription and triage-result publishing.
- `crate::core::all` (`ControllerFuture`, `RegisteredController`) and `crate::core` (`ControllerSchema`, `FieldSchema`, `TypeSchema`) — controller registry contract.
- `crate::openhuman::config` — `Config` (workspace dir for the DB path) and `config::rpc::load_config_with_timeout` in handlers.
- `crate::openhuman::agent::triage` — `run_triage`, `apply_decision`, `TriageOutcome`, `TriggerEnvelope`, `TriggerSource`, `TriageAction` for the background scoring/routing pipeline.
- `crate::rpc::RpcOutcome` — RPC response shaping.
- External crates: `rusqlite` (store), `chrono`, `uuid`, `serde_json`, `tokio`, `once_cell`, `async_trait`.

## Used by

- `src/core/all.rs` — registers the controllers/schemas into the RPC registry.
- `src/core/jsonrpc.rs` — calls `register_notification_bridge_subscriber()` at startup.
- `src/core/socketio.rs` — calls `subscribe_core_notifications()` to forward events to web clients.
- `src/openhuman/cron/scheduler.rs` and `src/openhuman/heartbeat/planner/*` reference the notification surface (e.g. triggering/observing notifications).

## Notes / gotchas

- `CoreNotificationEvent` ids embed a publish timestamp, so each cron run / webhook failure / subagent event produces a distinct notification-center entry rather than coalescing.
- `CoreNotificationCategory` must stay in sync with `NotificationCategory` in `app/src/store/notificationSlice.ts`.
- The ingest RPC returns immediately; triage runs in a spawned task and back-fills the score later — list/stats may show `importance_score: null` (unscored) until triage completes.
- Triage→score mapping is a fixed heuristic in `rpc::triage_action_to_score`: Drop 0.1, Acknowledge 0.35, React 0.65, Escalate 0.9.
- Routing re-reads provider settings just before escalation so a mid-flight settings toggle takes effect; routing requires `score >= importance_threshold` AND `route_to_orchestrator`.
- Dedup window is a hard-coded 60 seconds (`exists_recent` / `insert_if_not_recent`).
- The bridge bus is fire-and-forget: with no subscribers, events are dropped (`publish_core_notification` returns the receiver count).
