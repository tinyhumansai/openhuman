
#[async_trait]
impl Tool for ParallelResearchTool {
    fn name(&self) -> &str {
        "parallel_research"
    }

    fn description(&self) -> &str {
        "Deep web research via Parallel's Task API. Submit an objective and a processor \
         tier (`lite`, `base`, `core`, `ultra`) — Parallel browses many sources, \
         synthesises, and returns a single rich answer. Optionally pass an \
         `output_schema` (JSON schema) to force structured output. \
         Blocks inline until the run completes (up to ~10 minutes). \
         Use for tasks that need more than a single search/extract pair, e.g. \
         \"compare these three companies' financials\" or \"build a competitor matrix\"."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "input": {
                    "description": "The research objective — string or structured object",
                    "oneOf": [{ "type": "string" }, { "type": "object" }]
                },
                "processor": {
                    "type": "string",
                    "enum": ["lite", "base", "core", "ultra"],
                    "description": "Processor tier — lite (cheapest) → ultra (most thorough)"
                },
                "output_schema": {
                    "type": "object",
                    "description": "Optional JSON schema describing the desired structured output"
                },
                "timeout_seconds": {
                    "type": "integer",
                    "minimum": 10,
                    "maximum": 900,
                    "description": "Max time to wait inline (default 600)"
                }
            },
            "required": ["input", "processor"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let input = args
            .get("input")
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: input"))?;
        let processor = args
            .get("processor")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: processor"))?;

        let mut body = json!({
            "input": input,
            "processor": processor,
            "wait": true,
        });
        if let Some(schema) = args.get("output_schema") {
            body["outputSchema"] = schema.clone();
        }
        if let Some(t) = args.get("timeout_seconds").and_then(|v| v.as_u64()) {
            body["timeoutSeconds"] = json!(t.clamp(10, 900));
        }

        tracing::info!("[parallel_research] processor={}", processor);

        match self
            .client
            .post::<ResearchResponse>("/agent-integrations/parallel/research", &body)
            .await
        {
            Ok(resp) => {
                let display = match format_research_response(ResearchResponse {
                    run_id: resp.run_id.clone(),
                    status: resp.status.clone(),
                    result: resp.result.clone(),
                    cost_usd: resp.cost_usd,
                }) {
                    Ok(display) => display,
                    Err(message) => return Ok(ToolResult::error(message)),
                };
                Ok(ToolResult::success_with_markdown(
                    research_payload(&resp, &display),
                    display,
                ))
            }
            Err(e) => Ok(ToolResult::error(format!("Parallel research failed: {e}"))),
        }
    }
}

// ── ParallelEnrichTool ──────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct EnrichResponse {
    #[serde(default, rename = "runId")]
    run_id: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    output: Option<serde_json::Value>,
    #[serde(rename = "costUsd", default)]
    cost_usd: f64,
}

fn format_enrich_response(resp: EnrichResponse) -> Result<String, String> {
    if let Some(id) = &resp.run_id {
        tracing::debug!(
            "[parallel_enrich] completed run_id={} status={:?} cost_usd={:.4}",
            id,
            resp.status,
            resp.cost_usd
        );
    } else {
        tracing::debug!(
            "[parallel_enrich] completed without run_id status={:?} cost_usd={:.4}",
            resp.status,
            resp.cost_usd
        );
    }

    let mut out = String::new();
    if let Some(s) = &resp.status {
        out.push_str(&format!("Status: {}\n", s));
    }
    let Some(o) = resp.output else {
        let status = resp.status.as_deref().unwrap_or("unknown");
        tracing::debug!(
            "[parallel_enrich] incomplete blocking response status={} cost_usd={:.4}",
            status,
            resp.cost_usd
        );
        return Err(format!(
            "Parallel enrich did not return output before the inline wait completed (status: {status}). Try again with a higher timeout_seconds or a cheaper processor."
        ));
    };
    out.push_str("\nOutput:\n");
    out.push_str(&serde_json::to_string_pretty(&o).unwrap_or_default());
    out.push_str(&format!("\n\nCost: ${:.4}", resp.cost_usd));
    Ok(out)
}

fn enrich_payload(resp: &EnrichResponse, display: &str) -> serde_json::Value {
    json!({
        "display": display,
        "status": resp.status,
        "output": resp.output,
        "cost_usd": resp.cost_usd,
    })
}

/// Enrich an entity with structured web data — synchronous Task API run
/// with a required output schema.
pub struct ParallelEnrichTool {
    client: Arc<IntegrationClient>,
}

