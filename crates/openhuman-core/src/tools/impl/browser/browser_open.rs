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
}

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
        // The tier and the hourly budget are TinySecurity's, enforced once per call by
        // the tool gate before this runs.
        let url = args["url"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'url' parameter"))?;
        if let Err(error) = self.validate_url(url) {
            return Ok(ToolResult::error(error.to_string()));
        }
        let session = self.client.open_session_for_url(url).await?;
        let result = self
            .client
            .navigate(&session.id, NavigateRequest::new(url))
            .await;
        let close = self.client.close_session(&session.id).await;
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
