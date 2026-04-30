# Cron

Scheduled-job runtime. Owns cron-expression and human-delay parsing, the persistent job + run store, the polling scheduler that fires due jobs (`shell` and `agent` types), and the delivery layer that publishes events into the agent / channel pipelines. Does NOT own the actual agent execution (`agent::triage`) or shell sandboxing (`security::SecurityPolicy`).

## Public surface

- `pub struct CronJob` / `pub struct CronJobPatch` / `pub struct CronRun` / `pub enum JobType` / `pub enum Schedule` / `pub enum SessionTarget` / `pub struct DeliveryConfig` — `types.rs:1-100` — durable job + run model.
- `pub fn add_once` / `pub fn add_once_at` / `pub fn parse_human_delay` / `pub fn pause_job` / `pub fn resume_job` / `pub fn update_cron_job` — `ops.rs` (re-exported `mod.rs:12`).
- `pub fn schedule_cron_expression` / `pub fn next_run_for_schedule` / `pub fn normalize_expression` / `pub fn validate_schedule` — `schedule.rs` (re-exported `mod.rs:14-16`).
- `pub fn add_job` / `pub fn add_agent_job` / `pub fn add_agent_job_with_definition` / `pub fn add_shell_job` / `pub fn due_jobs` / `pub fn get_job` / `pub fn list_jobs` / `pub fn list_runs` / `pub fn record_last_run` / `pub fn record_run` / `pub fn remove_job` / `pub fn reschedule_after_run` / `pub fn update_job` — `store.rs` (re-exported `mod.rs:22-26`).
- `pub mod scheduler` (`pub async fn run(config: Config)`) — `scheduler.rs:19` — main poll loop.
- `pub mod seed` — `seed.rs` — install built-in jobs on first launch.
- `pub mod bus` — `bus.rs` — `CronDeliverySubscriber` for the event bus.
- RPC `cron.{add, list, update, remove, run, runs}` — `schemas.rs` (re-exported via `all_cron_controller_schemas` / `all_cron_registered_controllers`).

## Calls into

- `src/openhuman/agent/` — `agent` job type runs through `agent::triage::TriggerEnvelope::from_cron` + `apply_decision`.
- `src/openhuman/security/` — `SecurityPolicy::from_config` sandboxes shell jobs.
- `src/openhuman/config/` — `Config` provides poll interval, workspace dir, autonomy policy.
- `src/openhuman/health/` — `health::bus::register_health_subscriber` on startup.
- `src/openhuman/channels/` — `bus.rs` can fan delivery events into channels.
- `src/core/event_bus/` — `init_global`, `publish_global(DomainEvent::Cron(*))`.

## Called by

- `src/openhuman/tools/impl/system/schedule.rs` — `schedule` tool exposes cron operations to agents.
- `src/core/all.rs` — controller registry wires `all_cron_*`.
- Channel and agent runtimes consume `Cron` events via the bus.

## Delivery modes

A cron job's `DeliveryConfig.mode` decides where its output ends up:

- **`proactive`** (default for agent jobs) — `deliver_if_configured` publishes
  `DomainEvent::ProactiveMessageRequested`. The proactive subscriber
  (`channels::proactive`) always pushes to the in-app web stream and additionally
  mirrors to `channels_config.active_channel` when set. Use for jobs whose
  natural surface is the desktop UI (briefings, app-pushed notifications).
- **`announce`** — explicit channel-targeted delivery. Requires `channel` and
  `to`; publishes `DomainEvent::CronDeliveryRequested` and lands only in that
  channel. The agent layer should pick this mode when a cron is created from a
  non-web channel (Telegram, Discord, Slack, …) so the reminder ends up where
  the user asked for it. The `cron_add` tool validates `to` against the
  channel's `allowed_users` to reject cross-tenant targets.
- **`none`** — silent; output is stored in `last_output` only.

The `[Channel context]` block injected by `channels::runtime::dispatch` for
non-web inbound turns instructs the model to default to `announce` with the
current channel + reply target — that is the routing path for the Telegram
"remind me to drink water" use case in #928.

## Tests

- Unit: `ops_tests.rs`, `scheduler_tests.rs`, `store_tests.rs`.
- Schema/parsing coverage lives inside `schedule.rs` and `schemas.rs` `#[cfg(test)] mod tests` blocks.
- Delivery validation: `tools::impl::cron::add::tests` (announce-mode `allowed_users` checks).
