use super::*;

#[test]
fn catalog_mirrors_builtins() {
    use crate::agent_registry::agents::BUILTINS;

    let resources = resource_specs();
    for b in BUILTINS {
        let expected_uri = format!("openhuman://prompts/agents/{}", b.id);
        assert!(
            resources.iter().any(|r| r.uri == expected_uri),
            "RESOURCE_CATALOG is missing an entry for built-in agent `{}` \
             (expected URI `{}`). Add it to RESOURCE_CATALOG in resources.rs.",
            b.id,
            expected_uri
        );
    }

    let catalog_agent_count = resources
        .iter()
        .filter(|r| r.uri.starts_with("openhuman://prompts/agents/"))
        .count();
    assert_eq!(
        catalog_agent_count,
        BUILTINS.len(),
        "resource_specs has {catalog_agent_count} agent entries but BUILTINS has {}. \
         Remove stale entries from RESOURCE_CATALOG.",
        BUILTINS.len()
    );
}

#[test]
fn list_resources_returns_all_catalog_entries() {
    let resources = resource_specs();
    let expected_len = RESOURCE_CATALOG.len() + if cfg!(feature = "flows") { 2 } else { 0 };
    assert_eq!(
        resources.len(),
        expected_len,
        "resources/list includes static assets and the bus-backed flow prompts"
    );
    for entry in &resources {
        assert!(!entry.uri.is_empty(), "uri must be set");
        assert!(!entry.name.is_empty(), "name must be set");
        assert_eq!(entry.mime_type.as_deref(), Some("text/markdown"));
    }
}

#[cfg(feature = "flows")]
#[tokio::test]
async fn flow_prompt_resource_fails_closed_when_tinyflows_is_disabled() {
    let mut config = crate::config::Config::default();
    config.components.tinyflows = false;
    let ctx = crate::core::runtime::DispatchCtx::new(
        crate::core::runtime::CoreContext::for_test_with_config(
            crate::core::runtime::DomainSet::full(),
            config,
        ),
    );
    let error = read_resource(&ctx, "openhuman://prompts/agents/flow_discovery")
        .await
        .expect_err("disabled TinyFlows must not expose a copied prompt");
    assert_eq!(error.code(), -32603);
    assert!(error.message().contains("unavailable"));
}

#[test]
fn list_resources_includes_core_and_agent_uris() {
    let resources = resource_specs();
    let uris: Vec<&str> = resources.iter().map(|r| r.uri.as_str()).collect();
    for expected in [
        "openhuman://prompts/identity",
        "openhuman://prompts/soul",
        "openhuman://prompts/user",
        "openhuman://prompts/agents/orchestrator",
    ] {
        assert!(uris.contains(&expected), "missing URI {expected}");
    }
    assert!(
        !uris.contains(&"openhuman://prompts/agents/planner"),
        "retired planner prompt must not be listed as an agent resource"
    );
    for flow_uri in [
        "openhuman://prompts/agents/workflow_builder",
        "openhuman://prompts/agents/flow_discovery",
    ] {
        assert_eq!(
            uris.contains(&flow_uri),
            cfg!(feature = "flows"),
            "flow prompt resource feature gate mismatch for {flow_uri}"
        );
    }
}

#[test]
fn read_resource_returns_content_for_known_uri() {
    let result = read_static_resource("openhuman://prompts/identity").expect("should succeed");
    let contents = result["contents"].as_array().expect("contents array");
    assert_eq!(contents.len(), 1);
    assert_eq!(contents[0]["uri"], "openhuman://prompts/identity");
    assert_eq!(contents[0]["mimeType"], "text/markdown");
    assert!(!contents[0]["text"].as_str().unwrap_or("").is_empty());
}

#[test]
fn read_resource_returns_minus_32002_for_unknown_uri() {
    let err = read_static_resource("openhuman://prompts/agents/nonexistent")
        .expect_err("should fail for unknown URI");
    assert_eq!(err.code(), -32002);
    assert!(err.message().contains("nonexistent"));
}

#[tokio::test]
async fn read_resource_returns_content_for_each_subagent() {
    use crate::agent_registry::agents::BUILTINS;
    let config = crate::config::Config::default();
    let ctx = crate::core::runtime::DispatchCtx::new(
        crate::core::runtime::CoreContext::for_test_with_config(
            crate::core::runtime::DomainSet::full(),
            config,
        ),
    );
    for b in BUILTINS {
        let uri = format!("openhuman://prompts/agents/{}", b.id);
        let result = read_resource(&ctx, &uri)
            .await
            .unwrap_or_else(|_| panic!("read_resource failed for agent `{}`", b.id));
        let text = result["contents"][0]["text"].as_str().unwrap_or("");
        assert!(
            !text.is_empty(),
            "prompt content is empty for agent `{}`",
            b.id
        );
    }
}

#[test]
fn all_catalog_uris_are_unique() {
    let resources = resource_specs();
    let mut uris: Vec<&str> = resources.iter().map(|r| r.uri.as_str()).collect();
    let original_len = uris.len();
    uris.sort_unstable();
    uris.dedup();
    let deduped_len = uris.len();
    assert_eq!(
        original_len, deduped_len,
        "RESOURCE_CATALOG contains duplicate URIs"
    );
}
