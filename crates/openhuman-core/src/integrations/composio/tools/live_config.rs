//! [`live_composio_config`]: the config a Composio tool resolves its client
//! from on each call.

use crate::config::rpc as config_rpc;
use crate::config::Config;

/// The config one Composio tool call runs under.
///
/// A host-pinned credential makes the tool's own snapshot authoritative, so
/// an embedded agent keeps its own key, mode and entity. Otherwise the
/// snapshot's `config_path` is reloaded so a live `composio.mode` toggle takes
/// effect on the next call.
pub(crate) async fn live_composio_config(snapshot: &Config) -> Result<Config, String> {
    if snapshot.composio.host_credential.is_some() {
        tracing::debug!("[composio] host-pinned credential — using the agent's config");
        return Ok(snapshot.clone());
    }
    config_rpc::reload_config_snapshot_with_timeout(snapshot).await
}
