//! Raw integration coverage for the erased host-extension seam #5841 created.
//!
//! #5841 moved the `Tool` vocabulary into the shared `tinytools` crate. Two
//! trait methods named host-only types and could not go with it, so they now
//! ride `Tool::host_extension` / `Tool::host_call_extension` as
//! `dyn Any`, read back through typed free functions in
//! `openhuman::tools::traits`.
//!
//! That trade is invisible to the compiler in one direction. A producer that
//! stores a different concrete type still builds; the `downcast_ref` in the
//! reader simply returns `None`, and every consumer treats `None` as "this
//! tool has no such context".
//!
//! Every assertion that existed on this seam — in the e2e lane
//! (`tools_approval_channels_raw_coverage_e2e.rs`) and in the unit lane
//! (`tools/traits_tests.rs`) — asserts `is_none()`. The failure mode and the
//! only tested state were the same value. These tests assert the `Some` side.

use std::sync::Arc;

use serde_json::json;

use openhuman_core::tools::host_extensions::pack_registry_handle;
use openhuman_core::tools::toolpacks::registry::PACKS;
use openhuman_core::tools::toolpacks::{append_pack_tools, bind_pack_registry};
use tinytools::Tool;

/// A production pack tool's registry handle survives the round trip through
/// `dyn Any` **as the same handle**, not merely as some handle.
///
/// `UseSkillTool` is the only real producer in the tree. If its
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
async fn the_public_pack_builder_returns_the_handle_its_tool_uses() {
    let mut tools: Vec<Box<dyn Tool>> = Vec::new();
    let returned = append_pack_tools(&mut tools);
    let registry = Arc::new(tools);
    bind_pack_registry(&returned, &registry);
    let tool = registry.iter().find(|tool| tool.name() == "use_skill").expect("pack tool registered");

    let recovered = pack_registry_handle(tool.as_ref()).expect(
        "a pack tool must yield its PackRegistryHandle through the erased \
         host extension; None here is the silent-downcast failure that makes \
         toolpacks::ops skip the registry",
    );

    // Kept alive for the whole test: the handle stores a `Weak`, so dropping
    // this would make the binding unobservable and the assertion vacuous.
    recovered.bind(Arc::downgrade(&registry));

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
