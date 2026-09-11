
impl Config {
    fn apply_learning_env<E: super::env::EnvLookup + ?Sized>(&mut self, env: &E) {
        if let Some(flag) = env.get("OPENHUMAN_LEARNING_ENABLED") {
            let normalized = flag.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "1" | "true" | "yes" | "on" => self.learning.enabled = true,
                "0" | "false" | "no" | "off" => self.learning.enabled = false,
                _ => {}
            }
        }
        if let Some(flag) = env.get("OPENHUMAN_LEARNING_REFLECTION_ENABLED") {
            let normalized = flag.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "1" | "true" | "yes" | "on" => self.learning.reflection_enabled = true,
                "0" | "false" | "no" | "off" => self.learning.reflection_enabled = false,
                _ => {}
            }
        }
        if let Some(flag) = env.get("OPENHUMAN_LEARNING_USER_PROFILE_ENABLED") {
            let normalized = flag.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "1" | "true" | "yes" | "on" => self.learning.user_profile_enabled = true,
                "0" | "false" | "no" | "off" => self.learning.user_profile_enabled = false,
                _ => {}
            }
        }
        if let Some(flag) = env.get("OPENHUMAN_LEARNING_TOOL_TRACKING_ENABLED") {
            let normalized = flag.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "1" | "true" | "yes" | "on" => self.learning.tool_tracking_enabled = true,
                "0" | "false" | "no" | "off" => self.learning.tool_tracking_enabled = false,
                _ => {}
            }
        }
        if let Some(flag) = env.get("OPENHUMAN_LEARNING_TOOL_MEMORY_CAPTURE_ENABLED") {
            if let Some(enabled) = parse_env_bool(
                "OPENHUMAN_LEARNING_TOOL_MEMORY_CAPTURE_ENABLED",
                flag.as_str(),
            ) {
                self.learning.tool_memory_capture_enabled = enabled;
            }
        }
        if let Some(flag) = env.get("OPENHUMAN_LEARNING_EXPLICIT_PREFERENCES_ENABLED") {
            let normalized = flag.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "1" | "true" | "yes" | "on" => self.learning.explicit_preferences_enabled = true,
                "0" | "false" | "no" | "off" => self.learning.explicit_preferences_enabled = false,
                _ => {}
            }
        }
        if let Some(flag) = env.get("OPENHUMAN_LEARNING_GOALS_ENRICHMENT_ENABLED") {
            let normalized = flag.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "1" | "true" | "yes" | "on" => self.learning.goals_enrichment_enabled = true,
                "0" | "false" | "no" | "off" => self.learning.goals_enrichment_enabled = false,
                _ => {}
            }
        }
        if let Some(source) = env.get("OPENHUMAN_LEARNING_REFLECTION_SOURCE") {
            let normalized = source.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "local" => {
                    self.learning.reflection_source =
                        crate::openhuman::config::ReflectionSource::Local
                }
                "cloud" => {
                    self.learning.reflection_source =
                        crate::openhuman::config::ReflectionSource::Cloud
                }
                _ => {
                    tracing::warn!(
                        source = %source,
                        "ignoring invalid OPENHUMAN_LEARNING_REFLECTION_SOURCE (valid: local, cloud)"
                    );
                }
            }
        }
        if let Some(val) = env.get("OPENHUMAN_LEARNING_MAX_REFLECTIONS_PER_SESSION") {
            if let Ok(max) = val.trim().parse::<usize>() {
                self.learning.max_reflections_per_session = max;
            }
        }
        if let Some(val) = env.get("OPENHUMAN_LEARNING_MIN_TURN_COMPLEXITY") {
            if let Ok(min) = val.trim().parse::<usize>() {
                self.learning.min_turn_complexity = min;
            }
        }
        if let Some(flag) = env.get("OPENHUMAN_LEARNING_EPISODIC_CAPTURE_ENABLED") {
            if let Some(enabled) =
                parse_env_bool("OPENHUMAN_LEARNING_EPISODIC_CAPTURE_ENABLED", flag.as_str())
            {
                self.learning.episodic_capture_enabled = enabled;
            }
        }
        if let Some(flag) = env.get("OPENHUMAN_LEARNING_STM_RECALL_ENABLED") {
            if let Some(enabled) =
                parse_env_bool("OPENHUMAN_LEARNING_STM_RECALL_ENABLED", flag.as_str())
            {
                self.learning.stm_recall_enabled = enabled;
            }
        }
    }

    fn apply_memory_tree_env<E: super::env::EnvLookup + ?Sized>(&mut self, env: &E) {
        if let Ok(endpoint) = std::env::var("OPENHUMAN_MEMORY_EMBED_ENDPOINT") {
            let trimmed = endpoint.trim();
            self.memory_tree.embedding_endpoint = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            };
        }
        if let Ok(model) = std::env::var("OPENHUMAN_MEMORY_EMBED_MODEL") {
            let trimmed = model.trim();
            self.memory_tree.embedding_model = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            };
        }
        if let Ok(val) = std::env::var("OPENHUMAN_MEMORY_EMBED_TIMEOUT_MS") {
            if let Ok(timeout_ms) = val.trim().parse::<u64>() {
                if timeout_ms > 0 {
                    self.memory_tree.embedding_timeout_ms = Some(timeout_ms);
                }
            }
        }
        if let Ok(flag) = std::env::var("OPENHUMAN_MEMORY_EMBED_STRICT") {
            if let Some(strict) = parse_env_bool("OPENHUMAN_MEMORY_EMBED_STRICT", &flag) {
                self.memory_tree.embedding_strict = strict;
            }
        }
        if let Some(val) = env.get("OPENHUMAN_MEMORY_EMBED_RATE_LIMIT") {
            if let Ok(per_min) = val.trim().parse::<u32>() {
                self.memory.embedding_rate_limit_per_min = per_min;
            }
        }

        if let Ok(endpoint) = std::env::var("OPENHUMAN_MEMORY_EXTRACT_ENDPOINT") {
            let trimmed = endpoint.trim();
            self.memory_tree.llm_extractor_endpoint = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            };
        }
        if let Ok(model) = std::env::var("OPENHUMAN_MEMORY_EXTRACT_MODEL") {
            let trimmed = model.trim();
            self.memory_tree.llm_extractor_model = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            };
        }
        if let Ok(val) = std::env::var("OPENHUMAN_MEMORY_EXTRACT_TIMEOUT_MS") {
            if let Ok(ms) = val.trim().parse::<u64>() {
                if ms > 0 {
                    self.memory_tree.llm_extractor_timeout_ms = Some(ms);
                }
            }
        }

        if let Ok(endpoint) = std::env::var("OPENHUMAN_MEMORY_SUMMARISE_ENDPOINT") {
            let trimmed = endpoint.trim();
            self.memory_tree.llm_summariser_endpoint = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            };
        }
        if let Ok(model) = std::env::var("OPENHUMAN_MEMORY_SUMMARISE_MODEL") {
            let trimmed = model.trim();
            self.memory_tree.llm_summariser_model = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            };
        }
        if let Ok(val) = std::env::var("OPENHUMAN_MEMORY_SUMMARISE_TIMEOUT_MS") {
            if let Ok(ms) = val.trim().parse::<u64>() {
                if ms > 0 {
                    self.memory_tree.llm_summariser_timeout_ms = Some(ms);
                }
            }
        }

        if let Some(dir) = env.get("OPENHUMAN_MEMORY_TREE_CONTENT_DIR") {
            let trimmed = dir.trim();
            self.memory_tree.content_dir = if trimmed.is_empty() {
                None
            } else {
                Some(std::path::PathBuf::from(trimmed))
            };
        }

        if let Some(raw) = env.get("OPENHUMAN_MEMORY_TREE_LLM_BACKEND") {
            let trimmed = raw.trim();
            if !trimmed.is_empty() {
                match crate::openhuman::config::LlmBackend::parse(trimmed) {
                    Ok(b) => {
                        log::debug!(
                            "[memory_tree] OPENHUMAN_MEMORY_TREE_LLM_BACKEND override applied: {}",
                            b.as_str()
                        );
                        self.memory_tree.llm_backend = b;
                    }
                    Err(e) => {
                        tracing::warn!(
                            value = trimmed,
                            error = %e,
                            "ignoring invalid OPENHUMAN_MEMORY_TREE_LLM_BACKEND (valid: cloud, local)"
                        );
                    }
                }
            }
        }
        if let Some(raw) = env.get("OPENHUMAN_MEMORY_TREE_CLOUD_LLM_MODEL") {
            let trimmed = raw.trim();
            self.memory_tree.cloud_llm_model = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            };
        }

        if let Some(raw) = env.get("OPENHUMAN_MEMORY_TREE_SMART_WALK_MODEL") {
            let trimmed = raw.trim();
            self.memory_tree.smart_walk_model = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            };
        }

        if let Some(raw) = env.get("OPENHUMAN_MEMORY_TREE_CLOUD_SUMMARIZATION") {
            if let Some(val) = parse_env_bool("OPENHUMAN_MEMORY_TREE_CLOUD_SUMMARIZATION", &raw) {
                self.memory_tree.cloud_summarization_opt_in = val;
            }
        }
        if let Some(raw) = env.get("OPENHUMAN_MEMORY_TREE_SPACY_ENABLED") {
            if let Some(val) = parse_env_bool("OPENHUMAN_MEMORY_TREE_SPACY_ENABLED", &raw) {
                self.memory_tree.spacy_enabled = val;
            }
        }
    }

    /// `[subsystems.memory]` overrides — kernel.md §3.6 / plan-memory.md §4.5. Mirrors
    /// the `apply_memory_tree_env` reading pattern above. GREENFIELD: nothing
    /// reads `self.subsystems` yet, so these overrides have no runtime effect
    /// beyond making the field settable via env for forward compatibility.
    fn apply_subsystems_env<E: super::env::EnvLookup + ?Sized>(&mut self, env: &E) {
        if let Some(raw) = env.get("OPENHUMAN_MEMORY_DRIVER") {
            let trimmed = raw.trim();
            if !trimmed.is_empty() {
                self.subsystems.memory.driver = trimmed.to_string();
            }
        }

        if let Some(raw) = env.get("OPENHUMAN_MEMORY_HOOKS_AUTO_RECALL") {
            if let Some(val) = parse_env_bool("OPENHUMAN_MEMORY_HOOKS_AUTO_RECALL", &raw) {
                self.subsystems.memory.hooks.auto_recall = val;
            }
        }
        if let Some(raw) = env.get("OPENHUMAN_MEMORY_HOOKS_AUTO_CAPTURE") {
            if let Some(val) = parse_env_bool("OPENHUMAN_MEMORY_HOOKS_AUTO_CAPTURE", &raw) {
                self.subsystems.memory.hooks.auto_capture = val;
            }
        }
        if let Some(raw) = env.get("OPENHUMAN_MEMORY_HOOKS_MAX_CONTEXT_TOKENS") {
            let trimmed = raw.trim();
            if !trimmed.is_empty() {
                match trimmed.parse::<usize>() {
                    Ok(v) => self.subsystems.memory.hooks.max_context_tokens = v,
                    Err(_) => tracing::warn!(
                        value = %raw,
                        "invalid OPENHUMAN_MEMORY_HOOKS_MAX_CONTEXT_TOKENS ignored; expected an unsigned integer"
                    ),
                }
            }
        }
        if let Some(raw) = env.get("OPENHUMAN_MEMORY_HOOKS_RECALL_MAX_CHARS") {
            let trimmed = raw.trim();
            if !trimmed.is_empty() {
                match trimmed.parse::<usize>() {
                    Ok(v) => self.subsystems.memory.hooks.recall_max_chars = v,
                    Err(_) => tracing::warn!(
                        value = %raw,
                        "invalid OPENHUMAN_MEMORY_HOOKS_RECALL_MAX_CHARS ignored; expected an unsigned integer"
                    ),
                }
            }
        }
        if let Some(raw) = env.get("OPENHUMAN_MEMORY_HOOKS_CAPTURE_MAX_CHARS") {
            let trimmed = raw.trim();
            if !trimmed.is_empty() {
                match trimmed.parse::<usize>() {
                    Ok(v) => self.subsystems.memory.hooks.capture_max_chars = v,
                    Err(_) => tracing::warn!(
                        value = %raw,
                        "invalid OPENHUMAN_MEMORY_HOOKS_CAPTURE_MAX_CHARS ignored; expected an unsigned integer"
                    ),
                }
            }
        }
    }

    fn apply_update_env<E: super::env::EnvLookup + ?Sized>(&mut self, env: &E) {
        if let Some(flag) = env.get("OPENHUMAN_AUTO_UPDATE_ENABLED") {
            let normalized = flag.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "1" | "true" | "yes" | "on" => self.update.enabled = true,
                "0" | "false" | "no" | "off" => self.update.enabled = false,
                _ => {}
            }
        }
        if let Some(val) = env.get("OPENHUMAN_AUTO_UPDATE_INTERVAL_MINUTES") {
            if let Ok(minutes) = val.trim().parse::<u32>() {
                self.update.interval_minutes = minutes;
            }
        }
        if let Some(raw) = env.get("OPENHUMAN_AUTO_UPDATE_RESTART_STRATEGY") {
            match raw.trim().to_ascii_lowercase().as_str() {
                "self_replace" | "self-replace" | "self" => {
                    self.update.restart_strategy = UpdateRestartStrategy::SelfReplace;
                }
                "supervisor" | "stage_only" | "stage-only" => {
                    self.update.restart_strategy = UpdateRestartStrategy::Supervisor;
                }
                other => {
                    tracing::warn!(
                        value = other,
                        "ignoring invalid OPENHUMAN_AUTO_UPDATE_RESTART_STRATEGY \
                         (valid: self_replace, supervisor)"
                    );
                }
            }
        }
        if let Some(flag) = env.get("OPENHUMAN_AUTO_UPDATE_RPC_MUTATIONS_ENABLED") {
            if let Some(enabled) =
                parse_env_bool("OPENHUMAN_AUTO_UPDATE_RPC_MUTATIONS_ENABLED", &flag)
            {
                self.update.rpc_mutations_enabled = enabled;
            }
        }
    }

    fn apply_dictation_env<E: super::env::EnvLookup + ?Sized>(&mut self, env: &E) {
        if let Some(flag) = env.get("OPENHUMAN_DICTATION_ENABLED") {
            let normalized = flag.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "1" | "true" | "yes" | "on" => self.dictation.enabled = true,
                "0" | "false" | "no" | "off" => self.dictation.enabled = false,
                _ => {}
            }
        }
        if let Some(hotkey) = env.get("OPENHUMAN_DICTATION_HOTKEY") {
            let hotkey = hotkey.trim();
            if !hotkey.is_empty() {
                self.dictation.hotkey = hotkey.to_string();
            }
        }
        if let Some(mode) = env.get("OPENHUMAN_DICTATION_ACTIVATION_MODE") {
            let normalized = mode.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "toggle" => {
                    self.dictation.activation_mode =
                        crate::openhuman::config::DictationActivationMode::Toggle
                }
                "push" => {
                    self.dictation.activation_mode =
                        crate::openhuman::config::DictationActivationMode::Push
                }
                _ => {
                    tracing::warn!(
                        mode = %mode,
                        "ignoring invalid OPENHUMAN_DICTATION_ACTIVATION_MODE (valid: toggle, push)"
                    );
                }
            }
        }
        if let Some(flag) = env.get("OPENHUMAN_DICTATION_LLM_REFINEMENT") {
            let normalized = flag.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "1" | "true" | "yes" | "on" => self.dictation.llm_refinement = true,
                "0" | "false" | "no" | "off" => self.dictation.llm_refinement = false,
                _ => {}
            }
        }
        if let Some(flag) = env.get("OPENHUMAN_DICTATION_STREAMING") {
            let normalized = flag.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "1" | "true" | "yes" | "on" => self.dictation.streaming = true,
                "0" | "false" | "no" | "off" => self.dictation.streaming = false,
                _ => {}
            }
        }
        if let Some(val) = env.get("OPENHUMAN_DICTATION_STREAMING_INTERVAL_MS") {
            if let Ok(ms) = val.trim().parse::<u64>() {
                self.dictation.streaming_interval_ms = ms;
            }
        }
    }

    fn apply_context_env<E: super::env::EnvLookup + ?Sized>(&mut self, env: &E) {
        if let Some(flag) = env.get("OPENHUMAN_CONTEXT_ENABLED") {
            let normalized = flag.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "1" | "true" | "yes" | "on" => self.context.enabled = true,
                "0" | "false" | "no" | "off" => self.context.enabled = false,
                _ => {}
            }
        }
        if let Some(flag) = env.get("OPENHUMAN_CONTEXT_MICROCOMPACT_ENABLED") {
            let normalized = flag.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "1" | "true" | "yes" | "on" => self.context.microcompact_enabled = true,
                "0" | "false" | "no" | "off" => self.context.microcompact_enabled = false,
                _ => {}
            }
        }
        if let Some(flag) = env.get("OPENHUMAN_CONTEXT_AUTOCOMPACT_ENABLED") {
            let normalized = flag.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "1" | "true" | "yes" | "on" => self.context.autocompact_enabled = true,
                "0" | "false" | "no" | "off" => self.context.autocompact_enabled = false,
                _ => {}
            }
        }
        if let Some(val) = env.get("OPENHUMAN_CONTEXT_TOOL_RESULT_BUDGET_BYTES") {
            if let Ok(n) = val.trim().parse::<usize>() {
                self.context.tool_result_budget_bytes = n;
            }
        }
        // Kill-switch for native tool-output compaction (Stage 1a). On by
        // default; `OPENHUMAN_COMPACTION=0` disables it for a support/A-B
        // bisect. Accepts the canonical short name and the namespaced form.
        if let Some(flag) = env
            .get("OPENHUMAN_COMPACTION")
            .or_else(|| env.get("OPENHUMAN_CONTEXT_COMPACTION_ENABLED"))
        {
            match flag.trim().to_ascii_lowercase().as_str() {
                "1" | "true" | "yes" | "on" => self.context.compaction_enabled = true,
                "0" | "false" | "no" | "off" => self.context.compaction_enabled = false,
                _ => {}
            }
        }
        if let Some(model) = env.get("OPENHUMAN_CONTEXT_SUMMARIZER_MODEL") {
            let model = model.trim();
            if !model.is_empty() {
                self.context.summarizer_model = Some(model.to_string());
            }
        }
        let context_default = crate::openhuman::agent::context::DEFAULT_TOOL_RESULT_BUDGET_BYTES;
        let context_env_set = env.contains("OPENHUMAN_CONTEXT_TOOL_RESULT_BUDGET_BYTES");
        if !context_env_set
            && self.context.tool_result_budget_bytes == context_default
            && self.agent.tool_result_budget_bytes != context_default
        {
            tracing::warn!(
                old = self.agent.tool_result_budget_bytes,
                "[context:config] `agent.tool_result_budget_bytes` is \
                 deprecated — please move it to \
                 `context.tool_result_budget_bytes` in your config.toml"
            );
            self.context.tool_result_budget_bytes = self.agent.tool_result_budget_bytes;
        }
    }
}
