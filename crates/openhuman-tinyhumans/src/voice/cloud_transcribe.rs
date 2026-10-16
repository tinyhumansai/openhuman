//! OpenHuman authentication adapter for hosted speech-to-text.

use openhuman_embed::__host::config::Config;
use openhuman_embed::__host::core::Outcome;
use openhuman_embed::__host::security::credentials::session_support::resolve_backend_credential;

use crate::client::{
    flatten_authed_error, is_managed_endpoint_for_api_key, is_safe_endpoint_for_managed_bearer,
    BackendClient,
};

use tinyinference_voice::cloud::{CloudTranscribeOptions, CloudTranscribeResult};

/// The route this module posts to.
const TRANSCRIPTIONS_PATH: &str = "/openai/v1/audio/transcriptions";

/// Transcribe renderer-supplied base64 audio through the hosted backend.
pub async fn transcribe_cloud(
    config: &Config,
    audio_base64: &str,
    options: &CloudTranscribeOptions,
) -> Result<Outcome<CloudTranscribeResult>, String> {
    // The session JWT or the TinyHumans API key. The vendored client sends it
    // as `Authorization: Bearer`, which the backend accepts for either.
    let credential = resolve_backend_credential(config)?;
    let is_api_key = credential.is_api_key();
    let client = BackendClient::from_config(config).map_err(|error| error.to_string())?;
    let url = client
        .url_for(TRANSCRIPTIONS_PATH)
        .map_err(|error| error.to_string())?;
    let safe_for_bearer = is_safe_endpoint_for_managed_bearer(url.as_str());
    let managed_for_api_key = is_managed_endpoint_for_api_key(url.as_str());
    if !safe_for_bearer || (is_api_key && !managed_for_api_key) {
        return Err(
            "refusing to send the TinyHumans API key over a non-HTTPS, non-loopback endpoint"
                .to_string(),
        );
    }
    let http = client.raw_client().map_err(flatten_authed_error)?;
    let result = tinyinference_voice::cloud::transcribe(
        &http,
        url,
        credential.secret(),
        audio_base64,
        options,
    )
    .await
    .map_err(|error| classify_transcribe_error(error, is_api_key))?;
    Ok(Outcome::single_log(
        result,
        "cloud STT via POST /openai/v1/audio/transcriptions",
    ))
}

/// Tag a backend 401 with the `SESSION_EXPIRED` sentinel. The request carries
/// the app-session JWT to the hosted backend, so a 401 here is the session
/// lapsing — the JSON-RPC layer then re-auths instead of paging Sentry with
/// the raw `transcription request failed (401 …)` string.
fn classify_transcribe_error(error: String, is_api_key: bool) -> String {
    if error.starts_with("transcription request failed (401") {
        if is_api_key {
            log::info!("[voice][cloud_stt] backend rejected the API key (401)");
            format!("API_KEY_REJECTED: {error}")
        } else {
            log::info!("[voice][cloud_stt] backend rejected the session (401)");
            format!("SESSION_EXPIRED: {error}")
        }
    } else {
        error
    }
}

#[cfg(test)]
#[path = "cloud_transcribe_tests.rs"]
mod tests;
