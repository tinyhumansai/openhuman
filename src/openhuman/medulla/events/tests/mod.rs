//! Unit tests for the event vocabulary, split by the surface under test:
//! [`serde_tests`] covers JSON round-trips and deserialize tolerance;
//! [`derive_tests`] covers the read-only derivations.

use crate::openhuman::medulla::events::{EventEnvelope, SessionEvent};

mod serde_tests;

/// Build an envelope at `seq` with a zero timestamp for concise test setup.
pub(super) fn env(seq: u64, event: SessionEvent) -> EventEnvelope {
    EventEnvelope { seq, at: 0, event }
}
