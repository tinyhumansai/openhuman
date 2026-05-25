use super::*;
use crate::openhuman::config::Config;
use crate::openhuman::inference::provider::traits::Provider;
use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use tempfile::TempDir;

// ── Shared helpers ────────────────────────────────────────────────────────

fn test_config(tmp: &TempDir) -> Config {
    Config {
        workspace_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    }
}

/// A stub Provider whose `chat_with_system` returns a fixed string.
struct StubProvider {
    reply: String,
}

impl StubProvider {
    fn with_reply(reply: impl Into<String>) -> Self {
        Self {
            reply: reply.into(),
        }
    }
}

#[async_trait]
impl Provider for StubProvider {
    async fn chat_with_system(
        &self,
        _system: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<String> {
        log::debug!("[memory_tree_test] StubProvider::chat_with_system called");
        Ok(self.reply.clone())
    }
}

// ── group_by_hour ────────────────────────────────────────────────────────

#[test]
fn group_by_hour_empty_input_returns_empty_map() {
    log::debug!("[memory_tree_test] group_by_hour: empty input");
    let groups = group_by_hour(&[]);
    assert!(groups.is_empty());
}

#[test]
fn group_by_hour_single_entry_maps_to_correct_hour() {
    // Timestamp 1_711_958_400_000 = 2024-04-01T08:00:00Z
    let filename = "1711958400000_abc12345.md".to_string();
    let content = "hello".to_string();
    let entries = vec![(filename, content)];
    let groups = group_by_hour(&entries);
    log::debug!("[memory_tree_test] group_by_hour single: groups={groups:?}");
    assert_eq!(groups.len(), 1);
    let key = groups.keys().next().unwrap();
    // Hour node id should be "YYYY/MM/DD/HH"
    assert_eq!(key.matches('/').count(), 3, "hour id must have 3 slashes");
    assert!(
        key.starts_with("2024/04/01/"),
        "expected 2024-04-01 hour key; got: {key}"
    );
    assert!(key.ends_with("/08"), "expected hour /08; got: {key}");
}

#[test]
fn group_by_hour_same_hour_entries_are_merged() {
    // Two timestamps within the same hour.
    let ts_a = 1_711_958_400_000_i64; // 2024-04-01T08:00:00Z
    let ts_b = ts_a + 1_800_000; // +30 min, still same hour

    let entries = vec![
        (format!("{ts_a}_uuid1.md"), "msg-a".to_string()),
        (format!("{ts_b}_uuid2.md"), "msg-b".to_string()),
    ];
    let groups = group_by_hour(&entries);
    log::debug!("[memory_tree_test] group_by_hour same-hour: groups={groups:?}");
    assert_eq!(
        groups.len(),
        1,
        "same-hour entries must collapse into one group"
    );
    let contents = groups.values().next().unwrap();
    assert_eq!(contents.len(), 2);
    assert!(contents.contains(&"msg-a".to_string()));
    assert!(contents.contains(&"msg-b".to_string()));
}

#[test]
fn group_by_hour_different_hours_produce_separate_groups() {
    // 2024-04-01T08:00:00Z and 2024-04-01T09:00:00Z
    let ts_h8 = 1_711_958_400_000_i64;
    let ts_h9 = 1_711_962_000_000_i64;

    let entries = vec![
        (format!("{ts_h8}_uuid1.md"), "hour-8-msg".to_string()),
        (format!("{ts_h9}_uuid2.md"), "hour-9-msg".to_string()),
    ];
    let groups = group_by_hour(&entries);
    log::debug!("[memory_tree_test] group_by_hour diff-hours: groups={groups:?}");
    assert_eq!(groups.len(), 2);
    let keys: Vec<&String> = groups.keys().collect();
    // BTreeMap returns sorted keys.
    assert!(
        keys[0].ends_with("/08"),
        "first key should end in /08; got {}",
        keys[0]
    );
    assert!(
        keys[1].ends_with("/09"),
        "second key should end in /09; got {}",
        keys[1]
    );
}

#[test]
fn group_by_hour_unparseable_filename_falls_back_to_current_hour() {
    // A filename with no timestamp prefix should fall back without panic.
    let entries = vec![("bad-filename.md".to_string(), "content".to_string())];
    let groups = group_by_hour(&entries);
    // Must produce exactly one group (fallback to now).
    assert_eq!(groups.len(), 1);
    let key = groups.keys().next().unwrap();
    assert_eq!(
        key.matches('/').count(),
        3,
        "fallback key must still be a valid hour id"
    );
}

#[test]
fn group_by_hour_output_is_ordered_by_hour_id() {
    // BTreeMap guarantees sorted iteration — verify the API honours that.
    // Timestamps verified: 2024-04-01T{08,10,12}:00:00Z
    let ts_h08 = 1_711_958_400_000_i64; // 2024-04-01T08:00:00Z
    let ts_h10 = 1_711_965_600_000_i64; // 2024-04-01T10:00:00Z
    let ts_h12 = 1_711_972_800_000_i64; // 2024-04-01T12:00:00Z

    let entries = vec![
        (format!("{ts_h10}_a.md"), "10h".to_string()),
        (format!("{ts_h08}_b.md"), "8h".to_string()),
        (format!("{ts_h12}_c.md"), "12h".to_string()),
    ];
    let groups = group_by_hour(&entries);
    let keys: Vec<&String> = groups.keys().collect();
    log::debug!("[memory_tree_test] group_by_hour ordering: keys={keys:?}");
    assert_eq!(keys.len(), 3);
    // Sorted lexicographically: .../08 < .../10 < .../12
    assert!(keys[0] < keys[1]);
    assert!(keys[1] < keys[2]);
}

// ── propagate_node ────────────────────────────────────────────────────────

#[tokio::test]
async fn propagate_node_with_no_children_is_noop() {
    let tmp = TempDir::new().unwrap();
    let cfg = test_config(&tmp);
    std::fs::create_dir_all(&cfg.workspace_dir).unwrap();
    let provider = StubProvider::with_reply("should not be called");
    let model = "stub-model";
    // No children exist for "2024/03/15" — propagate must succeed silently.
    let result = propagate_node(
        &cfg,
        &provider,
        "test-ns",
        "2024/03/15",
        NodeLevel::Day,
        model,
    )
    .await;
    assert!(result.is_ok(), "empty children must not error: {result:?}");
    // No node should have been written.
    let node = store::read_node(&cfg, "test-ns", "2024/03/15").unwrap();
    assert!(
        node.is_none(),
        "propagate with no children must not write a node"
    );
}

#[tokio::test]
async fn propagate_node_day_from_hour_children_fits_budget() {
    // Write two small hour leaves; their combined tokens are well within
    // Day::max_tokens() so the LLM is NOT called — combined text is used directly.
    let tmp = TempDir::new().unwrap();
    let cfg = test_config(&tmp);
    std::fs::create_dir_all(&cfg.workspace_dir).unwrap();

    let now = Utc.with_ymd_and_hms(2024, 3, 15, 8, 0, 0).unwrap();
    let make_hour_node = |ns: &str, hour_id: &str, summary: &str| TreeNode {
        node_id: hour_id.to_string(),
        namespace: ns.to_string(),
        level: NodeLevel::Hour,
        parent_id: derive_parent_id(hour_id),
        summary: summary.to_string(),
        token_count: estimate_tokens(summary),
        child_count: 0,
        created_at: now,
        updated_at: now,
        metadata: None,
    };

    let ns = "test-ns";
    store::write_node(
        &cfg,
        &make_hour_node(ns, "2024/03/15/08", "Meeting at 8am."),
    )
    .unwrap();
    store::write_node(
        &cfg,
        &make_hour_node(ns, "2024/03/15/09", "Stand-up at 9am."),
    )
    .unwrap();

    // StubProvider reply should NOT be used when content fits budget.
    let provider = StubProvider::with_reply("SHOULD_NOT_APPEAR");
    propagate_node(&cfg, &provider, ns, "2024/03/15", NodeLevel::Day, "m")
        .await
        .unwrap();

    let day_node = store::read_node(&cfg, ns, "2024/03/15").unwrap().unwrap();
    log::debug!(
        "[memory_tree_test] propagate day: summary={}",
        day_node.summary
    );
    assert_eq!(day_node.level, NodeLevel::Day);
    assert!(day_node.summary.contains("Meeting at 8am."));
    assert!(day_node.summary.contains("Stand-up at 9am."));
    // LLM reply must not appear when content fits the budget.
    assert!(!day_node.summary.contains("SHOULD_NOT_APPEAR"));
    assert!(day_node.child_count >= 2);
}

#[tokio::test]
async fn propagate_node_month_from_day_children() {
    let tmp = TempDir::new().unwrap();
    let cfg = test_config(&tmp);
    std::fs::create_dir_all(&cfg.workspace_dir).unwrap();
    let now = Utc::now();
    let ns = "test-ns";

    let make_day = |id: &str, text: &str| TreeNode {
        node_id: id.to_string(),
        namespace: ns.to_string(),
        level: NodeLevel::Day,
        parent_id: derive_parent_id(id),
        summary: text.to_string(),
        token_count: estimate_tokens(text),
        child_count: 1,
        created_at: now,
        updated_at: now,
        metadata: None,
    };

    store::write_node(&cfg, &make_day("2024/03/14", "Day 14 recap.")).unwrap();
    store::write_node(&cfg, &make_day("2024/03/15", "Day 15 recap.")).unwrap();

    let provider = StubProvider::with_reply("Month summary from LLM.");
    propagate_node(&cfg, &provider, ns, "2024/03", NodeLevel::Month, "m")
        .await
        .unwrap();

    let month = store::read_node(&cfg, ns, "2024/03").unwrap().unwrap();
    assert_eq!(month.level, NodeLevel::Month);
    // Either both day summaries appear (budget fit) or the stub reply is used.
    let has_day_content = month.summary.contains("Day 14") || month.summary.contains("Day 15");
    let has_stub = month.summary.contains("Month summary from LLM.");
    assert!(
        has_day_content || has_stub,
        "month summary must contain day content or stub reply; got: {}",
        month.summary
    );
}

#[tokio::test]
async fn propagate_node_preserves_created_at_on_update() {
    // If a node already exists, propagate must NOT overwrite `created_at`.
    let tmp = TempDir::new().unwrap();
    let cfg = test_config(&tmp);
    std::fs::create_dir_all(&cfg.workspace_dir).unwrap();
    let ns = "test-ns";
    let now = Utc::now();

    // Write an existing day node.
    let existing = TreeNode {
        node_id: "2024/03/15".to_string(),
        namespace: ns.to_string(),
        level: NodeLevel::Day,
        parent_id: Some("2024/03".to_string()),
        summary: "old summary".to_string(),
        token_count: 10,
        child_count: 0,
        created_at: now - chrono::Duration::hours(5),
        updated_at: now - chrono::Duration::hours(5),
        metadata: None,
    };
    store::write_node(&cfg, &existing).unwrap();
    let original_created_at = existing.created_at;

    // Write a child so propagation has something to do.
    let child = TreeNode {
        node_id: "2024/03/15/10".to_string(),
        namespace: ns.to_string(),
        level: NodeLevel::Hour,
        parent_id: Some("2024/03/15".to_string()),
        summary: "hour content".to_string(),
        token_count: 5,
        child_count: 0,
        created_at: now,
        updated_at: now,
        metadata: None,
    };
    store::write_node(&cfg, &child).unwrap();

    let provider = StubProvider::with_reply("updated summary");
    propagate_node(&cfg, &provider, ns, "2024/03/15", NodeLevel::Day, "m")
        .await
        .unwrap();

    let updated = store::read_node(&cfg, ns, "2024/03/15").unwrap().unwrap();
    assert_eq!(
        updated.created_at, original_created_at,
        "created_at must be preserved across propagation updates"
    );
    assert!(
        updated.updated_at >= now,
        "updated_at must be refreshed on re-propagation"
    );
}

// ── run_summarization end-to-end ──────────────────────────────────────────

#[tokio::test]
async fn run_summarization_empty_buffer_returns_none() {
    let tmp = TempDir::new().unwrap();
    let cfg = test_config(&tmp);
    std::fs::create_dir_all(&cfg.workspace_dir).unwrap();
    let provider = StubProvider::with_reply("should not be called");
    let ts = Utc::now();
    let result = run_summarization(&cfg, &provider, "test-ns", ts)
        .await
        .unwrap();
    assert!(result.is_none(), "empty buffer must return None");
}

#[tokio::test]
async fn run_summarization_drains_buffer_and_writes_hour_node() {
    let tmp = TempDir::new().unwrap();
    let cfg = test_config(&tmp);
    std::fs::create_dir_all(&cfg.workspace_dir).unwrap();
    let ns = "test-ns";

    // Write two buffer entries at the same hour so they merge into one hour leaf.
    let ts = Utc.with_ymd_and_hms(2024, 3, 15, 10, 0, 0).unwrap();
    store::buffer_write(&cfg, ns, "entry one", &ts, None).unwrap();
    store::buffer_write(&cfg, ns, "entry two", &ts, None).unwrap();

    let provider = StubProvider::with_reply("hour leaf summary from LLM");
    let last_node = run_summarization(&cfg, &provider, ns, ts).await.unwrap();

    let node = last_node.expect("non-empty buffer must return an hour node");
    log::debug!(
        "[memory_tree_test] run_summarization: hour node_id={}",
        node.node_id
    );
    assert_eq!(node.level, NodeLevel::Hour);
    assert_eq!(node.namespace, ns);

    // Buffer must be drained after successful run.
    let remaining = store::buffer_read(&cfg, ns).unwrap();
    assert!(
        remaining.is_empty(),
        "buffer must be empty after summarization"
    );

    // The hour node must be persisted.
    let stored = store::read_node(&cfg, ns, &node.node_id).unwrap();
    assert!(stored.is_some(), "hour node must be written to disk");
}

#[tokio::test]
async fn run_summarization_builds_ancestor_chain() {
    // After a successful run the day/month/year/root ancestor chain must be written.
    let tmp = TempDir::new().unwrap();
    let cfg = test_config(&tmp);
    std::fs::create_dir_all(&cfg.workspace_dir).unwrap();
    let ns = "ancestor-test";
    let ts = Utc.with_ymd_and_hms(2024, 3, 15, 10, 0, 0).unwrap();

    store::buffer_write(&cfg, ns, "test content", &ts, None).unwrap();

    let provider = StubProvider::with_reply("summary text");
    run_summarization(&cfg, &provider, ns, ts).await.unwrap();

    // Day, month, year, and root must all be present.
    assert!(
        store::read_node(&cfg, ns, "2024/03/15").unwrap().is_some(),
        "day node must be propagated"
    );
    assert!(
        store::read_node(&cfg, ns, "2024/03").unwrap().is_some(),
        "month node must be propagated"
    );
    assert!(
        store::read_node(&cfg, ns, "2024").unwrap().is_some(),
        "year node must be propagated"
    );
    assert!(
        store::read_node(&cfg, ns, "root").unwrap().is_some(),
        "root node must be propagated"
    );
}

#[tokio::test]
async fn run_summarization_multi_hour_groups_produce_multiple_hour_leaves() {
    let tmp = TempDir::new().unwrap();
    let cfg = test_config(&tmp);
    std::fs::create_dir_all(&cfg.workspace_dir).unwrap();
    let ns = "multi-hour-test";

    let ts_h08 = Utc.with_ymd_and_hms(2024, 3, 15, 8, 0, 0).unwrap();
    let ts_h14 = Utc.with_ymd_and_hms(2024, 3, 15, 14, 0, 0).unwrap();

    // Write entries for two different hours.
    store::buffer_write(&cfg, ns, "morning entry", &ts_h08, None).unwrap();
    store::buffer_write(&cfg, ns, "afternoon entry", &ts_h14, None).unwrap();

    let provider = StubProvider::with_reply("grouped summary");
    run_summarization(&cfg, &provider, ns, ts_h14)
        .await
        .unwrap();

    // Both hour leaves must be written.
    let hour_08 = store::read_node(&cfg, ns, "2024/03/15/08").unwrap();
    let hour_14 = store::read_node(&cfg, ns, "2024/03/15/14").unwrap();
    assert!(hour_08.is_some(), "hour-08 leaf must be written");
    assert!(hour_14.is_some(), "hour-14 leaf must be written");

    // Buffer must be empty.
    assert!(store::buffer_read(&cfg, ns).unwrap().is_empty());
}

#[tokio::test]
async fn rebuild_tree_restores_buffer_and_rewrites_ancestors() {
    let tmp = TempDir::new().unwrap();
    let cfg = test_config(&tmp);
    std::fs::create_dir_all(&cfg.workspace_dir).unwrap();
    let ns = "rebuild-test";
    let ts = Utc.with_ymd_and_hms(2024, 3, 15, 10, 0, 0).unwrap();

    let make_hour = |id: &str, text: &str| TreeNode {
        node_id: id.to_string(),
        namespace: ns.to_string(),
        level: NodeLevel::Hour,
        parent_id: derive_parent_id(id),
        summary: text.to_string(),
        token_count: estimate_tokens(text),
        child_count: 0,
        created_at: ts,
        updated_at: ts,
        metadata: None,
    };

    // Seed the tree with hour leaves and an outdated ancestor/root.
    store::write_node(&cfg, &make_hour("2024/03/15/10", "hour ten")).unwrap();
    store::write_node(&cfg, &make_hour("2024/03/15/11", "hour eleven")).unwrap();
    store::write_node(
        &cfg,
        &TreeNode {
            node_id: "2024/03/15".into(),
            namespace: ns.into(),
            level: NodeLevel::Day,
            parent_id: Some("2024/03".into()),
            summary: "stale day".into(),
            token_count: 2,
            child_count: 1,
            created_at: ts,
            updated_at: ts,
            metadata: None,
        },
    )
    .unwrap();
    store::write_node(
        &cfg,
        &TreeNode {
            node_id: "root".into(),
            namespace: ns.into(),
            level: NodeLevel::Root,
            parent_id: None,
            summary: "stale root".into(),
            token_count: 2,
            child_count: 1,
            created_at: ts,
            updated_at: ts,
            metadata: None,
        },
    )
    .unwrap();

    // Preserve unsummarized buffer content across rebuild.
    store::buffer_write(&cfg, ns, "pending buffer item", &ts, None).unwrap();
    let provider = StubProvider::with_reply("rebuilt summary");

    let status = rebuild_tree(&cfg, &provider, ns).await.unwrap();
    assert!(status.total_nodes >= 5, "expected leaf + ancestor chain");

    let restored_buffer = store::buffer_read(&cfg, ns).unwrap();
    assert_eq!(
        restored_buffer.len(),
        1,
        "buffer entries must survive rebuild"
    );

    let day = store::read_node(&cfg, ns, "2024/03/15").unwrap().unwrap();
    assert!(
        day.summary.contains("hour ten") || day.summary.contains("rebuilt summary"),
        "day node should be regenerated from hour leaves"
    );

    let root = store::read_node(&cfg, ns, "root").unwrap().unwrap();
    assert!(
        root.summary.contains("rebuilt summary") || root.summary.contains("hour ten"),
        "root node should be regenerated during rebuild"
    );
}

#[tokio::test]
async fn rebuild_tree_on_empty_namespace_is_noop() {
    let tmp = TempDir::new().unwrap();
    let cfg = test_config(&tmp);
    std::fs::create_dir_all(&cfg.workspace_dir).unwrap();

    let provider = StubProvider::with_reply("unused");
    let status = rebuild_tree(&cfg, &provider, "empty-rebuild")
        .await
        .unwrap();
    assert_eq!(status.total_nodes, 0);
    assert_eq!(status.depth, 0);
}

#[tokio::test]
async fn summarize_to_limit_truncates_overlong_provider_output() {
    let provider = StubProvider::with_reply("x".repeat(MAX_SUMMARY_CHARS + 50));
    let summary = summarize_to_limit(&provider, "short input", 10, "day", "2024/03/15", "m")
        .await
        .unwrap();

    assert_eq!(summary.len(), 40, "max_tokens=10 should clamp to 40 chars");
    assert!(summary.chars().all(|c| c == 'x'));
}

#[test]
fn hour_id_from_buffer_filename_parses_and_rejects_invalid_inputs() {
    let parsed = hour_id_from_buffer_filename("1711958400000_uuid.md").unwrap();
    assert_eq!(parsed, "2024/04/01/08");

    assert!(hour_id_from_buffer_filename("not-a-timestamp.md").is_none());
    assert!(hour_id_from_buffer_filename("abc_123.md").is_none());
}

#[test]
fn derive_node_ids_from_hour_id_falls_back_for_non_hour_ids() {
    let ids = derive_node_ids_from_hour_id("2024/03/15");
    assert_eq!(
        ids,
        (
            "2024/03/15".to_string(),
            "unknown".to_string(),
            "unknown".to_string(),
            "unknown".to_string(),
            "root".to_string(),
        )
    );
}

#[test]
fn discover_active_namespaces_requires_markdown_entries_in_buffer() {
    let tmp = TempDir::new().unwrap();
    let cfg = test_config(&tmp);
    std::fs::create_dir_all(&cfg.workspace_dir).unwrap();

    let base = cfg.workspace_dir.join("memory").join("namespaces");
    std::fs::create_dir_all(base.join("alpha").join("tree").join("buffer")).unwrap();
    std::fs::create_dir_all(base.join("beta").join("tree").join("buffer")).unwrap();
    std::fs::create_dir_all(base.join("gamma").join("tree")).unwrap();

    std::fs::write(
        base.join("alpha")
            .join("tree")
            .join("buffer")
            .join("entry.md"),
        "alpha",
    )
    .unwrap();
    std::fs::write(
        base.join("beta")
            .join("tree")
            .join("buffer")
            .join("entry.txt"),
        "beta",
    )
    .unwrap();

    let active = discover_active_namespaces(&cfg);
    assert_eq!(active, vec!["alpha".to_string()]);
}
