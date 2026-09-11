use super::*;

/// CodeRabbit (PR #4702 review): parity coverage for the probe-override path
/// in `graph_output_field_warnings` — mirrors
/// `graph_wiring_warnings_suggests_the_probed_split_out_path_when_schema_is_unknown`
/// above, but for a downstream FIELD binding rather than `split_out.path`.
/// With no schema at all (`output_schema: None`, `output_fields: []`), the
/// field-not-in-output_fields check would otherwise stay silent (nothing
/// real to check against) — once `get_tool_output_sample` has probed the
/// slug, the probed `output_fields` become the ground truth: a binding to a
/// probed-real field is silent, and a binding to a field NOT in the probed
/// set is flagged, exactly like the schema-known case already covers.
#[tokio::test]
async fn graph_wiring_warnings_uses_the_probed_output_fields_when_schema_is_unknown() {
    let contract = ToolContract {
        slug: "GHPROBEFIELDS_LIST_REPOSITORY_ISSUES".to_string(),
        toolkit: "ghprobefields".to_string(),
        description: None,
        required_args: vec!["owner".to_string(), "repo".to_string()],
        input_schema: None,
        output_fields: vec![],
        output_schema: None,
        primary_array_path: None,
        is_curated: true,
    };
    seed_live_catalog_cache("ghprobefields", vec![contract]);
    seed_probe_cache(
        "GHPROBEFIELDS_LIST_REPOSITORY_ISSUES",
        ProbedOutputSample {
            primary_array_path: Some("data.issues".to_string()),
            output_fields: vec!["issues".to_string(), "total_count".to_string()],
            sample: json!({ "data": { "issues": [], "total_count": 0 } }),
        },
    );
    let config = Config::default();

    // A binding to a field the probe actually observed — silent.
    let real_field = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "GHPROBEFIELDS_LIST_REPOSITORY_ISSUES",
                "args": { "owner": "acme", "repo": "widgets" } } },
            { "id": "xform", "kind": "transform", "name": "Log",
              "config": { "set": { "note": "=nodes.post.item.json.data.total_count" } } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "post" },
            { "from_node": "post", "to_node": "xform" }
        ]
    }));
    assert!(
        graph_wiring_warnings(&config, &real_field).await.is_empty(),
        "a probed-real field must not warn: {:?}",
        graph_wiring_warnings(&config, &real_field).await
    );

    // A binding to a field the probe did NOT observe — flagged, using the
    // probed output_fields as ground truth even though the schema itself is
    // unknown.
    let fake_field = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "GHPROBEFIELDS_LIST_REPOSITORY_ISSUES",
                "args": { "owner": "acme", "repo": "widgets" } } },
            { "id": "xform", "kind": "transform", "name": "Log",
              "config": { "set": { "note": "=nodes.post.item.json.data.not_a_probed_field" } } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "post" },
            { "from_node": "post", "to_node": "xform" }
        ]
    }));
    let warnings = graph_wiring_warnings(&config, &fake_field).await;
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("not_a_probed_field") && w.contains("post")),
        "{warnings:?}"
    );
}

#[test]
fn degrade_completed_status_all_clean_stays_completed() {
    let steps = vec![clean_step("a"), clean_step("b")];
    assert_eq!(degrade_completed_status(&steps), "completed");
}

#[test]
fn degrade_completed_status_null_binding_becomes_warnings() {
    let mut warned = clean_step("a");
    warned.diagnostics = vec![json!({ "location": "args.to", "expression": "=item.to" })];
    let steps = vec![clean_step("trigger"), warned];
    assert_eq!(degrade_completed_status(&steps), "completed_with_warnings");
}

#[test]
fn degrade_completed_status_errored_step_becomes_failed() {
    let mut errored = clean_step("a");
    errored.status = Some("error".to_string());
    let steps = vec![clean_step("trigger"), errored];
    assert_eq!(degrade_completed_status(&steps), "failed");
}

