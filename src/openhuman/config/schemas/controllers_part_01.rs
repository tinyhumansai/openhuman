use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::ControllerSchema;
use crate::openhuman::config::rpc as config_rpc;

use super::helpers::{
    deserialize_params, to_json, ActivityLevelSettingsUpdate, AgentPathsUpdate,
    AgentSettingsUpdate, AnalyticsSettingsUpdate, AutonomySettingsUpdate, BrowserSettingsUpdate,
    ComposioTriggerSettingsUpdate, DictationSettingsUpdate, LocalAiSettingsUpdate,
    MemorySettingsUpdate, MemorySyncSettingsUpdate, ModelSettingsUpdate,
    OnboardingCompletedSetParams, PrivacyModeUpdate, RuntimeSettingsUpdate, SandboxSettingsUpdate,
    SearchSettingsUpdate, SetBrowserAllowAllParams, VoiceServerSettingsUpdate,
    WorkspaceOnboardingFlagParams, WorkspaceOnboardingFlagSetParams, DEFAULT_ONBOARDING_FLAG_NAME,
};
use super::schema_defs::schemas;

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("get_config"),
        schemas("get_client_config"),
        schemas("update_model_settings"),
        schemas("update_memory_settings"),
        schemas("update_runtime_settings"),
        schemas("update_browser_settings"),
        schemas("update_local_ai_settings"),
        schemas("resolve_api_url"),
        schemas("get_runtime_flags"),
        schemas("set_browser_allow_all"),
        schemas("workspace_onboarding_flag_exists"),
        schemas("workspace_onboarding_flag_set"),
        schemas("update_analytics_settings"),
        schemas("get_analytics_settings"),
        schemas("get_dashboard_settings"),
        schemas("agent_server_status"),
        schemas("reset_local_data"),
        schemas("get_data_paths"),
        schemas("get_agent_paths"),
        schemas("update_agent_paths"),
        schemas("get_onboarding_completed"),
        schemas("set_onboarding_completed"),
        schemas("get_dictation_settings"),
        schemas("update_dictation_settings"),
        schemas("get_voice_server_settings"),
        schemas("update_voice_server_settings"),
        schemas("update_composio_trigger_settings"),
        schemas("get_composio_trigger_settings"),
        schemas("get_autonomy_settings"),
        schemas("update_autonomy_settings"),
        schemas("get_privacy_mode"),
        schemas("set_privacy_mode"),
        schemas("get_agent_settings"),
        schemas("update_agent_settings"),
        schemas("update_search_settings"),
        schemas("get_search_settings"),
        schemas("get_activity_level_settings"),
        schemas("update_activity_level_settings"),
        schemas("get_memory_sync_settings"),
        schemas("update_memory_sync_settings"),
        schemas("get_sandbox_settings"),
        schemas("update_sandbox_settings"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("get_config"),
            handler: handle_get_config,
        },
        RegisteredController {
            schema: schemas("get_client_config"),
            handler: handle_get_client_config,
        },
        RegisteredController {
            schema: schemas("update_model_settings"),
            handler: handle_update_model_settings,
        },
        RegisteredController {
            schema: schemas("update_memory_settings"),
            handler: handle_update_memory_settings,
        },
        RegisteredController {
            schema: schemas("update_runtime_settings"),
            handler: handle_update_runtime_settings,
        },
        RegisteredController {
            schema: schemas("update_browser_settings"),
            handler: handle_update_browser_settings,
        },
        RegisteredController {
            schema: schemas("update_local_ai_settings"),
            handler: handle_update_local_ai_settings,
        },
        RegisteredController {
            schema: schemas("resolve_api_url"),
            handler: handle_resolve_api_url,
        },
        RegisteredController {
            schema: schemas("get_runtime_flags"),
            handler: handle_get_runtime_flags,
        },
        RegisteredController {
            schema: schemas("set_browser_allow_all"),
            handler: handle_set_browser_allow_all,
        },
        RegisteredController {
            schema: schemas("workspace_onboarding_flag_exists"),
            handler: handle_workspace_onboarding_flag_exists,
        },
        RegisteredController {
            schema: schemas("workspace_onboarding_flag_set"),
            handler: handle_workspace_onboarding_flag_set,
        },
        RegisteredController {
            schema: schemas("update_analytics_settings"),
            handler: handle_update_analytics_settings,
        },
        RegisteredController {
            schema: schemas("get_analytics_settings"),
            handler: handle_get_analytics_settings,
        },
        RegisteredController {
            schema: schemas("get_dashboard_settings"),
            handler: handle_get_dashboard_settings,
        },
        RegisteredController {
            schema: schemas("agent_server_status"),
            handler: handle_agent_server_status,
        },
        RegisteredController {
            schema: schemas("reset_local_data"),
            handler: handle_reset_local_data,
        },
        RegisteredController {
            schema: schemas("get_data_paths"),
            handler: handle_get_data_paths,
        },
        RegisteredController {
            schema: schemas("get_agent_paths"),
            handler: handle_get_agent_paths,
        },
        RegisteredController {
            schema: schemas("update_agent_paths"),
            handler: handle_update_agent_paths,
        },
        RegisteredController {
            schema: schemas("get_onboarding_completed"),
            handler: handle_get_onboarding_completed,
        },
        RegisteredController {
            schema: schemas("set_onboarding_completed"),
            handler: handle_set_onboarding_completed,
        },
        RegisteredController {
            schema: schemas("get_dictation_settings"),
            handler: handle_get_dictation_settings,
        },
        RegisteredController {
            schema: schemas("update_dictation_settings"),
            handler: handle_update_dictation_settings,
        },
        RegisteredController {
            schema: schemas("get_voice_server_settings"),
            handler: handle_get_voice_server_settings,
        },
        RegisteredController {
            schema: schemas("update_voice_server_settings"),
            handler: handle_update_voice_server_settings,
        },
        RegisteredController {
            schema: schemas("update_composio_trigger_settings"),
            handler: handle_update_composio_trigger_settings,
        },
        RegisteredController {
            schema: schemas("get_composio_trigger_settings"),
            handler: handle_get_composio_trigger_settings,
        },
        RegisteredController {
            schema: schemas("get_autonomy_settings"),
            handler: handle_get_autonomy_settings,
        },
        RegisteredController {
            schema: schemas("update_autonomy_settings"),
            handler: handle_update_autonomy_settings,
        },
        RegisteredController {
            schema: schemas("get_privacy_mode"),
            handler: handle_get_privacy_mode,
        },
        RegisteredController {
            schema: schemas("set_privacy_mode"),
            handler: handle_set_privacy_mode,
        },
        RegisteredController {
            schema: schemas("get_agent_settings"),
            handler: handle_get_agent_settings,
        },
        RegisteredController {
            schema: schemas("update_agent_settings"),
            handler: handle_update_agent_settings,
        },
        RegisteredController {
            schema: schemas("update_search_settings"),
            handler: handle_update_search_settings,
        },
        RegisteredController {
            schema: schemas("get_search_settings"),
            handler: handle_get_search_settings,
        },
        RegisteredController {
            schema: schemas("get_activity_level_settings"),
            handler: handle_get_activity_level_settings,
        },
        RegisteredController {
            schema: schemas("update_activity_level_settings"),
            handler: handle_update_activity_level_settings,
        },
        RegisteredController {
            schema: schemas("get_memory_sync_settings"),
            handler: handle_get_memory_sync_settings,
        },
        RegisteredController {
            schema: schemas("update_memory_sync_settings"),
            handler: handle_update_memory_sync_settings,
        },
        RegisteredController {
            schema: schemas("get_sandbox_settings"),
            handler: handle_get_sandbox_settings,
        },
        RegisteredController {
            schema: schemas("update_sandbox_settings"),
            handler: handle_update_sandbox_settings,
        },
    ]
}

