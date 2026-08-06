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

/// An automatic-trigger (`schedule`) graph — `trigger_is_automatic` returns
/// `true` for this, unlike [`trigger_graph`]'s manual (no `trigger_kind`)
/// trigger.
fn automatic_schedule_graph() -> WorkflowGraph {
    WorkflowGraph {
        nodes: vec![Node {
            id: "t".to_string(),
            kind: NodeKind::Trigger,
            type_version: 1,
            name: "Trigger".to_string(),
            config: serde_json::json!({ "trigger_kind": "schedule", "schedule": "0 9 * * *" }),
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

    let (listed, skipped) = list_flows(&config).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, flow.id);
    assert_eq!(skipped, 0);

    remove_flow(&config, &flow.id).unwrap();
    assert!(get_flow(&config, &flow.id).unwrap().is_none());
    assert!(list_flows(&config).unwrap().0.is_empty());
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
        false,
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
        None,  // enabled_override
        false, // force_disarm_if_automatic
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
        false,       // force_disarm_if_automatic
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
        false,       // force_disarm_if_automatic
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

/// R-m2 regression: the manual→automatic disarm decision must be computed
/// against the row `update_flow_graph` JUST re-read (`current`), never a
/// caller-supplied belief about the flow's prior state. Before the fix,
/// `ops::flows_update` computed this transition from an OUTER `existing`
/// read taken before calling into the store — a concurrent write between
/// that read and this call could make the transition invisible to the
/// caller, letting an automatic-trigger graph persist `enabled: true`.
///
/// Proven here without needing to fake a race: the disarm must fire from
/// `current.graph` (MANUAL) vs the new `graph` (automatic) alone, and must
/// WIN over an `enabled_override` that explicitly asks to stay enabled —
/// exactly the shape of override a stale caller-side decision could
/// otherwise have smuggled through.
#[test]
fn update_flow_graph_disarms_transition_from_the_fresh_row_even_when_override_asks_to_stay_enabled()
{
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow = create_flow(&config, "demo".to_string(), trigger_graph(), false, true).unwrap();
    assert!(flow.enabled, "flow created enabled");

    let updated = update_flow_graph(
        &config,
        &flow.id,
        flow.name.clone(),
        automatic_schedule_graph(),
        false,
        Some(true), // caller explicitly asks to stay enabled
        false,      // force_disarm_if_automatic (the remote-authoring flag) OFF —
        // proving the unconditional Rule 1 transition-disarm fires on its own
        None,
    )
    .unwrap();

    assert!(
        !updated.enabled,
        "a manual->automatic transition must disarm even when enabled_override asks to stay \
         enabled — the disarm always wins (R-m2)"
    );
    let reloaded = get_flow(&config, &flow.id).unwrap().unwrap();
    assert!(!reloaded.enabled);
}

/// Sibling of the above: when there is NO transition (the row was already
/// automatic before this call, matching what's actually in the DB right
/// now), an ordinary `enabled_override` is honoured normally — the fix must
/// not over-disarm every automatic-trigger update, only genuine
/// manual/none → automatic transitions (unless `force_disarm_if_automatic`
/// is also set).
#[test]
fn update_flow_graph_does_not_disarm_an_automatic_to_automatic_update() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow = create_flow(
        &config,
        "demo".to_string(),
        automatic_schedule_graph(),
        false,
        false,
    )
    .unwrap();
    assert!(!flow.enabled, "born disabled — armed explicitly next");
    let armed = set_enabled(&config, &flow.id, true).unwrap();
    assert!(armed.enabled);

    let updated = update_flow_graph(
        &config,
        &flow.id,
        flow.name.clone(),
        automatic_schedule_graph(),
        false,
        None,  // no explicit override — preserve current.enabled
        false, // force_disarm_if_automatic OFF
        None,
    )
    .unwrap();

    assert!(
        updated.enabled,
        "an automatic->automatic update (no transition) must not be auto-disarmed"
    );
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
        false,
        None,
    )
    .unwrap();
    assert!(updated.require_approval);

    let reloaded = get_flow(&config, &flow.id).unwrap().unwrap();
    assert!(reloaded.require_approval);
}

#[test]
fn legacy_flow_definitions_row_without_require_approval_column_defaults_false() {
    // A row inserted before the `require_approval` column existed. Schema
    // init (including the `add_column_if_missing` ALTER) runs once per
    // process per database file (R-m8) — since this test opens a fresh
    // per-`TempDir` database, that one-time init still runs here, simulating
    // a workspace opened once on an older build.
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

    let (enabled, skipped) = list_enabled_flows(&config).unwrap();
    assert_eq!(enabled.len(), 1);
    assert_eq!(enabled[0].id, enabled_flow.id);
    assert_eq!(skipped, 0);
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
        None,
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
    assert_eq!(list_flows(&config).unwrap().0.len(), 2);
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

// ── Orphaned-running-run reconciliation (bug B42) ──────────────────────────

#[test]
fn list_running_run_ids_returns_only_running_rows() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow = create_flow(&config, "demo".to_string(), trigger_graph(), false, true).unwrap();

    insert_flow_run(
        &config,
        "run-live-1",
        &flow.id,
        "run-live-1",
        "2026-01-01T00:00:00Z",
    )
    .unwrap();
    insert_flow_run(
        &config,
        "run-live-2",
        &flow.id,
        "run-live-2",
        "2026-01-01T00:00:01Z",
    )
    .unwrap();
    insert_flow_run(
        &config,
        "run-done",
        &flow.id,
        "run-done",
        "2026-01-01T00:00:02Z",
    )
    .unwrap();
    finish_flow_run(
        &config,
        "run-done",
        "completed",
        "2026-01-01T00:00:03Z",
        &[],
        &[],
        None,
        None,
    )
    .unwrap();

    let mut running = list_running_run_ids(&config, "2099-01-01T00:00:00Z").unwrap();
    running.sort();
    assert_eq!(
        running,
        vec![
            ("run-live-1".to_string(), flow.id.clone()),
            ("run-live-2".to_string(), flow.id.clone()),
        ],
        "only the two still-running rows must be listed, not the completed one"
    );
}

#[test]
fn list_running_run_ids_excludes_rows_started_at_or_after_the_floor() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow = create_flow(&config, "demo".to_string(), trigger_graph(), false, true).unwrap();

    insert_flow_run(
        &config,
        "run-old",
        &flow.id,
        "run-old",
        "2026-01-01T00:00:00Z",
    )
    .unwrap();
    insert_flow_run(
        &config,
        "run-at",
        &flow.id,
        "run-at",
        "2026-01-01T00:00:05Z",
    )
    .unwrap();
    insert_flow_run(
        &config,
        "run-new",
        &flow.id,
        "run-new",
        "2026-01-01T00:00:09Z",
    )
    .unwrap();

    // The floor is exclusive: a row stamped exactly at the boot floor was
    // inserted by THIS process (`start_flow_run_row` anchors the floor before
    // stamping), so it must fall outside the candidate set along with newer
    // rows — otherwise the sweep could interrupt a live run and drop its
    // checkpoint mid-flight.
    let running = list_running_run_ids(&config, "2026-01-01T00:00:05Z").unwrap();
    assert_eq!(
        running,
        vec![("run-old".to_string(), flow.id.clone())],
        "only rows strictly older than the floor are sweep candidates"
    );
}

