use super::*;
use crate::openhuman::config::Config;
use tempfile::TempDir;
use tinyflows::model::{Node, NodeKind, WorkflowGraph};

fn test_config(tmp: &TempDir) -> Config {
    let config = Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    config
}

fn trigger_graph() -> WorkflowGraph {
    WorkflowGraph {
        nodes: vec![Node {
            id: "t".to_string(),
            kind: NodeKind::Trigger,
            type_version: 1,
            name: "Trigger".to_string(),
            config: serde_json::Value::Null,
            ports: Vec::new(),
            position: None,
        }],
        ..Default::default()
    }
}

#[test]
fn create_get_list_delete_roundtrip() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let flow = create_flow(&config, "demo".to_string(), trigger_graph(), false, true).unwrap();
    assert_eq!(flow.name, "demo");
    assert!(flow.enabled);

    let fetched = get_flow(&config, &flow.id).unwrap().expect("flow present");
    assert_eq!(fetched.id, flow.id);
    assert_eq!(fetched.graph, flow.graph);

    let listed = list_flows(&config).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, flow.id);

    remove_flow(&config, &flow.id).unwrap();
    assert!(get_flow(&config, &flow.id).unwrap().is_none());
    assert!(list_flows(&config).unwrap().is_empty());
}

#[test]
fn get_flow_returns_none_for_unknown_id() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    assert!(get_flow(&config, "missing").unwrap().is_none());
}

#[test]
fn remove_flow_errors_when_not_found() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let err = remove_flow(&config, "missing").unwrap_err();
    assert!(err.to_string().contains("not found"));
}

#[test]
fn set_enabled_toggles_and_persists() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow = create_flow(&config, "demo".to_string(), trigger_graph(), false, true).unwrap();
    assert!(flow.enabled);

    let disabled = set_enabled(&config, &flow.id, false).unwrap();
    assert!(!disabled.enabled);

    let reloaded = get_flow(&config, &flow.id).unwrap().unwrap();
    assert!(!reloaded.enabled);

    let enabled = set_enabled(&config, &flow.id, true).unwrap();
    assert!(enabled.enabled);
}

#[test]
fn update_flow_graph_bumps_updated_at_and_preserves_created_at() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow = create_flow(&config, "demo".to_string(), trigger_graph(), false, true).unwrap();

    let mut new_graph = trigger_graph();
    new_graph.name = "renamed-graph".to_string();
    let updated = update_flow_graph(
        &config,
        &flow.id,
        "renamed".to_string(),
        new_graph,
        false,
        None,
        None,
    )
    .unwrap();

    assert_eq!(updated.name, "renamed");
    assert_eq!(updated.created_at, flow.created_at);
    assert_eq!(updated.graph.name, "renamed-graph");
}

/// `enabled_override: None` must leave the persisted `enabled` column
/// exactly as it was — `update_flow_graph` re-reads the current row and
/// falls back to `current.enabled`, not to whatever the caller might have
/// observed earlier.
#[test]
fn update_flow_graph_with_none_override_preserves_current_enabled_column() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow = create_flow(&config, "demo".to_string(), trigger_graph(), false, true).unwrap();
    assert!(flow.enabled, "flow created enabled");

    let updated = update_flow_graph(
        &config,
        &flow.id,
        flow.name.clone(),
        trigger_graph(),
        false,
        None, // enabled_override
        None,
    )
    .unwrap();

    assert!(
        updated.enabled,
        "a None override must preserve the row's current enabled state"
    );
    let reloaded = get_flow(&config, &flow.id).unwrap().unwrap();
    assert!(reloaded.enabled);
}

/// `enabled_override: Some(false)` must force-persist `enabled=false`
/// regardless of what the row's `enabled` column currently holds — this is
/// the mechanism `flows_update`'s B29 Rule 1 analogue relies on to disarm a
/// manual→automatic trigger transition in the same guarded write.
#[test]
fn update_flow_graph_with_some_false_override_forces_disabled() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow = create_flow(&config, "demo".to_string(), trigger_graph(), false, true).unwrap();
    assert!(flow.enabled, "flow created enabled");

    let updated = update_flow_graph(
        &config,
        &flow.id,
        flow.name.clone(),
        trigger_graph(),
        false,
        Some(false), // enabled_override
        None,
    )
    .unwrap();

    assert!(
        !updated.enabled,
        "a Some(false) override must force enabled=false even though the row was enabled"
    );
    let reloaded = get_flow(&config, &flow.id).unwrap().unwrap();
    assert!(!reloaded.enabled);
}

