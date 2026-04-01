use super::dispatcher::{
    NativeToolDispatcher, ParsedToolCall, ToolDispatcher, ToolExecutionResult, XmlToolDispatcher,
};
use super::hooks::{self, sanitize_tool_output, PostTurnHook, ToolCallRecord, TurnContext};
use super::memory_loader::{DefaultMemoryLoader, MemoryLoader};
use super::prompt::{PromptContext, SystemPromptBuilder};
use crate::openhuman::agent::host_runtime;
use crate::openhuman::config::Config;
use crate::openhuman::memory::{self, Memory, MemoryCategory};
use crate::openhuman::providers::{
    self, ChatMessage, ChatRequest, ConversationMessage, Provider, ToolCall,
};
use crate::openhuman::security::SecurityPolicy;
use crate::openhuman::tools::{self, Tool, ToolSpec};
use crate::openhuman::util::truncate_with_ellipsis;
use anyhow::Result;
use std::io::Write as IoWrite;
use std::sync::Arc;

pub struct Agent {
    provider: Box<dyn Provider>,
    tools: Vec<Box<dyn Tool>>,
    tool_specs: Vec<ToolSpec>,
    memory: Arc<dyn Memory>,
    prompt_builder: SystemPromptBuilder,
    tool_dispatcher: Box<dyn ToolDispatcher>,
    memory_loader: Box<dyn MemoryLoader>,
    config: crate::openhuman::config::AgentConfig,
    model_name: String,
    temperature: f64,
    workspace_dir: std::path::PathBuf,
    identity_config: crate::openhuman::config::IdentityConfig,
    skills: Vec<crate::openhuman::skills::Skill>,
    auto_save: bool,
    history: Vec<ConversationMessage>,
    classification_config: crate::openhuman::config::QueryClassificationConfig,
    available_hints: Vec<String>,
    post_turn_hooks: Vec<Arc<dyn PostTurnHook>>,
    learning_enabled: bool,
}

pub struct AgentBuilder {
    provider: Option<Box<dyn Provider>>,
    tools: Option<Vec<Box<dyn Tool>>>,
    memory: Option<Arc<dyn Memory>>,
    prompt_builder: Option<SystemPromptBuilder>,
    tool_dispatcher: Option<Box<dyn ToolDispatcher>>,
    memory_loader: Option<Box<dyn MemoryLoader>>,
    config: Option<crate::openhuman::config::AgentConfig>,
    model_name: Option<String>,
    temperature: Option<f64>,
    workspace_dir: Option<std::path::PathBuf>,
    identity_config: Option<crate::openhuman::config::IdentityConfig>,
    skills: Option<Vec<crate::openhuman::skills::Skill>>,
    auto_save: Option<bool>,
    classification_config: Option<crate::openhuman::config::QueryClassificationConfig>,
    available_hints: Option<Vec<String>>,
    post_turn_hooks: Vec<Arc<dyn PostTurnHook>>,
    learning_enabled: bool,
}

impl Default for AgentBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentBuilder {
    pub fn new() -> Self {
        Self {
            provider: None,
            tools: None,
            memory: None,
            prompt_builder: None,
            tool_dispatcher: None,
            memory_loader: None,
            config: None,
            model_name: None,
            temperature: None,
            workspace_dir: None,
            identity_config: None,
            skills: None,
            auto_save: None,
            classification_config: None,
            available_hints: None,
            post_turn_hooks: Vec::new(),
            learning_enabled: false,
        }
    }

    pub fn provider(mut self, provider: Box<dyn Provider>) -> Self {
        self.provider = Some(provider);
        self
    }

    pub fn tools(mut self, tools: Vec<Box<dyn Tool>>) -> Self {
        self.tools = Some(tools);
        self
    }

    pub fn memory(mut self, memory: Arc<dyn Memory>) -> Self {
        self.memory = Some(memory);
        self
    }

    pub fn prompt_builder(mut self, prompt_builder: SystemPromptBuilder) -> Self {
        self.prompt_builder = Some(prompt_builder);
        self
    }

    pub fn tool_dispatcher(mut self, tool_dispatcher: Box<dyn ToolDispatcher>) -> Self {
        self.tool_dispatcher = Some(tool_dispatcher);
        self
    }

    pub fn memory_loader(mut self, memory_loader: Box<dyn MemoryLoader>) -> Self {
        self.memory_loader = Some(memory_loader);
        self
    }