#[test]
fn mark_run_interrupted_reconciles_a_running_row_with_reason() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow = create_flow(&config, "demo".to_string(), trigger_graph(), false, true).unwrap();
    insert_flow_run(&config, "run-x", &flow.id, "run-x", "2026-01-01T00:00:00Z").unwrap();

    let flipped =
        mark_run_interrupted(&config, "run-x", "2026-01-01T00:05:00Z", "boom reason").unwrap();
    assert!(flipped, "a running row must be reconciled");

    let row = get_flow_run(&config, "run-x").unwrap().unwrap();
    assert_eq!(row.status, "interrupted");
    assert_eq!(row.finished_at.as_deref(), Some("2026-01-01T00:05:00Z"));
    assert_eq!(row.error.as_deref(), Some("boom reason"));
}

#[test]
fn mark_run_interrupted_is_a_noop_for_a_terminal_row() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow = create_flow(&config, "demo".to_string(), trigger_graph(), false, true).unwrap();
    insert_flow_run(&config, "run-y", &flow.id, "run-y", "2026-01-01T00:00:00Z").unwrap();
    finish_flow_run(
        &config,
        "run-y",
        "completed",
        "2026-01-01T00:00:01Z",
        &[],
        &[],
        None,
        None,
    )
    .unwrap();

    // The `status = 'running'` guard must protect an already-settled run.
    let flipped =
        mark_run_interrupted(&config, "run-y", "2026-01-01T00:05:00Z", "should not apply").unwrap();
    assert!(
        !flipped,
        "a completed run must never be clobbered to interrupted"
    );

    let row = get_flow_run(&config, "run-y").unwrap().unwrap();
    assert_eq!(row.status, "completed");
    assert!(row.error.is_none());
}