/// Regression for the silent live-arming race Codex flagged on this PR:
/// `flows_update` (ops.rs) makes its manual→automatic disarm decision from
/// an *outer* `existing` read taken before `update_flow_graph`'s own guarded
/// UPDATE re-reads the row. If a concurrent `flows_set_enabled(id, true)`
/// landed in that gap — which bumps `updated_at`, so it would NOT trip the
/// optimistic-concurrency conflict — the outer read would be stale while the
/// row is actually enabled by write time. This proves the mechanism the fix
/// relies on to close that race: an `enabled_override` of `Some(false)`
/// (what `flows_update` now passes unconditionally on a manual→automatic
/// transition, never gated on the stale outer read) always wins over
/// whatever the row's `enabled` column was concurrently flipped to,
/// simulated here by flipping it with `set_enabled` between the two calls.
#[test]
fn update_flow_graph_override_wins_over_concurrently_enabled_row() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow = create_flow(&config, "demo".to_string(), trigger_graph(), false, false).unwrap();
    assert!(!flow.enabled, "flow created disabled");

    // Simulates a concurrent `flows_set_enabled(id, true)` racing in after
    // `flows_update`'s outer `existing` read observed `enabled: false`, but
    // before its guarded `update_flow_graph` write below.
    let raced = set_enabled(&config, &flow.id, true).unwrap();
    assert!(raced.enabled);

    let updated = update_flow_graph(
        &config,
        &flow.id,
        flow.name.clone(),
        trigger_graph(),
        false,
        Some(false), // the unconditional disarm override
        None,
    )
    .unwrap();

    assert!(
        !updated.enabled,
        "the disarm override must win over a concurrently-enabled row, not the reverse"
    );
    let reloaded = get_flow(&config, &flow.id).unwrap().unwrap();
    assert!(!reloaded.enabled);
}

#[test]
fn record_run_sets_last_run_fields() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow = create_flow(&config, "demo".to_string(), trigger_graph(), false, true).unwrap();
    assert!(flow.last_run_at.is_none());

    record_run(&config, &flow.id, "completed").unwrap();
    let reloaded = get_flow(&config, &flow.id).unwrap().unwrap();
    assert!(reloaded.last_run_at.is_some());
    assert_eq!(reloaded.last_status.as_deref(), Some("completed"));
}

#[test]
fn stored_graph_older_than_current_schema_is_migrated_on_read() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    // Insert a raw, versionless graph row directly (bypassing create_flow's
    // typed path) to simulate a definition persisted by an older crate build.
    let legacy_graph_json = serde_json::json!({
        "name": "legacy",
        "nodes": [{ "id": "t", "kind": "trigger", "name": "Trigger" }],
        "edges": []
    })
    .to_string();

    with_connection(&config, |conn| {
        conn.execute(
            "INSERT INTO flow_definitions
                (id, name, graph_json, enabled, created_at, updated_at, last_run_at, last_status)
             VALUES ('legacy-1', 'legacy', ?1, 1, '2020-01-01T00:00:00Z', '2020-01-01T00:00:00Z', NULL, NULL)",
            rusqlite::params![legacy_graph_json],
        )?;
        Ok(())
    })
    .unwrap();

    let loaded = get_flow(&config, "legacy-1").unwrap().expect("row present");
    assert_eq!(
        loaded.graph.schema_version,
        tinyflows::model::CURRENT_SCHEMA_VERSION
    );
    assert_eq!(loaded.graph.nodes.len(), 1);
}

