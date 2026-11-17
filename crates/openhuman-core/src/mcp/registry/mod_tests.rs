//! The registry surface always-compiled callers depend on, pinned in both
//! builds.
//!
//! This module compiles with and without the `mcp` feature, so a signature
//! that differs between the real registry and `stub.rs` fails one of the two
//! builds here, even where the only caller is itself gated on `mcp`.

use super::*;
use crate::config::Config;
use crate::core::runtime::DispatchCtx;

/// Uses every mirrored function with its argument and return types spelled
/// out. It is built, never polled.
async fn shared_surface(config: &Config, ctx: &DispatchCtx) {
    let _: Vec<crate::core::all::RegisteredController> = all_mcp_registry_registered_controllers();
    let (): () = boot::spawn_installed_servers(config).await;
    let (): () = supervisor::run().await;
    let _: Result<String, String> = oauth::complete(config, "state", "code").await;
    let _: Vec<connections::ConnectedServerOverview> = connections::connected_overview(ctx).await;
    let _: Vec<(String, String, McpTool)> = connections::all_connected_tools(ctx).await;
    let _: Vec<(String, String, McpTool)> =
        connections::all_connected_tools_for_config(config).await;
}

#[test]
fn registry_surface_matches_in_both_builds() {
    let config = Config::default();
    let ctx = DispatchCtx::unbound();
    let _unpolled = shared_surface(&config, &ctx);
}
