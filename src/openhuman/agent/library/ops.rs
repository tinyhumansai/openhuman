use crate::openhuman::agent::harness::definition::{
    AgentDefinition, AgentDefinitionRegistry, AgentTier, DefinitionSource, ModelSpec, SandboxMode,
    SubagentEntry, ToolScope,
};
use crate::openhuman::config::rpc as config_rpc;

use super::types::{AgentDefinitionDisplay, AgentDefinitionModel, AgentDefinitionSource};

pub async fn list_definition_metadata() -> Result<Vec<AgentDefinitionDisplay>, String> {
    tracing::debug!("[rpc][agent_library][entry] list_definitions");
    if AgentDefinitionRegistry::global().is_none() {
        let config = config_rpc::load_config_with_timeout().await?;
        AgentDefinitionRegistry::init_global(&config.workspace_dir)
            .map_err(|e| format!("failed to initialise AgentDefinitionRegistry: {e}"))?;
    }
    let registry = AgentDefinitionRegistry::global()
        .ok_or_else(|| "AgentDefinitionRegistry not initialised".to_string())?;
    let definitions = registry
        .list()
        .into_iter()
        .map(metadata_from_definition)
        .collect::<Vec<_>>();
    tracing::debug!(
        count = definitions.len(),
        "[rpc][agent_library][exit] list_definitions"
    );
    Ok(definitions)
}

pub fn metadata_from_definition(def: &AgentDefinition) -> AgentDefinitionDisplay {
    let direct_tool_names = direct_tool_names(def);
    let uses_wildcard_tools = matches!(def.tools, ToolScope::Wildcard);
    let subagent_ids = def
        .subagents
        .iter()
        .filter_map(|entry| match entry {
            SubagentEntry::AgentId(id) => Some(id.clone()),
            SubagentEntry::Skills(_) => None,
        })
        .collect::<Vec<_>>();
    let can_run_as_user_facing_worker = def.id != "orchestrator"
        && matches!(def.agent_tier, AgentTier::Reasoning | AgentTier::Worker);

    AgentDefinitionDisplay {
        id: def.id.clone(),
        display_name: def.display_name().to_string(),
        when_to_use: def.when_to_use.clone(),
        tier: def.agent_tier.as_str().to_string(),
        model: model_metadata(&def.model),
        tools: def.tools.clone(),
        direct_tool_count: direct_tool_names.len(),
        direct_tool_names,
        uses_wildcard_tools,
        subagent_ids,
        includes_profile: !def.omit_profile,
        includes_memory_md: !def.omit_memory_md,
        includes_memory_context: !def.omit_memory_context,
        can_run_as_user_facing_worker,
        write_capable: is_write_capable(def),
        source: match &def.source {
            DefinitionSource::Builtin => AgentDefinitionSource::Builtin,
            DefinitionSource::File(_) => AgentDefinitionSource::Custom,
        },
    }
}

fn model_metadata(model: &ModelSpec) -> AgentDefinitionModel {
    match model {
        ModelSpec::Inherit => AgentDefinitionModel {
            kind: "inherit".to_string(),
            value: None,
        },
        ModelSpec::Exact(value) => AgentDefinitionModel {
            kind: "exact".to_string(),
            value: Some(value.clone()),
        },
        ModelSpec::Hint(value) => AgentDefinitionModel {
            kind: "hint".to_string(),
            value: Some(value.clone()),
        },
    }
}

fn direct_tool_names(def: &AgentDefinition) -> Vec<String> {
    let mut names = match &def.tools {
        ToolScope::Wildcard => Vec::new(),
        ToolScope::Named(names) => names.clone(),
    };
    for tool in &def.extra_tools {
        if !names.contains(tool) {
            names.push(tool.clone());
        }
    }
    names.retain(|name| !def.disallowed_tools.contains(name));
    names.sort();
    names
}

fn is_write_capable(def: &AgentDefinition) -> bool {
    if matches!(def.sandbox_mode, SandboxMode::ReadOnly) {
        return false;
    }
    if matches!(def.tools, ToolScope::Wildcard) {
        return true;
    }
    direct_tool_names(def).iter().any(|name| {
        let normalized = name.to_ascii_lowercase();
        normalized.contains("write")
            || normalized.contains("edit")
            || normalized.contains("execute")
            || normalized.contains("shell")
            || normalized.contains("apply")
            || normalized.contains("delete")
            || normalized.contains("remove")
            || normalized.contains("create")
            || normalized.contains("update")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::agent::harness::definition::{PromptSource, SkillsWildcard};

    fn definition() -> AgentDefinition {
        AgentDefinition {
            id: "researcher".to_string(),
            when_to_use: "Use for research.".to_string(),
            display_name: Some("Researcher".to_string()),
            system_prompt: PromptSource::Inline("hidden prompt".to_string()),
            omit_identity: true,
            omit_memory_context: true,
            omit_safety_preamble: true,
            omit_skills_catalog: true,
            omit_profile: false,
            omit_memory_md: true,
            model: ModelSpec::Hint("reasoning".to_string()),
            temperature: 0.2,
            tools: ToolScope::Named(vec!["web_search".to_string(), "file_read".to_string()]),
            disallowed_tools: vec!["file_read".to_string()],
            skill_filter: None,
            extra_tools: vec!["memory_search".to_string(), "web_search".to_string()],
            max_iterations: 8,
            iteration_policy: Default::default(),
            max_result_chars: None,
            timeout_secs: None,
            sandbox_mode: SandboxMode::ReadOnly,
            background: false,
            trigger_memory_agent: Default::default(),
            subagents: vec![
                SubagentEntry::AgentId("critic".to_string()),
                SubagentEntry::Skills(SkillsWildcard {
                    skills: "*".to_string(),
                }),
            ],
            delegate_name: None,
            agent_tier: AgentTier::Worker,
            source: DefinitionSource::Builtin,
        }
    }

    #[test]
    fn metadata_projection_omits_prompt_and_paths() {
        let display = metadata_from_definition(&definition());

        assert_eq!(display.id, "researcher");
        assert_eq!(display.display_name, "Researcher");
        assert_eq!(display.model.kind, "hint");
        assert_eq!(display.model.value.as_deref(), Some("reasoning"));
        match &display.tools {
            ToolScope::Named(names) => {
                assert_eq!(
                    names,
                    &vec!["web_search".to_string(), "file_read".to_string()]
                );
            }
            ToolScope::Wildcard => panic!("expected named tool scope"),
        }
        assert_eq!(
            display.direct_tool_names,
            vec!["memory_search", "web_search"]
        );
        assert_eq!(display.direct_tool_count, 2);
        assert!(!display.uses_wildcard_tools);
        assert_eq!(display.subagent_ids, vec!["critic"]);
        assert!(display.includes_profile);
        assert!(!display.includes_memory_md);
        assert!(!display.includes_memory_context);
        assert!(display.can_run_as_user_facing_worker);
        assert!(!display.write_capable);

        let json = serde_json::to_value(display).expect("serialize display");
        assert!(json.get("system_prompt").is_none());
        assert!(json.get("prompt").is_none());
    }
}
