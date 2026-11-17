use super::*;

use crate::skills::catalog::test_fixtures::{hermes_item, Fixture};

const ENV: [&str; 3] = [
    "OPENHUMAN_SKILL_REGISTRY_CATALOG_URL",
    "OPENHUMAN_SKILL_REGISTRY_CACHE_DIR",
    "OPENHUMAN_SKILL_INSTALL_ALLOW_LOCAL_HTTP",
];

#[test]
fn install_tool_is_external_effect_so_it_routes_through_approval_gate() {
    let tool = SkillRegistryInstallTool::new(Arc::new(Config::default()));
    assert_eq!(tool.name(), "skill_registry_install");
    assert!(
        tool.external_effect(),
        "skill_registry_install must declare external_effect so the harness gates it"
    );
    assert!(matches!(tool.permission_level(), PermissionLevel::Write));
}

#[test]
fn read_only_skill_tools_are_not_gated() {
    assert!(!SkillRegistryBrowseTool.external_effect());
    assert!(!SkillRegistrySearchTool.external_effect());
    assert!(!SkillRegistrySourcesTool.external_effect());
}

#[test]
fn paging_arguments_default_clamp_and_accept_an_offset() {
    let defaults = paged_query(&json!({}), "q");
    assert_eq!(defaults.page, Some(1));
    assert_eq!(defaults.page_size, Some(PAGE_DEFAULT_LIMIT));

    let clamped = paged_query(&json!({ "limit": 0, "page": 0 }), "q");
    assert_eq!(clamped.page_size, Some(1));
    assert_eq!(clamped.page, Some(1));

    let capped = paged_query(&json!({ "limit": 10_000 }), "q");
    assert_eq!(capped.page_size, Some(PAGE_MAX_LIMIT));

    let by_offset = paged_query(&json!({ "offset": 40, "limit": 10 }), "q");
    assert_eq!(by_offset.page, Some(5));

    let filtered = paged_query(&json!({ "source": " ClawHub ", "category": "" }), "q");
    assert_eq!(filtered.upstreams, ["ClawHub"]);
    assert!(filtered.categories.is_empty());
}

#[test]
fn registry_errors_carry_the_tool_status_markers() {
    let missing = registry_tool_error(
        "install skill 'x'",
        &tinyskills::RegistryError::NotFound {
            id: "x".into(),
            closest: Vec::new(),
        },
    );
    assert!(
        missing.output().starts_with(NOT_FOUND_MARKER),
        "{}",
        missing.output()
    );
    assert!(missing.output().contains("SKILL_REGISTRY_NOT_FOUND: "));

    let portal = registry_tool_error(
        "install skill 'x'",
        &tinyskills::RegistryError::NoDirectDownload {
            name: "x".into(),
            source_url: None,
        },
    );
    assert!(
        portal.output().starts_with(UNSUPPORTED_MARKER),
        "{}",
        portal.output()
    );

    let outage = registry_tool_error(
        "browse skill catalog",
        &tinyskills::RegistryError::Unavailable { status: 503 },
    );
    assert!(outage
        .output()
        .starts_with("Failed to browse skill catalog: SKILL_REGISTRY_UNAVAILABLE: "));
}

async fn run(tool: &dyn Tool, args: serde_json::Value) -> serde_json::Value {
    let result = tool.execute(args).await.expect("execute");
    serde_json::from_str(&result.output()).expect("tool result is json")
}