fn handle_get_config(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async { to_json(config_rpc::load_and_get_config_snapshot().await?) })
}

fn handle_get_client_config(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        log::debug!("[config][rpc] get_client_config enter");
        match config_rpc::load_and_get_client_config_snapshot().await {
            Ok(snapshot) => to_json(snapshot),
            Err(err) => {
                log::warn!("[config][rpc] get_client_config load failed: {err}");
                Err(err)
            }
        }
    })
}

fn handle_update_model_settings(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let update = deserialize_params::<ModelSettingsUpdate>(params)?;
        let patch = config_rpc::ModelSettingsPatch {
            api_url: update.api_url,
            inference_url: update.inference_url,
            api_key: update.api_key,
            default_model: update.default_model,
            default_temperature: update.default_temperature,
            model_routes: update.model_routes.map(|routes| {
                routes
                    .into_iter()
                    .map(|r| crate::openhuman::config::ModelRouteConfig {
                        hint: r.hint,
                        model: r.model,
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
                            "[config] update_model_settings: dropping {} reserved cloud provider slug(s)",
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
                        .filter(|e| {
                            let trimmed = e.slug.trim();
                            trimmed.is_empty() || !is_slug_reserved(trimmed)
                        })
                        .map(|e| {
                            let slug = e.slug.trim().to_string();
                            if slug.is_empty() {
                                return Err(
                                    "cloud provider slug must not be empty".to_string()
                                );
                            }
                            let auth_style = match e
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
                            let id = e
                                .id
                                .filter(|s| !s.trim().is_empty())
                                .unwrap_or_else(|| generate_provider_id(&slug));
                            let label = e
                                .label
                                .filter(|s| !s.trim().is_empty())
                                .unwrap_or_else(|| slug.clone());
                            let mut entry = CloudProviderCreds {
                                id,
                                slug,
                                label,
                                endpoint: e.endpoint,
                                auth_style,
                                legacy_type: e.legacy_type,
                                default_model: e.default_model,
                            };
                            // Apply any remaining legacy-field migration.
                            migrate_legacy_fields(&mut entry);
                            Ok(entry)
                        })
                        .collect::<Result<Vec<_>, String>>()
                })
                .transpose()?,
            // The config-domain RPC doesn't carry a model-registry payload — the
            // per-model vision registry is updated via the inference-domain
            // `inference_update_model_settings` path.
            model_registry: None,
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
        to_json(config_rpc::load_and_apply_model_settings(patch).await?)
    })
}

fn handle_update_memory_settings(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let update = deserialize_params::<MemorySettingsUpdate>(params)?;
        let patch = config_rpc::MemorySettingsPatch {
            backend: update.backend,
            auto_save: update.auto_save,
            embedding_provider: update.embedding_provider,
            embedding_model: update.embedding_model,
            embedding_dimensions: update.embedding_dimensions,
            memory_window: update.memory_window,
        };
        to_json(config_rpc::load_and_apply_memory_settings(patch).await?)
    })
}

