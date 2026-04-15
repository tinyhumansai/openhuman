// Composio Tool Provider — optional managed tool surface with 1000+ OAuth integrations.
//
// When enabled, OpenHuman can execute actions on Gmail, Notion, GitHub, Slack, etc.
// through Composio's API without storing raw OAuth tokens locally.
//
// This is opt-in. Users who prefer sovereign/local-only mode skip this entirely.
// The Composio API key is stored in the encrypted secret store.

use crate::openhuman::security::policy::ToolOperation;
use crate::openhuman::security::SecurityPolicy;
use crate::openhuman::tools::traits::{Tool, ToolCategory, ToolResult};
use anyhow::Context;
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

const COMPOSIO_API_BASE_V2: &str = "https://backend.composio.dev/api/v2";
const COMPOSIO_API_BASE_V3: &str = "https://backend.composio.dev/api/v3";

fn ensure_https(url: &str) -> anyhow::Result<()> {
    if !url.starts_with("https://") {
        anyhow::bail!(
            "Refusing to transmit sensitive data over non-HTTPS URL: URL scheme must be https"
        );
    }
    Ok(())
}

/// A tool that proxies actions to the Composio managed tool platform.
pub struct ComposioTool {
    api_key: String,
    default_entity_id: String,
    security: Arc<SecurityPolicy>,
}

impl ComposioTool {
    pub fn new(
        api_key: &str,
        default_entity_id: Option<&str>,
        security: Arc<SecurityPolicy>,
    ) -> Self {
        Self {
            api_key: api_key.to_string(),
            default_entity_id: normalize_entity_id(default_entity_id.unwrap_or("default")),
            security,
        }
    }

    fn client(&self) -> Client {
        crate::openhuman::config::build_runtime_proxy_client_with_timeouts("tool.composio", 60, 10)
    }

    /// List available Composio apps/actions for the authenticated user.
    ///
    /// Uses v3 endpoint first and falls back to v2 for compatibility.
    pub async fn list_actions(
        &self,
        app_name: Option<&str>,
    ) -> anyhow::Result<Vec<ComposioAction>> {
        match self.list_actions_v3(app_name).await {
            Ok(items) => Ok(items),
            Err(v3_err) => {
                let v2 = self.list_actions_v2(app_name).await;
                match v2 {
                    Ok(items) => Ok(items),
                    Err(v2_err) => anyhow::bail!(
                        "Composio action listing failed on v3 ({v3_err}) and v2 fallback ({v2_err})"
                    ),
                }
            }
        }
    }

    async fn list_actions_v3(&self, app_name: Option<&str>) -> anyhow::Result<Vec<ComposioAction>> {
        let url = format!("{COMPOSIO_API_BASE_V3}/tools");
        let mut req = self.client().get(&url).header("x-api-key", &self.api_key);

        req = req.query(&[("limit", "200")]);
        if let Some(app) = app_name.map(str::trim).filter(|app| !app.is_empty()) {
            req = req.query(&[("toolkits", app), ("toolkit_slug", app)]);
        }

        let resp = req.send().await?;
        if !resp.status().is_success() {
            let err = response_error(resp).await;
            anyhow::bail!("Composio v3 API error: {err}");
        }

        let body: ComposioToolsResponse = resp
            .json()
            .await
            .context("Failed to decode Composio v3 tools response")?;
        Ok(map_v3_tools_to_actions(body.items))
    }

    async fn list_actions_v2(&self, app_name: Option<&str>) -> anyhow::Result<Vec<ComposioAction>> {
        let mut url = format!("{COMPOSIO_API_BASE_V2}/actions");
        if let Some(app) = app_name {
            url = format!("{url}?appNames={app}");
        }

        let resp = self
            .client()
            .get(&url)
            .header("x-api-key", &self.api_key)
            .send()
            .await?;

        if !resp.status().is_success() {
            let err = response_error(resp).await;
            anyhow::bail!("Composio v2 API error: {err}");
        }

        let body: ComposioActionsResponse = resp
            .json()
            .await
            .context("Failed to decode Composio v2 actions response")?;
        Ok(body.items)
    }

    /// Execute a Composio action/tool with given parameters.
    ///
    /// Uses v3 endpoint first and falls back to v2 for compatibility.
    pub async fn execute_action(
        &self,
        action_name: &str,
        params: serde_json::Value,
        entity_id: Option<&str>,
        connected_account_ref: Option<&str>,
    ) -> anyhow::Result<serde_json::Value> {
        let tool_slug = normalize_tool_slug(action_name);

        match self
            .execute_action_v3(&tool_slug, params.clone(), entity_id, connected_account_ref)
            .await
        {
            Ok(result) => Ok(result),
            Err(v3_err) => match self.execute_action_v2(action_name, params, entity_id).await {
                Ok(result) => Ok(result),
                Err(v2_err) => anyhow::bail!(
                    "Composio execute failed on v3 ({v3_err}) and v2 fallback ({v2_err})"
                ),
            },
        }
    }