#[test]
fn kv_get_set_round_trips_and_is_namespace_scoped() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    assert!(kv_get(&config, "ns1", "k").unwrap().is_none());

    kv_set(&config, "ns1", "k", &serde_json::json!({"v": 1})).unwrap();
    assert_eq!(
        kv_get(&config, "ns1", "k").unwrap(),
        Some(serde_json::json!({"v": 1}))
    );

    // A different namespace does not see ns1's value.
    assert!(kv_get(&config, "ns2", "k").unwrap().is_none());

    // Overwrite.
    kv_set(&config, "ns1", "k", &serde_json::json!(2)).unwrap();
    assert_eq!(
        kv_get(&config, "ns1", "k").unwrap(),
        Some(serde_json::json!(2))
    );
}

// ── require_approval ─────────────────────────────────────────────────────

#[test]
fn create_flow_persists_require_approval() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let flow = create_flow(&config, "demo".to_string(), trigger_graph(), true, true).unwrap();
    assert!(flow.require_approval);

    let reloaded = get_flow(&config, &flow.id).unwrap().unwrap();
    assert!(reloaded.require_approval);
}

#[test]
fn update_flow_graph_can_change_require_approval() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow = create_flow(&config, "demo".to_string(), trigger_graph(), false, true).unwrap();
    assert!(!flow.require_approval);

    let updated = update_flow_graph(
        &config,
        &flow.id,
        flow.name.clone(),
        trigger_graph(),
        true,
        None,
        None,
    )
    .unwrap();
    assert!(updated.require_approval);

    let reloaded = get_flow(&config, &flow.id).unwrap().unwrap();
    assert!(reloaded.require_approval);
}

#[test]
fn legacy_flow_definitions_row_without_require_approval_column_defaults_false() {
    // A row inserted before the `require_approval` column existed (the
    // `add_column_if_missing` ALTER runs on every `with_connection` call, so
    // this simulates a workspace opened once on an older build).
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let legacy_graph_json = serde_json::to_string(&trigger_graph()).unwrap();
    with_connection(&config, |conn| {
        conn.execute(
            "INSERT INTO flow_definitions
                (id, name, graph_json, enabled, created_at, updated_at, last_run_at, last_status)
             VALUES ('legacy-2', 'legacy', ?1, 1, '2020-01-01T00:00:00Z', '2020-01-01T00:00:00Z', NULL, NULL)",
            rusqlite::params![legacy_graph_json],
        )?;
        Ok(())
    })
    .unwrap();

    let loaded = get_flow(&config, "legacy-2").unwrap().expect("row present");
    assert!(!loaded.require_approval);
}

// ── list_enabled_flows ────────────────────────────────────────────────────

#[test]
fn list_enabled_flows_excludes_disabled() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let enabled_flow =
        create_flow(&config, "enabled".to_string(), trigger_graph(), false, true).unwrap();
    let disabled_flow = create_flow(
        &config,
        "disabled".to_string(),
        trigger_graph(),
        false,
        true,
    )
    .unwrap();
    set_enabled(&config, &disabled_flow.id, false).unwrap();

    let enabled = list_enabled_flows(&config).unwrap();
    assert_eq!(enabled.len(), 1);
    assert_eq!(enabled[0].id, enabled_flow.id);
}

// ── flow_runs CRUD ────────────────────────────────────────────────────────

#[test]
fn flow_run_insert_finish_get_round_trip() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow = create_flow(&config, "demo".to_string(), trigger_graph(), false, true).unwrap();

    let thread_id = format!("flow:{}:run-1", flow.id);
    insert_flow_run(
        &config,
        &thread_id,
        &flow.id,
        &thread_id,
        "2026-01-01T00:00:00Z",
    )
    .unwrap();

    let running = get_flow_run(&config, &thread_id)
        .unwrap()
        .expect("row present");
    assert_eq!(running.status, "running");
    assert!(running.finished_at.is_none());
    assert!(running.steps.is_empty());

    let steps = vec![FlowRunStep {
        node_id: "t".to_string(),
        output: serde_json::json!([{"json": {"x": 1}}]),
        port: None,
        ..Default::default()
    }];
    finish_flow_run(
        &config,
        &thread_id,
        "completed",
        "2026-01-01T00:00:01Z",
        &steps,
        &[],
        None,
    )
    .unwrap();

    let finished = get_flow_run(&config, &thread_id)
        .unwrap()
        .expect("row present");
    assert_eq!(finished.status, "completed");
    assert_eq!(
        finished.finished_at.as_deref(),
        Some("2026-01-01T00:00:01Z")
    );
    assert_eq!(finished.steps.len(), 1);
    assert_eq!(finished.steps[0].node_id, "t");
    assert!(finished.pending_approvals.is_empty());
    assert!(finished.error.is_none());
}

