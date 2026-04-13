//! Apify actor execution and dataset retrieval integration tools.
//!
//! **Scope**: All (agent loop + CLI/RPC).
//!
//! **Endpoints**:
//!   - `POST /agent-integrations/apify/run`
//!   - `GET /agent-integrations/apify/runs/{runId}`
//!   - `GET /agent-integrations/apify/runs/{runId}/results`
//!
//! Apify runs can be synchronous or asynchronous. The run tool starts an actor
//! and can optionally wait for completion; the status/results tools let the
//! caller poll long-running jobs and fetch the final dataset.

use super::IntegrationClient;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolCategory, ToolResult};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

#[derive(Debug, Deserialize)]
struct ApifyRunResponse {
    #[serde(rename = "runId", default)]
    run_id: String,
    #[serde(rename = "actorId", default)]
    actor_id: String,
    #[serde(default)]
    status: String,
    #[serde(rename = "datasetId", default)]
    dataset_id: Option<String>,
    #[serde(default)]
    items: Option<Vec<serde_json::Value>>,
    #[serde(rename = "costUsd", default)]
    cost_usd: f64,
}

#[derive(Debug, Deserialize)]
struct ApifyGetRunResultsResponse {
    #[serde(default)]
    items: Vec<serde_json::Value>,
    #[serde(default)]
    total: u64,
}

