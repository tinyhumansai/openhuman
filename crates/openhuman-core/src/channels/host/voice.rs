//! Voice host ports for the TinyChannels module: scoped speech-to-text,
//! text-to-speech and the per-turn spoken-reply policy.
//!
//! The module calls these over the `ChannelsHost` callback object. Everything
//! here logs request ids, sizes and durations; transcripts, reply text and
//! audio are never logged.

use crate::config::schema::{ChannelVoiceConfig, VoiceReplyMode};
use crate::config::Config;
use std::sync::Arc;
use std::time::Instant;
use tinychannels_bus::{
    HostSpeechSynthesisRequest, HostSpeechSynthesisResult, HostTranscriptionRequest,
    HostTranscriptionResult, HostVoiceReplyMode, HostVoiceReplyPolicy, HostVoiceReplyPolicyRequest,
    HostVoiceScope, HostVoiceTranscriptionRequest,
};

const LOG_PREFIX: &str = "[channels_voice]";
/// Conservative speaking rate used to bound synthesis before the backend call.
const SPOKEN_CHARS_PER_SEC: usize = 15;

fn thread_key(scope: &HostVoiceScope) -> &str {
    scope
        .conversation
        .thread_id
        .as_deref()
        .filter(|id| !id.is_empty())
        .unwrap_or(&scope.conversation.id)
}

fn has_authenticated_channel_identity(scope: &HostVoiceScope) -> bool {
    !scope.configured_instance.trim().is_empty()
        && scope.channel.id == tinychannels_bus::provider_id(&scope.configured_instance)
        && scope.channel.account_id.as_deref() == Some(scope.configured_instance.as_str())
}

fn wire_mode(mode: VoiceReplyMode) -> HostVoiceReplyMode {
    match mode {
        VoiceReplyMode::Mirror => HostVoiceReplyMode::Mirror,
        VoiceReplyMode::Always => HostVoiceReplyMode::Always,
        VoiceReplyMode::Never => HostVoiceReplyMode::Never,
        VoiceReplyMode::SpeakReply => HostVoiceReplyMode::SpeakReply,
    }
}

/// Resolve the spoken-reply policy for one scoped turn. The module supplies
/// the inbound facts (`inbound_has_voice`, `explicitly_requested`) with the
/// request and applies them to the returned mode; the host owns only the
/// configured mode.
pub(crate) fn resolve_policy(
    voice: &ChannelVoiceConfig,
    request: &HostVoiceReplyPolicyRequest,
) -> HostVoiceReplyPolicy {
    let scope = &request.scope;
    let mode = voice.resolve_mode(&scope.configured_instance, thread_key(scope));
    HostVoiceReplyPolicy {
        mode: wire_mode(mode),
    }
}

fn validate_scope(scope: &HostVoiceScope) -> anyhow::Result<()> {
    if !has_authenticated_channel_identity(scope)
        || scope.conversation.id.trim().is_empty()
        || scope.turn_id.trim().is_empty()
    {
        anyhow::bail!("voice scope is incomplete");
    }
    Ok(())
}

/// Reject replies that cannot fit the channel's outbound duration cap.
pub(crate) fn check_outbound_length(
    voice: &ChannelVoiceConfig,
    channel: &str,
    text: &str,
) -> anyhow::Result<()> {
    let max_secs = voice.resolve_max_outbound_secs(channel) as usize;
    if text.chars().count() > max_secs.saturating_mul(SPOKEN_CHARS_PER_SEC) {
        anyhow::bail!("reply exceeds the maximum spoken duration ({max_secs}s)");
    }
    Ok(())
}

/// Speech-to-text and text-to-speech over OpenHuman's voice domain.
pub(crate) struct ChannelVoiceHost {
    pub config: Arc<Config>,
}