#[test]
fn degrade_completed_status_error_outranks_diagnostics() {
    // A step can carry both an error status and null-resolution diagnostics
    // (e.g. it errored trying to use the unresolved value) — failed wins.
    let mut errored_with_diagnostics = clean_step("a");
    errored_with_diagnostics.status = Some("error".to_string());
    errored_with_diagnostics.diagnostics =
        vec![json!({ "location": "args.to", "expression": "=item.to" })];
    let steps = vec![errored_with_diagnostics];
    assert_eq!(degrade_completed_status(&steps), "failed");
}

#[test]
fn failed_step_error_summary_none_when_no_step_errored() {
    let steps = vec![clean_step("a"), clean_step("b")];
    assert_eq!(failed_step_error_summary(&steps), None);
}

#[test]
fn failed_step_error_summary_names_the_errored_node() {
    let mut errored = clean_step("x");
    errored.status = Some("error".to_string());
    let steps = vec![clean_step("trigger"), errored];
    let summary = failed_step_error_summary(&steps).expect("an errored step must summarize");
    assert!(summary.contains('x'), "got: {summary}");
}

#[test]
fn failed_step_error_summary_names_every_errored_node() {
    let mut errored_a = clean_step("a");
    errored_a.status = Some("error".to_string());
    let mut errored_b = clean_step("b");
    errored_b.status = Some("error".to_string());
    let steps = vec![errored_a, errored_b];
    let summary = failed_step_error_summary(&steps).unwrap();
    assert!(
        summary.contains('a') && summary.contains('b'),
        "got: {summary}"
    );
}

#[test]
fn envelope_violation_detected() {
    // `summarize` DOES declare a matching schema, but the binding reaches
    // into `.item.channel` (skipping `.json`) — that dereferences the
    // `{json,text,raw}` envelope wrapper itself, not the field inside it.
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "summarize", "kind": "agent", "name": "Summarize",
              "config": { "prompt": "summarize",
                "output_parser": { "schema": { "type": "object",
                    "properties": { "channel": { "type": "string" } } } } } },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "SLACK_SEND_MESSAGE",
                "args": { "channel": "=nodes.summarize.item.channel" } } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "summarize" },
            { "from_node": "summarize", "to_node": "post" }
        ]
    }));
    let errors = validate_binding_resolvability(&g);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("json"), "{}", errors[0]);
    assert!(errors[0].contains("summarize"), "{}", errors[0]);
}

#[test]
fn non_enveloping_node_binding_is_accepted() {
    // `code` nodes emit their item directly (no envelope) — `.item.<field>`
    // is the correct, and only, form.
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "compute", "kind": "code", "name": "Compute",
              "config": { "language": "javascript", "source": "return {channel:'general'};" } },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "SLACK_SEND_MESSAGE",
                "args": { "channel": "=nodes.compute.item.channel" } } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "compute" },
            { "from_node": "compute", "to_node": "post" }
        ]
    }));
    assert!(
        validate_binding_resolvability(&g).is_empty(),
        "{:?}",
        validate_binding_resolvability(&g)
    );
}

#[test]
fn literal_args_unaffected() {
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "SLACK_SEND_MESSAGE",
                "args": { "channel": "general", "count": 3, "cc": ["a@b.com"] } } }
        ],
        "edges": [ { "from_node": "t", "to_node": "post" } ]
    }));
    assert!(validate_binding_resolvability(&g).is_empty());
}

#[test]
fn agent_prompt_binding_unaffected() {
    // The field-addressability checks are scoped to `tool_call` `args` only
    // — an agent's own `prompt` referencing a dangling/unschemad node path is
    // NOT inspected for that, even though it IS inspected for the narrower
    // "reads as prose, not jq" case (see the tests below). A simple dotted
    // path — even one pointing at a missing node — is a real, valid
    // expression (it just resolves to `null` at runtime, same as any other
    // dangling reference), so it's accepted here.
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "summarize", "kind": "agent", "name": "Summarize",
              "config": { "prompt": "=nodes.missing.item.channel" } }
        ],
        "edges": [ { "from_node": "t", "to_node": "summarize" } ]
    }));
    assert!(validate_binding_resolvability(&g).is_empty());
}