    pub fn config(mut self, config: crate::openhuman::config::AgentConfig) -> Self {
        self.config = Some(config);
        self
    }

    pub fn model_name(mut self, model_name: String) -> Self {
        self.model_name = Some(model_name);
        self
    }

    pub fn temperature(mut self, temperature: f64) -> Self {
        self.temperature = Some(temperature);
        self
    }

    pub fn workspace_dir(mut self, workspace_dir: std::path::PathBuf) -> Self {
        self.workspace_dir = Some(workspace_dir);
        self
    }

    pub fn identity_config(
        mut self,
        identity_config: crate::openhuman::config::IdentityConfig,
    ) -> Self {
        self.identity_config = Some(identity_config);
        self
    }

    pub fn skills(mut self, skills: Vec<crate::openhuman::skills::Skill>) -> Self {
        self.skills = Some(skills);
        self
    }

    pub fn auto_save(mut self, auto_save: bool) -> Self {
        self.auto_save = Some(auto_save);
        self
    }

    pub fn classification_config(
        mut self,
        classification_config: crate::openhuman::config::QueryClassificationConfig,
    ) -> Self {
        self.classification_config = Some(classification_config);
        self
    }

    pub fn available_hints(mut self, available_hints: Vec<String>) -> Self {
        self.available_hints = Some(available_hints);
        self
    }

    pub fn post_turn_hooks(mut self, hooks: Vec<Arc<dyn PostTurnHook>>) -> Self {
        self.post_turn_hooks = hooks;
        self
    }

    pub fn learning_enabled(mut self, enabled: bool) -> Self {
        self.learning_enabled = enabled;
        self
    }

    pub fn build(self) -> Result<Agent> {
        let tools = self
            .tools
            .ok_or_else(|| anyhow::anyhow!("tools are required"))?;
        let tool_specs = tools.iter().map(|tool| tool.spec()).collect();

        Ok(Agent {
            provider: self
                .provider
                .ok_or_else(|| anyhow::anyhow!("provider is required"))?,
            tools,
            tool_specs,
            memory: self
                .memory
                .ok_or_else(|| anyhow::anyhow!("memory is required"))?,
            prompt_builder: self
                .prompt_builder
                .unwrap_or_else(SystemPromptBuilder::with_defaults),
            tool_dispatcher: self
                .tool_dispatcher
                .ok_or_else(|| anyhow::anyhow!("tool_dispatcher is required"))?,
            memory_loader: self
                .memory_loader
                .unwrap_or_else(|| Box::new(DefaultMemoryLoader::default())),
            config: self.config.unwrap_or_default(),
            model_name: self.model_name.unwrap_or_else(|| "neocortex-mk1".into()),
            temperature: self.temperature.unwrap_or(0.7),
            workspace_dir: self
                .workspace_dir
                .unwrap_or_else(|| std::path::PathBuf::from(".")),
            identity_config: self.identity_config.unwrap_or_default(),
            skills: self.skills.unwrap_or_default(),
            auto_save: self.auto_save.unwrap_or(false),
            history: Vec::new(),
            classification_config: self.classification_config.unwrap_or_default(),
            available_hints: self.available_hints.unwrap_or_default(),
            post_turn_hooks: self.post_turn_hooks,
            learning_enabled: self.learning_enabled,
        })
    }
}

impl Agent {
    fn with_fallback_tool_call_ids(
        mut parsed_calls: Vec<ParsedToolCall>,
        iteration: usize,
    ) -> Vec<ParsedToolCall> {
        for (idx, call) in parsed_calls.iter_mut().enumerate() {
            if call.tool_call_id.is_none() {
                call.tool_call_id = Some(format!("parsed-{}-{}", iteration + 1, idx + 1));
            }
        }
        parsed_calls
    }

    fn persisted_tool_calls_for_history(
        response: &crate::openhuman::providers::ChatResponse,
        parsed_calls: &[ParsedToolCall],
        iteration: usize,
    ) -> Vec<ToolCall> {
        if !response.tool_calls.is_empty() {
            return response.tool_calls.clone();
        }

        parsed_calls
            .iter()
            .enumerate()
            .map(|(idx, call)| ToolCall {
                id: call
                    .tool_call_id
                    .clone()
                    .unwrap_or_else(|| format!("parsed-{}-{}", iteration + 1, idx + 1)),
                name: call.name.clone(),
                arguments: call.arguments.to_string(),
            })
            .collect()
    }

