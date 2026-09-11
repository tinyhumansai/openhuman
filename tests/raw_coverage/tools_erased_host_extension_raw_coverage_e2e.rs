//! Raw integration coverage for the erased host-extension seam #5841 created.
//!
//! #5841 moved the `Tool` vocabulary into the shared `tinytools` crate. Two
//! trait methods named host-only types and could not go with it, so they now
//! ride `Tool::host_extension` / `Tool::host_call_extension` as
//! `dyn Any`, read back through typed free functions in
//! `openhuman::tools::traits`.
//!
//! That trade is invisible to the compiler in one direction. A producer that
//! stores a different concrete type still builds; the `downcast_ref` /
//! `downcast` in the reader simply returns `None`, and every consumer treats
//! `None` as "this tool has no such context". For
//! `generated_runtime_context` that consumer is
//! `agent::tinyagents::middleware::generated_context`, which feeds
//! `tool_policy::GeneratedToolRuntimeContext` — so a silent `None` means the
//! provider / capability / risk rules for a generated tool are simply not
//! applied.
//!
//! Every assertion that existed on this seam — in the e2e lane
//! (`tools_approval_channels_raw_coverage_e2e.rs`) and in the unit lane
//! (`tools/traits_tests.rs`) — asserts `is_none()`. The failure mode and the
//! only tested state were the same value. These tests assert the `Some` side.

use std::any::Any;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use openhuman_core::openhuman::agent::tool_policy::{
    GeneratedToolRuntimeContext, GeneratedToolRuntimeRisk,
};
use openhuman_core::openhuman::skills::types::tool_result_from_mcp;
use openhuman_core::openhuman::tools::toolpacks::registry::PACKS;
use openhuman_core::openhuman::tools::toolpacks::tools::{LoadSkillTool, PackRegistryHandle};
use openhuman_core::openhuman::tools::traits::{
    generated_runtime_context, pack_registry_handle, PermissionLevel, Tool, ToolResult,
};

/// A production pack tool's registry handle survives the round trip through
/// `dyn Any` **as the same handle**, not merely as some handle.
///
/// `LoadSkillTool` is one of the two real producers in the tree. If its
/// `host_extension` ever stored something other than a `PackRegistryHandle`,
/// this is the only place that would notice: `toolpacks::ops` reads the handle
/// back through the same free function and, on `None`, silently skips the pack
/// registry rather than failing.
///
/// `is_some()` alone is too weak to catch the interesting half of that. A
/// producer that handed back a *different* `PackRegistryHandle` — a fresh
/// `default()`, or a second instance — satisfies `is_some()` while
/// `toolpacks::ops` reads an unbound registry and skips the pack exactly as if
/// the downcast had failed. `PackRegistryHandle` is `Clone + Default` with a
/// private `Arc<OnceLock<_>>` and no `PartialEq`, so identity is not directly
/// comparable; it is observable, because two handles share one `OnceLock` only
/// if they are the same handle. So bind through the **recovered** handle and
/// observe the effect on the **tool**, which reads its own.
///
/// `render_pack` distinguishes the two states by message, which is what makes
/// this a real discriminator rather than a smoke test:
///
/// * handle unbound  ⇒ "The skill registry is not available in this session"
/// * handle bound, registry empty ⇒ "Skill `…` has no tools available"
///
/// An empty registry is deliberate: the second message proves the binding was
/// observed without needing to construct a real packed tool.
#[tokio::test]
async fn a_pack_tools_registry_handle_reads_back_as_the_same_handle() {
    let tool = LoadSkillTool::new(PackRegistryHandle::default());

    let recovered = pack_registry_handle(&tool).expect(
        "a pack tool must yield its PackRegistryHandle through the erased \
         host extension; None here is the silent-downcast failure that makes \
         toolpacks::ops skip the registry",
    );

    // Kept alive for the whole test: the handle stores a `Weak`, so dropping
    // this would make the binding unobservable and the assertion vacuous.
    let registry: Arc<Vec<Box<dyn Tool>>> = Arc::new(Vec::new());
    recovered.bind(Arc::downgrade(&registry));

    let skill = PACKS
        .first()
        .expect("the pack registry ships at least one pack")
        .id;
    let rendered = tool
        .execute(json!({ "skill": skill }))
        .await
        .expect("load_skill reports failure in its ToolResult, never as Err")
        .output()
        .to_string();

    assert!(
        !rendered.contains("registry is not available"),
        "binding through the recovered handle did not reach the tool's own \
         handle, so the erased extension yielded a different \
         PackRegistryHandle than the one supplied — the failure `is_some()` \
         cannot see. Rendered: {rendered}"
    );
    assert!(
        rendered.contains("has no tools available"),
        "the tool should have got as far as walking the (empty) bound \
         registry; a different message means this test is no longer \
         discriminating between bound and unbound. Rendered: {rendered}"
    );
}

