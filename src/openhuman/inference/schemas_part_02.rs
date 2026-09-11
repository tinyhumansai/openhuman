
fn handle_inference_update_model_settings(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let update = deserialize_params::<InferenceUpdateModelSettingsParams>(params)?;
        let patch = config_rpc::ModelSettingsPatch {
            api_url: update.api_url,
            inference_url: update.inference_url,
            api_key: update.api_key,
            default_model: update.default_model,
            default_temperature: update.default_temperature,
            model_routes: update.model_routes.map(|routes| {
                routes
                    .into_iter()
                    .map(|route| crate::openhuman::config::ModelRouteConfig {
                        hint: route.hint,
                        model: route.model,
                    })
                    .collect()
            }),
            cloud_providers: update
                .cloud_providers
                .map(|entries| {
                    use crate::openhuman::config::schema::cloud_providers::{
                        generate_provider_id, is_slug_reserved, migrate_legacy_fields, AuthStyle,
                        CloudProviderCreds,
                    };
                    let reserved_count = entries
                        .iter()
                        .filter(|e| {
                            let t = e.slug.trim();
                            !t.is_empty() && is_slug_reserved(t)
                        })
                        .count();
                    if reserved_count > 0 {
                        log::debug!(
                            "[inference] update_model_settings: dropping {} reserved cloud provider slug(s)",
                            reserved_count
                        );
                    }
                    entries
                        .into_iter()
                        // Silently drop entries whose (non-empty) slug is reserved —
                        // typically the migration-seeded "openhuman" / "cloud" /
                        // "pid" built-ins that the frontend echoes back on every
                        // save (see `migrations::unify_ai_provider_settings`).
                        // Empty slugs still fall through so the explicit
                        // validation error below fires for actual frontend
                        // bugs. `apply_model_settings` re-injects the existing
                        // reserved entries from the stored config so they
                        // aren't dropped on save.
                        .filter(|entry| {
                            let trimmed = entry.slug.trim();
                            trimmed.is_empty() || !is_slug_reserved(trimmed)
                        })
                        .map(|entry| {
                            let slug = entry.slug.trim().to_string();
                            if slug.is_empty() {
                                return Err("cloud provider slug must not be empty".to_string());
                            }
                            let auth_style = match entry
                                .auth_style
                                .as_deref()
                                .unwrap_or("bearer")
                                .to_ascii_lowercase()
                                .as_str()
                            {
                                "bearer" => AuthStyle::Bearer,
                                "anthropic" => AuthStyle::Anthropic,
                                "openhuman_jwt" | "openhumanjwt" => AuthStyle::OpenhumanJwt,
                                "none" => AuthStyle::None,
                                other => {
                                    return Err(format!(
                                        "unknown auth_style '{}'; valid: bearer, anthropic, openhuman_jwt, none",
                                        other
                                    ))
                                }
                            };
                            let id = entry
                                .id
                                .filter(|s| !s.trim().is_empty())
                                .unwrap_or_else(|| generate_provider_id(&slug));
                            let label = entry
                                .label
                                .filter(|s| !s.trim().is_empty())
                                .unwrap_or_else(|| slug.clone());
                            let mut provider = CloudProviderCreds {
                                id,
                                slug,
                                label,
                                endpoint: entry.endpoint,
                                auth_style,
                                legacy_type: entry.legacy_type,
                                default_model: entry.default_model,
                            };
                            migrate_legacy_fields(&mut provider);
                            Ok(provider)
                        })
                        .collect::<Result<Vec<_>, String>>()
                })
                .transpose()?,
            model_registry: update.model_registry,
            primary_cloud: update.primary_cloud,
            chat_provider: update.chat_provider,
            reasoning_provider: update.reasoning_provider,
            agentic_provider: update.agentic_provider,
            coding_provider: update.coding_provider,
            vision_provider: update.vision_provider,
            memory_provider: update.memory_provider,
            embeddings_provider: update.embeddings_provider,
            heartbeat_provider: update.heartbeat_provider,
            learning_provider: update.learning_provider,
            subconscious_provider: update.subconscious_provider,
        };
        to_json(crate::openhuman::inference::rpc::inference_update_model_settings(patch).await?)
    })
}

fn handle_inference_update_local_settings(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let update = deserialize_params::<InferenceUpdateLocalSettingsParams>(params)?;
        let base_url = match update.base_url {
            None => None,
            Some(Value::Null) => Some(None),
            Some(Value::String(value)) => Some(Some(value)),
            Some(_) => return Err("invalid params: base_url must be a string or null".to_string()),
        };
        let patch = config_rpc::LocalAiSettingsPatch {
            runtime_enabled: update.runtime_enabled,
            opt_in_confirmed: update.opt_in_confirmed,
            provider: update.provider,
            base_url,
            model_id: update.model_id,
            chat_model_id: update.chat_model_id,
            usage_embeddings: update.usage_embeddings,
            usage_heartbeat: update.usage_heartbeat,
            usage_learning_reflection: update.usage_learning_reflection,
            usage_subconscious: update.usage_subconscious,
            api_key: update.api_key,
        };
        to_json(crate::openhuman::inference::rpc::inference_update_local_settings(patch).await?)
    })
}

fn handle_inference_list_models(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let request = deserialize_params::<InferenceListModelsParams>(params)?;
        to_json(
            crate::openhuman::inference::rpc::inference_list_models(&request.provider_id).await?,
        )
    })
}

