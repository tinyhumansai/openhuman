use super::*;

#[tokio::test]
async fn edit_workflow_rejects_a_result_that_is_structurally_invalid() {
    use crate::openhuman::flows::DraftOrigin;
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let draft = ops::flows_draft_create(
        &config,
        None,
        "Structural repair".to_string(),
        valid_graph(),
        DraftOrigin::Chat,
    )
    .unwrap()
    .value;
    let tool = EditWorkflowTool::new(config.clone());
    // Removing the only trigger leaves the graph structurally invalid.
    let result = tool
        .execute(json!({
            "draft_id": draft.id,
            "ops": [ { "op": "remove_node", "id": "t" } ]
        }))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("trigger"), "{}", result.output());
    let reloaded = ops::flows_draft_get(&config, &draft.id).unwrap().value;
    assert!(
        reloaded.graph["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|node| node["id"] != "t"),
        "structurally invalid applied edits remain available for the repair turn"
    );
}

#[tokio::test]
async fn edit_workflow_rejects_an_engine_incompatible_result() {
    use crate::openhuman::flows::DraftOrigin;
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let safe_graph = json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Trigger" },
            { "id": "outer", "kind": "condition", "name": "Outer", "config": { "field": "outer" } },
            { "id": "inner", "kind": "condition", "name": "Inner", "config": { "field": "inner" } },
            { "id": "outer_else", "kind": "output_parser", "name": "Outer else" },
            { "id": "inner_else", "kind": "output_parser", "name": "Inner else" },
            { "id": "a", "kind": "output_parser", "name": "A" },
            { "id": "c", "kind": "output_parser", "name": "C" },
            { "id": "m", "kind": "merge", "name": "Merge" }
        ],
        "edges": [
            { "from_node": "t", "from_port": "main", "to_node": "outer" },
            { "from_node": "t", "from_port": "main", "to_node": "c" },
            { "from_node": "outer", "from_port": "true", "to_node": "inner" },
            { "from_node": "outer", "from_port": "false", "to_node": "outer_else" },
            { "from_node": "inner", "from_port": "true", "to_node": "a" },
            { "from_node": "inner", "from_port": "false", "to_node": "inner_else" },
            { "from_node": "a", "from_port": "main", "to_node": "m" }
        ]
    });
    let draft = ops::flows_draft_create(
        &config,
        None,
        "Safe draft".to_string(),
        safe_graph.clone(),
        DraftOrigin::Chat,
    )
    .unwrap()
    .value;
    let tool = EditWorkflowTool::new(config.clone());
    let result = tool
        .execute(json!({
            "draft_id": draft.id,
            "ops": [
                { "op": "add_edge", "edge": { "from_node": "c", "from_port": "main", "to_node": "m" } }
            ]
        }))
        .await
        .unwrap();

    assert!(result.is_error, "{}", result.output());
    assert!(
        result
            .output()
            .contains("unsupported_nested_conditional_fan_in"),
        "{}",
        result.output()
    );
    let reloaded = ops::flows_draft_get(&config, &draft.id).unwrap().value;
    assert_eq!(
        reloaded.graph, safe_graph,
        "a rejected edit must not advance the durable draft"
    );
}

#[tokio::test]
async fn edit_workflow_does_not_persist_an_incompatible_saved_child_reference() {
    use crate::openhuman::flows::DraftOrigin;
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
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
    let child_graph = ops::migrate_and_deserialize_graph(legacy_child).unwrap();
    tinyflows::validate::validate(&child_graph).unwrap();
    let child = crate::openhuman::flows::store::create_flow(
        &config,
        "Legacy unsafe child".to_string(),
        child_graph,
        false,
        false,
    )
    .unwrap();
    let safe_graph = json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Trigger" },
            {
                "id": "child",
                "kind": "sub_workflow",
                "name": "Child",
                "config": { "workflow_id": "=inputs.workflow_id" }
            }
        ],
        "edges": [{ "from_node": "t", "to_node": "child" }]
    });
    let draft = ops::flows_draft_create(
        &config,
        None,
        "Safe draft".to_string(),
        safe_graph.clone(),
        DraftOrigin::Chat,
    )
    .unwrap()
    .value;

    let result = EditWorkflowTool::new(config.clone())
        .execute(json!({
            "draft_id": draft.id,
            "ops": [{
                "op": "update_node_config",
                "id": "child",
                "config": { "workflow_id": child.id }
            }]
        }))
        .await
        .unwrap();

    assert!(result.is_error, "{}", result.output());
    assert!(
        result
            .output()
            .contains("unsupported_nested_conditional_fan_in"),
        "{}",
        result.output()
    );
    let reloaded = ops::flows_draft_get(&config, &draft.id).unwrap().value;
    assert_eq!(
        reloaded.graph, safe_graph,
        "a rejected saved-child edit must not advance the durable draft"
    );
}