    pub fn builder() -> AgentBuilder {
        AgentBuilder::new()
    }

    pub fn history(&self) -> &[ConversationMessage] {
        &self.history
    }

    pub fn clear_history(&mut self) {
        self.history.clear();
    }

    pub fn from_config(config: &Config) -> Result<Self> {
        let runtime: Arc<dyn host_runtime::RuntimeAdapter> =
            Arc::from(host_runtime::create_runtime(&config.runtime)?);
        let security = Arc::new(SecurityPolicy::from_config(
            &config.autonomy,
            &config.workspace_dir,
        ));

        let memory: Arc<dyn Memory> = Arc::from(memory::create_memory_with_storage_and_routes(
            &config.memory,
            &config.embedding_routes,
            Some(&config.storage.provider.config),
            &config.workspace_dir,
            config.api_key.as_deref(),
        )?);

        let composio_key = if config.composio.enabled {
            config.composio.api_key.as_deref()
        } else {
            None
        };
        let composio_entity_id = if config.composio.enabled {
            Some(config.composio.entity_id.as_str())
        } else {
            None
        };

        let tools = tools::all_tools_with_runtime(
            Arc::new(config.clone()),
            &security,
            runtime,
            memory.clone(),
            composio_key,
            composio_entity_id,
            &config.browser,
            &config.http_request,
            &config.workspace_dir,
            &config.agents,
            config.api_key.as_deref(),
            config,
        );

        let model_name = config
            .default_model
            .as_deref()
            .unwrap_or("neocortex-mk1")
            .to_string();

        let provider: Box<dyn Provider> = providers::create_routed_provider(
            config.api_key.as_deref(),
            config.api_url.as_deref(),
            &config.reliability,
            &config.model_routes,
            &model_name,
        )?;

        let dispatcher_choice = config.agent.tool_dispatcher.as_str();
        let tool_dispatcher: Box<dyn ToolDispatcher> = match dispatcher_choice {
            "native" => Box::new(NativeToolDispatcher),
            "xml" => Box::new(XmlToolDispatcher),
            _ if provider.supports_native_tools() => Box::new(NativeToolDispatcher),
            _ => Box::new(XmlToolDispatcher),
        };

        let available_hints: Vec<String> =
            config.model_routes.iter().map(|r| r.hint.clone()).collect();

        // Build prompt builder, optionally with learning sections
        let mut prompt_builder = SystemPromptBuilder::with_defaults();
        if config.learning.enabled {
            prompt_builder = prompt_builder
                .add_section(Box::new(
                    crate::openhuman::learning::LearnedContextSection::new(memory.clone()),
                ))
                .add_section(Box::new(
                    crate::openhuman::learning::UserProfileSection::new(memory.clone()),
                ));
            log::info!("[learning] prompt sections registered (learned_context, user_profile)");
        }

        // Build post-turn hooks when learning is enabled
        let mut post_turn_hooks: Vec<Arc<dyn super::hooks::PostTurnHook>> = Vec::new();
        if config.learning.enabled {
            let full_config = Arc::new(config.clone());

            if config.learning.reflection_enabled {
                // For cloud reflection, wrap the provider in an Arc.
                // For local, no provider needed.
                let reflection_provider: Option<Arc<dyn crate::openhuman::providers::Provider>> =
                    if config.learning.reflection_source
                        == crate::openhuman::config::ReflectionSource::Cloud
                    {
                        Some(Arc::from(providers::create_routed_provider(
                            config.api_key.as_deref(),
                            config.api_url.as_deref(),
                            &config.reliability,
                            &config.model_routes,
                            &model_name,
                        )?))
                    } else {
                        None
                    };
                post_turn_hooks.push(Arc::new(crate::openhuman::learning::ReflectionHook::new(
                    config.learning.clone(),
                    full_config.clone(),
                    memory.clone(),
                    reflection_provider,
                )));
                log::info!(
                    "[learning] reflection hook registered (source={:?})",
                    config.learning.reflection_source
                );
            }

            if config.learning.user_profile_enabled {
                post_turn_hooks.push(Arc::new(crate::openhuman::learning::UserProfileHook::new(
                    config.learning.clone(),
                    memory.clone(),
                )));
                log::info!("[learning] user_profile hook registered");
            }

            if config.learning.tool_tracking_enabled {
                post_turn_hooks.push(Arc::new(crate::openhuman::learning::ToolTrackerHook::new(
                    config.learning.clone(),
                    memory.clone(),
                )));
                log::info!("[learning] tool_tracker hook registered");
            }
        }

        Agent::builder()
            .provider(provider)
            .tools(tools)
            .memory(memory)
            .tool_dispatcher(tool_dispatcher)
            .memory_loader(Box::new(
                DefaultMemoryLoader::new(5, config.memory.min_relevance_score)
                    .with_max_chars(config.agent.max_memory_context_chars),
            ))
            .prompt_builder(prompt_builder)
            .config(config.agent.clone())
            .model_name(model_name)
            .temperature(config.default_temperature)
            .workspace_dir(config.workspace_dir.clone())
            .classification_config(config.query_classification.clone())
            .available_hints(available_hints)
            .identity_config(config.identity.clone())
            .skills(crate::openhuman::skills::load_skills(&config.workspace_dir))
            .auto_save(config.memory.auto_save)
            .post_turn_hooks(post_turn_hooks)
            .learning_enabled(config.learning.enabled)
            .build()
    }

