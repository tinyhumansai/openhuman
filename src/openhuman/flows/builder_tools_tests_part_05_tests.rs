use super::*;

/// No-regression companion: cancelling with the CORRECT owning flow_id must
/// still work exactly as before the T-M3 fix.
#[tokio::test]
async fn cancel_flow_run_cancels_when_flow_id_matches_the_owner() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let flow = ops::flows_create(
        &config,
        "F".to_string(),
        cancel_test_approval_gated_graph(),
        false,
    )
    .await
    .unwrap()
    .value;
    let run = ops::flows_run(
        &config,
        &flow.id,
        json!({}),
        serde_json::Map::new(),
        crate::openhuman::flows::FlowRunTrigger::Rpc,
    )
    .await
    .unwrap();
    let run_id = run.value["thread_id"].as_str().unwrap().to_string();

    let tool = CancelFlowRunTool::new(config.clone());
    let result = tool
        .execute(json!({ "flow_id": flow.id, "run_id": run_id.clone() }))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.output());

    let run_row = ops::flows_get_run(&config, &run_id).await.unwrap().value;
    assert_eq!(run_row.status, "cancelled");
}

#[tokio::test]
async fn cancel_flow_run_missing_flow_id_errs() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let tool = CancelFlowRunTool::new(config);
    let result = tool.execute(json!({ "run_id": "some-run" })).await.unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("flow_id"));
}

/// T-M3 (part b): the approval gate routes any `external_effect() == true`
/// tool through `ApprovalGate` before `execute()` runs
/// (`ApprovalSecurityMiddleware::has_external_effect` in
/// `tinyagents::middleware`, keyed purely off `external_effect_with_args`).
/// `cancel_flow_run` now reports `external_effect() == true`
/// (`phase4_write_tools_have_the_right_permissions` above pins the flag
/// itself), so it parks on any surface with a live gate — exactly like
/// `resume_flow_run` — instead of executing unapproved.
#[test]
fn cancel_flow_run_is_external_effect_so_the_middleware_parks_it() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let tool = CancelFlowRunTool::new(config);
    assert!(
        tool.external_effect(),
        "cancel_flow_run must be external_effect so ApprovalSecurityMiddleware routes it \
         through ApprovalGate::intercept_audited before execute() runs"
    );
}

// ── WS2: unified draft_id|flow_id|graph handles + explicit persistence state ──

#[tokio::test]
async fn edit_workflow_by_flow_id_seeds_a_retrievable_draft_and_marks_unpersisted() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    // A saved flow to edit — editing it must NOT write onto the flow (the WS2
    // bug: a flow_id edit used to persist nothing and return no handle).
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

    // The edit lives on a NEW draft, is explicitly NOT persisted, and echoes the
    // flow it derives from plus a `next` hint naming the draft.
    assert_eq!(parsed["persisted"], false);
    assert_eq!(parsed["flow_id"], flow.id.as_str());
    let draft_id = parsed["draft_id"]
        .as_str()
        .expect("edit_workflow by flow_id returns a draft_id")
        .to_string();
    assert!(parsed["next"].as_str().unwrap().contains(&draft_id));

    // The draft is retrievable via ops::flows_draft_get and holds the EDITED
    // graph, linked back to the source flow.
    let draft = ops::flows_draft_get(&config, &draft_id).unwrap().value;
    assert_eq!(draft.flow_id.as_deref(), Some(flow.id.as_str()));
    let agent = draft.graph["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"] == "a")
        .unwrap();
    assert_eq!(agent["name"], "Renamed step");

    // The SAVED flow is untouched — the whole point of WS2.
    let saved = ops::flows_get(&config, &flow.id).await.unwrap().value;
    let saved_graph = serde_json::to_value(&saved.graph).unwrap();
    let saved_agent = saved_graph["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"] == "a")
        .unwrap();
    assert_eq!(
        saved_agent["name"], "Summarize",
        "the flow must not be edited"
    );
}

#[tokio::test]
async fn dry_run_workflow_by_flow_id_runs_the_saved_flow_graph() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow = ops::flows_create(&config, "Runnable".to_string(), valid_graph(), false)
        .await
        .unwrap()
        .value;
    let tool = DryRunWorkflowTool::new(config.clone());
    let result = tool.execute(json!({ "flow_id": flow.id })).await.unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(parsed["sandbox"], true);
    assert_eq!(parsed["ok"], true);
}