#[tokio::test]
async fn edit_workflow_preserves_non_engine_gate_edits_in_the_draft() {
    use crate::openhuman::flows::DraftOrigin;
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let draft = ops::flows_draft_create(
        &config,
        None,
        "Binding follow-up".to_string(),
        unresolvable_binding_graph(),
        DraftOrigin::Chat,
    )
    .unwrap()
    .value;
    let tool = EditWorkflowTool::new(config.clone());
    let result = tool
        .execute(json!({
            "draft_id": draft.id,
            "ops": [
                { "op": "set_node_name", "id": "summarize", "name": "Renamed before binding fix" }
            ]
        }))
        .await
        .unwrap();

    assert!(
        result.is_error,
        "binding gate should still reject the proposal"
    );
    let reloaded = ops::flows_draft_get(&config, &draft.id).unwrap().value;
    let renamed = reloaded.graph["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["id"] == "summarize")
        .unwrap();
    assert_eq!(renamed["name"], "Renamed before binding fix");
}

#[tokio::test]
async fn edit_workflow_edits_a_saved_flow_by_id() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    // Create a saved flow to edit.
    let flow = ops::flows_create(&config, "Base flow".to_string(), valid_graph(), false)
        .await
        .unwrap()
        .value;

    let tool = EditWorkflowTool::new(config.clone());
    let result = tool
        .execute(json!({
            "flow_id": flow.id,
            "ops": [ { "op": "set_node_name", "id": "a", "name": "Renamed step" } ]
        }))
        .await
        .unwrap();

    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    // Default name falls back to the base flow's name.
    assert_eq!(parsed["name"], "Base flow");
    let nodes = parsed["graph"]["nodes"].as_array().unwrap();
    let agent = nodes.iter().find(|n| n["id"] == "a").unwrap();
    assert_eq!(agent["name"], "Renamed step");
}

// ── validate_workflow (F3: standalone check) ─────────────────────────────────

