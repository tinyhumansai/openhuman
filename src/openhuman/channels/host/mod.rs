//! OpenHuman implementation of the `tinychannels::host` capability boundary.
//!
//! [`build_channel_host`] assembles the concrete [`tinychannels::ChannelHost`]
//! from OpenHuman internals (voice, inference, approvals, conversation store,
//! shutdown registry, web event bus). [`build_provider_context`] wraps it into
//! the [`tinychannels::host::ProviderContext`] handed to channel providers.
//!
//! Ported providers reach host capabilities through this context instead of
//! calling OpenHuman internals directly — the inversion that lets them live in
//! the standalone `tinychannels` crate. Lean providers ignore the host.

mod adapters;

pub use adapters::{
    ConfigAllowlistStore, ConversationHistoryStore, CoreApprovalGate, CoreShutdownRegistry,
    InferenceReactionGate, OpenHumanEventSink, VoiceSynthesizer, VoiceTranscriber,
};

use std::sync::Arc;

use tinychannels::host::{ChannelHostBuilder, ProviderContext};
use tinychannels::ChannelHost;

use crate::openhuman::config::Config;

/// Assemble the full OpenHuman [`ChannelHost`] from a config snapshot.
///
/// Wires every capability OpenHuman can back today: lifecycle (shutdown),
/// STT, TTS, reaction gate, approval-reply parsing, conversation history, and
/// the web-channel event sink. Capabilities OpenHuman cannot yet express
/// portably (turn dispatch, run ledger, pairing) are simply left unset — a
/// provider that needs one degrades gracefully.
pub fn build_channel_host(config: Arc<Config>) -> Arc<dyn ChannelHost> {
    ChannelHostBuilder::new()
        .lifecycle(Arc::new(CoreShutdownRegistry))
        .transcriber(Arc::new(VoiceTranscriber {
            config: Arc::clone(&config),
        }))
        .synthesizer(Arc::new(VoiceSynthesizer {
            config: Arc::clone(&config),
        }))
        .reactions(Arc::new(InferenceReactionGate {
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

/// Build the [`ProviderContext`] handed to a channel provider at construction:
/// the assembled host + the channels config + a pre-built HTTP client.
pub fn build_provider_context(config: &Config, http_client: reqwest::Client) -> ProviderContext {
    ProviderContext::new(
        build_channel_host(Arc::new(config.clone())),
        config.channels_config.clone(),
        http_client,
    )
}

#[cfg(test)]
mod tests;
