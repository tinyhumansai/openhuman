//! JSON-RPC controller surface for inference operations.

use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::config::Config;
use crate::openhuman::inference::local as local_runtime;
use crate::openhuman::inference::local::ops::ReactionDecision;
use crate::openhuman::inference::provider as providers;
use crate::openhuman::inference::{device, presets, sentiment, SentimentResult};
use crate::openhuman::inference::{LocalAiEmbeddingResult, LocalAiStatus};
use crate::rpc::RpcOutcome;
use serde_json::{json, Value};
use tracing::{debug, error, warn};

const LOG_PREFIX: &str = "[inference::ops]";

/// User picked a provider id (slug) that isn't registered in the cloud
/// provider list — e.g. selecting `"ollama"` as a cloud provider when it's
/// actually a local runtime. Matches the literal phrase emitted at
/// `src/openhuman/inference/provider/ops.rs:54`
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

/// Returns `true` when the error from a provider chat attempt is a known,
/// expected user-state or provider-state condition that already has its own
/// Sentry report (or is deterministically expected and has no remediation
/// path):
///
/// - **401 Unauthorized** — API key revoked / wrong key. Already reported by
///   the provider layer's `api_error` path. An ops-level duplicate adds noise
///   with no additional context.
/// - **429 Too Many Requests / rate-limit** — Quota exhaustion. Already
///   covered by the `is_upstream_rate_limit_message` classifier in
///   `expected_error_kind`; the reliable-provider layer retries with
///   backoff before propagating.
/// - **Model not found** — User selected a model that doesn't exist for
///   their key. The provider layer already classifies this as a config
///   rejection (TAURI-RUST-68, ~1309 events).
///
/// The matcher is intentionally broad so the ops-level wrapper stays out
/// of the Sentry funnel for all provider-state failures — the underlying
/// call site (`compatible.rs` / `report_error_or_expected`) is already
/// responsible for the authoritative report. Unclassified failures (5xx,
/// unexpected payloads, network errors) are NOT matched and still escalate.
fn is_expected_chat_failure(err: &str) -> bool {
    crate::core::observability::expected_error_kind(err).is_some()
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct InferenceTestProviderModelResult {
    pub reply: String,
}

pub async fn inference_status(config: &Config) -> Result<RpcOutcome<LocalAiStatus>, String> {
    debug!("{LOG_PREFIX} status:start");
    let result = local_runtime::rpc::local_ai_status(config).await;
    match &result {
        Ok(outcome) => debug!(state = %outcome.value.state, "{LOG_PREFIX} status:ok"),
        Err(err) => error!(error = %err, "{LOG_PREFIX} status:error"),
    }
    result
}

pub async fn inference_summarize(
    config: &Config,
    text: &str,
    max_tokens: Option<u32>,
) -> Result<RpcOutcome<String>, String> {
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
        Err(err) => error!(error = %err, "{LOG_PREFIX} summarize:error"),
    }
    result
}

pub async fn inference_prompt(
    config: &Config,
    prompt: &str,
    max_tokens: Option<u32>,
    no_think: Option<bool>,
) -> Result<RpcOutcome<String>, String> {
    debug!(
        prompt_len = prompt.len(),
        ?max_tokens,
        ?no_think,
        "{LOG_PREFIX} prompt:start"
    );
    let result = local_runtime::rpc::local_ai_prompt(config, prompt, max_tokens, no_think).await;
    match &result {
        Ok(outcome) => debug!(output_len = outcome.value.len(), "{LOG_PREFIX} prompt:ok"),
        Err(err) => error!(error = %err, "{LOG_PREFIX} prompt:error"),
    }
    result
}

pub async fn inference_vision_prompt(
    config: &Config,
    prompt: &str,
    image_refs: &[String],
    max_tokens: Option<u32>,
) -> Result<RpcOutcome<String>, String> {
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
        Err(err) => error!(error = %err, "{LOG_PREFIX} vision_prompt:error"),
    }
    result
}

pub async fn inference_embed(
    config: &Config,
    inputs: &[String],
) -> Result<RpcOutcome<LocalAiEmbeddingResult>, String> {
    debug!(input_count = inputs.len(), "{LOG_PREFIX} embed:start");
    let result = local_runtime::rpc::local_ai_embed(config, inputs).await;
    match &result {
        Ok(outcome) => debug!(
            vector_count = outcome.value.vectors.len(),
            dimensions = outcome.value.dimensions,
            "{LOG_PREFIX} embed:ok"
        ),
        Err(err) => error!(error = %err, "{LOG_PREFIX} embed:error"),
    }
    result
}