#[tokio::test]
async fn validate_workflow_reports_ok_for_a_valid_graph() {
    let tmp = TempDir::new().unwrap();
    let tool = ValidateWorkflowTool::new(test_config(&tmp));
    let result = tool
        .execute(json!({ "graph": valid_graph() }))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(parsed["ok"], true);
    assert_eq!(parsed["structurally_valid"], true);
    assert_eq!(parsed["errors"].as_array().unwrap().len(), 0);
    assert_eq!(parsed["gate_errors"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn validate_workflow_surfaces_all_structural_errors() {
    let tmp = TempDir::new().unwrap();
    let tool = ValidateWorkflowTool::new(test_config(&tmp));
    // No trigger + a dangling edge.
    let graph = json!({
        "nodes": [ { "id": "a", "kind": "agent", "name": "A", "config": { "prompt": "hi" } } ],
        "edges": [ { "from_node": "a", "to_node": "ghost" } ]
    });
    let result = tool.execute(json!({ "graph": graph })).await.unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(parsed["ok"], false);
    assert_eq!(parsed["structurally_valid"], false);
    let codes: Vec<&str> = parsed["error_details"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["code"].as_str().unwrap())
        .collect();
    assert!(codes.contains(&"missing_trigger"), "{codes:?}");
    assert!(codes.contains(&"unknown_node"), "{codes:?}");
}

#[tokio::test]
async fn validate_workflow_requires_a_base() {
    let tmp = TempDir::new().unwrap();
    let tool = ValidateWorkflowTool::new(test_config(&tmp));
    let result = tool.execute(json!({})).await.unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("flow_id"));
}

// T-m4: a gate-check failure (e.g. a migrate/deserialize error surfaced after
// structural validation passed) must fail CLOSED — `ok` must never be true
// when the hard gates did not actually run. Regression test for the bug
// where `Err(_) => Vec::new()` let an empty `gate_errors` masquerade as
// "gates passed".
#[test]
fn validate_workflow_report_fails_closed_when_gate_check_errors() {
    assert!(!validate_workflow_report_is_ok(true, &[], true));
}

#[test]
fn validate_workflow_report_ok_when_structurally_valid_and_gates_pass() {
    assert!(validate_workflow_report_is_ok(true, &[], false));
}

#[test]
fn validate_workflow_report_not_ok_when_structurally_invalid() {
    assert!(!validate_workflow_report_is_ok(false, &[], false));
}

#[test]
fn validate_workflow_report_not_ok_when_gate_errors_present() {
    assert!(!validate_workflow_report_is_ok(
        true,
        &["unresolvable binding".to_string()],
        false
    ));
}

#[tokio::test]
async fn edit_workflow_edits_a_draft_and_writes_back() {
    use crate::openhuman::flows::DraftOrigin;
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    // A draft holding the base graph.
    let draft = ops::flows_draft_create(
        &config,
        None,
        "Draft flow".to_string(),
        valid_graph(),
        DraftOrigin::Chat,
    )
    .unwrap()
    .value;

    let tool = EditWorkflowTool::new(config.clone());
    let result = tool
        .execute(json!({
            "draft_id": draft.id,
            "ops": [ { "op": "add_node", "node": { "id": "b", "kind": "merge", "name": "Join" } } ]
        }))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(parsed["draft_id"], draft.id);
    assert_eq!(parsed["graph"]["nodes"].as_array().unwrap().len(), 3);

    // The edit was written back to the draft (survives for the next turn).
    let reloaded = ops::flows_draft_get(&config, &draft.id).unwrap().value;
    assert_eq!(reloaded.graph["nodes"].as_array().unwrap().len(), 3);
}

// T-m6: when the draft write-back itself fails (here: a genuine permission
// denial on the drafts dir, not a mock), the response must surface the
// failure instead of claiming "Edits live on draft {id}" — the exact
// wording that used to ship regardless of whether the write actually landed.
#[cfg(unix)]
#[tokio::test]
async fn edit_workflow_surfaces_draft_write_back_failure() {
    use crate::openhuman::flows::DraftOrigin;
    use std::os::unix::fs::PermissionsExt;

    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let draft = ops::flows_draft_create(
        &config,
        None,
        "Draft flow".to_string(),
        valid_graph(),
        DraftOrigin::Chat,
    )
    .unwrap()
    .value;

    // Force the final `flows_draft_update` write to genuinely fail: strip
    // write permission from the drafts dir after the draft file already
    // exists in it (create_dir_all is a no-op; the write of the new tmp
    // file inside it is what fails).
    let drafts_dir = config.workspace_dir.join("flows").join("drafts");
    std::fs::set_permissions(&drafts_dir, std::fs::Permissions::from_mode(0o555)).unwrap();
    let probe = drafts_dir.join(".write_probe");
    let write_is_blocked = std::fs::write(&probe, b"x").is_err();
    let _ = std::fs::remove_file(&probe);
    if !write_is_blocked {
        // Running as root — permissions are ignored, assertion is moot.
        std::fs::set_permissions(&drafts_dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        return;
    }

    let tool = EditWorkflowTool::new(config.clone());
    let result = tool
        .execute(json!({
            "draft_id": draft.id,
            "ops": [ { "op": "add_node", "node": { "id": "b", "kind": "merge", "name": "Join" } } ]
        }))
        .await
        .unwrap();

    // Restore so the tempdir can be cleaned up.
    std::fs::set_permissions(&drafts_dir, std::fs::Permissions::from_mode(0o755)).unwrap();

    assert!(result.is_error, "{}", result.output());
    assert!(
        !result.output().contains("Edits live on draft"),
        "must not claim the edit landed on the draft when the write-back failed: {}",
        result.output()
    );
    assert!(
        result.output().contains("PREVIOUS graph"),
        "{}",
        result.output()
    );

    // The draft on disk still holds the original (pre-edit) graph — the
    // write genuinely never landed.
    let reloaded = ops::flows_draft_get(&config, &draft.id).unwrap().value;
    assert_eq!(reloaded.graph["nodes"].as_array().unwrap().len(), 2);
}

// ── Phase 4: gated create / duplicate / debug loop (F4) ──────────────────────

#[tokio::test]
async fn create_workflow_creates_a_disabled_flow() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let tool = CreateWorkflowTool::new(config.clone());
    // valid_graph has a manual trigger — flows_create would normally make it
    // enabled; create_workflow must force it DISABLED.
    let result = tool
        .execute(json!({ "name": "Agent-made", "graph": valid_graph() }))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(parsed["type"], "workflow_created");
    assert_eq!(parsed["enabled"], false);
    // Persisted and really disabled.
    let flow_id = parsed["flow_id"].as_str().unwrap();
    let flow = ops::flows_get(&config, flow_id).await.unwrap().value;
    assert!(!flow.enabled, "agent-created flows are born disabled");
}

// T-m3: when the force-disable write itself fails, the response must
// report the flow's REAL state (still enabled) rather than unconditionally
// claiming "enabled": false. Exercised directly on the pure decision
// function `create_workflow_report` — reaching the true failure via a
// genuine concurrent store error would need a test-only seam inside
// `execute()` that production code shouldn't carry.
#[test]
fn create_workflow_report_is_honest_when_force_disable_fails() {
    let (enabled, note) = create_workflow_report(true, false);
    assert!(enabled, "must report the flow as still enabled");
    assert!(
        note.contains("ENABLED"),
        "note must surface the real state, not the intended DISABLED one: {note}"
    );
}

#[test]
fn create_workflow_report_reports_disabled_on_success() {
    let (enabled, note) = create_workflow_report(true, true);
    assert!(!enabled);
    assert!(note.contains("DISABLED"));
}

#[test]
fn create_workflow_report_never_attempted_disable_stays_disabled() {
    // born_enabled = false: flows_create already created it disabled
    // (e.g. an automatic-trigger graph), so no force-disable is attempted.
    let (enabled, note) = create_workflow_report(false, true);
    assert!(!enabled);
    assert!(note.contains("DISABLED"));
}

#[tokio::test]
async fn create_workflow_rejects_an_invalid_graph() {
    let tmp = TempDir::new().unwrap();
    let tool = CreateWorkflowTool::new(test_config(&tmp));
    let bad = json!({
        "nodes": [ { "id": "a", "kind": "output_parser", "name": "A" } ],
        "edges": []
    });
    let result = tool
        .execute(json!({ "name": "Bad", "graph": bad }))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("create_workflow again"));
}

#[tokio::test]
async fn duplicate_flow_creates_a_disabled_copy() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow = ops::flows_create(&config, "Original".to_string(), valid_graph(), false)
        .await
        .unwrap()
        .value;
    let tool = DuplicateFlowTool::new(config.clone());
    let result = tool.execute(json!({ "flow_id": flow.id })).await.unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(parsed["type"], "workflow_duplicated");
    assert_eq!(parsed["enabled"], false);
    assert_ne!(parsed["flow_id"].as_str().unwrap(), flow.id);
}

