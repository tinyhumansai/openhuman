//! JSON-RPC controller surface for inference operations.

use crate::config::rpc as config_rpc;
use crate::config::Config;
use crate::core::runtime::DispatchCtx;
use crate::core::Outcome;
use crate::inference::host_runtime as local_runtime;
use crate::inference::provider as providers;
use serde_json::{json, Value};
use tinyinference_llm::message::Message;
use tinyinference_llm::model::ModelRequest;
use tinyinference_llm::sentiment::{parse_sentiment_response, SentimentResult};
use tinyinference_local::status::LocalAiStatus;
use tracing::{debug, error, warn};

const LOG_PREFIX: &str = "[inference::ops]";

/// User picked a provider id (slug) that isn't registered in the cloud
/// provider list — e.g. selecting `"ollama"` as a cloud provider when it's
/// actually a local runtime. Matches the literal phrase emitted at
/// `crates/openhuman-core/src/inference/provider/ops.rs:54`
/// (`"no cloud provider with id or slug '{}' found"`).
///
/// Used by [`inference_list_models`] to demote this user-config case to
/// `warn!` so it stops escalating to Sentry (TAURI-RUST-X, ~5740 events).
/// The matcher is anchored on the exact phrase so unrelated sibling
/// failures (TAURI-RUST-12 JSON parse, TAURI-RUST-2W reqwest builder,
/// TAURI-RUST-JP local ollama_admin transport) still surface as real
/// errors.
fn is_unknown_provider_user_config(err: &str) -> bool {
    err.contains("no cloud provider with id or slug")
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct InferenceTestChatModelResult {
    pub reply: String,
}

fn expected_test_provider_model_error_kind(
    err: &str,
) -> Option<crate::core::observability::ExpectedErrorKind> {
    crate::core::observability::expected_error_kind(err)
}

pub async fn inference_status(config: &Config) -> Result<Outcome<LocalAiStatus>, String> {
    debug!("{LOG_PREFIX} status:start");
    let result = local_runtime::rpc::local_ai_status(config).await;
    match &result {
        Ok(outcome) => debug!(state = %outcome.value.state, "{LOG_PREFIX} status:ok"),
        Err(err) => warn!(error = %err, "{LOG_PREFIX} status:error"),
    }
    result
}

pub async fn inference_summarize(
    config: &Config,
    text: &str,
    max_tokens: Option<u32>,
) -> Result<Outcome<String>, String> {
    debug!(
        text_len = text.len(),
        ?max_tokens,
        "{LOG_PREFIX} summarize:start"
    );
    let result = local_runtime::rpc::local_ai_summarize(config, text, max_tokens).await;
    match &result {
        Ok(outcome) => debug!(
            output_len = outcome.value.len(),
            "{LOG_PREFIX} summarize:ok"
        ),
        Err(err) => warn!(error = %err, "{LOG_PREFIX} summarize:error"),
    }
    result
}

pub async fn inference_prompt(
    config: &Config,
    prompt: &str,
    max_tokens: Option<u32>,
    no_think: Option<bool>,
) -> Result<Outcome<String>, String> {
    debug!(
        prompt_len = prompt.len(),
        ?max_tokens,
        ?no_think,
        "{LOG_PREFIX} prompt:start"
    );
    let result = local_runtime::rpc::local_ai_prompt(config, prompt, max_tokens, no_think).await;
    match &result {
        Ok(outcome) => debug!(output_len = outcome.value.len(), "{LOG_PREFIX} prompt:ok"),
        Err(err) => warn!(error = %err, "{LOG_PREFIX} prompt:error"),
    }
    result
}

pub async fn inference_vision_prompt(
    config: &Config,
    prompt: &str,
    image_refs: &[String],
    max_tokens: Option<u32>,
) -> Result<Outcome<String>, String> {
    debug!(
        prompt_len = prompt.len(),
        image_count = image_refs.len(),
        ?max_tokens,
        "{LOG_PREFIX} vision_prompt:start"
    );
    let result =
        local_runtime::rpc::local_ai_vision_prompt(config, prompt, image_refs, max_tokens).await;
    match &result {
        Ok(outcome) => debug!(
            output_len = outcome.value.len(),
            "{LOG_PREFIX} vision_prompt:ok"
        ),
        Err(err) => warn!(error = %err, "{LOG_PREFIX} vision_prompt:error"),
    }
    result
}

pub async fn inference_test_provider_model(
    config: &Config,
    workload: &str,
    provider: &str,
    prompt: &str,
) -> Result<Outcome<InferenceTestChatModelResult>, String> {
    debug!(
        workload,
        provider,
        prompt_len = prompt.len(),
        "{LOG_PREFIX} test_provider_model:start"
    );
    let local = provider.trim().starts_with("lmstudio:")
        || provider.trim().starts_with("ollama:")
        || provider.trim().starts_with("mlx:")
        || provider.trim().starts_with("omlx:")
        || provider.trim().starts_with("local-openai:");
    let (chat_model, model) = if local {
        debug!(
            provider,
            "{LOG_PREFIX} test_provider_model:build_local_model"
        );
        crate::inference::provider::factory::create_local_chat_model_from_string(provider, config)
    } else {
        debug!(provider, "{LOG_PREFIX} test_provider_model:build_model");
        crate::inference::provider::create_chat_model_from_string_with_model_id(
            workload,
            provider,
            config,
            config.default_temperature,
        )
    }
    .map_err(|e| e.to_string())?;
    debug!(%model, local, "{LOG_PREFIX} test_provider_model:invoke");
    let result = chat_model
        .invoke(
            &(),
            ModelRequest::new(vec![Message::user(prompt)])
                .with_model(model)
                .with_temperature(config.default_temperature),
        )
        .await
        .map_err(|e| e.to_string())
        .map(|response| {
            Outcome::single_log(
                InferenceTestChatModelResult {
                    reply: response.text(),
                },
                "provider model test completed",
            )
        });
    match &result {
        Ok(outcome) => debug!(
            output_len = outcome.value.reply.len(),
            "{LOG_PREFIX} test_provider_model:ok"
        ),
        Err(err) => {
            if let Some(kind) = expected_test_provider_model_error_kind(err) {
                warn!(
                    workload,
                    provider,
                    expected_error_kind = ?kind,
                    error = %err,
                    "{LOG_PREFIX} test_provider_model:expected_error"
                );
            } else {
                error!(
                    workload,
                    provider,
                    error = %err,
                    "{LOG_PREFIX} test_provider_model:error"
                );
            }
        }
    }
    result
}

pub async fn inference_analyze_sentiment(
    config: &Config,
    message: &str,
) -> Result<Outcome<SentimentResult>, String> {
    debug!(
        message_len = message.len(),
        "{LOG_PREFIX} analyze_sentiment:start"
    );
    if message.trim().is_empty() {
        return Ok(Outcome::single_log(
            SentimentResult::neutral(),
            "empty message — neutral sentiment",
        ));
    }

    let service = local_runtime::global(config);
    if service.status().state != "ready" {
        return Ok(Outcome::single_log(
            SentimentResult::neutral(),
            "local model not ready",
        ));
    }

    let prompt = format!(
        "Classify the emotion and sentiment of this user message.\n\
         Reply with EXACTLY three words separated by spaces:\n\
         EMOTION VALENCE CONFIDENCE\n\
         Where EMOTION is one of: joy, sadness, anger, surprise, fear, disgust, neutral\n\
         VALENCE is one of: positive, negative, neutral\n\
         CONFIDENCE is a number from 0.0 to 1.0\n\n\
         User message: {message}"
    );
    let runtime = crate::inference::local_runtime_config(config);
    let Some(_permit) = crate::platform::llm_gate::wait_for_capacity().await else {
        return Ok(Outcome::single_log(
            SentimentResult::neutral(),
            "local inference paused while signed out",
        ));
    };
    let result = match service.prompt(&runtime, &prompt, Some(8), true).await {
        Ok(raw) => parse_sentiment_response(&raw.trim().to_lowercase()),
        Err(error) => {
            debug!(%error, "{LOG_PREFIX} sentiment inference failed; returning neutral");
            SentimentResult::neutral()
        }
    };
    let result = Ok(Outcome::single_log(result, "sentiment analysis completed"));
    match &result {
        Ok(outcome) => {
            debug!(valence = %outcome.value.valence, "{LOG_PREFIX} analyze_sentiment:ok")
        }
        Err(err) => warn!(error = %err, "{LOG_PREFIX} analyze_sentiment:error"),
    }
    result
}

pub async fn inference_get_client_config(ctx: &DispatchCtx) -> Result<Outcome<Value>, String> {
    debug!("{LOG_PREFIX} get_client_config:start");
    let result = config_rpc::load_and_get_client_config_snapshot(ctx).await;
    match &result {
        Ok(_) => debug!("{LOG_PREFIX} get_client_config:ok"),
        Err(err) => warn!(error = %err, "{LOG_PREFIX} get_client_config:error"),
    }
    result
}

pub async fn inference_update_model_settings(
    ctx: &DispatchCtx,
    update: config_rpc::ModelSettingsPatch,
) -> Result<Outcome<Value>, String> {
    debug!("{LOG_PREFIX} update_model_settings:start");
    let result = config_rpc::load_and_apply_model_settings(ctx, update).await;
    match &result {
        Ok(_) => debug!("{LOG_PREFIX} update_model_settings:ok"),
        Err(err) => warn!(error = %err, "{LOG_PREFIX} update_model_settings:error"),
    }
    result
}

pub async fn inference_update_local_settings(
    ctx: &DispatchCtx,
    update: config_rpc::LocalAiSettingsPatch,
) -> Result<Outcome<Value>, String> {
    debug!("{LOG_PREFIX} update_local_settings:start");
    let result = config_rpc::load_and_apply_local_ai_settings(ctx, update).await;
    match &result {
        Ok(_) => {
            debug!("{LOG_PREFIX} update_local_settings:ok");
            // The endpoint, provider or models may have changed: drop the
            // cached probe verdict so the next status poll re-probes the
            // user's runtime instead of reporting the old endpoint's state.
            match ctx.load_config().await {
                Ok(config) => {
                    let runtime = crate::inference::local_runtime_config(&config);
                    local_runtime::global(&config).reset_to_idle(&runtime);
                    debug!("{LOG_PREFIX} update_local_settings:probe_reset");
                }
                Err(err) => warn!(
                    error = %err,
                    "{LOG_PREFIX} update_local_settings:probe_reset_skipped (config reload failed)"
                ),
            }
        }
        Err(err) => warn!(error = %err, "{LOG_PREFIX} update_local_settings:error"),
    }
    result
}

pub async fn inference_list_models(provider_id: &str) -> Result<Outcome<Value>, String> {
    debug!(provider_id, "{LOG_PREFIX} list_models:start");
    let result = providers::ops::list_configured_models(provider_id).await;
    match &result {
        Ok(_) => debug!("{LOG_PREFIX} list_models:ok"),
        Err(err) => {
            if is_unknown_provider_user_config(err) {
                // User selected a provider id that isn't a registered
                // cloud provider (e.g. picking "ollama", a local runtime).
                // Demote to `warn!` so it stays in local logs but doesn't
                // escalate to Sentry. Targets TAURI-RUST-X (~5740 events).
                warn!(
                    provider_id,
                    error = %err,
                    "{LOG_PREFIX} list_models:unknown-provider (user-config)"
                );
            } else if let Some(kind) = crate::core::observability::expected_error_kind(err) {
                // Classify at the TYPED SOURCE — run the raw provider error
                // through the central classifier BEFORE the
                // `[inference::ops] list_models:error: …` prefix is applied,
                // then `warn!` so the Sentry tracing layer records at most a
                // breadcrumb instead of a hard error event.
                //
                // TAURI-RUST-8X3: a user pointed a custom OpenAI-compatible
                // provider at a base URL with no `/models` route, so the
                // probe returns `provider returned 404: 404 page not found`.
                // That is a preventable user-state condition (wrong base URL;
                // the dropdown already surfaces an actionable hint inline) —
                // not a code bug. The 404 arm of
                // `is_provider_user_state_message` matched the *raw* error
                // string fine, but the previous `error!` path captured the
                // PREFIXED log line, so the demotion never reached Sentry's
                // classifier. Classifying the raw `err` here removes that
                // dependency on the log-string shape entirely; any
                // `ExpectedErrorKind` the central classifier recognizes is
                // demoted at the source.
                warn!(
                    provider_id,
                    error = %err,
                    expected_kind = ?kind,
                    "{LOG_PREFIX} list_models:expected (user-config): {err}"
                );
            } else {
                // Real error — embed `{err}` in the format string so
                // Sentry's event title carries the actionable cause
                // instead of the opaque `list_models:error` shape that
                // made TAURI-RUST-X untriageable.
                error!(
                    provider_id,
                    error = %err,
                    "{LOG_PREFIX} list_models:error: {err}"
                );
            }
        }
    }
    result
}

/// Snapshot of BYO provider auth failures (invalid / revoked key, 401 / 403)
/// recorded this process. Backs the AI-settings provider-error notice so a
/// key that breaks at runtime — most often in a silent background loop like
/// memory summarization (TAURI-RUST-4RC) — is surfaced inline next to the key
/// editor, not only in the notification center. Cleared when the user updates
/// or removes the offending key.
pub async fn inference_provider_auth_errors() -> Result<Outcome<Value>, String> {
    let errors = crate::inference::auth_error_registry::snapshot();
    debug!(count = errors.len(), "{LOG_PREFIX} provider_auth_errors:ok");
    Ok(Outcome::single_log(
        json!({ "errors": errors }),
        "inference provider auth errors fetched",
    ))
}

pub async fn inference_openai_oauth_start(config: &Config) -> Result<Outcome<Value>, String> {
    debug!("{LOG_PREFIX} openai_oauth_start:start");
    let result = crate::security::credentials::openai_oauth::start_openai_oauth(config)
        .await
        .map(|start| {
            Outcome::single_log(
                json!({
                    "authUrl": start.auth_url,
                    "state": start.state,
                    "redirectUri": start.redirect_uri,
                }),
                "openai oauth authorize url ready",
            )
        });
    match &result {
        Ok(_) => debug!("{LOG_PREFIX} openai_oauth_start:ok"),
        Err(err) => warn!(error = %err, "{LOG_PREFIX} openai_oauth_start:error"),
    }
    result
}

pub async fn inference_openai_oauth_complete(
    config: &Config,
    callback_url: &str,
) -> Result<Outcome<Value>, String> {
    debug!(
        callback_len = callback_url.len(),
        "{LOG_PREFIX} openai_oauth_complete:start"
    );
    let result =
        crate::security::credentials::openai_oauth::complete_openai_oauth(config, callback_url)
            .await
            .map(|payload| Outcome::single_log(payload, "openai oauth connected"));
    match &result {
        Ok(_) => debug!("{LOG_PREFIX} openai_oauth_complete:ok"),
        Err(err) => warn!(error = %err, "{LOG_PREFIX} openai_oauth_complete:error"),
    }
    result
}

pub async fn inference_openai_oauth_import_codex_cli(
    config: &Config,
) -> Result<Outcome<Value>, String> {
    debug!("{LOG_PREFIX} openai_oauth_import_codex_cli:start");
    let result =
        crate::security::credentials::openai_oauth::import_openai_oauth_from_codex_cli(config)
            .map(|payload| Outcome::single_log(payload, "openai oauth imported from codex cli"));
    match &result {
        Ok(_) => debug!("{LOG_PREFIX} openai_oauth_import_codex_cli:ok"),
        // Most failures here are expected user-state (no `~/.codex/auth.json`,
        // user never ran `codex login`, stale/empty file) — the UI already
        // surfaces the actionable error, so route through the observability
        // classifier to keep that flood out of Sentry (TAURI-RUST-83A) while a
        // genuine keyring/persist defect still falls through to a real event.
        Err(err) => crate::core::observability::report_error_or_expected(
            err,
            "inference",
            "openai_oauth_import_codex_cli",
            &[],
        ),
    }
    result
}

pub async fn inference_openai_oauth_status(config: &Config) -> Result<Outcome<Value>, String> {
    debug!("{LOG_PREFIX} openai_oauth_status:start");
    let result =
        crate::security::credentials::openai_oauth::openai_oauth_status(config).map(|status| {
            Outcome::single_log(
                json!({
                    "connected": status.connected,
                    "profileId": status.profile_id,
                    "expiresAt": status.expires_at,
                    "authMethod": status.auth_method,
                }),
                "openai oauth status",
            )
        });
    match &result {
        Ok(_) => debug!("{LOG_PREFIX} openai_oauth_status:ok"),
        Err(err) => warn!(error = %err, "{LOG_PREFIX} openai_oauth_status:error"),
    }
    result
}

pub async fn inference_openai_oauth_disconnect(config: &Config) -> Result<Outcome<Value>, String> {
    debug!("{LOG_PREFIX} openai_oauth_disconnect:start");
    let result = crate::security::credentials::openai_oauth::disconnect_openai_oauth(config)
        .await
        .map(|payload| Outcome::single_log(payload, "openai oauth disconnected"));
    match &result {
        Ok(_) => debug!("{LOG_PREFIX} openai_oauth_disconnect:ok"),
        Err(err) => warn!(error = %err, "{LOG_PREFIX} openai_oauth_disconnect:error"),
    }
    result
}

pub async fn inference_diagnostics(config: &Config) -> Result<Outcome<Value>, String> {
    debug!("{LOG_PREFIX} diagnostics:start");
    let service = local_runtime::global(config);
    // Return the diagnostics payload directly (no `{result, logs}` wrap) so
    // callers (UI + json_rpc_e2e tests) can read `provider`, `lm_studio_running`,
    // etc. straight off the response — mirrors the legacy
    // `local_ai_diagnostics` shape that the test asserts against.
    let runtime = crate::inference::local_runtime_config(config);
    let result = service
        .diagnostics(&runtime)
        .await
        .map(|value| Outcome::new(value, Vec::new()));
    match &result {
        Ok(_) => debug!("{LOG_PREFIX} diagnostics:ok"),
        Err(err) => warn!(error = %err, "{LOG_PREFIX} diagnostics:error"),
    }
    result
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
