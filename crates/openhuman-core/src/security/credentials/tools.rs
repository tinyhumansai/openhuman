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

use crate::config::Config;
use crate::security::credentials;
use tinytools::{Tool, ToolResult};

/// Dispatch a hosted `auth.oauth_*` controller through the registry.
///
/// The backend-brokered OAuth RPCs live in `openhuman-tinyhumans`
/// (`hosted::oauth`) and are registered only when that layer is installed, so
/// the tools reach them by wire name instead of calling them directly. With no
/// hosted layer the tool reports the `BACKEND_UNAVAILABLE:` sentinel.
async fn invoke_hosted(
    method: &str,
    params: serde_json::Map<String, Value>,
) -> anyhow::Result<Value> {
    match crate::core::all::try_invoke_registered_rpc(method, params).await {
        Some(Ok(value)) => Ok(strip_log_envelope(value)),
        Some(Err(err)) => Err(anyhow::anyhow!("{err}")),
        None => {
            log::debug!("[tool][credentials] {method} not registered (no hosted layer)");
            Err(anyhow::anyhow!(
                "{}{method} requires the hosted TinyHumans layer",
                crate::core::observability::BACKEND_UNAVAILABLE_PREFIX
            ))
        }
    }
}

/// `{ "result": v, "logs": [...] }` → `v`; anything else unchanged.
fn strip_log_envelope(value: Value) -> Value {
    match value {
        Value::Object(mut map)
            if map.len() == 2 && map.contains_key("result") && map.contains_key("logs") =>
        {
            map.remove("result").unwrap_or(Value::Null)
        }
        other => other,
    }
}

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
        "Return the current auth/session state (signed-in flag, credential kind, \
         the signed-in user's profile — no token material)."
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
        let mut params = serde_json::Map::new();
        params.insert("provider".into(), json!(provider));
        if let Some(skill_id) = args.get("skill_id").and_then(Value::as_str) {
            params.insert("skillId".into(), json!(skill_id));
        }
        let value = invoke_hosted("openhuman.auth_oauth_connect", params)
            .await
            .map_err(|e| anyhow::anyhow!("oauth_connect_url: {e}"))?;
        Ok(ToolResult::success(serde_json::to_string(&value)?))
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
        let value = invoke_hosted("openhuman.auth_oauth_list_integrations", Default::default())
            .await
            .map_err(|e| anyhow::anyhow!("oauth_list: {e}"))?;
        Ok(ToolResult::success(serde_json::to_string(&value)?))
    }
    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tests;
