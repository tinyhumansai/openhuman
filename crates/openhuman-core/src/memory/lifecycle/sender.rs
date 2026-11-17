//! Turns an external-channel origin's sender into the observed actor recorded
//! by TinyMemory. Sender normalization is owned by TinyMemory's API.

use tinymemory_api::ObservedActor;

use crate::core::runtime::turn_origin::AgentTurnOrigin;

/// The sender observed in an external-channel user turn, or `None` for any
/// other origin, a missing sender, or a cron replay.
#[must_use]
pub fn channel_actor(origin: &AgentTurnOrigin) -> Option<ObservedActor> {
    let AgentTurnOrigin::ExternalChannel {
        channel,
        sender: Some(sender),
        sender_name,
        message_id,
        ..
    } = origin
    else {
        return None;
    };
    if message_id.starts_with("cron:") {
        return None;
    }
    ObservedActor::for_channel_sender(channel, sender, sender_name.as_deref())
}

#[cfg(test)]
#[path = "sender_tests.rs"]
mod tests;
