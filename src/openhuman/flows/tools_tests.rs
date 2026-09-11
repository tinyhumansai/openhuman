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
    // A proposal is never a persisted flow — the payload must say so (WS2) so
    // an agent can't misread it as a save confirmation.
    assert_eq!(parsed["persisted"], false);
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

#[test]
fn dedup_config_hint_is_truncated_for_a_long_key_expression() {
    // CodeRabbit (PR #5265): unlike the other config_hint branches, the
    // dedup branch returned `format!("key: {k}")` unwrapped by
    // `truncate_hint`, so an oversized `config.key` expression could make
    // the proposal/summary payload unbounded.
    let long_key = format!("=item.{}", "x".repeat(200));
    let graph = WorkflowGraph {
        nodes: vec![Node {
            id: "dd".to_string(),
            kind: NodeKind::Dedup,
            type_version: 1,
            name: "Dedup".to_string(),
            config: json!({ "key": long_key }),
            ports: Vec::new(),
            position: None,
        }],
        ..Default::default()
    };

    let summary = build_summary(&graph);
    let hint = summary["steps"][0]["config_hint"].as_str().unwrap();
    assert!(
        hint.chars().count() <= MAX_CONFIG_HINT_CHARS,
        "hint not truncated: {} chars: {hint}",
        hint.chars().count()
    );
    assert!(hint.ends_with('…'), "expected an ellipsis marker: {hint}");
    assert!(
        hint.starts_with("key: "),
        "expected the key: prefix: {hint}"
    );
}

#[test]
fn approval_config_hint_prefers_the_review_title() {
    let graph = WorkflowGraph {
        nodes: vec![Node {
            id: "review".to_string(),
            kind: NodeKind::Approval,
            type_version: 1,
            name: "Review".to_string(),
            config: json!({
                "title": "Publish this draft?",
                "prompt": "Approve publication"
            }),
            ports: Vec::new(),
            position: None,
        }],
        ..Default::default()
    };

    let summary = build_summary(&graph);
    assert_eq!(summary["steps"][0]["config_hint"], "Publish this draft?");
}

#[test]
fn shell_config_hint_prefers_the_script_path_and_truncates_inline_source() {
    let graph = WorkflowGraph {
        nodes: vec![
            Node {
                id: "path".to_string(),
                kind: NodeKind::Shell,
                type_version: 1,
                name: "Script file".to_string(),
                config: json!({
                    "script_path": "scripts/report.sh",
                    "source": "ignored when a path is present"
                }),
                ports: Vec::new(),
                position: None,
            },
            Node {
                id: "inline".to_string(),
                kind: NodeKind::Shell,
                type_version: 1,
                name: "Inline script".to_string(),
                config: json!({ "source": "x".repeat(200) }),
                ports: Vec::new(),
                position: None,
            },
        ],
        ..Default::default()
    };

    let summary = build_summary(&graph);
    assert_eq!(
        summary["steps"][0]["config_hint"],
        "script: scripts/report.sh"
    );
    let inline_hint = summary["steps"][1]["config_hint"].as_str().unwrap();
    assert_eq!(inline_hint.chars().count(), MAX_CONFIG_HINT_CHARS);
    assert!(inline_hint.ends_with('…'));
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
async fn propose_workflow_rejects_agent_binding_missing_declared_field() {
    // A declared schema that omits `channel` proves `notify`'s binding resolves
    // null at runtime. Unlike a schema-less agent (whose host-defined output is
    // merely unverifiable), propose_workflow must
    // REJECT this outright rather than warn (warning_count would have been 0
    // here — nothing stopped this from reaching save_workflow before).
    let tmp = TempDir::new().unwrap();
    let tool = ProposeWorkflowTool::new(test_config(&tmp));

    let graph = json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "summarize", "kind": "agent", "name": "Summarize",
              "config": { "agent_ref": "researcher", "prompt": "summarize",
                "output_parser": { "schema": { "type": "object",
                  "properties": { "summary": { "type": "string" } } } } } },
            { "id": "notify", "kind": "tool_call", "name": "Notify",
              "config": { "slug": "SLACK_SEND_MESSAGE",
                "args": { "channel": "=nodes.summarize.item.json.channel", "text": "A notification" } } }
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

#[tokio::test]
async fn propose_workflow_rejects_an_incompatible_saved_child_reference() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let tool = ProposeWorkflowTool::new(Arc::clone(&config));

    // Simulate a legacy child saved before the current TinyFlows engine
    // rejected nested conditional fan-in. The parent itself is structurally
    // valid, so only the config-aware shared builder gate can catch this.
    let legacy_child = json!({
        "nodes": [
            { "id": "start", "kind": "trigger", "name": "Trigger" },
            { "id": "outer", "kind": "condition", "name": "Outer", "config": { "field": "outer" } },
            { "id": "inner", "kind": "condition", "name": "Inner", "config": { "field": "inner" } },
            { "id": "outer_else", "kind": "output_parser", "name": "Outer else" },
            { "id": "inner_else", "kind": "output_parser", "name": "Inner else" },
            { "id": "a", "kind": "output_parser", "name": "A" },
            { "id": "c", "kind": "output_parser", "name": "C" },
            { "id": "m", "kind": "merge", "name": "Merge" }
        ],
        "edges": [
            { "from_node": "start", "to_node": "outer" },
            { "from_node": "start", "to_node": "c" },
            { "from_node": "outer", "from_port": "true", "to_node": "inner" },
            { "from_node": "outer", "from_port": "false", "to_node": "outer_else" },
            { "from_node": "inner", "from_port": "true", "to_node": "a" },
            { "from_node": "inner", "from_port": "false", "to_node": "inner_else" },
            { "from_node": "a", "to_node": "m" },
            { "from_node": "c", "to_node": "m" }
        ]
    });
    let child_graph = crate::openhuman::flows::ops::migrate_and_deserialize_graph(legacy_child)
        .expect("legacy child should deserialize");
    tinyflows::validate::validate(&child_graph)
        .expect("legacy child should remain structurally valid");
    let child = crate::openhuman::flows::store::create_flow(
        &config,
        "Legacy unsafe child".to_string(),
        child_graph,
        false,
        false,
    )
    .unwrap();
    let parent = json!({
        "nodes": [
            { "id": "start", "kind": "trigger", "name": "Trigger" },
            {
                "id": "saved-child",
                "kind": "sub_workflow",
                "name": "Saved child",
                "config": { "workflow_id": child.id }
            }
        ],
        "edges": [{ "from_node": "start", "to_node": "saved-child" }]
    });

    let result = tool
        .execute(json!({ "name": "Parent", "graph": parent }))
        .await
        .unwrap();

    assert!(result.is_error, "must reject unsafe saved child");
    let output = result.output();
    assert!(
        output.contains("unsupported_nested_conditional_fan_in"),
        "{output}"
    );
    assert!(output.contains(&child.id), "{output}");
    assert!(output.contains("saved-child"), "{output}");
    assert!(output.contains("call propose_workflow again"), "{output}");
}

