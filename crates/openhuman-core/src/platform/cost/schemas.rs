//! Controller schemas and JSON-RPC dispatchers for the cost dashboard.

use crate::core::runtime::DispatchCtx;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::Outcome;
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

use super::rpc as cost_rpc;

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct DailyHistoryParams {
    #[serde(default)]
    days: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UsageLogParams {
    #[serde(default = "default_usage_days")]
    days: u32,
    #[serde(default = "default_usage_limit")]
    limit: usize,
}

fn default_usage_days() -> u32 {
    30
}

/// `cost_report` params. Each report takes only what its schema declares.
#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReportParams {
    #[serde(default)]
    days: Option<u32>,
    #[serde(default)]
    group_by: Vec<super::report::GroupKey>,
    #[serde(default)]
    filter: super::report::ReportFilter,
}

/// `cost_cache_report` params.
#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CacheReportParams {
    #[serde(default)]
    days: Option<u32>,
    #[serde(default)]
    filter: super::report::ReportFilter,
    #[serde(default)]
    limit: Option<usize>,
}

fn default_usage_limit() -> usize {
    250
}

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schema_for("cost_get_dashboard"),
        schema_for("cost_get_daily_history"),
        schema_for("cost_get_summary"),
        schema_for("cost_get_usage_log"),
        schema_for("cost_report"),
        schema_for("cost_cache_report"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schema_for("cost_get_dashboard"),
            handler: handle_cost_get_dashboard,
        },
        RegisteredController {
            schema: schema_for("cost_get_daily_history"),
            handler: handle_cost_get_daily_history,
        },
        RegisteredController {
            schema: schema_for("cost_get_summary"),
            handler: handle_cost_get_summary,
        },
        RegisteredController {
            schema: schema_for("cost_get_usage_log"),
            handler: handle_cost_get_usage_log,
        },
        RegisteredController {
            schema: schema_for("cost_report"),
            handler: handle_cost_report,
        },
        RegisteredController {
            schema: schema_for("cost_cache_report"),
            handler: handle_cost_cache_report,
        },
    ]
}