/// `expire_parked_runs` must return only the runs it ACTUALLY flipped, not the
/// candidates its `SELECT` saw.
///
/// The `SELECT` and each row's guarded `UPDATE` are separate statements on an
/// autocommit connection, so a concurrent `mark_run_resuming` can claim a row in
/// between. The per-row `WHERE status = 'pending_approval'` keeps that row safe,
/// but returning the unfiltered candidate list would let the caller act on a run
/// it never expired — dropping the checkpoint out from under a live resume and
/// publishing a terminal `FlowRunFinished` for a run still executing. That false
/// event is the worse half: the frontend de-dupes terminal events per
/// `flow_id:run_id`, so the run's real completion would later be discarded.
#[test]
fn expire_parked_runs_returns_only_rows_it_actually_flipped() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow = create_flow(&config, "ttl".to_string(), trigger_graph(), false, true).unwrap();

    let stale_at = "2000-01-01T00:00:00+00:00";
    for id in ["claimed-run", "genuinely-stale-run"] {
        insert_flow_run(&config, id, &flow.id, id, stale_at).unwrap();
        finish_flow_run(
            &config,
            id,
            "pending_approval",
            stale_at,
            &[],
            &["gate".to_string()],
            None,
            // No graph pin (T-M1): this fixture is about the TTL sweep's
            // candidates-vs-sweeps behaviour, not stale-approval detection, so
            // these rows stand in for pre-pin legacy parks.
            None,
        )
        .unwrap();
    }

    // Simulate the race: one candidate is claimed by a resume after the sweep's
    // SELECT would have seen it, but before its UPDATE lands.
    assert!(mark_run_resuming(&config, "claimed-run").unwrap());

    let swept = expire_parked_runs(
        &config,
        "2099-01-01T00:00:00+00:00",
        "2026-01-01T00:00:00+00:00",
        "expired",
    )
    .unwrap();

    let swept_ids: Vec<&str> = swept.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(
        swept_ids,
        vec!["genuinely-stale-run"],
        "only the row whose guarded UPDATE matched may be reported as swept"
    );
    assert_eq!(
        get_flow_run(&config, "claimed-run")
            .unwrap()
            .unwrap()
            .status,
        "running",
        "the claimed run must keep executing, untouched by the sweep"
    );
    assert_eq!(
        get_flow_run(&config, "genuinely-stale-run")
            .unwrap()
            .unwrap()
            .status,
        "cancelled"
    );
}

// ── R-M4: corrupt/unmigratable graph_json rows must not brick a list ────────

