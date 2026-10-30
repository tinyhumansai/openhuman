// ── Voice-provider (ElevenLabs) account / key misconfiguration ─────────────
//
// TAURI-RUST-15BM / -15CT (ElevenLabs `401 payment_required`) and
// TAURI-RUST-1635 (ElevenLabs `invalid_api_key`, "API key ID used as API key").
// These are third-party account or user-entered-key states, not code defects:
// the upstream account has a failed payment, or the user pasted an API key
// *ID* instead of the key. OpenHuman has no lever to fix either, so they are
// mapped to an actionable user-facing message and demoted out of Sentry.

/// User-facing message for an ElevenLabs `payment_required` rejection.
pub const VOICE_PROVIDER_PAYMENT_REQUIRED_USER_MESSAGE: &str =
    "Your ElevenLabs subscription has a payment issue. Update billing to continue.";

/// User-facing message for an ElevenLabs key-ID-instead-of-key rejection.
pub const VOICE_PROVIDER_KEY_ID_USER_MESSAGE: &str =
    "That looks like an ElevenLabs key ID, not a key. Keys start with `sk_`. Re-enter the API key.";

/// Map a raw voice-provider error to an actionable user-facing message, or
/// `None` when the error is not one of the known upstream-config conditions.
///
/// Matching is anchored on the upstream wording only; response bodies are not
/// assumed to have any other structure.
pub fn voice_provider_user_message(message: &str) -> Option<&'static str> {
    let lower = message.to_ascii_lowercase();
    if lower.contains("api key id used as api key") {
        return Some(VOICE_PROVIDER_KEY_ID_USER_MESSAGE);
    }
    if lower.contains("elevenlabs")
        && (lower.contains("payment_required") || lower.contains("failed or incomplete payment"))
    {
        return Some(VOICE_PROVIDER_PAYMENT_REQUIRED_USER_MESSAGE);
    }
    None
}

/// Whether `message` is a raw upstream voice-provider config error or one of
/// our own mapped user messages (the latter so the RPC-boundary re-report of
/// the already-mapped string is demoted too).
pub fn is_voice_provider_user_state_message(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    voice_provider_user_message(&lower).is_some()
        || lower.contains(&VOICE_PROVIDER_PAYMENT_REQUIRED_USER_MESSAGE.to_ascii_lowercase())
        || lower.contains(&VOICE_PROVIDER_KEY_ID_USER_MESSAGE.to_ascii_lowercase())
}

/// Replace a known upstream voice-provider error with its user-facing message,
/// logging the raw error at `warn`. Unknown errors pass through unchanged.
pub fn map_voice_provider_error(raw: String) -> String {
    match voice_provider_user_message(&raw) {
        Some(user) => {
            tracing::warn!(
                "[voice-provider] upstream account/key problem mapped to user message: {raw}"
            );
            user.to_string()
        }
        None => raw,
    }
}
