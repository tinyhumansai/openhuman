use super::*;

fn config_with_rules(tmp: &tempfile::TempDir) -> crate::config::Config {
    let config = crate::config::Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        tool_rules: tinytools::ToolRules::from_allow_deny(Vec::<String>::new(), ["shell"]),
        ..crate::config::Config::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    config
}

#[test]
fn a_session_composes_the_operator_and_definition_layers() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = config_with_rules(&tmp);
    let mut definition = crate::agent::registry::agents::load_builtins()
        .unwrap()
        .into_iter()
        .find(|def| def.id == "orchestrator")
        .expect("orchestrator");
    definition.disallowed_tools = vec!["web_*".to_string()];
    let host = OpenHumanSessionHost::from_config_with_definition(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &definition,
    )
    .expect("session builds");

    let rules = host.session_tool_rules();
    let names: Vec<_> = rules
        .layers
        .iter()
        .map(|layer| layer.name.clone().unwrap_or_default())
        .collect();
    assert_eq!(names, ["config", "agent:orchestrator"]);
    let context = tinytools::RuleContext::new();
    let visible = |name: &str| {
        rules.visible(
            &tinytools::ToolSubject::named(name),
            &context,
            tinytools::Surface::Catalog,
        )
    };
    assert!(!visible("shell"));
    assert!(!visible("web_fetch"));
    assert!(visible("file_read"));
}

/// A text-dialect session over the per-thread goal tools under `rules`,
/// returning its rendered prompt and declared tool names.
async fn goal_session_prompt(rules: serde_json::Value) -> (String, Vec<String>) {
    let _ = crate::agent::harness::host_definition::AgentDefinitionRegistry::init_global_builtins();
    let action_dir = tempfile::tempdir().expect("tempdir");
    let config = crate::config::Config {
        tool_rules: serde_json::from_value(rules).expect("rules"),
        ..crate::config::Config::default()
    };
    let model: std::sync::Arc<dyn tinyinference_llm::model::ChatModel<()>> =
        std::sync::Arc::new(tinyagents_harness::testkit::ScriptedModel::new(Vec::new()));
    let mut host = crate::agent::SessionHostBuilder::new()
        .chat_model_with_config(model, std::sync::Arc::new(config))
        .tools(crate::agent::goals::goal_tools(action_dir.path()))
        .action_dir(action_dir.path().to_path_buf())
        .tool_dispatcher(Box::new(tinytools_agent::dialect::XmlDialect))
        .build()
        .expect("session build");
    host.set_thread_id(Some("thread-1"));
    host.ensure_runtime_session().expect("runtime session");
    let prelude = host
        .runtime_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .prelude
        .clone()
        .expect("prelude");
    let prompt = prelude
        .build_system_prompt_tiered()
        .expect("system prompt")
        .text;
    let declared = prelude
        .prepare(true)
        .await
        .expect("tool surface")
        .tools
        .expect("declared tools")
        .specs()
        .iter()
        .map(|spec| spec.name.clone())
        .collect();
    (prompt, declared)
}

/// The host renders its own catalogue into a text-dialect prompt, so a tool
/// the rules hide must leave that catalogue too — while staying in the
/// declared tool list the harness admits calls from.
#[tokio::test]
async fn a_rule_hidden_tool_leaves_the_rendered_prompt_but_stays_declared() {
    let (control, _) = goal_session_prompt(serde_json::json!({})).await;
    assert!(
        control.contains("goal_complete"),
        "the fixture surfaces goal_complete"
    );

    let (prompt, declared) = goal_session_prompt(serde_json::json!({ "rules": [
        { "effect": "hide", "match": { "name": "goal_complete" } },
    ] }))
    .await;
    assert!(
        !prompt.contains("goal_complete"),
        "a hidden tool is not catalogued"
    );
    assert!(
        declared.iter().any(|name| name == "goal_complete"),
        "a hidden tool stays callable"
    );
}