#[test]
fn list_flows_skips_a_corrupt_row_and_reports_the_count() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let good_a = create_flow(&config, "good-a".to_string(), trigger_graph(), false, true).unwrap();
    let bad = create_flow(&config, "bad".to_string(), trigger_graph(), false, true).unwrap();
    let good_b = create_flow(&config, "good-b".to_string(), trigger_graph(), false, true).unwrap();
    force_corrupt_graph_json_for_test(&config, &bad.id, "{ not even valid json").unwrap();

    let (flows, skipped) = list_flows(&config).unwrap();
    assert_eq!(
        skipped, 1,
        "exactly the one corrupt row must be counted as skipped"
    );
    let ids: Vec<&str> = flows.iter().map(|f| f.id.as_str()).collect();
    assert_eq!(
        flows.len(),
        2,
        "the two good rows must still be returned: {ids:?}"
    );
    assert!(ids.contains(&good_a.id.as_str()));
    assert!(ids.contains(&good_b.id.as_str()));
    assert!(!ids.contains(&bad.id.as_str()));
}

#[test]
fn list_flows_skips_a_row_whose_schema_version_is_newer_than_this_build_supports() {
    // The real-world R-M4 scenario: a user ran a newer build that persisted a
    // graph at a `schema_version` this build's `tinyflows::migrate::migrate`
    // cannot step backward from, then downgraded.
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let good = create_flow(&config, "good".to_string(), trigger_graph(), false, true).unwrap();
    let too_new =
        create_flow(&config, "too-new".to_string(), trigger_graph(), false, true).unwrap();
    let newer_schema_json = serde_json::json!({
        "schema_version": 999,
        "name": "from-the-future",
        "nodes": [],
        "edges": []
    })
    .to_string();
    force_corrupt_graph_json_for_test(&config, &too_new.id, &newer_schema_json).unwrap();

    let (flows, skipped) = list_flows(&config).unwrap();
    assert_eq!(skipped, 1);
    assert_eq!(flows.len(), 1);
    assert_eq!(flows[0].id, good.id);
}

#[test]
fn list_enabled_flows_still_returns_the_good_rows_when_one_is_corrupt() {
    // This is the blast-radius scenario R-M4 flags for `bus.rs::handle_app_event`:
    // `list_enabled_flows` backs ALL `app_event` trigger dispatch, so one
    // corrupt enabled flow must not blackhole matching for every other one.
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let good = create_flow(&config, "good".to_string(), trigger_graph(), false, true).unwrap();
    let bad = create_flow(&config, "bad".to_string(), trigger_graph(), false, true).unwrap();
    force_corrupt_graph_json_for_test(&config, &bad.id, "not json at all").unwrap();

    let (enabled, skipped) = list_enabled_flows(&config).unwrap();
    assert_eq!(skipped, 1);
    assert_eq!(enabled.len(), 1);
    assert_eq!(enabled[0].id, good.id);
}

#[test]
fn list_enabled_flows_excludes_a_corrupt_disabled_row_without_counting_it_as_skipped() {
    // A corrupt row that was never enabled must not even be attempted for
    // decode by `list_enabled_flows` (the WHERE clause filters it out at the
    // SQL layer before `map_flow_row` ever runs) — it is neither returned nor
    // counted as skipped by this particular listing.
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let good = create_flow(&config, "good".to_string(), trigger_graph(), false, true).unwrap();
    let disabled_and_corrupt = create_flow(
        &config,
        "disabled-bad".to_string(),
        trigger_graph(),
        false,
        true,
    )
    .unwrap();
    set_enabled(&config, &disabled_and_corrupt.id, false).unwrap();
    force_corrupt_graph_json_for_test(&config, &disabled_and_corrupt.id, "{{{").unwrap();

    let (enabled, skipped) = list_enabled_flows(&config).unwrap();
    assert_eq!(skipped, 0);
    assert_eq!(enabled.len(), 1);
    assert_eq!(enabled[0].id, good.id);
}

// ── R-m1: concurrent step upserts must not lose a step ──────────────────────

