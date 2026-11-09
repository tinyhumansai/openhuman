use super::*;
use serde_json::json;

#[tokio::test]
async fn module_tool_descriptors_preserve_run_and_sandbox_safety_metadata() {
    let config = Arc::new(Config::default());
    let catalog = CopilotToolCatalog::load(config).expect("module tool catalog loads");

    let run = catalog.tool("run_flow").expect("run_flow is catalogued");
    assert_eq!(run.permission_level(), PermissionLevel::Write);
    assert!(run.external_effect());
    assert_eq!(run.parameters_schema()["required"], json!(["flow_id"]));

    let dry_run = catalog
        .tool("dry_run_workflow")
        .expect("dry_run_workflow is catalogued");
    assert_eq!(dry_run.permission_level(), PermissionLevel::None);
    assert!(!dry_run.external_effect());
}

#[tokio::test]
async fn model_arguments_cannot_replace_the_scoped_host_callback_address() {
    let workspace = tempfile::tempdir().expect("temporary workspace");
    let mut config = Config::default();
    config.workspace_dir = workspace.path().to_path_buf();
    config.config_path = workspace.path().join("config.toml");
    let catalog = CopilotToolCatalog::load(Arc::new(config)).expect("module tool catalog loads");
    let tool = catalog
        .tool("list_flows")
        .expect("list_flows is catalogued");
    let result = tool
        .execute(json!({
            "host": {
                "bus_name": "ai.tinyhumans.attacker",
                "object_path": "/attacker/copilot-host"
            }
        }))
        .await
        .expect("the outer invocation supplies its own host callback");
    assert!(
        !result.is_error,
        "model arguments must not redirect the scoped callback: {}",
        result.output()
    );
    let output: Value = serde_json::from_str(&result.output()).expect("tool JSON result");
    assert_eq!(output["flows"], json!([]), "{output}");
}

fn tool_context(
    core: &Arc<crate::core::runtime::CoreContext>,
    id: &str,
) -> tinyagents_harness::tool::ToolExecutionContext {
    let run = crate::agent::tinyagents::host::run_context::OpenHumanRunContext::new()
        .with_core(Some(Arc::clone(core)))
        .into_tinyagents(tinyagents_harness::context::RunConfig::new(id));
    tinyagents_harness::tool::ToolExecutionContext::from_run_context(
        &run,
        tinyagents_harness::ids::CallId::new(format!("{id}-call")),
    )
}

#[tokio::test]
async fn concurrent_tool_callbacks_keep_each_explicit_profile_scope() {
    use crate::core::runtime::{ContextOverlay, CoreContext, DomainSet};
    use crate::tools::toolpacks::ToolGroups;

    let workspace = tempfile::tempdir().expect("temporary workspace");
    let mut config = Config::default();
    config.workspace_dir = workspace.path().join("workspace");
    config.action_dir = config.workspace_dir.clone();
    config.config_path = workspace.path().join("config.toml");
    std::fs::create_dir_all(&config.workspace_dir).expect("workspace directory");

    let root = CoreContext::for_test_with_config(DomainSet::full(), config.clone());
    let alice = root.derive_with(
        ContextOverlay::new(config.clone(), DomainSet::full(), ToolGroups::default())
            .profile("copilot-alice"),
    );
    let bob = root.derive_with(
        ContextOverlay::new(config.clone(), DomainSet::full(), ToolGroups::default())
            .profile("copilot-bob"),
    );
    let graph = json!({
        "nodes": [{"id":"trigger","kind":"trigger","name":"Trigger","config":{}}],
        "edges": []
    });
    for (context, name) in [(&alice, "Alice only"), (&bob, "Bob only")] {
        let context = Arc::clone(context);
        let config = config.clone();
        let graph = graph.clone();
        let name = name.to_string();
        CoreContext::scope(context.clone(), async move {
            crate::flows::ops::flows_create(&config, name, graph, false)
                .await
                .expect("profile flow creation");
        })
        .await;
    }

    let catalog = CopilotToolCatalog::load(Arc::new(config)).expect("module tool catalog loads");
    let tool = catalog
        .tool("list_flows")
        .expect("list_flows is catalogued");
    let alice_call = tool_context(&alice, "copilot-alice");
    let bob_call = tool_context(&bob, "copilot-bob");
    let alice_future = tool.execute_with_context(
        json!({}),
        tinytools::ToolCallOptions::default(),
        Some(&alice_call),
    );
    let bob_future = tool.execute_with_context(
        json!({}),
        tinytools::ToolCallOptions::default(),
        Some(&bob_call),
    );
    let (alice_result, bob_result) = tokio::join!(alice_future, bob_future);
    let alice_result = alice_result.expect("Alice callback result");
    let bob_result = bob_result.expect("Bob callback result");
    assert!(!alice_result.is_error, "{}", alice_result.output());
    assert!(!bob_result.is_error, "{}", bob_result.output());
    assert!(alice_result.output().contains("Alice only"));
    assert!(!alice_result.output().contains("Bob only"));
    assert!(bob_result.output().contains("Bob only"));
    assert!(!bob_result.output().contains("Alice only"));
}