    fn trim_history(&mut self) {
        let max = self.config.max_history_messages;
        if self.history.len() <= max {
            return;
        }

        let mut system_messages = Vec::new();
        let mut other_messages = Vec::new();

        for msg in self.history.drain(..) {
            match &msg {
                ConversationMessage::Chat(chat) if chat.role == "system" => {
                    system_messages.push(msg);
                }
                _ => other_messages.push(msg),
            }
        }

        if other_messages.len() > max {
            let drop_count = other_messages.len() - max;
            other_messages.drain(0..drop_count);
        }

        self.history = system_messages;
        self.history.extend(other_messages);
    }

    /// Pre-fetch learned context data from memory (async, non-blocking).
    async fn fetch_learned_context(&self) -> crate::openhuman::agent::prompt::LearnedContextData {
        use crate::openhuman::agent::prompt::LearnedContextData;

        if !self.learning_enabled {
            return LearnedContextData::default();
        }

        let obs_entries = self
            .memory
            .list(
                Some(&MemoryCategory::Custom("learning_observations".into())),
                None,
            )
            .await
            .unwrap_or_default();

        let pat_entries = self
            .memory
            .list(
                Some(&MemoryCategory::Custom("learning_patterns".into())),
                None,
            )
            .await
            .unwrap_or_default();

        let profile_entries = self
            .memory
            .list(Some(&MemoryCategory::Custom("user_profile".into())), None)
            .await
            .unwrap_or_default();

        LearnedContextData {
            observations: obs_entries
                .iter()
                .rev()
                .take(5)
                .map(|e| sanitize_learned_entry(&e.content))
                .collect(),
            patterns: pat_entries
                .iter()
                .take(3)
                .map(|e| sanitize_learned_entry(&e.content))
                .collect(),
            user_profile: profile_entries
                .iter()
                .take(20)
                .map(|e| sanitize_learned_entry(&e.content))
                .collect(),
        }
    }

    fn build_system_prompt(
        &self,
        learned: crate::openhuman::agent::prompt::LearnedContextData,
    ) -> Result<String> {
        let instructions = self.tool_dispatcher.prompt_instructions(&self.tools);
        let ctx = PromptContext {
            workspace_dir: &self.workspace_dir,
            model_name: &self.model_name,
            tools: &self.tools,
            skills: &self.skills,
            identity_config: Some(&self.identity_config),
            dispatcher_instructions: &instructions,
            learned,
        };
        self.prompt_builder.build(&ctx)
    }

    /// Sanitize tool output to prevent PII/secrets in learning data.
    /// Returns a safe metadata string: tool type, status, and error class.
    fn sanitize_tool_output(raw_output: &str, tool_name: &str, success: bool) -> String {
        if success {
            // For successful calls, return a structured summary without raw data
            let char_count = raw_output.chars().count();
            format!(
                "tool={} status=success output_length={}",
                tool_name, char_count
            )
        } else {
            // For errors, classify the error type without exposing details
            let error_class = if raw_output.contains("permission") || raw_output.contains("denied")
            {
                "permission_error"
            } else if raw_output.contains("not found") || raw_output.contains("404") {
                "not_found"
            } else if raw_output.contains("timeout") || raw_output.contains("timed out") {
                "timeout"
            } else if raw_output.contains("network") || raw_output.contains("connection") {
                "network_error"
            } else if raw_output.contains("invalid") || raw_output.contains("parse") {
                "validation_error"
            } else {
                "unknown_error"
            };
            format!("tool={} status=error class={}", tool_name, error_class)
        }
    }