// ── agent-prompt invalid-jq gate (PR C) ─────────────────────────────────────

#[test]
fn agent_prompt_prose_written_as_expression_is_rejected() {
    // The exact live-failure shape: a builder smuggled upstream data into the
    // prompt via a jq `=`-expression, but the result is prose, not a valid jq
    // program — it resolves to `null` at runtime, handing the agent an empty
    // prompt (the root-cause bug `input_context` exists to fix).
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "classify", "kind": "agent", "name": "Classify",
              "config": { "prompt": "=You are given an email: .item. Classify the following \
                  email as urgent/normal/low priority. Return JSON with fields \"priority\" and \
                  \"reason\"." } }
        ],
        "edges": [ { "from_node": "t", "to_node": "classify" } ]
    }));
    let errors = validate_binding_resolvability(&g);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("classify"), "{}", errors[0]);
    // The gate is `tinyflows::gates`', so the message says what is wrong with
    // the graph and stops there. WHERE to put the data instead — this host's
    // `input_context` node-config key, which its own LLM adapter reads and
    // which the engine knows nothing about — is taught by the workflow_builder
    // archetype, not by a crate-level error.
    assert!(errors[0].contains("does not interpolate"), "{}", errors[0]);
}

#[test]
fn agent_prompt_jq_concatenation_is_accepted() {
    // A real jq program built from string-literal concatenation is a
    // legitimate, resolvable expression — not the prose failure mode above.
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "greet", "kind": "agent", "name": "Greet",
              "config": { "prompt": "=\"Hi \" + .item.name" } }
        ],
        "edges": [ { "from_node": "t", "to_node": "greet" } ]
    }));
    assert!(
        validate_binding_resolvability(&g).is_empty(),
        "{:?}",
        validate_binding_resolvability(&g)
    );
}

#[test]
fn agent_plain_prompt_is_accepted() {
    // No leading `=` at all — an ordinary instruction string, never inspected
    // by this gate regardless of content.
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "classify", "kind": "agent", "name": "Classify",
              "config": { "prompt": "Classify the email as urgent, normal, or low priority.",
                "input_context": "=item" } }
        ],
        "edges": [ { "from_node": "t", "to_node": "classify" } ]
    }));
    assert!(validate_binding_resolvability(&g).is_empty());
}

#[test]
fn agent_prompt_with_escaped_quote_inside_jq_string_is_accepted() {
    // Regression for the quote-toggle desync: an escaped quote (`\"`) inside
    // a jq string literal must not flip the strip pass's `in_str` state.
    // Before the fix, the text between the escaped quote and the string's
    // real closing quote ("hello world") leaked out of the string-stripping
    // pass as if it were bare jq code, tripping the "two consecutive
    // barewords" prose heuristic and rejecting this otherwise-valid
    // concatenation expression.
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "greet", "kind": "agent", "name": "Greet",
              "config": { "prompt": "=\"Say \\\"hello world\\\" nicely\" + .item.name" } }
        ],
        "edges": [ { "from_node": "t", "to_node": "greet" } ]
    }));
    assert!(
        validate_binding_resolvability(&g).is_empty(),
        "{:?}",
        validate_binding_resolvability(&g)
    );
}

#[test]
fn agent_prose_prompt_with_populated_messages_is_accepted() {
    // Both runtime paths (`build_completion_messages` /
    // `node_request_to_prompt` in `tinyflows/caps.rs`) fall through to a
    // populated `messages` array once `prompt` resolves to `null` — exactly
    // what this prose-as-`=`-expression prompt does. So a node with real
    // `messages` never actually runs on the null prompt; this gate must not
    // reject the graph for a vestigial/unused `prompt` field alongside it.
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "classify", "kind": "agent", "name": "Classify",
              "config": {
                  "prompt": "=You are given an email: .item. Classify the following email.",
                  "messages": [ { "role": "user", "content": "Classify this email." } ]
              } }
        ],
        "edges": [ { "from_node": "t", "to_node": "classify" } ]
    }));
    assert!(
        validate_binding_resolvability(&g).is_empty(),
        "{:?}",
        validate_binding_resolvability(&g)
    );
}

