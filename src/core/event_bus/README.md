# Event Bus

In-process pub/sub plus typed request/response. Owns the global `EventBus` singleton (built on `tokio::sync::broadcast`), the `DomainEvent` enum that names every cross-module event, the `NativeRegistry` (one-to-one typed dispatch keyed by method string with zero serialization), the `EventHandler` trait + `SubscriptionHandle` RAII guard, and the bundled `TracingSubscriber` debug logger. ~33 internal call sites — every domain that emits or consumes cross-module events lives here.

## Public surface

- `pub struct EventBus` — `bus.rs` — broadcast singleton over `tokio::sync::broadcast`.
- `pub const DEFAULT_CAPACITY: usize = 256` — `bus.rs` — default channel capacity.
- `pub fn init_global(capacity: usize) -> &'static EventBus` — `bus.rs` — initialize once at startup via `OnceLock::get_or_init`; subsequent calls return the already-initialized bus (capacity argument ignored).
- `pub fn global() -> Option<&'static EventBus>` — `bus.rs` — accessor; returns `None` before `init_global`.
- `pub fn publish_global(event: DomainEvent)` — `bus.rs` — fire-and-forget broadcast.
- `pub fn subscribe_global(handler: Arc<dyn EventHandler>) -> Option<SubscriptionHandle>` — `bus.rs` — register a subscriber.
- `pub enum DomainEvent` — `events.rs` — `#[non_exhaustive]` catalog of events; current variants cover Agent (`AgentTurnStarted/Completed`, `AgentError`), Memory (`MemoryStored`, `MemoryRecalled`), Channels (`ChannelInboundMessage`, `ChannelMessageReceived/Processed`, `ChannelReactionReceived/Sent`, `ChannelConnected/Disconnected`), Cron (`CronJobTriggered/Completed`, `CronDeliveryRequested`), Skills, Tools, Webhooks, and System.
- `pub trait EventHandler` — `subscriber.rs:12-24` — `name()` + optional `domains()` filter + async `handle()`.
- `pub struct SubscriptionHandle` — `subscriber.rs:29` — RAII; drop aborts the subscriber task.
- `pub struct TracingSubscriber` — `tracing.rs` — built-in handler that logs every event at `debug` level.
- `pub struct NativeRegistry` — `native_request.rs` — typed in-process request/response dispatcher keyed by method string.
- `pub enum NativeRequestError` — `native_request.rs` — `MethodNotFound`, `TypeMismatch`, etc.
- `pub fn init_native_registry() -> &'static NativeRegistry` / `pub fn native_registry() -> Option<&'static NativeRegistry>` / `pub fn register_native_global` / `pub fn request_native_global` — `native_request.rs`.
- `pub mod testing` — `testing.rs` — helpers to build isolated bus / registry instances per test.

## Calls into

- `tokio::sync::broadcast` for the broadcast channel.
- `async_trait` and `tokio::task::JoinHandle` for handler plumbing.
- No openhuman-domain dependencies — this module sits below every domain.

## Called by

- ~33 sites across the workspace. Hot consumers:
- `src/openhuman/agent/bus.rs`, `agent/triage/{events,evaluator,escalation}.rs`, `tools/impl/agent/{dispatch,spawn_subagent}.rs` — agent + sub-agent events.
- `src/openhuman/memory/conversations/bus.rs` — conversation persistence subscriber.
- `src/openhuman/channels/bus.rs` — `ChannelInboundSubscriber`.
- `src/openhuman/cron/{bus,scheduler}.rs` — `CronDeliverySubscriber` + `CronJobTriggered` emission.
- `src/openhuman/webhooks/bus.rs` — `WebhookRequestSubscriber`.
- `src/openhuman/health/bus.rs` — health-event subscriber.
- `src/openhuman/update/scheduler.rs` — update-cycle events.
- `src/openhuman/tree_summarizer/{engine,bus}.rs` — async summarisation triggers.
- `src/openhuman/composio/bus.rs`, `notifications/`, `learning/` — analytics fan-out.

## Tests

- Unit: `bus_tests.rs`, `events_tests.rs`, `native_request_tests.rs`.
- Test infrastructure: `testing.rs` exposes helpers; many domain tests construct a fresh `NativeRegistry::new()` for isolation, or override an existing method by re-registering it.
