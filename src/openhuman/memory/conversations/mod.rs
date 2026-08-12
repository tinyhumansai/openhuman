//! Host layer over [`tinymemory_core::conversations`].
//!
//! The domain lives in the extracted crate; the `core::bus` subscribers stay
//! here — they name `DomainEvent` and `BUS`, which are OpenHuman vocabulary.
//! The glob re-export keeps every historical `memory::conversations::…` path resolving.

pub use tinymemory_core::conversations::*;

mod bus;

pub use bus::register_conversation_persistence_subscriber;