/// A generated tool's per-call runtime context survives the round trip, field
/// for field.
///
/// The context is carried by value through `Box<dyn Any>`, so this asserts the
/// fields the policy layer actually reads rather than only that *something*
/// came back — a downcast to a same-shaped but different type would satisfy
/// `is_some()` and still lose the policy inputs.
///
/// The producer here is a local fixture on purpose: no production tool
/// implements `host_call_extension` (see `W8-test-findings.md`), so there is no
/// shipped tool to drive this through. What is pinned is the seam itself.
#[test]
fn a_generated_tools_runtime_context_reads_back_with_its_policy_fields() {
    let context = generated_runtime_context(&GeneratedTool, &json!({"to": "someone"}))
        .expect("a tool that supplies a per-call context must yield it back");

    assert_eq!(context.provider_id, "mail.runtime");
    assert_eq!(context.capability_id, "email.send");
    assert_eq!(
        context.risk,
        GeneratedToolRuntimeRisk::ExternalWrite,
        "risk is what the policy layer gates on; losing it downgrades an \
         external write to the default"
    );
    assert_eq!(context.source_digest.as_deref(), Some("sha256:abc"));
    assert_eq!(context.approval_id.as_deref(), Some("approval-1"));
}

/// The MCP → host result conversion keeps the error flag the right way round.
///
/// #5841 turned this from a `From` impl into the free function
/// `skills::types::tool_result_from_mcp`, because `ToolResult` became a foreign
/// type and the orphan rule forbade the impl. The PR's own note gives the
/// reason it stays written exactly once: spelled out at each of its three call
/// sites, it would be three chances to get the error flag backwards. Nothing
/// asserted the orientation.
#[test]
fn an_mcp_error_result_stays_an_error_through_the_conversion() {
    let failed = tool_result_from_mcp(tinymcp_bus::McpToolResult {
        content: vec![tinymcp_bus::McpToolContent::Text {
            text: "server refused".to_string(),
        }],
        is_error: true,
        ..Default::default()
    });
    assert!(
        failed.is_error,
        "an MCP result flagged as an error must stay an error; inverting this \
         reports a failed tool call to the model as a success"
    );

    let ok = tool_result_from_mcp(tinymcp_bus::McpToolResult {
        content: vec![tinymcp_bus::McpToolContent::Text {
            text: "done".to_string(),
        }],
        is_error: false,
        ..Default::default()
    });
    assert!(
        !ok.is_error,
        "a successful MCP result must not be reported as an error"
    );
}

// ── fixtures ──────────────────────────────────────────────────────────────

struct PlainTool;

#[async_trait]
impl Tool for PlainTool {
    fn name(&self) -> &str {
        "plain_tool"
    }

    fn description(&self) -> &str {
        "A tool that stores nothing on either erased extension."
    }

    fn parameters_schema(&self) -> Value {
        json!({"type": "object"})
    }

    async fn execute(&self, _args: Value) -> anyhow::Result<ToolResult> {
        Ok(ToolResult::success("ok"))
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }
}

struct GeneratedTool;

#[async_trait]
impl Tool for GeneratedTool {
    fn name(&self) -> &str {
        "generated_tool"
    }

    fn description(&self) -> &str {
        "Stands in for a generated tool carrying a per-call runtime context."
    }

    fn parameters_schema(&self) -> Value {
        json!({"type": "object"})
    }

    async fn execute(&self, _args: Value) -> anyhow::Result<ToolResult> {
        Ok(ToolResult::success("sent"))
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    fn host_call_extension(&self, _args: &Value) -> Option<Box<dyn Any + Send + Sync>> {
        Some(Box::new(GeneratedToolRuntimeContext {
            provider_id: "mail.runtime".to_string(),
            capability_id: "email.send".to_string(),
            risk: GeneratedToolRuntimeRisk::ExternalWrite,
            source_digest: Some("sha256:abc".to_string()),
            approval_id: Some("approval-1".to_string()),
        }))
    }
}