pub async fn inference_test_provider_model(
    config: &Config,
    workload: &str,
    provider: &str,
    prompt: &str,
) -> Result<RpcOutcome<InferenceTestProviderModelResult>, String> {
    debug!(
        workload,
        provider,
        prompt_len = prompt.len(),
        "{LOG_PREFIX} test_provider_model:start"
    );
    let result =
        if provider.trim().starts_with("lmstudio:") || provider.trim().starts_with("ollama:") {
            log::debug!("{LOG_PREFIX} test_provider_model: routing to local provider={provider}");
            let (chat_provider, model) =
            crate::openhuman::inference::provider::factory::create_local_chat_provider_from_string(
                provider, config,
            )
            .map_err(|e| e.to_string())?;
            log::debug!("{LOG_PREFIX} test_provider_model: invoking local model={model}");
            chat_provider
                .simple_chat(prompt, &model, config.default_temperature)
                .await
                .map_err(|e| e.to_string())
                .map(|reply| {
                    RpcOutcome::single_log(
                        InferenceTestProviderModelResult { reply },
                        "provider model test completed",
                    )
                })
        } else {
            let (chat_provider, model) =
                crate::openhuman::inference::provider::factory::create_chat_provider_from_string(
                    workload, provider, config,
                )
                .map_err(|e| e.to_string())?;
            chat_provider
                .simple_chat(prompt, &model, config.default_temperature)
                .await
                .map_err(|e| e.to_string())
                .map(|reply| {
                    RpcOutcome::single_log(
                        InferenceTestProviderModelResult { reply },
                        "provider model test completed",
                    )
                })
        };
    match &result {
        Ok(outcome) => debug!(
            output_len = outcome.value.reply.len(),
            "{LOG_PREFIX} test_provider_model:ok"
        ),
        Err(err) => {
            if is_expected_chat_failure(err) {
                // Provider-state / user-config failure (401, 429, model not
                // found, API key missing, etc.). The underlying provider
                // layer already emitted its own Sentry event or classified
                // this as expected. An ops-level duplicate adds noise.
                // Targets TAURI-RUST-68 (~1,309 events).
                warn!(
                    provider,
                    error = %err,
                    "{LOG_PREFIX} test_provider_model:expected-error (no Sentry)"
                );
            } else {
                error!(error = %err, "{LOG_PREFIX} test_provider_model:error");
            }
        }
    }
    result
}

pub async fn inference_should_react(
    config: &Config,
    message: &str,
    channel_type: &str,
) -> Result<RpcOutcome<ReactionDecision>, String> {
    debug!(
        message_len = message.len(),
        channel_type, "{LOG_PREFIX} should_react:start"
    );
    let result = local_runtime::rpc::local_ai_should_react(config, message, channel_type).await;
    match &result {
        Ok(outcome) => debug!(
            should_react = outcome.value.should_react,
            "{LOG_PREFIX} should_react:ok"
        ),
        Err(err) => error!(error = %err, "{LOG_PREFIX} should_react:error"),
    }
    result
}

pub async fn inference_analyze_sentiment(
    config: &Config,
    message: &str,
) -> Result<RpcOutcome<SentimentResult>, String> {
    debug!(
        message_len = message.len(),
        "{LOG_PREFIX} analyze_sentiment:start"
    );
    let result = sentiment::local_ai_analyze_sentiment(config, message).await;
    match &result {
        Ok(outcome) => {
            debug!(valence = %outcome.value.valence, "{LOG_PREFIX} analyze_sentiment:ok")
        }
        Err(err) => error!(error = %err, "{LOG_PREFIX} analyze_sentiment:error"),
    }
    result
}

pub async fn inference_get_client_config() -> Result<RpcOutcome<Value>, String> {
    debug!("{LOG_PREFIX} get_client_config:start");
    let result = config_rpc::load_and_get_client_config_snapshot().await;
    match &result {
        Ok(_) => debug!("{LOG_PREFIX} get_client_config:ok"),
        Err(err) => error!(error = %err, "{LOG_PREFIX} get_client_config:error"),
    }
    result
}

