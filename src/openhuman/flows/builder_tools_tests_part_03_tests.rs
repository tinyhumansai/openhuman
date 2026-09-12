use super::*;

#[tokio::test]
async fn dry_run_flags_null_resolved_agent_input_context() {
    // The B7 counterpart to `dry_run_flags_null_resolved_agent_prompt`:
    // `input_context` has been the agent's primary upstream-data channel
    // since #4590, so a null-resolved `input_context` is just as
    // execution-breaking as a null `prompt` — the agent runs with no
    // upstream data at all. Must fail the dry run the same way.
    let tool = DryRunWorkflowTool::new(test_config(&TempDir::new().unwrap()));
    let graph = json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "classify", "kind": "agent", "name": "Classify",
              "config": { "prompt": "Classify the email as urgent, normal, or low priority.",
                "input_context": "=nodes.missing.item.json.body" } }
        ],
        "edges": [ { "from_node": "t", "to_node": "classify" } ]
    });

    let result = tool.execute(json!({ "graph": graph })).await.unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(
        parsed["ok"], false,
        "a null-resolved agent input_context must fail the dry run: {parsed}"
    );
    let agent_input_context_nulls = parsed["agent_input_context_nulls"]
        .as_array()
        .expect("agent_input_context_nulls array");
    assert_eq!(agent_input_context_nulls.len(), 1, "{parsed}");
    assert_eq!(agent_input_context_nulls[0]["node_id"], "classify");
    assert_eq!(agent_input_context_nulls[0]["location"], "input_context");
    assert!(
        agent_input_context_nulls[0]["suggestion"]
            .as_str()
            .unwrap()
            .contains("upstream"),
        "{parsed}"
    );
}

#[tokio::test]
async fn dry_run_passes_when_agent_uses_input_context_instead_of_prompt_expression() {
    // The FALSE-POSITIVE-PREVENTION case: the same data need, wired the
    // correct way — `input_context` carries the upstream item, `prompt`
    // stays a plain instruction with no leading `=`. This must dry-run green.
    let tool = DryRunWorkflowTool::new(test_config(&TempDir::new().unwrap()));
    let graph = json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "classify", "kind": "agent", "name": "Classify",
              "config": { "prompt": "Classify the email as urgent, normal, or low priority.",
                "input_context": "=item" } }
        ],
        "edges": [ { "from_node": "t", "to_node": "classify" } ]
    });

    let result = tool.execute(json!({ "graph": graph })).await.unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(parsed["ok"], true, "{parsed}");
    assert!(
        parsed["agent_prompt_nulls"].as_array().unwrap().is_empty(),
        "{parsed}"
    );
    assert!(
        parsed["agent_input_context_nulls"]
            .as_array()
            .unwrap()
            .is_empty(),
        "{parsed}"
    );
}

#[tokio::test]
async fn dry_run_warns_on_unexercised_agent_after_condition() {
    // B15's dry-run blind spot: `gate` is a `condition` wired with only a
    // `true` edge to `classify`. The dry run's default trigger input is `{}`
    // (no `input` param passed), so `gate`'s configured field ("active") is
    // absent — falsey — and the condition emits `false`. Since `false` has no
    // outgoing edge, `classify` never executes at all: not a null resolution,
    // not a node error, just silently unexercised. A real trigger's payload
    // could easily carry `active: true` and take the other branch, so the
    // dry run must still surface this as a warning even though `ok` stays
    // `true` — there's nothing here that flips it to a hard reject.
    let tool = DryRunWorkflowTool::new(test_config(&TempDir::new().unwrap()));
    let graph = json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "gate", "kind": "condition", "name": "Gate",
              "config": { "field": "active" } },
            { "id": "classify", "kind": "agent", "name": "Classify",
              "config": { "prompt": "Classify the item.", "input_context": "=item" } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "gate" },
            { "from_node": "gate", "from_port": "true", "to_node": "classify" }
        ]
    });

    let result = tool.execute(json!({ "graph": graph })).await.unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(
        parsed["ok"], true,
        "an unexercised branch is a warning, not a hard reject: {parsed}"
    );
    let warnings = parsed["routing_divergence_warnings"]
        .as_array()
        .expect("routing_divergence_warnings array");
    assert_eq!(warnings.len(), 1, "{parsed}");
    assert_eq!(warnings[0]["node_id"], "classify");
    assert_eq!(warnings[0]["condition_node_id"], "gate");
    assert!(
        warnings[0]["message"]
            .as_str()
            .unwrap()
            .contains("classify"),
        "{parsed}"
    );
}

