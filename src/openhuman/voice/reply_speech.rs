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
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::api::config::effective_backend_api_url;
use crate::api::jwt::get_session_token;
use crate::api::BackendOAuthClient;
use crate::openhuman::config::Config;
use crate::rpc::RpcOutcome;

const LOG_PREFIX: &str = "[voice_reply]";

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
    use once_cell::sync::Lazy;
    use std::sync::Mutex;

    /// FIFO log of every `text` argument that flowed through the test-seam
    /// short-circuit in [`super::synthesize_reply`]. Cleared between tests
    /// with [`clear`].
    pub static OBSERVED_CALLS: Lazy<Mutex<Vec<String>>> = Lazy::new(|| Mutex::new(Vec::new()));

    /// Clear the observation log.
    pub fn clear() {
        OBSERVED_CALLS.lock().unwrap().clear();
    }

    /// Snapshot of the observation log.
    pub fn observed() -> Vec<String> {
        OBSERVED_CALLS.lock().unwrap().clone()
    }
}

/// One frame on the viseme timeline. `viseme` is an Oculus / Microsoft
/// 15-set code (`sil, PP, FF, TH, DD, kk, CH, SS, nn, RR, aa, E, I, O, U`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VisemeFrame {
    pub viseme: String,
    pub start_ms: u64,
    pub end_ms: u64,
}

/// Char-level timing returned by some backends (e.g. ElevenLabs alignment).
/// Not directly rendered, but kept so the UI can derive a fallback timeline
/// when the backend does not ship visemes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AlignmentFrame {
    pub char: String,
    pub start_ms: u64,
    pub end_ms: u64,
}

/// Normalized response handed to the UI — matches the existing TS shape so
/// the frontend swap is a one-line change.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplySpeechResult {
    pub audio_base64: String,
    pub audio_mime: String,
    pub visemes: Vec<VisemeFrame>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alignment: Option<Vec<AlignmentFrame>>,
}

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
/// Uses [`BackendOAuthClient`] for the same reason `referral` does: the
/// desktop WebView's `fetch` to the backend can fail with an opaque
/// "Load failed" (CORS/TLS quirks), and routing through the core gives us
/// a consistent auth + retry surface.
pub async fn synthesize_reply(
    config: &Config,
    text: &str,
    opts: &ReplySpeechOptions,
) -> Result<RpcOutcome<ReplySpeechResult>, String> {
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
        return Ok(RpcOutcome::single_log(
            ReplySpeechResult {
                audio_base64: String::new(),
                audio_mime: "audio/mpeg".to_string(),
                visemes: Vec::new(),
                alignment: None,
            },
            "voice reply synthesized (test seam short-circuit)",
        ));
    }

    let token = get_session_token(config)
        .map_err(|e| e.to_string())?
        .and_then(|t| {
            let s = t.trim().to_string();
            if s.is_empty() {
                None
            } else {
                Some(s)
            }
        })
        .ok_or_else(|| "no backend session token; sign in first".to_string())?;

    let api_url = effective_backend_api_url(&config.api_url);
    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;

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
            &token,
            Method::POST,
            "/openai/v1/audio/speech",
            Some(Value::Object(body)),
        )
        .await
        .map_err(crate::api::flatten_authed_error)?;

    let result = normalize_response(&raw);
    debug!(
        "{LOG_PREFIX} synthesized audio_bytes={} visemes={} alignment={}",
        result.audio_base64.len(),
        result.visemes.len(),
        result.alignment.as_ref().map_or(0, Vec::len)
    );

    Ok(RpcOutcome::single_log(
        result,
        "voice reply synthesized via POST /openai/v1/audio/speech",
    ))
}

/// Translate the backend's tolerant response shape into the UI contract.
/// Accepts `visemes` / `cues` / `viseme_cues`, and per-frame
/// `start_ms`+`end_ms` or `time_ms`+`duration_ms`.
fn normalize_response(raw: &Value) -> ReplySpeechResult {
    let audio_base64 = raw
        .get("audio_base64")
        .or_else(|| raw.get("audio"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let audio_mime = raw
        .get("audio_mime")
        .or_else(|| raw.get("mime"))
        .and_then(Value::as_str)
        .unwrap_or("audio/mpeg")
        .to_string();

    let cues = raw
        .get("visemes")
        .or_else(|| raw.get("cues"))
        .or_else(|| raw.get("viseme_cues"));
    let visemes = cues
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(parse_cue).collect::<Vec<_>>())
        .unwrap_or_default();

    let alignment = raw
        .get("alignment")
        .or_else(|| raw.get("characters"))
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(parse_alignment).collect::<Vec<_>>());

    ReplySpeechResult {
        audio_base64,
        audio_mime,
        visemes,
        alignment,
    }
}

fn parse_cue(v: &Value) -> Option<VisemeFrame> {
    let viseme = v
        .get("viseme")
        .or_else(|| v.get("v"))
        .or_else(|| v.get("code"))
        .and_then(Value::as_str)?
        .to_string();
    if viseme.is_empty() {
        return None;
    }
    let start = read_u64(v, &["start_ms", "time_ms", "t"]).unwrap_or(0);
    let end = read_u64(v, &["end_ms"])
        .or_else(|| {
            let t = read_u64(v, &["time_ms", "t"])?;
            let d = read_u64(v, &["duration_ms", "d"])?;
            Some(t + d)
        })
        .unwrap_or(start + 80);
    if end <= start {
        return None;
    }
    Some(VisemeFrame {
        viseme,
        start_ms: start,
        end_ms: end,
    })
}

