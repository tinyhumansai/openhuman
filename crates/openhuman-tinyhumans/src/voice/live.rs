//! The hosted live voice providers: `gemini-hosted` and `elevenlabs-hosted`.
//!
//! Both are reached through the TinyHumans backend. Minting the Gemini Live
//! relay ticket and the ElevenLabs signed URL happens here, against the
//! backend, because `tinyliveagents` deliberately holds no backend or
//! credential code. They register with the core's live-provider registry
//! ([`register`]); the core carries no hosted entry of its own.

use openhuman_embed::__host::config::Config;
use openhuman_embed::__host::security::credentials::session_support::resolve_backend_credential;
use openhuman_embed::__host::voice_registry::{
    register_live_provider, LiveProviderEntry, LiveProviderKind, LiveProviderPlacement,
    LiveVoiceError, PreparedProvider, GEMINI_LANGUAGES, GEMINI_VOICES,
    LIVE_PROVIDER_ELEVENLABS_HOSTED, LIVE_PROVIDER_GEMINI_HOSTED,
};
use reqwest::Method;
use serde_json::{json, Value};
use std::sync::Arc;
use tinyagents_live::tinyliveagents::elevenlabs::ElevenLabsConvai;
use tinyagents_live::tinyliveagents::gemini::{self, GeminiRelay};
use tinyagents_live::tinyliveagents::LiveConfig;

use super::realtime::{ensure_secure_backend_url, mint_voice_agent_signed_url};
use crate::client::{flatten_authed_error, BackendClient};

const LOG_PREFIX: &str = "[voice-live]";

/// The backend route that mints a Gemini Live relay ticket.
pub(crate) const GEMINI_LIVE_SESSIONS_PATH: &str = "/agent-integrations/gemini/live/sessions";

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_string()).collect()
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|v| !v.trim().is_empty())
}

/// Whether a backend credential is available for the hosted providers.
fn backend_ready(config: &Config) -> bool {
    resolve_backend_credential(config).is_ok()
}

/// Shapes the session configuration for `gemini-hosted` from the user's settings.
pub(crate) fn configure_gemini(config: &Config, system_instruction: &str, live: &mut LiveConfig) {
    let settings = &config.voice_live;
    live.system_instruction = Some(system_instruction.to_string());
    live.model = settings.gemini.model.clone();
    live.voice = settings.gemini.voice.clone();
    live.language = settings.gemini.language.clone();
    // Keep long conversations inside the context window.
    live.provider_options = json!({ "context_window_compression": { "slidingWindow": {} } });
}

/// Shapes the session configuration for `elevenlabs-hosted`.
pub(crate) fn configure_elevenlabs(
    config: &Config,
    _system_instruction: &str,
    live: &mut LiveConfig,
) {
    // The hosted agent owns its prompt and its brain (OpenHuman over the
    // Custom-LLM relay); only the voice may be overridden.
    live.voice = non_empty(config.voice_live.elevenlabs.voice_id.clone());
}

/// A minted relay ticket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GeminiTicket {
    pub(crate) ws_url: String,
    pub(crate) session_id: String,
    pub(crate) model: String,
}

/// Parses the backend's ticket response (`data`-wrapped or bare).
pub(crate) fn parse_ticket(raw: &Value) -> Result<GeminiTicket, LiveVoiceError> {
    let data = raw.get("data").unwrap_or(raw);
    let field = |name: &str| {
        data.get(name)
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
    };
    let ws_url = field("wsUrl").ok_or_else(|| {
        LiveVoiceError::backend("backend returned no wsUrl for the live session".to_string())
    })?;
    Ok(GeminiTicket {
        ws_url,
        session_id: field("sessionId").unwrap_or_default(),
        model: field("model").unwrap_or_default(),
    })
}

async fn mint_gemini_ticket(
    config: &Config,
    live: &LiveConfig,
) -> Result<GeminiTicket, LiveVoiceError> {
    let credential = resolve_backend_credential(config).map_err(LiveVoiceError::not_configured)?;
    let client =
        BackendClient::from_config(config).map_err(|e| LiveVoiceError::backend(e.to_string()))?;
    ensure_secure_backend_url(client.base_url()).map_err(LiveVoiceError::backend)?;
    let body = gemini::ticket_request(live);
    let raw = client
        .authed_json(
            &credential,
            Method::POST,
            GEMINI_LIVE_SESSIONS_PATH,
            Some(body),
        )
        .await
        .map_err(|error| LiveVoiceError::backend(flatten_authed_error(error)))?;
    parse_ticket(&raw)
}

/// Connects `gemini-hosted`: mints a relay ticket and points a relay at it.
pub(crate) async fn prepare_gemini(
    config: Config,
    live: LiveConfig,
) -> Result<PreparedProvider, LiveVoiceError> {
    let ticket = mint_gemini_ticket(&config, &live).await?;
    log::debug!(
        "{LOG_PREFIX} minted gemini live ticket session={} model={}",
        ticket.session_id,
        ticket.model
    );
    let relay =
        GeminiRelay::connect_url(ticket.ws_url).with_session(ticket.session_id, ticket.model);
    Ok(PreparedProvider {
        provider: Box::new(relay),
        config: live,
    })
}

/// Connects `elevenlabs-hosted`: mints a signed agent URL.
pub(crate) async fn prepare_elevenlabs(
    config: Config,
    mut live: LiveConfig,
) -> Result<PreparedProvider, LiveVoiceError> {
    let signed = mint_voice_agent_signed_url(&config)
        .await
        .map_err(LiveVoiceError::backend)?
        .value;
    // `user_token` is the identity the backend's Custom-LLM relay verifies on
    // every relayed turn (#5399).
    live.provider_options = json!({
        "user_id": signed.user_token,
        "custom_llm_extra_body": { "user": signed.user_token },
    });
    Ok(PreparedProvider {
        provider: Box::new(ElevenLabsConvai::connect_url(signed.signed_url)),
        config: live,
    })
}

/// Registers both hosted live providers under their configured ids, listed
/// ahead of the built-in providers as the catalogue has always shown them.
pub(super) fn register() {
    register_live_provider(
        LIVE_PROVIDER_GEMINI_HOSTED,
        LiveProviderEntry {
            label: "Gemini Live (TinyHumans)".to_string(),
            kind: LiveProviderKind::Hosted,
            voices: strings(GEMINI_VOICES),
            languages: strings(GEMINI_LANGUAGES),
            key_slug: None,
            configured: Arc::new(backend_ready),
            configure: Some(Arc::new(configure_gemini)),
            prepare: Arc::new(|config, live| Box::pin(prepare_gemini(config, live))),
            placement: LiveProviderPlacement::BeforeBuiltIn,
        },
    );
    register_live_provider(
        LIVE_PROVIDER_ELEVENLABS_HOSTED,
        LiveProviderEntry {
            label: "ElevenLabs Agent (TinyHumans)".to_string(),
            kind: LiveProviderKind::Hosted,
            voices: Vec::new(),
            languages: Vec::new(),
            key_slug: None,
            configured: Arc::new(backend_ready),
            configure: Some(Arc::new(configure_elevenlabs)),
            prepare: Arc::new(|config, live| Box::pin(prepare_elevenlabs(config, live))),
            placement: LiveProviderPlacement::BeforeBuiltIn,
        },
    );
    log::debug!("{LOG_PREFIX} registered hosted live providers");
}

#[cfg(test)]
#[path = "live_tests.rs"]
mod tests;