/// Docs-drift guard (F2): `propose_workflow`'s hand-written description and the
/// typed node-kind contracts are two views of the SAME DSL, and they must not
/// diverge. If a node kind is added/renamed or a required config field changes
/// in `node_contracts.rs`, this fails until the tool description is updated to
/// match — the "prose can never diverge from code" check the plan calls for.
#[test]
fn propose_workflow_description_matches_typed_node_contracts() {
    let tmp = TempDir::new().unwrap();
    let tool = ProposeWorkflowTool::new(test_config(&tmp));
    let desc = tool.description();
    for contract in crate::openhuman::flows::all_node_kind_contracts() {
        assert!(
            desc.contains(&contract.kind),
            "propose_workflow description is missing node kind `{}` — update it to match \
             node_contracts.rs",
            contract.kind
        );
        for field in contract.config_fields.iter().filter(|f| f.required) {
            assert!(
                desc.contains(&field.name),
                "propose_workflow description is missing required field `config.{}` of node kind \
                 `{}` — update it to match node_contracts.rs",
                field.name,
                contract.kind
            );
        }
    }
}

/// Same drift class as the description guard above, but for the JSON `enum`
/// in `parameters_schema()`: a strict schema-constrained caller (some tool-use
/// providers validate arguments against the advertised schema before
/// `execute` ever runs) can only submit a `kind` this enum lists, regardless
/// of what the prose teaches or what `validate_and_migrate_graph` accepts. A
/// node kind present in `node_contracts.rs` but missing here would silently
/// be unreachable through `propose_workflow` for such a caller — this is the
/// exact class of bug the `loop` kind hit when it was documented in prose
/// but left off this enum.
#[test]
fn propose_workflow_schema_enum_matches_typed_node_contracts() {
    let tmp = TempDir::new().unwrap();
    let tool = ProposeWorkflowTool::new(test_config(&tmp));
    let schema = tool.parameters_schema();
    let enum_kinds: Vec<&str> = schema["properties"]["graph"]["properties"]["nodes"]["items"]
        ["properties"]["kind"]["enum"]
        .as_array()
        .expect("kind enum must be an array")
        .iter()
        .map(|v| v.as_str().expect("enum entries are strings"))
        .collect();
    for contract in crate::openhuman::flows::all_node_kind_contracts() {
        assert!(
            enum_kinds.contains(&contract.kind.as_str()),
            "propose_workflow's parameters_schema `kind` enum is missing node kind `{}` — \
             update it to match node_contracts.rs",
            contract.kind
        );
    }
}