#[test]
fn agent_prose_prompt_with_empty_messages_is_still_rejected() {
    // An empty `messages` array doesn't supply the turn at runtime (both
    // `build_completion_messages` and `node_request_to_prompt` treat an empty
    // array the same as absent) — the prose-prompt gate must still apply.
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "classify", "kind": "agent", "name": "Classify",
              "config": {
                  "prompt": "=You are given an email: .item. Classify the following email.",
                  "messages": []
              } }
        ],
        "edges": [ { "from_node": "t", "to_node": "classify" } ]
    }));
    let errors = validate_binding_resolvability(&g);
    assert_eq!(errors.len(), 1, "{errors:?}");
}

#[test]
fn finalize_terminal_status_pending_approval_wins_over_error() {
    // Precedence: an outstanding pending_approval always wins, even if a step
    // also settled with an error — mirrors degrade_completed_status's own
    // precedence rule, now centralized in finalize_terminal_status.
    let mut errored = clean_step("a");
    errored.status = Some("error".to_string());
    let steps = vec![errored];
    let (status, error) = finalize_terminal_status(&steps, &["gate".to_string()]);
    assert_eq!(status, "pending_approval");
    assert_eq!(error, None);
}

#[test]
fn finalize_terminal_status_populates_error_on_degraded_failure() {
    let mut errored = clean_step("x");
    errored.status = Some("error".to_string());
    let steps = vec![errored];
    let (status, error) = finalize_terminal_status(&steps, &[]);
    assert_eq!(status, "failed");
    assert!(error.unwrap().contains('x'));
}

#[test]
fn finalize_terminal_status_no_error_when_clean() {
    let steps = vec![clean_step("a")];
    let (status, error) = finalize_terminal_status(&steps, &[]);
    assert_eq!(status, "completed");
    assert_eq!(error, None);
}