#[tokio::test]
async fn list_flow_runs_is_empty_for_a_fresh_flow() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow = ops::flows_create(&config, "F".to_string(), valid_graph(), false)
        .await
        .unwrap()
        .value;
    let tool = ListFlowRunsTool::new(config.clone());
    let result = tool.execute(json!({ "flow_id": flow.id })).await.unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(parsed["runs"].as_array().unwrap().len(), 0);
}

#[test]
fn phase4_write_tools_have_the_right_permissions() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    assert_eq!(
        CreateWorkflowTool::new(config.clone()).permission_level(),
        PermissionLevel::Write
    );
    assert!(CreateWorkflowTool::new(config.clone()).external_effect());
    assert_eq!(
        CancelFlowRunTool::new(config.clone()).permission_level(),
        PermissionLevel::Write
    );
    // T-M3 fix: cancel_flow_run now parks for approval like every other
    // write-class flow-run control tool.
    assert!(CancelFlowRunTool::new(config.clone()).external_effect());
    assert_eq!(
        ResumeFlowRunTool::new(config.clone()).permission_level(),
        PermissionLevel::Execute
    );
    assert_eq!(
        ListFlowRunsTool::new(config.clone()).permission_level(),
        PermissionLevel::None
    );
}

/// SECURITY (T-M3): the tool must refuse to cancel a run that belongs to a
/// DIFFERENT flow than the one the caller named — closing the "arbitrary
/// run_id, no ownership check" gap the tool's own doc used to admit.
#[tokio::test]
async fn cancel_flow_run_refuses_a_run_the_caller_does_not_own() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let owner_flow = ops::flows_create(
        &config,
        "owner".to_string(),
        cancel_test_approval_gated_graph(),
        false,
    )
    .await
    .unwrap()
    .value;
    let other_flow = ops::flows_create(
        &config,
        "other".to_string(),
        cancel_test_approval_gated_graph(),
        false,
    )
    .await
    .unwrap()
    .value;

    let run = ops::flows_run(
        &config,
        &owner_flow.id,
        json!({}),
        serde_json::Map::new(),
        crate::openhuman::flows::FlowRunTrigger::Rpc,
    )
    .await
    .unwrap();
    let run_id = run.value["thread_id"].as_str().unwrap().to_string();
    assert_eq!(
        ops::flows_get_run(&config, &run_id)
            .await
            .unwrap()
            .value
            .status,
        "pending_approval"
    );

    let tool = CancelFlowRunTool::new(config.clone());
    let result = tool
        .execute(json!({ "flow_id": other_flow.id, "run_id": run_id.clone() }))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(
        result.output().contains("belongs to flow"),
        "{}",
        result.output()
    );

    // The refused attempt must not have touched the run at all.
    let run_row = ops::flows_get_run(&config, &run_id).await.unwrap().value;
    assert_eq!(run_row.status, "pending_approval");
}