    async fn execute_tool_call(
        &self,
        call: &ParsedToolCall,
    ) -> (ToolExecutionResult, ToolCallRecord) {
        let started = std::time::Instant::now();
        log::info!("[agent_loop] tool start name={}", call.name);
        let (result, success) =
            if let Some(tool) = self.tools.iter().find(|t| t.name() == call.name) {
                match tool.execute(call.arguments.clone()).await {
                    Ok(r) => {
                        if r.success {
                            (r.output, true)
                        } else {
                            (format!("Error: {}", r.error.unwrap_or(r.output)), false)
                        }
                    }
                    Err(e) => (format!("Error executing {}: {e}", call.name), false),
                }
            } else {
                (format!("Unknown tool: {}", call.name), false)
            };
        let elapsed_ms = started.elapsed().as_millis() as u64;
        log::info!(
            "[agent_loop] tool finish name={} elapsed_ms={} output_chars={} success={}",
            call.name,
            elapsed_ms,
            result.chars().count(),
            success
        );

        let output_summary = sanitize_tool_output(&result, &call.name, success);

        let record = ToolCallRecord {
            name: call.name.clone(),
            arguments: call.arguments.clone(),
            success,
            output_summary,
            duration_ms: elapsed_ms,
        };

        let exec_result = ToolExecutionResult {
            name: call.name.clone(),
            output: result,
            success,
            tool_call_id: call.tool_call_id.clone(),
        };

        (exec_result, record)
    }

    async fn execute_tools(
        &self,
        calls: &[ParsedToolCall],
    ) -> (Vec<ToolExecutionResult>, Vec<ToolCallRecord>) {
        let mut results = Vec::with_capacity(calls.len());
        let mut records = Vec::with_capacity(calls.len());
        for call in calls {
            let (exec_result, record) = self.execute_tool_call(call).await;
            results.push(exec_result);
            records.push(record);
        }
        (results, records)
    }

    fn classify_model(&self, user_message: &str) -> String {
        if let Some(hint) = super::classifier::classify(&self.classification_config, user_message) {
            if self.available_hints.contains(&hint) {
                tracing::info!(hint = hint.as_str(), "Auto-classified query");
                return format!("hint:{hint}");
            }
        }
        self.model_name.clone()
    }

