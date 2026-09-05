
impl Config {
    /// Resolve the root directory where chunk `.md` files are stored.
    ///
    /// Resolution order:
    /// 1. `memory_tree.content_dir` if `Some`.
    /// 2. Default: `<workspace_dir>/memory_tree/content/`.
    ///
    /// This is the only place in the codebase that should compute the content
    /// root — all code that needs the path should call this method.
    pub fn memory_tree_content_root(&self) -> PathBuf {
        self.memory_tree
            .content_dir
            .clone()
            .unwrap_or_else(|| self.workspace_dir.join("memory_tree").join("content"))
    }

    /// Read the per-workload provider string and return the local model id
    /// when the workload is routed to Ollama.
    ///
    /// Recognised workload names:
    /// `"chat"`, `"reasoning"`, `"agentic"`, `"coding"`, `"memory"`, `"embeddings"`,
    /// `"heartbeat"`, `"learning"`, `"subconscious"`.
    ///
    /// Returns `None` when the provider isn't `"ollama:<model>"` (including
    /// when the field is unset, blank, `"cloud"`, or any other prefix).
    /// This is the single source of truth for "is this workload local?" —
    /// callers MUST NOT consult the legacy `local_ai.usage.*` booleans or
    /// `memory_tree.llm_backend`. Those fields are deprecated zombies kept
    /// for migration only.
    pub fn workload_local_model(&self, workload: &str) -> Option<String> {
        let raw = match workload {
            "chat" => self.chat_provider.as_deref(),
            "reasoning" => self.reasoning_provider.as_deref(),
            "agentic" => self.agentic_provider.as_deref(),
            "coding" => self.coding_provider.as_deref(),
            "vision" => self.vision_provider.as_deref(),
            "memory" => self.memory_provider.as_deref(),
            "embeddings" => self.embeddings_provider.as_deref(),
            "heartbeat" => self.heartbeat_provider.as_deref(),
            "learning" => self.learning_provider.as_deref(),
            "subconscious" => self.subconscious_provider.as_deref(),
            _ => None,
        }?;
        let trimmed = raw.trim();
        let model = trimmed.strip_prefix("ollama:")?.trim();
        if model.is_empty() {
            None
        } else {
            Some(model.to_string())
        }
    }

    /// `true` when `workload_local_model` returns `Some` for the named
    /// workload. Convenience wrapper for the common "do I dispatch
    /// locally?" branch.
    pub fn workload_uses_local(&self, workload: &str) -> bool {
        self.workload_local_model(workload).is_some()
    }

    /// Prompt directive for background LLM artifacts, if configured.
    pub fn output_language_directive(&self) -> Option<String> {
        output_language_directive(self.output_language.as_deref())
    }

    /// Resolve an exact model pin for an agent, if configured.
    ///
    /// Precedence is intentionally narrow and deterministic:
    /// 1. `orchestrator.model` when resolving the front-line orchestrator.
    /// 2. `[teams.<agent_id>]` entries, with `lead_model` used for agents
    ///    that can delegate and `agent_model` used for leaf workers.
    /// 3. Built-in aliases such as `[teams.research]` for `researcher` and
    ///    `[teams.code]` for `code_executor`, matching the issue examples.
    ///
    /// Empty strings are ignored so partially-written configs fall back to
    /// the existing auto-routing path.
    pub fn configured_agent_model(&self, agent_id: &str, is_team_lead: bool) -> Option<&str> {
        fn clean(model: Option<&str>) -> Option<&str> {
            model.map(str::trim).filter(|value| !value.is_empty())
        }

        let agent_id = agent_id.trim();
        if agent_id.is_empty() {
            return None;
        }

        if agent_id == "orchestrator" {
            if let Some(model) = clean(self.orchestrator.model.as_deref()) {
                return Some(model);
            }
        }

        if let Some(model) = self
            .teams
            .get(agent_id)
            .and_then(|team| team.model_for_role(is_team_lead))
        {
            return Some(model);
        }

        if let Some(stripped) = agent_id.strip_suffix("_agent") {
            if let Some(model) = self
                .teams
                .get(stripped)
                .and_then(|team| team.model_for_role(is_team_lead))
            {
                return Some(model);
            }
        }

        let aliases: &[&str] = match agent_id {
            "researcher" => &["research"],
            "code_executor" => &["code"],
            "tool_maker" | "tools_agent" => &["tools"],
            "integrations_agent" => &["integrations"],
            _ => &[],
        };

        aliases.iter().find_map(|alias| {
            self.teams
                .get(*alias)
                .and_then(|team| team.model_for_role(is_team_lead))
        })
    }
}

