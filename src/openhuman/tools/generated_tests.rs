use super::*;
use serde_json::json;

struct EchoAdapter;

#[async_trait]
impl GeneratedToolAdapter for EchoAdapter {
    fn id(&self) -> &str {
        "echo-adapter"
    }

    async fn execute(
        &self,
        definition: &GeneratedToolDefinition,
        args: Value,
    ) -> anyhow::Result<ToolResult> {
        Ok(ToolResult::success(
            json!({
                "tool": definition.name,
                "adapter": definition.adapter_id,
                "args": args,
            })
            .to_string(),
        ))
    }
}

fn sample_definition() -> GeneratedToolDefinition {
    let mut definition = GeneratedToolDefinition::new(
        "send_update",
        "Send a scoped update through a trusted adapter.",
        json!({
            "type": "object",
            "properties": {
                "message": { "type": "string" }
            },
            "required": ["message"]
        }),
        "echo-adapter",
    );
    definition.permission_level = PermissionLevel::Write;
    definition.provider_id = Some("trusted.runtime".into());
    definition.capability_id = Some("updates.send".into());
    definition.source_digest = Some("sha256:abc".into());
    definition.risk = Some(GeneratedToolRisk::ExternalWrite);
    definition
}

fn admission_config() -> GeneratedToolAdmissionConfig {
    GeneratedToolAdmissionConfig {
        enforce_provenance: true,
        trusted_providers: BTreeSet::from(["trusted.runtime".to_string()]),
        ..Default::default()
    }
}

#[tokio::test]
async fn generated_tool_executes_through_adapter() {
    let tool = GeneratedTool::new(sample_definition(), Arc::new(EchoAdapter)).unwrap();

    let result = tool
        .execute(json!({ "message": "hello" }))
        .await
        .expect("execute");

    assert_eq!(tool.name(), "send_update");
    assert_eq!(tool.permission_level(), PermissionLevel::Write);
    assert_eq!(tool.category(), ToolCategory::Skill);
    assert!(result.output().contains("send_update"));
    assert!(result.output().contains("hello"));
}

#[test]
fn generated_tools_from_definitions_returns_tool_objects() {
    let tools =
        generated_tools_from_definitions(vec![sample_definition()], Arc::new(EchoAdapter)).unwrap();

    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name(), "send_update");
    assert_eq!(tools[0].parameters_schema()["type"], json!("object"));
}

#[test]
fn generated_tool_rejects_adapter_mismatch() {
    let mut definition = sample_definition();
    definition.adapter_id = "missing-adapter".into();

    match GeneratedTool::new(definition, Arc::new(EchoAdapter)) {
        Ok(_) => panic!("adapter mismatch should fail"),
        Err(err) => assert!(err.to_string().contains("requires adapter")),
    }
}

#[test]
fn generated_tool_rejects_blank_adapter_id() {
    let mut definition = sample_definition();
    definition.adapter_id = "  ".into();

    match GeneratedTool::new(definition, Arc::new(EchoAdapter)) {
        Ok(_) => panic!("blank adapter_id should fail"),
        Err(err) => assert!(err.to_string().contains("adapter_id must be non-empty")),
    }
}

#[test]
fn generated_tool_normalizes_definition_fields() {
    let mut definition = sample_definition();
    definition.name = " send_update ".into();
    definition.description = " Send a scoped update. ".into();
    definition.adapter_id = " echo-adapter ".into();

    let tool = GeneratedTool::new(definition, Arc::new(EchoAdapter)).unwrap();

    assert_eq!(tool.name(), "send_update");
    assert_eq!(tool.description(), "Send a scoped update.");
    assert_eq!(tool.definition().adapter_id, "echo-adapter");
    assert_eq!(
        tool.definition().provider_id.as_deref(),
        Some("trusted.runtime")
    );
}

#[test]
fn admission_allows_trusted_generated_tool() {
    let report = admit_generated_tool_definitions(vec![sample_definition()], &admission_config());

    assert_eq!(report.admitted.len(), 1);
    assert!(report.rejected.is_empty());
}