#[tokio::test]
async fn dry_run_no_routing_divergence_warning_when_every_node_executes() {
    // FALSE-POSITIVE-PREVENTION: a condition whose taken branch under the
    // default mock input DOES reach the downstream agent must not warn.
    let tool = DryRunWorkflowTool::new(test_config(&TempDir::new().unwrap()));
    let graph = json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "gate", "kind": "condition", "name": "Gate",
              "config": { "field": "active" } },
            { "id": "classify", "kind": "agent", "name": "Classify",
              "config": { "prompt": "Classify the item.", "input_context": "=item" } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "gate" },
            { "from_node": "gate", "from_port": "false", "to_node": "classify" }
        ]
    });

    let result = tool.execute(json!({ "graph": graph })).await.unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(parsed["ok"], true, "{parsed}");
    assert!(
        parsed["routing_divergence_warnings"]
            .as_array()
            .unwrap()
            .is_empty(),
        "{parsed}"
    );
}

/// (systemic tool-contract fix, Part 2b) A missing required Composio arg is
/// now a HARD REJECT at `revise_workflow` — `validate_tool_contracts` runs
/// ahead of the older advisory `graph_wiring_warnings` check and catches the
/// exact same condition first, so the graph never gets far enough to merely
/// warn about it. `graph_wiring_warnings`'s own required-arg warning (still
/// exercised directly in `ops_tests.rs`) stays as a defense-in-depth
/// fallback for any caller that doesn't also run `validate_tool_contracts`.
#[tokio::test]
async fn revise_workflow_rejects_a_missing_required_composio_arg() {
    seed_live_catalog_cache("gmail", vec![seeded_gmail_send_contract()]);

    let tmp = TempDir::new().unwrap();
    let tool = ReviseWorkflowTool::new(test_config(&tmp));
    let result = tool
        .execute(json!({
            "name": "Send mail",
            "graph": {
                "nodes": [
                    { "id": "t", "kind": "trigger", "name": "Manual" },
                    { "id": "send", "kind": "tool_call", "name": "Send",
                      // `body` wired via expression (counts as wired); `to` absent.
                      "config": { "slug": "GMAIL_SEND_EMAIL",
                                  "args": { "body": "=item.text" } } }
                ],
                "edges": [ { "from_node": "t", "to_node": "send" } ]
            }
        }))
        .await
        .unwrap();

    assert!(
        result.is_error,
        "a missing required arg must now hard-reject"
    );
    let output = result.output();
    assert!(output.contains("send"), "{output}");
    assert!(output.contains("`to`"), "{output}");
    // `body` is wired (expression) — never named as missing.
    assert!(!output.contains("`body`"), "{output}");
}

#[tokio::test]
async fn save_workflow_missing_flow_id_is_error() {
    let tmp = TempDir::new().unwrap();
    let tool = SaveWorkflowTool::new(test_config(&tmp));
    // Persisting a definition is a Write-class action (no external effect at
    // save time — the flow's own runs govern that).
    assert_eq!(tool.permission_level(), PermissionLevel::Write);
    assert!(!tool.external_effect());

    let result = tool
        .execute(json!({ "graph": valid_graph() }))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("Missing 'flow_id'"));
}

#[tokio::test]
async fn save_workflow_unknown_flow_is_error() {
    let tmp = TempDir::new().unwrap();
    let tool = SaveWorkflowTool::new(test_config(&tmp));

    let result = tool
        .execute(json!({ "flow_id": "nope", "graph": valid_graph() }))
        .await
        .unwrap();
    assert!(result.is_error, "save onto a nonexistent flow must fail");
    assert!(result.output().contains("nope"));
}

#[tokio::test]
async fn save_workflow_persists_graph_and_name_onto_existing_flow() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow_id = seed_flow(&config, "Blank flow").await;
    let tool = SaveWorkflowTool::new(config.clone());

    let result = tool
        .execute(json!({
            "flow_id": flow_id,
            "graph": valid_graph(),
            "name": "AI News Digest"
        }))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.output());

    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(parsed["type"], "workflow_saved");
    assert_eq!(parsed["flow_id"], flow_id.as_str());
    assert_eq!(parsed["name"], "AI News Digest");
    assert_eq!(parsed["node_count"], 2);
    // Enablement / approval gate are NOT touched by the tool.
    assert_eq!(parsed["require_approval"], true);

    // The graph + name really persisted.
    let saved = ops::flows_get(&config, &flow_id).await.unwrap().value;
    assert_eq!(saved.name, "AI News Digest");
    assert_eq!(saved.graph.nodes.len(), 2);
}

