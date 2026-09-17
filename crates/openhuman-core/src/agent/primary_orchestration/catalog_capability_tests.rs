use super::capability::CapabilitySideEffect;
use super::catalog::{build_capability_catalog, CapabilityCatalogInputs};
use crate::tools::traits::{PermissionLevel, Tool};

struct StubTool {
    name: String,
    permission: PermissionLevel,
}

impl StubTool {
    fn new(name: impl Into<String>, permission: PermissionLevel) -> Self {
        Self {
            name: name.into(),
            permission,
        }
    }

    fn write(name: impl Into<String>) -> Box<dyn Tool> {
        Box::new(Self::new(name, PermissionLevel::Write))
    }

    fn read_only(name: impl Into<String>) -> Box<dyn Tool> {
        Box::new(Self::new(name, PermissionLevel::ReadOnly))
    }

    fn none(name: impl Into<String>) -> Box<dyn Tool> {
        Box::new(Self::new(name, PermissionLevel::None))
    }
}

#[async_trait::async_trait]
impl Tool for StubTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        "stub tool for capability catalog testing"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {}
        })
    }

    async fn execute(
        &self,
        _input: serde_json::Value,
    ) -> anyhow::Result<crate::tools::traits::ToolResult> {
        Ok(crate::tools::traits::ToolResult::success("stub"))
    }

    fn permission_level(&self) -> PermissionLevel {
        self.permission
    }
}

#[test]
fn permission_is_copied_and_side_effects_distinguish_local_and_external_read_write() {
    // 1. Local backend with None, ReadOnly, Write
    // We can't put duplicate names in the same catalog call, so test individually
    let inputs = CapabilityCatalogInputs::default();

    let cat_none = build_capability_catalog(&[StubTool::none("file_read")], &inputs).unwrap();
    assert_eq!(
        cat_none.routes[0].capability.permission,
        PermissionLevel::None
    );
    assert_eq!(
        cat_none.routes[0].capability.side_effect,
        CapabilitySideEffect::None
    );

    let cat_read = build_capability_catalog(&[StubTool::read_only("file_read")], &inputs).unwrap();
    assert_eq!(
        cat_read.routes[0].capability.permission,
        PermissionLevel::ReadOnly
    );
    assert_eq!(
        cat_read.routes[0].capability.side_effect,
        CapabilitySideEffect::LocalRead
    );

    let cat_write = build_capability_catalog(&[StubTool::write("file_read")], &inputs).unwrap();
    assert_eq!(
        cat_write.routes[0].capability.permission,
        PermissionLevel::Write
    );
    assert_eq!(
        cat_write.routes[0].capability.side_effect,
        CapabilitySideEffect::LocalWrite
    );

    // 2. External backends distinguish ExternalRead and ExternalWrite
    // DirectNetwork: web_fetch
    let net_read = build_capability_catalog(&[StubTool::read_only("web_fetch")], &inputs).unwrap();
    assert_eq!(
        net_read.routes[0].capability.permission,
        PermissionLevel::ReadOnly
    );
    assert_eq!(
        net_read.routes[0].capability.side_effect,
        CapabilitySideEffect::ExternalRead
    );

    let net_write = build_capability_catalog(&[StubTool::write("web_fetch")], &inputs).unwrap();
    assert_eq!(
        net_write.routes[0].capability.permission,
        PermissionLevel::Write
    );
    assert_eq!(
        net_write.routes[0].capability.side_effect,
        CapabilitySideEffect::ExternalWrite
    );

    // LocalBrowser: browser
    let browser_read =
        build_capability_catalog(&[StubTool::read_only("browser")], &inputs).unwrap();
    assert_eq!(
        browser_read.routes[0].capability.side_effect,
        CapabilitySideEffect::ExternalRead
    );

    let browser_write = build_capability_catalog(&[StubTool::write("browser")], &inputs).unwrap();
    assert_eq!(
        browser_write.routes[0].capability.side_effect,
        CapabilitySideEffect::ExternalWrite
    );

    // Byok: exa_search
    let byok_read =
        build_capability_catalog(&[StubTool::read_only("exa_search")], &inputs).unwrap();
    assert_eq!(
        byok_read.routes[0].capability.side_effect,
        CapabilitySideEffect::ExternalRead
    );

    let byok_write = build_capability_catalog(&[StubTool::write("exa_search")], &inputs).unwrap();
    assert_eq!(
        byok_write.routes[0].capability.side_effect,
        CapabilitySideEffect::ExternalWrite
    );

    // Managed: parallel_search
    let managed_read =
        build_capability_catalog(&[StubTool::read_only("parallel_search")], &inputs).unwrap();
    assert_eq!(
        managed_read.routes[0].capability.side_effect,
        CapabilitySideEffect::ExternalRead
    );

    let managed_write =
        build_capability_catalog(&[StubTool::write("parallel_search")], &inputs).unwrap();
    assert_eq!(
        managed_write.routes[0].capability.side_effect,
        CapabilitySideEffect::ExternalWrite
    );
}
