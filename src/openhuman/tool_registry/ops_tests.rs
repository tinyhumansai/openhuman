use super::*;
use crate::core::{FieldSchema, TypeSchema};
use crate::openhuman::config::schema::{
    CapabilityProviderConfig, CapabilityProviderTrustState, Config,
};

#[test]
fn registry_entries_include_mcp_and_controller_tools() {
    let entries = registry_entries();

    let memory_search = entries
        .iter()
        .find(|entry| entry.tool_id == "memory.search")
        .expect("memory.search mcp tool");
    assert_eq!(memory_search.transport, ToolRegistryTransport::McpStdio);
    assert_eq!(memory_search.route["method"], json!("tools/call"));
    assert_eq!(memory_search.health, ToolRegistryHealth::Available);

    let web_search = entries
        .iter()
        .find(|entry| entry.tool_id == "tools.web_search")
        .expect("tools.web_search controller tool");
    assert_eq!(web_search.transport, ToolRegistryTransport::JsonRpc);
    assert_eq!(
        web_search.route["method"],
        json!("openhuman.tools_web_search")
    );
    assert_eq!(web_search.input_schema["type"], json!("object"));
}

#[test]
fn registry_entries_are_unique_and_sorted_by_tool_id() {
    let entries = registry_entries();
    let ids = entries
        .iter()
        .map(|entry| entry.tool_id.as_str())
        .collect::<Vec<_>>();
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    sorted.dedup();

    assert_eq!(ids, sorted);
}

#[test]
fn diagnostics_reports_inventory_and_policy_surfaces() {
    let outcome = diagnostics_for_config(&Config::default());

    assert!(outcome.value.total_tools > 0);
    assert_eq!(outcome.value.total_tools, outcome.value.enabled_tools);
    assert!(outcome.value.mcp_stdio_tools > 0);
    assert!(outcome.value.json_rpc_tools > 0);
    assert!(outcome
        .value
        .policy_surfaces
        .iter()
        .any(|tool_id| tool_id == "security.policy_info"));
    assert!(outcome
        .value
        .possible_write_surfaces
        .iter()
        .any(|tool_id| tool_id == "tools.composio_execute"));

    assert!(!outcome.value.posture.autonomy_level.is_empty());
    assert!(outcome.value.mcp_write_audit.enabled);
}

#[tokio::test]
async fn diagnostics_loads_active_capability_provider_config() {
    let _lock = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let tmp = tempfile::tempdir().expect("tempdir");
    let _env = EnvRestore::set_path("OPENHUMAN_WORKSPACE", tmp.path());
    std::fs::write(
        tmp.path().join("config.toml"),
        r#"
[[capability_providers]]
id = "Runtime Provider"
display_name = "Runtime Provider"
trust_state = "trusted"
enabled = true
"#,
    )
    .expect("write config");

    let outcome = diagnostics().await.expect("diagnostics");

    assert_eq!(outcome.value.capability_providers.total_providers, 1);
    assert_eq!(outcome.value.capability_providers.enabled_providers, 1);
    assert_eq!(outcome.value.capability_providers.trusted_providers, 1);
    assert_eq!(
        outcome.value.capability_providers.trusted_enabled_providers,
        1
    );
    assert!(outcome
        .value
        .capability_providers
        .registry_errors
        .is_empty());
}

#[test]
fn diagnostics_for_config_reports_capability_provider_summary() {
    let config = Config {
        capability_providers: vec![
            capability_provider(
                "trusted-enabled",
                CapabilityProviderTrustState::Trusted,
                true,
            ),
            capability_provider(
                "trusted-disabled",
                CapabilityProviderTrustState::Trusted,
                false,
            ),
            capability_provider(
                "untrusted-enabled",
                CapabilityProviderTrustState::Untrusted,
                true,
            ),
        ],
        ..Config::default()
    };

    let outcome = diagnostics_for_config(&config);

    assert_eq!(outcome.value.capability_providers.total_providers, 3);
    assert_eq!(outcome.value.capability_providers.enabled_providers, 2);
    assert_eq!(outcome.value.capability_providers.trusted_providers, 2);
    assert_eq!(
        outcome.value.capability_providers.trusted_enabled_providers,
        1
    );
    assert!(outcome
        .value
        .capability_providers
        .registry_errors
        .is_empty());
}

#[test]
fn diagnostics_for_config_reports_capability_provider_errors() {
    let config = Config {
        capability_providers: vec![
            capability_provider("Acme Tools", CapabilityProviderTrustState::Trusted, true),
            capability_provider("acme-tools", CapabilityProviderTrustState::Trusted, true),
        ],
        ..Config::default()
    };

    let outcome = diagnostics_for_config(&config);

    assert_eq!(outcome.value.capability_providers.total_providers, 2);
    assert_eq!(outcome.value.capability_providers.enabled_providers, 0);
    assert!(outcome.value.capability_providers.registry_errors[0].contains("duplicate"));
    assert!(outcome.value.capability_providers.registry_errors[0].contains("acme-tools"));
}

