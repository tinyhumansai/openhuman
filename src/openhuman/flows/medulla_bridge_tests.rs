use super::*;
use crate::openhuman::config::Config;
use serde_json::json;
use tempfile::TempDir;
use tinyflows::model::{Node, NodeKind};

/// A config pointed at a throwaway workspace, so every test addresses its own
/// SQLite store instead of the developer's real one.
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

fn node(id: &str, kind: NodeKind, name: &str, config: Value) -> Node {
    Node {
        id: id.to_string(),
        kind,
        type_version: 1,
        name: name.to_string(),
        config,
        ports: Vec::new(),
        position: None,
    }
}

/// A minimal manual-trigger graph with one named step.
fn graph_with_step(step_name: &str) -> WorkflowGraph {
    WorkflowGraph {
        nodes: vec![
            node(
                "t",
                NodeKind::Trigger,
                "Trigger",
                json!({ "trigger_kind": "manual" }),
            ),
            node(
                "s",
                NodeKind::Code,
                step_name,
                json!({ "code": "return {}" }),
            ),
        ],
        ..Default::default()
    }
}

fn flow(id: &str, name: &str, graph: WorkflowGraph) -> Flow {
    Flow {
        id: id.to_string(),
        name: name.to_string(),
        enabled: true,
        graph,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
        last_run_at: None,
        last_status: None,
        require_approval: false,
    }
}

// ── advert projection ────────────────────────────────────────────────────────

#[test]
fn describe_flow_counts_every_node_and_names_the_trigger() {
    let descriptor = describe_flow(&flow("f1", "Deploy", graph_with_step("Ship it")));
    assert_eq!(descriptor.id, "f1");
    assert_eq!(descriptor.name, "Deploy");
    // Trigger + step: the whole graph, not just the executable half.
    assert_eq!(descriptor.node_count, 2);
    assert_eq!(descriptor.enabled, Some(true));
    assert_eq!(descriptor.trigger_kind.as_deref(), Some("manual"));
    assert_eq!(descriptor.description, "on manual → Ship it");
}

#[test]
fn describe_flow_projects_declared_inputs_onto_the_advert() {
    // The orchestrator picks a workflow off the advert, so the advert has to
    // carry what running it requires — otherwise it can only find out by
    // fetching the graph, or by trying and failing.
    let mut graph = graph_with_step("Ship it");
    graph.inputs = vec![
        tinyflows::model::WorkflowInput::new("repo", tinyflows::model::InputType::String)
            .required()
            .with_description("Repo to deploy"),
        tinyflows::model::WorkflowInput::new("depth", tinyflows::model::InputType::Number)
            .with_default(serde_json::json!(3)),
    ];

    let descriptor = describe_flow(&flow("f1", "Deploy", graph));
    assert_eq!(descriptor.inputs.len(), 2);

    let repo = &descriptor.inputs[0];
    assert_eq!(repo.name, "repo");
    assert_eq!(repo.ty, "string");
    assert!(repo.required);
    assert_eq!(repo.description, "Repo to deploy");
    assert_eq!(repo.default, None);

    let depth = &descriptor.inputs[1];
    assert_eq!(depth.ty, "number");
    assert!(!depth.required);
    assert_eq!(depth.default, Some(serde_json::json!(3)));
    assert!(
        depth.description.is_empty(),
        "an undescribed input sends no description rather than a null"
    );
}

#[test]
fn a_workflow_declaring_no_inputs_advertises_none() {
    let descriptor = describe_flow(&flow("f1", "Deploy", graph_with_step("Ship it")));
    assert!(descriptor.inputs.is_empty());
    let wire = serde_json::to_value(&descriptor).unwrap();
    assert!(wire.get("inputs").is_none());
}

