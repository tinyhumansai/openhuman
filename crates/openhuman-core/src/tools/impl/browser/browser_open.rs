//! Simple TinyComputer entry point for opening a permitted page.
use crate::modules::browser::BrowserClient;
use crate::security::SecurityPolicy;
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;
use tinycomputer_bus::browser::NavigateRequest;
use tinytools::{Tool, ToolResult};

pub struct BrowserOpenTool {
    security: Arc<SecurityPolicy>,
    client: Arc<BrowserClient>,
}

impl BrowserOpenTool {
    pub fn new(security: Arc<SecurityPolicy>, client: Arc<BrowserClient>) -> Self {
        Self { security, client }
    }

    fn validate_url(&self, raw: &str) -> anyhow::Result<()> {
        let url = reqwest::Url::parse(raw)?;
        if url.scheme() != "https" {
            anyhow::bail!("Only https:// URLs are allowed");
        }
        if !url.username().is_empty() || url.password().is_some() {
            anyhow::bail!("URL userinfo is not allowed");
        }
        self.client.check_url(raw).map_err(anyhow::Error::new)
    }

    /// The client a call opens its session with: one on a fresh profile in a
    /// run whose agent's browser is view-only (`AgentExt::browser_view_only`),
    /// so such a run never reaches the persistent, signed-in profile.
    fn client_for(&self, context: Option<&dyn tinytools::ToolRunContext>) -> Arc<BrowserClient> {
        if crate::agent::tinyagents::host::run_context::browser_view_only_for_tool(context) {
            tracing::debug!("[tool.browser_open] view-only run: fresh profile");
            Arc::new(self.client.with_fresh_profile())
        } else {
            self.client.clone()
        }
    }

    async fn open(&self, args: serde_json::Value, client: &BrowserClient) -> anyhow::Result<ToolResult> {

#[async_trait]
impl Tool for BrowserOpenTool {
    fn exposure(&self) -> tinytools::ToolExposure {
        tinytools::ToolExposure::Deferred
    }
    fn name(&self) -> &str {
        "browser_open"
    }
    fn description(&self) -> &str {
        "Open an approved HTTPS URL in a one-shot TinyComputer browser session, report its title and URL, then close it. For DOM inspection or tasks, use browser action=open in the persistent conversation session."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({"type":"object","properties":{"url":{"type":"string","description":"HTTPS URL to open"}},"required":["url"]})
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        self.open(args, &self.client).await
    }
    async fn execute_with_context(
        &self,
        args: serde_json::Value,
        _options: tinytools::ToolCallOptions,
        context: Option<&dyn tinytools::ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        self.open(args, &self.client_for(context)).await
    }
}

impl BrowserOpenTool {
    async fn open_url(&self, args: serde_json::Value, client: &BrowserClient) -> anyhow::Result<ToolResult> {
        let url = args["url"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'url' parameter"))?;
        if !self.security.can_act() {
            return Ok(ToolResult::error(format!(
                "{} Action blocked: autonomy is read-only",
                tinytools::status::POLICY_BLOCKED_MARKER
            )));
        }
        if let Err(error) = self.validate_url(url) {
            return Ok(ToolResult::error(error.to_string()));
        }
        let session = client.open_session_for_url(url).await?;
        let result = client.navigate(&session.id, NavigateRequest::new(url)).await;
        let close = client.close_session(&session.id).await;
        match (result, close) {
            (Ok(page), Ok(())) => Ok(ToolResult::success(serde_json::to_string_pretty(&page)?)),
            (Err(error), _) => Ok(ToolResult::error(error.to_string())),
            (_, Err(error)) => Ok(ToolResult::error(error.to_string())),
        }
    }
}

#[cfg(test)]
#[path = "browser_open_tests.rs"]
mod tests;