    fn build_execute_action_v3_request(
        tool_slug: &str,
        params: serde_json::Value,
        entity_id: Option<&str>,
        connected_account_ref: Option<&str>,
    ) -> (String, serde_json::Value) {
        let url = format!("{COMPOSIO_API_BASE_V3}/tools/{tool_slug}/execute");
        let account_ref = connected_account_ref.and_then(|candidate| {
            let trimmed_candidate = candidate.trim();
            (!trimmed_candidate.is_empty()).then_some(trimmed_candidate)
        });

        let mut body = json!({
            "arguments": params,
        });

        if let Some(entity) = entity_id {
            body["user_id"] = json!(entity);
        }
        if let Some(account_ref) = account_ref {
            body["connected_account_id"] = json!(account_ref);
        }

        (url, body)
    }

    async fn execute_action_v3(
        &self,
        tool_slug: &str,
        params: serde_json::Value,
        entity_id: Option<&str>,
        connected_account_ref: Option<&str>,
    ) -> anyhow::Result<serde_json::Value> {
        let (url, body) = Self::build_execute_action_v3_request(
            tool_slug,
            params,
            entity_id,
            connected_account_ref,
        );

        ensure_https(&url)?;

        let resp = self
            .client()
            .post(&url)
            .header("x-api-key", &self.api_key)
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let err = response_error(resp).await;
            anyhow::bail!("Composio v3 action execution failed: {err}");
        }

        let result: serde_json::Value = resp
            .json()
            .await
            .context("Failed to decode Composio v3 execute response")?;
        Ok(result)
    }

    async fn execute_action_v2(
        &self,
        action_name: &str,
        params: serde_json::Value,
        entity_id: Option<&str>,
    ) -> anyhow::Result<serde_json::Value> {
        let url = format!("{COMPOSIO_API_BASE_V2}/actions/{action_name}/execute");

        let mut body = json!({
            "input": params,
        });

        if let Some(entity) = entity_id {
            body["entityId"] = json!(entity);
        }

        let resp = self
            .client()
            .post(&url)
            .header("x-api-key", &self.api_key)
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let err = response_error(resp).await;
            anyhow::bail!("Composio v2 action execution failed: {err}");
        }

        let result: serde_json::Value = resp
            .json()
            .await
            .context("Failed to decode Composio v2 execute response")?;
        Ok(result)
    }

    /// Get the OAuth connection URL for a specific app/toolkit or auth config.
    ///
    /// Uses v3 endpoint first and falls back to v2 for compatibility.
    pub async fn get_connection_url(
        &self,
        app_name: Option<&str>,
        auth_config_id: Option<&str>,
        entity_id: &str,
    ) -> anyhow::Result<String> {
        let v3 = self
            .get_connection_url_v3(app_name, auth_config_id, entity_id)
            .await;
        match v3 {
            Ok(url) => Ok(url),
            Err(v3_err) => {
                let app = app_name.ok_or_else(|| {
                    anyhow::anyhow!(
                        "Composio v3 connect failed ({v3_err}) and v2 fallback requires 'app'"
                    )
                })?;
                match self.get_connection_url_v2(app, entity_id).await {
                    Ok(url) => Ok(url),
                    Err(v2_err) => anyhow::bail!(
                        "Composio connect failed on v3 ({v3_err}) and v2 fallback ({v2_err})"
                    ),
                }
            }
        }
    }

    async fn get_connection_url_v3(
        &self,
        app_name: Option<&str>,
        auth_config_id: Option<&str>,
        entity_id: &str,
    ) -> anyhow::Result<String> {
        let auth_config_id = match auth_config_id {
            Some(id) => id.to_string(),
            None => {
                let app = app_name.ok_or_else(|| {
                    anyhow::anyhow!("Missing 'app' or 'auth_config_id' for v3 connect")
                })?;
                self.resolve_auth_config_id(app).await?
            }
        };

        let url = format!("{COMPOSIO_API_BASE_V3}/connected_accounts/link");
        let body = json!({
            "auth_config_id": auth_config_id,
            "user_id": entity_id,
        });

        let resp = self
            .client()
            .post(&url)
            .header("x-api-key", &self.api_key)
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let err = response_error(resp).await;
            anyhow::bail!("Composio v3 connect failed: {err}");
        }

        let result: serde_json::Value = resp
            .json()
            .await
            .context("Failed to decode Composio v3 connect response")?;
        extract_redirect_url(&result)
            .ok_or_else(|| anyhow::anyhow!("No redirect URL in Composio v3 response"))
    }

    async fn get_connection_url_v2(
        &self,
        app_name: &str,
        entity_id: &str,
    ) -> anyhow::Result<String> {
        let url = format!("{COMPOSIO_API_BASE_V2}/connectedAccounts");

        let body = json!({
            "integrationId": app_name,
            "entityId": entity_id,
        });

        let resp = self
            .client()
            .post(&url)
            .header("x-api-key", &self.api_key)
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let err = response_error(resp).await;
            anyhow::bail!("Composio v2 connect failed: {err}");
        }

        let result: serde_json::Value = resp
            .json()
            .await
            .context("Failed to decode Composio v2 connect response")?;
        extract_redirect_url(&result)
            .ok_or_else(|| anyhow::anyhow!("No redirect URL in Composio v2 response"))
    }

    async fn resolve_auth_config_id(&self, app_name: &str) -> anyhow::Result<String> {
        let url = format!("{COMPOSIO_API_BASE_V3}/auth_configs");

        let resp = self
            .client()
            .get(&url)
            .header("x-api-key", &self.api_key)
            .query(&[
                ("toolkit_slug", app_name),
                ("show_disabled", "true"),
                ("limit", "25"),
            ])
            .send()
            .await?;

        if !resp.status().is_success() {
            let err = response_error(resp).await;
            anyhow::bail!("Composio v3 auth config lookup failed: {err}");
        }

        let body: ComposioAuthConfigsResponse = resp
            .json()
            .await
            .context("Failed to decode Composio v3 auth configs response")?;

        if body.items.is_empty() {
            anyhow::bail!(
                "No auth config found for toolkit '{app_name}'. Create one in Composio first."
            );
        }

        let preferred = body
            .items
            .iter()
            .find(|cfg| cfg.is_enabled())
            .or_else(|| body.items.first())
            .context("No usable auth config returned by Composio")?;

        Ok(preferred.id.clone())
    }
}