fn schema_for(function: &str) -> ControllerSchema {
    match function {
        "cost_get_dashboard" => ControllerSchema {
            namespace: "cost",
            function: "get_dashboard",
            description:
                "Fetch the 7-day cost & token dashboard payload: per-day buckets, summary \
                 metrics, budget utilisation, and per-model breakdown.",
            inputs: vec![],
            outputs: vec![json_output(
                "dashboard",
                "Dashboard payload with `days`, `byModel`, summary fields and budget status.",
            )],
        },
        "cost_get_daily_history" => ControllerSchema {
            namespace: "cost",
            function: "get_daily_history",
            description: "Fetch a per-day cost/token history for the requested span (default 7 \
                          days, clamped to [1, 366]).",
            inputs: vec![FieldSchema {
                name: "days",
                ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                comment: "Number of trailing days to include (default 7).",
                required: false,
            }],
            outputs: vec![json_output(
                "entries",
                "Ordered list of daily entries, oldest first; gaps zero-filled.",
            )],
        },
        "cost_get_summary" => ControllerSchema {
            namespace: "cost",
            function: "get_summary",
            description: "Fetch the live session / daily / monthly cost summary.",
            inputs: vec![],
            outputs: vec![json_output(
                "summary",
                "Aggregated cost & token usage for the current session and active period.",
            )],
        },
        "cost_get_usage_log" => ControllerSchema {
            namespace: "cost",
            function: "get_usage_log",
            description: "Fetch a bounded recent cost usage log with per-record rows and spend \
                          distribution by inferred category.",
            inputs: vec![
                FieldSchema {
                    name: "days",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment:
                        "Number of trailing days to include (default 30, clamped to [1, 366]).",
                    required: false,
                },
                FieldSchema {
                    name: "limit",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment:
                        "Maximum number of records to return (default 250, clamped to [1, 1000]).",
                    required: false,
                },
            ],
            outputs: vec![json_output(
                "usage_log",
                "Usage records plus category totals, newest records first.",
            )],
        },
        "cost_report" => ControllerSchema {
            namespace: "cost",
            function: "report",
            description:
                "Usage report over the last `days`: calls, input/output/cached/cache-write/\
                          reasoning tokens, cost split into provider-charged and estimated, and \
                          the prompt-cache hit ratio — in total and grouped by any of day, week, \
                          month, model, provider, route, agent, thread, origin, session_agent.",
            inputs: vec![days_input(), group_by_input(), filter_input()],
            outputs: vec![json_output(
                "report",
                "`totals` and `rows` (one per group, most expensive first), each with a `key` map.",
            )],
        },
        "cost_cache_report" => ControllerSchema {
            namespace: "cost",
            function: "cache_report",
            description: "Prompt-cache behaviour over the last `days`, call by call: hit ratio, \
                          cold calls (a repeat call in a thread that read nothing from the cache) \
                          and what uncached input cost above the cached rate.",
            inputs: vec![
                days_input(),
                filter_input(),
                FieldSchema {
                    name: "limit",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Newest calls to list (default 200, clamped to [1, 1000]); totals \
                              cover every call.",
                    required: false,
                },
            ],
            outputs: vec![json_output(
                "cache_report",
                "`calls`, `cache_hit_ratio`, `cold_calls` and `uncached_premium_usd`.",
            )],
        },
        _ => ControllerSchema {
            namespace: "cost",
            function: "unknown",
            description: "Unknown cost controller.",
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

/// Short opaque correlation id for log threading across an async handler
/// invocation. Eight hex chars are enough to disambiguate concurrent
/// dashboard polls without bloating log lines, and the value is local
/// so it does not leak across processes.
fn new_correlation_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..8].to_string()
}

fn handle_cost_get_dashboard(ctx: DispatchCtx, _params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let cid = new_correlation_id();
        log::debug!(target: "cost_rpc", "[cost_rpc][{cid}] cost_get_dashboard.entry");
        let config = ctx.load_config().await.inspect_err(|err| {
            log::warn!(target: "cost_rpc", "[cost_rpc][{cid}] cost_get_dashboard.config_load_failed err={err}");
        })?;
        let outcome = cost_rpc::dashboard(&config).map_err(|e| {
            let s = e.to_string();
            log::warn!(target: "cost_rpc", "[cost_rpc][{cid}] cost_get_dashboard.error err={s}");
            s
        })?;
        let json = to_json(outcome);
        log::debug!(target: "cost_rpc", "[cost_rpc][{cid}] cost_get_dashboard.exit ok={}", json.is_ok());
        json
    })
}

fn handle_cost_get_daily_history(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let cid = new_correlation_id();
        log::debug!(target: "cost_rpc", "[cost_rpc][{cid}] cost_get_daily_history.entry");
        let config = ctx.load_config().await.inspect_err(|err| {
            log::warn!(target: "cost_rpc", "[cost_rpc][{cid}] cost_get_daily_history.config_load_failed err={err}");
        })?;
        let payload = if params.is_empty() {
            DailyHistoryParams::default()
        } else {
            serde_json::from_value::<DailyHistoryParams>(Value::Object(params)).map_err(|e| {
                let s = format!("invalid params: {e}");
                log::warn!(target: "cost_rpc", "[cost_rpc][{cid}] cost_get_daily_history.bad_params err={s}");
                s
            })?
        };
        let days = payload.days.unwrap_or(7);
        let outcome = cost_rpc::daily_history(&config, days).map_err(|e| {
            let s = e.to_string();
            log::warn!(target: "cost_rpc", "[cost_rpc][{cid}] cost_get_daily_history.error err={s}");
            s
        })?;
        let json = to_json(outcome);
        log::debug!(
            target: "cost_rpc",
            "[cost_rpc][{cid}] cost_get_daily_history.exit days={days} ok={}",
            json.is_ok()
        );
        json
    })
}

