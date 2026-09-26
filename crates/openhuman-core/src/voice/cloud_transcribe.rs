//! OpenHuman authentication adapter for hosted speech-to-text.

use crate::api::config::effective_backend_api_url;
use crate::api::jwt::get_session_token;
use crate::api::BackendOAuthClient;
use crate::config::Config;
use crate::rpc::RpcOutcome;

pub use tinyinference_voice::cloud::{CloudTranscribeOptions, CloudTranscribeResult};

/// Transcribe renderer-supplied base64 audio through the hosted backend.
pub async fn transcribe_cloud(
    config: &Config,
    audio_base64: &str,
    options: &CloudTranscribeOptions,
) -> Result<RpcOutcome<CloudTranscribeResult>, String> {
    let token = get_session_token(config)
        .map_err(|error| error.to_string())?
        .filter(|token| !token.trim().is_empty())
        .ok_or_else(|| "no backend session token; sign in first".to_string())?;
    if crate::security::credentials::session_support::is_local_session_token(&token) {
        log::debug!("[voice][cloud_stt] offline local session — hosted STT unavailable");
        return Err(
            crate::security::credentials::session_support::LOCAL_SESSION_BACKEND_UNAVAILABLE
                .to_string(),
        );
    }
    let client = BackendOAuthClient::new(&effective_backend_api_url(&config.api_url))
        .map_err(|error| error.to_string())?;
    let url = client
        .url_for("/openai/v1/audio/transcriptions")
        .map_err(|error| error.to_string())?;
    let http = client
        .raw_client()
        .map_err(crate::api::flatten_authed_error)?;
    let result = tinyinference_voice::cloud::transcribe(&http, url, &token, audio_base64, options)
        .await
        .map_err(classify_transcribe_error)?;
    Ok(RpcOutcome::single_log(
        result,
        "cloud STT via POST /openai/v1/audio/transcriptions",
    ))
}

/// Tag a backend 401 with the `SESSION_EXPIRED` sentinel. The request carries
/// the app-session JWT to the hosted backend, so a 401 here is the session
/// lapsing — the JSON-RPC layer then re-auths instead of paging Sentry with
/// the raw `transcription request failed (401 …)` string.
fn classify_transcribe_error(error: String) -> String {
    if error.starts_with("transcription request failed (401") {
        log::info!("[voice][cloud_stt] backend rejected the session (401)");
        format!("SESSION_EXPIRED: {error}")
    } else {
        error
    }
}

#[cfg(test)]
#[path = "cloud_transcribe_tests.rs"]
mod tests;