#[tokio::test]
async fn module_run_flow_uses_the_host_detached_runner_and_requires_a_saved_flow() {
    let workspace = tempfile::tempdir().expect("temporary workspace");
    let mut config = Config::default();
    config.workspace_dir = workspace.path().join("workspace");
    config.action_dir = config.workspace_dir.clone();
    config.config_path = workspace.path().join("config.toml");
    std::fs::create_dir_all(&config.workspace_dir).expect("workspace directory");
    let catalog =
        CopilotToolCatalog::load(Arc::new(config.clone())).expect("module tool catalog loads");
    let tool = catalog.tool("run_flow").expect("run_flow is catalogued");

    let missing = tool
        .execute(json!({ "flow_id": "not-saved" }))
        .await
        .expect("missing flow is a reported tool failure");
    assert!(missing.is_error, "{}", missing.output());
    assert!(
        missing.output().contains("not-saved"),
        "{}",
        missing.output()
    );

    let flow = crate::flows::ops::flows_create(
        &config,
        "Detached Copilot run".into(),
        json!({
            "nodes": [{"id":"trigger","kind":"trigger","name":"Trigger","config":{}}],
            "edges": []
        }),
        false,
    )
    .await
    .expect("saved flow creates")
    .value;
    let result = tool
        .execute(json!({ "flow_id": flow.id, "input": { "source": "copilot" } }))
        .await
        .expect("saved flow starts through the host callback");
    assert!(!result.is_error, "{}", result.output());
    let started: Value = serde_json::from_str(&result.output()).expect("run result JSON");
    assert_eq!(started["type"], "workflow_run_started");
    assert_eq!(started["status"], "running");
    assert!(started["run_id"].as_str().is_some_and(|id| !id.is_empty()));
    assert_eq!(started["flow_id"], flow.id);
}

#[tokio::test]
async fn module_proposal_tool_uses_host_gates_without_persisting() {
    let workspace = tempfile::tempdir().expect("temporary workspace");
    let mut config = Config::default();
    config.workspace_dir = workspace.path().join("workspace");
    config.action_dir = config.workspace_dir.clone();
    config.config_path = workspace.path().join("config.toml");
    std::fs::create_dir_all(&config.workspace_dir).expect("workspace directory");
    let catalog =
        CopilotToolCatalog::load(Arc::new(config.clone())).expect("module tool catalog loads");
    let tool = catalog
        .tool("propose_workflow")
        .expect("propose_workflow is catalogued");

    let invalid = tool
        .execute(json!({ "name": "Invalid", "graph": { "nodes": [], "edges": [] } }))
        .await
        .expect("invalid candidate is a reported tool error");
    assert!(invalid.is_error, "{}", invalid.output());

    let valid = tool
        .execute(json!({
            "name": "Proposed only",
            "graph": {
                "nodes": [{"id":"trigger","kind":"trigger","name":"Trigger","config":{}}],
                "edges": []
            }
        }))
        .await
        .expect("valid candidate uses the scoped BuildProposal callback");
    assert!(!valid.is_error, "{}", valid.output());
    let proposal: Value = serde_json::from_str(&valid.output()).expect("proposal JSON");
    assert_eq!(proposal["type"], "workflow_proposal");
    assert_eq!(proposal["persisted"], false);
    assert!(crate::flows::ops::flows_list(&config)
        .await
        .expect("flow list")
        .value
        .is_empty());
}
