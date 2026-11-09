//! Host policy over `tinyinference_llm::failure::http`: the body/status
//! classifiers and user messages live upstream; this module owns only the
//! OpenHuman-specific reaction (backend polarity guards, `SessionExpired` /
//! `ProviderApiKeyRejected` publishing, Sentry demotion) behind an
//! [`ErrorReporter`].

#[cfg(test)]
#[path = "http_error_tests.rs"]
mod tests;

use crate::inference::registry::classify_backend_error;
use tinyinference_core::sanitize::sanitize_api_error;
use tinyinference_llm::failure::demotion::{demote_decision, DemotionKind};
use tinyinference_llm::failure::http::{
    classify_and_report_http_failure, ErrorReporter, ProviderHttpFailure, ProviderHttpFailureFacts,
};
use tinyinference_llm::failure::{
    local_provider_no_model_loaded_user_message, ollama_cloud_internal_500_user_message,
    should_report_provider_http_status,
};

/// Whether a non-2xx provider status is worth a per-attempt Sentry report.
pub fn should_report_provider_http_failure(status: reqwest::StatusCode) -> bool {
    should_report_provider_http_status(status.as_u16())
}

/// Whether this is the OpenHuman backend rejecting the app session JWT. This
/// path only sends the session credential, so `api_key` is always false.
pub fn is_backend_auth_failure(provider: &str, status: reqwest::StatusCode) -> bool {
    crate::core::session_expiry::backend_status_means_session_expired(status.as_u16(), false)
        && classify_backend_error(Some(provider), "").is_some_and(|result| result.provider_owned)
}

/// Whether the response carries a managed-backend `errorCode` the backend
/// already owns. Only trustworthy on the managed backend provider.
pub fn is_backend_error_code_owned(provider: &str, body: &str) -> bool {
    classify_backend_error(Some(provider), body)
        .is_some_and(|result| result.backend_owned && !result.reportable)
}

fn log_user_state(
    level_warn: bool,
    kind: &str,
    reason: &str,
    operation: &str,
    provider: &str,
    model: Option<&str>,
    status: u16,
) {
    if level_warn {
        tracing::warn!(
            domain = "llm_provider",
            operation,
            provider,
            model = model.unwrap_or(""),
            status,
            failure = "non_2xx",
            kind,
            reason,
            "[llm_provider] {operation} {reason} ({status}) — not reporting to Sentry"
        );
    } else {
        tracing::info!(
            domain = "llm_provider",
            operation,
            provider,
            model = model.unwrap_or(""),
            status,
            failure = "non_2xx",
            kind,
            reason,
            "[llm_provider] {operation} {reason} ({status}) — not reporting to Sentry"
        );
    }
}

/// Demote a BYO provider auth failure and, on the first record of the episode,
/// surface it to the user (the registry latch keeps retries from re-flooding).
pub fn log_byo_provider_auth_failure(
    operation: &str,
    provider: &str,
    model: Option<&str>,
    status: reqwest::StatusCode,
) {
    log_user_state(
        false,
        "provider_user_state",
        "byo_provider_auth_failure",
        operation,
        provider,
        model,
        status.as_u16(),
    );
    let status_code = status.as_u16();
    if crate::inference::auth_error_registry::record(provider, status_code) {
        crate::core::bus::BUS.publish(crate::core::events::DomainEvent::ProviderApiKeyRejected {
            provider: provider.to_string(),
            message: crate::inference::auth_error_registry::auth_error_message(
                provider,
                status_code,
            ),
        });
    }
}

/// Publish `SessionExpired` for a backend session 401 and skip the Sentry report.
/// `message` is re-sanitized before it reaches the subscriber's logs.
pub fn publish_backend_session_expired(
    operation: &str,
    provider: &str,
    status: reqwest::StatusCode,
    message: &str,
) {
    tracing::warn!(
        domain = "llm_provider",
        operation = operation,
        provider = provider,
        status = status.as_u16(),
        "[llm_provider] backend auth failure ({status}) — publishing SessionExpired"
    );
    crate::core::bus::BUS.publish(crate::core::events::DomainEvent::SessionExpired {
        source: "llm_provider.openhuman_backend".to_string(),
        reason: sanitize_api_error(message),
        // Generic provider errors do not retain the request bearer. Without
        // request-time provenance the credentials subscriber must not sign
        // out whichever session happens to be current now.
        credential_fingerprint: None,
    });
}

/// The OpenHuman [`ErrorReporter`]: applies backend polarity guards and
/// demotes expected user/provider state; reports the rest to Sentry.
struct HostErrorReporter<'a> {
    message: &'a str,
    backend_error_code_owned: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HostDemotion {
    Provider(DemotionKind),
    BackendOwned,
    Report,
}

/// Compose the host's backend `errorCode` guard at its historical position:
/// after quota demotion, before the remaining provider/account classifications.
fn select_demotion(
    facts: &ProviderHttpFailureFacts,
    provider_is_managed: bool,
    backend_error_code_owned: bool,
) -> HostDemotion {
    let decision = demote_decision(facts, provider_is_managed);
    if let Some(
        kind @ (DemotionKind::BudgetExhausted
        | DemotionKind::NoModelLoaded
        | DemotionKind::CustomUpstreamBadRequest
        | DemotionKind::AccessPolicyDenied
        | DemotionKind::ConfigurationRejected
        | DemotionKind::ContextWindowExceeded
        | DemotionKind::QuotaExhausted),
    ) = decision
    {
        HostDemotion::Provider(kind)
    } else if backend_error_code_owned {
        HostDemotion::BackendOwned
    } else if let Some(kind) = decision {
        HostDemotion::Provider(kind)
    } else {
        HostDemotion::Report
    }
}

