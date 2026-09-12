use serde_json::{json, Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::search::tools::SEARXNG_MAX_RESULTS;
use crate::openhuman::tools::traits::Tool;
use crate::rpc::RpcOutcome;

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        tools_schemas("tools_composio_execute"),
        tools_schemas("tools_web_search"),
        tools_schemas("tools_seltz_search"),
        tools_schemas("tools_querit_search"),
        tools_schemas("tools_searxng_search"),
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
            schema: tools_schemas("tools_seltz_search"),
            handler: handle_seltz_search,
        },
        RegisteredController {
            schema: tools_schemas("tools_querit_search"),
            handler: handle_querit_search,
        },
        RegisteredController {
            schema: tools_schemas("tools_searxng_search"),
            handler: handle_searxng_search,
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
            description: "Execute a Composio action. Routes through the mode-aware \
                          factory: backend mode proxies via the OpenHuman backend; \
                          direct mode calls backend.composio.dev with the user's own \
                          API key. Exposed for Tauri-driven flows (e.g. onboarding) \
                          that orchestrate tool calls themselves.",
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
            outputs: vec![
                FieldSchema {
                    name: "results",
                    ty: TypeSchema::Array(Box::new(TypeSchema::Json)),
                    comment: "Each item: {url, title, publish_date?, excerpts[]}.",
                    required: true,
                },
                FieldSchema {
                    name: "provider",
                    ty: TypeSchema::String,
                    comment: "Upstream provider the managed backend resolved this \
                              search to, for attribution. Reported by the backend when \
                              it names one; falls back to the managed default.",
                    required: true,
                },
            ],
        },
        "tools_seltz_search" => ControllerSchema {
            namespace: "tools",
            function: "seltz_search",
            description: "Web search via the Seltz API. Returns structured results with \
                          URLs, content, and optional published dates. Supports domain \
                          filtering, date ranges, and news scope.",
            inputs: vec![
                FieldSchema {
                    name: "query",
                    ty: TypeSchema::String,
                    comment: "Search query string.",
                    required: true,
                },
                FieldSchema {
                    name: "max_results",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Max results (1-20, default 10).",
                    required: false,
                },
                FieldSchema {
                    name: "include_domains",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Array(Box::new(
                        TypeSchema::String,
                    )))),
                    comment: "Restrict results to these domains.",
                    required: false,
                },
                FieldSchema {
                    name: "exclude_domains",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Array(Box::new(
                        TypeSchema::String,
                    )))),
                    comment: "Exclude results from these domains.",
                    required: false,
                },
                FieldSchema {
                    name: "from_date",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Only results published on or after (YYYY-MM-DD).",
                    required: false,
                },
                FieldSchema {
                    name: "to_date",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Only results published on or before (YYYY-MM-DD).",
                    required: false,
                },
                FieldSchema {
                    name: "scope",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Restrict to a scope, e.g. \"news\".",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "documents",
                ty: TypeSchema::Array(Box::new(TypeSchema::Json)),
                comment: "Each item: {url, content, title?, published_date?}.",
                required: true,
            }],
        },
        "tools_querit_search" => ControllerSchema {
            namespace: "tools",
            function: "querit_search",
            description: "Web search via the Querit API. Returns current results with URLs, \
                          snippets, site names, and page age. Supports site filters, \
                          time ranges, country filters, and language filters.",
            inputs: vec![
                FieldSchema {
                    name: "query",
                    ty: TypeSchema::String,
                    comment: "Search query string.",
                    required: true,
                },
                FieldSchema {
                    name: "max_results",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Max results (1-20, default 10).",
                    required: false,
                },
                FieldSchema {
                    name: "count",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Querit-native alias for max_results.",
                    required: false,
                },
                FieldSchema {
                    name: "filters",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment:
                        "Querit-native filters object with sites, timeRange, geo, and languages.",
                    required: false,
                },
                FieldSchema {
                    name: "include_domains",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Array(Box::new(
                        TypeSchema::String,
                    )))),
                    comment: "Only fetch results from these domains.",
                    required: false,
                },
                FieldSchema {
                    name: "exclude_domains",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Array(Box::new(
                        TypeSchema::String,
                    )))),
                    comment: "Exclude results from these domains.",
                    required: false,
                },
                FieldSchema {
                    name: "time_range",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Querit date filter: d7, w2, m6, y1, or YYYY-MM-DDtoYYYY-MM-DD.",
                    required: false,
                },
                FieldSchema {
                    name: "from_date",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Start date for a Querit date-range filter (YYYY-MM-DD).",
                    required: false,
                },
                FieldSchema {
                    name: "to_date",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "End date for a Querit date-range filter (YYYY-MM-DD).",
                    required: false,
                },
                FieldSchema {
                    name: "countries",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Array(Box::new(
                        TypeSchema::String,
                    )))),
                    comment: "Country filters, e.g. united states, japan, germany.",
                    required: false,
                },
                FieldSchema {
                    name: "languages",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Array(Box::new(
                        TypeSchema::String,
                    )))),
                    comment: "Language filters, e.g. english, japanese, german.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "results",
                ty: TypeSchema::String,
                comment: "Formatted Querit search results.",
                required: true,
            }],
        },
        "tools_searxng_search" => ControllerSchema {
            namespace: "tools",
            function: "searxng_search",
            description:
                "Web search via a user-configured SearXNG instance. Returns normalized \
                          results with title, URL, snippet, and source. Intended for private, \
                          self-hosted search without routing queries through the OpenHuman backend.",
            inputs: vec![
                FieldSchema {
                    name: "query",
                    ty: TypeSchema::String,
                    comment: "Search query string.",
                    required: true,
                },
                FieldSchema {
                    name: "categories",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Array(Box::new(
                        TypeSchema::Enum {
                            variants: vec!["web", "general", "news", "images"],
                        },
                    )))),
                    comment: "Optional SearXNG categories. `web` maps to SearXNG `general`.",
                    required: false,
                },
                FieldSchema {
                    name: "language",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Optional language code, e.g. `en`, `zh-CN`, or `fr`.",
                    required: false,
                },
                FieldSchema {
                    name: "max_results",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Max results (1-50, default from searxng.max_results).",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "results",
                ty: TypeSchema::Array(Box::new(TypeSchema::Json)),
                comment: "Each item: {title, url, snippet, source}.",
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
        // Route through the mode-aware factory so direct-mode users
        // hit their personal Composio tenant when the Tauri shell
        // calls `tools.composio_execute` (e.g. onboarding-driven
        // flows). Pre-fix, the controller hard-bound to the
        // backend-only `build_composio_client` and silently 4xx'd for
        // direct-mode users (#1710). Mirrors
        // `composio::ops::composio_execute`.
        use crate::openhuman::integrations::composio::client::{
            create_composio_client, direct_execute, ComposioClientKind,
        };
        let kind =
            create_composio_client(&config).map_err(|e| format!("tools.composio_execute: {e}"))?;
        tracing::debug!(
            action = %action,
            mode = %config.composio.mode,
            "[tools][composio_execute] executing action"
        );
        let resp = match kind {
            ComposioClientKind::Backend(client) => {
                tracing::debug!(action = %action, "[tools][composio_execute] branch=backend");
                client
                    .execute_tool(&action, action_args)
                    .await
                    .map_err(|e| format!("composio execute_tool (backend) failed: {e:#}"))?
            }
            ComposioClientKind::Direct(direct) => {
                tracing::debug!(action = %action, "[tools][composio_execute] branch=direct");
                direct_execute(
                    &direct,
                    &action,
                    action_args,
                    &config.composio.entity_id,
                    None,
                )
                .await
                .map_err(|e| format!("composio execute_tool (direct) failed: {e:#}"))?
            }
        };
        tracing::debug!(
            action = %action,
            successful = resp.successful,
            "[tools][composio_execute] complete"
        );

        let payload = json!({
            "successful": resp.successful,
            "data": resp.data,
            "error": resp.error,
            "cost_usd": resp.cost_usd,
            "markdown_formatted": resp.markdown_formatted,
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
            .post::<crate::openhuman::search::tools::SearchResponse>(
                "/agent-integrations/parallel/search",
                &body,
            )
            .await
            .map_err(|e| format!("parallel search failed: {e:#}"))?;

        let count = resp.results.len();
        // Attribute the search to the provider the managed backend resolved to,
        // so this RPC surface labels a call the same way the agent-facing
        // `web_search` tool does (#5136).
        let provider = crate::openhuman::search::tools::resolve_managed_provider(&resp);
        let payload = json!({ "results": resp.results, "provider": provider });
        // Log the query's length, never its text: a search query is
        // user-authored and can carry PII or credentials. This matches the
        // sibling seltz/querit handlers below, which already log `query_len`.
        let log = vec![format!(
            "tools.web_search: query_len={} results={count} provider={provider}",
            query.chars().count()
        )];
        RpcOutcome::new(payload, log).into_cli_compatible_json()
    })
}