#[tokio::test]
async fn save_workflow_rejects_invalid_graph_and_leaves_flow_intact() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow_id = seed_flow(&config, "Blank flow").await;
    let tool = SaveWorkflowTool::new(config.clone());

    let result = tool
        .execute(json!({
            "flow_id": flow_id,
            // No trigger node — fails tinyflows validation.
            "graph": { "nodes": [ { "id": "a", "kind": "agent", "name": "A" } ], "edges": [] }
        }))
        .await
        .unwrap();
    assert!(result.is_error);

    let saved = ops::flows_get(&config, &flow_id).await.unwrap().value;
    assert_eq!(saved.name, "Blank flow");
    assert_eq!(
        saved.graph.nodes.len(),
        1,
        "original graph must be untouched"
    );
}

#[tokio::test]
async fn save_workflow_surfaces_auto_disarm_warning_on_manual_to_automatic_transition() {
    // Regression for #4889 + the stale-docs issue that motivated this test:
    // `flows_update` auto-disables a flow whenever its trigger transitions
    // from manual to automatic on an already-enabled flow, but `save_workflow`
    // used to drop `flows_update`'s explanatory `RpcOutcome.logs` entirely —
    // the agent had no way to relay the disarm to the user. Assert both the
    // disarm itself and that its log now surfaces in `save_workflow`'s
    // `warnings`.
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow_id = seed_flow(&config, "Manual flow").await;
    let seeded = ops::flows_get(&config, &flow_id).await.unwrap().value;
    assert!(
        seeded.enabled,
        "precondition: a manual-trigger flow persists enabled from create"
    );

    let tool = SaveWorkflowTool::new(config.clone());
    let result = tool
        .execute(json!({
            "flow_id": flow_id,
            "graph": schedule_trigger_graph(),
        }))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.output());

    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(
        parsed["enabled"], false,
        "manual→automatic transition on an enabled flow must auto-disable it: {parsed}"
    );
    let warnings = parsed["warnings"]
        .as_array()
        .expect("warnings must be an array");
    assert!(
        warnings
            .iter()
            .any(|w| w.as_str().unwrap_or("").contains("auto-disabled")),
        "save_workflow must surface flows_update's disarm log as a warning, got: {parsed}"
    );
    let flow_updated_boilerplate = format!("flow updated: {flow_id}");
    assert!(
        warnings
            .iter()
            .all(|w| w.as_str().unwrap_or("") != flow_updated_boilerplate),
        "save_workflow must exclude the redundant \"flow updated: <id>\" boilerplate \
         from warnings, got: {parsed}"
    );

    // Persisted, not just returned in-memory.
    let reloaded = ops::flows_get(&config, &flow_id).await.unwrap().value;
    assert!(!reloaded.enabled);
}

#[tokio::test]
async fn save_workflow_rejects_agent_binding_missing_declared_field() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow_id = seed_flow(&config, "Blank flow").await;
    let tool = SaveWorkflowTool::new(config.clone());

    let result = tool
        .execute(json!({ "flow_id": flow_id, "graph": unresolvable_binding_graph() }))
        .await
        .unwrap();

    assert!(result.is_error, "must be rejected: {}", result.output());
    let output = result.output();
    assert!(output.contains("notify"), "{output}");
    assert!(output.contains("channel"), "{output}");
    assert!(output.contains("summarize"), "{output}");
    assert!(output.contains("output_parser.schema"), "{output}");

    // The flow it tried to save onto must be untouched.
    let saved = ops::flows_get(&config, &flow_id).await.unwrap().value;
    assert_eq!(saved.name, "Blank flow");
    assert_eq!(
        saved.graph.nodes.len(),
        1,
        "original graph must be untouched"
    );
}