#[async_trait]
impl Tool for ComposioTool {
    fn name(&self) -> &str {
        "composio"
    }

    fn description(&self) -> &str {
        "Execute actions on 1000+ apps via Composio (Gmail, Notion, GitHub, Slack, etc.). \
         Use action='list' to see available actions, action='execute' with action_name/tool_slug, params, and optional connected_account_id, \
         or action='connect' with app/auth_config_id to get OAuth URL."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "The operation: 'list' (list available actions), 'execute' (run an action), or 'connect' (get OAuth URL)",
                    "enum": ["list", "execute", "connect"]
                },
                "app": {
                    "type": "string",
                    "description": "Toolkit slug filter for 'list', or toolkit/app for 'connect' (e.g. 'gmail', 'notion', 'github')"
                },
                "action_name": {
                    "type": "string",
                    "description": "Action/tool identifier to execute (legacy aliases supported)"
                },
                "tool_slug": {
                    "type": "string",
                    "description": "Preferred v3 tool slug to execute (alias of action_name)"
                },
                "params": {
                    "type": "object",
                    "description": "Parameters to pass to the action"
                },
                "entity_id": {
                    "type": "string",
                    "description": "Entity/user ID for multi-user setups (defaults to composio.entity_id from config)"
                },
                "auth_config_id": {
                    "type": "string",
                    "description": "Optional Composio v3 auth config id for connect flow"
                },
                "connected_account_id": {
                    "type": "string",
                    "description": "Optional connected account ID for execute flow when a specific account is required"
                }
            },
            "required": ["action"]
        })
    }

    fn category(&self) -> ToolCategory {
        // Composio proxies to external SaaS (Gmail, Notion, …) — surface
        // it in the Skill category so the skills sub-agent
        // (`category_filter = "skill"`) can see and call it.
        ToolCategory::Skill
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'action' parameter"))?;

        let entity_id = args
            .get("entity_id")
            .and_then(|v| v.as_str())
            .unwrap_or(self.default_entity_id.as_str());

        match action {
            "list" => {
                let app = args.get("app").and_then(|v| v.as_str());
                match self.list_actions(app).await {
                    Ok(actions) => {
                        let summary: Vec<String> = actions
                            .iter()
                            .take(20)
                            .map(|a| {
                                format!(
                                    "- {} ({}): {}",
                                    a.name,
                                    a.app_name.as_deref().unwrap_or("?"),
                                    a.description.as_deref().unwrap_or("")
                                )
                            })
                            .collect();
                        let total = actions.len();
                        let output = format!(
                            "Found {total} available actions:\n{}{}",
                            summary.join("\n"),
                            if total > 20 {
                                format!("\n... and {} more", total - 20)
                            } else {
                                String::new()
                            }
                        );
                        Ok(ToolResult::success(output))
                    }
                    Err(e) => Ok(ToolResult::error(format!("Failed to list actions: {e}"))),
                }
            }

            "execute" => {
                if let Err(error) = self
                    .security
                    .enforce_tool_operation(ToolOperation::Act, "composio.execute")
                {
                    return Ok(ToolResult::error(error));
                }

                let action_name = args
                    .get("tool_slug")
                    .or_else(|| args.get("action_name"))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        anyhow::anyhow!("Missing 'action_name' (or 'tool_slug') for execute")
                    })?;

                let params = args.get("params").cloned().unwrap_or(json!({}));
                let acct_ref = args.get("connected_account_id").and_then(|v| v.as_str());

                match self
                    .execute_action(action_name, params, Some(entity_id), acct_ref)
                    .await
                {
                    Ok(result) => {
                        let output = serde_json::to_string_pretty(&result)
                            .unwrap_or_else(|_| format!("{result:?}"));
                        Ok(ToolResult::success(output))
                    }
                    Err(e) => Ok(ToolResult::error(format!("Action execution failed: {e}"))),
                }
            }

            "connect" => {
                if let Err(error) = self
                    .security
                    .enforce_tool_operation(ToolOperation::Act, "composio.connect")
                {
                    return Ok(ToolResult::error(error));
                }

                let app = args.get("app").and_then(|v| v.as_str());
                let auth_config_id = args.get("auth_config_id").and_then(|v| v.as_str());

                if app.is_none() && auth_config_id.is_none() {
                    anyhow::bail!("Missing 'app' or 'auth_config_id' for connect");
                }

                match self
                    .get_connection_url(app, auth_config_id, entity_id)
                    .await
                {
                    Ok(url) => {
                        let target =
                            app.unwrap_or(auth_config_id.unwrap_or("provided auth config"));
                        Ok(ToolResult::success(format!(
                            "Open this URL to connect {target}:\n{url}"
                        )))
                    }
                    Err(e) => Ok(ToolResult::error(format!(
                        "Failed to get connection URL: {e}"
                    ))),
                }
            }

            _ => Ok(ToolResult::error(format!(
                "Unknown action '{action}'. Use 'list', 'execute', or 'connect'."
            ))),
        }
    }
}