fn handle_update_runtime_settings(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let update = deserialize_params::<RuntimeSettingsUpdate>(params)?;
        let patch = config_rpc::RuntimeSettingsPatch {
            kind: update.kind,
            reasoning_enabled: update.reasoning_enabled,
        };
        to_json(config_rpc::load_and_apply_runtime_settings(patch).await?)
    })
}

pub(super) fn handle_get_autonomy_settings(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { to_json(config_rpc::get_autonomy_settings().await?) })
}

pub(super) fn handle_update_autonomy_settings(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let update = deserialize_params::<AutonomySettingsUpdate>(params)?;
        let patch = config_rpc::AutonomySettingsPatch {
            level: update.level,
            workspace_only: update.workspace_only,
            allowed_commands: update.allowed_commands,
            forbidden_paths: update.forbidden_paths,
            trusted_roots: update.trusted_roots,
            allow_tool_install: update.allow_tool_install,
            max_actions_per_hour: update
                .max_actions_per_hour
                .map(|v| u32::try_from(v).unwrap_or(u32::MAX)),
            auto_approve: update.auto_approve,
            require_task_plan_approval: update.require_task_plan_approval,
            auto_approve_all: update.auto_approve_all,
        };
        to_json(config_rpc::load_and_apply_autonomy_settings(patch).await?)
    })
}

pub(super) fn handle_get_privacy_mode(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { to_json(config_rpc::get_privacy_mode().await?) })
}

pub(super) fn handle_set_privacy_mode(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let update = deserialize_params::<PrivacyModeUpdate>(params)?;
        let patch = config_rpc::PrivacySettingsPatch { mode: update.mode };
        to_json(config_rpc::load_and_apply_privacy_settings(patch).await?)
    })
}

fn handle_get_agent_settings(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async {
        log::debug!("[config][rpc] get_agent_settings enter");
        match config_rpc::get_agent_settings().await {
            Ok(outcome) => {
                log::debug!("[config][rpc] get_agent_settings ok");
                to_json(outcome)
            }
            Err(err) => {
                log::warn!("[config][rpc] get_agent_settings failed: {err}");
                Err(err)
            }
        }
    })
}

fn handle_update_agent_settings(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        log::debug!("[config][rpc] update_agent_settings enter");
        let update = match deserialize_params::<AgentSettingsUpdate>(params) {
            Ok(u) => u,
            Err(err) => {
                log::warn!("[config][rpc] update_agent_settings invalid params: {err}");
                return Err(err);
            }
        };
        let patch = config_rpc::AgentSettingsPatch {
            agent_timeout_secs: update.agent_timeout_secs,
        };
        match config_rpc::load_and_apply_agent_settings(patch).await {
            Ok(outcome) => {
                log::debug!("[config][rpc] update_agent_settings ok");
                to_json(outcome)
            }
            Err(err) => {
                log::warn!("[config][rpc] update_agent_settings failed: {err}");
                Err(err)
            }
        }
    })
}

fn handle_update_browser_settings(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let update = deserialize_params::<BrowserSettingsUpdate>(params)?;
        let patch = config_rpc::BrowserSettingsPatch {
            enabled: update.enabled,
            backend: update.backend,
        };
        to_json(config_rpc::load_and_apply_browser_settings(patch).await?)
    })
}