#[test]
fn a_blank_name_stays_absent_on_the_wire_rather_than_empty() {
    let descriptor = describe_flow(&flow("f1", "   ", graph_with_step("Step")));
    assert!(descriptor.name.is_empty());
    let wire = serde_json::to_value(&descriptor).unwrap();
    // The contract is an ABSENT key, not `""` — both sides treat it as optional.
    assert!(wire.get("name").is_none(), "wire = {wire}");
    assert_eq!(wire["nodeCount"], 2);
}

#[test]
fn a_triggerless_graph_claims_no_trigger_kind() {
    let graph = WorkflowGraph {
        nodes: vec![node("s", NodeKind::Code, "Step", json!({}))],
        ..Default::default()
    };
    let descriptor = describe_flow(&flow("f1", "Orphan", graph));
    assert_eq!(descriptor.trigger_kind, None);
    assert_eq!(descriptor.description, "no trigger → Step");
}

#[test]
fn a_long_graph_description_is_truncated() {
    let nodes = (0..80)
        .map(|i| {
            node(
                &format!("s{i}"),
                NodeKind::Code,
                "A fairly long step name",
                json!({}),
            )
        })
        .collect();
    let descriptor = describe_flow(&flow(
        "f1",
        "Big",
        WorkflowGraph {
            nodes,
            ..Default::default()
        },
    ));
    assert_eq!(
        descriptor.description.chars().count(),
        MAX_DESCRIPTION_CHARS + 1
    );
    assert!(descriptor.description.ends_with('…'));
}

// ── node kinds ───────────────────────────────────────────────────────────────

#[test]
fn node_kinds_renders_the_catalog_in_the_ports_camel_case_shape() {
    let bridge = FlowsWorkflowBridge::new();
    let all = bridge.node_kinds(None).unwrap();
    let kinds = all["kinds"].as_array().unwrap();
    assert_eq!(kinds.len(), NODE_KINDS.len());
    // `configFields`, not `config_fields`: the backend spreads this object onto
    // the port type, so a snake_case key would miss the field entirely.
    assert!(kinds[0].get("configFields").is_some());
    assert!(kinds[0].get("config_fields").is_none());

    let one = bridge.node_kinds(Some("agent")).unwrap();
    assert_eq!(one["kinds"].as_array().unwrap().len(), 1);
    assert_eq!(one["kinds"][0]["kind"], "agent");
}

#[test]
fn an_unknown_node_kind_reports_the_real_ones() {
    let err = FlowsWorkflowBridge::new()
        .node_kinds(Some("teleport"))
        .unwrap_err();
    assert!(err.contains("teleport"), "{err}");
    assert!(err.contains("tool_call"), "{err}");
}

#[test]
fn a_blank_kind_filter_means_the_whole_catalog() {
    let bridge = FlowsWorkflowBridge::new();
    assert_eq!(
        bridge.node_kinds(Some("  ")).unwrap(),
        bridge.node_kinds(None).unwrap()
    );
}

// ── run projection ───────────────────────────────────────────────────────────

#[test]
fn run_json_emits_epoch_millis_and_omits_what_it_cannot_read() {
    let run = super::super::types::FlowRun {
        id: "r1".to_string(),
        flow_id: "f1".to_string(),
        thread_id: "r1".to_string(),
        status: "completed".to_string(),
        started_at: "2026-01-01T00:00:00Z".to_string(),
        finished_at: Some("not a timestamp".to_string()),
        steps: Vec::new(),
        pending_approvals: Vec::new(),
        error: None,
        graph_hash: None,
    };
    let value = run_json(&run);
    assert_eq!(value["id"], "r1");
    assert_eq!(value["workflowId"], "f1");
    assert_eq!(value["startedAt"], 1_767_225_600_000_i64);
    // An unreadable stamp is absent, never `0` — that would read as 1970.
    assert!(value.get("finishedAt").is_none(), "{value}");
    assert!(value.get("error").is_none(), "{value}");
    // The step-by-step payload belongs to the run inspector, not to this read.
    assert!(value.get("steps").is_none(), "{value}");
}

// ── the store reads, over a real SQLite store ────────────────────────────────

