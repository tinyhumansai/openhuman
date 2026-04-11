//! Cross-module event bus for decoupled events and typed in-process requests.
//!
//! The event bus is a **singleton** — one instance for the entire application.
//! Call [`init_global`] once at startup, then use [`publish_global`],
//! [`subscribe_global`], [`register_native_global`], and
//! [`request_native_global`] from any module.
//!
//! # Two surfaces
//!
//! 1. **Broadcast pub/sub** ([`publish_global`] / [`subscribe_global`]) —
//!    fire-and-forget notification of [`DomainEvent`] variants. One publisher,
//!    many subscribers, no back-channel.
//! 2. **Native request/response** ([`register_native_global`] /
//!    [`request_native_global`]) — one-to-one typed Rust dispatch keyed by a
//!    method string. Zero serialization: trait objects, [`std::sync::Arc`]s,
//!    [`tokio::sync::mpsc::Sender`]s, and oneshot channels pass through
//!    unchanged. Use this for in-process module-to-module calls that need
//!    non-serializable payloads (hot-path data, streaming, async resolution).
//!
//! # Usage
//!
//! ```ignore
//! use crate::core::event_bus::{
//!     publish_global, register_native_global, request_native_global,
//!     subscribe_global, DomainEvent,
//! };
//!
//! // Publish a broadcast event
//! publish_global(DomainEvent::SystemStartup { component: "example".into() });
//!
//! // Register a native request handler at startup
//! register_native_global::<MyReq, MyResp, _, _>("my_domain.do_thing", |req| async move {
//!     Ok(MyResp { /* ... */ })
//! }).await;
//!
//! // Dispatch a native request from any module
//! let resp: MyResp = request_native_global("my_domain.do_thing", MyReq { /* ... */ }).await?;
//! ```

mod bus;
mod events;
mod native_request;
mod subscriber;
pub mod testing;
mod tracing;

pub use bus::{global, init_global, publish_global, subscribe_global, EventBus, DEFAULT_CAPACITY};
pub use events::DomainEvent;
pub use native_request::{
    init_native_registry, native_registry, register_native_global, request_native_global,
    NativeRegistry, NativeRequestError,
};
pub use subscriber::{EventHandler, SubscriptionHandle};
pub use tracing::TracingSubscriber;
