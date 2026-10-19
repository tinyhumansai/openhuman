//! OpenHuman privacy policy at speech request boundaries.
//!
//! Speech uploads carry user content even when the destination is the managed
//! backend. They must use inference egress, not integration egress (whose
//! control-plane exemption would otherwise allow `/openai/v1/audio/*`).

use crate::security::egress::{
    emit_external_transfer, enforce_egress, DataKind, EgressDescriptor, EgressReason,
};

/// Enforce before disclosing and sending. Disclose only the actual destination
/// origin, never its URL credentials, query parameters, filenames or payload.
/// Cloud/provider routes remain external even with a loopback proxy; local
/// Piper synthesis does not call this boundary.
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

// A speech disclosure names one configured origin. Do not replay user content
// to a redirect target that has not been checked or disclosed by OpenHuman.
pub(super) fn speech_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("failed to build speech HTTP client")
}
