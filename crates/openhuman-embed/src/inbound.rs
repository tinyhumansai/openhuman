//! Per-conversation inbound debounce for hosts that run their own message loop.
//!
//! [`KeyedDebouncer`] is the state machine OpenHuman's channel dispatch uses:
//! messages pushed under one key within the quiet window become one batch,
//! a key never has two batches running, and items that arrive while a batch
//! runs are collected into one follow-up. It owns no timer; the host passes
//! the current [`tokio::time::Instant`] and sleeps until
//! [`KeyedDebouncer::next_wakeup`].
//!
//! ```
//! use openhuman_embed::inbound::{DebounceItem, InboundDebounceConfig, KeyedDebouncer, Push};
//! use std::time::Duration;
//! use tokio::time::Instant;
//!
//! struct Text(&'static str);
//!
//! impl DebounceItem for Text {
//!     fn chars(&self) -> usize { self.0.len() }
//!     fn carries_media(&self) -> bool { false }
//!     fn mergeable(&self) -> bool { !self.0.starts_with('/') }
//! }
//!
//! let settings = InboundDebounceConfig::default().settings_for("whatsapp");
//! let mut debouncer = KeyedDebouncer::new();
//! let t0 = Instant::now();
//! assert_eq!(debouncer.push("chat-1", Text("hi"), &settings, t0), Push::Opened);
//! debouncer.push("chat-1", Text("are you there?"), &settings, t0 + Duration::from_millis(400));
//!
//! let due = debouncer.next_wakeup().expect("a quiet window is open");
//! let ready = debouncer.poll_ready(due);
//! assert_eq!(ready.len(), 1);
//! assert_eq!(ready[0].1.len(), 2);
//! debouncer.complete(&"chat-1");
//! assert!(debouncer.is_empty());
//! ```

pub use openhuman_core::config::{InboundDebounceConfig, InboundDebounceOverride};
pub use openhuman_core::inbound_debounce::{
    DebounceItem, DebounceSettings, KeyedDebouncer, Push, MAX_TRACKED_KEYS,
};

#[cfg(test)]
#[path = "inbound_tests.rs"]
mod tests;
