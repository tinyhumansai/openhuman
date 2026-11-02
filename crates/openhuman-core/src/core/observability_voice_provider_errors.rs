// ── Voice-provider (ElevenLabs) account / key misconfiguration ─────────────
//
// Two different owners hide behind the same ElevenLabs wording:
//
// - **BYOK** (the external STT/TTS providers, called with the user's own
//   ElevenLabs key): `payment_required` is the user's own subscription and
//   "API key ID used as API key" (TAURI-RUST-1635) is a pasted key ID. Both
//   are user state: mapped to an actionable message and demoted from Sentry.
// - **Hosted** (`/openai/v1/audio/*` on the TinyHumans backend, which always
//   calls ElevenLabs with the server's own key and wraps any upstream failure
//   as `ElevenLabs request failed (<status>): <body>`): a `payment_required`
//   there (TAURI-RUST-15BM / -15CT) is TinyHumans' own account, which the user
//   cannot fix. It stays a reported error, and the user is only told the
//   service is unavailable.

/// User-facing message for an ElevenLabs `payment_required` rejection of the
/// user's own (BYOK) key.
pub const VOICE_PROVIDER_PAYMENT_REQUIRED_USER_MESSAGE: &str =
    "Your ElevenLabs subscription has a payment issue. Update billing to continue.";

/// User-facing message for an ElevenLabs key-ID-instead-of-key rejection.
pub const VOICE_PROVIDER_KEY_ID_USER_MESSAGE: &str =
    "That looks like an ElevenLabs key ID, not a key. Keys start with `sk_`. Re-enter the API key.";

/// User-facing message for a hosted voice request the backend's own upstream
/// provider refused. Nothing the user can change, so no instruction.
pub const HOSTED_VOICE_UNAVAILABLE_USER_MESSAGE: &str =
    "The voice service is temporarily unavailable. Please try again later.";

/// The TinyHumans backend's wrappings of an upstream ElevenLabs failure
/// (`elevenlabsService` in the backend: TTS, streaming TTS, transcription).
const HOSTED_ELEVENLABS_FAILURE_MARKERS: &[&str] = &[
    "elevenlabs request failed (",
    "elevenlabs streaming tts failed (",
    "elevenlabs transcription failed (",
];

/// The BYOK ElevenLabs client's error prefixes (`tinyinference_voice`
/// `external_stt` / `external_tts`, ElevenLabs API style). The OpenAI-compatible
/// and Deepgram styles use different prefixes, so matching on these keeps the
/// ElevenLabs-specific messages off the other providers.
const BYOK_ELEVENLABS_ERROR_PREFIXES: &[&str] = &[
    "[voice-stt] elevenlabs error ",
    "[voice-tts] elevenlabs error ",
];

/// A BYOK voice-provider condition the user can fix.
#[derive(Debug, Clone, Copy)]
enum ByokCondition {
    KeyId,
    PaymentRequired,
}

/// Classify `lower` (ASCII-lowercased) as a BYOK ElevenLabs user-config error.
fn byok_condition(lower: &str) -> Option<ByokCondition> {
    if is_hosted_elevenlabs_failure(lower)
        || !BYOK_ELEVENLABS_ERROR_PREFIXES
            .iter()
            .any(|prefix| lower.contains(prefix))
    {
        return None;
    }
    if lower.contains("api key id used as api key") {
        Some(ByokCondition::KeyId)
    } else if lower.contains("payment_required") || lower.contains("failed or incomplete payment") {
        Some(ByokCondition::PaymentRequired)
    } else {
        None
    }
}

/// Whether `lower` (ASCII-lowercased) is a hosted request refused on the
/// backend's own ElevenLabs account.
fn is_hosted_elevenlabs_failure(lower: &str) -> bool {
    HOSTED_ELEVENLABS_FAILURE_MARKERS
        .iter()
        .any(|marker| lower.contains(marker))
}

/// Map a raw BYOK ElevenLabs error to an actionable user-facing message, or
/// `None` when the error is not one of the known user-config conditions.
/// Hosted failures and other providers are never matched: see the module
/// comment and [`BYOK_ELEVENLABS_ERROR_PREFIXES`].
pub fn voice_provider_user_message(message: &str) -> Option<&'static str> {
    match byok_condition(&message.to_ascii_lowercase())? {
        ByokCondition::KeyId => Some(VOICE_PROVIDER_KEY_ID_USER_MESSAGE),
        ByokCondition::PaymentRequired => Some(VOICE_PROVIDER_PAYMENT_REQUIRED_USER_MESSAGE),
    }
}

/// Whether `message` is a raw BYOK voice-provider config error or one of our
/// own mapped BYOK user messages (the latter so the RPC-boundary re-report of
/// the already-mapped string is demoted too).
pub fn is_voice_provider_user_state_message(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    voice_provider_user_message(&lower).is_some()
        || lower.contains(&VOICE_PROVIDER_PAYMENT_REQUIRED_USER_MESSAGE.to_ascii_lowercase())
        || lower.contains(&VOICE_PROVIDER_KEY_ID_USER_MESSAGE.to_ascii_lowercase())
}

/// Replace a known BYOK voice-provider error with its user-facing message,
/// logging only the condition at `warn` (the raw error carries the upstream
/// response body). Unknown errors pass through unchanged.
pub fn map_voice_provider_error(raw: String) -> String {
    match voice_provider_user_message(&raw) {
        Some(user) => {
            let condition = byok_condition(&raw.to_ascii_lowercase());
            tracing::warn!(
                "[voice-provider] ElevenLabs account/key problem mapped to user message: {condition:?}"
            );
            user.to_string()
        }
        None => raw,
    }
}

/// Replace a hosted voice error caused by the backend's upstream provider with
/// [`HOSTED_VOICE_UNAVAILABLE_USER_MESSAGE`]. Only for call sites whose raw
/// failure has already been reported (`authed_json` reports every non-2xx it
/// does not classify), so only a fixed category is logged here. Other errors
/// pass through unchanged.
pub fn map_hosted_voice_error(raw: String) -> String {
    if is_hosted_elevenlabs_failure(&raw.to_ascii_lowercase()) {
        tracing::warn!("[voice-provider] hosted upstream provider failure mapped to user message");
        HOSTED_VOICE_UNAVAILABLE_USER_MESSAGE.to_string()
    } else {
        raw
    }
}