/// Drive a synchronous bridge read the way the transport does — from a blocking
/// thread — since that is the only context its `block_on` is legal in.
async fn off_runtime<T, F>(read: F) -> T
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    tokio::task::spawn_blocking(read).await.unwrap()
}

#[tokio::test]
async fn list_and_get_answer_out_of_the_real_store() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let created = ops::flows_create(
        &config,
        "Deploy".to_string(),
        serde_json::to_value(graph_with_step("Ship it")).unwrap(),
        false,
    )
    .await
    .unwrap()
    .value;

    let bridge = Arc::new(FlowsWorkflowBridge::pinned(Arc::clone(&config)));

    let listed = off_runtime({
        let bridge = Arc::clone(&bridge);
        move || bridge.list()
    })
    .await;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, created.id);
    assert_eq!(listed[0].node_count, 2);

    let id = created.id.clone();
    let detail = off_runtime({
        let bridge = Arc::clone(&bridge);
        move || bridge.get(&id)
    })
    .await
    .unwrap();
    // The graph crosses whole and opaque — nothing above this parses it.
    assert_eq!(detail["id"], created.id);
    assert!(detail["graph"]["nodes"].as_array().unwrap().len() == 2);
}

#[tokio::test]
async fn an_unknown_id_is_a_readable_error_not_an_empty_answer() {
    let tmp = TempDir::new().unwrap();
    let bridge = Arc::new(FlowsWorkflowBridge::pinned(test_config(&tmp)));
    let err = off_runtime(move || bridge.get("nope")).await.unwrap_err();
    assert!(err.contains("nope"), "{err}");
}

#[tokio::test]
async fn runs_answers_with_an_empty_window_for_a_flow_that_never_ran() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let created = ops::flows_create(
        &config,
        "Deploy".to_string(),
        serde_json::to_value(graph_with_step("Ship it")).unwrap(),
        false,
    )
    .await
    .unwrap()
    .value;
    let bridge = Arc::new(FlowsWorkflowBridge::pinned(config));
    let runs = off_runtime(move || bridge.runs(&created.id)).await.unwrap();
    assert_eq!(runs["runs"], json!([]));
}

// ── persisting a proposal ────────────────────────────────────────────────────

#[tokio::test]
async fn a_created_workflow_always_requires_approval() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let proposal = json!({
        "name": "Remote build",
        "graph": serde_json::to_value(graph_with_step("Ship it")).unwrap(),
        // The proposal's own answer is deliberately ignored on this path.
        "require_approval": false,
    });

    let applied = apply_proposal(&config, None, &proposal, None)
        .await
        .unwrap();
    assert!(applied.created);
    let saved = ops::flows_get(&config, &applied.flow.id)
        .await
        .unwrap()
        .value;
    assert_eq!(saved.name, "Remote build");
    assert!(
        saved.require_approval,
        "a remotely authored workflow must still park its outbound actions"
    );
}

#[tokio::test]
async fn an_update_cannot_lower_the_approval_requirement() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let created = ops::flows_create(
        &config,
        "Deploy".to_string(),
        serde_json::to_value(graph_with_step("Ship it")).unwrap(),
        true,
    )
    .await
    .unwrap()
    .value;

    let proposal = json!({
        "name": "Deploy v2",
        "graph": serde_json::to_value(graph_with_step("Ship it twice")).unwrap(),
        "require_approval": false,
    });
    let applied = apply_proposal(
        &config,
        Some(&created.id),
        &proposal,
        Some(&created.updated_at),
    )
    .await
    .unwrap();
    assert!(!applied.created, "an update is not a creation");
    assert_eq!(applied.flow.id, created.id);

    let saved = ops::flows_get(&config, &created.id).await.unwrap().value;
    assert_eq!(saved.name, "Deploy v2");
    assert!(
        saved.require_approval,
        "the user's approval requirement survives a remote turn"
    );
}

