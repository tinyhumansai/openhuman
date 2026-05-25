//! Boot-time spawn of installed local MCP servers.
//!
//! On core startup we iterate every [`InstalledServer`] in
//! [`super::store`] and bring up its stdio subprocess via
//! [`super::connections::connect`]. Errors are logged per-server and never
//! block boot — a misbehaving server should not prevent the rest of the
//! core from coming up.
//!
//! HTTP-remote MCP servers are out of scope here: they have no subprocess
//! to spawn. Once the `InstalledServer` model grows a remote-transport
//! variant this function will skip them (or call a remote "warm-up" path).

use crate::openhuman::config::Config;

use super::{connections, store};

/// Spawn every locally-installed MCP server. Per-server failures are logged
/// and swallowed.
pub async fn spawn_installed_servers(config: &Config) {
    let servers = match store::list_servers(config) {
        Ok(s) => s,
        Err(err) => {
            tracing::warn!("[mcp-registry] boot: list_servers failed: {err}");
            return;
        }
    };

    if servers.is_empty() {
        tracing::debug!("[mcp-registry] boot: no installed servers to spawn");
        return;
    }

    tracing::info!(
        "[mcp-registry] boot: spawning {} installed server(s)",
        servers.len()
    );

    for server in servers {
        let server_id = server.server_id.clone();
        let qualified = server.qualified_name.clone();
        match connections::connect(config, &server).await {
            Ok(tools) => tracing::info!(
                "[mcp-registry] boot: connected server_id={} qualified={} tools={}",
                server_id,
                qualified,
                tools.len()
            ),
            Err(err) => tracing::warn!(
                "[mcp-registry] boot: connect failed server_id={} qualified={} err={err}",
                server_id,
                qualified
            ),
        }
    }
}
