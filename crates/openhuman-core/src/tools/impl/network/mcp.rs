use crate::mcp::config_servers::{McpDefinitionAuth, McpRegistrySource, McpServerRegistry};
use crate::security::{SecurityPolicy, ToolOperation};
use async_trait::async_trait;
use base64::Engine as _;
use serde_json::{json, Value};
use std::sync::Arc;
use tinytools::{PermissionLevel, Tool, ToolCallOptions, ToolContent, ToolResult};

pub struct McpListServersTool {
    registry: Arc<McpServerRegistry>,
}

impl McpListServersTool {
    pub fn new(registry: Arc<McpServerRegistry>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Tool for McpListServersTool {
    fn name(&self) -> &str {
        "mcp_list_servers"
    }

    fn description(&self) -> &str {
        "List named remote MCP servers registered in OpenHuman core. Use this before browsing tools on a specific MCP server."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    fn supports_markdown(&self) -> bool {
        true
    }

    async fn execute(&self, _args: Value) -> anyhow::Result<ToolResult> {
        let servers = self
            .registry
            .list()
            .into_iter()
            .map(|server| {
                json!({
                    "name": server.name,
                    "endpoint": endpoint_without_query(&server.endpoint),
                    "description": server.description,
                    "timeout_secs": server.timeout_secs,
                    "allowed_tools": server.allowed_tools,
                    "disallowed_tools": server.disallowed_tools,
                    "auth_configured": !matches!(server.auth, McpDefinitionAuth::None),
                    "auth_kind": auth_kind(&server.auth),
                    "source": server.source,
                })
            })
            .collect::<Vec<_>>();

        let markdown = if servers.is_empty() {
            "# MCP Servers\n\nNo remote MCP servers are registered.".to_string()
        } else {
            let mut md = String::from("# MCP Servers\n");
            for server in self.registry.list() {
                let source = match server.source {
                    McpRegistrySource::Config => "config",
                    McpRegistrySource::Host => "host",
                    _ => "unknown",
                };
                md.push_str(&format!(
                    "\n- **{}** ({source})\n  - endpoint: `{}`\n  - auth: `{}`",
                    server.name,
                    endpoint_without_query(&server.endpoint),
                    auth_kind(&server.auth)
                ));
                if let Some(description) = server.description.as_deref() {
                    md.push_str(&format!("\n  - {description}"));
                }
                if !server.allowed_tools.is_empty() {
                    md.push_str(&format!(
                        "\n  - allowed tools: `{}`",
                        server.allowed_tools.join("`, `")
                    ));
                }
                if !server.disallowed_tools.is_empty() {
                    md.push_str(&format!(
                        "\n  - disallowed tools: `{}`",
                        server.disallowed_tools.join("`, `")
                    ));
                }
            }
            md
        };

        Ok(ToolResult::success_with_markdown(
            json!({ "servers": servers }),
            markdown,
        ))
    }
}

pub struct McpListToolsTool {
    registry: Arc<McpServerRegistry>,
}

impl McpListToolsTool {
    pub fn new(registry: Arc<McpServerRegistry>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Tool for McpListToolsTool {
    fn name(&self) -> &str {
        "mcp_list_tools"
    }

    fn description(&self) -> &str {
        "List tools exposed by a named remote MCP server. Use this before calling `mcp_call_tool`."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "server": {
                    "type": "string",
                    "description": "Registered MCP server name from `mcp_list_servers`."
                }
            },
            "required": ["server"],
            "additionalProperties": false
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    fn supports_markdown(&self) -> bool {
        true
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let server = required_string_arg(&args, "server")?;
        let scrubber = SecretScrubber::for_server(&self.registry, &server);
        let tools = match self.registry.list_tools(&server).await {
            Ok(tools) => tools,
            Err(err) => {
                return Ok(ToolResult::error(
                    scrubber.scrub(&format!("mcp_list_tools failed: {err}")),
                ))
            }
        };

        let payload = tools
            .iter()
            .map(|tool| {
                json!({
                    "name": tool.name,
                    "title": tool.display_title(),
                    "description": tool.display_description(),
                    "input_schema": tool.input_schema,
                })
            })
            .collect::<Vec<_>>();

        let mut markdown = format!("# MCP Tools: `{server}`\n");
        if tools.is_empty() {
            markdown.push_str("\nNo tools were returned by the remote server.");
        } else {
            for tool in &tools {
                let desc = tool
                    .display_description()
                    .unwrap_or_else(|| "No description.".to_string());
                markdown.push_str(&format!(
                    "\n- **{}**: {}\n  - schema: `{}`",
                    tool.name,
                    desc,
                    serde_json::to_string(&tool.input_schema).unwrap_or_else(|_| "{}".into())
                ));
            }
        }

        Ok(scrubber.scrub_result(ToolResult::success_with_markdown(
            json!({ "server": server, "tools": payload }),
            markdown,
        )))
    }
}

pub struct McpCallTool {
    registry: Arc<McpServerRegistry>,
    security: Arc<SecurityPolicy>,
}

impl McpCallTool {
    pub fn new(registry: Arc<McpServerRegistry>, security: Arc<SecurityPolicy>) -> Self {
        Self { registry, security }
    }
}

#[async_trait]
impl Tool for McpCallTool {
    fn name(&self) -> &str {
        "mcp_call_tool"
    }

    fn description(&self) -> &str {
        "Call a tool on a named remote MCP server. First inspect available tools with `mcp_list_tools`, then pass the remote tool name and its JSON arguments here."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "server": {
                    "type": "string",
                    "description": "Registered MCP server name from `mcp_list_servers`."
                },
                "tool": {
                    "type": "string",
                    "description": "Remote MCP tool name from `mcp_list_tools`."
                },
                "arguments": {
                    "type": "object",
                    "description": "Arguments object passed through to the remote MCP tool."
                }
            },
            "required": ["server", "tool", "arguments"],
            "additionalProperties": false
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }

    fn supports_markdown(&self) -> bool {
        true
    }

    async fn execute_with_options(
        &self,
        args: Value,
        options: ToolCallOptions,
    ) -> anyhow::Result<ToolResult> {
        self.security
            .enforce_tool_operation(ToolOperation::Act, self.name())
            .map_err(|err| anyhow::anyhow!(err))?;

        let server = required_string_arg(&args, "server")?;
        let tool = required_string_arg(&args, "tool")?;
        let arguments = args
            .get("arguments")
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("missing required `arguments` object"))?;
        if !arguments.is_object() {
            return Ok(ToolResult::error("`arguments` must be an object"));
        }

        let scrubber = SecretScrubber::for_server(&self.registry, &server);
        let mut result = match self.registry.call_tool(&server, &tool, arguments).await {
            Ok(result) => result.rendered,
            Err(err) => {
                return Ok(ToolResult::error(
                    scrubber.scrub(&format!("mcp_call_tool failed: {err}")),
                ))
            }
        };

        if options.prefer_markdown && result.markdown_formatted.is_none() {
            result.markdown_formatted = Some(result.output());
        }
        Ok(scrubber.scrub_result(crate::skills::types::tool_result_from_mcp(result)))
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        self.execute_with_options(args, ToolCallOptions::default())
            .await
    }
}

const REDACTED: &str = "[redacted]";
const MIN_QUERY_SECRET_LEN: usize = 8;
const CREDENTIAL_QUERY_PARAM_NEEDLES: [&str; 6] =
    ["token", "key", "secret", "password", "auth", "sig"];

struct SecretScrubber {
    secrets: Vec<String>,
}

impl SecretScrubber {
    fn for_server(registry: &McpServerRegistry, server: &str) -> Self {
        let Some(definition) = registry.get(server) else {
            return Self {
                secrets: Vec::new(),
            };
        };
        Self::new(&definition.auth, &definition.endpoint)
    }

    fn new(auth: &McpDefinitionAuth, endpoint: &str) -> Self {
        let mut raw: Vec<String> = Vec::new();
        match auth {
            McpDefinitionAuth::BearerToken { token } => raw.push(token.clone()),
            McpDefinitionAuth::Basic { username, password } => {
                raw.push(username.clone());
                raw.push(password.clone());
                raw.push(
                    base64::engine::general_purpose::STANDARD
                        .encode(format!("{username}:{password}")),
                );
            }
            McpDefinitionAuth::Header { value, .. } => raw.push(value.clone()),
            McpDefinitionAuth::Headers { headers } => {
                raw.extend(headers.iter().map(|header| header.value.clone()));
            }
            McpDefinitionAuth::QueryParam { value, .. } => raw.push(value.clone()),
            _ => {}
        }
        if endpoint_query(endpoint).is_some() {
            if let Ok(url) = url::Url::parse(endpoint) {
                raw.extend(url.query_pairs().filter_map(|(name, value)| {
                    let name = name.to_ascii_lowercase();
                    let credential_like = CREDENTIAL_QUERY_PARAM_NEEDLES
                        .iter()
                        .any(|needle| name.contains(needle));
                    let value = value.into_owned();
                    (credential_like && value.len() >= MIN_QUERY_SECRET_LEN).then_some(value)
                }));
            }
        }

        let mut secrets: Vec<String> = Vec::new();
        for value in raw {
            let value = value.trim();
            if value.is_empty() {
                continue;
            }
            let encoded = urlencoding::encode(value).into_owned();
            if encoded != value {
                secrets.push(encoded);
            }
            secrets.push(value.to_string());
        }
        secrets.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
        secrets.dedup();
        Self { secrets }
    }

    fn scrub(&self, text: &str) -> String {
        let mut out = text.to_string();
        for secret in &self.secrets {
            if out.contains(secret.as_str()) {
                out = out.replace(secret.as_str(), REDACTED);
            }
        }
        out
    }

    fn scrub_value(&self, value: &mut Value) {
        match value {
            Value::String(text) => *text = self.scrub(text),
            Value::Array(items) => items.iter_mut().for_each(|item| self.scrub_value(item)),
            Value::Object(map) => {
                let entries = std::mem::take(map);
                for (key, mut item) in entries {
                    self.scrub_value(&mut item);
                    map.insert(self.scrub(&key), item);
                }
            }
            _ => {}
        }
    }

    fn scrub_result(&self, mut result: ToolResult) -> ToolResult {
        if self.secrets.is_empty() {
            return result;
        }
        let mut redactions = 0usize;
        for block in &mut result.content {
            match block {
                ToolContent::Text { text } => {
                    let scrubbed = self.scrub(text);
                    if scrubbed != *text {
                        redactions += 1;
                        *text = scrubbed;
                    }
                }
                ToolContent::Json { data } => {
                    let before = data.clone();
                    self.scrub_value(data);
                    if *data != before {
                        redactions += 1;
                    }
                }
                _ => {}
            }
        }
        if let Some(markdown) = result.markdown_formatted.as_mut() {
            let scrubbed = self.scrub(markdown);
            if scrubbed != *markdown {
                redactions += 1;
                *markdown = scrubbed;
            }
        }
        if redactions > 0 {
            tracing::debug!(
                redactions,
                "[mcp] redacted configured secrets from tool output"
            );
        }
        result
    }
}

fn endpoint_query(endpoint: &str) -> Option<&str> {
    let (_, rest) = endpoint.split_once('?')?;
    let query = rest.split('#').next().unwrap_or_default();
    (!query.is_empty()).then_some(query)
}

fn auth_kind(auth: &McpDefinitionAuth) -> &'static str {
    match auth {
        McpDefinitionAuth::None => "none",
        McpDefinitionAuth::BearerToken { .. } => "bearer_token",
        McpDefinitionAuth::Basic { .. } => "basic",
        McpDefinitionAuth::Header { .. } => "header",
        McpDefinitionAuth::Headers { .. } => "headers",
        McpDefinitionAuth::QueryParam { .. } => "query_param",
        _ => "unknown",
    }
}

fn endpoint_without_query(endpoint: &str) -> &str {
    endpoint
        .find(['?', '#'])
        .map_or(endpoint, |cut| &endpoint[..cut])
}

fn required_string_arg(args: &Value, key: &str) -> anyhow::Result<String> {
    let value = args
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("missing required `{key}`"))?;
    Ok(value.to_string())
}

#[cfg(test)]
#[path = "mcp_tests.rs"]
mod tests;
