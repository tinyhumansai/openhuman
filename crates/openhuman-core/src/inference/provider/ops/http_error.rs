//! Host policy over `tinyinference_llm::failure::http`: the body/status
//! classifiers and user messages live upstream; this module owns only the
//! OpenHuman-specific reaction (backend polarity guards, `SessionExpired` /
//! `ProviderApiKeyRejected` publishing, Sentry demotion) behind an
//! [`ErrorReporter`].

#[cfg(test)]
#[path = "http_error_tests.rs"]
mod tests;

use crate::inference::provider::openhuman_backend_model;
use tinyinference_core::sanitize::sanitize_api_error;
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

/// Whether this is the OpenHuman backend rejecting the app session JWT.
pub fn is_backend_auth_failure(provider: &str, status: reqwest::StatusCode) -> bool {
    matches!(status.as_u16(), 401 | 403) && provider == openhuman_backend_model::PROVIDER_LABEL
}

/// Whether the response carries a managed-backend `errorCode` the backend
/// already owns. Only trustworthy on the managed backend provider.
pub fn is_backend_error_code_owned(provider: &str, body: &str) -> bool {
    provider == openhuman_backend_model::PROVIDER_LABEL
        && crate::inference::provider::backend_error_code_skips_sentry(body)
}

/// Config-rejection demotion policy: the managed backend's unknown-model body
/// is user-state on every provider; the remaining rejection phrases stay
/// third-party scoped so a backend regression still reaches Sentry.
pub(crate) fn config_rejection_demoted(facts: &ProviderHttpFailureFacts, provider: &str) -> bool {
    facts.configuration_rejected
        && (facts.unknown_model || provider != openhuman_backend_model::PROVIDER_LABEL)
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

/// Publish `SessionExpired` for a backend 401/403 and skip the Sentry report.
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
    });
}

/// The OpenHuman [`ErrorReporter`]: applies backend polarity guards and
/// demotes expected user/provider state; reports the rest to Sentry.
struct HostErrorReporter<'a> {
    message: &'a str,
    backend_error_code_owned: bool,
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
        let is_backend = provider == openhuman_backend_model::PROVIDER_LABEL;
        let status_code =
            reqwest::StatusCode::from_u16(status).unwrap_or(reqwest::StatusCode::BAD_GATEWAY);
        let demote = |kind: &str, reason: &str, warn: bool| {
            log_user_state(warn, kind, reason, operation, provider, model, status);
        };
        let config_rejected = config_rejection_demoted(&facts, provider);

        if matches!(status, 401 | 403) && is_backend {
            publish_backend_session_expired(operation, provider, status_code, self.message);
        } else if facts.budget_exhausted {
            demote("budget", "budget-exhausted 400", false);
        } else if facts.no_model_loaded {
            demote(
                "provider_user_state",
                "local_provider_no_model_loaded",
                false,
            );
        } else if facts.custom_upstream_bad_request {
            demote(
                "provider_user_state",
                "custom_openai_upstream_bad_request",
                false,
            );
        } else if facts.access_policy_denied {
            demote(
                "provider_access_policy",
                "provider access-policy 403",
                false,
            );
        } else if config_rejected {
            demote(
                "provider_config_rejection",
                "provider config-rejection",
                false,
            );
        } else if facts.context_window_exceeded {
            demote("context_window_exceeded", "context-window exceeded", true);
        } else if facts.quota_exhausted {
            demote("quota_exhausted", "provider monthly-quota exhausted", false);
        } else if self.backend_error_code_owned {
            demote("backend_error_code", "backend errorCode owned", false);
        } else if facts.api_key_rejected && !is_backend {
            log_byo_provider_auth_failure(operation, provider, model, status_code);
        } else if facts.oauth_session_expired && !is_backend {
            demote("provider_user_state", "openai_oauth_session_expired", false);
        } else if facts.insufficient_credits {
            demote(
                "insufficient_credits",
                "provider insufficient-credits 402",
                false,
            );
        } else if facts.ollama_cloud_internal_error {
            demote(
                "ollama_cloud_internal_500",
                "Ollama Cloud hosted-inference 500",
                false,
            );
        } else if facts.moderation_rejected {
            demote(
                "external_moderation_rejection",
                "external content-moderation rejection",
                false,
            );
        } else if should_report_provider_http_status(status) {
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
    }
}

/// Build a sanitized provider error from a failed HTTP response, classifying
/// it upstream and routing the reaction through [`HostErrorReporter`].
pub async fn api_error(provider: &str, response: reqwest::Response) -> anyhow::Error {
    let status = response.status();
    let body = response
        .text()
        .await
        .unwrap_or_else(|_| "<failed to read provider error body>".to_string());
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
