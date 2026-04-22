# Event bus (`src/core/event_bus/`)

A typed pub/sub event bus for **decoupled cross-module communication** plus a **native, in-process typed request/response** surface. Both are singletons — one instance each for the whole application. Do **not** construct `EventBus` or `NativeRegistry` directly; use the module-level functions.

**When to use which surface:**

- **Broadcast events** (`publish_global` / `subscribe_global`) — fire-and-forget notification. One publisher, many subscribers, no return value. Use when a module needs to _announce_ something happened and other modules may react independently.
- **Native request/response** (`register_native_global` / `request_native_global`) — one-to-one typed Rust dispatch keyed by a method string. **Zero serialization**: trait objects (`Arc<dyn Provider>`), streaming channels (`mpsc::Sender<T>`), oneshot senders, and anything else `Send + 'static` all pass through unchanged. Use when a module needs a typed return value from another module in-process. This is **internal-only** — anything that needs to be callable over JSON-RPC should register against `src/core/all.rs` instead.

**Core types** (all in `src/core/event_bus/`):

| Type | File | Purpose |
|------|------|---------|
| `DomainEvent` | `events.rs` | `#[non_exhaustive]` enum — all cross-module events live here, grouped by domain |
| `EventBus` | `bus.rs` | Singleton backed by `tokio::sync::broadcast`. Construction is `pub(crate)` — tests only |
| `NativeRegistry` / `NativeRequestError` | `native_request.rs` | In-process typed request/response registry keyed by method name. Rust types only — passes trait objects, `mpsc::Sender`, and `oneshot::Sender` through without serialization |
| `EventHandler` | `subscriber.rs` | Async trait with optional `domains()` filter for selective subscription |
| `SubscriptionHandle` | `subscriber.rs` | RAII handle — subscriber task is cancelled on drop |
| `TracingSubscriber` | `tracing.rs` | Built-in debug logger for all events (registered at startup) |

**Singleton API** (all modules use these — never hold or pass `EventBus` / `NativeRegistry` instances):

| Function | Purpose |
|----------|---------|
| `event_bus::init_global(capacity)` | Initialize both singletons (broadcast bus + native registry) at startup (once) |
| `event_bus::publish_global(event)` | Publish a broadcast event from anywhere (no-op if not yet initialized) |
| `event_bus::subscribe_global(handler)` | Subscribe to broadcast events from anywhere (returns `None` if not yet initialized) |
| `event_bus::register_native_global(method, handler)` | Register a typed native request handler for a method name — called at startup by each domain's `bus.rs` |
| `event_bus::request_native_global(method, req)` | Dispatch a typed native request to the registered handler — zero serialization |
| `event_bus::global()` / `event_bus::native_registry()` | Get the underlying singleton for advanced use |

**Domains:** `agent`, `memory`, `channel`, `cron`, `skill`, `tool`, `webhook`, `system`. See `events.rs` for the full variant list — events carry rich payloads so subscribers have everything they need.

**Domain subscriber files** — each domain owns its `bus.rs` with `EventHandler` impls:
- `cron/bus.rs` — `CronDeliverySubscriber` (delivers job output to channels)
- `webhooks/bus.rs` — `WebhookRequestSubscriber` (routes incoming requests to skills, emits responses via socket)
- `channels/bus.rs` — `ChannelInboundSubscriber` (runs agent loop for inbound socket messages)
- `skills/bus.rs` — stub for future subscribers

**Adding events for a new domain:**

1. Add variants to `DomainEvent` in `events.rs` (prefix with domain name, e.g. `BillingInvoiceCreated { ... }`).
2. Add the domain string to the `domain()` match arm.
3. Create a `bus.rs` file **inside your domain module** (e.g. `src/openhuman/billing/bus.rs`) for subscriber implementations — each domain owns its handlers.
4. Register subscribers in startup (e.g. `channels/runtime/startup.rs`) via the singleton.
5. Publish events with `event_bus::publish_global(DomainEvent::YourEvent { ... })`.

**Example — publishing:**
```rust
use crate::core::event_bus::{publish_global, DomainEvent};

publish_global(DomainEvent::CronDeliveryRequested {
    job_id: job.id.clone(),
    channel: "telegram".into(),
    target: "chat-123".into(),
    output: "Job completed".into(),
});
```

**Example — subscribing (trait-based, in `<domain>/bus.rs`):**
```rust
use crate::core::event_bus::{DomainEvent, EventHandler};
use async_trait::async_trait;

pub struct MyDomainSubscriber { /* dependencies */ }

#[async_trait]
impl EventHandler for MyDomainSubscriber {
    fn name(&self) -> &str { "my_domain::handler" }
    fn domains(&self) -> Option<&[&str]> { Some(&["cron"]) } // filter by domain
    async fn handle(&self, event: &DomainEvent) {
        if let DomainEvent::CronJobCompleted { job_id, success } = event {
            // react to the event
        }
    }
}
```

**Convention:** Name the handler struct `<Purpose>Subscriber` (e.g. `CronDeliverySubscriber`) and the `name()` return value `"<domain>::<purpose>"` for grep-friendly tracing output.

**Adding a native request handler for a new domain:**

1. Define the **request and response types** in the domain (e.g. `src/openhuman/billing/bus.rs`). Use owned fields, `Arc`s, and channels — not borrows. Types only need `Send + 'static`, not `Serialize`.
2. Register the handler at startup from the same `bus.rs`, keyed by a stable method name prefixed with the domain (e.g. `"billing.charge_invoice"`).
3. Callers import the request/response types from the domain's public surface and dispatch via `request_native_global`.
4. Method name convention: `"<domain>.<verb>"` — same naming scheme as JSON-RPC method roots for consistency, but these are **not** exposed over JSON-RPC.

**Example — native request (typed request/response, in `<domain>/bus.rs`):**
```rust
use crate::core::event_bus::{register_native_global, request_native_global};
use std::sync::Arc;
use tokio::sync::mpsc;

// Request carries non-serializable state directly — trait objects and
// streaming channels all pass through unchanged.
pub struct BillingChargeRequest {
    pub provider: Arc<dyn BillingProvider>,
    pub amount_cents: u64,
    pub progress_tx: Option<mpsc::Sender<String>>,
}
pub struct BillingChargeResponse {
    pub charge_id: String,
}

// At startup:
pub async fn register_billing_handlers() {
    register_native_global::<BillingChargeRequest, BillingChargeResponse, _, _>(
        "billing.charge",
        |req| async move {
            let id = req.provider.charge(req.amount_cents).await
                .map_err(|e| e.to_string())?;
            Ok(BillingChargeResponse { charge_id: id })
        },
    ).await;
}

// From another module:
let resp: BillingChargeResponse = request_native_global(
    "billing.charge",
    BillingChargeRequest { provider, amount_cents: 500, progress_tx: None },
).await?;
```

**Tests:** override production handlers by calling `register_native_global` again for the same method before exercising the code under test — the most recent registration wins. For full isolation, construct a fresh `NativeRegistry` directly via `NativeRegistry::new()` and use its `register` / `request` methods.