fn schedule_graph(cron: &str) -> Value {
    json!({
        "name": "scheduled",
        "nodes": [{
            "id": "t",
            "kind": "trigger",
            "name": "Trigger",
            "config": { "trigger_kind": "schedule", "schedule": cron }
        }],
        "edges": []
    })
}

#[tokio::test]
async fn a_remote_automatic_revision_requires_explicit_rearming() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let created = ops::flows_create(
        &config,
        "Scheduled".to_string(),
        schedule_graph("0 9 * * *"),
        true,
    )
    .await
    .unwrap()
    .value;
    assert!(!created.enabled, "automatic flows are born disarmed");

    let armed = ops::flows_set_enabled(&config, &created.id, true)
        .await
        .unwrap()
        .value;
    assert!(armed.enabled, "precondition: the user explicitly armed it");

    let proposal = json!({
        "name": "Scheduled v2",
        "graph": schedule_graph("0 10 * * *"),
    });
    let applied = apply_proposal(&config, Some(&armed.id), &proposal, Some(&armed.updated_at))
        .await
        .unwrap();

    assert!(!applied.flow.enabled);
    let saved = ops::flows_get(&config, &armed.id).await.unwrap().value;
    assert!(!saved.enabled, "the revised schedule must stay disarmed");
}

#[tokio::test]
async fn a_proposal_without_a_graph_is_refused() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let err = apply_proposal(&config, None, &json!({ "name": "Nothing" }), None)
        .await
        .unwrap_err();
    assert!(err.contains("no graph"), "{err}");
}

#[tokio::test]
async fn an_update_refuses_to_overwrite_a_concurrent_edit() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let created = ops::flows_create(
        &config,
        "Deploy".to_string(),
        serde_json::to_value(graph_with_step("Ship it")).unwrap(),
        true,
    )
    .await
    .unwrap()
    .value;

    ops::flows_update(
        &config,
        &created.id,
        Some("User edit".to_string()),
        None,
        None,
        Some(created.updated_at.clone()),
    )
    .await
    .unwrap();

    let proposal = json!({
        "name": "Copilot edit",
        "graph": serde_json::to_value(graph_with_step("Ship it twice")).unwrap(),
    });
    let err = apply_proposal(
        &config,
        Some(&created.id),
        &proposal,
        Some(&created.updated_at),
    )
    .await
    .unwrap_err();
    assert!(err.contains("conflict"), "{err}");

    let saved = ops::flows_get(&config, &created.id).await.unwrap().value;
    assert_eq!(saved.name, "User edit");
}

// ── the accountability diff ──────────────────────────────────────────────────

#[test]
fn diff_reports_a_creation_with_its_size() {
    let after = vec![flow("f1", "Deploy", graph_with_step("Ship it"))];
    let changes = diff_flows(&[], &after);
    assert_eq!(changes.len(), 1);
    assert!(
        changes[0].contains("created workflow \"Deploy\" (f1)"),
        "{changes:?}"
    );
    assert!(changes[0].contains("2 node(s)"), "{changes:?}");
}

#[test]
fn diff_reports_a_rewrite_a_rename_and_a_deletion() {
    let before = vec![
        flow("f1", "Deploy", graph_with_step("Ship it")),
        flow("f2", "Doomed", graph_with_step("Step")),
    ];
    let mut renamed = flow("f1", "Deploy v2", graph_with_step("Ship it faster"));
    renamed.enabled = false;
    let changes = diff_flows(&before, &[renamed]);

    assert!(
        changes
            .iter()
            .any(|line| line.contains("renamed workflow f1")),
        "{changes:?}"
    );
    assert!(
        changes
            .iter()
            .any(|line| line.contains("rewrote the graph")),
        "{changes:?}"
    );
    assert!(
        changes
            .iter()
            .any(|line| line.starts_with("disabled workflow")),
        "{changes:?}"
    );
    assert!(
        changes
            .iter()
            .any(|line| line.contains("deleted workflow \"Doomed\" (f2)")),
        "{changes:?}"
    );
}

