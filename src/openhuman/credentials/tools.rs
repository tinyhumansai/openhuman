//! LLM-callable wrappers over the `credentials` domain — READ-ONLY surface.
//!
//! These expose non-secret reads only: the list of stored credential profiles
//! (no token material), the session/auth state, the current user profile, the
//! OAuth connect URL, and the list of available integrations. All default-ON.
//!
//! The sensitive surface — storing/removing credentials, switching the active
//! profile, reading bearer/JWT plaintext, OAuth token handoff/revoke, Composio
//! key storage — is intentionally NOT exposed as agent tools (exfiltration /
//! auth-mutation risk). Those stay RPC-only.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::openhuman::config::Config;
use crate::openhuman::credentials;
use crate::openhuman::tools::traits::{Tool, ToolResult};

macro_rules! emit {
    ($outcome:expr, $name:literal) => {{
        let outcome = $outcome.map_err(|e| anyhow::anyhow!(concat!($name, ": {}"), e))?;
        Ok(ToolResult::success(serde_json::to_string(&outcome.value)?))
    }};
}

/// List stored credential profiles (no secrets).
pub struct CredentialListTool {
    config: Arc<Config>,
}
impl CredentialListTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}
#[async_trait]
impl Tool for CredentialListTool {
    fn name(&self) -> &str {
        "credential_list"
    }
    fn description(&self) -> &str {
        "List stored credential profiles (provider + profile names only — no \
         secret/token material). Optional `provider` filter."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": { "provider": { "type": "string" } } })
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][credentials] list invoked");
        let provider = args
            .get("provider")
            .and_then(Value::as_str)
            .map(str::to_string);
        emit!(
            credentials::list_provider_credentials(&self.config, provider).await,
            "credential_list"
        )
    }
    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

/// Auth/session state (no tokens).
pub struct SessionStateTool {
    config: Arc<Config>,
}
impl SessionStateTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}
#[async_trait]
impl Tool for SessionStateTool {
    fn name(&self) -> &str {
        "session_state"
    }
    fn description(&self) -> &str {
        "Return the current auth/session state (signed-in flag, profile info — \
         no token material)."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": {} })
    }
    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][credentials] session_state invoked");
        emit!(
            credentials::auth_get_state(&self.config).await,
            "session_state"
        )
    }
    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

/// Current user profile.
pub struct SessionGetUserTool {
    config: Arc<Config>,
}
impl SessionGetUserTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}
#[async_trait]
impl Tool for SessionGetUserTool {
    fn name(&self) -> &str {
        "session_get_user"
    }
    fn description(&self) -> &str {
        "Return the current signed-in user's profile (name/email/plan). Does not \
         expose the session token."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": {} })
    }
    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][credentials] get_user invoked");
        emit!(
            credentials::auth_get_me(&self.config).await,
            "session_get_user"
        )
    }
    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

/// OAuth authorize URL for a provider.
pub struct OAuthConnectUrlTool {
    config: Arc<Config>,
}
impl OAuthConnectUrlTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}
#[async_trait]
impl Tool for OAuthConnectUrlTool {
    fn name(&self) -> &str {
        "oauth_connect_url"
    }
    fn description(&self) -> &str {
        "Return an OAuth authorize URL for a `provider` (and optional `skill_id`) \
         the user can open to connect an integration. Returns a URL + state; \
         does not complete the connection."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "provider": { "type": "string" },
                "skill_id": { "type": "string" }
            },
            "required": ["provider"]
        })
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][credentials] oauth_connect invoked");
        let provider = args
            .get("provider")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow::anyhow!("missing required string argument `provider`"))?;
        let skill_id = args.get("skill_id").and_then(Value::as_str);
        emit!(
            credentials::oauth_connect(&self.config, provider, skill_id, None, None).await,
            "oauth_connect_url"
        )
    }
}

/// List available OAuth integrations.
pub struct OAuthListTool {
    config: Arc<Config>,
}
impl OAuthListTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}
#[async_trait]
impl Tool for OAuthListTool {
    fn name(&self) -> &str {
        "oauth_list"
    }
    fn description(&self) -> &str {
        "List the user's available/connected OAuth integrations."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": {} })
    }
    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][credentials] oauth_list invoked");
        emit!(
            credentials::oauth_list_integrations(&self.config).await,
            "oauth_list"
        )
    }
    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::tools::traits::{PermissionLevel, ToolScope};

    fn cfg() -> Arc<Config> {
        Arc::new(Config::default())
    }

    #[test]
    fn metadata() {
        assert_eq!(CredentialListTool::new(cfg()).name(), "credential_list");
        assert_eq!(SessionStateTool::new(cfg()).name(), "session_state");
        assert_eq!(OAuthConnectUrlTool::new(cfg()).name(), "oauth_connect_url");
        assert_eq!(
            CredentialListTool::new(cfg()).permission_level(),
            PermissionLevel::ReadOnly
        );
        assert_eq!(SessionStateTool::new(cfg()).scope(), ToolScope::All);
    }

    #[tokio::test]
    async fn oauth_connect_requires_provider() {
        let err = OAuthConnectUrlTool::new(cfg())
            .execute(json!({}))
            .await
            .expect_err("missing provider");
        assert!(err.to_string().contains("provider"));
    }
}