#[test]
fn concurrent_step_upserts_do_not_lose_a_step() {
    // Two observer callbacks for parallel branch nodes of the same run,
    // racing to persist their step. Before the `BEGIN IMMEDIATE` fix this was
    // a classic untransacted read-modify-write: both threads could read the
    // same pre-write `steps_json`, and whichever `UPDATE` landed last would
    // silently discard the other thread's step — permanently, since the
    // post-hoc `settle_steps` reconstruction only refills a missing node with
    // `status: None`, not its real outcome.
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow = create_flow(&config, "demo".to_string(), trigger_graph(), false, true).unwrap();
    let run_id = "run-concurrent";
    insert_flow_run(&config, run_id, &flow.id, run_id, "2026-01-01T00:00:00Z").unwrap();

    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));

    let config_a = config.clone();
    let barrier_a = barrier.clone();
    let handle_a = std::thread::spawn(move || {
        barrier_a.wait();
        upsert_flow_run_step(
            &config_a,
            run_id,
            &FlowRunStep {
                node_id: "branch-a".to_string(),
                output: serde_json::json!([{"json": {"a": 1}}]),
                status: Some("success".to_string()),
                ..Default::default()
            },
        )
    });

    let config_b = config.clone();
    let barrier_b = barrier.clone();
    let handle_b = std::thread::spawn(move || {
        barrier_b.wait();
        upsert_flow_run_step(
            &config_b,
            run_id,
            &FlowRunStep {
                node_id: "branch-b".to_string(),
                output: serde_json::json!([{"json": {"b": 1}}]),
                status: Some("success".to_string()),
                ..Default::default()
            },
        )
    });

    handle_a.join().unwrap().unwrap();
    handle_b.join().unwrap().unwrap();

    let row = get_flow_run(&config, run_id).unwrap().unwrap();
    let node_ids: std::collections::HashSet<&str> =
        row.steps.iter().map(|s| s.node_id.as_str()).collect();
    assert_eq!(
        row.steps.len(),
        2,
        "both concurrent steps must survive, none silently dropped: {:?}",
        row.steps
    );
    assert!(node_ids.contains("branch-a"));
    assert!(node_ids.contains("branch-b"));
}

#[test]
fn concurrent_upserts_to_the_same_node_id_do_not_corrupt_the_step_list() {
    // Same run, same node_id, racing "replace" writes — the transaction must
    // still leave exactly one entry for that node (whichever write wins the
    // serialization order), never a torn/duplicated list.
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow = create_flow(&config, "demo".to_string(), trigger_graph(), false, true).unwrap();
    let run_id = "run-same-node";
    insert_flow_run(&config, run_id, &flow.id, run_id, "2026-01-01T00:00:00Z").unwrap();

    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let mut handles = Vec::new();
    for i in 0..2 {
        let config = config.clone();
        let barrier = barrier.clone();
        handles.push(std::thread::spawn(move || {
            barrier.wait();
            upsert_flow_run_step(
                &config,
                run_id,
                &FlowRunStep {
                    node_id: "same-node".to_string(),
                    output: serde_json::json!([{"json": {"attempt": i}}]),
                    status: Some("success".to_string()),
                    ..Default::default()
                },
            )
        }));
    }
    for h in handles {
        h.join().unwrap().unwrap();
    }

    let row = get_flow_run(&config, run_id).unwrap().unwrap();
    assert_eq!(
        row.steps.len(),
        1,
        "a re-upsert of the same node_id must replace, not duplicate: {:?}",
        row.steps
    );
    assert_eq!(row.steps[0].node_id, "same-node");
}

// ── R-m8: schema init is gated to once per process per database path ───────

#[test]
fn schema_initializes_correctly_on_a_fresh_database_and_is_idempotent_across_calls() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    // First-ever call against this database file in the process: exercises
    // the full schema DDL (CREATE TABLE batch + indexes) plus the
    // `require_approval` `add_column_if_missing` migration on a database that
    // has never been opened before.
    let flow = create_flow(
        &config,
        "fresh-db".to_string(),
        trigger_graph(),
        true, // require_approval
        true,
    )
    .unwrap();
    assert!(
        flow.require_approval,
        "the post-hoc require_approval column must exist and be writable on a brand-new db"
    );

    // Repeat calls against the SAME path must not need (or re-run) DDL —
    // proves the cached "already initialized" state doesn't break ordinary
    // reads/writes on reuse.
    let (listed, skipped) = list_flows(&config).unwrap();
    assert_eq!(skipped, 0);
    assert_eq!(listed.len(), 1);
    assert!(listed[0].require_approval);

    let reloaded = get_flow(&config, &flow.id).unwrap().unwrap();
    assert!(reloaded.require_approval);

    let run_id = "run-schema-check";
    insert_flow_run(&config, run_id, &flow.id, run_id, "2026-01-01T00:00:00Z").unwrap();
    assert!(get_flow_run(&config, run_id).unwrap().is_some());
}