    pub async fn turn(&mut self, user_message: &str) -> Result<String> {
        let turn_started = std::time::Instant::now();
        log::info!(
            "[agent_loop] turn start message_chars={} history_len={} max_tool_iterations={}",
            user_message.chars().count(),
            self.history.len(),
            self.config.max_tool_iterations
        );
        // Pre-fetch learned context async before building the system prompt
        let learned = self.fetch_learned_context().await;

        if self.history.is_empty() {
            let system_prompt = self.build_system_prompt(learned)?;
            log::info!(
                "[agent_loop] system prompt built chars={} content=\n{}",
                system_prompt.chars().count(),
                system_prompt
            );
            self.history
                .push(ConversationMessage::Chat(ChatMessage::system(
                    system_prompt,
                )));
        } else if self.learning_enabled {
            // Rebuild system prompt on subsequent turns to include newly learned context
            let system_prompt = self.build_system_prompt(learned)?;
            if let Some(pos) = self.history.iter().position(
                |msg| matches!(msg, ConversationMessage::Chat(chat) if chat.role == "system"),
            ) {
                self.history[pos] = ConversationMessage::Chat(ChatMessage::system(system_prompt));
                log::debug!("[agent_loop] system prompt refreshed with learned context");
            }
        }

        if self.auto_save {
            let _ = self
                .memory
                .store("user_msg", user_message, MemoryCategory::Conversation, None)
                .await;
        }

        let context = self
            .memory_loader
            .load_context(self.memory.as_ref(), user_message)
            .await
            .unwrap_or_default();

        let enriched = if context.is_empty() {
            user_message.to_string()
        } else {
            format!("{context}{user_message}")
        };

        self.history
            .push(ConversationMessage::Chat(ChatMessage::user(enriched)));

        let effective_model = self.classify_model(user_message);
        log::info!("[agent_loop] model selected model={}", effective_model);

        // Collect tool call records across all iterations for post-turn hooks
        let mut all_tool_records: Vec<ToolCallRecord> = Vec::new();

        for iteration in 0..self.config.max_tool_iterations {
            log::info!(
                "[agent_loop] iteration start i={} history_len={}",
                iteration + 1,
                self.history.len()
            );
            let messages = self.tool_dispatcher.to_provider_messages(&self.history);
            log::info!(
                "[agent_loop] provider request i={} messages={} send_tool_specs={}",
                iteration + 1,
                messages.len(),
                self.tool_dispatcher.should_send_tool_specs()
            );
            let provider_started = std::time::Instant::now();
            let response = match self
                .provider
                .chat(
                    ChatRequest {
                        messages: &messages,
                        tools: if self.tool_dispatcher.should_send_tool_specs() {
                            Some(&self.tool_specs)
                        } else {
                            None
                        },
                        system_prompt_cache_boundary: None,
                    },
                    &effective_model,
                    self.temperature,
                )
                .await
            {
                Ok(resp) => {
                    log::info!(
                        "[agent_loop] provider response i={} elapsed_ms={} text_chars={} native_tool_calls={}",
                        iteration + 1,
                        provider_started.elapsed().as_millis(),
                        resp.text.as_ref().map_or(0, |t| t.chars().count()),
                        resp.tool_calls.len()
                    );
                    log::debug!("[agent_loop] provider response: {resp:?}");
                    resp
                }
                Err(err) => return Err(err),
            };

            let (text, calls) = self.tool_dispatcher.parse_response(&response);
            let calls = Self::with_fallback_tool_call_ids(calls, iteration);
            log::info!(
                "[agent_loop] parsed response i={} parsed_text_chars={} parsed_tool_calls={}",
                iteration + 1,
                text.chars().count(),
                calls.len()
            );
            if calls.is_empty() {
                let final_text = if text.is_empty() {
                    response.text.unwrap_or_default()
                } else {
                    text
                };
                log::info!(
                    "[agent_loop] final response i={} final_chars={}",
                    iteration + 1,
                    final_text.chars().count()
                );

                self.history
                    .push(ConversationMessage::Chat(ChatMessage::assistant(
                        final_text.clone(),
                    )));
                self.trim_history();

                if self.auto_save {
                    let summary = truncate_with_ellipsis(&final_text, 100);
                    let _ = self
                        .memory
                        .store("assistant_resp", &summary, MemoryCategory::Daily, None)
                        .await;
                }

                // Fire post-turn hooks (non-blocking)
                if !self.post_turn_hooks.is_empty() {
                    let ctx = TurnContext {
                        user_message: user_message.to_string(),
                        assistant_response: final_text.clone(),
                        tool_calls: all_tool_records,
                        turn_duration_ms: turn_started.elapsed().as_millis() as u64,
                        session_id: None,
                        iteration_count: iteration + 1,
                    };
                    hooks::fire_hooks(&self.post_turn_hooks, ctx);
                }

                return Ok(final_text);
            }

            if !text.is_empty() {
                log::info!(
                    "[agent_loop] assistant pre-tool text i={} chars={}",
                    iteration + 1,
                    text.chars().count()
                );
                self.history
                    .push(ConversationMessage::Chat(ChatMessage::assistant(
                        text.clone(),
                    )));
                print!("{text}");
                let _ = std::io::stdout().flush();
            }
            let tool_names: Vec<&str> = calls.iter().map(|call| call.name.as_str()).collect();
            log::info!(
                "[agent_loop] executing tools i={} names={:?}",
                iteration + 1,
                tool_names
            );
            let persisted_tool_calls =
                Self::persisted_tool_calls_for_history(&response, &calls, iteration);
            log::info!(
                "[agent_loop] persisting assistant tool calls i={} persisted_tool_calls={} parsed_tool_calls={}",
                iteration + 1,
                persisted_tool_calls.len(),
                calls.len()
            );
            self.history.push(ConversationMessage::AssistantToolCalls {
                text: if text.is_empty() {
                    None
                } else {
                    Some(text.clone())
                },
                tool_calls: persisted_tool_calls,
            });

            let (results, records) = self.execute_tools(&calls).await;
            all_tool_records.extend(records);
            log::info!(
                "[agent_loop] tool results complete i={} result_count={}",
                iteration + 1,
                results.len()
            );
            let formatted = self.tool_dispatcher.format_results(&results);
            self.history.push(formatted);
            self.trim_history();
            log::info!(
                "[agent_loop] iteration end i={} history_len={}",
                iteration + 1,
                self.history.len()
            );
        }

        log::warn!(
            "[agent_loop] exceeded maximum tool iterations max={}",
            self.config.max_tool_iterations
        );
        anyhow::bail!(
            "Agent exceeded maximum tool iterations ({})",
            self.config.max_tool_iterations
        )
    }

