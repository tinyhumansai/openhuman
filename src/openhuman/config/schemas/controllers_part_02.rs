
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
            stt_engine: update.stt_engine,
        };
        let result = config_rpc::load_and_apply_voice_server_settings(patch).await?;
        // Apply the always-on toggle live (start/idle the capture loop) so the
        // Settings switch takes effect without a restart. Don't fail the RPC if
        // the reload hiccups, but DO surface it — otherwise the saved setting
        // silently wouldn't apply until the next launch.
        match config_rpc::load_config_with_timeout().await {
            Ok(config) => {
                log::info!("[config][rpc] voice settings saved; applying live always-on state");
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
            exa_api_key: update.exa_api_key,
            tavily_api_key: update.tavily_api_key,
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