fn normalize_entity_id(entity_id: &str) -> String {
    let trimmed = entity_id.trim();
    if trimmed.is_empty() {
        "default".to_string()
    } else {
        trimmed.to_string()
    }
}

fn normalize_tool_slug(action_name: &str) -> String {
    action_name.trim().replace('_', "-").to_ascii_lowercase()
}

fn map_v3_tools_to_actions(items: Vec<ComposioV3Tool>) -> Vec<ComposioAction> {
    items
        .into_iter()
        .filter_map(|item| {
            let name = item.slug.or(item.name.clone())?;
            let app_name = item
                .toolkit
                .as_ref()
                .and_then(|toolkit| toolkit.slug.clone().or(toolkit.name.clone()))
                .or(item.app_name);
            let description = item.description.or(item.name);
            Some(ComposioAction {
                name,
                app_name,
                description,
                enabled: true,
            })
        })
        .collect()
}

fn extract_redirect_url(result: &serde_json::Value) -> Option<String> {
    result
        .get("redirect_url")
        .and_then(|v| v.as_str())
        .or_else(|| result.get("redirectUrl").and_then(|v| v.as_str()))
        .or_else(|| {
            result
                .get("data")
                .and_then(|v| v.get("redirect_url"))
                .and_then(|v| v.as_str())
        })
        .map(ToString::to_string)
}

async fn response_error(resp: reqwest::Response) -> String {
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if body.trim().is_empty() {
        return format!("HTTP {}", status.as_u16());
    }

    if let Some(api_error) = extract_api_error_message(&body) {
        return format!(
            "HTTP {}: {}",
            status.as_u16(),
            sanitize_error_message(&api_error)
        );
    }

    format!("HTTP {}", status.as_u16())
}