#[tokio::test]
async fn browse_and_search_return_bounded_pages_with_their_freshness() {
    let _env = crate::skills::catalog::TEST_ENV_LOCK.lock().await;
    let items = (0..45)
        .map(|i| hermes_item(&format!("review-{i:02}"), "built-in"))
        .collect();
    let fixture = Fixture::start(items).await;
    let cache = tempfile::tempdir().unwrap();
    std::env::set_var(ENV[0], format!("{}/skills.json", fixture.base));
    std::env::set_var(ENV[1], cache.path());
    std::env::set_var(ENV[2], "1");

    let first = run(&SkillRegistrySearchTool, json!({ "query": "review" })).await;
    assert_eq!(first["total"], 45);
    assert_eq!(first["page"], 1);
    assert_eq!(first["count"], PAGE_DEFAULT_LIMIT);
    assert_eq!(first["next_page"], 2);
    assert_eq!(first["freshness"], "live");
    assert!(first["last_error"].is_null());

    let last = run(
        &SkillRegistrySearchTool,
        json!({ "query": "review", "page": 5, "limit": 10 }),
    )
    .await;
    assert_eq!(last["count"], 5);
    assert_eq!(last["entries"][0]["id"], "review-40");
    assert!(last["next_page"].is_null(), "no page after the last one");

    let browse = run(&SkillRegistryBrowseTool, json!({})).await;
    assert_eq!(browse["count"], PAGE_DEFAULT_LIMIT);
    assert_eq!(browse["total_pages"], 3);

    let sources = run(&SkillRegistrySourcesTool, json!({})).await;
    assert_eq!(sources["sources"][0], "built-in");
    assert_eq!(sources["counts"][0]["count"], 45);

    for name in ENV {
        std::env::remove_var(name);
    }
}

#[test]
fn install_tool_schema_has_no_scan_acknowledgement() {
    let schema = SkillRegistryInstallTool::new(Arc::new(Config::default())).parameters_schema();
    let properties = schema["properties"].as_object().expect("properties");
    assert_eq!(properties.keys().collect::<Vec<_>>(), ["entry_id"]);
    assert!(!schema.to_string().contains("acknowledge"));
    assert!(!schema.to_string().contains("digest"));
}

#[tokio::test]
async fn the_install_tool_cannot_acknowledge_scan_findings() {
    let _env = crate::skills::catalog::TEST_ENV_LOCK.lock().await;
    let fixture = Fixture::start(vec![hermes_item("agent-poisoned", "built-in")]).await;
    fixture
        .blocked_documents
        .store(usize::MAX, std::sync::atomic::Ordering::SeqCst);
    let cache = tempfile::tempdir().unwrap();
    std::env::set_var(ENV[0], format!("{}/skills.json", fixture.base));
    std::env::set_var(ENV[1], cache.path());
    std::env::set_var(ENV[2], "1");
    std::env::set_var(
        crate::skills::catalog::registry::DOWNLOAD_BASE_URL_ENV,
        format!("{}/skills", fixture.base),
    );

    let workspace = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.workspace_dir = workspace.path().to_path_buf();
    let seen = match ops::install_from_catalog(
        workspace.path(),
        "agent-poisoned",
        crate::skills::ops_install::ScanAcknowledgement::Absent,
    )
    .await
    .expect("install")
    {
        crate::skills::ops_install::SkillInstallOutcome::ScanBlocked(blocked) => blocked.digest,
        other => panic!("expected scan_blocked, got {other:?}"),
    };
    let tool = SkillRegistryInstallTool::new(Arc::new(config));
    let result = tool
        .execute(json!({
            "entry_id": "agent-poisoned",
            "acknowledged_digest": seen,
            "acknowledge_scan_findings": true,
        }))
        .await
        .expect("execute");

    for name in ENV {
        std::env::remove_var(name);
    }
    std::env::remove_var(crate::skills::catalog::registry::DOWNLOAD_BASE_URL_ENV);

    assert!(result.is_error, "a blocked install is not a success");
    let body: serde_json::Value = serde_json::from_str(&result.output()).expect("json");
    assert_eq!(body["status"], "scan_blocked");
    assert_eq!(body["target"], "agent-poisoned");
    assert!(
        body.get("digest").is_none(),
        "the agent is never handed the digest"
    );
    assert_eq!(body["findings"][0]["check"], "invisible_code_points");
    assert!(body["instruction"]
        .as_str()
        .unwrap()
        .contains("install it themselves"));
    assert_eq!(
        fixture
            .document_hits
            .load(std::sync::atomic::Ordering::SeqCst),
        4,
        "two scans for the direct call, two for the tool call"
    );
    assert!(!dirs::home_dir()
        .unwrap()
        .join(".openhuman/skills/agent-poisoned")
        .exists());
}