    pub async fn run_single(&mut self, message: &str) -> Result<String> {
        self.turn(message).await
    }

    pub async fn run_interactive(&mut self) -> Result<()> {
        println!("🦀 OpenHuman Interactive Mode");
        println!("Type /quit to exit.\n");

        let (tx, mut rx) = tokio::sync::mpsc::channel(32);
        let cli = crate::openhuman::channels::CliChannel::new();

        let listen_handle = tokio::spawn(async move {
            let _ = crate::openhuman::channels::Channel::listen(&cli, tx).await;
        });

        while let Some(msg) = rx.recv().await {
            let response = match self.turn(&msg.content).await {
                Ok(resp) => resp,
                Err(e) => {
                    eprintln!("\nError: {e}\n");
                    continue;
                }
            };
            println!("\n{response}\n");
        }

        listen_handle.abort();
        Ok(())
    }
}

pub async fn run(
    config: Config,
    message: Option<String>,
    model_override: Option<String>,
    temperature: f64,
) -> Result<()> {
    let mut effective_config = config;
    if let Some(m) = model_override {
        effective_config.default_model = Some(m);
    }
    effective_config.default_temperature = temperature;

    let mut agent = Agent::from_config(&effective_config)?;

    if let Some(msg) = message {
        let response = agent.run_single(&msg).await?;
        println!("{response}");
    } else {
        agent.run_interactive().await?;
    }

    Ok(())
}