#[test]
fn admission_normalizes_provider_ids_before_policy_checks() {
    let mut definition = sample_definition();
    definition.provider_id = Some(" Trusted.Runtime ".into());
    let config = GeneratedToolAdmissionConfig {
        enforce_provenance: true,
        trusted_providers: BTreeSet::from(["TRUSTED.RUNTIME".to_string()]),
        ..Default::default()
    };

    let report = admit_generated_tool_definitions(vec![definition], &config);

    assert_eq!(report.admitted.len(), 1);
    assert!(report.rejected.is_empty());
    assert_eq!(
        report.admitted[0].provider_id.as_deref(),
        Some("trusted.runtime")
    );
}

#[test]
fn admission_rejects_invalid_provider_ids_when_enforced() {
    let mut definition = sample_definition();
    definition.provider_id = Some("bad/provider".into());

    let report = admit_generated_tool_definitions(vec![definition], &admission_config());

    assert!(report.admitted.is_empty());
    assert!(report.rejected[0].reason.contains("invalid provider_id"));
}

#[test]
fn admission_disabled_preserves_legacy_generated_tools() {
    let mut definition = sample_definition();
    definition.provider_id = None;
    definition.capability_id = None;
    definition.source_digest = None;
    definition.risk = None;

    let report = admit_generated_tool_definitions(
        vec![definition],
        &GeneratedToolAdmissionConfig::default(),
    );

    assert_eq!(report.admitted.len(), 1);
    assert!(report.rejected.is_empty());
}

#[test]
fn admission_rejects_untrusted_provider() {
    let mut definition = sample_definition();
    definition.provider_id = Some("other.runtime".into());

    let report = admit_generated_tool_definitions(vec![definition], &admission_config());

    assert!(report.admitted.is_empty());
    assert!(report.rejected[0].reason.contains("not trusted"));
}

#[test]
fn admission_rejects_disabled_provider() {
    let mut definition = sample_definition();
    let config = GeneratedToolAdmissionConfig {
        enforce_provenance: true,
        trusted_providers: BTreeSet::from(["trusted.runtime".to_string()]),
        disabled_providers: BTreeSet::from(["trusted.runtime".to_string()]),
        ..Default::default()
    };
    definition.provider_id = Some("trusted.runtime".into());
    let report = admit_generated_tool_definitions(vec![definition], &config);
    assert!(report.admitted.is_empty());
    assert!(report.rejected[0].reason.contains("disabled"));
}

#[test]
fn admission_rejects_disabled_capabilities() {
    let definition = sample_definition();
    let config = GeneratedToolAdmissionConfig {
        enforce_provenance: true,
        trusted_providers: BTreeSet::from(["trusted.runtime".to_string()]),
        disabled_capabilities: BTreeSet::from(["updates.send".to_string()]),
        ..Default::default()
    };

    let report = admit_generated_tool_definitions(vec![definition], &config);

    assert!(report.admitted.is_empty());
    assert!(report.rejected[0].reason.contains("disabled"));
}

#[test]
fn admission_rejects_duplicate_tool_names() {
    let report = admit_generated_tool_definitions(
        vec![sample_definition(), sample_definition()],
        &admission_config(),
    );

    assert_eq!(report.admitted.len(), 1);
    assert!(report.rejected[0].reason.contains("duplicate"));
}

#[test]
fn admission_rejects_missing_risk_when_enforced() {
    let mut definition = sample_definition();
    definition.risk = None;

    let report = admit_generated_tool_definitions(vec![definition], &admission_config());

    assert!(report.admitted.is_empty());
    assert!(report.rejected[0].reason.contains("missing risk"));
}

#[test]
fn admission_rejects_unsafe_names() {
    let mut definition = sample_definition();
    definition.name = "Bad Tool".into();

    let report = admit_generated_tool_definitions(vec![definition], &admission_config());

    assert!(report.admitted.is_empty());
    assert!(report.rejected[0].reason.contains("unsupported characters"));
}

#[tokio::test]
async fn generated_tool_marks_external_risk_as_external_effect() {
    let tool = GeneratedTool::new(sample_definition(), Arc::new(EchoAdapter)).unwrap();

    assert!(tool.external_effect());
}

#[tokio::test]
async fn generated_tool_marks_execute_risk_as_external_effect() {
    let mut definition = sample_definition();
    definition.risk = Some(GeneratedToolRisk::Execute);
    let tool = GeneratedTool::new(definition, Arc::new(EchoAdapter)).unwrap();

    assert!(tool.external_effect());
}
