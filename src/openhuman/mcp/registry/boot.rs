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

use futures::stream::StreamExt;

use crate::openhuman::config::Config;

use super::types::InstalledServer;
use super::{connections, store};

/// How many installed MCP servers are brought up concurrently at boot.
///
/// Each `connect` spawns a stdio subprocess and does the MCP `initialize` +
/// `tools/list` handshake, so the connects are independent network/process
/// round-trips — serial spawning made boot latency the *sum* of every
/// server's warmup. Bounded concurrency overlaps the handshakes while still
/// capping the subprocess spawn burst (a user with dozens of servers
/// shouldn't fork them all at once). The registry insert each `connect`
/// performs is internally synchronized, so there is no shared-state hazard.
const BOOT_SPAWN_CONCURRENCY: usize = 8;

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

    spawn_servers_concurrently(servers, |server| async move {
        connections::connect(config, &server)
            .await
            .map(|tools| tools.len())
    })
    .await;
}

/// Bring up `servers` with bounded concurrency, logging per-server outcomes.
///
/// Disabled servers are filtered (and logged) before fan-out. `connect_fn`
/// takes the server **by value** (not `&InstalledServer`) so the returned
/// future owns its input — borrowing the argument would force rustc into a
/// higher-ranked `FnOnce` bound that fails to infer. Order is irrelevant:
/// each connect's effect (registry insert) is independent, so this uses
/// `for_each_concurrent` rather than an ordered combinator.
async fn spawn_servers_concurrently<F, Fut>(servers: Vec<InstalledServer>, connect_fn: F)
where
    F: Fn(InstalledServer) -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<usize>>,
{
    let enabled = servers.into_iter().filter(|server| {
        if !server.enabled {
            tracing::info!(
                "[mcp-registry] boot: skipping disabled server_id={} qualified={}",
                server.server_id,
                server.qualified_name
            );
        }
        server.enabled
    });

    futures::stream::iter(enabled)
        .for_each_concurrent(BOOT_SPAWN_CONCURRENCY, |server| {
            let connect_fn = &connect_fn;
            let server_id = server.server_id.clone();
            let qualified = server.qualified_name.clone();
            async move {
                match connect_fn(server).await {
                    Ok(tool_count) => tracing::info!(
                        "[mcp-registry] boot: connected server_id={} qualified={} tools={}",
                        server_id,
                        qualified,
                        tool_count
                    ),
                    Err(err) => tracing::warn!(
                        "[mcp-registry] boot: connect failed server_id={} qualified={} err={err}",
                        server_id,
                        qualified
                    ),
                }
            }
        })
        .await;
}

#[cfg(test)]
#[path = "boot_tests.rs"]
mod tests;
