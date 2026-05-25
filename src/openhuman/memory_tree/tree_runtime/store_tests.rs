use super::*;
use tempfile::TempDir;

fn test_config(tmp: &TempDir) -> Config {
    let config = Config {
        workspace_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    config
}

fn make_node(namespace: &str, node_id: &str, summary: &str) -> TreeNode {
    let level = level_from_node_id(node_id);
    TreeNode {
        node_id: node_id.to_string(),
        namespace: namespace.to_string(),
        level,
        parent_id: derive_parent_id(node_id),
        summary: summary.to_string(),
        token_count: estimate_tokens(summary),
        child_count: 0,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        metadata: None,
    }
}

#[test]
fn write_and_read_node_roundtrip() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let ns = "test-ns";

    let node = make_node(ns, "root", "All-time summary of events.");
    write_node(&config, &node).unwrap();

    let read_back = read_node(&config, ns, "root").unwrap().unwrap();
    assert_eq!(read_back.node_id, "root");
    assert_eq!(read_back.level, NodeLevel::Root);
    assert_eq!(read_back.summary, "All-time summary of events.");
    assert!(read_back.parent_id.is_none());
}

#[test]
fn write_and_read_hour_leaf() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let ns = "test-ns";

    let node = make_node(ns, "2024/03/15/14", "Hour 14 summary.");
    write_node(&config, &node).unwrap();

    let read_back = read_node(&config, ns, "2024/03/15/14").unwrap().unwrap();
    assert_eq!(read_back.level, NodeLevel::Hour);
    assert_eq!(read_back.parent_id.as_deref(), Some("2024/03/15"));
    assert_eq!(read_back.summary, "Hour 14 summary.");
}

#[test]
fn read_children_of_day() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let ns = "test-ns";

    // Write some hour leaves
    for hour in [10, 11, 14] {
        let node = make_node(
            ns,
            &format!("2024/03/15/{hour:02}"),
            &format!("Hour {hour}."),
        );
        write_node(&config, &node).unwrap();
    }
    // Write the day summary (should not appear as a child)
    let day = make_node(ns, "2024/03/15", "Day summary.");
    write_node(&config, &day).unwrap();

    let children = read_children(&config, ns, "2024/03/15").unwrap();
    assert_eq!(children.len(), 3);
    assert_eq!(children[0].node_id, "2024/03/15/10");
    assert_eq!(children[1].node_id, "2024/03/15/11");
    assert_eq!(children[2].node_id, "2024/03/15/14");
}

#[test]
fn read_children_of_root() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let ns = "test-ns";

    for year in ["2023", "2024"] {
        let node = make_node(ns, year, &format!("Year {year} summary."));
        write_node(&config, &node).unwrap();
    }

    let children = read_children(&config, ns, "root").unwrap();
    assert_eq!(children.len(), 2);
    assert_eq!(children[0].node_id, "2023");
    assert_eq!(children[1].node_id, "2024");
}

#[test]
fn read_node_missing_returns_none() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    assert!(read_node(&config, "ns", "root").unwrap().is_none());
}

#[test]
fn count_nodes_and_status() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let ns = "test-ns";

    write_node(&config, &make_node(ns, "root", "root")).unwrap();
    write_node(&config, &make_node(ns, "2024", "year")).unwrap();
    write_node(&config, &make_node(ns, "2024/03", "month")).unwrap();
    write_node(&config, &make_node(ns, "2024/03/15", "day")).unwrap();
    write_node(&config, &make_node(ns, "2024/03/15/14", "hour")).unwrap();

    assert_eq!(count_nodes(&config, ns).unwrap(), 5);

    let status = get_tree_status(&config, ns).unwrap();
    assert_eq!(status.total_nodes, 5);
    assert_eq!(status.depth, 5);
}

#[test]
fn delete_tree_removes_all() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let ns = "test-ns";

    write_node(&config, &make_node(ns, "root", "root")).unwrap();
    write_node(&config, &make_node(ns, "2024/03/15/14", "hour")).unwrap();

    let deleted = delete_tree(&config, ns).unwrap();
    assert!(deleted >= 2);
    assert_eq!(count_nodes(&config, ns).unwrap(), 0);
}

#[test]
fn buffer_write_and_drain() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let ns = "test-ns";
    let ts1 = Utc.with_ymd_and_hms(2024, 3, 15, 10, 0, 0).unwrap();
    let ts2 = Utc.with_ymd_and_hms(2024, 3, 15, 11, 0, 0).unwrap();

    buffer_write(&config, ns, "entry one", &ts1, None).unwrap();
    buffer_write(&config, ns, "entry two", &ts2, None).unwrap();

    let drained = buffer_drain(&config, ns).unwrap();
    assert_eq!(drained.len(), 2);
    // Sorted by filename (timestamp prefix), so ts1 < ts2
    assert_eq!(drained[0].1, "entry one");
    assert_eq!(drained[1].1, "entry two");

    // Buffer should be empty now
    let again = buffer_drain(&config, ns).unwrap();
    assert!(again.is_empty());
}