#[test]
fn finish_flow_run_records_error_on_failure() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow = create_flow(&config, "demo".to_string(), trigger_graph(), false, true).unwrap();
    let thread_id = format!("flow:{}:run-2", flow.id);
    insert_flow_run(
        &config,
        &thread_id,
        &flow.id,
        &thread_id,
        "2026-01-01T00:00:00Z",
    )
    .unwrap();

    finish_flow_run(
        &config,
        &thread_id,
        "failed",
        "2026-01-01T00:00:01Z",
        &[],
        &[],
        Some("boom"),
    )
    .unwrap();

    let finished = get_flow_run(&config, &thread_id).unwrap().unwrap();
    assert_eq!(finished.status, "failed");
    assert_eq!(finished.error.as_deref(), Some("boom"));
}

#[test]
fn get_flow_run_returns_none_for_unknown_id() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    assert!(get_flow_run(&config, "missing").unwrap().is_none());
}

#[test]
fn list_flow_runs_orders_newest_first_and_is_scoped_to_flow() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow_a = create_flow(&config, "a".to_string(), trigger_graph(), false, true).unwrap();
    let flow_b = create_flow(&config, "b".to_string(), trigger_graph(), false, true).unwrap();

    insert_flow_run(
        &config,
        "run-a1",
        &flow_a.id,
        "run-a1",
        "2026-01-01T00:00:00Z",
    )
    .unwrap();
    insert_flow_run(
        &config,
        "run-a2",
        &flow_a.id,
        "run-a2",
        "2026-01-02T00:00:00Z",
    )
    .unwrap();
    insert_flow_run(
        &config,
        "run-b1",
        &flow_b.id,
        "run-b1",
        "2026-01-01T00:00:00Z",
    )
    .unwrap();

    let runs_a = list_flow_runs(&config, &flow_a.id, 10).unwrap();
    assert_eq!(runs_a.len(), 2);
    assert_eq!(runs_a[0].id, "run-a2", "newest run must come first");
    assert_eq!(runs_a[1].id, "run-a1");

    let runs_b = list_flow_runs(&config, &flow_b.id, 10).unwrap();
    assert_eq!(runs_b.len(), 1);
    assert_eq!(runs_b[0].id, "run-b1");
}

// ── insert_duplicate_flow ─────────────────────────────────────────────────

#[test]
fn insert_duplicate_flow_makes_a_disabled_copy_with_new_id_and_same_graph() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    // Enabled source with require_approval + a distinctive graph name.
    let mut graph = trigger_graph();
    graph.name = "original-graph".to_string();
    let source = create_flow(&config, "My Flow".to_string(), graph, true, true).unwrap();
    assert!(source.enabled);
    record_run(&config, &source.id, "completed").unwrap();
    let source = get_flow(&config, &source.id).unwrap().unwrap();
    assert!(source.last_status.is_some());

    let copy = insert_duplicate_flow(&config, &source, "My Flow (copy)".to_string()).unwrap();

    // New id, suffixed name, DISABLED, run history reset.
    assert_ne!(copy.id, source.id);
    assert_eq!(copy.name, "My Flow (copy)");
    assert!(
        !copy.enabled,
        "duplicate must be disabled so it never fires"
    );
    assert!(copy.last_run_at.is_none());
    assert!(copy.last_status.is_none());
    // Same graph + require_approval carried over.
    assert_eq!(copy.graph, source.graph);
    assert_eq!(copy.graph.name, "original-graph");
    assert!(copy.require_approval);

    // Persisted and independent — both rows exist.
    let reloaded = get_flow(&config, &copy.id).unwrap().unwrap();
    assert!(!reloaded.enabled);
    assert_eq!(reloaded.graph, source.graph);
    assert_eq!(list_flows(&config).unwrap().len(), 2);
}

// ── prune_flow_runs ───────────────────────────────────────────────────────