#[tokio::test]
async fn save_workflow_accepts_correctly_schemad_graph() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow_id = seed_flow(&config, "Blank flow").await;
    let tool = SaveWorkflowTool::new(config.clone());

    let graph = json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "summarize", "kind": "agent", "name": "Summarize",
              "config": { "agent_ref": "researcher", "prompt": "summarize",
                "output_parser": { "schema": { "type": "object",
                    "required": ["channel"],
                    "properties": { "channel": { "type": "string" } } } } } },
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
        .execute(json!({ "flow_id": flow_id, "graph": graph, "name": "Summarize and notify" }))
        .await
        .unwrap();

    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(parsed["type"], "workflow_saved");
    assert_eq!(parsed["node_count"], 3);

    let saved = ops::flows_get(&config, &flow_id).await.unwrap().value;
    assert_eq!(saved.name, "Summarize and notify");
    assert_eq!(saved.graph.nodes.len(), 3);
}

#[tokio::test]
async fn list_node_kinds_tool_returns_every_kind() {
    let tool = ListNodeKindsTool::new();
    let result = tool.execute(json!({})).await.unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    let kinds = parsed["node_kinds"].as_array().unwrap();
    assert_eq!(kinds.len(), crate::openhuman::flows::NODE_KINDS.len());
    // The tool must advertise the whole catalog, not a subset that happens to
    // include the kinds someone remembered to name here — a kind the engine
    // knows but this tool omits is a kind the builder agent cannot reach.
    for kind in crate::openhuman::flows::NODE_KINDS {
        assert!(
            kinds.iter().any(|k| k["kind"] == kind),
            "list_node_kinds omits `{kind}`"
        );
    }
    // Each entry carries a kind + summary + the config-field name lists.
    assert!(kinds.iter().all(|k| k.get("summary").is_some()));
}

