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
#[path = "composio_tests.rs"]
mod tests;
