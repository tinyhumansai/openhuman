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
fn normalize_accepts_seconds_keys() {
    // The cloud backend ships per-frame timing as seconds (`startSeconds`/
    // `endSeconds`); the parser must convert to ms rather than dropping it
    // (which collapsed every frame to start=0/end=80 and froze the mouth).
    let raw = json!({
        "audio_base64": "AAA=",
        "visemes": [
            { "viseme": "sil", "startSeconds": 0.0, "endSeconds": 0.12 },
            { "viseme": "aa", "startSeconds": 0.12, "endSeconds": 0.45 },
            // A gap before the next cue (0.45 → 0.90) is a real pause: the
            // mouth rests there. Preserved because we keep the true ends.
            { "viseme": "PP", "startSeconds": 0.9, "endSeconds": 1.05 },
        ],
        "alignment": [
            { "char": "h", "startSeconds": 0.0, "endSeconds": 0.05 },
        ],
    });
    let r = normalize_response(&raw);
    assert_eq!(r.visemes.len(), 3);
    assert_eq!(
        r.visemes[0],
        VisemeFrame {
            viseme: "sil".into(),
            start_ms: 0,
            end_ms: 120
        }
    );
    assert_eq!(
        r.visemes[1],
        VisemeFrame {
            viseme: "aa".into(),
            start_ms: 120,
            end_ms: 450
        }
    );
    assert_eq!(
        r.visemes[2],
        VisemeFrame {
            viseme: "PP".into(),
            start_ms: 900,
            end_ms: 1050
        }
    );
    let alignment = r.alignment.expect("alignment present");
    assert_eq!(alignment.len(), 1);
    assert_eq!(alignment[0].start_ms, 0);
    assert_eq!(alignment[0].end_ms, 50);
}

#[test]
fn normalize_accepts_short_seconds_duration_aliases() {
    let raw = json!({
        "audio_base64": "AAA=",
        "visemes": [
            { "viseme": "aa", "startSec": 1.2, "durationSec": 0.15 },
        ],
    });
    let r = normalize_response(&raw);
    assert_eq!(
        r.visemes,
        vec![VisemeFrame {
            viseme: "aa".into(),
            start_ms: 1200,
            end_ms: 1350
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
        anyhow::anyhow!("connect timeout").context("backend request POST /openai/v1/audio/speech"),
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