fn seed_run(config: &Config, flow_id: &str, id: &str, day: u32, status: &str) {
    let started = format!("2026-01-{day:02}T00:00:00Z");
    insert_flow_run(config, id, flow_id, id, &started).unwrap();
    if status != "running" {
        finish_flow_run(
            config,
            id,
            status,
            &format!("2026-01-{day:02}T00:00:05Z"),
            &[],
            &[],
            None,
        )
        .unwrap();
    }
}

#[test]
fn prune_flow_runs_keeps_newest_n_terminal_runs() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow = create_flow(&config, "demo".to_string(), trigger_graph(), false, true).unwrap();

    // 5 completed runs on ascending days.
    for i in 1..=5 {
        seed_run(&config, &flow.id, &format!("run-{i}"), i, "completed");
    }

    let deleted = prune_flow_runs(&config, &flow.id, 2).unwrap();
    assert_eq!(deleted, 3, "5 terminal runs, keep 2 => 3 pruned");

    let remaining = list_flow_runs(&config, &flow.id, 100).unwrap();
    let ids: Vec<_> = remaining.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, vec!["run-5", "run-4"], "newest two survive");
}

#[test]
fn prune_flow_runs_never_removes_pending_approval_run() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow = create_flow(&config, "demo".to_string(), trigger_graph(), false, true).unwrap();

    // An OLD parked pending_approval run (day 1) plus newer completed runs.
    seed_run(&config, &flow.id, "parked", 1, "pending_approval");
    for i in 2..=5 {
        seed_run(&config, &flow.id, &format!("run-{i}"), i, "completed");
    }

    // keep=1 would normally leave only the newest run; the parked one must
    // still survive despite being the oldest and outside the newest-1 window.
    let deleted = prune_flow_runs(&config, &flow.id, 1).unwrap();
    let remaining = list_flow_runs(&config, &flow.id, 100).unwrap();
    let ids: std::collections::HashSet<_> = remaining.iter().map(|r| r.id.as_str()).collect();
    assert!(
        ids.contains("parked"),
        "a pending_approval run must never be pruned out from under a resume"
    );
    assert!(ids.contains("run-5"), "newest terminal run kept");
    // Only terminal runs 2..4 were eligible; 5 kept by window => 3 deleted.
    assert_eq!(deleted, 3);
}

#[test]
fn prune_flow_runs_leaves_running_rows_alone() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow = create_flow(&config, "demo".to_string(), trigger_graph(), false, true).unwrap();

    seed_run(&config, &flow.id, "live", 1, "running");
    for i in 2..=4 {
        seed_run(&config, &flow.id, &format!("run-{i}"), i, "completed");
    }

    prune_flow_runs(&config, &flow.id, 1).unwrap();
    let remaining = list_flow_runs(&config, &flow.id, 100).unwrap();
    let ids: std::collections::HashSet<_> = remaining.iter().map(|r| r.id.as_str()).collect();
    assert!(ids.contains("live"), "a running run is never pruned");
}

#[test]
fn insert_flow_run_auto_prunes_beyond_retention_cap() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow = create_flow(&config, "demo".to_string(), trigger_graph(), false, true).unwrap();

    // Seed exactly MAX_FLOW_RUNS_PER_FLOW completed runs.
    let cap = MAX_FLOW_RUNS_PER_FLOW;
    for i in 0..cap {
        let id = format!("run-{i:04}");
        insert_flow_run(
            &config,
            &id,
            &flow.id,
            &id,
            &format!("2026-01-01T00:00:{i:02}Z"),
        )
        .unwrap();
        finish_flow_run(
            &config,
            &id,
            "completed",
            "2026-01-01T00:01:00Z",
            &[],
            &[],
            None,
        )
        .unwrap();
    }
    assert_eq!(
        list_flow_runs(&config, &flow.id, cap * 2).unwrap().len(),
        cap
    );

    // One more insert should trigger the retention prune, keeping <= cap.
    let extra = "run-extra";
    insert_flow_run(&config, extra, &flow.id, extra, "2026-01-02T00:00:00Z").unwrap();
    let count = list_flow_runs(&config, &flow.id, cap * 2).unwrap().len();
    assert!(
        count <= cap,
        "auto-prune should keep run count within cap ({count} > {cap})"
    );
}