fn parse_alignment(v: &Value) -> Option<AlignmentFrame> {
    let ch = v.get("char").and_then(Value::as_str)?.to_string();
    let start = read_u64(v, &["start_ms"])?;
    let end = read_u64(v, &["end_ms"])?;
    if end <= start {
        return None;
    }
    Some(AlignmentFrame {
        char: ch,
        start_ms: start,
        end_ms: end,
    })
}

fn read_u64(v: &Value, keys: &[&str]) -> Option<u64> {
    for k in keys {
        if let Some(n) = v.get(*k).and_then(Value::as_u64) {
            return Some(n);
        }
        if let Some(f) = v.get(*k).and_then(Value::as_f64) {
            if f.is_finite() && f >= 0.0 {
                return Some(f as u64);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn normalize_canonical_shape() {
        let raw = json!({
            "audio_base64": "AAA=",
            "audio_mime": "audio/mpeg",
            "visemes": [
                { "viseme": "sil", "start_ms": 0, "end_ms": 100 },
                { "viseme": "aa", "start_ms": 100, "end_ms": 250 },
            ],
        });
        let r = normalize_response(&raw);
        assert_eq!(r.audio_base64, "AAA=");
        assert_eq!(r.audio_mime, "audio/mpeg");
        assert_eq!(r.visemes.len(), 2);
        assert_eq!(r.visemes[1].viseme, "aa");
        assert_eq!(r.visemes[1].end_ms, 250);
    }

    #[test]
    fn normalize_accepts_cues_and_short_keys() {
        let raw = json!({
            "audio": "BBB=",
            "mime": "audio/wav",
            "cues": [{ "v": "PP", "t": 0, "d": 80 }],
        });
        let r = normalize_response(&raw);
        assert_eq!(r.audio_base64, "BBB=");
        assert_eq!(r.audio_mime, "audio/wav");
        assert_eq!(
            r.visemes,
            vec![VisemeFrame {
                viseme: "PP".into(),
                start_ms: 0,
                end_ms: 80
            }]
        );
    }

    #[test]
    fn normalize_drops_malformed_cues() {
        let raw = json!({
            "audio_base64": "CCC=",
            "visemes": [
                { "viseme": "aa", "start_ms": 0, "end_ms": 100 },
                { "viseme": "",   "start_ms": 100, "end_ms": 200 },
                { "viseme": "PP", "start_ms": 200, "end_ms": 200 },
            ],
        });
        let r = normalize_response(&raw);
        assert_eq!(r.visemes.len(), 1);
        assert_eq!(r.visemes[0].viseme, "aa");
    }

    #[test]
    fn normalize_passes_through_alignment() {
        let raw = json!({
            "audio_base64": "DDD=",
            "alignment": [{ "char": "h", "start_ms": 0, "end_ms": 50 }],
        });
        let r = normalize_response(&raw);
        assert_eq!(r.alignment.as_deref().unwrap()[0].char, "h");
    }

    #[test]
    fn tts_unauthorized_flattens_to_session_expiry_not_hard_error() {
        // TAURI-RUST-8X1: a lapsed-session 401 on the TTS endpoint
        // (`POST /openai/v1/audio/speech`) used to be flattened with
        // `e.to_string()`, producing the raw "backend rejected session token …"
        // Display string that matched none of the session-expiry classifiers and
        // leaked to Sentry as a hard error. `synthesize_reply` now flattens the
        // typed `BackendApiError::Unauthorized` via `crate::api::flatten_authed_error`
        // (the #3384 team/billing pattern), so it carries the SESSION_EXPIRED
        // sentinel and is recognised + demoted by the JSON-RPC dispatcher.
        //
        // This test couples the exact TTS endpoint's typed 401 to the live
        // classifier: build the typed error → flatten → classify. If either the
        // sentinel mapping or the classifier drifts, this fails instead of
        // silently re-leaking the TTS 401.
        let flat = crate::api::flatten_authed_error(anyhow::Error::new(
            crate::api::BackendApiError::Unauthorized {
                method: "POST".to_string(),
                path: "/openai/v1/audio/speech".to_string(),
            },
        ));

        assert!(
            flat.contains("SESSION_EXPIRED"),
            "flattened TTS 401 must carry the sentinel, got: {flat}"
        );
        assert!(
            flat.contains("/openai/v1/audio/speech"),
            "path preserved for logs: {flat}"
        );
        assert!(
            crate::core::observability::is_session_expired_message(&flat),
            "flattened TTS Unauthorized must classify as session expiry (demoted, \
             not a hard error): {flat}"
        );
    }

    #[test]
    fn tts_non_auth_error_is_not_demoted_to_session_expiry() {
        // A genuine TTS failure (timeout, 5xx, …) must keep its full anyhow chain
        // and NOT be demoted — real backend/TTS breakage must still reach Sentry.
        let flat = crate::api::flatten_authed_error(
            anyhow::anyhow!("connect timeout")
                .context("backend request POST /openai/v1/audio/speech"),
        );

        assert!(
            !flat.contains("SESSION_EXPIRED"),
            "non-auth TTS error must not be demoted: {flat}"
        );
        assert!(
            flat.contains("connect timeout"),
            "underlying cause preserved: {flat}"
        );
        assert!(
            !crate::core::observability::is_session_expired_message(&flat),
            "non-auth TTS error must NOT classify as session expiry: {flat}"
        );
    }
}