/// Regression for issue #4593 (widened for #4881's `resume_flow_run`/
/// `cancel_flow_run` addition to the belt): the `flows_build` builder turn
/// runs under `AgentTurnOrigin::Cli`, which makes the `ApprovalGate`
/// auto-allow every `external_effect` tool. The flows live-runner (`run_flow`)
/// and the run-resume tool (`resume_flow_run`) both execute/advance a *live*
/// saved flow's real outbound effects, so both must be unreachable on this
/// path — `restrict_builder_toolset` drops them (plus `cancel_flow_run`, out
/// of caution) from the builder's callable belt while leaving the authoring
/// tools in place so the turn still functions (never fail-closes).
#[tokio::test]
async fn flows_build_hides_the_live_run_tool_from_the_builder_belt() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    // Document WHY each run-advancing tool must be hidden: running or
    // resuming a saved flow fires real Slack/Gmail/HTTP/code effects, so both
    // are external-effect tools. This pins that invariant independently of
    // belt name-resolution so the hide-list can't silently stop covering a
    // live-run/resume tool.
    use crate::openhuman::tools::Tool as _;
    let live_runner =
        crate::openhuman::flows::tools::RunFlowTool::new(std::sync::Arc::new(config.clone()));
    assert!(
        live_runner.external_effect(),
        "the flows live-runner must be external-effect for the #4593 concern to apply"
    );
    let resumer = crate::openhuman::flows::builder_tools::ResumeFlowRunTool::new(
        std::sync::Arc::new(config.clone()),
    );
    assert!(
        resumer.external_effect(),
        "resume_flow_run advances a real run's outbound effects, so it must be \
         external-effect for the same #4593/#4881 concern to apply"
    );
    let canceller = crate::openhuman::flows::builder_tools::CancelFlowRunTool::new(
        std::sync::Arc::new(config.clone()),
    );
    assert!(
        canceller.external_effect(),
        "cancel_flow_run is external-effect since the T-M3 fix — it stays hidden on THIS \
         (Cli-origin, auto-allow) path regardless, because that gate is exactly what this \
         origin bypasses; see restrict_builder_toolset's doc"
    );

    // Building an agent constructs a memory client, which needs the host seams
    // wired. `Once`-guarded, so this is free when another test got there first.
    crate::openhuman::agent::harness::AgentDefinitionRegistry::init_global(&config.workspace_dir)
        .expect("agent registry init");
    let mut agent =
        crate::openhuman::agent::Agent::from_config_for_agent(&config, "workflow_builder")
            .expect("build workflow_builder agent");
    agent.set_agent_definition_name("workflow_builder".to_string());

    // Precondition: the builder advertises all four run-advancing tools on its
    // belt before restriction — the exact set #4593/#4881 are about.
    let visible_before = agent.visible_tool_names_for_test();
    for present in ["run_flow", "resume_flow_run", "cancel_flow_run"] {
        assert!(
            visible_before.contains(present),
            "precondition: workflow_builder belt should advertise `{present}`; visible = \
             {visible_before:?}"
        );
    }

    restrict_builder_toolset(&mut agent);

    // After restriction none of the run-advancing tools are callable on the
    // flows_build path — the hide-list covers all of them (#4593 + #4881).
    let visible = agent.visible_tool_names_for_test();
    for hidden in [
        "run_workflow",
        "run_flow",
        "resume_flow_run",
        "cancel_flow_run",
    ] {
        assert!(
            !visible.contains(hidden),
            "run-advancing tool `{hidden}` must be hidden on the flows_build path; visible = \
             {visible:?}"
        );
    }
    // Authoring / read tools — including the born-disabled `create_workflow`
    // and `duplicate_flow` — stay reachable so the builder turn still works
    // headlessly under the CLI origin (no fail-close).
    for keep in [
        "propose_workflow",
        "revise_workflow",
        "save_workflow",
        "dry_run_workflow",
        "list_flows",
        "create_workflow",
        "duplicate_flow",
    ] {
        assert!(
            visible.contains(keep),
            "authoring tool `{keep}` must remain visible after restriction; visible = {visible:?}"
        );
    }
}

/// Pins the exact contents of both `flows_build` hide-lists so a future edit
/// can't silently narrow/widen either belt without a test catching it
/// (PR3: flows-copilot-live-run-approval).
#[test]
fn flows_build_hide_lists_have_the_expected_contents() {
    assert_eq!(
        FLOWS_BUILD_COPILOT_HIDDEN_TOOLS,
        ["run_workflow", "cancel_flow_run"],
        "the streaming (copilot) hide-list must hide the legacy `run_workflow` AND \
         `cancel_flow_run`. The T-M3 fix DID give the latter `external_effect() == true` \
         plus a run-ownership guard, so it would now park safely here — but unhiding it \
         is a capability expansion (letting an authoring turn tear down a user-started \
         run), not a security fix, and that product decision has not been taken. Only \
         `run_flow`/`resume_flow_run` stay visible, gated by the WebChat approval surface"
    );
    for tool in [
        "run_workflow",
        "run_flow",
        "resume_flow_run",
        "cancel_flow_run",
    ] {
        assert!(
            FLOWS_BUILD_HIDDEN_TOOLS.contains(&tool),
            "the headless hide-list must still contain `{tool}` (existing #4593/#4881 \
             contract) — {FLOWS_BUILD_HIDDEN_TOOLS:?}"
        );
    }
}

