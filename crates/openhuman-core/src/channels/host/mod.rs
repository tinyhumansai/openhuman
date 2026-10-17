//! OpenHuman host callbacks and event subscribers for the TinyChannels module.
//!
//! Production startup serves the typed callback surface from `runtime::startup`;
//! legacy provider-trait adapters remain compiled only for their unit tests.

mod adapters;
mod channel_events;
pub(crate) mod remote_control;

pub(crate) use adapters::{ConfigAllowlistStore, OpenHumanEventSink, VoiceTranscriber};
#[cfg(test)]
pub use adapters::{
    ConversationHistoryStore, CoreApprovalGate, CoreShutdownRegistry, VoiceSynthesizer,
};
pub use channel_events::{ChannelApprovalSurfaceSubscriber, ChannelTurnStateSubscriber};

#[cfg(test)]
use std::sync::Arc;

#[cfg(test)]
use tinychannels::host::ChannelHostBuilder;
#[cfg(test)]
use tinychannels::ChannelHost;

#[cfg(test)]
use crate::config::Config;

/// Assemble the full OpenHuman [`ChannelHost`] from a config snapshot.
///
/// Wires every capability OpenHuman can back today: lifecycle (shutdown),
/// STT, TTS, approval-reply parsing, conversation history, and the
/// web-channel event sink. Capabilities OpenHuman cannot yet express portably
/// (turn dispatch, run ledger, pairing) are simply left unset — a provider
/// that needs one degrades gracefully. Emoji reactions are deliberately unset:
/// the local-model reaction gate was removed, so providers never react.
#[cfg(test)]
pub fn build_channel_host(config: Arc<Config>) -> Arc<dyn ChannelHost> {
    ChannelHostBuilder::new()
        .lifecycle(Arc::new(CoreShutdownRegistry))
        .transcriber(Arc::new(VoiceTranscriber {
            config: Arc::clone(&config),
        }))
        .synthesizer(Arc::new(VoiceSynthesizer {
            config: Arc::clone(&config),
        }))
        .approvals(Arc::new(CoreApprovalGate))
        .conversations(Arc::new(ConversationHistoryStore {
            workspace_dir: config.workspace_dir.clone(),
        }))
        .events(Arc::new(OpenHumanEventSink))
        .allowlist(Arc::new(ConfigAllowlistStore))
        .build()
}

#[cfg(test)]
#[path = "host_tests.rs"]
mod tests;