impl ErrorReporter for HostErrorReporter<'_> {
    fn report(&self, failure: ProviderHttpFailure<'_>) {
        let ProviderHttpFailure {
            provider,
            operation,
            model,
            status,
            facts,
        } = failure;
        let is_backend =
            classify_backend_error(Some(provider), "").is_some_and(|result| result.provider_owned);
        let status_code =
            reqwest::StatusCode::from_u16(status).unwrap_or(reqwest::StatusCode::BAD_GATEWAY);
        let demote = |kind: &str, reason: &str, warn: bool| {
            log_user_state(warn, kind, reason, operation, provider, model, status);
        };
        let decision = select_demotion(&facts, is_backend, self.backend_error_code_owned);

        if is_backend
            && crate::core::session_expiry::backend_status_means_session_expired(status, false)
        {
            publish_backend_session_expired(operation, provider, status_code, self.message);
        } else {
            match decision {
                HostDemotion::Provider(DemotionKind::BudgetExhausted) => {
                    demote("budget", "budget-exhausted 400", false)
                }
                HostDemotion::Provider(DemotionKind::NoModelLoaded) => demote(
                    "provider_user_state",
                    "local_provider_no_model_loaded",
                    false,
                ),
                HostDemotion::Provider(DemotionKind::CustomUpstreamBadRequest) => demote(
                    "provider_user_state",
                    "custom_openai_upstream_bad_request",
                    false,
                ),
                HostDemotion::Provider(DemotionKind::AccessPolicyDenied) => demote(
                    "provider_access_policy",
                    "provider access-policy 403",
                    false,
                ),
                HostDemotion::Provider(DemotionKind::ConfigurationRejected) => demote(
                    "provider_config_rejection",
                    "provider config-rejection",
                    false,
                ),
                HostDemotion::Provider(DemotionKind::ContextWindowExceeded) => {
                    demote("context_window_exceeded", "context-window exceeded", true)
                }
                HostDemotion::Provider(DemotionKind::QuotaExhausted) => {
                    demote("quota_exhausted", "provider monthly-quota exhausted", false)
                }
                HostDemotion::BackendOwned => {
                    demote("backend_error_code", "backend errorCode owned", false)
                }
                HostDemotion::Provider(DemotionKind::ApiKeyRejected) => {
                    log_byo_provider_auth_failure(operation, provider, model, status_code)
                }
                HostDemotion::Provider(DemotionKind::OAuthSessionExpired) => {
                    demote("provider_user_state", "openai_oauth_session_expired", false)
                }
                HostDemotion::Provider(DemotionKind::InsufficientCredits) => demote(
                    "insufficient_credits",
                    "provider insufficient-credits 402",
                    false,
                ),
                HostDemotion::Provider(DemotionKind::OllamaCloudInternalError) => demote(
                    "ollama_cloud_internal_500",
                    "Ollama Cloud hosted-inference 500",
                    false,
                ),
                HostDemotion::Provider(DemotionKind::ModerationRejected) => demote(
                    "external_moderation_rejection",
                    "external content-moderation rejection",
                    false,
                ),
                HostDemotion::Report if should_report_provider_http_status(status) => {
                    let status_str = status.to_string();
                    crate::core::observability::report_error(
                        self.message,
                        "llm_provider",
                        "api_error",
                        &[
                            ("provider", provider),
                            ("status", status_str.as_str()),
                            ("failure", "non_2xx"),
                        ],
                    );
                }
                HostDemotion::Report => {}
            }
        }
    }
}

/// An error body is only classified and quoted (a few hundred characters at
/// most), so read no more than this: a misbehaving endpoint must not make the
/// client buffer an unbounded body.
const MAX_ERROR_BODY_BYTES: usize = 64 * 1024;

/// The first [`MAX_ERROR_BODY_BYTES`] of a failed response's body.
async fn read_error_body(mut response: reqwest::Response) -> String {
    let mut body = Vec::new();
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                let room = MAX_ERROR_BODY_BYTES - body.len();
                body.extend_from_slice(&chunk[..chunk.len().min(room)]);
                if body.len() == MAX_ERROR_BODY_BYTES {
                    break;
                }
            }
            Ok(None) => break,
            Err(_) if body.is_empty() => {
                return "<failed to read provider error body>".to_string();
            }
            Err(_) => break,
        }
    }
    String::from_utf8_lossy(&body).into_owned()
}

/// Build a sanitized provider error from a failed HTTP response, classifying
/// it upstream and routing the reaction through [`HostErrorReporter`].
pub async fn api_error(provider: &str, response: reqwest::Response) -> anyhow::Error {
    let status = response.status();
    let body = read_error_body(response).await;
    let message = format!(
        "{provider} API error ({status}): {}",
        sanitize_api_error(&body)
    );
    let reporter = HostErrorReporter {
        message: &message,
        backend_error_code_owned: is_backend_error_code_owned(provider, &body),
    };
    let facts = classify_and_report_http_failure(
        provider,
        "api_error",
        None,
        status.as_u16(),
        &body,
        &reporter,
    );
    if facts.ollama_cloud_internal_error {
        return anyhow::anyhow!(ollama_cloud_internal_500_user_message(
            None,
            status.as_u16()
        ));
    }
    if facts.no_model_loaded {
        return anyhow::anyhow!(local_provider_no_model_loaded_user_message());
    }
    anyhow::anyhow!(message)
}
