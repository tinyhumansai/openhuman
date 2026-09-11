//! Voice domain event publishers. Publishing the PTT transcript-committed
//! event here lets downstream subscribers react without coupling to the
//! channel-web flow.

use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use crate::core::events::VoiceEvent;

/// Publish a [`VoiceEvent::PttTranscriptCommitted`] event.
pub fn publish_ptt_transcript_committed(
    thread_id: String,
    session_id: u64,
    text_len: usize,
    held_ms: u64,
    finalized_by_watchdog: bool,
) {
    BUS.publish(DomainEvent::Voice(VoiceEvent::PttTranscriptCommitted {
        thread_id,
        session_id,
        text_len,
        held_ms,
        finalized_by_watchdog,
    }));
}

#[cfg(test)]
#[path = "bus_tests.rs"]
mod tests;
