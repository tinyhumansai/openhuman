//! Raw integration coverage for the public host-owned tool-pack binding API.
//!
//! The TinyAgents tool's generic `ToolCallBinding` extension intentionally no
//! longer exposes its private registry handle. OpenHuman therefore retains the
//! handle returned by `append_pack_tools` and binds it to the session registry.

use std::sync::Arc;

use serde_json::json;

use openhuman_core::tools::toolpacks::registry::PACKS;
use openhuman_core::tools::toolpacks::{append_pack_tools, bind_pack_registry};
use tinytools::Tool;

#[tokio::test]
async fn the_public_pack_builder_returns_the_handle_its_tool_uses() {
    let mut tools: Vec<Box<dyn Tool>> = Vec::new();
    let handle = append_pack_tools(&mut tools);
    let registry = Arc::new(tools);
    bind_pack_registry(&handle, &registry);
    let tool = registry
        .iter()
        .find(|tool| tool.name() == "use_skill")
        .expect("pack tool registered");

    let skill = PACKS
        .first()
        .expect("the pack registry ships at least one pack")
        .id;
    let rendered = tool
        .execute(json!({ "skill": skill }))
        .await
        .expect("use_skill reports failure in its ToolResult, never as Err")
        .output()
        .to_string();

    assert!(
        !rendered.contains("registry is not available"),
        "binding the returned handle did not reach the tool's registry: {rendered}"
    );
    assert!(
        rendered.contains("has no tools available"),
        "the tool should have reached the bound empty registry: {rendered}"
    );
}
