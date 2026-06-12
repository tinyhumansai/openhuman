use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::ControllerSchema;
use crate::openhuman::config::rpc as config_rpc;

use super::helpers::{
    deserialize_params, to_json, ActivityLevelSettingsUpdate, AgentPathsUpdate,
    AgentSettingsUpdate, AnalyticsSettingsUpdate, AutonomySettingsUpdate, BrowserSettingsUpdate,
    ComposioTriggerSettingsUpdate, DictationSettingsUpdate, LocalAiSettingsUpdate,
    MeetSettingsUpdate, MemorySettingsUpdate, MemorySyncSettingsUpdate, ModelSettingsUpdate,
    OnboardingCompletedSetParams, RuntimeSettingsUpdate, SandboxSettingsUpdate,
    ScreenIntelligenceSettingsUpdate, SearchSettingsUpdate, SetBrowserAllowAllParams,
    VoiceServerSettingsUpdate, WorkspaceOnboardingFlagParams, WorkspaceOnboardingFlagSetParams,
    DEFAULT_ONBOARDING_FLAG_NAME,
};
use super::schema_defs::schemas;

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("get_config"),
        schemas("get_client_config"),
        schemas("update_model_settings"),
        schemas("update_memory_settings"),
        schemas("update_screen_intelligence_settings"),
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
        schemas("update_meet_settings"),
        schemas("get_meet_settings"),
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
            schema: schemas("update_screen_intelligence_settings"),
            handler: handle_update_screen_intelligence_settings,
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
            schema: schemas("update_meet_settings"),
            handler: handle_update_meet_settings,
        },
        RegisteredController {
            schema: schemas("get_meet_settings"),
            handler: handle_get_meet_settings,
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

fn handle_update_screen_intelligence_settings(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let update = deserialize_params::<ScreenIntelligenceSettingsUpdate>(params)?;
        let patch = config_rpc::ScreenIntelligenceSettingsPatch {
            enabled: update.enabled,
            capture_policy: update.capture_policy,
            policy_mode: update.policy_mode,
            baseline_fps: update.baseline_fps,
            vision_enabled: update.vision_enabled,
            autocomplete_enabled: update.autocomplete_enabled,
            use_vision_model: update.use_vision_model,
            keep_screenshots: update.keep_screenshots,
            allowlist: update.allowlist,
            denylist: update.denylist,
        };
        to_json(config_rpc::load_and_apply_screen_intelligence_settings(patch).await?)
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
        };
        to_json(config_rpc::load_and_apply_autonomy_settings(patch).await?)
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

fn handle_update_meet_settings(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        log::debug!("[config][rpc] update_meet_settings enter");
        let update = match deserialize_params::<MeetSettingsUpdate>(params) {
            Ok(u) => u,
            Err(err) => {
                log::warn!("[config][rpc] update_meet_settings invalid params: {err}");
                return Err(err);
            }
        };
        log::debug!(
            "[config][rpc] update_meet_settings patch auto_orchestrator_handoff={:?}",
            update.auto_orchestrator_handoff
        );
        let patch = config_rpc::MeetSettingsPatch {
            auto_orchestrator_handoff: update.auto_orchestrator_handoff,
        };
        match config_rpc::load_and_apply_meet_settings(patch).await {
            Ok(outcome) => {
                log::debug!("[config][rpc] update_meet_settings ok");
                to_json(outcome)
            }
            Err(err) => {
                log::warn!("[config][rpc] update_meet_settings failed: {err}");
                Err(err)
            }
        }
    })
}

fn handle_get_meet_settings(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async {
        use crate::rpc::RpcOutcome;
        log::debug!("[config][rpc] get_meet_settings enter");
        let config = match config_rpc::load_config_with_timeout().await {
            Ok(c) => c,
            Err(err) => {
                log::warn!("[config][rpc] get_meet_settings load failed: {err}");
                return Err(err);
            }
        };
        let auto_orchestrator_handoff = config.meet.auto_orchestrator_handoff;
        log::debug!(
            "[config][rpc] get_meet_settings ok auto_orchestrator_handoff={auto_orchestrator_handoff}"
        );
        let result = serde_json::json!({
            "auto_orchestrator_handoff": auto_orchestrator_handoff,
        });
        to_json(RpcOutcome::new(
            result,
            vec!["meet settings read".to_string()],
        ))
    })
}

fn handle_agent_server_status(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async { to_json(config_rpc::agent_server_status()) })
}

fn handle_reset_local_data(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async { to_json(config_rpc::reset_local_data().await?) })
}

fn handle_get_data_paths(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async {
        log::debug!("[config][rpc] get_data_paths enter");
        match config_rpc::get_data_paths().await {
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

fn handle_get_onboarding_completed(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async { to_json(config_rpc::get_onboarding_completed().await?) })
}

fn handle_get_dictation_settings(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async { to_json(config_rpc::get_dictation_settings().await?) })
}

fn handle_update_dictation_settings(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let update = deserialize_params::<DictationSettingsUpdate>(params)?;
        let patch = config_rpc::DictationSettingsPatch {
            enabled: update.enabled,
            hotkey: update.hotkey,
            activation_mode: update.activation_mode,
            llm_refinement: update.llm_refinement,
            streaming: update.streaming,
            streaming_interval_ms: update.streaming_interval_ms,
        };
        to_json(config_rpc::load_and_apply_dictation_settings(patch).await?)
    })
}

fn handle_get_voice_server_settings(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async { to_json(config_rpc::get_voice_server_settings().await?) })
}