/// Streaming (copilot) path: `restrict_builder_toolset_for_copilot` leaves
/// `run_flow` / `resume_flow_run` visible on the builder's belt — they're gated
/// by the WebChat approval surface, not hidden — while hiding the unrelated
/// legacy `run_workflow` AND `cancel_flow_run`, and keeping every authoring
/// tool reachable (PR3: flows-copilot-live-run-approval). The T-M3 fix made
/// `cancel_flow_run` safe to unhide (external_effect + run-ownership guard),
/// but doing so would newly let an authoring turn tear down a user-started
/// run — a product decision, deliberately not taken here.
#[tokio::test]
async fn flows_build_copilot_toolset_unhides_the_live_run_tools() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    // Building an agent constructs a memory client, which needs the host seams
    // wired. `Once`-guarded, so this is free when another test got there first.
    crate::openhuman::agent::harness::AgentDefinitionRegistry::init_global(&config.workspace_dir)
        .expect("agent registry init");
    let mut agent =
        crate::openhuman::agent::Agent::from_config_for_agent(&config, "workflow_builder")
            .expect("build workflow_builder agent");
    agent.set_agent_definition_name("workflow_builder".to_string());

    restrict_builder_toolset_for_copilot(&mut agent);

    let visible = agent.visible_tool_names_for_test();
    for still_reachable in ["run_flow", "resume_flow_run"] {
        assert!(
            visible.contains(still_reachable),
            "`{still_reachable}` must stay reachable on the streaming copilot path — it \
             is gated behind the WebChat approval surface, not hidden; visible = {visible:?}"
        );
    }
    for hidden in ["run_workflow", "cancel_flow_run"] {
        assert!(
            !visible.contains(hidden),
            "`{hidden}` must stay hidden on the copilot path (unrelated legacy runner / \
             a cancel that is now safe to unhide but deliberately still gated behind a \
             product decision); visible = {visible:?}"
        );
    }
    for keep in [
        "propose_workflow",
        "revise_workflow",
        "save_workflow",
        "dry_run_workflow",
        "list_flows",
        "create_workflow",
        "duplicate_flow",
    ] {
        assert!(
            visible.contains(keep),
            "authoring tool `{keep}` must remain visible on the copilot path; visible = \
             {visible:?}"
        );
    }
}

/// Regression for issue #4868 (systemic fix, superseding the old B31
/// per-caller `apply_builder_iteration_cap` override): `flows_build` must get
/// an agent carrying the `workflow_builder` `AgentDefinition`'s
/// `effective_max_iterations()` (50, from `agent.toml`'s
/// `iteration_policy = "extended"`), not the global `Config::default()`
/// `agent.max_tool_iterations` (10) — and it must get this from the shared
/// resolution point in `build_session_agent_inner`, with **no** per-caller
/// override needed (that function was deleted as part of #4868).
#[tokio::test]
async fn flows_build_applies_the_builder_definitions_effective_iteration_cap() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    // Precondition: the global default really is lower than the definition's
    // effective cap, otherwise this test can't distinguish the two.
    assert_eq!(config.agent.max_tool_iterations, 10);

    // Building an agent constructs a memory client, which needs the host seams
    // wired. `Once`-guarded, so this is free when another test got there first.
    crate::openhuman::agent::harness::AgentDefinitionRegistry::init_global(&config.workspace_dir)
        .expect("agent registry init");
    let def = crate::openhuman::agent::harness::AgentDefinitionRegistry::global()
        .expect("registry initialised")
        .get("workflow_builder")
        .expect("workflow_builder definition registered")
        .clone();
    let expected = def.effective_max_iterations();
    assert_eq!(
        expected, 50,
        "workflow_builder's agent.toml is expected to declare iteration_policy = \"extended\", \
         yielding an effective cap of EXTENDED_MAX_TOOL_ITERATIONS (50)"
    );

    // End-to-end: the agent actually built for this path carries the
    // definition's cap straight off the unmodified `config` — the session
    // builder resolves it internally now, no `flows_build`-side override.
    let agent = crate::openhuman::agent::Agent::from_config_for_agent(&config, "workflow_builder")
        .expect("build workflow_builder agent");
    assert_eq!(agent.agent_config().max_tool_iterations, expected);
    assert_ne!(
        agent.agent_config().max_tool_iterations,
        config.agent.max_tool_iterations,
        "sanity: the resolved cap must actually differ from the unmodified global config"
    );
}