fn handle_seltz_search(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let query = params
            .get("query")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .ok_or_else(|| "missing or empty `query`".to_string())?;
        let max_results = params
            .get("max_results")
            .and_then(Value::as_u64)
            .map(|n| n.clamp(1, 20) as usize)
            .unwrap_or(10);

        let config = config_rpc::load_config_with_timeout().await?;

        if !config.seltz.enabled {
            tracing::debug!("[rpc][tools.seltz_search] seltz disabled — rejecting");
            return Err("Seltz search is not enabled. Set SELTZ_API_KEY to enable.".to_string());
        }

        let has_include_domains = params.get("include_domains").is_some();
        let has_exclude_domains = params.get("exclude_domains").is_some();
        let has_scope = params.get("scope").is_some();

        tracing::debug!(
            query_len = query.chars().count(),
            max_results,
            has_include_domains,
            has_exclude_domains,
            has_scope,
            "[rpc][tools.seltz_search] start"
        );

        let tool = crate::openhuman::search::tools::SeltzSearchTool::new(
            config.seltz.api_key.clone(),
            config.seltz.api_url.clone(),
            max_results,
            config.seltz.timeout_secs,
        );

        // Build args JSON with all optional fields.
        let mut args = json!({ "query": query, "max_results": max_results });
        let args_map = args.as_object_mut().unwrap();
        if let Some(v) = params.get("include_domains") {
            args_map.insert("include_domains".to_string(), v.clone());
        }
        if let Some(v) = params.get("exclude_domains") {
            args_map.insert("exclude_domains".to_string(), v.clone());
        }
        if let Some(v) = params.get("from_date") {
            args_map.insert("from_date".to_string(), v.clone());
        }
        if let Some(v) = params.get("to_date") {
            args_map.insert("to_date".to_string(), v.clone());
        }
        if let Some(v) = params.get("scope") {
            args_map.insert("scope".to_string(), v.clone());
        }

        let result = tool
            .execute(args)
            .await
            .map_err(|e| format!("seltz search failed: {e:#}"))?;

        let payload = json!({ "documents": result.output() });
        let log = vec![format!(
            "[rpc][tools.seltz_search] success query_len={} max_results={}",
            query.chars().count(),
            max_results
        )];
        RpcOutcome::new(payload, log).into_cli_compatible_json()
    })
}

