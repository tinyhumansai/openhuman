use super::*;

#[test]
fn catalog_mirrors_builtins() {
    use crate::openhuman::agent::registry::agents::BUILTINS;

    for b in BUILTINS {
        let expected_uri = format!("openhuman://prompts/agents/{}", b.id);
        assert!(
            RESOURCE_CATALOG.iter().any(|r| r.uri == expected_uri),
            "RESOURCE_CATALOG is missing an entry for built-in agent `{}` \
             (expected URI `{}`). Add it to RESOURCE_CATALOG in resources.rs.",
            b.id,
            expected_uri
        );
    }

    let catalog_agent_count = RESOURCE_CATALOG
        .iter()
        .filter(|r| r.uri.starts_with("openhuman://prompts/agents/"))
        .count();
    assert_eq!(
        catalog_agent_count,
        BUILTINS.len(),
        "RESOURCE_CATALOG has {catalog_agent_count} agent entries but BUILTINS has {}. \
         Remove stale entries from RESOURCE_CATALOG.",
        BUILTINS.len()
    );
}

#[test]
fn list_resources_returns_all_catalog_entries() {
    let result = list_resources_result();
    let resources = result["resources"].as_array().expect("resources array");
    assert_eq!(
        resources.len(),
        RESOURCE_CATALOG.len(),
        "resources/list count mismatch"
    );
    // Every entry has required fields
    for entry in resources {
        assert!(entry["uri"].is_string(), "uri must be string");
        assert!(entry["name"].is_string(), "name must be string");
        assert_eq!(entry["mimeType"], "text/markdown");
    }
}

#[test]
fn list_resources_includes_core_and_agent_uris() {
    let result = list_resources_result();
    let uris: Vec<&str> = result["resources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["uri"].as_str().unwrap())
        .collect();
    for expected in [
        "openhuman://prompts/identity",
        "openhuman://prompts/soul",
        "openhuman://prompts/user",
        "openhuman://prompts/agents/orchestrator",
        "openhuman://prompts/agents/mcp_setup",
    ] {
        assert!(uris.contains(&expected), "missing URI {expected}");
    }
}

#[test]
fn read_resource_returns_content_for_known_uri() {
    let params = json!({ "uri": "openhuman://prompts/identity" });
    let result = read_resource_result(&params).expect("should succeed");
    let contents = result["contents"].as_array().expect("contents array");
    assert_eq!(contents.len(), 1);
    assert_eq!(contents[0]["uri"], "openhuman://prompts/identity");
    assert_eq!(contents[0]["mimeType"], "text/markdown");
    assert!(!contents[0]["text"].as_str().unwrap_or("").is_empty());
}

#[test]
fn read_resource_returns_minus_32002_for_unknown_uri() {
    let params = json!({ "uri": "openhuman://prompts/agents/nonexistent" });
    let err = read_resource_result(&params).expect_err("should fail for unknown URI");
    assert_eq!(err.0, -32002);
    assert!(err.2.contains("nonexistent"));
}

#[test]
fn read_resource_returns_minus_32602_for_missing_uri() {
    let params = json!({});
    let err = read_resource_result(&params).expect_err("should fail without uri");
    assert_eq!(err.0, -32602);
}

#[test]
fn read_resource_returns_content_for_each_subagent() {
    use crate::openhuman::agent::registry::agents::BUILTINS;
    for b in BUILTINS {
        let uri = format!("openhuman://prompts/agents/{}", b.id);
        let params = json!({ "uri": uri });
        let result = read_resource_result(&params)
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
fn list_resource_templates_returns_empty_array() {
    let result = list_resource_templates_result();
    let templates = result["resourceTemplates"]
        .as_array()
        .expect("resourceTemplates must be a JSON array");
    assert!(
        templates.is_empty(),
        "resources/templates/list must return an empty array — the catalog is static"
    );
}

#[test]
fn all_catalog_uris_are_unique() {
    let mut uris: Vec<&str> = RESOURCE_CATALOG.iter().map(|r| r.uri).collect();
    let original_len = uris.len();
    uris.sort_unstable();
    uris.dedup();
    let deduped_len = uris.len();
    assert_eq!(
        original_len, deduped_len,
        "RESOURCE_CATALOG contains duplicate URIs"
    );
}