fn sanitize_error_message(message: &str) -> String {
    let mut sanitized = message.replace('\n', " ");
    for marker in [
        "connected_account_id",
        "connectedAccountId",
        "entity_id",
        "entityId",
        "user_id",
        "userId",
    ] {
        sanitized = sanitized.replace(marker, "[redacted]");
    }

    let max_chars = 240;
    if sanitized.chars().count() <= max_chars {
        sanitized
    } else {
        let mut end = max_chars;
        while end > 0 && !sanitized.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &sanitized[..end])
    }
}

fn extract_api_error_message(body: &str) -> Option<String> {
    let parsed: serde_json::Value = serde_json::from_str(body).ok()?;
    parsed
        .get("error")
        .and_then(|v| v.get("message"))
        .and_then(|v| v.as_str())
        .map(ToString::to_string)
        .or_else(|| {
            parsed
                .get("message")
                .and_then(|v| v.as_str())
                .map(ToString::to_string)
        })
}

// ── API response types ──────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct ComposioActionsResponse {
    #[serde(default)]
    items: Vec<ComposioAction>,
}

#[derive(Debug, Deserialize)]
struct ComposioToolsResponse {
    #[serde(default)]
    items: Vec<ComposioV3Tool>,
}

#[derive(Debug, Clone, Deserialize)]
struct ComposioV3Tool {
    #[serde(default)]
    slug: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(rename = "appName", default)]
    app_name: Option<String>,
    #[serde(default)]
    toolkit: Option<ComposioToolkitRef>,
}

#[derive(Debug, Clone, Deserialize)]
struct ComposioToolkitRef {
    #[serde(default)]
    slug: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ComposioAuthConfigsResponse {
    #[serde(default)]
    items: Vec<ComposioAuthConfig>,
}

#[derive(Debug, Clone, Deserialize)]
struct ComposioAuthConfig {
    id: String,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    enabled: Option<bool>,
}

impl ComposioAuthConfig {
    fn is_enabled(&self) -> bool {
        self.enabled.unwrap_or(false)
            || self
                .status
                .as_deref()
                .is_some_and(|v| v.eq_ignore_ascii_case("enabled"))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposioAction {
    pub name: String,
    #[serde(rename = "appName")]
    pub app_name: Option<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub enabled: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::security::{AutonomyLevel, SecurityPolicy};

    fn test_security() -> Arc<SecurityPolicy> {
        Arc::new(SecurityPolicy::default())
    }

    // ── Constructor ───────────────────────────────────────────

    #[test]
    fn composio_tool_has_correct_name() {
        let tool = ComposioTool::new("test-key", None, test_security());
        assert_eq!(tool.name(), "composio");
    }

    #[test]
    fn composio_tool_has_description() {
        let tool = ComposioTool::new("test-key", None, test_security());
        assert!(!tool.description().is_empty());
        assert!(tool.description().contains("1000+"));
    }

    #[test]
    fn composio_tool_schema_has_required_fields() {
        let tool = ComposioTool::new("test-key", None, test_security());
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["action"].is_object());
        assert!(schema["properties"]["action_name"].is_object());
        assert!(schema["properties"]["tool_slug"].is_object());
        assert!(schema["properties"]["params"].is_object());
        assert!(schema["properties"]["app"].is_object());
        assert!(schema["properties"]["auth_config_id"].is_object());
        assert!(schema["properties"]["connected_account_id"].is_object());
        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&json!("action")));
    }

    #[test]
    fn composio_tool_spec_roundtrip() {
        let tool = ComposioTool::new("test-key", None, test_security());
        let spec = tool.spec();
        assert_eq!(spec.name, "composio");
        assert!(spec.parameters.is_object());
    }

    // ── Execute validation ────────────────────────────────────

    #[tokio::test]
    async fn execute_missing_action_returns_error() {
        let tool = ComposioTool::new("test-key", None, test_security());
        let result = tool.execute(json!({})).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn execute_unknown_action_returns_error() {
        let tool = ComposioTool::new("test-key", None, test_security());
        let result = tool.execute(json!({"action": "unknown"})).await.unwrap();
        assert!(result.is_error);
        assert!(&result.output().contains("Unknown action"));
    }

    #[tokio::test]
    async fn execute_without_action_name_returns_error() {
        let tool = ComposioTool::new("test-key", None, test_security());
        let result = tool.execute(json!({"action": "execute"})).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn connect_without_target_returns_error() {
        let tool = ComposioTool::new("test-key", None, test_security());
        let result = tool.execute(json!({"action": "connect"})).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn execute_blocked_in_readonly_mode() {
        let readonly = Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::ReadOnly,
            ..SecurityPolicy::default()
        });
        let tool = ComposioTool::new("test-key", None, readonly);
        let result = tool
            .execute(json!({
                "action": "execute",
                "action_name": "GITHUB_LIST_REPOS"
            }))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("read-only mode"));
    }

