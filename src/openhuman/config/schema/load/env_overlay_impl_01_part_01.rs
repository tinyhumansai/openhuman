
impl Config {

    pub fn apply_env_overrides(&mut self) {
        use super::env::ProcessEnv;
        self.apply_env_overrides_from(&ProcessEnv);
    }

    pub(super) fn apply_env_overrides_from(
        &mut self,
        env: &(dyn super::env::EnvLookup + Send + Sync),
    ) {
        self.apply_env_overlay_with(env);

        if self.proxy.enabled && self.proxy.scope == ProxyScope::Environment {
            self.proxy.apply_to_process_env();
        }

        set_runtime_proxy_config(self.proxy.clone());

        crate::openhuman::inference::embeddings::rate_limit::set_embedding_rate_limit(
            self.memory.embedding_rate_limit_per_min,
        );

        // Launch flags are process-local and intentionally win over both the
        // persisted file and ordinary environment overlays. They are applied
        // after loading so `openhuman -p <provider> -m <model>` never mutates
        // config.toml and desktop launches remain unaffected.
        super::super::cli_overrides::apply_cli_inference_overrides(self);
    }

    /// Pure-ish env overlay: applies overrides read from `env` to `self`.
    ///
    /// "Pure-ish" because it still emits `tracing` logs and calls
    /// `self.proxy.validate()` (which only reads). Crucially, it does
    /// **not** write to the process environment nor the
    /// `set_runtime_proxy_config` global — those stay in the public
    /// [`Self::apply_env_overrides`] wrapper so unit tests can call this
    /// with a [`HashMapEnv`] (see tests) without requiring the
    /// `TEST_ENV_LOCK` or tainting sibling tests.
    pub(crate) fn apply_env_overlay_with<E: super::env::EnvLookup + ?Sized>(&mut self, env: &E) {
        // Only the namespaced `OPENHUMAN_MODEL` is honoured. The bare `MODEL`
        // env var used to be accepted as an alias but collides with vendor
        // asset-tag env vars (e.g. Dell OptiPlex sets `MODEL=7080`), which
        // silently clobbered the LLM model and 400'd every backend call
        // (Sentry OPENHUMAN-TAURI-J8).
        if let Some(model) = env.get("OPENHUMAN_MODEL") {
            let trimmed = model.trim();
            if !trimmed.is_empty() {
                self.default_model = Some(trimmed.to_string());
            }
        }

        if let Some(workspace) = env.get("OPENHUMAN_WORKSPACE") {
            if !workspace.is_empty() {
                let (_, workspace_dir) =
                    super::dirs::resolve_config_dir_for_workspace(&PathBuf::from(workspace));
                self.workspace_dir = workspace_dir;
            }
        }

        if let Some(v) = env.get("OPENHUMAN_ACTION_DIR") {
            let trimmed = v.trim();
            if !trimmed.is_empty() {
                self.action_dir = PathBuf::from(trimmed);
            }
        }

        if let Some(temp_str) = env.get("OPENHUMAN_TEMPERATURE") {
            if let Ok(temp) = temp_str.parse::<f64>() {
                if (0.0..=2.0).contains(&temp) {
                    self.default_temperature = temp;
                }
            }
        }

        if let Some(raw) = env.get("OPENHUMAN_MAX_ACTIONS_PER_HOUR") {
            let trimmed = raw.trim();
            if !trimmed.is_empty() {
                match trimmed.parse::<u32>() {
                    Ok(limit) => self.autonomy.max_actions_per_hour = limit,
                    Err(_) => tracing::warn!(
                        value = %raw,
                        "invalid OPENHUMAN_MAX_ACTIONS_PER_HOUR ignored; expected an unsigned integer"
                    ),
                }
            }
        }

        if let Some(raw) = env.get(MEMORY_SYNC_INTERVAL_SECS_ENV_VAR) {
            let trimmed = raw.trim();
            if !trimmed.is_empty() {
                match trimmed.parse::<u64>() {
                    Ok(secs) => self.memory_sync_interval_secs = Some(secs),
                    Err(_) => tracing::warn!(
                        env = %MEMORY_SYNC_INTERVAL_SECS_ENV_VAR,
                        value = %raw,
                        "invalid memory-sync interval ignored; expected an unsigned integer (0 = manual)"
                    ),
                }
            }
        }

        if let Some(language) = env.get("OPENHUMAN_OUTPUT_LANGUAGE") {
            let language = language.trim();
            if !language.is_empty() {
                self.output_language = Some(language.to_string());
            }
        }

        if let Some(url) = env.get("YOUPET_CORE_API_URL") {
            let trimmed = url.trim().trim_end_matches('/');
            if !trimmed.is_empty() {
                self.youpet.core_api_url = trimmed.to_string();
            }
        }
        if let Some(token) = env.get("YOUPET_SERVICE_TOKEN") {
            let trimmed = token.trim();
            if !trimmed.is_empty() {
                self.youpet.service_token = Some(trimmed.to_string());
            }
        }
        if let Some(actor) = env.get("YOUPET_WORKBENCH_ACTOR_ID") {
            let trimmed = actor.trim();
            if !trimmed.is_empty() {
                self.youpet.workbench_actor_id = trimmed.to_string();
            }
        }
        if let Some(operator) = env.get("YOUPET_OPERATOR_USER_ID") {
            let trimmed = operator.trim();
            self.youpet.operator_user_id = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            };
        }
        if let Some(tenant) = env.get("YOUPET_TENANT_ID") {
            let trimmed = tenant.trim();
            self.youpet.tenant_id = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            };
        }

        if let Some(flag) = env.get_any(&["OPENHUMAN_REASONING_ENABLED", "REASONING_ENABLED"]) {
            let normalized = flag.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "1" | "true" | "yes" | "on" => self.runtime.reasoning_enabled = Some(true),
                "0" | "false" | "no" | "off" => self.runtime.reasoning_enabled = Some(false),
                _ => {}
            }
        }

        if let Some(flag) = env.get_any(&["OPENHUMAN_SHELL_HIDE_WINDOW", "SHELL_HIDE_WINDOW"]) {
            match classify_shell_hide_window(&flag) {
                // An empty / whitespace-only value means the var is present but
                // unset (common when a `.env` or launcher exports `VAR=`). Treat
                // it as absent — keep the current value rather than warning on
                // every boot. Trace-level so the no-op stays diagnosable without
                // the INFO/WARN noise this change exists to remove.
                ShellHideWindowParse::Unset => tracing::trace!(
                    "[config][shell] OPENHUMAN_SHELL_HIDE_WINDOW empty value treated as unset; \
                     keeping hide_window={}",
                    self.shell.hide_window
                ),
                ShellHideWindowParse::Set(value) => {
                    self.shell.hide_window = value;
                    tracing::debug!(
                        value = %flag,
                        "[config][shell] OPENHUMAN_SHELL_HIDE_WINDOW applied: hide_window={value}"
                    );
                }
                ShellHideWindowParse::Unrecognized => tracing::warn!(
                    value = %flag,
                    "[config][shell] OPENHUMAN_SHELL_HIDE_WINDOW unrecognized value ignored; \
                     keeping current hide_window={}",
                    self.shell.hide_window
                ),
            }
        }

        self.apply_search_env(env);
        self.apply_proxy_env(env);
        self.apply_runtime_env(env);
        self.apply_observability_env(env);
        self.apply_learning_env(env);
        self.apply_memory_tree_env(env);
        self.apply_subsystems_env(env);
        self.apply_update_env(env);
        self.apply_dictation_env(env);
        self.apply_context_env(env);
    }

    fn apply_search_env<E: super::env::EnvLookup + ?Sized>(&mut self, env: &E) {
        if let Some(key) = env.get_any(&["OPENHUMAN_SELTZ_API_KEY", "SELTZ_API_KEY"]) {
            if !key.is_empty() {
                self.seltz.api_key = Some(key);
                self.seltz.enabled = true;
            }
        }
        if let Some(url) = env.get_any(&["OPENHUMAN_SELTZ_API_URL", "SELTZ_API_URL"]) {
            if !url.is_empty() {
                self.seltz.api_url = Some(url);
            }
        }
        if let Some(max) = env.get_any(&["OPENHUMAN_SELTZ_MAX_RESULTS", "SELTZ_MAX_RESULTS"]) {
            if let Ok(n) = max.parse::<usize>() {
                if (1..=20).contains(&n) {
                    self.seltz.max_results = n;
                }
            }
        }

        if let Some(flag) = env.get_any(&["OPENHUMAN_SEARXNG_ENABLED", "SEARXNG_ENABLED"]) {
            if let Some(enabled) = parse_env_bool("OPENHUMAN_SEARXNG_ENABLED", &flag) {
                self.searxng.enabled = enabled;
            }
        }
        if let Some(url) = env.get_any(&["OPENHUMAN_SEARXNG_BASE_URL", "SEARXNG_BASE_URL"]) {
            let url = url.trim();
            if !url.is_empty() {
                self.searxng.base_url = url.to_string();
            }
        }
        if let Some(max) = env.get_any(&["OPENHUMAN_SEARXNG_MAX_RESULTS", "SEARXNG_MAX_RESULTS"]) {
            if let Ok(n) = max.parse::<usize>() {
                if (1..=50).contains(&n) {
                    self.searxng.max_results = n;
                }
            }
        }
        if let Some(language) = env.get_any(&[
            "OPENHUMAN_SEARXNG_DEFAULT_LANGUAGE",
            "SEARXNG_DEFAULT_LANGUAGE",
        ]) {
            let language = language.trim();
            if !language.is_empty() {
                self.searxng.default_language = language.to_string();
            }
        }
        if let Some(timeout_secs) = env.get_any(&[
            "OPENHUMAN_SEARXNG_TIMEOUT_SECS",
            "OPENHUMAN_SEARXNG_TIMEOUT_SECONDS",
            "SEARXNG_TIMEOUT_SECS",
            "SEARXNG_TIMEOUT_SECONDS",
        ]) {
            if let Ok(timeout_secs) = timeout_secs.parse::<u64>() {
                if timeout_secs > 0 {
                    self.searxng.timeout_secs = timeout_secs;
                }
            }
        }

        if let Some(engine) = env.get_any(&["OPENHUMAN_SEARCH_ENGINE", "SEARCH_ENGINE"]) {
            let engine = engine.trim().to_ascii_lowercase();
            if !engine.is_empty() {
                self.search.engine = engine;
            }
        }
        if let Some(key) = env.get_any(&["OPENHUMAN_PARALLEL_API_KEY", "PARALLEL_API_KEY"]) {
            if !key.trim().is_empty() {
                self.search.parallel.api_key = Some(key);
            }
        }
        if let Some(key) = env.get_any(&["OPENHUMAN_BRAVE_API_KEY", "BRAVE_API_KEY"]) {
            if !key.trim().is_empty() {
                self.search.brave.api_key = Some(key);
            }
        }
        if let Some(key) = env.get_any(&["OPENHUMAN_QUERIT_API_KEY", "QUERIT_API_KEY"]) {
            if !key.trim().is_empty() {
                self.search.querit.api_key = Some(key);
            }
        }
        if let Some(key) = env.get_any(&["OPENHUMAN_EXA_API_KEY", "EXA_API_KEY"]) {
            if !key.trim().is_empty() {
                self.search.exa.api_key = Some(key);
            }
        }
        if let Some(max) = env.get_any(&["OPENHUMAN_SEARCH_MAX_RESULTS", "SEARCH_MAX_RESULTS"]) {
            if let Ok(n) = max.parse::<usize>() {
                if (1..=20).contains(&n) {
                    self.search.max_results = n;
                }
            }
        }
        if let Some(t) = env.get_any(&["OPENHUMAN_SEARCH_TIMEOUT_SECS", "SEARCH_TIMEOUT_SECS"]) {
            if let Ok(n) = t.parse::<u64>() {
                if n > 0 {
                    self.search.timeout_secs = n;
                }
            }
        }

        if env.contains("OPENHUMAN_WEB_SEARCH_ENABLED") {
            log::warn!(
                "[config] OPENHUMAN_WEB_SEARCH_ENABLED is deprecated and ignored — \
                 web search is always registered; provider/API-key overrides were removed."
            );
        }

        if let Some(max_results) =
            env.get_any(&["OPENHUMAN_WEB_SEARCH_MAX_RESULTS", "WEB_SEARCH_MAX_RESULTS"])
        {
            if let Ok(max_results) = max_results.parse::<usize>() {
                if (1..=10).contains(&max_results) {
                    self.web_search.max_results = max_results;
                }
            }
        }

        if let Some(timeout_secs) = env.get_any(&[
            "OPENHUMAN_WEB_SEARCH_TIMEOUT_SECS",
            "WEB_SEARCH_TIMEOUT_SECS",
        ]) {
            if let Ok(timeout_secs) = timeout_secs.parse::<u64>() {
                if timeout_secs > 0 {
                    self.web_search.timeout_secs = timeout_secs;
                }
            }
        }
    }

    fn apply_proxy_env<E: super::env::EnvLookup + ?Sized>(&mut self, env: &E) {
        let explicit_proxy_enabled = env
            .get("OPENHUMAN_PROXY_ENABLED")
            .as_deref()
            .and_then(parse_proxy_enabled);
        if let Some(enabled) = explicit_proxy_enabled {
            self.proxy.enabled = enabled;
        }

        let mut proxy_url_overridden = false;
        if let Some(proxy_url) = env.get_any(&["OPENHUMAN_HTTP_PROXY", "HTTP_PROXY"]) {
            self.proxy.http_proxy = normalize_proxy_url_option(Some(&proxy_url));
            proxy_url_overridden = true;
        }
        if let Some(proxy_url) = env.get_any(&["OPENHUMAN_HTTPS_PROXY", "HTTPS_PROXY"]) {
            self.proxy.https_proxy = normalize_proxy_url_option(Some(&proxy_url));
            proxy_url_overridden = true;
        }
        if let Some(proxy_url) = env.get_any(&["OPENHUMAN_ALL_PROXY", "ALL_PROXY"]) {
            self.proxy.all_proxy = normalize_proxy_url_option(Some(&proxy_url));
            proxy_url_overridden = true;
        }
        if let Some(no_proxy) = env.get_any(&["OPENHUMAN_NO_PROXY", "NO_PROXY"]) {
            self.proxy.no_proxy = normalize_no_proxy_list(vec![no_proxy]);
        }

        if explicit_proxy_enabled.is_none()
            && proxy_url_overridden
            && self.proxy.has_any_proxy_url()
        {
            self.proxy.enabled = true;
        }

        if let Some(scope_raw) = env.get("OPENHUMAN_PROXY_SCOPE") {
            let trimmed = scope_raw.trim();
            if !trimmed.is_empty() {
                match parse_proxy_scope(trimmed) {
                    Some(scope) => self.proxy.scope = scope,
                    None => {
                        tracing::warn!("Invalid OPENHUMAN_PROXY_SCOPE value {:?} ignored", trimmed);
                    }
                }
            }
        }

        if let Some(services_raw) = env.get("OPENHUMAN_PROXY_SERVICES") {
            self.proxy.services = normalize_service_list(vec![services_raw]);
        }

        if let Err(error) = self.proxy.validate() {
            tracing::warn!("Invalid proxy configuration ignored: {error}");
            self.proxy.enabled = false;
        }
    }

    fn apply_runtime_env<E: super::env::EnvLookup + ?Sized>(&mut self, env: &E) {
        if let Some(tier_str) = env.get("OPENHUMAN_LOCAL_AI_TIER") {
            let tier_str = tier_str.trim().to_ascii_lowercase();
            if !tier_str.is_empty() {
                if let Some(tier) =
                    crate::openhuman::inference::presets::ModelTier::from_str_opt(&tier_str)
                {
                    if tier == crate::openhuman::inference::presets::ModelTier::Custom {
                        tracing::warn!(
                            tier = %tier_str,
                            "ignoring custom OPENHUMAN_LOCAL_AI_TIER; only built-in presets are supported"
                        );
                    } else if !tier.is_mvp_allowed() {
                        tracing::warn!(
                            tier = %tier_str,
                            "ignoring OPENHUMAN_LOCAL_AI_TIER outside the 1B local-model allowlist"
                        );
                    } else {
                        crate::openhuman::inference::presets::apply_preset_to_config(
                            &mut self.local_ai,
                            tier,
                        );
                        tracing::debug!(
                            tier = %tier_str,
                            "applied local AI tier from OPENHUMAN_LOCAL_AI_TIER"
                        );
                    }
                } else {
                    tracing::warn!(
                        tier = %tier_str,
                        "ignoring invalid OPENHUMAN_LOCAL_AI_TIER (valid: ram_2_4gb)"
                    );
                }
            }
        }

        if let Some(flag) = env.get("OPENHUMAN_NODE_ENABLED") {
            if let Some(enabled) = parse_env_bool("OPENHUMAN_NODE_ENABLED", &flag) {
                self.node.enabled = enabled;
            }
        }
        if let Some(version) = env.get("OPENHUMAN_NODE_VERSION") {
            let trimmed = version.trim();
            if !trimmed.is_empty() {
                self.node.version = trimmed.to_string();
            }
        }
        if let Some(dir) = env.get("OPENHUMAN_NODE_CACHE_DIR") {
            let trimmed = dir.trim();
            if !trimmed.is_empty() {
                self.node.cache_dir = trimmed.to_string();
            }
        }
        if let Some(flag) = env.get("OPENHUMAN_NODE_PREFER_SYSTEM") {
            if let Some(prefer_system) = parse_env_bool("OPENHUMAN_NODE_PREFER_SYSTEM", &flag) {
                self.node.prefer_system = prefer_system;
            }
        }

        if let Some(flag) = env.get("OPENHUMAN_RUNTIME_PYTHON_ENABLED") {
            if let Some(enabled) = parse_env_bool("OPENHUMAN_RUNTIME_PYTHON_ENABLED", &flag) {
                self.runtime_python.enabled = enabled;
            }
        }
        if let Some(version) = env.get("OPENHUMAN_RUNTIME_PYTHON_MINIMUM_VERSION") {
            let trimmed = version.trim();
            if !trimmed.is_empty() {
                self.runtime_python.minimum_version = trimmed.to_string();
            }
        }
        if let Some(dir) = env.get("OPENHUMAN_RUNTIME_PYTHON_CACHE_DIR") {
            self.runtime_python.cache_dir = dir.trim().to_string();
        }
        if let Some(tag) = env.get("OPENHUMAN_RUNTIME_PYTHON_MANAGED_RELEASE_TAG") {
            self.runtime_python.managed_release_tag = tag.trim().to_string();
        }
        if let Some(flag) = env.get("OPENHUMAN_RUNTIME_PYTHON_PREFER_SYSTEM") {
            if let Some(prefer_system) =
                parse_env_bool("OPENHUMAN_RUNTIME_PYTHON_PREFER_SYSTEM", &flag)
            {
                self.runtime_python.prefer_system = prefer_system;
            }
        }
        if let Some(command) = env.get("OPENHUMAN_RUNTIME_PYTHON_PREFERRED_COMMAND") {
            self.runtime_python.preferred_command = command.trim().to_string();
        }

        // --- Shared language-runtime pool (#5106) --------------------------
        if let Some(flag) = env.get("OPENHUMAN_RUNTIME_POOL_ENABLED") {
            if let Some(enabled) = parse_env_bool("OPENHUMAN_RUNTIME_POOL_ENABLED", &flag) {
                self.runtime_pool.enabled = enabled;
            }
        }
        if let Some(raw) = env.get("OPENHUMAN_RUNTIME_POOL_NODE_MAX_WORKERS") {
            match raw.trim().parse::<usize>() {
                Ok(n) => self.runtime_pool.node.max_workers = n,
                Err(e) => tracing::warn!(
                    value = %raw,
                    error = %e,
                    "[config] ignoring invalid OPENHUMAN_RUNTIME_POOL_NODE_MAX_WORKERS"
                ),
            }
        }
        if let Some(raw) = env.get("OPENHUMAN_RUNTIME_POOL_PYTHON_MAX_WORKERS") {
            match raw.trim().parse::<usize>() {
                Ok(n) => self.runtime_pool.python.max_workers = n,
                Err(e) => tracing::warn!(
                    value = %raw,
                    error = %e,
                    "[config] ignoring invalid OPENHUMAN_RUNTIME_POOL_PYTHON_MAX_WORKERS"
                ),
            }
        }

        // --- TokenJuice content router -------------------------------------
        if let Some(flag) = env.get("OPENHUMAN_TOKENJUICE_ENABLED") {
            if let Some(v) = parse_env_bool("OPENHUMAN_TOKENJUICE_ENABLED", &flag) {
                self.tokenjuice.router_enabled = v;
            }
        }
        if let Some(flag) = env.get("OPENHUMAN_TOKENJUICE_CCR_ENABLED") {
            if let Some(v) = parse_env_bool("OPENHUMAN_TOKENJUICE_CCR_ENABLED", &flag) {
                self.tokenjuice.ccr_enabled = v;
            }
        }
        if let Some(flag) = env.get("OPENHUMAN_TOKENJUICE_CCR_DISK_ENABLED") {
            if let Some(v) = parse_env_bool("OPENHUMAN_TOKENJUICE_CCR_DISK_ENABLED", &flag) {
                self.tokenjuice.ccr_disk_enabled = v;
            }
        }
        if let Some(flag) = env.get("OPENHUMAN_TOKENJUICE_SEARCH_ENABLED") {
            if let Some(v) = parse_env_bool("OPENHUMAN_TOKENJUICE_SEARCH_ENABLED", &flag) {
                self.tokenjuice.search_enabled = v;
            }
        }
        if let Some(flag) = env.get("OPENHUMAN_TOKENJUICE_CODE_ENABLED") {
            if let Some(v) = parse_env_bool("OPENHUMAN_TOKENJUICE_CODE_ENABLED", &flag) {
                self.tokenjuice.code_enabled = v;
            }
        }
        if let Some(flag) = env.get("OPENHUMAN_TOKENJUICE_HTML_ENABLED") {
            if let Some(v) = parse_env_bool("OPENHUMAN_TOKENJUICE_HTML_ENABLED", &flag) {
                self.tokenjuice.html_enabled = v;
            }
        }
        if let Some(s) = env.get("OPENHUMAN_TOKENJUICE_MAX_CACHE_ENTRIES") {
            if let Ok(v) = s.trim().parse::<usize>() {
                self.tokenjuice.max_cache_entries = v;
            }
        }
        if let Some(s) = env.get("OPENHUMAN_TOKENJUICE_MAX_CACHE_BYTES") {
            if let Ok(v) = s.trim().parse::<usize>() {
                self.tokenjuice.max_cache_bytes = v;
            }
        }
        if let Some(s) = env.get("OPENHUMAN_TOKENJUICE_CCR_TTL_SECS") {
            if let Ok(v) = s.trim().parse::<u64>() {
                self.tokenjuice.ccr_ttl_secs = Some(v);
            }
        }
        if let Some(s) = env.get("OPENHUMAN_TOKENJUICE_CCR_MIN_TOKENS") {
            if let Ok(v) = s.trim().parse::<usize>() {
                self.tokenjuice.ccr_min_tokens = v;
            }
        }
        // ML plain-text compressor (Kompress).
        if let Some(flag) = env.get("OPENHUMAN_TOKENJUICE_ML_COMPRESSION_ENABLED") {
            if let Some(v) = parse_env_bool("OPENHUMAN_TOKENJUICE_ML_COMPRESSION_ENABLED", &flag) {
                self.tokenjuice.ml_compression_enabled = v;
            }
        }
        if let Some(m) = env.get("OPENHUMAN_TOKENJUICE_ML_MODEL_ID") {
            let t = m.trim();
            if !t.is_empty() {
                self.tokenjuice.ml_model_id = t.to_string();
            }
        }
        if let Some(d) = env.get("OPENHUMAN_TOKENJUICE_ML_DEVICE") {
            let t = d.trim();
            if !t.is_empty() {
                self.tokenjuice.ml_device = t.to_string();
            }
        }
        if let Some(r) = env.get("OPENHUMAN_TOKENJUICE_ML_TARGET_RATIO") {
            if let Ok(v) = r.trim().parse::<f64>() {
                if (0.0..=1.0).contains(&v) {
                    self.tokenjuice.ml_target_ratio = v;
                }
            }
        }
        if let Some(s) = env.get("OPENHUMAN_TOKENJUICE_ML_SIDECAR_IDLE_TIMEOUT_SECS") {
            if let Ok(v) = s.trim().parse::<u64>() {
                self.tokenjuice.ml_sidecar_idle_timeout_secs = v;
            }
        }
        if let Some(s) = env.get("OPENHUMAN_TOKENJUICE_ML_MAX_INPUT_CHARS") {
            if let Ok(v) = s.trim().parse::<usize>() {
                self.tokenjuice.ml_max_input_chars = v;
            }
        }
    }

    fn apply_observability_env<E: super::env::EnvLookup + ?Sized>(&mut self, env: &E) {
        let dsn_value = env
            .get("OPENHUMAN_CORE_SENTRY_DSN")
            .or_else(|| env.get("OPENHUMAN_SENTRY_DSN"))
            .or_else(|| option_env!("OPENHUMAN_CORE_SENTRY_DSN").map(|s| s.to_string()))
            .or_else(|| option_env!("OPENHUMAN_SENTRY_DSN").map(|s| s.to_string()));
        if let Some(dsn) = dsn_value {
            let dsn = dsn.trim();
            if !dsn.is_empty() {
                self.observability.sentry_dsn = Some(dsn.to_string());
            }
        }

        if let Some(flag) = env.get("OPENHUMAN_ANALYTICS_ENABLED") {
            let normalized = flag.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "1" | "true" | "yes" | "on" => self.observability.analytics_enabled = true,
                "0" | "false" | "no" | "off" => self.observability.analytics_enabled = false,
                _ => {}
            }
        }

        // Opt-in: export prompt/reply content on trace spans (default off — a
        // deliberate PII reversal). Token/cost export is unaffected by this flag.
        if let Some(flag) = env.get("OPENHUMAN_AGENT_TRACING_CAPTURE_CONTENT") {
            let normalized = flag.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "1" | "true" | "yes" | "on" => {
                    self.observability.agent_tracing.capture_content = true
                }
                "0" | "false" | "no" | "off" => {
                    self.observability.agent_tracing.capture_content = false
                }
                _ => {}
            }
        }
    }
}