impl Default for Config {
    fn default() -> Self {
        let openhuman_dir =
            crate::openhuman::config::default_root_openhuman_dir().unwrap_or_else(|_| {
                let home = UserDirs::new()
                    .map_or_else(|| PathBuf::from("."), |u| u.home_dir().to_path_buf());
                let dir_name = if crate::api::config::is_staging_app_env(
                    crate::api::config::app_env_from_env().as_deref(),
                ) {
                    ".openhuman-staging"
                } else {
                    ".openhuman"
                };
                home.join(dir_name)
            });

        Self {
            workspace_dir: openhuman_dir.join("workspace"),
            action_dir: crate::openhuman::config::default_action_dir(),
            action_dir_override: None,
            config_path: openhuman_dir.join("config.toml"),
            cli_inference_snapshot: None,
            recovered_from_corruption: false,
            schema_version: 0,
            api_url: None,
            api_key: None,
            inference_url: None,
            ephemeral_route: None,
            default_model: Some(DEFAULT_MODEL.to_string()),
            default_temperature: DEFAULT_TEMPERATURE,
            output_language: None,
            temperature_unsupported_models: default_temperature_unsupported_models(),
            observability: ObservabilityConfig::default(),
            dashboard: DashboardConfig::default(),
            autonomy: AutonomyConfig::default(),
            hooks: super::HooksConfig::default(),
            privacy: PrivacyConfig::default(),
            sandbox: SandboxConfig::default(),
            runtime: RuntimeConfig::default(),
            shell: ShellConfig::default(),
            reliability: ReliabilityConfig::default(),
            scheduler: SchedulerConfig::default(),
            scheduler_gate: SchedulerGateConfig::default(),
            agent_activity_level: AgentActivityLevel::default(),
            memory_sync_interval_secs: None,
            agent: AgentConfig::default(),
            orchestrator: OrchestratorModelConfig::default(),
            teams: HashMap::new(),
            context: ContextConfig::default(),
            model_routes: Vec::new(),
            embedding_routes: Vec::new(),
            heartbeat: HeartbeatConfig::default(),
            subconscious: crate::openhuman::config::schema::SubconsciousConfig::default(),
            cron: CronConfig::default(),
            task_sources: TaskSourcesConfig::default(),
            youpet: YouPetConfig::default(),
            channels_config: ChannelsConfig::default(),
            memory: MemoryConfig::default(),
            memory_tree: MemoryTreeConfig::default(),
            storage: StorageConfig::default(),
            subsystems: SubsystemsConfig::default(),
            composio: ComposioConfig::default(),
            secrets: SecretsConfig::default(),
            browser: BrowserConfig::default(),
            http_request: HttpRequestConfig::default(),
            curl: CurlConfig::default(),
            gitbooks: GitbooksConfig::default(),
            mcp_client: McpClientConfig::default(),
            modules: super::ModulesConfig::default(),
            capability_providers: Vec::new(),
            multimodal: MultimodalConfig::default(),
            multimodal_files: MultimodalFileConfig::default(),
            seltz: SeltzConfig::default(),
            searxng: SearxngConfig::default(),
            web_search: WebSearchConfig::default(),
            search: SearchConfig::default(),
            proxy: ProxyConfig::default(),
            cost: CostConfig::default(),
            memory_sources: Vec::new(),
            agent_registry: crate::openhuman::agent::registry::types::AgentRegistryConfig::default(
            ),
            agents: HashMap::new(),
            local_ai: LocalAiConfig::default(),
            claude_agent_sdk: ClaudeAgentSdkConfig::default(),
            cloud_providers: Vec::new(),
            primary_cloud: None,
            chat_provider: None,
            reasoning_provider: None,
            agentic_provider: None,
            coding_provider: None,
            vision_provider: None,
            memory_provider: None,
            embeddings_provider: None,
            heartbeat_provider: None,
            learning_provider: None,
            subconscious_provider: None,
            node: NodeConfig::default(),
            runtime_python: RuntimePythonConfig::default(),
            runtime_pool: RuntimePoolConfig::default(),
            tokenjuice: TokenjuiceConfig::default(),
            hosting: HostingConfig::default(),
            voice_server: VoiceServerConfig::default(),
            voice_providers: Vec::new(),
            stt_provider: None,
            tts_provider: None,
            integrations: IntegrationsConfig::default(),
            learning: LearningConfig::default(),
            update: UpdateConfig::default(),
            dictation: DictationConfig::default(),
            onboarding_completed: false,
            chat_onboarding_completed: false,
            model_registry: Vec::new(),
            composio_source_caps_migration_version: 0,
        }
    }
}