#[test]
fn looks_write_capable_detects_action_prefixes_and_suffixes() {
    assert!(looks_write_capable("user.create"));
    assert!(looks_write_capable("create.user"));
    assert!(looks_write_capable("tools.composio_execute"));
    assert!(!looks_write_capable("tools.search"));
}

#[test]
fn is_policy_surface_includes_policy_namespaces() {
    assert!(is_policy_surface("security.audit_status"));
    assert!(is_policy_surface("approval.request"));
    assert!(is_policy_surface("tool_registry.diagnostics"));
    assert!(!is_policy_surface("tools.web_search"));
}

#[test]
fn insert_registry_entry_skips_duplicate_tool_id() {
    let mut entries = BTreeMap::new();
    let first_entry = ToolRegistryEntry {
        tool_id: "duplicate.tool".to_string(),
        name: "duplicate.tool".to_string(),
        title: "First Entry".to_string(),
        description: "First description.".to_string(),
        version: REGISTRY_ENTRY_VERSION.to_string(),
        transport: ToolRegistryTransport::JsonRpc,
        route: json!({}),
        input_schema: json!({}),
        output_schema: json!({}),
        allowed_agents: vec!["*".to_string()],
        tags: vec!["test".to_string()],
        enabled: true,
        health: ToolRegistryHealth::Available,
    };
    let second_entry = ToolRegistryEntry {
        title: "Second Entry".to_string(),
        description: "Second description.".to_string(),
        ..first_entry.clone()
    };

    insert_registry_entry(&mut entries, first_entry, "first");
    // Should not panic; first entry is kept, second is silently dropped.
    insert_registry_entry(&mut entries, second_entry, "second");

    assert_eq!(entries.len(), 1);
    assert_eq!(entries["duplicate.tool"].title, "First Entry");
}

#[test]
fn get_tool_trims_and_returns_exact_entry() {
    let outcome = get_tool("  memory.search  ").expect("registry lookup");
    assert_eq!(outcome.value.tool_id, "memory.search");
}

#[test]
fn get_tool_rejects_blank_id() {
    let err = get_tool("  ").expect_err("blank id should fail");
    assert!(err.contains("non-empty"));
}

#[test]
fn get_tool_reports_unknown_id() {
    let err = get_tool("missing.tool").expect_err("unknown id should fail");
    assert!(err.contains("missing.tool"));
}

#[test]
fn all_registry_entries_have_non_empty_name_and_description() {
    let entries = registry_entries();
    assert!(
        !entries.is_empty(),
        "registry must contain at least one tool"
    );
    let mut violations: Vec<String> = Vec::new();
    for entry in &entries {
        if entry.name.trim().is_empty() {
            violations.push(format!("tool_id='{}' has empty name", entry.tool_id));
        }
        if entry.description.trim().is_empty() {
            violations.push(format!("tool_id='{}' has empty description", entry.tool_id));
        }
    }
    assert!(
        violations.is_empty(),
        "registry integrity violations:\n{}",
        violations.join("\n")
    );
}

#[test]
fn controller_json_schema_marks_required_and_optional_fields() {
    let schema = schema_fields_to_json_schema(&[
        FieldSchema {
            name: "query",
            ty: TypeSchema::String,
            comment: "Query text.",
            required: true,
        },
        FieldSchema {
            name: "max_results",
            ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
            comment: "Optional cap.",
            required: false,
        },
    ]);

    assert_eq!(schema["required"], json!(["query"]));
    assert_eq!(schema["properties"]["query"]["type"], json!("string"));
    assert_eq!(
        schema["properties"]["max_results"]["anyOf"][0]["type"],
        json!("integer")
    );
    assert_eq!(
        schema["properties"]["max_results"]["description"],
        json!("Optional cap.")
    );
}

fn capability_provider(
    id: &str,
    trust_state: CapabilityProviderTrustState,
    enabled: bool,
) -> CapabilityProviderConfig {
    CapabilityProviderConfig {
        id: id.to_string(),
        display_name: id.to_string(),
        source_uri: None,
        source_digest: None,
        trust_state,
        enabled,
    }
}

struct EnvRestore {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
}

impl EnvRestore {
    fn set_path(key: &'static str, value: &std::path::Path) -> Self {
        let previous = std::env::var_os(key);
        std::env::set_var(key, value);
        Self { key, previous }
    }
}

impl Drop for EnvRestore {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}