fn handle_update_local_ai_settings(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let update = deserialize_params::<LocalAiSettingsUpdate>(params)?;
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
        to_json(config_rpc::load_and_apply_local_ai_settings(patch).await?)
    })
}

fn handle_get_runtime_flags(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async { to_json(config_rpc::get_runtime_flags()) })
}

fn handle_resolve_api_url(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async { to_json(config_rpc::load_and_resolve_api_url().await?) })
}

fn handle_set_browser_allow_all(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<SetBrowserAllowAllParams>(params)?;
        to_json(config_rpc::set_browser_allow_all(payload.enabled)?)
    })
}

fn handle_workspace_onboarding_flag_exists(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<WorkspaceOnboardingFlagParams>(params)?;
        to_json(
            config_rpc::workspace_onboarding_flag_resolve(
                payload.flag_name,
                DEFAULT_ONBOARDING_FLAG_NAME,
            )
            .await?,
        )
    })
}

fn handle_workspace_onboarding_flag_set(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<WorkspaceOnboardingFlagSetParams>(params)?;
        to_json(
            config_rpc::workspace_onboarding_flag_set(
                payload.flag_name,
                DEFAULT_ONBOARDING_FLAG_NAME,
                payload.value,
            )
            .await?,
        )
    })
}

fn handle_update_analytics_settings(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let update = deserialize_params::<AnalyticsSettingsUpdate>(params)?;
        let patch = config_rpc::AnalyticsSettingsPatch {
            enabled: update.enabled,
        };
        to_json(config_rpc::load_and_apply_analytics_settings(patch).await?)
    })
}

fn handle_get_analytics_settings(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async {
        use crate::rpc::RpcOutcome;
        let config = config_rpc::load_config_with_timeout().await?;
        let result = serde_json::json!({
            "enabled": config.observability.analytics_enabled,
        });
        to_json(RpcOutcome::new(
            result,
            vec!["analytics settings read".to_string()],
        ))
    })
}

fn handle_get_dashboard_settings(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async { to_json(config_rpc::get_dashboard_settings().await?) })
}

fn handle_agent_server_status(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async { to_json(config_rpc::agent_server_status()) })
}

fn handle_reset_local_data(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async { to_json(config_rpc::reset_local_data().await?) })
}

fn handle_get_data_paths(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        log::debug!("[config][rpc] get_data_paths enter");
        match resolve_data_paths(params).await {
            Ok(outcome) => {
                log::debug!("[config][rpc] get_data_paths ok");
                to_json(outcome)
            }
            Err(err) => {
                log::warn!("[config][rpc] get_data_paths fail: {err}");
                Err(err)
            }
        }
    })
}

/// Resolve the data paths for `get_data_paths`, honoring an optional `user_id`
/// param. The Clear App Data flow passes the signed-in id (#4950) because it
/// removes the active-user marker *before* the reset resolves paths — without
/// the explicit id the core would fall back to the pre-login `users/local` dir
/// and leave the real user's data behind. Absent/blank → marker-based
/// resolution (the default used by the agent tool and diagnostics).
async fn resolve_data_paths(
    params: Map<String, Value>,
) -> Result<crate::rpc::RpcOutcome<Value>, String> {
    let user_id = params
        .get("user_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(|id| id.to_string());
    log::debug!(
        "[config][rpc] get_data_paths: explicit_user_id={}",
        user_id.is_some()
    );
    match user_id.as_deref() {
        Some(id) => config_rpc::get_data_paths_for_user(id).await,
        None => config_rpc::get_data_paths().await,
    }
}

pub(super) fn handle_get_agent_paths(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async {
        log::debug!("[config][rpc] get_agent_paths enter");
        match config_rpc::get_agent_paths().await {
            Ok(outcome) => {
                log::debug!("[config][rpc] get_agent_paths ok");
                to_json(outcome)
            }
            Err(err) => {
                log::warn!("[config][rpc] get_agent_paths fail: {err}");
                Err(err)
            }
        }
    })
}

fn handle_update_agent_paths(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        log::debug!("[config][rpc] update_agent_paths enter");
        let update = match deserialize_params::<AgentPathsUpdate>(params) {
            Ok(u) => u,
            Err(err) => {
                log::warn!("[config][rpc] update_agent_paths invalid params: {err}");
                return Err(err);
            }
        };
        let patch = config_rpc::AgentPathsPatch {
            action_dir: update.action_dir,
        };
        match config_rpc::load_and_apply_agent_paths_settings(patch).await {
            Ok(outcome) => {
                log::debug!("[config][rpc] update_agent_paths ok");
                to_json(outcome)
            }
            Err(err) => {
                log::warn!("[config][rpc] update_agent_paths failed: {err}");
                Err(err)
            }
        }
    })
}