fn handle_cost_get_summary(ctx: DispatchCtx, _params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let cid = new_correlation_id();
        log::debug!(target: "cost_rpc", "[cost_rpc][{cid}] cost_get_summary.entry");
        let config = ctx.load_config().await.inspect_err(|err| {
            log::warn!(target: "cost_rpc", "[cost_rpc][{cid}] cost_get_summary.config_load_failed err={err}");
        })?;
        let outcome = cost_rpc::summary(&config).map_err(|e| {
            let s = e.to_string();
            log::warn!(target: "cost_rpc", "[cost_rpc][{cid}] cost_get_summary.error err={s}");
            s
        })?;
        let json = to_json(outcome);
        log::debug!(target: "cost_rpc", "[cost_rpc][{cid}] cost_get_summary.exit ok={}", json.is_ok());
        json
    })
}

fn handle_cost_get_usage_log(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let cid = new_correlation_id();
        log::debug!(target: "cost_rpc", "[cost_rpc][{cid}] cost_get_usage_log.entry");
        let config = ctx.load_config().await.inspect_err(|err| {
            log::warn!(target: "cost_rpc", "[cost_rpc][{cid}] cost_get_usage_log.config_load_failed err={err}");
        })?;
        let payload = if params.is_empty() {
            UsageLogParams {
                days: default_usage_days(),
                limit: default_usage_limit(),
            }
        } else {
            serde_json::from_value::<UsageLogParams>(Value::Object(params)).map_err(|e| {
                let s = format!("invalid params: {e}");
                log::warn!(target: "cost_rpc", "[cost_rpc][{cid}] cost_get_usage_log.bad_params err={s}");
                s
            })?
        };
        let outcome = cost_rpc::usage_log(&config, payload.days, payload.limit).map_err(|e| {
            let s = e.to_string();
            log::warn!(target: "cost_rpc", "[cost_rpc][{cid}] cost_get_usage_log.error err={s}");
            s
        })?;
        let json = to_json(outcome);
        log::debug!(
            target: "cost_rpc",
            "[cost_rpc][{cid}] cost_get_usage_log.exit days={} limit={} ok={}",
            payload.days,
            payload.limit,
            json.is_ok()
        );
        json
    })
}

fn parse_report_params<T: serde::de::DeserializeOwned + Default>(
    params: Map<String, Value>,
) -> Result<T, String> {
    if params.is_empty() {
        return Ok(T::default());
    }
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

fn handle_cost_report(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let cid = new_correlation_id();
        log::debug!(target: "cost_rpc", "[cost_rpc][{cid}] cost_report.entry");
        let config = ctx.load_config().await?;
        let p: ReportParams = parse_report_params(params)?;
        let days = p.days.unwrap_or_else(default_usage_days);
        let outcome = cost_rpc::usage_report(&config, days, &p.group_by, &p.filter)
            .map_err(|e| e.to_string())?;
        to_json(outcome)
    })
}

fn handle_cost_cache_report(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let cid = new_correlation_id();
        log::debug!(target: "cost_rpc", "[cost_rpc][{cid}] cost_cache_report.entry");
        let config = ctx.load_config().await?;
        let p: CacheReportParams = parse_report_params(params)?;
        let days = p.days.unwrap_or_else(default_usage_days);
        let limit = p.limit.unwrap_or(200);
        let outcome =
            cost_rpc::cache_report(&config, days, &p.filter, limit).map_err(|e| e.to_string())?;
        to_json(outcome)
    })
}

fn days_input() -> FieldSchema {
    FieldSchema {
        name: "days",
        ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
        comment: "Trailing days to cover (default 30, clamped to [1, 366]).",
        required: false,
    }
}

fn group_by_input() -> FieldSchema {
    FieldSchema {
        name: "groupBy",
        ty: TypeSchema::Option(Box::new(TypeSchema::Array(Box::new(TypeSchema::Enum {
            variants: vec![
                "day",
                "week",
                "month",
                "model",
                "provider",
                "route",
                "agent",
                "thread",
                "origin",
                "session_agent",
            ],
        })))),
        comment: "Dimensions to group by, in order; omit for totals only.",
        required: false,
    }
}

fn filter_input() -> FieldSchema {
    FieldSchema {
        name: "filter",
        ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
        comment: "Optional `{thread_id, agent_id, model, provider, route, origin, \
                  session_agent}`; every set field must match.",
        required: false,
    }
}

fn to_json(outcome: Outcome<Value>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

fn json_output(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Json,
        comment,
        required: true,
    }
}

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
