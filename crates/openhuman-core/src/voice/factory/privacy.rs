//! Privacy Mode chokepoint for speech-to-text construction.
//!
//! Mirrors `inference::provider::factory::access_gates`: a pure decision
//! function plus a thin enforcement wrapper that reads the live policy, so the
//! truth table is unit-testable without process-global state.
//!
//! No local speech-to-text engine ships with the product, but a voice provider
//! entry is an OpenAI-style endpoint, so a transcription server the user runs
//! on this machine keeps audio on it. Under `LocalOnly` that is the only
//! speech-to-text permitted: a loopback endpoint passes; a registered provider
//! (the hosted proxy), a remote endpoint and a LAN address are refused when the
//! provider is built, before any audio is sent (#6870). The hosted transcribe
//! path calls [`enforce_local_only_stt`] itself, so the retained
//! `voice_cloud_transcribe` RPC is held to the same rule.
//!
//! Live voice sessions (`voice::live`) are a separate path and not covered.

use crate::config::PrivacyMode;

/// Destination named in the refusal for a registered provider. Registered
/// providers carry no endpoint the core can inspect, so they fail closed.
pub(super) const HOSTED_STT_LABEL: &str = "a hosted transcription service";

/// Destination named for an endpoint whose host cannot be established.
pub(super) const UNPARSEABLE_ENDPOINT_LABEL: &str = "an endpoint that could not be parsed";

/// Pure decision: under `mode`, would building an STT provider that posts audio
/// to `endpoint` violate local-only? `None` means permitted; `Some` names the
/// destination for the error. `endpoint` is `None` for a registered provider.
pub(super) fn local_only_stt_violation(
    mode: PrivacyMode,
    endpoint: Option<&str>,
) -> Option<String> {
    if mode != PrivacyMode::LocalOnly {
        return None;
    }
    let Some(endpoint) = endpoint else {
        return Some(HOSTED_STT_LABEL.to_string());
    };
    match ::url::Url::parse(endpoint.trim()) {
        Ok(parsed) => match parsed.host_str() {
            Some(host) if crate::util::url::loopback_host(host) => None,
            Some(host) => Some(host.to_string()),
            None => Some(UNPARSEABLE_ENDPOINT_LABEL.to_string()),
        },
        // A destination we cannot establish is not a local one. The raw value
        // is not echoed: it may carry credentials.
        Err(_) => Some(UNPARSEABLE_ENDPOINT_LABEL.to_string()),
    }
}

/// Refuse speech-to-text that would send microphone audio off this machine
/// while the live policy is `LocalOnly`. `endpoint` is `None` for a provider
/// whose endpoint the core cannot inspect (the hosted proxy).
pub fn enforce_local_only_stt(provider: &str, endpoint: Option<&str>) -> anyhow::Result<()> {
    let mode = crate::security::live_policy::current_privacy_mode();
    let Some(destination) = local_only_stt_violation(mode, endpoint) else {
        log::debug!(
            "{} privacy_mode={mode:?} stt provider='{provider}' — transcription permitted",
            super::helpers::LOG_PREFIX
        );
        return Ok(());
    };
    log::warn!(
        "[privacy][voice-factory] LocalOnly BLOCK: stt provider='{provider}' would send audio to {destination} — refused"
    );
    anyhow::bail!(
        "Local-only privacy mode is active: transcription would send microphone audio to \
         {destination}. Point speech-to-text at a server on this computer (localhost) or \
         change privacy mode in Settings."
    )
}

#[cfg(test)]
#[path = "privacy_tests.rs"]
mod tests;