impl ChannelVoiceHost {
    /// Unscoped transcription, honouring the user's local/cloud STT setting.
    pub(crate) async fn transcribe_host(
        &self,
        request: HostTranscriptionRequest,
    ) -> anyhow::Result<HostTranscriptionResult> {
        let request_id = uuid::Uuid::new_v4();
        let started = Instant::now();
        let provider = crate::voice::effective_stt_provider(&self.config);
        tracing::debug!(
            "{LOG_PREFIX} stt start request_id={request_id} provider={provider} bytes_b64={}",
            request.audio_base64.len()
        );
        let stt = crate::voice::create_stt_provider(&provider, "", &self.config)?;
        let outcome = stt
            .transcribe(
                &self.config,
                &request.audio_base64,
                request.mime_type.as_deref(),
                request.file_name.as_deref(),
                request.language.as_deref(),
            )
            .await
            .map_err(|e| {
                tracing::warn!(
                    "{LOG_PREFIX} stt failed request_id={request_id} elapsed_ms={}",
                    started.elapsed().as_millis()
                );
                anyhow::anyhow!(e)
            })?;
        tracing::debug!(
            "{LOG_PREFIX} stt done request_id={request_id} elapsed_ms={} chars={}",
            started.elapsed().as_millis(),
            outcome.value.text.chars().count()
        );
        Ok(HostTranscriptionResult {
            text: outcome.value.text,
            language: request.language,
            duration_secs: None,
        })
    }

    /// Scoped transcription used by the shared voice pipeline.
    pub(crate) async fn transcribe_voice(
        &self,
        request: HostVoiceTranscriptionRequest,
    ) -> anyhow::Result<HostTranscriptionResult> {
        validate_scope(&request.scope)?;
        self.transcribe_host(HostTranscriptionRequest {
            audio_base64: request.audio_base64,
            mime_type: request.mime_type,
            file_name: request.file_name,
            language: request.language,
            scope: Some(request.scope),
        })
        .await
    }

    /// Scoped synthesis through the hosted reply-speech endpoint.
    pub(crate) async fn synthesize_speech(
        &self,
        request: HostSpeechSynthesisRequest,
    ) -> anyhow::Result<HostSpeechSynthesisResult> {
        validate_scope(&request.scope)?;
        let channel = request.scope.configured_instance.as_str();
        let voice_cfg = &self.config.channel_voice;
        check_outbound_length(voice_cfg, channel, &request.text)?;
        crate::modules::ops::ensure_loaded(&self.config, "tinyvoice")
            .await
            .map_err(anyhow::Error::msg)?;
        let request_id = uuid::Uuid::new_v4();
        let started = Instant::now();
        let voice = request
            .voice
            .clone()
            .or_else(|| voice_cfg.resolve_voice(channel).map(str::to_owned));
        tracing::debug!(
            "{LOG_PREFIX} tts start request_id={request_id} channel={channel} chars={}",
            request.text.chars().count()
        );
        let outcome = crate::voice::reply_synthesis::synthesize_reply(
            &self.config,
            &request.text,
            voice.as_deref(),
        )
        .await
        .map_err(|e| {
            tracing::warn!(
                "{LOG_PREFIX} tts failed request_id={request_id} elapsed_ms={}",
                started.elapsed().as_millis()
            );
            anyhow::anyhow!(e)
        })?;
        let speech = outcome.value;
        if speech.audio_base64.is_empty() {
            anyhow::bail!("speech synthesis returned no audio");
        }
        tracing::debug!(
            "{LOG_PREFIX} tts done request_id={request_id} elapsed_ms={} bytes_b64={}",
            started.elapsed().as_millis(),
            speech.audio_base64.len()
        );
        Ok(HostSpeechSynthesisResult {
            audio_base64: speech.audio_base64,
            mime_type: speech.audio_mime,
            duration_ms: None,
            visemes: None,
        })
    }

    /// Host-authoritative reply policy for the scoped turn.
    pub(crate) fn reply_policy(
        &self,
        request: &HostVoiceReplyPolicyRequest,
    ) -> HostVoiceReplyPolicy {
        let policy = resolve_policy(&self.config.channel_voice, request);
        tracing::debug!(
            "{LOG_PREFIX} policy channel={} mode={:?} inbound_voice={} explicit={}",
            request.scope.configured_instance,
            policy.mode,
            request.inbound_has_voice,
            request.explicitly_requested
        );
        policy
    }
}

#[cfg(test)]
#[path = "voice_tests.rs"]
mod tests;
