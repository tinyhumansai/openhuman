use super::*;

#[test]
fn all_topologies_includes_the_member_graph() {
    let reports = all_graph_topologies();
    let member = reports
        .iter()
        .find(|r| r.name == "agent_teams:member")
        .expect("the agent_teams member graph should be exported");

    // The member graph is a fixed, well-formed structure.
    assert!(
        member.ok,
        "member graph should validate structurally: {:?}",
        member.errors
    );
    assert!(member.errors.is_empty());
}

#[test]
fn all_topologies_includes_delegation_and_workflow_scheduler() {
    let reports = all_graph_topologies();
    for name in [
        "delegation",
        "workflow_runs:scheduler",
        "spawn_parallel_graph",
    ] {
        let report = reports
            .iter()
            .find(|r| r.name == name)
            .unwrap_or_else(|| panic!("the {name} graph should be exported"));
        assert!(
            report.ok,
            "{name} graph should validate structurally: {:?}",
            report.errors
        );
        assert!(
            report.mermaid.contains("flowchart"),
            "{name} mermaid should render: {}",
            report.mermaid
        );
    }
}

#[test]
fn all_topologies_names_do_not_collide_with_production_tools() {
    let tmp = tempfile::tempdir().expect("create tempdir");
    let config = std::sync::Arc::new(crate::openhuman::config::Config::default());
    let security = std::sync::Arc::new(crate::openhuman::security::SecurityPolicy::default());
    let audit = std::sync::Arc::new(
        crate::openhuman::security::AuditLogger::new(
            crate::openhuman::config::AuditConfig {
                enabled: false,
                log_path: "audit.log".into(),
                max_size_mb: 10,
            },
            tmp.path().to_path_buf(),
        )
        .expect("create audit logger"),
    );
    let browser = crate::openhuman::config::BrowserConfig::default();
    let http = crate::openhuman::config::HttpRequestConfig::default();
    let agents = std::collections::HashMap::new();

    let tools = crate::openhuman::tools::all_tools(
        config.clone(),
        &security,
        audit,
        &browser,
        &http,
        tmp.path(),
        &agents,
        &config,
    );
    let tool_names: std::collections::HashSet<_> = tools.iter().map(|t| t.name()).collect();

    // Verify runtime tool `spawn_parallel_agents` is present in the checked tool set
    assert!(
        tool_names.contains("spawn_parallel_agents"),
        "expected spawn_parallel_agents in production all_tools registration"
    );

    let reports = all_graph_topologies();
    for report in &reports {
        assert!(
            !tool_names.contains(report.name),
            "graph name '{}' collides with registered tool of the same name",
            report.name
        );
    }
}

#[test]
fn delegation_topology_names_the_revision_loop_nodes() {
    let t = super::super::delegation::delegation_graph_topology().expect("builds");
    let names: Vec<&str> = t.nodes.iter().map(|n| n.id.as_str()).collect();
    for expected in ["plan", "execute", "review", "finalize"] {
        assert!(
            names.contains(&expected),
            "missing node {expected}: {names:?}"
        );
    }
}

#[test]
fn member_report_renders_mermaid_and_valid_json() {
    let t = crate::openhuman::agent::orchestration::agent_teams::member_graph_topology()
        .expect("member topology builds");
    let report = describe("agent_teams:member", &t);

    // Mermaid is a flowchart with at least the entry node rendered.
    assert!(
        report.mermaid.contains("flowchart"),
        "mermaid should be a flowchart: {}",
        report.mermaid
    );
    assert!(!t.nodes.is_empty(), "the graph should declare nodes");

    // JSON round-trips to a value carrying the same node set.
    let parsed: serde_json::Value =
        serde_json::from_str(&report.json).expect("topology JSON parses");
    assert!(
        parsed.get("nodes").is_some(),
        "serialized topology should carry its nodes: {}",
        report.json
    );
}