#[test]
fn schema_initializes_independently_for_each_distinct_database_path() {
    // Regression guard for the once-per-process cache: if it were keyed by a
    // single process-wide flag instead of by database path, opening a SECOND
    // independent workspace after the first would silently skip schema
    // creation and every write against it would fail with "no such table".
    let tmp_a = TempDir::new().unwrap();
    let config_a = test_config(&tmp_a);
    let flow_a = create_flow(&config_a, "a".to_string(), trigger_graph(), false, true).unwrap();

    let tmp_b = TempDir::new().unwrap();
    let config_b = test_config(&tmp_b);
    let flow_b = create_flow(&config_b, "b".to_string(), trigger_graph(), false, true).unwrap();

    assert_eq!(list_flows(&config_a).unwrap().0.len(), 1);
    assert_eq!(list_flows(&config_b).unwrap().0.len(), 1);
    assert_eq!(
        get_flow(&config_a, &flow_a.id).unwrap().unwrap().id,
        flow_a.id
    );
    assert_eq!(
        get_flow(&config_b, &flow_b.id).unwrap().unwrap().id,
        flow_b.id
    );
}

/// R-m8 regression: gating the DDL behind a per-path "already initialized" set
/// must not cost the store its self-healing.
///
/// Before the gate existed, the DDL ran on every `with_connection` call, so a
/// database deleted or replaced at runtime (workspace reset, manual deletion,
/// a restore) recovered on the very next call — `Connection::open` creates a
/// fresh empty file and `CREATE TABLE IF NOT EXISTS` repopulates it. With a
/// naive cache the set still reports "initialized" while the file behind it is
/// empty, and every query afterwards fails `no such table: flow_definitions`
/// until the process restarts. This pins the verify-on-hit that restores it.
#[test]
fn schema_reinitializes_when_the_database_file_is_deleted_at_runtime() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    // First use populates the per-path cache and creates the schema.
    let flow = create_flow(
        &config,
        "before-deletion".to_string(),
        trigger_graph(),
        false,
        true,
    )
    .unwrap();
    let (flows, _skipped) = list_flows(&config).unwrap();
    assert_eq!(flows.len(), 1, "sanity: the flow was persisted");

    // Simulate a workspace reset / manual deletion while the process lives on.
    let db_path = config.workspace_dir.join("flows").join("flows.db");
    assert!(
        db_path.exists(),
        "sanity: the flows db exists before deletion"
    );
    std::fs::remove_file(&db_path).unwrap();
    // WAL sidecars must go too, or SQLite can resurrect pages from them.
    let _ = std::fs::remove_file(db_path.with_extension("db-wal"));
    let _ = std::fs::remove_file(db_path.with_extension("db-shm"));

    // The cache still says this path is initialized. Without the verify-on-hit
    // this errors with `no such table: flow_definitions`.
    let (flows_after, skipped_after) = list_flows(&config)
        .expect("a deleted database must be re-initialized, not left wedged at 'no such table'");
    assert!(
        flows_after.is_empty(),
        "the recreated database starts empty — the prior flow is genuinely gone"
    );
    assert_eq!(skipped_after, 0, "an empty database skips nothing");

    // And the store is fully usable again, not merely readable.
    let recreated = create_flow(
        &config,
        "after-deletion".to_string(),
        trigger_graph(),
        false,
        true,
    )
    .expect("writes must work against the re-initialized schema");
    assert_ne!(recreated.id, flow.id);
    let (flows_final, _) = list_flows(&config).unwrap();
    assert_eq!(flows_final.len(), 1);
}