#[tokio::test]
async fn get_node_kind_contract_tool_returns_contract_and_rejects_unknown() {
    let tool = GetNodeKindContractTool::new();

    let ok = tool.execute(json!({ "kind": "tool_call" })).await.unwrap();
    assert!(!ok.is_error, "{}", ok.output());
    let parsed: Value = serde_json::from_str(&ok.output()).unwrap();
    assert_eq!(parsed["kind"], "tool_call");
    assert!(parsed["config_fields"]
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f["name"] == "slug"));
    // Host overlay is present on the tool's output.
    assert!(parsed["notes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|n| n.as_str().unwrap_or("").contains("Composio")));

    let bad = tool.execute(json!({ "kind": "nope" })).await.unwrap();
    assert!(bad.is_error);
    assert!(bad.output().contains("list_node_kinds"));
    assert!(bad.output().contains(&format!(
        "{} valid kinds",
        crate::openhuman::flows::NODE_KINDS.len()
    )));

    let missing = tool.execute(json!({})).await.unwrap();
    assert!(missing.is_error);
}

// ── edit_workflow (F1: structured incremental edits) ─────────────────────────

#[tokio::test]
async fn edit_workflow_applies_ops_to_inline_graph_and_returns_proposal() {
    let tmp = TempDir::new().unwrap();
    let tool = EditWorkflowTool::new(test_config(&tmp));

    // Add a merge node `b` and wire the agent into it.
    let result = tool
        .execute(json!({
            "graph": valid_graph(),
            "name": "Edited flow",
            "instruction": "add a merge step",
            "ops": [
                { "op": "add_node", "node": { "id": "b", "kind": "merge", "name": "Join" } },
                { "op": "add_edge", "edge": { "from_node": "a", "to_node": "b" } }
            ]
        }))
        .await
        .unwrap();

    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(parsed["type"], "workflow_proposal");
    assert_eq!(parsed["name"], "Edited flow");
    assert_eq!(parsed["graph"]["nodes"].as_array().unwrap().len(), 3);
    assert_eq!(parsed["graph"]["edges"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn edit_workflow_update_node_config_merge_patches() {
    let tmp = TempDir::new().unwrap();
    let tool = EditWorkflowTool::new(test_config(&tmp));

    let result = tool
        .execute(json!({
            "graph": valid_graph(),
            "ops": [
                { "op": "update_node_config", "id": "a", "config": { "prompt": "new instruction" } }
            ]
        }))
        .await
        .unwrap();

    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    let nodes = parsed["graph"]["nodes"].as_array().unwrap();
    let agent = nodes.iter().find(|n| n["id"] == "a").unwrap();
    assert_eq!(agent["config"]["prompt"], "new instruction");
}

#[tokio::test]
async fn edit_workflow_requires_a_base() {
    let tmp = TempDir::new().unwrap();
    let tool = EditWorkflowTool::new(test_config(&tmp));
    let result = tool
        .execute(json!({ "ops": [ { "op": "remove_node", "id": "a" } ] }))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("flow_id"));
}

#[tokio::test]
async fn edit_workflow_reports_failing_op_with_guidance() {
    let tmp = TempDir::new().unwrap();
    let tool = EditWorkflowTool::new(test_config(&tmp));
    let result = tool
        .execute(json!({
            "graph": valid_graph(),
            "ops": [ { "op": "remove_node", "id": "ghost" } ]
        }))
        .await
        .unwrap();
    assert!(result.is_error);
    let out = result.output();
    assert!(out.contains("remove_node"), "{out}");
    assert!(out.contains("edit_workflow again"), "{out}");
}

#[tokio::test]
async fn edit_workflow_bad_op_reports_index_type_and_shape() {
    let tmp = TempDir::new().unwrap();
    let tool = EditWorkflowTool::new(test_config(&tmp));
    // ops 0 and 1 are well-formed; op 2 is an add_node missing its `node`.
    let result = tool
        .execute(json!({
            "graph": valid_graph(),
            "ops": [
                { "op": "set_node_name", "id": "a", "name": "One" },
                { "op": "set_node_name", "id": "a", "name": "Two" },
                { "op": "add_node", "id": "b" }
            ]
        }))
        .await
        .unwrap();
    assert!(result.is_error, "{}", result.output());
    let out = result.output();
    // Names the failing op index, its op type, and the expected shape for it.
    assert!(out.contains("op 2"), "{out}");
    assert!(out.contains("add_node"), "{out}");
    assert!(out.contains("node:"), "expected add_node shape in: {out}");
    assert!(out.contains("edit_workflow again"), "{out}");
}

#[tokio::test]
async fn edit_workflow_missing_op_field_lists_valid_types() {
    let tmp = TempDir::new().unwrap();
    let tool = EditWorkflowTool::new(test_config(&tmp));
    let result = tool
        .execute(json!({
            "graph": valid_graph(),
            "ops": [ { "id": "a", "name": "No op tag" } ]
        }))
        .await
        .unwrap();
    assert!(result.is_error, "{}", result.output());
    let out = result.output();
    assert!(out.contains("op 0"), "{out}");
    assert!(out.contains("missing `op` field"), "{out}");
    assert!(out.contains("update_node_config"), "{out}");
}

#[tokio::test]
async fn edit_workflow_add_node_exists_carries_ordering_hint() {
    let tmp = TempDir::new().unwrap();
    let tool = EditWorkflowTool::new(test_config(&tmp));
    // Re-adding an existing node id fails in-order; the hint should point at the
    // remove-first / patch-in-place fix.
    let result = tool
        .execute(json!({
            "graph": valid_graph(),
            "ops": [
                { "op": "add_node", "node": { "id": "a", "kind": "merge", "name": "Dup" } }
            ]
        }))
        .await
        .unwrap();
    assert!(result.is_error, "{}", result.output());
    let out = result.output();
    assert!(out.contains("already exists"), "{out}");
    assert!(out.contains("array order"), "{out}");
    assert!(out.contains("remove_node"), "{out}");
    assert!(out.contains("update_node_config"), "{out}");
}

#[tokio::test]
async fn edit_workflow_accepts_node_id_aliases_end_to_end() {
    let tmp = TempDir::new().unwrap();
    let tool = EditWorkflowTool::new(test_config(&tmp));
    // A valid ops array using the `node_id` alias (the natural agent guess)
    // applies cleanly through edit_workflow.
    let result = tool
        .execute(json!({
            "graph": valid_graph(),
            "name": "Aliased edit",
            "ops": [
                { "op": "update_node_config", "node_id": "a", "config": { "prompt": "aliased" } },
                { "op": "set_node_name", "node_id": "a", "name": "Aliased step" }
            ]
        }))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(parsed["type"], "workflow_proposal");
    let nodes = parsed["graph"]["nodes"].as_array().unwrap();
    let agent = nodes.iter().find(|n| n["id"] == "a").unwrap();
    assert_eq!(agent["config"]["prompt"], "aliased");
    assert_eq!(agent["name"], "Aliased step");
}
