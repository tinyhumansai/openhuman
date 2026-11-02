//! Live voice operations behind the `voice.live_*` controllers.

use std::time::{Duration, Instant};

use tinyagents_live::tinyliveagents::LiveEvent;

use super::error::LiveVoiceError;
use super::providers;
use super::session;
use super::types::{
    GeminiLiveSettingsPatch, LiveProvidersResponse, LiveSettingsPatch, LiveTestResult,
    LiveVoiceConfig, SarvamLiveSettingsPatch,
};
use crate::config::schema::voice_live::is_live_provider;
use crate::config::Config;
use crate::core::Outcome;

/// How long a provider test waits for the session to become ready.
pub(crate) const TEST_TIMEOUT: Duration = Duration::from_secs(20);

/// The provider catalogue and the default provider.
pub async fn live_providers(config: &Config) -> Outcome<LiveProvidersResponse> {
    Outcome::new(
        LiveProvidersResponse {
            default_provider: config.voice_live.default_provider.clone(),
            providers: providers::provider_infos(config).await,
        },
        Vec::new(),
    )
}

/// The stored live voice settings.
pub fn live_settings_get(config: &Config) -> Outcome<LiveVoiceConfig> {
    Outcome::new(config.voice_live.clone(), Vec::new())
}

/// Applies `patch` to `config` (without saving).
pub(crate) fn apply_patch(config: &mut Config, patch: LiveSettingsPatch) -> Result<(), String> {
    if let Some(provider) = patch.default_provider {
        if !is_live_provider(&provider) {
            return Err(format!("unknown live provider `{provider}`"));
        }
        config.voice_live.default_provider = provider;
    }
    if let Some(gemini) = patch.gemini {
        merge_gemini(&mut config.voice_live.gemini, gemini);
    }
    if let Some(sarvam) = patch.sarvam {
        merge_sarvam(&mut config.voice_live.sarvam, sarvam);
    }
    if let Some(elevenlabs) = patch.elevenlabs {
        if let Some(value) = elevenlabs.voice_id {
            config.voice_live.elevenlabs.voice_id = value;
        }
    }
    Ok(())
}

fn merge_gemini(
    settings: &mut crate::config::schema::voice_live::GeminiLiveSettings,
    patch: GeminiLiveSettingsPatch,
) {
    if let Some(value) = patch.model {
        settings.model = value;
    }
    if let Some(value) = patch.voice {
        settings.voice = value;
    }
    if let Some(value) = patch.language {
        settings.language = value;
    }
}

fn merge_sarvam(
    settings: &mut crate::config::schema::voice_live::SarvamLiveSettings,
    patch: SarvamLiveSettingsPatch,
) {
    if let Some(value) = patch.language {
        settings.language = value;
    }
    if let Some(value) = patch.speaker {
        settings.speaker = value;
    }
    if let Some(value) = patch.model {
        settings.model = value;
    }
}

/// Updates and saves the live voice settings.
pub async fn live_settings_set(
    config: &mut Config,
    patch: LiveSettingsPatch,
) -> Result<Outcome<LiveVoiceConfig>, String> {
    apply_patch(config, patch)?;
    config
        .save()
        .await
        .map_err(crate::config::ops::config_save_error)?;
    tracing::debug!(
        default_provider = %config.voice_live.default_provider,
        "[voice-live] settings saved"
    );
    Ok(Outcome::new(config.voice_live.clone(), Vec::new()))
}

/// Opens a session on `provider`, waits for it to be ready, and closes it.
pub async fn live_test_provider(config: &Config, provider: &str) -> Outcome<LiveTestResult> {
    let started = Instant::now();
    let result = test_connect(config, provider).await;
    let value = match result {
        Ok(()) => LiveTestResult {
            ok: true,
            latency_ms: Some(u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)),
            error: None,
        },
        Err(error) => LiveTestResult {
            ok: false,
            latency_ms: None,
            error: Some(error.to_string()),
        },
    };
    tracing::debug!(
        provider,
        ok = value.ok,
        "[voice-live] provider test finished"
    );
    Outcome::new(value, Vec::new())
}

async fn test_connect(config: &Config, provider: &str) -> Result<(), LiveVoiceError> {
    let provider = session::resolve_provider(config, Some(provider))?;
    let live = providers::live_config(config, &provider, "Connection test.", 16_000);
    let prepared = providers::prepare(config, &provider, live).await?;
    let mut live_session = prepared.provider.connect(prepared.config).await?;
    let sender = live_session.sender();
    let outcome = match tokio::time::timeout(TEST_TIMEOUT, live_session.recv()).await {
        Err(_) => Err(LiveVoiceError::new(
            "timeout",
            "the provider did not become ready",
        )),
        Ok(Some(LiveEvent::Ready(_))) => Ok(()),
        Ok(Some(LiveEvent::Error { error, .. })) => Err(error.into()),
        Ok(Some(LiveEvent::Closed(tinyagents_live::tinyliveagents::CloseReason::Error(error)))) => {
            Err(error.into())
        }
        Ok(_) => Err(LiveVoiceError::new(
            "provider",
            "the provider closed before it was ready",
        )),
    };
    let _ = sender.close().await;
    outcome
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
