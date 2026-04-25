//! Controller schemas for the `tools` namespace.
//!
//! Exposes a small allowlist of tool-like operations to the Tauri shell
//! over JSON-RPC. The Tauri host needs these so the onboarding flow can
//! drive Composio + Parallel-backed web search itself (orchestration in
//! the renderer; external calls still go through the core's auth / proxy
//! layer). Anything **not** in this file remains agent-only.

use serde_json::{json, Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;
use crate::rpc::RpcOutcome;

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        tools_schemas("tools_composio_execute"),
        tools_schemas("tools_web_search"),
        tools_schemas("tools_apify_linkedin_scrape"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: tools_schemas("tools_composio_execute"),
            handler: handle_composio_execute,
        },
        RegisteredController {
            schema: tools_schemas("tools_web_search"),
            handler: handle_web_search,
        },
        RegisteredController {
            schema: tools_schemas("tools_apify_linkedin_scrape"),
            handler: handle_apify_linkedin_scrape,
        },
    ]
}

pub fn tools_schemas(function: &str) -> ControllerSchema {
    match function {
        "tools_composio_execute" => ControllerSchema {
            namespace: "tools",
            function: "composio_execute",
            description: "Execute a Composio action via the backend proxy. Thin wrapper \
                          around `ComposioClient::execute_tool` exposed for Tauri-driven \
                          flows (e.g. onboarding) that orchestrate tool calls themselves.",
            inputs: vec![
                FieldSchema {
                    name: "action",
                    ty: TypeSchema::String,
                    comment: "Composio action slug (e.g. `GMAIL_FETCH_EMAILS`).",
                    required: true,
                },
                FieldSchema {
                    name: "params",
                    ty: TypeSchema::Json,
                    comment: "Action parameters object passed straight through to Composio.",
                    required: false,
                },
            ],
            outputs: vec![
                FieldSchema {
                    name: "successful",
                    ty: TypeSchema::Bool,
                    comment: "Whether the upstream provider reported success.",
                    required: true,
                },
                FieldSchema {
                    name: "data",
                    ty: TypeSchema::Json,
                    comment: "Raw provider response.",
                    required: true,
                },
                FieldSchema {
                    name: "error",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Provider error message if `successful` is false.",
                    required: false,
                },
            ],
        },
        "tools_web_search" => ControllerSchema {
            namespace: "tools",
            function: "web_search",
            description: "Web search via the backend Parallel proxy. Returns structured \
                          results so callers can inspect titles, URLs, and excerpts \
                          without parsing the agent-facing pretty text.",
            inputs: vec![
                FieldSchema {
                    name: "query",
                    ty: TypeSchema::String,
                    comment: "Search query string.",
                    required: true,
                },
                FieldSchema {
                    name: "objective",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Optional objective sent to Parallel (defaults to `query`).",
                    required: false,
                },
                FieldSchema {
                    name: "max_results",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Max results (1-10, default 5).",
                    required: false,
                },
                FieldSchema {
                    name: "timeout_secs",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Request timeout in seconds (default 15).",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "results",
                ty: TypeSchema::Array(Box::new(TypeSchema::Json)),
                comment: "Each item: {url, title, publish_date?, excerpts[]}.",
                required: true,
            }],
        },
        "tools_apify_linkedin_scrape" => ControllerSchema {
            namespace: "tools",
            function: "apify_linkedin_scrape",
            description: "Run the Apify LinkedIn profile scraper actor on a single profile \
                          URL and return both the raw scraped item and a pre-rendered \
                          markdown view of it (same layout as the legacy enrichment pipeline).",
            inputs: vec![FieldSchema {
                name: "profile_url",
                ty: TypeSchema::String,
                comment: "Canonical LinkedIn profile URL (`https://www.linkedin.com/in/<slug>`).",
                required: true,
            }],
            outputs: vec![
                FieldSchema {
                    name: "data",
                    ty: TypeSchema::Json,
                    comment: "Raw scraped profile JSON from Apify.",
                    required: true,
                },
                FieldSchema {
                    name: "markdown",
                    ty: TypeSchema::String,
                    comment: "Markdown rendering of the scraped profile (full, pre-summary).",
                    required: true,
                },
            ],
        },
        _ => ControllerSchema {
            namespace: "tools",
            function: "unknown",
            description: "Unknown tools controller.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "error",
                ty: TypeSchema::String,
                comment: "Lookup error details.",
                required: true,
            }],
        },
    }
}