#[test]
fn buffer_write_with_metadata() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let ns = "test-ns";
    let now = Utc::now();

    let meta = serde_json::json!({"source": "test", "priority": 1});
    buffer_write(&config, ns, "entry with meta", &now, Some(&meta)).unwrap();

    let drained = buffer_drain(&config, ns).unwrap();
    assert_eq!(drained.len(), 1);
    // Content should be stripped of frontmatter
    assert_eq!(drained[0].1, "entry with meta");
}

#[test]
fn ancestors_walk_to_root() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let ns = "test-ns";

    write_node(&config, &make_node(ns, "root", "root")).unwrap();
    write_node(&config, &make_node(ns, "2024", "year")).unwrap();
    write_node(&config, &make_node(ns, "2024/03", "month")).unwrap();
    write_node(&config, &make_node(ns, "2024/03/15", "day")).unwrap();

    let ancestors = read_ancestors(&config, ns, "2024/03/15/14").unwrap();
    let ids: Vec<&str> = ancestors.iter().map(|n| n.node_id.as_str()).collect();
    assert_eq!(ids, vec!["2024/03/15", "2024/03", "2024", "root"]);
}

#[test]
fn frontmatter_parsing() {
    let raw = "---\nnode_id: \"root\"\nlevel: root\ntoken_count: 42\n---\n\nHello world.";
    let (fm, body) = split_frontmatter(raw);
    assert_eq!(fm.get("level").unwrap(), "root");
    assert_eq!(fm.get("token_count").unwrap(), "42");
    assert_eq!(body, "Hello world.");
}

#[test]
fn validate_node_id_accepts_valid() {
    assert!(validate_node_id("root").is_ok());
    assert!(validate_node_id("2024").is_ok());
    assert!(validate_node_id("2024/03").is_ok());
    assert!(validate_node_id("2024/03/15").is_ok());
    assert!(validate_node_id("2024/03/15/14").is_ok());
}

#[test]
fn validate_node_id_rejects_traversal() {
    assert!(validate_node_id("..").is_err());
    assert!(validate_node_id("../etc").is_err());
    assert!(validate_node_id("2024/../etc").is_err());
    assert!(validate_node_id("/2024").is_err());
    assert!(validate_node_id("2024/").is_err());
}

#[test]
fn validate_node_id_rejects_non_numeric() {
    assert!(validate_node_id("abc").is_err());
    assert!(validate_node_id("2024/abc").is_err());
    assert!(validate_node_id("2024/03/15/foo").is_err());
}

#[test]
fn validate_node_id_rejects_out_of_range() {
    assert!(validate_node_id("2024/13").is_err()); // month 13
    assert!(validate_node_id("2024/03/32").is_err()); // day 32
    assert!(validate_node_id("2024/03/15/24").is_err()); // hour 24
}

#[test]
fn validate_namespace_rejects_dangerous() {
    assert!(validate_namespace("").is_err());
    assert!(validate_namespace("  ").is_err());
    assert!(validate_namespace("../etc").is_err());
    assert!(validate_namespace("/absolute").is_err());
}

#[test]
fn validate_namespace_accepts_valid() {
    assert!(validate_namespace("my-namespace").is_ok());
    assert!(validate_namespace("skill:gmail:user@example.com").is_ok());
}

#[test]
fn list_namespaces_with_root_returns_only_summarised() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    // ns_a has a root node — should be returned.
    write_node(&config, &make_node("ns_a", "root", "alpha summary")).unwrap();
    // ns_b has only an hour leaf, no root — should be filtered out.
    write_node(&config, &make_node("ns_b", "2024/03/15/14", "hour")).unwrap();
    // ns_c has a root.
    write_node(&config, &make_node("ns_c", "root", "gamma summary")).unwrap();

    let listed = list_namespaces_with_root(&config).unwrap();
    // Sorted alphabetically for cache stability — see fn docs.
    assert_eq!(listed, vec!["ns_a".to_string(), "ns_c".to_string()]);
}

#[test]
fn collect_root_summaries_respects_per_namespace_cap() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let big = "x".repeat(50);
    write_node(&config, &make_node("ns", "root", &big)).unwrap();

    // Per-namespace cap of 10 should clip the body.
    let result = collect_root_summaries_with_caps(&config.workspace_dir, 10, 10_000);
    assert_eq!(result.len(), 1);
    let (ns, body) = &result[0];
    assert_eq!(ns, "ns");
    assert!(
        body.starts_with("xxxxxxxxxx"),
        "expected the first 10 x's, got: {body}"
    );
    assert!(body.contains("[... truncated]"));
}

#[test]
fn collect_root_summaries_stops_at_total_cap() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    write_node(&config, &make_node("aaa", "root", "first")).unwrap();
    write_node(&config, &make_node("bbb", "root", "second")).unwrap();
    write_node(&config, &make_node("ccc", "root", "third")).unwrap();

    // Total cap of 5 chars — should accept aaa ("first" = 5),
    // then break before reading bbb because total >= cap.
    let result = collect_root_summaries_with_caps(&config.workspace_dir, 100, 5);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].0, "aaa");
}

#[test]
fn collect_root_summaries_returns_empty_for_unknown_workspace() {
    let tmp = TempDir::new().unwrap();
    let result = collect_root_summaries_with_caps(&tmp.path().join("nope"), 100, 1000);
    assert!(result.is_empty());
}