fn handle_inference_device_profile(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(
        async move { to_json(crate::openhuman::inference::rpc::inference_device_profile().await?) },
    )
}

fn handle_inference_provider_auth_errors(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        to_json(crate::openhuman::inference::rpc::inference_provider_auth_errors().await?)
    })
}

fn handle_inference_presets(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { to_json(crate::openhuman::inference::rpc::inference_presets().await?) })
}

fn handle_inference_apply_preset(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let request = deserialize_params::<InferenceApplyPresetParams>(params)?;
        to_json(crate::openhuman::inference::rpc::inference_apply_preset(&request.tier).await?)
    })
}

fn handle_inference_openai_oauth_start(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(crate::openhuman::inference::rpc::inference_openai_oauth_start(&config).await?)
    })
}

fn handle_inference_openai_oauth_complete(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload = deserialize_params::<InferenceOpenAiOAuthCompleteParams>(params)?;
        to_json(
            crate::openhuman::inference::rpc::inference_openai_oauth_complete(
                &config,
                payload.callback_url.trim(),
            )
            .await?,
        )
    })
}

fn handle_inference_openai_oauth_import_codex_cli(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(
            crate::openhuman::inference::rpc::inference_openai_oauth_import_codex_cli(&config)
                .await?,
        )
    })
}

fn handle_inference_openai_oauth_status(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(crate::openhuman::inference::rpc::inference_openai_oauth_status(&config).await?)
    })
}

fn handle_inference_openai_oauth_disconnect(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(crate::openhuman::inference::rpc::inference_openai_oauth_disconnect(&config).await?)
    })
}

fn handle_inference_diagnostics(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(crate::openhuman::inference::rpc::inference_diagnostics(&config).await?)
    })
}

fn handle_inference_summarize(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<InferenceSummarizeParams>(params)?;
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(
            crate::openhuman::inference::rpc::inference_summarize(&config, &p.text, p.max_tokens)
                .await?,
        )
    })
}

fn handle_inference_prompt(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<InferencePromptParams>(params)?;
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(
            crate::openhuman::inference::rpc::inference_prompt(
                &config,
                &p.prompt,
                p.max_tokens,
                p.no_think,
            )
            .await?,
        )
    })
}

fn handle_inference_vision_prompt(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<InferenceVisionPromptParams>(params)?;
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(
            crate::openhuman::inference::rpc::inference_vision_prompt(
                &config,
                &p.prompt,
                &p.image_refs,
                p.max_tokens,
            )
            .await?,
        )
    })
}

fn handle_inference_test_provider_model(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<InferenceTestChatModelParams>(params)?;
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(
            crate::openhuman::inference::rpc::inference_test_provider_model(
                &config,
                &p.workload,
                &p.provider,
                p.prompt.as_deref().unwrap_or("Hello world"),
            )
            .await?,
        )
    })
}

fn handle_inference_should_react(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<InferenceShouldReactParams>(params)?;
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(
            crate::openhuman::inference::rpc::inference_should_react(
                &config,
                &p.message,
                &p.channel_type,
            )
            .await?,
        )
    })
}

fn handle_inference_analyze_sentiment(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<InferenceAnalyzeSentimentParams>(params)?;
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(
            crate::openhuman::inference::rpc::inference_analyze_sentiment(&config, &p.message)
                .await?,
        )
    })
}

fn handle_inference_claude_code_status(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let status = tokio::task::spawn_blocking(
            crate::openhuman::inference::provider::claude_code::version_check::probe,
        )
        .await
        .map_err(|e| format!("claude_code_status join error: {e}"))?;
        to_json(RpcOutcome::new(status, vec![]))
    })
}

fn handle_inference_claude_code_auth_status(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let auth = tokio::task::spawn_blocking(
            crate::openhuman::inference::provider::claude_code::auth_status::probe,
        )
        .await
        .map_err(|e| format!("claude_code_auth_status join error: {e}"))?;
        to_json(RpcOutcome::new(auth, vec![]))
    })
}

fn handle_inference_claude_code_settings(_params: Map<String, Value>) -> ControllerFuture {
    use crate::openhuman::inference::provider::claude_code::settings;
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let settings = settings::load_for_config(&config);
        log::debug!(
            "[rpc][inference.claude_code_settings] full_access={}",
            settings.full_access
        );
        to_json(RpcOutcome::new(settings, vec![]))
    })
}

fn handle_inference_claude_code_set_full_access(params: Map<String, Value>) -> ControllerFuture {
    use crate::openhuman::inference::provider::claude_code::settings;
    Box::pin(async move {
        let p = deserialize_params::<InferenceClaudeCodeSetFullAccessParams>(params)?;
        let config = config_rpc::load_config_with_timeout().await?;
        let settings = settings::save_full_access_for_config(&config, p.enabled)
            .map_err(|e| format!("failed to persist claude code settings: {e}"))?;
        log::info!(
            "[rpc][inference.claude_code_set_full_access] persisted full_access={}",
            settings.full_access
        );
        to_json(RpcOutcome::new(settings, vec![]))
    })
}

fn deserialize_params<T: DeserializeOwned>(params: Map<String, Value>) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

fn deserialize_present_json<'de, D>(deserializer: D) -> Result<Option<Value>, D::Error>
where
    D: Deserializer<'de>,
{
    Value::deserialize(deserializer).map(Some)
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}