fn handle_update_voice_server_settings(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let update = deserialize_params::<VoiceServerSettingsUpdate>(params)?;
        let patch = config_rpc::VoiceServerSettingsPatch {
            auto_start: update.auto_start,
            hotkey: update.hotkey,
            activation_mode: update.activation_mode,
            skip_cleanup: update.skip_cleanup,
            min_duration_secs: update.min_duration_secs,
            silence_threshold: update.silence_threshold,
            custom_dictionary: update.custom_dictionary,
            always_on_enabled: update.always_on_enabled,
            wake_word: update.wake_word,
        };
        let result = config_rpc::load_and_apply_voice_server_settings(patch).await?;
        // Apply the always-on toggle live (start/idle the capture loop) so the
        // Settings switch takes effect without a restart. Don't fail the RPC if
        // the reload hiccups, but DO surface it — otherwise the saved setting
        // silently wouldn't apply until the next launch.
        match config_rpc::load_config_with_timeout().await {
            Ok(config) => {
                log::debug!("[config][rpc] voice settings saved; applying live always-on state");
                crate::openhuman::voice::always_on::start_if_enabled(&config).await;
            }
            Err(error) => {
                log::warn!(
                    "[config][rpc] voice settings saved, but live always-on apply was skipped \
                     (config reload failed): {error}"
                );
            }
        }
        to_json(result)
    })
}

fn handle_set_onboarding_completed(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<OnboardingCompletedSetParams>(params)?;
        to_json(config_rpc::set_onboarding_completed(payload.value).await?)
    })
}

fn handle_update_composio_trigger_settings(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        log::debug!("[config][rpc] update_composio_trigger_settings enter");
        let update = match deserialize_params::<ComposioTriggerSettingsUpdate>(params) {
            Ok(u) => u,
            Err(err) => {
                log::warn!("[config][rpc] update_composio_trigger_settings invalid params: {err}");
                return Err(err);
            }
        };
        let patch = config_rpc::ComposioTriggerSettingsPatch {
            triage_disabled: update.triage_disabled,
            triage_disabled_toolkits: update.triage_disabled_toolkits,
        };
        match config_rpc::load_and_apply_composio_trigger_settings(patch).await {
            Ok(outcome) => {
                log::debug!("[config][rpc] update_composio_trigger_settings ok");
                to_json(outcome)
            }
            Err(err) => {
                log::warn!("[config][rpc] update_composio_trigger_settings failed: {err}");
                Err(err)
            }
        }
    })
}

fn handle_get_composio_trigger_settings(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async {
        log::debug!("[config][rpc] get_composio_trigger_settings enter");
        match config_rpc::get_composio_trigger_settings().await {
            Ok(outcome) => {
                log::debug!("[config][rpc] get_composio_trigger_settings ok");
                to_json(outcome)
            }
            Err(err) => {
                log::warn!("[config][rpc] get_composio_trigger_settings failed: {err}");
                Err(err)
            }
        }
    })
}

fn handle_update_search_settings(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        log::debug!("[config][rpc] update_search_settings enter");
        let update = match deserialize_params::<SearchSettingsUpdate>(params) {
            Ok(u) => u,
            Err(err) => {
                log::warn!("[config][rpc] update_search_settings invalid params: {err}");
                return Err(err);
            }
        };
        let patch = config_rpc::SearchSettingsPatch {
            engine: update.engine,
            max_results: update.max_results,
            timeout_secs: update.timeout_secs,
            parallel_api_key: update.parallel_api_key,
            brave_api_key: update.brave_api_key,
            querit_api_key: update.querit_api_key,
            allowed_domains: update.allowed_domains,
            allow_all: update.allow_all,
        };
        match config_rpc::load_and_apply_search_settings(patch).await {
            Ok(outcome) => {
                log::debug!("[config][rpc] update_search_settings ok");
                to_json(outcome)
            }
            Err(err) => {
                log::warn!("[config][rpc] update_search_settings failed: {err}");
                Err(err)
            }
        }
    })
}

fn handle_get_search_settings(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async {
        log::debug!("[config][rpc] get_search_settings enter");
        match config_rpc::get_search_settings().await {
            Ok(outcome) => {
                log::debug!("[config][rpc] get_search_settings ok");
                to_json(outcome)
            }
            Err(err) => {
                log::warn!("[config][rpc] get_search_settings failed: {err}");
                Err(err)
            }
        }
    })
}

fn handle_get_activity_level_settings(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { to_json(config_rpc::get_activity_level_settings().await?) })
}

fn handle_update_activity_level_settings(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let update = deserialize_params::<ActivityLevelSettingsUpdate>(params)?;
        let patch = config_rpc::ActivityLevelSettingsPatch {
            level: update.level,
        };
        to_json(config_rpc::load_and_apply_activity_level_settings(patch).await?)
    })
}

fn handle_get_memory_sync_settings(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { to_json(config_rpc::get_memory_sync_settings().await?) })
}

fn handle_update_memory_sync_settings(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let update = deserialize_params::<MemorySyncSettingsUpdate>(params)?;
        let patch = config_rpc::MemorySyncSettingsPatch {
            sync_interval_secs: update.sync_interval_secs,
        };
        to_json(config_rpc::load_and_apply_memory_sync_settings(patch).await?)
    })
}

fn handle_get_sandbox_settings(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { to_json(config_rpc::get_sandbox_settings().await?) })
}

fn handle_update_sandbox_settings(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let update = deserialize_params::<SandboxSettingsUpdate>(params)?;
        let patch = config_rpc::SandboxSettingsPatch {
            backend: update.backend,
            enabled: update.enabled,
            docker_image: update.docker_image,
            docker_memory_limit_mb: update.docker_memory_limit_mb,
            docker_cpu_limit: update.docker_cpu_limit,
            env_passthrough: update.env_passthrough,
        };
        to_json(config_rpc::load_and_apply_sandbox_settings(patch).await?)
    })
}
