#[test]
fn all_topologies_names_do_not_collide_with_production_tools() {
    let tmp = tempfile::tempdir().expect("create tempdir");
    let config = std::sync::Arc::new(crate::config::Config::default());
    let security = std::sync::Arc::new(crate::security::SecurityPolicy::default());
    let browser = crate::config::BrowserConfig::default();
    let http = crate::config::HttpRequestConfig::default();
    let agents = std::collections::HashMap::new();

    let tools = crate::tools::ops::all_tools(
        config.clone(),
        &security,
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

    let reports = tinyagents_orchestration::topology::all_graph_topologies();
    for report in &reports {
        assert!(
            !tool_names.contains(report.name),
            "graph name '{}' collides with registered tool of the same name",
            report.name
        );
    }
}
