//! Cross-module event bus for decoupled pub/sub communication.
//!
//! The event bus is a **singleton** — one instance for the entire application.
//! Call [`init_global`] once at startup, then use [`publish_global`] and
//! [`subscribe_global`] from any module.
//!
//! # Usage
//!
//! ```ignore
//! use crate::openhuman::event_bus::{publish_global, subscribe_global, DomainEvent};
//!
//! // Publish from anywhere
//! publish_global(DomainEvent::SystemStartup { component: "example".into() });
//!
//! // Subscribe from anywhere
//! let _handle = subscribe_global(Arc::new(MyHandler));
//! ```

mod bus;
mod events;
mod subscriber;
mod tracing;

pub use bus::{global, init_global, publish_global, subscribe_global, EventBus};
pub use events::DomainEvent;
pub use subscriber::{EventHandler, SubscriptionHandle};
pub use tracing::TracingSubscriber;
