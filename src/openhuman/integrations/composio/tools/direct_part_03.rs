
#[async_trait]
impl Tool for ComposioTool {
    fn name(&self) -> &str {
        "composio"
    }

    fn description(&self) -> &str {
        "Execute actions on 1000+ apps via Composio (Gmail, Notion, GitHub, Slack, etc.). \
         Use action='list' to see available actions, action='execute' with action_name/tool_slug, params, and optional connected_account_id, \
         or action='connect' with app/auth_config_id to get OAuth URL. \
         For Gmail: GMAIL_FETCH_EMAILS supports standard Gmail search syntax in the 'query' param — \
         use query='from:me' or query='label:SENT' to retrieve sent emails, query='label:INBOX' for inbox, \
         query='is:unread' for unread mail, etc. Sent mail is synced and searchable."
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
        // it in the Workflow category so the skills sub-agent
        // (`category_filter = "skill"`) can see and call it.
        ToolCategory::Workflow
    }

    fn external_effect(&self) -> bool {
        // Conservative default for the arg-less path: assume any
        // composio call is a write so callers that don't reach the
        // args-aware override still get gated. The harness uses
        // `external_effect_with_args` (below) which inspects
        // `action` and lets read-only branches through.
        true
    }

    fn external_effect_with_args(&self, args: &serde_json::Value) -> bool {
        // `action="list"` enumerates available Composio actions —
        // a read-only catalog call. `action="connect"` only returns
        // an OAuth URL the user then visits manually; the
        // subsequent OAuth handoff is its own consent flow so the
        // tool call itself has no outbound side effect to gate.
        // `action="execute"` (or anything unknown / missing) is the
        // write path and routes through the approval gate.
        !matches!(
            args.get("action").and_then(|v| v.as_str()),
            Some("list") | Some("connect")
        )
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

    crate::openhuman::util::truncate_with_ellipsis(&sanitized, 240)
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
    /// JSON schema for the tool parameters. Composio v3 names this
    /// `input_parameters`; older payloads use `parameters`. Either
    /// shape deserialises into this field, and we re-emit it as
    /// `ComposioToolFunction::parameters` so direct-mode users get
    /// the same model-callable schema backend mode surfaces.
    #[serde(default, alias = "parameters")]
    input_parameters: Option<serde_json::Value>,
    /// JSON schema for the tool's OUTPUT/return value, per Composio v3
    /// `/tools`'s `output_parameters` field ("Schema definition of return
    /// values from the tool" —
    /// <https://docs.composio.dev/reference/api-reference/tools/getTools>).
    /// Re-emitted as `ComposioToolFunction::output_parameters` so callers
    /// can ground a downstream binding in the tool's real output field
    /// names instead of guessing them.
    #[serde(default)]
    output_parameters: Option<serde_json::Value>,
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

/// Direct-mode tool definition lifted from Composio v3 `/tools`.
///
/// Carries the `input_parameters` JSON schema so the upstream
/// `composio_list_tools` direct branch can hand the LLM agent a
/// model-callable function shape — same fields backend mode surfaces
/// through `ComposioToolSchema`.
///
/// Kept distinct from `ComposioAction` (legacy flattened shape) so
/// new callers explicitly opt into the schema-preserving variant.
#[derive(Debug, Clone)]
pub struct ComposioToolSchemaV3 {
    pub slug: String,
    pub description: Option<String>,
    pub toolkit_slug: Option<String>,
    pub input_parameters: Option<serde_json::Value>,
    /// See [`ComposioV3Tool::output_parameters`] — Composio v3's schema for
    /// the action's return value, when published.
    pub output_parameters: Option<serde_json::Value>,
}

impl ComposioToolSchemaV3 {
    fn from_v3_tool(item: ComposioV3Tool) -> Self {
        let slug = item
            .slug
            .clone()
            .or_else(|| item.name.clone())
            .unwrap_or_default();
        let toolkit_slug = item
            .toolkit
            .as_ref()
            .and_then(|t| t.slug.clone().or(t.name.clone()))
            .or(item.app_name);
        Self {
            slug,
            description: item.description.or(item.name),
            toolkit_slug,
            input_parameters: item.input_parameters,
            output_parameters: item.output_parameters,
        }
    }
}

// ── v3 /connected_accounts envelope ─────────────────────────────────
//
// Public so the `composio/client.rs::direct_list_connections` helper
// in the domain layer can reshape it into the canonical
// `ComposioConnection` type. Kept distinct from `ComposioConnection`
// itself (which is the backend-proxied envelope) so the two paths
// don't get coupled — Composio v3 may add or rename fields and we'd
// rather adjust the mapping than reshuffle the public type.

#[derive(Debug, Deserialize)]
struct ComposioConnectedAccountsResponse {
    #[serde(default)]
    items: Vec<ComposioConnectedAccount>,
}

/// One v3 connected-account row.
///
/// Field shapes follow Composio's v3 docs as of May 2026. `toolkit` may
/// be either a string slug (older payloads) or a nested object with a
/// `slug` field (newer payloads); [`Self::toolkit_slug`] extracts the
/// canonical slug from either shape.
#[derive(Debug, Clone, Deserialize)]
pub struct ComposioConnectedAccount {
    #[serde(default)]
    pub id: String,
    /// `"ACTIVE"`, `"INITIATED"`, `"FAILED"`, … — passed through as-is
    /// so the caller's status filter (`ComposioConnection::is_active`)
    /// applies uniformly across both backend-proxied and direct paths.
    #[serde(default)]
    pub status: Option<String>,
    /// Composio uses `created_at` (snake_case) at v3. We keep both
    /// spellings to tolerate any upstream drift back to `createdAt`.
    #[serde(default, alias = "createdAt")]
    pub created_at: Option<String>,
    /// Toolkit may be a plain string slug or a nested
    /// `ComposioToolkitRef`. Extracted via [`Self::toolkit_slug`].
    #[serde(default)]
    toolkit: Option<serde_json::Value>,
    /// Older payload shape — a top-level `app_name` string. Used as
    /// a fallback when `toolkit` is absent or unparseable.
    #[serde(default, rename = "appName", alias = "app_name")]
    app_name: Option<String>,
}

impl ComposioConnectedAccount {
    /// Best-effort extract of the toolkit slug from the
    /// possibly-polymorphic `toolkit` field, falling back to
    /// `app_name`. Returns `None` only when no recognizable slug
    /// representation is present.
    pub fn toolkit_slug(&self) -> Option<String> {
        if let Some(value) = &self.toolkit {
            match value {
                serde_json::Value::String(s) => {
                    let t = s.trim();
                    if !t.is_empty() {
                        return Some(t.to_string());
                    }
                }
                serde_json::Value::Object(map) => {
                    for key in ["slug", "id", "name", "key"] {
                        if let Some(serde_json::Value::String(s)) = map.get(key) {
                            let t = s.trim();
                            if !t.is_empty() {
                                return Some(t.to_string());
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        self.app_name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    }
}
