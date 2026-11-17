//! [`all_composio_agent_tools`]: build the full set of Composio agent
//! tools once the user is signed in (backend session or direct-mode key).

use std::sync::Arc;

use tinytools::Tool;

use super::authorize::ComposioAuthorizeTool;
use super::connect::ComposioConnectTool;
use super::execute::ComposioExecuteTool;
use super::list_connections::ComposioListConnectionsTool;
use super::list_toolkits::ComposioListToolkitsTool;
use super::list_tools::ComposioListToolsTool;

pub async fn all_composio_agent_tools(config: &crate::config::Config) -> Vec<Box<dyn Tool>> {
    // Registration gate: ask the mode-aware probe "can this user call
    // composio at all?" — true when EITHER a backend session token OR a
    // stored/inline direct-mode API key is present. The pre-fix path
    // called `build_composio_client(...).is_none()`, which is
    // backend-only and silently dropped the 5 generic agent tools for
    // direct-mode users (#1710). Per-action dispatch inside each tool
    // re-resolves through the factory so the live `composio.mode`
    // toggle keeps winning.
    if !crate::integrations::composio::user_is_signed_in_to_composio(config).await {
        tracing::debug!(
            "[composio] agent tools not registered — user is not signed in to composio \
             (no backend session and no direct API key)"
        );
        return Vec::new();
    }
    // All five tools resolve their client per call through the
    // mode-aware factory; they only need a handle to the live root
    // config to do so. Sharing one `Arc<Config>` keeps the registration
    // cheap (no repeated `Config::clone` walks) and ensures every tool
    // sees the same live snapshot.
    let config_arc = Arc::new(config.clone());
    let tools: Vec<Box<dyn Tool>> = vec![
        Box::new(ComposioListToolkitsTool::new(config_arc.clone())),
        Box::new(ComposioListConnectionsTool::new(config_arc.clone())),
        Box::new(ComposioAuthorizeTool::new(config_arc.clone())),
        // Inline-in-chat OAuth connect card (#3993). Raises an approval card
        // with a Connect button instead of handing the agent a raw URL.
        Box::new(ComposioConnectTool::new(config_arc.clone())),
        Box::new(ComposioListToolsTool::new(config_arc.clone())),
        Box::new(ComposioExecuteTool::new(config_arc)),
        // Pref-elevation is intentionally NOT an agent-callable tool;
        // the user must flip it themselves in the Connections UI.
        // See the long comment above the (removed) ComposioEnableScopeTool.
    ];
    tracing::debug!(count = tools.len(), "[composio] agent tools registered");
    tools
}