pub async fn inference_update_model_settings(
    update: config_rpc::ModelSettingsPatch,
) -> Result<RpcOutcome<Value>, String> {
    debug!("{LOG_PREFIX} update_model_settings:start");
    let result = config_rpc::load_and_apply_model_settings(update).await;
    match &result {
        Ok(_) => debug!("{LOG_PREFIX} update_model_settings:ok"),
        Err(err) => error!(error = %err, "{LOG_PREFIX} update_model_settings:error"),
    }
    result
}

pub async fn inference_update_local_settings(
    update: config_rpc::LocalAiSettingsPatch,
) -> Result<RpcOutcome<Value>, String> {
    debug!("{LOG_PREFIX} update_local_settings:start");
    let result = config_rpc::load_and_apply_local_ai_settings(update).await;
    match &result {
        Ok(_) => debug!("{LOG_PREFIX} update_local_settings:ok"),
        Err(err) => error!(error = %err, "{LOG_PREFIX} update_local_settings:error"),
    }
    result
}

pub async fn inference_list_models(provider_id: &str) -> Result<RpcOutcome<Value>, String> {
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

pub async fn inference_device_profile() -> Result<RpcOutcome<Value>, String> {
    debug!("{LOG_PREFIX} device_profile:start");
    let profile = device::detect_device_profile();
    let result = Ok(RpcOutcome::single_log(
        serde_json::to_value(profile).map_err(|e| format!("serialize: {e}"))?,
        "inference device profile fetched",
    ));
    debug!("{LOG_PREFIX} device_profile:ok");
    result
}

pub async fn inference_presets() -> Result<RpcOutcome<Value>, String> {
    debug!("{LOG_PREFIX} presets:start");
    let config = config_rpc::load_config_with_timeout().await?;
    let device = device::detect_device_profile();
    let recommended = presets::recommend_tier(&device);
    let current = presets::current_tier_from_config(&config.local_ai);
    let selected_tier = config.local_ai.selected_tier.as_ref().and_then(|value| {
        let normalized = value.trim().to_ascii_lowercase();
        presets::ModelTier::from_str_opt(&normalized)
            .map(|tier| tier.as_str().to_string())
            .or_else(|| (!normalized.is_empty()).then_some(normalized))
    });
    let presets = presets::mvp_presets();
    let recommend_disabled = presets::should_default_to_cloud_fallback(&device);
    let result = Ok(RpcOutcome::single_log(
        json!({
            "presets": presets,
            "recommended_tier": recommended,
            "current_tier": current,
            "selected_tier": selected_tier,
            "device": device,
            "recommend_disabled": recommend_disabled,
            "local_ai_enabled": config.local_ai.runtime_enabled,
        }),
        "inference presets fetched",
    ));
    debug!("{LOG_PREFIX} presets:ok");
    result
}

pub async fn inference_apply_preset(tier: &str) -> Result<RpcOutcome<Value>, String> {
    let tier_str = tier.trim().to_ascii_lowercase();
    debug!(tier = %tier_str, "{LOG_PREFIX} apply_preset:start");

    if tier_str == "disabled" {
        let mut config = config_rpc::load_config_with_timeout().await?;
        config.local_ai.runtime_enabled = false;
        config.local_ai.selected_tier = Some("disabled".to_string());
        config.local_ai.opt_in_confirmed = false;
        config
            .save()
            .await
            .map_err(|e| format!("save config: {e}"))?;
        debug!("{LOG_PREFIX} apply_preset:disabled");
        return Ok(RpcOutcome::single_log(
            json!({
                "applied_tier": "disabled",
                "local_ai_enabled": false,
            }),
            "inference preset applied",
        ));
    }

    let tier = presets::ModelTier::from_str_opt(&tier_str).ok_or_else(|| {
        format!(
            "invalid tier '{}': expected one of disabled or ram_2_4gb",
            tier_str
        )
    })?;

    if tier == presets::ModelTier::Custom {
        return Err("cannot apply 'custom' tier; set model IDs directly".to_string());
    }
    if !tier.is_mvp_allowed() {
        return Err(format!(
            "tier '{}' is not available in this build; only the 1B local model preset is supported",
            tier_str
        ));
    }

    let mut config = config_rpc::load_config_with_timeout().await?;
    config.local_ai.runtime_enabled = true;
    config.local_ai.opt_in_confirmed = true;
    presets::apply_preset_to_config(&mut config.local_ai, tier);
    config
        .save()
        .await
        .map_err(|e| format!("save config: {e}"))?;

    debug!(tier = %tier_str, "{LOG_PREFIX} apply_preset:ok");
    Ok(RpcOutcome::single_log(
        json!({
            "applied_tier": tier,
            "chat_model_id": config.local_ai.chat_model_id,
            "vision_model_id": config.local_ai.vision_model_id,
            "embedding_model_id": config.local_ai.embedding_model_id,
            "quantization": config.local_ai.quantization,
            "vision_mode": presets::vision_mode_for_config(&config.local_ai),
            "local_ai_enabled": true,
        }),
        "inference preset applied",
    ))
}

pub async fn inference_openai_oauth_start(config: &Config) -> Result<RpcOutcome<Value>, String> {
    debug!("{LOG_PREFIX} openai_oauth_start:start");
    let result =
        crate::openhuman::inference::openai_oauth::start_openai_oauth(config).map(|start| {
            RpcOutcome::single_log(
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
        Err(err) => error!(error = %err, "{LOG_PREFIX} openai_oauth_start:error"),
    }
    result
}

pub async fn inference_openai_oauth_complete(
    config: &Config,
    callback_url: &str,
) -> Result<RpcOutcome<Value>, String> {
    debug!(
        callback_len = callback_url.len(),
        "{LOG_PREFIX} openai_oauth_complete:start"
    );
    let result =
        crate::openhuman::inference::openai_oauth::complete_openai_oauth(config, callback_url)
            .await
            .map(|payload| RpcOutcome::single_log(payload, "openai oauth connected"));
    match &result {
        Ok(_) => debug!("{LOG_PREFIX} openai_oauth_complete:ok"),
        Err(err) => error!(error = %err, "{LOG_PREFIX} openai_oauth_complete:error"),
    }
    result
}

pub async fn inference_openai_oauth_import_codex_cli(
    config: &Config,
) -> Result<RpcOutcome<Value>, String> {
    debug!("{LOG_PREFIX} openai_oauth_import_codex_cli:start");
    let result =
        crate::openhuman::inference::openai_oauth::import_openai_oauth_from_codex_cli(config)
            .map(|payload| RpcOutcome::single_log(payload, "openai oauth imported from codex cli"));
    match &result {
        Ok(_) => debug!("{LOG_PREFIX} openai_oauth_import_codex_cli:ok"),
        Err(err) => error!(error = %err, "{LOG_PREFIX} openai_oauth_import_codex_cli:error"),
    }
    result
}

pub async fn inference_openai_oauth_status(config: &Config) -> Result<RpcOutcome<Value>, String> {
    debug!("{LOG_PREFIX} openai_oauth_status:start");
    let result =
        crate::openhuman::inference::openai_oauth::openai_oauth_status(config).map(|status| {
            RpcOutcome::single_log(
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
        Err(err) => error!(error = %err, "{LOG_PREFIX} openai_oauth_status:error"),
    }
    result
}

pub async fn inference_openai_oauth_disconnect(
    config: &Config,
) -> Result<RpcOutcome<Value>, String> {
    debug!("{LOG_PREFIX} openai_oauth_disconnect:start");
    let result = crate::openhuman::inference::openai_oauth::disconnect_openai_oauth(config)
        .map(|payload| RpcOutcome::single_log(payload, "openai oauth disconnected"));
    match &result {
        Ok(_) => debug!("{LOG_PREFIX} openai_oauth_disconnect:ok"),
        Err(err) => error!(error = %err, "{LOG_PREFIX} openai_oauth_disconnect:error"),
    }
    result
}

pub async fn inference_diagnostics(config: &Config) -> Result<RpcOutcome<Value>, String> {
    debug!("{LOG_PREFIX} diagnostics:start");
    let service = local_runtime::global(config);
    // Return the diagnostics payload directly (no `{result, logs}` wrap) so
    // callers (UI + json_rpc_e2e tests) can read `provider`, `lm_studio_running`,
    // etc. straight off the response — mirrors the legacy
    // `local_ai_diagnostics` shape that the test asserts against.
    let result = service
        .diagnostics(config)
        .await
        .map(|value| RpcOutcome::new(value, Vec::new()));
    match &result {
        Ok(_) => debug!("{LOG_PREFIX} diagnostics:ok"),
        Err(err) => error!(error = %err, "{LOG_PREFIX} diagnostics:error"),
    }
    result
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