fn summarize_json_array(items: &[serde_json::Value], max_items: usize) -> String {
    items
        .iter()
        .take(max_items)
        .enumerate()
        .map(|(idx, item)| format!("{}. {}", idx + 1, item))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Start an Apify actor run for scraping or data-collection workflows.
pub struct ApifyRunActorTool {
    client: Arc<IntegrationClient>,
}

impl ApifyRunActorTool {
    pub fn new(client: Arc<IntegrationClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for ApifyRunActorTool {
    fn name(&self) -> &str {
        "apify_run_actor"
    }

    fn description(&self) -> &str {
        "Run an Apify actor with a JSON input payload. Use this for hosted \
         scrapers, crawlers, and data-collection jobs. Set `sync=true` to wait \
         for completion or `sync=false` for long-running jobs, then poll with \
         apify_get_run_status and fetch results with apify_get_run_results."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "actor_id": {
                    "type": "string",
                    "description": "Apify actor ID or actor slug to execute"
                },
                "input": {
                    "type": "object",
                    "description": "Actor input JSON object passed through to Apify"
                },
                "sync": {
                    "type": "boolean",
                    "description": "Wait for the actor to finish before returning (default true)",
                    "default": true
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Max seconds to wait when sync=true (1-3600, default 120)",
                    "minimum": 1,
                    "maximum": 3600,
                    "default": 120
                },
                "memory_mbytes": {
                    "type": "integer",
                    "description": "Optional Apify memory allocation in MB (128-32768)",
                    "minimum": 128,
                    "maximum": 32768
                }
            },
            "required": ["actor_id", "input"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Skill
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let actor_id = args
            .get("actor_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: actor_id"))?;
        if actor_id.trim().is_empty() {
            return Ok(ToolResult::error("actor_id cannot be empty"));
        }

        let Some(input) = args.get("input") else {
            return Err(anyhow::anyhow!("Missing required parameter: input"));
        };
        if !input.is_object() {
            return Ok(ToolResult::error("input must be a JSON object"));
        }

        let sync = args.get("sync").and_then(|v| v.as_bool()).unwrap_or(true);
        let timeout_secs = args
            .get("timeout_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(120)
            .clamp(1, 3600);
        let memory_mbytes = args
            .get("memory_mbytes")
            .and_then(|v| v.as_u64())
            .map(|v| v.clamp(128, 32768));

        let mut body = json!({
            "actorId": actor_id,
            "input": input,
            "sync": sync,
            "timeoutSecs": timeout_secs,
        });
        if let Some(memory_mbytes) = memory_mbytes {
            body["memoryMbytes"] = json!(memory_mbytes);
        }

        tracing::info!(
            actor_id = actor_id,
            sync = sync,
            timeout_secs = timeout_secs,
            has_memory_override = memory_mbytes.is_some(),
            "[apify_run_actor] starting actor run"
        );

        match self
            .client
            .post::<ApifyRunResponse>("/agent-integrations/apify/run", &body)
            .await
        {
            Ok(resp) => {
                let mut lines = vec![
                    format!("Apify run started for actor: {}", resp.actor_id),
                    format!("Run ID: {}", resp.run_id),
                    format!("Status: {}", resp.status),
                ];

                if let Some(dataset_id) = resp.dataset_id.as_deref() {
                    lines.push(format!("Dataset ID: {}", dataset_id));
                }

                if let Some(items) = resp.items.as_ref() {
                    lines.push(format!("Returned {} result item(s).", items.len()));
                    if !items.is_empty() {
                        lines.push("Sample results:".to_string());
                        lines.push(summarize_json_array(items, 3));
                    }
                } else if !sync {
                    lines.push(
                        "This run is still in progress. Poll with apify_get_run_status."
                            .to_string(),
                    );
                }

                lines.push(format!("Cost: ${:.4}", resp.cost_usd));
                Ok(ToolResult::success(lines.join("\n")))
            }
            Err(e) => Ok(ToolResult::error(format!("Apify actor run failed: {e}"))),
        }
    }
}

/// Fetch the current status for an existing Apify actor run.
pub struct ApifyGetRunStatusTool {
    client: Arc<IntegrationClient>,
}

impl ApifyGetRunStatusTool {
    pub fn new(client: Arc<IntegrationClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for ApifyGetRunStatusTool {
    fn name(&self) -> &str {
        "apify_get_run_status"
    }

    fn description(&self) -> &str {
        "Get the status of a previously started Apify run. Use this to poll \
         long-running actor jobs until they reach SUCCEEDED, FAILED, TIMED-OUT, \
         or ABORTED."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "run_id": {
                    "type": "string",
                    "description": "Apify run ID returned by apify_run_actor"
                }
            },
            "required": ["run_id"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Skill
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let run_id = args
            .get("run_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: run_id"))?;
        if run_id.trim().is_empty() {
            return Ok(ToolResult::error("run_id cannot be empty"));
        }

        let path = format!("/agent-integrations/apify/runs/{run_id}");
        tracing::debug!(
            run_id = run_id,
            "[apify_get_run_status] fetching run status"
        );

        match self.client.get::<ApifyRunResponse>(&path).await {
            Ok(resp) => {
                let mut lines = vec![
                    format!("Run ID: {}", resp.run_id),
                    format!("Actor ID: {}", resp.actor_id),
                    format!("Status: {}", resp.status),
                ];
                if let Some(dataset_id) = resp.dataset_id.as_deref() {
                    lines.push(format!("Dataset ID: {}", dataset_id));
                }
                lines.push(format!("Cost: ${:.4}", resp.cost_usd));
                Ok(ToolResult::success(lines.join("\n")))
            }
            Err(e) => Ok(ToolResult::error(format!(
                "Apify get run status failed: {e}"
            ))),
        }
    }
}

/// Fetch dataset items for a completed Apify actor run.
pub struct ApifyGetRunResultsTool {
    client: Arc<IntegrationClient>,
}

impl ApifyGetRunResultsTool {
    pub fn new(client: Arc<IntegrationClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for ApifyGetRunResultsTool {
    fn name(&self) -> &str {
        "apify_get_run_results"
    }

    fn description(&self) -> &str {
        "Fetch dataset items from a completed Apify run. Supports optional \
         pagination with `limit` and `offset`."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "run_id": {
                    "type": "string",
                    "description": "Apify run ID returned by apify_run_actor"
                },
                "limit": {
                    "type": "integer",
                    "description": "Max dataset items to return (1-1000)",
                    "minimum": 1,
                    "maximum": 1000
                },
                "offset": {
                    "type": "integer",
                    "description": "Pagination offset (0-100000)",
                    "minimum": 0,
                    "maximum": 100000
                }
            },
            "required": ["run_id"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Skill
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let run_id = args
            .get("run_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: run_id"))?;
        if run_id.trim().is_empty() {
            return Ok(ToolResult::error("run_id cannot be empty"));
        }

        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|v| v.clamp(1, 1000));
        let offset = args
            .get("offset")
            .and_then(|v| v.as_u64())
            .map(|v| v.clamp(0, 100000));

        let mut path = format!("/agent-integrations/apify/runs/{run_id}/results");
        if limit.is_some() || offset.is_some() {
            let mut query = Vec::new();
            if let Some(limit) = limit {
                query.push(format!("limit={limit}"));
            }
            if let Some(offset) = offset {
                query.push(format!("offset={offset}"));
            }
            path.push('?');
            path.push_str(&query.join("&"));
        }

        tracing::debug!(
            run_id = run_id,
            limit = limit,
            offset = offset,
            "[apify_get_run_results] fetching dataset items"
        );

        match self.client.get::<ApifyGetRunResultsResponse>(&path).await {
            Ok(resp) => {
                if resp.items.is_empty() {
                    return Ok(ToolResult::success(format!(
                        "No dataset items found for run {}",
                        run_id
                    )));
                }

                let mut lines = vec![
                    format!("Fetched {} dataset item(s).", resp.items.len()),
                    format!("Total available: {}", resp.total),
                    "Sample results:".to_string(),
                    summarize_json_array(&resp.items, 5),
                ];

                if resp.items.len() > 5 {
                    lines.push("Output truncated to the first 5 items.".to_string());
                }

                Ok(ToolResult::success(lines.join("\n")))
            }
            Err(e) => Ok(ToolResult::error(format!(
                "Apify get run results failed: {e}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_client() -> Arc<IntegrationClient> {
        Arc::new(IntegrationClient::new(
            "http://test.example".into(),
            "tok".into(),
        ))
    }

    #[test]
    fn run_tool_metadata() {
        let tool = ApifyRunActorTool::new(test_client());
        assert_eq!(tool.name(), "apify_run_actor");
        assert_eq!(tool.permission_level(), PermissionLevel::Execute);
        assert_eq!(tool.category(), ToolCategory::Skill);
        assert!(tool.description().contains("Apify actor"));
    }

    #[test]
    fn run_tool_schema_has_required_fields() {
        let tool = ApifyRunActorTool::new(test_client());
        let schema = tool.parameters_schema();
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "actor_id"));
        assert!(required.iter().any(|v| v == "input"));
    }

    #[tokio::test]
    async fn run_tool_rejects_missing_actor_id() {
        let tool = ApifyRunActorTool::new(test_client());
        let result = tool.execute(json!({"input": {}})).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn run_tool_rejects_empty_actor_id() {
        let tool = ApifyRunActorTool::new(test_client());
        let result = tool
            .execute(json!({"actor_id": "", "input": {}}))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("actor_id"));
    }

    #[tokio::test]
    async fn run_tool_rejects_non_object_input() {
        let tool = ApifyRunActorTool::new(test_client());
        let result = tool
            .execute(json!({"actor_id": "apify/web-scraper", "input": []}))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("input must be a JSON object"));
    }

    #[test]
    fn status_tool_metadata() {
        let tool = ApifyGetRunStatusTool::new(test_client());
        assert_eq!(tool.name(), "apify_get_run_status");
        assert_eq!(tool.category(), ToolCategory::Skill);
    }

    #[tokio::test]
    async fn status_tool_rejects_empty_run_id() {
        let tool = ApifyGetRunStatusTool::new(test_client());
        let result = tool.execute(json!({"run_id": ""})).await.unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("run_id"));
    }

    #[test]
    fn results_tool_schema_supports_pagination() {
        let tool = ApifyGetRunResultsTool::new(test_client());
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["limit"].is_object());
        assert!(schema["properties"]["offset"].is_object());
    }

    #[tokio::test]
    async fn results_tool_rejects_empty_run_id() {
        let tool = ApifyGetRunResultsTool::new(test_client());
        let result = tool.execute(json!({"run_id": ""})).await.unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("run_id"));
    }

    #[test]
    fn run_response_deserializes() {
        let json = r#"{
            "runId":"run-123",
            "actorId":"apify/web-scraper",
            "status":"SUCCEEDED",
            "datasetId":"dataset-123",
            "items":[{"url":"https://example.com"}],
            "costUsd":0.3
        }"#;
        let resp: ApifyRunResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.run_id, "run-123");
        assert_eq!(resp.actor_id, "apify/web-scraper");
        assert_eq!(resp.status, "SUCCEEDED");
        assert_eq!(resp.dataset_id.as_deref(), Some("dataset-123"));
        assert_eq!(resp.items.unwrap().len(), 1);
        assert!((resp.cost_usd - 0.3).abs() < f64::EPSILON);
    }

    #[test]
    fn results_response_deserializes() {
        let json = r#"{"items":[{"foo":"bar"}],"total":42}"#;
        let resp: ApifyGetRunResultsResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.items.len(), 1);
        assert_eq!(resp.total, 42);
    }
}