/// Sanitize a learned memory entry before injecting into the system prompt.
/// Strips raw data, limits length, and removes potential secrets.
fn sanitize_learned_entry(content: &str) -> String {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    // Truncate to a safe length
    let max_len = 200;
    let sanitized: String = trimmed.chars().take(max_len).collect();
    // Strip anything that looks like a secret/token
    if sanitized.contains("Bearer ")
        || sanitized.contains("sk-")
        || sanitized.contains("ghp_")
        || sanitized.contains("-----BEGIN")
    {
        return "[redacted: potential secret]".to_string();
    }
    sanitized
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use parking_lot::Mutex;

    struct MockProvider {
        responses: Mutex<Vec<crate::openhuman::providers::ChatResponse>>,
    }

    #[async_trait]
    impl Provider for MockProvider {
        async fn chat_with_system(
            &self,
            _system_prompt: Option<&str>,
            _message: &str,
            _model: &str,
            _temperature: f64,
        ) -> Result<String> {
            Ok("ok".into())
        }

        async fn chat(
            &self,
            _request: ChatRequest<'_>,
            _model: &str,
            _temperature: f64,
        ) -> Result<crate::openhuman::providers::ChatResponse> {
            let mut guard = self.responses.lock();
            if guard.is_empty() {
                return Ok(crate::openhuman::providers::ChatResponse {
                    text: Some("done".into()),
                    tool_calls: vec![],
                    usage: None,
                });
            }
            Ok(guard.remove(0))
        }
    }

    struct MockTool;

    #[async_trait]
    impl Tool for MockTool {
        fn name(&self) -> &str {
            "echo"
        }

        fn description(&self) -> &str {
            "echo"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }

        async fn execute(
            &self,
            _args: serde_json::Value,
        ) -> Result<crate::openhuman::tools::ToolResult> {
            Ok(crate::openhuman::tools::ToolResult {
                success: true,
                output: "tool-out".into(),
                error: None,
            })
        }
    }

    #[tokio::test]
    async fn turn_without_tools_returns_text() {
        let workspace = tempfile::TempDir::new().expect("temp workspace");
        let workspace_path = workspace.path().to_path_buf();

        let provider = Box::new(MockProvider {
            responses: Mutex::new(vec![crate::openhuman::providers::ChatResponse {
                text: Some("hello".into()),
                tool_calls: vec![],
                usage: None,
            }]),
        });

        let memory_cfg = crate::openhuman::config::MemoryConfig {
            backend: "none".into(),
            ..crate::openhuman::config::MemoryConfig::default()
        };
        let mem: Arc<dyn Memory> = Arc::from(
            crate::openhuman::memory::create_memory(&memory_cfg, &workspace_path, None).unwrap(),
        );

        let mut agent = Agent::builder()
            .provider(provider)
            .tools(vec![Box::new(MockTool)])
            .memory(mem)
            .tool_dispatcher(Box::new(XmlToolDispatcher))
            .workspace_dir(workspace_path)
            .build()
            .unwrap();

        let response = agent.turn("hi").await.unwrap();
        assert_eq!(response, "hello");
    }

    #[tokio::test]
    async fn turn_with_native_dispatcher_handles_tool_results_variant() {
        let workspace = tempfile::TempDir::new().expect("temp workspace");
        let workspace_path = workspace.path().to_path_buf();

        let provider = Box::new(MockProvider {
            responses: Mutex::new(vec![
                crate::openhuman::providers::ChatResponse {
                    text: Some(String::new()),
                    tool_calls: vec![crate::openhuman::providers::ToolCall {
                        id: "tc1".into(),
                        name: "echo".into(),
                        arguments: "{}".into(),
                    }],
                    usage: None,
                },
                crate::openhuman::providers::ChatResponse {
                    text: Some("done".into()),
                    tool_calls: vec![],
                    usage: None,
                },
            ]),
        });

        let memory_cfg = crate::openhuman::config::MemoryConfig {
            backend: "none".into(),
            ..crate::openhuman::config::MemoryConfig::default()
        };
        let mem: Arc<dyn Memory> = Arc::from(
            crate::openhuman::memory::create_memory(&memory_cfg, &workspace_path, None).unwrap(),
        );

        let mut agent = Agent::builder()
            .provider(provider)
            .tools(vec![Box::new(MockTool)])
            .memory(mem)
            .tool_dispatcher(Box::new(NativeToolDispatcher))
            .workspace_dir(workspace_path)
            .build()
            .unwrap();

        let response = agent.turn("hi").await.unwrap();
        assert_eq!(response, "done");
        assert!(agent
            .history()
            .iter()
            .any(|msg| matches!(msg, ConversationMessage::ToolResults(_))));
    }

    #[tokio::test]
    async fn turn_with_native_dispatcher_persists_fallback_tool_calls() {
        let workspace = tempfile::TempDir::new().expect("temp workspace");
        let workspace_path = workspace.path().to_path_buf();

        let provider = Box::new(MockProvider {
            responses: Mutex::new(vec![
                crate::openhuman::providers::ChatResponse {
                    text: Some(
                        "Checking...\n<tool_call>{\"name\":\"echo\",\"arguments\":{}}</tool_call>"
                            .into(),
                    ),
                    tool_calls: vec![],
                    usage: None,
                },
                crate::openhuman::providers::ChatResponse {
                    text: Some("done".into()),
                    tool_calls: vec![],
                    usage: None,
                },
            ]),
        });

        let memory_cfg = crate::openhuman::config::MemoryConfig {
            backend: "none".into(),
            ..crate::openhuman::config::MemoryConfig::default()
        };
        let mem: Arc<dyn Memory> = Arc::from(
            crate::openhuman::memory::create_memory(&memory_cfg, &workspace_path, None).unwrap(),
        );

        let mut agent = Agent::builder()
            .provider(provider)
            .tools(vec![Box::new(MockTool)])
            .memory(mem)
            .tool_dispatcher(Box::new(NativeToolDispatcher))
            .workspace_dir(workspace_path)
            .build()
            .unwrap();

        let response = agent.turn("hi").await.unwrap();
        assert_eq!(response, "done");

        let persisted_calls = agent
            .history()
            .iter()
            .find_map(|msg| match msg {
                ConversationMessage::AssistantToolCalls { tool_calls, .. } => Some(tool_calls),
                _ => None,
            })
            .expect("assistant tool calls should be persisted");
        assert_eq!(persisted_calls.len(), 1);
        assert_eq!(persisted_calls[0].name, "echo");
    }
}
