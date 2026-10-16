//! Reply-speech synthesis — proxies the hosted backend's
//! `/openai/v1/audio/speech` endpoint (ElevenLabs under the hood) so the
//! desktop UI does not have to talk to it directly. Returns base64-encoded
//! audio + an Oculus-15 viseme alignment timeline the mascot uses for
//! lip-sync.
//!
//! Lives in the voice domain because the response is consumed by the
//! mascot's lipsync pipeline (`useHumanMascot` → `findActiveFrame` →
//! `oculusVisemeToShape`).
//!
//! Approval gate (#1339) classification: **internal**. Reply-speech is
//! the user's own assistant speaking through the user's own speakers
//! — there is no outbound side effect visible to a third party.
//! Coordinate with #1206 voice work: if `reply_speech` is ever wrapped
//! in a `Tool` impl, the `external_effect()` method MUST stay `false`
//! (the trait's default) so the approval gate never prompts on TTS.

use log::{debug, warn};
use reqwest::Method;
use serde_json::{json, Value};

use openhuman_embed::__host::config::Config;
use openhuman_embed::__host::core::Outcome;
use openhuman_embed::__host::security::credentials::session_support::resolve_backend_credential;

use crate::client::{flatten_authed_error, BackendClient};

const LOG_PREFIX: &str = "[voice_reply]";

/// The route this module posts to.
const SPEECH_PATH: &str = "/openai/v1/audio/speech";

/// Env var that activates the [`test_seam`] short-circuit at runtime. When
/// set to `1` / `true`, [`synthesize_reply`] records the requested text
/// into [`test_seam::OBSERVED_CALLS`] and returns a stub
/// [`ReplySpeechResult`] *without* contacting the hosted backend. Anything
/// else (unset, `0`, `false`, …) leaves the production code path
/// untouched.
///
/// The env-var gate (rather than a `#[cfg(test)]` gate) is deliberate:
/// integration tests in `tests/` are compiled against the production
/// `openhuman_core` crate, so a unit-only `cfg(test)` block would not be
/// visible from there. The observer module itself is always compiled,
/// but its only producer is this env-gated branch and its only consumer
/// is the test harness, so production callers never touch it.
pub const TEST_SEAM_ENV: &str = "OPENHUMAN_TEST_REPLY_SPEECH_SEAM";

fn test_seam_enabled() -> bool {
    matches!(
        std::env::var(TEST_SEAM_ENV).ok().as_deref(),
        Some("1") | Some("true") | Some("TRUE")
    )
}

/// Test seam observation log. See [`TEST_SEAM_ENV`] for the activation
/// gate. Always compiled (the visibility lets `tests/json_rpc_e2e.rs`
/// inspect calls), but only written to when the env gate is on.
pub mod test_seam {
    use std::sync::Mutex;

    /// FIFO log of every `text` argument that flowed through the test-seam
    /// short-circuit in [`super::synthesize_reply`]. Cleared between tests
    /// with [`clear`].
    pub static OBSERVED_CALLS: Mutex<Vec<String>> = Mutex::new(Vec::new());

    /// Clear the observation log.
    pub fn clear() {
        OBSERVED_CALLS.lock().unwrap().clear();
    }

    /// Snapshot of the observation log.
    pub fn observed() -> Vec<String> {
        OBSERVED_CALLS.lock().unwrap().clone()
    }
}

// The response types and the tolerant-shape parsers live in
// `tinyinference_voice::reply`; the UI contract is unchanged.
use tinyinference_voice::reply::{normalize_response, ReplySpeech as ReplySpeechResult};

/// Caller-tunable knobs.
#[derive(Debug, Default, Clone)]
pub struct ReplySpeechOptions {
    pub voice_id: Option<String>,
    pub model_id: Option<String>,
    pub output_format: Option<String>,
    /// ElevenLabs `voice_settings` blob — passed through verbatim.
    /// Typical fields: `stability`, `similarity_boost`, `style`,
    /// `use_speaker_boost`. The backend forwards this to ElevenLabs;
    /// unknown keys are dropped server-side.
    pub voice_settings: Option<Value>,
}

