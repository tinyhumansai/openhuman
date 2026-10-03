use super::*;

fn visible_names(agent_id: &str) -> std::collections::HashSet<String> {
    let definition = crate::agent::harness::AgentDefinitionRegistry::builtins_only()
        .get(agent_id)
        .cloned()
        .unwrap_or_else(|| panic!("built-in agent definition not found: {agent_id}"));
    visible_names_for(&definition)
}

fn visible_names_for(
    definition: &crate::agent::harness::definition::AgentDefinition,
) -> std::collections::HashSet<String> {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);
    let agent =
        crate::agent::OpenHumanSessionHost::from_config_with_definition(&config, definition)
            .unwrap_or_else(|e| panic!("{} session build: {e}", definition.id));
    agent
        .visible_tool_specs_arc()
        .iter()
        .map(|spec| spec.name.clone())
        .collect()
}

#[test]
fn resetting_wildcard_visibility_keeps_collapsed_exposure() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);
    let definition = super::wildcard_probe_def();
    let mut agent =
        crate::agent::OpenHumanSessionHost::from_config_with_definition(&config, &definition)
            .expect("build wildcard agent");

    agent.set_visible_tool_names(std::collections::HashSet::new());

    let visible = agent.visible_tool_specs_arc();
    assert!(visible.iter().any(|spec| spec.name == "todo"));
    assert!(!visible.iter().any(|spec| spec.name.starts_with("todo_")));
}

#[test]
fn hiding_and_reseeding_wildcard_visibility_keeps_collapsed_exposure() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);
    let definition = super::wildcard_probe_def();
    let mut agent =
        crate::agent::OpenHumanSessionHost::from_config_with_definition(&config, &definition)
            .expect("build wildcard agent");

    agent.set_visible_tool_names(std::collections::HashSet::new());
    agent.hide_tools(&["todo"]);

    let visible = agent.visible_tool_specs_arc();
    assert!(!visible.iter().any(|spec| spec.name == "todo"));
    assert!(!visible.iter().any(|spec| spec.name.starts_with("todo_")));
}

/// A wildcard belt advertises the collapsed tool, never its `Hidden` members.
///
/// Both halves are asserted: the members gone AND the replacement present. A
/// belt that lost the surface entirely would pass a members-only check.
/// Regressed silently once already — `4efbea728` dropped the only production
/// call to `strip_deferred_from_visible`, so every wildcard agent shipped
/// `todo` beside the eight `todo_*` tools it replaces. The v1 `memory_*` tool
/// family is gone for good (memory v2 has one `memory` tool, registered only
/// while an engine is usable); none of it may come back.
#[test]
fn wildcard_belt_advertises_collapsed_tools_not_their_hidden_members() {
    let visible = visible_names_for(&super::wildcard_probe_def());

    assert!(
        visible.contains("todo"),
        "wildcard belt must advertise `todo`; got {visible:?}"
    );
    let leaked: Vec<&String> = visible
        .iter()
        .filter(|name| name.starts_with("memory_") || name.starts_with("todo_"))
        .collect();
    assert!(
        leaked.is_empty(),
        "`todo_*` members and v1 `memory_*` tools must not ship: {leaked:?}"
    );
}