impl ParallelEnrichTool {
    pub fn new(client: Arc<IntegrationClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for ParallelEnrichTool {
    fn name(&self) -> &str {
        "parallel_enrich"
    }

    fn description(&self) -> &str {
        "Enrich an entity (company, person, product) with structured web data. \
         Pass an `input` (the thing to enrich) plus a JSON `output_schema` describing \
         the fields you want filled in — Parallel returns a structured object \
         conforming to that schema. Blocks until the run completes."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "input": {
                    "description": "Entity to enrich — string or object",
                    "oneOf": [{ "type": "string" }, { "type": "object" }]
                },
                "processor": {
                    "type": "string",
                    "enum": ["lite", "base", "core", "ultra"],
                    "description": "Processor tier — lite (cheapest) → ultra (most thorough)"
                },
                "output_schema": {
                    "type": "object",
                    "description": "JSON schema for the structured output (required)"
                },
                "timeout_seconds": {
                    "type": "integer",
                    "minimum": 10,
                    "maximum": 900,
                    "description": "Max time to wait (default 600)"
                }
            },
            "required": ["input", "processor", "output_schema"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let input = args
            .get("input")
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: input"))?;
        let processor = args
            .get("processor")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: processor"))?;
        let output_schema = args
            .get("output_schema")
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: output_schema"))?;

        let mut body = json!({
            "input": input,
            "processor": processor,
            "outputSchema": output_schema,
        });
        if let Some(t) = args.get("timeout_seconds").and_then(|v| v.as_u64()) {
            body["timeoutSeconds"] = json!(t.clamp(10, 900));
        }

        tracing::info!("[parallel_enrich] processor={}", processor);

        match self
            .client
            .post::<EnrichResponse>("/agent-integrations/parallel/enrich", &body)
            .await
        {
            Ok(resp) => {
                let display = match format_enrich_response(EnrichResponse {
                    run_id: resp.run_id.clone(),
                    status: resp.status.clone(),
                    output: resp.output.clone(),
                    cost_usd: resp.cost_usd,
                }) {
                    Ok(display) => display,
                    Err(message) => return Ok(ToolResult::error(message)),
                };
                Ok(ToolResult::success_with_markdown(
                    enrich_payload(&resp, &display),
                    display,
                ))
            }
            Err(e) => Ok(ToolResult::error(format!("Parallel enrich failed: {e}"))),
        }
    }
}

// ── ParallelDatasetTool ─────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct DatasetResponse {
    #[serde(rename = "findallId", default)]
    findall_id: String,
    #[serde(default)]
    status: serde_json::Value,
    #[serde(rename = "matchLimit", default)]
    match_limit: u64,
    #[serde(rename = "costUsd", default)]
    cost_usd: f64,
}

/// Generate a web dataset via Parallel's FindAll — kicks off an async run
/// that produces structured candidate matches.
pub struct ParallelDatasetTool {
    client: Arc<IntegrationClient>,
}

impl ParallelDatasetTool {
    pub fn new(client: Arc<IntegrationClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for ParallelDatasetTool {
    fn name(&self) -> &str {
        "parallel_dataset"
    }

    fn description(&self) -> &str {
        "Generate a web dataset via Parallel FindAll. Describe an `objective`, \
         the `entity_type` you want (e.g. \"SaaS company\", \"academic paper\"), \
         and a list of `match_conditions` — each a `name` plus an optional \
         `description`. Parallel discovers and enriches matching candidates \
         in the background. This call returns the run ID and pre-authorised cost; \
         use `match_limit` to cap how many candidates are produced. \
         Run is async — fetch results separately by `findall_id`."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "objective": { "type": "string", "description": "What dataset to build" },
                "entity_type": { "type": "string", "description": "What kind of entity to find" },
                "match_conditions": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": 20,
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": { "type": "string" },
                            "description": { "type": "string" }
                        },
                        "required": ["name"]
                    }
                },
                "generator": {
                    "type": "string",
                    "enum": ["preview", "base", "core", "pro"],
                    "description": "Generator tier (default base)"
                },
                "match_limit": {
                    "type": "integer",
                    "minimum": 5,
                    "maximum": 1000,
                    "description": "Max candidates to produce (default 10)"
                }
            },
            "required": ["objective", "entity_type", "match_conditions"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let objective = args
            .get("objective")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: objective"))?;
        let entity_type = args
            .get("entity_type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: entity_type"))?;
        let match_conditions = args
            .get("match_conditions")
            .and_then(|v| v.as_array())
            .filter(|a| !a.is_empty())
            .ok_or_else(|| anyhow::anyhow!("match_conditions must be a non-empty array"))?;

        let mut body = json!({
            "objective": objective,
            "entityType": entity_type,
            "matchConditions": match_conditions,
        });
        if let Some(g) = args.get("generator").and_then(|v| v.as_str()) {
            body["generator"] = json!(g);
        }
        if let Some(l) = args.get("match_limit").and_then(|v| v.as_u64()) {
            body["matchLimit"] = json!(l.clamp(5, 1000));
        }

        tracing::info!("[parallel_dataset] entity_type={}", entity_type);

        match self
            .client
            .post::<DatasetResponse>("/agent-integrations/parallel/dataset", &body)
            .await
        {
            Ok(resp) => {
                let out = format!(
                    "Dataset run started\n  findall_id: {}\n  match_limit: {}\n  status: {}\n\nCost (pre-authorised): ${:.4}\n\nResults are produced asynchronously — fetch them later by findall_id.",
                    resp.findall_id,
                    resp.match_limit,
                    serde_json::to_string(&resp.status).unwrap_or_default(),
                    resp.cost_usd
                );
                Ok(ToolResult::success(out))
            }
            Err(e) => Ok(ToolResult::error(format!("Parallel dataset failed: {e}"))),
        }
    }
}