#[test]
fn a_turn_that_changed_nothing_claims_nothing() {
    let flows = vec![flow("f1", "Deploy", graph_with_step("Ship it"))];
    // The whole point: the agent's account of its turn cannot inflate this.
    assert!(diff_flows(&flows, &flows).is_empty());
}

#[test]
fn diff_reports_an_approval_requirement_change() {
    let before = vec![flow("f1", "Deploy", graph_with_step("Ship it"))];
    let mut after = before.clone();
    after[0].require_approval = true;
    let changes = diff_flows(&before, &after);
    assert_eq!(changes.len(), 1);
    assert!(changes[0].contains("now requires approval"), "{changes:?}");
}

#[test]
fn scoped_diff_excludes_concurrent_changes_to_other_workflows() {
    let before = vec![
        flow("target", "Deploy", graph_with_step("Ship it")),
        flow("other", "Unrelated", graph_with_step("Before")),
    ];
    let after = vec![
        flow("target", "Deploy v2", graph_with_step("Ship it twice")),
        flow("other", "User edit", graph_with_step("After")),
        flow("new", "Also user-created", graph_with_step("Step")),
    ];

    let changes = diff_workflow(&before, &after, "target");
    assert!(
        changes.iter().all(|line| !line.contains("other")
            && !line.contains("Unrelated")
            && !line.contains("User edit")
            && !line.contains("new")
            && !line.contains("Also user-created")),
        "{changes:?}"
    );
    assert!(
        changes
            .iter()
            .any(|line| line.contains("renamed workflow target")),
        "{changes:?}"
    );
    assert!(
        changes
            .iter()
            .any(|line| line.contains("rewrote the graph")),
        "{changes:?}"
    );
}

#[test]
fn medulla_copilot_hides_every_persistence_tool() {
    assert_eq!(
        MEDULLA_COPILOT_HIDDEN_TOOLS,
        [
            "save_workflow",
            "create_workflow",
            "duplicate_flow",
            "edit_workflow"
        ]
    );
}

#[test]
fn accountability_uses_the_committed_snapshot_not_a_later_user_edit() {
    let before = vec![flow("target", "Deploy", graph_with_step("Ship it"))];
    let committed = flow("target", "Copilot edit", graph_with_step("Ship it twice"));
    let later_user_edit = flow("target", "User edit", graph_with_step("Something else"));

    let changes = diff_workflow(&before, std::slice::from_ref(&committed), "target");
    assert!(
        changes.iter().any(|line| line.contains("Copilot edit")),
        "{changes:?}"
    );
    assert!(
        changes
            .iter()
            .all(|line| !line.contains(&later_user_edit.name)),
        "{changes:?}"
    );
}

// ── the copilot turn's request shaping ───────────────────────────────────────

#[test]
fn naming_no_workflow_briefs_a_create() {
    let request = builder_request("build a deploy flow", None, &[]).unwrap();
    assert_eq!(request.mode, BuildMode::Create);
    assert!(request.graph.is_none());
    assert!(request.flow_id.is_none());
    assert_eq!(request.instruction, "build a deploy flow");
}

#[test]
fn naming_a_workflow_briefs_a_revise_of_that_exact_graph() {
    let flows = vec![flow("f1", "Deploy", graph_with_step("Ship it"))];
    let request = builder_request("add a retry", Some("f1"), &flows).unwrap();
    assert_eq!(request.mode, BuildMode::Revise);
    assert_eq!(request.flow_id.as_deref(), Some("f1"));
    assert_eq!(
        request.graph.unwrap(),
        serde_json::to_value(&flows[0].graph).unwrap()
    );
}

#[test]
fn an_unknown_workflow_id_fails_rather_than_silently_creating_a_second_one() {
    let err = builder_request("add a retry", Some("ghost"), &[]).unwrap_err();
    assert!(err.contains("ghost"), "{err}");
}
