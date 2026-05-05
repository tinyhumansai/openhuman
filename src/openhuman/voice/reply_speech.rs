//! Reply-speech synthesis — proxies the hosted backend's
//! `/openai/v1/audio/speech` endpoint (ElevenLabs under the hood) so the
//! desktop UI does not have to talk to it directly. Returns base64-encoded
//! audio + an Oculus-15 viseme alignment timeline the mascot uses for
//! lip-sync.
//!
//! Lives in the voice domain because the response is consumed by the
//! mascot's lipsync pipeline (`useHumanMascot` → `findActiveFrame` →
//! `oculusVisemeToShape`).

use log::debug;
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::api::config::effective_api_url;
use crate::api::jwt::get_session_token;
use crate::api::BackendOAuthClient;
use crate::openhuman::config::Config;
use crate::rpc::RpcOutcome;

const LOG_PREFIX: &str = "[voice_reply]";

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

    let api_url = effective_api_url(&config.api_url);
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

    debug!(
        "{LOG_PREFIX} synthesize chars={} voice={}",
        trimmed.len(),
        opts.voice_id.as_deref().unwrap_or("default")
    );

    let raw = client
        .authed_json(
            &token,
            Method::POST,
            "/openai/v1/audio/speech",
            Some(Value::Object(body)),
        )
        .await
        .map_err(|e| e.to_string())?;

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
}
