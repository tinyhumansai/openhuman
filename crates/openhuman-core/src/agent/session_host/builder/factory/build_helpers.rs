use super::*;

pub(super) fn prompt_builder(
    target_def: Option<&crate::agent_registry::host_definition::AgentDefinition>,
    config: &Config,
) -> Result<SystemPromptBuilder> {
    Ok(match target_def {
        Some(def) => match &def.system_prompt {
            PromptSource::Dynamic(()) => SystemPromptBuilder::from_dynamic(def.prompt_fn()?),
            PromptSource::Verbatim(text) => SystemPromptBuilder::verbatim(text.clone()),
            PromptSource::Inline(text) => SystemPromptBuilder::for_subagent(
                text.clone(),
                def.extensions.omit_identity,
                def.extensions.omit_safety_preamble,
            ),
            PromptSource::File { path } => {
                let prompt_root = config.workspace_dir.join("agent").join("prompts");
                let workspace_path = prompt_root.join(path);
                let body_text = if workspace_path.is_file() {
                    match crate::security::validate_path_within_root(&workspace_path, &prompt_root)
                    {
                        Ok(resolved) => std::fs::read_to_string(&resolved).unwrap_or_else(|e| {
                            log::warn!(
                                "[agent::builder] failed to read prompt {}: {e} — using empty body",
                                workspace_path.display()
                            );
                            String::new()
                        }),
                        Err(e) => {
                            log::warn!(
                                "[agent::builder] prompt path rejected: {e} — using empty body"
                            );
                            String::new()
                        }
                    }
                } else {
                    log::debug!(
                        "[agent::builder] prompt file {} not found — using empty body",
                        path
                    );
                    String::new()
                };
                SystemPromptBuilder::for_subagent(
                    body_text,
                    def.extensions.omit_identity,
                    def.extensions.omit_safety_preamble,
                )
            }
        },
        None => SystemPromptBuilder::with_defaults(),
    })
}

pub(super) fn tool_dispatcher(
    tools: &[Box<dyn Tool>],
    delegation_tools: &[Box<dyn Tool>],
    dispatcher_choice: &str,
    supports_native: bool,
    agent_id: &str,
) -> Box<dyn ToolDialect> {
    // Build the P-Format registry AFTER the tool list is finalised
    // (including orchestrator tools) so every tool gets a signature
    // entry. The registry is self-contained — it doesn't hold a
    // reference back into the tools Vec.
    let pformat_registry = tinytools_agent::build_registry(
        tools
            .iter()
            .chain(delegation_tools.iter())
            .map(|tool| (tool.name(), tool.parameters_schema())),
    );
    let dispatcher_kind = ToolCallFormat::resolve(dispatcher_choice, supports_native);
    let tool_dispatcher: Box<dyn ToolDialect> = match dispatcher_kind {
        ToolCallFormat::Native => Box::new(NativeDialect),
        ToolCallFormat::Json => Box::new(XmlDialect),
        ToolCallFormat::PFormat => Box::new(PFormatDialect::new(pformat_registry.clone())),
        ToolCallFormat::Python => Box::new(CodeDialect::new(
            CodeStyle::Python,
            pformat_registry.clone(),
        )),
        ToolCallFormat::TypeScript => Box::new(CodeDialect::new(
            CodeStyle::TypeScript,
            pformat_registry.clone(),
        )),
    };

    log::debug!(
        "[agent] tool dispatcher selected: choice={dispatcher_choice} agent_id={agent_id} \
             kind={dispatcher_kind:?} sends_tool_specs={} pformat_registry_entries={}",
        tool_dispatcher.should_send_tool_specs(),
        pformat_registry.len()
    );
    tool_dispatcher
}

pub(super) fn effective_settings(
    target_def: Option<&crate::agent_registry::host_definition::AgentDefinition>,
    config: &Config,
) -> (f64, bool, tinyjuice_bus::types::AgentTokenjuiceCompression) {
    let effective_temperature = target_def
        .map(|def| def.temperature)
        .unwrap_or(config.default_temperature);
    let effective_omit_memory_context = target_def
        .map(|def| def.extensions.omit_memory_context)
        .unwrap_or(false);
    let effective_tokenjuice_compression = target_def
        .map(|def| def.extensions.tokenjuice_for(&def.model))
        .unwrap_or(tinyjuice_bus::types::AgentTokenjuiceCompression::Full);
    (
        effective_temperature,
        effective_omit_memory_context,
        effective_tokenjuice_compression,
    )
}

pub(super) fn effective_agent_config(
    config: &Config,
    target_def: Option<&crate::agent_registry::host_definition::AgentDefinition>,
    agent_id: &str,
) -> crate::config::AgentConfig {
    // Resolve the per-turn iteration cap in one place so an explicit
    // operator override takes precedence over the selected definition,
    // while definitions still take precedence over the global default.
    let mut effective_agent_config = config.agent.clone();
    let effective_cap =
        super::super::super::iteration_cap::resolve_max_tool_iterations(&config.agent, target_def);
    if let Some(def) = target_def {
        log::info!(
            "[agent::builder] resolved iteration cap for agent_id={}: \
         definition.max_iterations={} iteration_policy={:?} -> effective={} \
         (global default {}, explicit override {:?})",
            agent_id,
            def.max_iterations,
            def.iteration_policy,
            effective_cap,
            config.agent.max_tool_iterations,
            config.agent.max_tool_iterations_override,
        );
    }
    effective_agent_config.max_tool_iterations = effective_cap;
    effective_agent_config
}

pub(super) fn log_agent_definition_name(agent_id: &str) {
    // Keep the definition stamp visible at the point the session is built.
    log::debug!(
        "[agent::builder] stamping agent_definition_name={} onto session agent",
        agent_id
    );
}

pub(super) fn payload_summarizer(
    host_only: bool,
    agent_id: &str,
    config: &Config,
) -> Option<std::sync::Arc<crate::agent::tinyagents::payload_summarizer::DynPayloadSummarizer>> {
    if !host_only && super::super::super::summarizes_tool_output(agent_id, config) {
        match crate::agent_registry::host_definition::AgentDefinitionRegistry::current() {
            Some(reg) => match reg.get("summarizer") {
                Some(summarizer_def) => {
                    log::info!(
                        "[agent::builder] wiring payload_summarizer for orchestrator: \
                     threshold_tokens={} max_tokens={}",
                        config.context.summarizer_payload_threshold_tokens,
                        config.context.summarizer_max_payload_tokens
                    );
                    Some(std::sync::Arc::new(
                    crate::agent::tinyagents::payload_summarizer::SubagentPayloadSummarizer::new(
                        summarizer_def.clone(),
                    ),
                ))
                }
                None => {
                    log::warn!(
                        "[agent::builder] orchestrator requested payload_summarizer but \
                     `summarizer` definition is not in the registry — proceeding without it"
                    );
                    None
                }
            },
            None => {
                log::warn!(
                    "[agent::builder] orchestrator requested payload_summarizer but \
                 AgentDefinitionRegistry is not initialised — proceeding without it"
                );
                None
            }
        }
    } else {
        None
    }
}