/// Synthesize the agent's reply through the hosted backend.
///
/// Uses [`BackendClient`] for the same reason `referral` does: the
/// desktop WebView's `fetch` to the backend can fail with an opaque
/// "Load failed" (CORS/TLS quirks), and routing through the core gives us
/// a consistent auth + retry surface.
pub async fn synthesize_reply(
    config: &Config,
    text: &str,
    opts: &ReplySpeechOptions,
) -> Result<Outcome<ReplySpeechResult>, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("text is required".to_string());
    }

    // Test seam: when OPENHUMAN_TEST_REPLY_SPEECH_SEAM is set (and only in
    // debug builds — the seam is structurally dead in release), record the
    // call and short-circuit before hitting the backend.
    // See `test_seam` module docs and `TEST_SEAM_ENV` for the activation gate.
    if cfg!(debug_assertions) && test_seam_enabled() {
        warn!(
            "[voice_reply] TEST SEAM ACTIVE — synthesize_reply short-circuited ({} is set); skipping backend call",
            TEST_SEAM_ENV
        );
        let _ = (config, opts);
        test_seam::OBSERVED_CALLS
            .lock()
            .unwrap()
            .push(trimmed.to_string());
        return Ok(Outcome::single_log(
            ReplySpeechResult {
                audio_base64: String::new(),
                audio_mime: "audio/mpeg".to_string(),
                visemes: Vec::new(),
                alignment: None,
            },
            "voice reply synthesized (test seam short-circuit)",
        ));
    }

    // API key (sent as `x-api-key`) or live session JWT (sent as Bearer).
    let credential = resolve_backend_credential(config)?;
    let client = BackendClient::from_config(config).map_err(|e| e.to_string())?;

    let mut body = serde_json::Map::new();
    body.insert("text".to_string(), json!(trimmed));
    body.insert("with_visemes".to_string(), json!(true));
    if let Some(v) = opts
        .voice_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        body.insert("voice_id".to_string(), json!(v));
    }
    if let Some(v) = opts
        .model_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        body.insert("model_id".to_string(), json!(v));
    }
    if let Some(v) = opts
        .output_format
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        body.insert("output_format".to_string(), json!(v));
    }
    if let Some(settings) = opts.voice_settings.as_ref() {
        if !settings.is_null() {
            body.insert("voice_settings".to_string(), settings.clone());
        }
    }

    debug!(
        "{LOG_PREFIX} synthesize chars={} voice={}",
        trimmed.len(),
        opts.voice_id.as_deref().unwrap_or("default")
    );

    // `flatten_authed_error` maps the typed `BackendApiError::Unauthorized`
    // (expected session-lapse 401 from `authed_json`) onto the `SESSION_EXPIRED`
    // sentinel so the JSON-RPC layer (`core/jsonrpc.rs::is_session_expired_error`)
    // classifies it as session expiry and skips Sentry, matching the #3384
    // team/billing pattern. The previous `e.to_string()` produced the raw
    // "backend rejected session token on POST /openai/v1/audio/speech" Display
    // string, which matched none of the session-expiry classifiers and leaked
    // every lapsed-session TTS 401 to Sentry (TAURI-RUST-8X1). Every other error
    // keeps its full `{e:#}` anyhow chain so genuine TTS failures still report.
    let raw = client
        .authed_json(
            &credential,
            Method::POST,
            SPEECH_PATH,
            Some(Value::Object(body)),
        )
        .await
        .map_err(flatten_authed_error)?;

    let result = normalize_response(&raw);
    debug!(
        "{LOG_PREFIX} synthesized audio_bytes={} visemes={} alignment={}",
        result.audio_base64.len(),
        result.visemes.len(),
        result.alignment.as_ref().map_or(0, Vec::len)
    );

    Ok(Outcome::single_log(
        result,
        "voice reply synthesized via POST /openai/v1/audio/speech",
    ))
}

#[cfg(test)]
#[path = "reply_speech_tests.rs"]
mod tests;