struct FakeTool(&'static str, tinytools::ToolExposure);

#[async_trait::async_trait]
impl tinytools::Tool for FakeTool {
    fn name(&self) -> &str {
        self.0
    }
    fn description(&self) -> &str {
        "fake"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }
    fn exposure(&self) -> tinytools::ToolExposure {
        self.1
    }
    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<tinytools::ToolResult> {
        Ok(tinytools::ToolResult::success("ok"))
    }
}

fn build_with(
    tools: Vec<Box<dyn tinytools::Tool>>,
    visible: std::collections::HashSet<String>,
) -> crate::agent::OpenHumanSessionHost {
    let model: std::sync::Arc<dyn tinyinference_llm::model::ChatModel<()>> =
        std::sync::Arc::new(tinyagents_harness::testkit::ScriptedModel::new(Vec::new()));
    crate::agent::SessionHostBuilder::new()
        .chat_model(model)
        .tools(tools)
        .visible_tool_names(visible)
        .tool_dispatcher(Box::new(tinytools_agent::dialect::XmlDialect))
        .build()
        .expect("session build")
}

fn direct_and_deferred() -> Vec<Box<dyn tinytools::Tool>> {
    vec![
        Box::new(FakeTool("plain", tinytools::ToolExposure::Direct)),
        Box::new(FakeTool("rare", tinytools::ToolExposure::Deferred)),
    ]
}

/// A wildcard belt withholds every `Deferred` registration from the wire and
/// keeps it reachable: never advertised, always in the deferred set the
/// harness registers for its `tool_search` bridge, and classified `Allow` by
/// the policy — a found tool the gate refuses as prompt-hidden is the
/// unusable find this replaced.
#[test]
fn wildcard_belt_defers_but_keeps_reachable() {
    let agent = build_with(direct_and_deferred(), std::collections::HashSet::new());

    assert_eq!(
        agent.deferred_tool_names_for_test(),
        &std::collections::HashSet::from(["rare".to_string()])
    );
    assert!(agent.visible_tool_names_for_test().contains("plain"));
    assert!(!agent.visible_tool_names_for_test().contains("rare"));
    assert!(
        !agent
            .visible_tool_specs_arc()
            .iter()
            .any(|spec| spec.name == "rare"),
        "a deferred tool must not be in the prompt's spec list"
    );
    assert!(
        agent.tool_policy_session_for_test().is_allowed("rare"),
        "a deferred tool must be callable once found"
    );
}

/// A named belt that lists `tool_search` opts into discovery: the name itself
/// is the harness's intrinsic bridge and leaves the allowlist, every
/// `Deferred` registration becomes reachable whether or not the belt named
/// it, and the belt's own `Direct` entries stay exactly as written.
#[test]
fn named_belt_opts_into_discovery_by_naming_tool_search() {
    let visible: std::collections::HashSet<String> = [
        "plain",
        crate::tools::implementations::meta::TOOL_SEARCH_NAME,
    ]
    .into_iter()
    .map(String::from)
    .collect();
    let agent = build_with(direct_and_deferred(), visible);

    assert_eq!(
        agent.visible_tool_names_for_test(),
        &std::collections::HashSet::from(["plain".to_string()])
    );
    assert_eq!(
        agent.deferred_tool_names_for_test(),
        &std::collections::HashSet::from(["rare".to_string()])
    );
    assert!(agent.tool_policy_session_for_test().is_allowed("rare"));
}

/// A named belt that does not opt in reaches nothing beyond what it wrote
/// down: no deferred set, and a deferred tool it did not name stays hidden.
#[test]
fn named_belt_without_tool_search_reaches_no_deferred_tool() {
    let visible: std::collections::HashSet<String> = std::iter::once("plain".to_string()).collect();
    let agent = build_with(direct_and_deferred(), visible);

    assert!(agent.deferred_tool_names_for_test().is_empty());
    assert!(!agent.tool_policy_session_for_test().is_allowed("rare"));
}

/// A hand-written `[tools] named` belt is left exactly as written: exposure is
/// only for the wildcard belt. `critic` names `file_read` among its tools; it
/// must keep it and gain nothing a wildcard would add.
#[test]
fn named_belt_keeps_its_listed_members() {
    let visible = visible_names("critic");
    assert!(
        visible.contains("file_read"),
        "named belt must keep the members it lists; got {visible:?}"
    );
    assert!(
        !visible.contains("shell"),
        "named belt must not gain unlisted tools; got {visible:?}"
    );
    assert!(
        !visible.contains(crate::memory::tools::MEMORY_TOOL_NAME),
        "named belt must not gain the memory tool; got {visible:?}"
    );
}

/// The tools of the specialists the inline skills replaced are `Deferred`:
/// off the orchestrator's wire, in its deferred set (so `tool_search` finds
/// them), and callable once found. This is the whole bargain that let
/// `settings_agent`, `scheduler_agent`, `help`, `code_executor` and `critic`
/// go — a regression here silently takes those capabilities from chat.
#[test]
fn orchestrator_reaches_the_replaced_specialists_tools_through_discovery() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);
    let definition = crate::agent::harness::AgentDefinitionRegistry::builtins_only()
        .get("orchestrator")
        .cloned()
        .expect("orchestrator built-in definition");
    let agent =
        crate::agent::OpenHumanSessionHost::from_config_with_definition(&config, &definition)
            .expect("orchestrator session build");
    let deferred = agent.deferred_tool_names_for_test();
    let visible = agent.visible_tool_names_for_test();
    let policy = agent.tool_policy_session_for_test();
    // Unconditionally registered members only: `node_exec` / `npm_exec` need
    // the managed Node runtime and the wallet family needs the `web3`
    // feature, so a test profile without them cannot see them either way.
    for tool in [
        "cron",
        "config_snapshot",
        "service_restart",
        "gitbooks_search",
        "edit",
        "curl",
        "read_diff",
        "run_linter",
        "run_tests",
        "mcp_registry_installed_list",
    ] {
        assert!(
            deferred.contains(tool),
            "`{tool}` must be deferred; got {deferred:?}"
        );
        assert!(!visible.contains(tool), "`{tool}` must stay off the wire");
        assert!(
            policy.is_allowed(tool),
            "`{tool}` must be callable once found"
        );
    }
}
