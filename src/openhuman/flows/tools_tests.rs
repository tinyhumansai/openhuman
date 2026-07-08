use super::*;
use crate::openhuman::config::Config;
use serde_json::json;
use tempfile::TempDir;

fn test_config(tmp: &TempDir) -> Arc<Config> {
    let config = Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    Arc::new(config)
}

fn valid_graph() -> Value {
    json!({
        "nodes": [
            {
                "id": "t",
                "kind": "trigger",
                "name": "Every morning",
                "config": { "trigger_kind": "schedule", "schedule": { "kind": "cron", "expr": "0 9 * * *" } }
            },
            {
                "id": "a",
                "kind": "agent",
                "name": "Summarize",
                "config": { "prompt": "Summarize yesterday's messages" }
            },
            {
                "id": "s",
                "kind": "tool_call",
                "name": "Post to Slack",
                "config": { "slug": "slack.post_message", "args": { "channel": "#general" } }
            }
        ],
        "edges": [
            { "from_node": "t", "to_node": "a" },
            { "from_node": "a", "to_node": "s" }
        ]
    })
}

#[tokio::test]
async fn valid_graph_returns_workflow_proposal_success() {
    let tmp = TempDir::new().unwrap();
    let tool = ProposeWorkflowTool::new(test_config(&tmp));

    let result = tool
        .execute(json!({ "name": "Daily standup summary", "graph": valid_graph() }))
        .await
        .unwrap();

    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).expect("valid JSON output");
    assert_eq!(parsed["type"], "workflow_proposal");
    assert_eq!(parsed["name"], "Daily standup summary");
    assert_eq!(parsed["graph"]["nodes"].as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn no_trigger_graph_is_an_error() {
    let tmp = TempDir::new().unwrap();
    let tool = ProposeWorkflowTool::new(test_config(&tmp));

    let graph_without_trigger = json!({
        "nodes": [ { "id": "a", "kind": "output_parser", "name": "A" } ],
        "edges": []
    });

    let result = tool
        .execute(json!({ "name": "bad", "graph": graph_without_trigger }))
        .await
        .unwrap();

    assert!(result.is_error);
    assert!(
        result.output().to_lowercase().contains("trigger"),
        "expected a trigger-related validation error, got: {}",
        result.output()
    );
}

#[tokio::test]
async fn missing_name_is_an_error() {
    let tmp = TempDir::new().unwrap();
    let tool = ProposeWorkflowTool::new(test_config(&tmp));

    let result = tool
        .execute(json!({ "graph": valid_graph() }))
        .await
        .unwrap();

    assert!(result.is_error);
    assert!(result.output().contains("Missing 'name'"));
}

#[tokio::test]
async fn missing_graph_is_an_error() {
    let tmp = TempDir::new().unwrap();
    let tool = ProposeWorkflowTool::new(test_config(&tmp));

    let result = tool
        .execute(json!({ "name": "no graph here" }))
        .await
        .unwrap();

    assert!(result.is_error);
    assert!(result.output().contains("Missing 'graph'"));
}

#[tokio::test]
async fn omitted_require_approval_defaults_true_in_result() {
    let tmp = TempDir::new().unwrap();
    let tool = ProposeWorkflowTool::new(test_config(&tmp));

    let result = tool
        .execute(json!({ "name": "demo", "graph": valid_graph() }))
        .await
        .unwrap();

    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(parsed["require_approval"], true);
}

#[tokio::test]
async fn explicit_require_approval_false_is_respected() {
    let tmp = TempDir::new().unwrap();
    let tool = ProposeWorkflowTool::new(test_config(&tmp));

    let result = tool
        .execute(json!({ "name": "demo", "graph": valid_graph(), "require_approval": false }))
        .await
        .unwrap();

    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(parsed["require_approval"], false);
}

#[tokio::test]
async fn explicit_require_approval_true_is_respected() {
    let tmp = TempDir::new().unwrap();
    let tool = ProposeWorkflowTool::new(test_config(&tmp));

    let result = tool
        .execute(json!({ "name": "demo", "graph": valid_graph(), "require_approval": true }))
        .await
        .unwrap();

    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(parsed["require_approval"], true);
}

#[tokio::test]
async fn summary_step_count_and_kinds_are_correct() {
    let tmp = TempDir::new().unwrap();
    let tool = ProposeWorkflowTool::new(test_config(&tmp));

    let result = tool
        .execute(json!({ "name": "demo", "graph": valid_graph() }))
        .await
        .unwrap();

    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    let steps = parsed["summary"]["steps"].as_array().unwrap();
    // 3 nodes total, minus the 1 trigger = 2 steps.
    assert_eq!(steps.len(), 2);
    assert_eq!(steps[0]["kind"], "agent");
    assert_eq!(steps[0]["name"], "Summarize");
    assert_eq!(steps[0]["config_hint"], "Summarize yesterday's messages");
    assert_eq!(steps[1]["kind"], "tool_call");
    assert_eq!(steps[1]["name"], "Post to Slack");
    assert_eq!(steps[1]["config_hint"], "slack.post_message");
}

#[tokio::test]
async fn summary_trigger_describes_schedule() {
    let tmp = TempDir::new().unwrap();
    let tool = ProposeWorkflowTool::new(test_config(&tmp));

    let result = tool
        .execute(json!({ "name": "demo", "graph": valid_graph() }))
        .await
        .unwrap();

    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(parsed["summary"]["trigger"], "schedule: 0 9 * * *");
}

#[tokio::test]
async fn summary_trigger_describes_manual_default() {
    let tmp = TempDir::new().unwrap();
    let tool = ProposeWorkflowTool::new(test_config(&tmp));

    let graph = json!({
        "nodes": [ { "id": "t", "kind": "trigger", "name": "Manual start" } ],
        "edges": []
    });

    let result = tool
        .execute(json!({ "name": "demo", "graph": graph }))
        .await
        .unwrap();

    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(parsed["summary"]["trigger"], "manual");
    assert!(parsed["summary"]["steps"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn summary_trigger_describes_app_event() {
    let tmp = TempDir::new().unwrap();
    let tool = ProposeWorkflowTool::new(test_config(&tmp));

    let graph = json!({
        "nodes": [
            {
                "id": "t",
                "kind": "trigger",
                "name": "On new email",
                "config": {
                    "trigger_kind": "app_event",
                    "toolkit": "gmail",
                    "trigger_slug": "GMAIL_NEW_GMAIL_MESSAGE"
                }
            }
        ],
        "edges": []
    });

    let result = tool
        .execute(json!({ "name": "demo", "graph": graph }))
        .await
        .unwrap();

    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(
        parsed["summary"]["trigger"],
        "app event: gmail/GMAIL_NEW_GMAIL_MESSAGE"
    );
}

#[test]
fn propose_workflow_never_creates_a_flow() {
    // The tool must have no way to persist a flow — the human-in-the-loop
    // invariant (issue B4) rests entirely on `external_effect() == false` and
    // `permission_level() == None` (no gate would even fire if this ever
    // regressed to true, but a saved flow must still only ever be created by
    // the user's own `flows_create` click).
    let tmp = TempDir::new().unwrap();
    let tool = ProposeWorkflowTool::new(test_config(&tmp));
    assert_eq!(tool.permission_level(), PermissionLevel::None);
    assert!(!tool.external_effect());
}

#[test]
fn tool_name_and_schema_are_stable() {
    let tmp = TempDir::new().unwrap();
    let tool = ProposeWorkflowTool::new(test_config(&tmp));
    assert_eq!(tool.name(), "propose_workflow");

    let schema = tool.parameters_schema();
    let required = schema["required"].as_array().unwrap();
    assert!(required.iter().any(|v| v.as_str() == Some("name")));
    assert!(required.iter().any(|v| v.as_str() == Some("graph")));
}

#[test]
fn display_label_humanizes_the_tool_name() {
    let tmp = TempDir::new().unwrap();
    let tool = ProposeWorkflowTool::new(test_config(&tmp));
    assert_eq!(
        tool.display_label(&Value::Null).as_deref(),
        Some("Propose Workflow")
    );
}

// ── enforcing binding-resolvability gate ────────────────────────────────────

#[tokio::test]
async fn propose_workflow_rejects_unschemad_agent_binding() {
    // The proven live-failure graph: `summarize` has no `output_parser.schema`,
    // so `post`'s `args.channel` binding is guaranteed to resolve null at
    // runtime. Unlike the advisory dry-run check, propose_workflow must
    // REJECT this outright rather than warn (warning_count would have been 0
    // here — nothing stopped this from reaching save_workflow before).
    let tmp = TempDir::new().unwrap();
    let tool = ProposeWorkflowTool::new(test_config(&tmp));

    let graph = json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "summarize", "kind": "agent", "name": "Summarize",
              "config": { "agent_ref": "researcher", "prompt": "summarize" } },
            { "id": "notify", "kind": "tool_call", "name": "Notify",
              "config": { "slug": "SLACK_SEND_MESSAGE",
                "args": { "channel": "=nodes.summarize.item.json.channel" } } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "summarize" },
            { "from_node": "summarize", "to_node": "notify" }
        ]
    });

    let result = tool
        .execute(json!({ "name": "Summarize and notify", "graph": graph }))
        .await
        .unwrap();

    assert!(result.is_error, "must be rejected: {}", result.output());
    let output = result.output();
    assert!(output.contains("notify"), "{output}");
    assert!(output.contains("channel"), "{output}");
    assert!(output.contains("summarize"), "{output}");
    assert!(
        output.contains("output_parser.schema"),
        "must name the missing schema as the fix: {output}"
    );
}