fn handle_composio_execute(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let action = params
            .get("action")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| "missing required `action`".to_string())?;
        let action_args = params.get("params").cloned();

        let config = config_rpc::load_config_with_timeout().await?;
        let client = crate::openhuman::composio::client::build_composio_client(&config)
            .ok_or_else(|| {
                "composio client unavailable — user not signed in to backend".to_string()
            })?;

        let resp = client
            .execute_tool(&action, action_args)
            .await
            .map_err(|e| format!("composio execute_tool failed: {e:#}"))?;

        let payload = json!({
            "successful": resp.successful,
            "data": resp.data,
            "error": resp.error,
            "cost_usd": resp.cost_usd,
        });
        let log = vec![format!(
            "tools.composio_execute: action={action} successful={}",
            resp.successful
        )];
        RpcOutcome::new(payload, log).into_cli_compatible_json()
    })
}

fn handle_web_search(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let query = params
            .get("query")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .ok_or_else(|| "missing or empty `query`".to_string())?;
        let objective = params
            .get("objective")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| query.clone());
        let max_results = params
            .get("max_results")
            .and_then(Value::as_u64)
            .map(|n| n.clamp(1, 10) as usize)
            .unwrap_or(5);
        let timeout_secs = params
            .get("timeout_secs")
            .and_then(Value::as_u64)
            .map(|n| n.max(1))
            .unwrap_or(15);

        let config = config_rpc::load_config_with_timeout().await?;
        let client = crate::openhuman::integrations::build_client(&config).ok_or_else(|| {
            "web search unavailable — no backend session token. Sign in first.".to_string()
        })?;

        // Body matches `parallelSearchSchema` (backend-2/.../validators/agentIntegration.validator.ts).
        // `timeout_secs` remains accepted in our RPC schema for compatibility
        // with existing callers, but the upstream validator currently strips
        // unknown keys and Parallel governs its own per-mode deadline.
        let _ = timeout_secs;
        let body = json!({
            "objective": objective,
            "searchQueries": [query],
            "mode": "fast",
            "excerpts": {
                "maxResults": max_results,
                "maxCharsPerResult": 500
            }
        });

        let resp = client
            .post::<crate::openhuman::integrations::parallel::SearchResponse>(
                "/agent-integrations/parallel/search",
                &body,
            )
            .await
            .map_err(|e| format!("parallel search failed: {e:#}"))?;

        let count = resp.results.len();
        let payload = json!({ "results": resp.results });
        let log = vec![format!(
            "tools.web_search: query=\"{query}\" results={count}"
        )];
        RpcOutcome::new(payload, log).into_cli_compatible_json()
    })
}

fn handle_apify_linkedin_scrape(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let profile_url = params
            .get("profile_url")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .ok_or_else(|| "missing or empty `profile_url`".to_string())?;

        let config = config_rpc::load_config_with_timeout().await?;
        let client = crate::openhuman::integrations::build_client(&config).ok_or_else(|| {
            "Apify scrape unavailable — no backend session token. Sign in first.".to_string()
        })?;

        let data = crate::openhuman::learning::linkedin_enrichment::scrape_linkedin_profile(
            &client,
            &profile_url,
        )
        .await
        .map_err(|e| format!("Apify LinkedIn scrape failed: {e:#}"))?;

        let markdown = crate::openhuman::learning::linkedin_enrichment::render_profile_markdown(
            &profile_url,
            &data,
        );

        let payload = json!({ "data": data, "markdown": markdown });
        let log = vec![format!(
            "tools.apify_linkedin_scrape: url={profile_url} markdown_chars={}",
            markdown.chars().count()
        )];
        RpcOutcome::new(payload, log).into_cli_compatible_json()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_schemas_returns_three() {
        assert_eq!(all_controller_schemas().len(), 3);
    }

    #[test]
    fn all_controllers_returns_three() {
        assert_eq!(all_registered_controllers().len(), 3);
    }

    #[test]
    fn apify_linkedin_scrape_schema_shape() {
        let s = tools_schemas("tools_apify_linkedin_scrape");
        assert_eq!(s.namespace, "tools");
        assert_eq!(s.function, "apify_linkedin_scrape");
        assert!(s
            .inputs
            .iter()
            .any(|f| f.name == "profile_url" && f.required));
    }

    #[test]
    fn composio_execute_schema_shape() {
        let s = tools_schemas("tools_composio_execute");
        assert_eq!(s.namespace, "tools");
        assert_eq!(s.function, "composio_execute");
        assert!(s.inputs.iter().any(|f| f.name == "action" && f.required));
    }

    #[test]
    fn web_search_schema_shape() {
        let s = tools_schemas("tools_web_search");
        assert_eq!(s.namespace, "tools");
        assert_eq!(s.function, "web_search");
        assert!(s.inputs.iter().any(|f| f.name == "query" && f.required));
    }

    #[test]
    fn unknown_function_returns_unknown() {
        let s = tools_schemas("nonexistent");
        assert_eq!(s.function, "unknown");
    }
}
