//! Cross-module event bus for decoupled events and typed in-process requests.
//!
//! The event bus is a **singleton** — one instance for the entire application.
//! It serves as the central nervous system of OpenHuman, allowing different
//! modules (like memory, skills, and agents) to communicate without
//! direct dependencies.
//!
//! Call [`init_global`] once at startup, then use [`publish_global`],
//! [`subscribe_global`], [`register_native_global`], and
//! [`request_native_global`] from any module.
//!
//! # Two Surfaces
//!
//! 1. **Broadcast Pub/Sub** ([`publish_global`] / [`subscribe_global`])
//!    - Built on `tokio::sync::broadcast`.
//!    - **Many-to-many**: One publisher, zero or more subscribers.
//!    - **Fire-and-forget**: No feedback from subscribers to the publisher.
//!    - **Decoupled**: Use this for notifications like "a message was received"
//!      or "a skill was loaded".
//!
//! 2. **Native Request/Response** ([`register_native_global`] / [`request_native_global`])
//!    - **One-to-one**: Each method name has exactly one registered handler.
//!    - **Typed**: Payloads are Rust types, checked at runtime via `TypeId`.
//!    - **Zero Serialization**: Directly passes pointers, `Arc`s, and channels.
//!    - **Coupled (but in-process)**: Use this for direct module-to-module
//!      calls that need non-serializable data or immediate responses.
//!
//! # Architecture
//!
//! The bus is designed to be initialized early in the application lifecycle.
//! Once [`init_global`] is called, the bus is available globally. This allows
//! modules to register their handlers and subscribers in their own `bus.rs`
//! or `mod.rs` files during startup.
//!
//! # Usage
//!
//! ```ignore
//! use crate::core::event_bus::{
//!     publish_global, register_native_global, request_native_global,
//!     subscribe_global, DomainEvent,
//! };
//!
//! // Example 1: Broadcasting a system event
//! publish_global(DomainEvent::SystemStartup { component: "example".into() });
//!
//! // Example 2: Registering a native request handler
//! register_native_global::<MyReq, MyResp, _, _>("my_domain.do_thing", |req| async move {
//!     // Process request...
//!     Ok(MyResp { /* ... */ })
//! });
//!
//! // Example 3: Dispatching a native request
//! let resp: MyResp = request_native_global("my_domain.do_thing", MyReq { /* ... */ }).await?;
//! ```

mod bus;
mod events;
mod native_request;
mod subscriber;
pub mod testing;
mod tracing;

pub use bus::{global, init_global, publish_global, subscribe_global, EventBus, DEFAULT_CAPACITY};
pub use events::{BackendMeetTurn, DomainEvent, VoiceEvent};
pub use native_request::{
    init_native_registry, native_registry, register_native_global, request_native_global,
    NativeRegistry, NativeRequestError,
};
pub use subscriber::{EventHandler, SubscriptionHandle};
pub use tracing::TracingSubscriber;