#[tokio::test]
async fn validate_workflow_by_draft_id_checks_the_draft_graph() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let draft = ops::flows_draft_create(
        &config,
        None,
        "Draft".to_string(),
        valid_graph(),
        crate::openhuman::flows::DraftOrigin::Chat,
    )
    .unwrap()
    .value;
    let tool = ValidateWorkflowTool::new(config.clone());
    let result = tool.execute(json!({ "draft_id": draft.id })).await.unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(parsed["ok"], true);
    assert_eq!(parsed["structurally_valid"], true);
}

#[tokio::test]
async fn save_workflow_by_draft_id_persists_the_draft_graph_onto_the_flow() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    // A flow seeded with a bare 1-node graph.
    let flow_id = seed_flow(&config, "Blank flow").await;
    // A draft holding the richer 2-node valid graph, linked to that flow.
    let draft = ops::flows_draft_create(
        &config,
        Some(flow_id.clone()),
        "Draft".to_string(),
        valid_graph(),
        crate::openhuman::flows::DraftOrigin::Chat,
    )
    .unwrap()
    .value;

    let tool = SaveWorkflowTool::new(config.clone());
    let result = tool
        .execute(json!({ "flow_id": flow_id, "draft_id": draft.id }))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(parsed["type"], "workflow_saved");
    assert_eq!(parsed["persisted"], true);
    assert_eq!(parsed["node_count"], 2);

    // The draft's graph really landed on the flow.
    let saved = ops::flows_get(&config, &flow_id).await.unwrap().value;
    assert_eq!(saved.graph.nodes.len(), 2);
}

#[tokio::test]
async fn revise_workflow_proposal_is_marked_unpersisted() {
    let tmp = TempDir::new().unwrap();
    let tool = ReviseWorkflowTool::new(test_config(&tmp));
    let result = tool
        .execute(json!({ "name": "R", "graph": valid_graph() }))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(parsed["persisted"], false);
}

/// Docs-drift guard (T-m2): the top-of-file module doc table went stale
/// enough to list 11 of ~22 tools, mis-describe `DryRunWorkflowTool`'s
/// permission, and claim a `create_workflow`-adjacent invariant the code
/// didn't hold — all silently, because nothing checked the table against the
/// actual `impl Tool for` list. This mirrors the pattern
/// `propose_workflow_description_matches_typed_node_contracts`
/// (`tools_tests.rs`) established for node-kind contracts: derive the ground
/// truth from the SAME source file rather than hardcoding a second list here
/// (a hardcoded list would just be a new place to go stale), and fail loudly
/// in both directions — a real tool missing from the table, or a table entry
/// naming a tool that no longer exists.
#[test]
fn module_doc_tool_table_matches_registered_tools() {
    const SOURCE: &str = concat!(
        include_str!("builder_tools.rs"),
        include_str!("builder_tools_part_01.rs"),
        include_str!("builder_tools_part_02.rs"),
        include_str!("builder_tools_part_03.rs"),
        include_str!("builder_tools_part_04.rs"),
        include_str!("builder_tools_part_05.rs"),
        include_str!("builder_tools_part_06.rs"),
        include_str!("builder_tools_part_07.rs"),
    );

    let module_doc: String = SOURCE
        .lines()
        .filter(|line| line.trim_start().starts_with("//!"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !module_doc.is_empty(),
        "sanity: expected builder_tools.rs to carry a top-of-file `//!` module doc"
    );

    let impl_re = regex::Regex::new(r"impl Tool for (\w+)\s*\{").expect("valid regex");
    let registered: std::collections::BTreeSet<String> = impl_re
        .captures_iter(SOURCE)
        .map(|c| c[1].to_string())
        .collect();
    assert!(
        !registered.is_empty(),
        "sanity: expected at least one `impl Tool for` in the builder_tools module"
    );

    for tool in &registered {
        assert!(
            module_doc.contains(tool.as_str()),
            "module doc table is missing `{tool}` — every `impl Tool for` in this file \
             must be listed in the top-of-file doc table (T-m2)"
        );
    }

    // The reverse direction: every `[`FooTool`]` reference in the doc must
    // name a tool that actually still exists, so a removed/renamed tool
    // can't leave a stale row behind.
    let doc_ref_re = regex::Regex::new(r"\[`(\w+)`\]").expect("valid regex");
    for cap in doc_ref_re.captures_iter(&module_doc) {
        let name: &str = &cap[1];
        if name.ends_with("Tool") {
            assert!(
                registered.contains(name),
                "module doc table references `{name}`, but no `impl Tool for {name}` exists \
                 in this file — the doc table has a stale entry"
            );
        }
    }
}
