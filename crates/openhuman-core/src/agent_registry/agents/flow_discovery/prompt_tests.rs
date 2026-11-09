use crate::agent::prompts::PromptContext;
use crate::agent_registry::agents::flow_discovery::prompt::build;
use tinytools_agent::dialect::ToolCallFormat;

fn ctx<'a>(
    workspace: &'a std::path::Path,
    visible: &'a std::collections::HashSet<String>,
    archetype: Option<&'a str>,
) -> PromptContext<'a> {
    PromptContext {
        workspace_dir: workspace,
        model_name: "test",
        agent_id: "flow_discovery",
        flow_archetype_text: archetype,
        tools: &[],
        workflows: &[],
        dispatcher_instructions: "",
        visible_tool_names: visible,
        tool_call_format: ToolCallFormat::PFormat,
        connected_integrations: &[],
        connected_identities_md: String::new(),
        user_identity: None,
        personality_roster: vec![],
        agents_md_global: None,
        agents_md_local: None,
    }
}

#[tokio::test]
async fn module_prompt_teaches_the_read_only_emit_invariant() {
    let workspace = tempfile::tempdir().expect("workspace");
    let config = crate::config::Config {
        workspace_dir: workspace.path().to_path_buf(),
        ..crate::config::Config::default()
    };
    let registry = crate::agent_registry::host_definition::AgentDefinitionRegistry::builtins_only();
    let definition = registry
        .get("flow_discovery")
        .expect("flow_discovery builtin");
    let archetype =
        crate::agent::prompts::flow_archetype::load_for_definition(Some(&config), definition)
            .await
            .expect("TinyFlows GetPrompt succeeds")
            .expect("flow builtin prompt");
    let body = build(&ctx(
        workspace.path(),
        &std::collections::HashSet::new(),
        Some(&archetype),
    ))
    .expect("compose module prompt");
    let lc = body.to_lowercase();
    assert!(lc.contains("suggest_workflows"), "must name the emit tool");
    assert!(
        lc.contains("read-only") || lc.contains("never act") || lc.contains("never build"),
        "prompt must teach the read-only invariant"
    );
}

#[test]
fn flow_discovery_requires_an_explicit_archetype_override() {
    let workspace = tempfile::tempdir().expect("workspace");
    let error = build(&ctx(
        workspace.path(),
        &std::collections::HashSet::new(),
        None,
    ))
    .expect_err("no static or host fallback is allowed");
    assert!(error.to_string().contains("GetPrompt"));
}