    #[tokio::test]
    async fn execute_blocked_when_rate_limited() {
        let limited = Arc::new(SecurityPolicy {
            max_actions_per_hour: 0,
            ..SecurityPolicy::default()
        });
        let tool = ComposioTool::new("test-key", None, limited);
        let result = tool
            .execute(json!({
                "action": "execute",
                "action_name": "GITHUB_LIST_REPOS"
            }))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("Rate limit exceeded"));
    }

    // ── API response parsing ──────────────────────────────────

    #[test]
    fn composio_action_deserializes() {
        let json_str = r#"{"name": "GMAIL_FETCH_EMAILS", "appName": "gmail", "description": "Fetch emails", "enabled": true}"#;
        let action: ComposioAction = serde_json::from_str(json_str).unwrap();
        assert_eq!(action.name, "GMAIL_FETCH_EMAILS");
        assert_eq!(action.app_name.as_deref(), Some("gmail"));
        assert!(action.enabled);
    }

    #[test]
    fn composio_actions_response_deserializes() {
        let json_str = r#"{"items": [{"name": "TEST_ACTION", "appName": "test", "description": "A test", "enabled": true}]}"#;
        let resp: ComposioActionsResponse = serde_json::from_str(json_str).unwrap();
        assert_eq!(resp.items.len(), 1);
        assert_eq!(resp.items[0].name, "TEST_ACTION");
    }

    #[test]
    fn composio_actions_response_empty() {
        let json_str = r#"{"items": []}"#;
        let resp: ComposioActionsResponse = serde_json::from_str(json_str).unwrap();
        assert!(resp.items.is_empty());
    }

    #[test]
    fn composio_actions_response_missing_items_defaults() {
        let json_str = r"{}";
        let resp: ComposioActionsResponse = serde_json::from_str(json_str).unwrap();
        assert!(resp.items.is_empty());
    }

    #[test]
    fn composio_v3_tools_response_maps_to_actions() {
        let json_str = r#"{
            "items": [
                {
                    "slug": "gmail-fetch-emails",
                    "name": "Gmail Fetch Emails",
                    "description": "Fetch inbox emails",
                    "toolkit": { "slug": "gmail", "name": "Gmail" }
                }
            ]
        }"#;
        let resp: ComposioToolsResponse = serde_json::from_str(json_str).unwrap();
        let actions = map_v3_tools_to_actions(resp.items);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].name, "gmail-fetch-emails");
        assert_eq!(actions[0].app_name.as_deref(), Some("gmail"));
        assert_eq!(
            actions[0].description.as_deref(),
            Some("Fetch inbox emails")
        );
    }

    #[test]
    fn normalize_entity_id_falls_back_to_default_when_blank() {
        assert_eq!(normalize_entity_id("   "), "default");
        assert_eq!(normalize_entity_id("workspace-user"), "workspace-user");
    }

    #[test]
    fn normalize_tool_slug_supports_legacy_action_name() {
        assert_eq!(
            normalize_tool_slug("GMAIL_FETCH_EMAILS"),
            "gmail-fetch-emails"
        );
        assert_eq!(
            normalize_tool_slug(" github-list-repos "),
            "github-list-repos"
        );
    }

    #[test]
    fn extract_redirect_url_supports_v2_and_v3_shapes() {
        let v2 = json!({"redirectUrl": "https://app.composio.dev/connect-v2"});
        let v3 = json!({"redirect_url": "https://app.composio.dev/connect-v3"});
        let nested = json!({"data": {"redirect_url": "https://app.composio.dev/connect-nested"}});

        assert_eq!(
            extract_redirect_url(&v2).as_deref(),
            Some("https://app.composio.dev/connect-v2")
        );
        assert_eq!(
            extract_redirect_url(&v3).as_deref(),
            Some("https://app.composio.dev/connect-v3")
        );
        assert_eq!(
            extract_redirect_url(&nested).as_deref(),
            Some("https://app.composio.dev/connect-nested")
        );
    }

    #[test]
    fn auth_config_prefers_enabled_status() {
        let enabled = ComposioAuthConfig {
            id: "cfg_1".into(),
            status: Some("ENABLED".into()),
            enabled: None,
        };
        let disabled = ComposioAuthConfig {
            id: "cfg_2".into(),
            status: Some("DISABLED".into()),
            enabled: Some(false),
        };

        assert!(enabled.is_enabled());
        assert!(!disabled.is_enabled());
    }

    #[test]
    fn extract_api_error_message_from_common_shapes() {
        let nested = r#"{"error":{"message":"tool not found"}}"#;
        let flat = r#"{"message":"invalid api key"}"#;

        assert_eq!(
            extract_api_error_message(nested).as_deref(),
            Some("tool not found")
        );
        assert_eq!(
            extract_api_error_message(flat).as_deref(),
            Some("invalid api key")
        );
        assert_eq!(extract_api_error_message("not-json"), None);
    }

    #[test]
    fn composio_action_with_null_fields() {
        let json_str =
            r#"{"name": "TEST_ACTION", "appName": null, "description": null, "enabled": false}"#;
        let action: ComposioAction = serde_json::from_str(json_str).unwrap();
        assert_eq!(action.name, "TEST_ACTION");
        assert!(action.app_name.is_none());
        assert!(action.description.is_none());
        assert!(!action.enabled);
    }

    #[test]
    fn composio_action_with_special_characters() {
        let json_str = r#"{"name": "GMAIL_SEND_EMAIL_WITH_ATTACHMENT", "appName": "gmail", "description": "Send email with attachment & special chars: <>'\"\"", "enabled": true}"#;
        let action: ComposioAction = serde_json::from_str(json_str).unwrap();
        assert_eq!(action.name, "GMAIL_SEND_EMAIL_WITH_ATTACHMENT");
        assert!(action.description.as_ref().unwrap().contains('&'));
        assert!(action.description.as_ref().unwrap().contains('<'));
    }

    #[test]
    fn composio_action_with_unicode() {
        let json_str = r#"{"name": "SLACK_SEND_MESSAGE", "appName": "slack", "description": "Send message with emoji 🎉 and unicode 中文", "enabled": true}"#;
        let action: ComposioAction = serde_json::from_str(json_str).unwrap();
        assert!(action.description.as_ref().unwrap().contains("🎉"));
        assert!(action.description.as_ref().unwrap().contains("中文"));
    }

    #[test]
    fn composio_malformed_json_returns_error() {
        let json_str = r#"{"name": "TEST_ACTION", "appName": "gmail", }"#;
        let result: Result<ComposioAction, _> = serde_json::from_str(json_str);
        assert!(result.is_err());
    }

    #[test]
    fn composio_empty_json_string_returns_error() {
        let json_str = r#" ""#;
        let result: Result<ComposioAction, _> = serde_json::from_str(json_str);
        assert!(result.is_err());
    }

    #[test]
    fn composio_large_actions_list() {
        let mut items = Vec::new();
        for i in 0..100 {
            items.push(json!({
                "name": format!("ACTION_{i}"),
                "appName": "test",
                "description": "Test action",
                "enabled": true
            }));
        }
        let json_str = json!({"items": items}).to_string();
        let resp: ComposioActionsResponse = serde_json::from_str(&json_str).unwrap();
        assert_eq!(resp.items.len(), 100);
    }

    #[test]
    fn composio_api_base_url_is_v3() {
        assert_eq!(COMPOSIO_API_BASE_V3, "https://backend.composio.dev/api/v3");
    }

    #[test]
    fn build_execute_action_v3_request_uses_fixed_endpoint_and_body_account_id() {
        let (url, body) = ComposioTool::build_execute_action_v3_request(
            "gmail-send-email",
            json!({"to": "test@example.com"}),
            Some("workspace-user"),
            Some("account-42"),
        );

        assert_eq!(
            url,
            "https://backend.composio.dev/api/v3/tools/gmail-send-email/execute"
        );
        assert_eq!(body["arguments"]["to"], json!("test@example.com"));
        assert_eq!(body["user_id"], json!("workspace-user"));
        assert_eq!(body["connected_account_id"], json!("account-42"));
    }

    #[test]
    fn build_execute_action_v3_request_drops_blank_optional_fields() {
        let (url, body) = ComposioTool::build_execute_action_v3_request(
            "github-list-repos",
            json!({}),
            None,
            Some("   "),
        );

        assert_eq!(
            url,
            "https://backend.composio.dev/api/v3/tools/github-list-repos/execute"
        );
        assert_eq!(body["arguments"], json!({}));
        assert!(body.get("connected_account_id").is_none());
        assert!(body.get("user_id").is_none());
    }

    // ── ensure_https ──────────────────────────────────────────────────────────

    #[test]
    fn ensure_https_accepts_https_url() {
        assert!(ensure_https("https://backend.composio.dev/api/v3/tools").is_ok());
    }

    #[test]
    fn ensure_https_rejects_http_url() {
        let err = ensure_https("http://backend.composio.dev/api/v3/tools").unwrap_err();
        assert!(err.to_string().contains("non-HTTPS"));
    }

    #[test]
    fn ensure_https_rejects_ftp_url() {
        assert!(ensure_https("ftp://example.com").is_err());
    }

    // ── sanitize_error_message ────────────────────────────────────────────────

    #[test]
    fn sanitize_error_message_replaces_sensitive_fields() {
        let msg = "Invalid connected_account_id value for entity_id: user-123";
        let sanitized = sanitize_error_message(msg);
        assert!(!sanitized.contains("connected_account_id"));
        assert!(!sanitized.contains("entity_id"));
        assert!(sanitized.contains("[redacted]"));
    }

    #[test]
    fn sanitize_error_message_replaces_newlines_with_spaces() {
        let msg = "line1\nline2\nline3";
        let sanitized = sanitize_error_message(msg);
        assert!(!sanitized.contains('\n'));
        assert!(sanitized.contains("line1"));
        assert!(sanitized.contains("line2"));
    }

    #[test]
    fn sanitize_error_message_truncates_long_messages() {
        let long_msg = "x".repeat(500);
        let sanitized = sanitize_error_message(&long_msg);
        assert!(
            sanitized.chars().count() <= 243,
            "should be at most 240 chars + '...'"
        );
        assert!(
            sanitized.ends_with("..."),
            "truncated message should end with '...'"
        );
    }

    #[test]
    fn sanitize_error_message_does_not_truncate_short_messages() {
        let short = "Something went wrong";
        let sanitized = sanitize_error_message(short);
        assert_eq!(sanitized, short);
    }

    #[test]
    fn sanitize_error_message_replaces_all_sensitive_variants() {
        // camelCase variants
        let msg = "Error for connectedAccountId and entityId and userId";
        let sanitized = sanitize_error_message(msg);
        assert!(
            !sanitized.contains("connectedAccountId"),
            "camelCase connectedAccountId should be redacted"
        );
        assert!(
            !sanitized.contains("entityId"),
            "camelCase entityId should be redacted"
        );
        assert!(
            !sanitized.contains("userId"),
            "camelCase userId should be redacted"
        );
    }

    // ── composio_auth_config enabled detection ────────────────────────────────

    #[test]
    fn auth_config_enabled_by_flag() {
        let cfg = ComposioAuthConfig {
            id: "cfg_x".into(),
            status: None,
            enabled: Some(true),
        };
        assert!(cfg.is_enabled());
    }

    #[test]
    fn auth_config_not_enabled_when_both_missing() {
        let cfg = ComposioAuthConfig {
            id: "cfg_x".into(),
            status: None,
            enabled: None,
        };
        assert!(!cfg.is_enabled());
    }

    // ── map_v3_tools_to_actions: item without slug falls back to name ─────────

    #[test]
    fn map_v3_tools_uses_name_when_slug_missing() {
        let items = vec![ComposioV3Tool {
            slug: None,
            name: Some("My Tool".into()),
            description: None,
            app_name: Some("myapp".into()),
            toolkit: None,
        }];
        let actions = map_v3_tools_to_actions(items);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].name, "My Tool");
        assert_eq!(actions[0].app_name.as_deref(), Some("myapp"));
    }

    #[test]
    fn map_v3_tools_skips_items_without_slug_or_name() {
        let items = vec![ComposioV3Tool {
            slug: None,
            name: None,
            description: Some("desc".into()),
            app_name: None,
            toolkit: None,
        }];
        let actions = map_v3_tools_to_actions(items);
        assert!(
            actions.is_empty(),
            "item with no slug or name should be filtered out"
        );
    }

    #[test]
    fn map_v3_tools_prefers_toolkit_slug_over_app_name() {
        let items = vec![ComposioV3Tool {
            slug: Some("tool-slug".into()),
            name: None,
            description: None,
            app_name: Some("fallback-app".into()),
            toolkit: Some(ComposioToolkitRef {
                slug: Some("preferred-app".into()),
                name: None,
            }),
        }];
        let actions = map_v3_tools_to_actions(items);
        assert_eq!(actions[0].app_name.as_deref(), Some("preferred-app"));
    }

    // ── category ──────────────────────────────────────────────────────────────

    #[test]
    fn composio_tool_category_is_skill() {
        use crate::openhuman::tools::traits::ToolCategory;
        let tool = ComposioTool::new("key", None, test_security());
        assert_eq!(tool.category(), ToolCategory::Skill);
    }
}