#[test]
fn list_flow_runs_respects_limit() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow = create_flow(&config, "demo".to_string(), trigger_graph(), false, true).unwrap();

    for i in 0..3 {
        let id = format!("run-{i}");
        insert_flow_run(
            &config,
            &id,
            &flow.id,
            &id,
            &format!("2026-01-0{}T00:00:00Z", i + 1),
        )
        .unwrap();
    }

    let limited = list_flow_runs(&config, &flow.id, 2).unwrap();
    assert_eq!(limited.len(), 2);
}

// ── flow_suggestions ─────────────────────────────────────────────────────────

fn sample_suggestion(id: &str, title: &str) -> FlowSuggestion {
    FlowSuggestion {
        id: id.to_string(),
        title: title.to_string(),
        one_liner: "does a useful thing".to_string(),
        rationale: "grounded in your data".to_string(),
        trigger_hint: Some("schedule".to_string()),
        steps_outline: vec!["step one".to_string(), "step two".to_string()],
        suggested_connections: vec!["composio:gmail:conn_1".to_string()],
        suggested_slugs: vec!["GMAIL_SEND_EMAIL".to_string()],
        build_prompt: "Build a workflow that…".to_string(),
        confidence: 0.7,
        status: SuggestionStatus::New,
        created_at: "2026-07-05T00:00:00Z".to_string(),
        source_run_id: Some("run-1".to_string()),
    }
}

#[test]
fn suggestions_upsert_list_roundtrip() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let written = upsert_suggestions(
        &config,
        &[
            sample_suggestion("s1", "Alpha"),
            sample_suggestion("s2", "Beta"),
        ],
    )
    .unwrap();
    assert_eq!(written, 2);

    let all = list_suggestions(&config, Some(SuggestionStatus::New), 50).unwrap();
    assert_eq!(all.len(), 2);
    // Round-trips the JSON-encoded vec columns.
    let alpha = all.iter().find(|s| s.id == "s1").unwrap();
    assert_eq!(alpha.steps_outline.len(), 2);
    assert_eq!(alpha.suggested_connections, vec!["composio:gmail:conn_1"]);
    assert_eq!(alpha.suggested_slugs, vec!["GMAIL_SEND_EMAIL"]);
    assert_eq!(alpha.trigger_hint.as_deref(), Some("schedule"));
}

#[test]
fn upsert_suggestions_preserves_user_status_on_rerun() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    upsert_suggestions(&config, &[sample_suggestion("s1", "Alpha")]).unwrap();
    // User dismisses it.
    assert!(set_suggestion_status(&config, "s1", SuggestionStatus::Dismissed).unwrap());

    // A later discovery run re-proposes the identical idea (same id) with a
    // refreshed pitch — the dismissal must survive.
    let mut refreshed = sample_suggestion("s1", "Alpha (refined)");
    refreshed.status = SuggestionStatus::New; // agent always emits `New`
    upsert_suggestions(&config, &[refreshed]).unwrap();

    let dismissed = list_suggestions(&config, Some(SuggestionStatus::Dismissed), 50).unwrap();
    assert_eq!(dismissed.len(), 1);
    assert_eq!(dismissed[0].title, "Alpha (refined)"); // pitch fields refreshed
                                                       // …but it is NOT back in the active `New` list.
    let active = list_suggestions(&config, Some(SuggestionStatus::New), 50).unwrap();
    assert!(active.is_empty());
}

#[test]
fn set_suggestion_status_returns_false_for_unknown_id() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    assert!(!set_suggestion_status(&config, "missing", SuggestionStatus::Built).unwrap());
}

#[test]
fn list_suggestions_without_status_returns_all() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    upsert_suggestions(&config, &[sample_suggestion("s1", "Alpha")]).unwrap();
    set_suggestion_status(&config, "s1", SuggestionStatus::Built).unwrap();
    // Filtered to `New` → empty; unfiltered → present.
    assert!(list_suggestions(&config, Some(SuggestionStatus::New), 50)
        .unwrap()
        .is_empty());
    assert_eq!(list_suggestions(&config, None, 50).unwrap().len(), 1);
}

#[test]
fn upsert_suggestions_empty_is_noop() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    assert_eq!(upsert_suggestions(&config, &[]).unwrap(), 0);
}
