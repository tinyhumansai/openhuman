//! Privacy policy for hosted speech requests that leave this machine.
//!
//! The hosted transcription and reply-speech routes moved here with the
//! TinyHumans voice client. They still have to use inference egress, because
//! the integration control-plane exemption would otherwise allow
//! `/openai/v1/audio/*` without a disclosure.

pub(super) use openhuman_embed::__host::egress::DataKind;
use openhuman_embed::__host::egress::{
    emit_external_transfer, enforce_egress, EgressDescriptor, EgressReason,
};

/// Enforce before disclosing and sending. Disclose only the actual destination
/// origin, never its URL credentials, query parameters, filenames or payload.
pub(super) fn disclose_speech_transfer(
    provider_slug: &str,
    endpoint: &str,
    data_kind: DataKind,
) -> Result<(), String> {
    let url = reqwest::Url::parse(endpoint)
        .map_err(|_| "invalid speech provider endpoint".to_string())?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err("speech provider endpoint must be an HTTP(S) URL".to_string());
    }
    let descriptor = EgressDescriptor::new(
        provider_slug,
        url.origin().ascii_serialization(),
        true,
        EgressReason::Inference,
        vec![data_kind],
    );
    enforce_egress(&descriptor).map_err(|error| error.to_string())?;
    emit_external_transfer(None, descriptor);
    Ok(())
}

#[cfg(test)]
#[path = "egress_tests.rs"]
mod tests;