fn handle_querit_search(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let query = params
            .get("query")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .ok_or_else(|| "missing or empty `query`".to_string())?;
        let max_results = params
            .get("max_results")
            .or_else(|| params.get("count"))
            .and_then(Value::as_u64)
            .map(|n| n.clamp(1, 20) as usize)
            .unwrap_or(10);

        let config = config_rpc::load_config_with_timeout().await?;
        if !config.search.querit.has_key() {
            tracing::debug!("[rpc][tools.querit_search] querit not configured — rejecting");
            return Err("Querit search is not enabled. Set QUERIT_API_KEY to enable.".to_string());
        }

        let has_include_domains = params.get("include_domains").is_some();
        let has_exclude_domains = params.get("exclude_domains").is_some();
        let has_time_range = params.get("time_range").is_some();
        let has_countries = params.get("countries").is_some();
        let has_languages = params.get("languages").is_some();
        let has_native_filters = params.get("filters").is_some();

        tracing::debug!(
            query_len = query.chars().count(),
            max_results,
            has_include_domains,
            has_exclude_domains,
            has_time_range,
            has_countries,
            has_languages,
            has_native_filters,
            "[rpc][tools.querit_search] start"
        );

        let tool = crate::openhuman::search::tools::QueritSearchTool::new(
            config.search.querit.api_key.clone(),
            None,
            max_results,
            config.search.timeout_secs,
        );

        let mut args = json!({ "query": query, "max_results": max_results });
        let args_map = args.as_object_mut().unwrap();
        for key in [
            "count",
            "filters",
            "include_domains",
            "exclude_domains",
            "time_range",
            "from_date",
            "to_date",
            "countries",
            "languages",
        ] {
            if let Some(v) = params.get(key) {
                args_map.insert(key.to_string(), v.clone());
            }
        }

        let result = tool
            .execute(args)
            .await
            .map_err(|e| format!("querit search failed: {e:#}"))?;

        let payload = json!({ "results": result.output() });
        let log = vec![format!(
            "[rpc][tools.querit_search] success query_len={} max_results={}",
            query.chars().count(),
            max_results
        )];
        RpcOutcome::new(payload, log).into_cli_compatible_json()
    })
}
