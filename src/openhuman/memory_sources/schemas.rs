//! Controller-registry schemas for `openhuman.memory_sources_*`.

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::rpc::RpcOutcome;

use super::rpc;

const NAMESPACE: &str = "memory_sources";

fn kind_specific_fields() -> Vec<FieldSchema> {
    vec![
        FieldSchema {
            name: "toolkit",
            ty: TypeSchema::Option(Box::new(TypeSchema::String)),
            comment: "Composio toolkit slug.",
            required: false,
        },
        FieldSchema {
            name: "connection_id",
            ty: TypeSchema::Option(Box::new(TypeSchema::String)),
            comment: "Composio connection id.",
            required: false,
        },
        FieldSchema {
            name: "path",
            ty: TypeSchema::Option(Box::new(TypeSchema::String)),
            comment: "Local folder path.",
            required: false,
        },
        FieldSchema {
            name: "glob",
            ty: TypeSchema::Option(Box::new(TypeSchema::String)),
            comment: "Glob pattern for folder sources.",
            required: false,
        },
        FieldSchema {
            name: "url",
            ty: TypeSchema::Option(Box::new(TypeSchema::String)),
            comment: "URL for github_repo, rss_feed, or web_page sources.",
            required: false,
        },
        FieldSchema {
            name: "branch",
            ty: TypeSchema::Option(Box::new(TypeSchema::String)),
            comment: "Git branch for github_repo sources.",
            required: false,
        },
        FieldSchema {
            name: "paths",
            ty: TypeSchema::Array(Box::new(TypeSchema::String)),
            comment: "Path filters for github_repo sources.",
            required: false,
        },
        FieldSchema {
            name: "max_commits",
            ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
            comment: "Max commits per sync for github_repo sources.",
            required: false,
        },
        FieldSchema {
            name: "max_issues",
            ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
            comment: "Max issues per sync for github_repo sources.",
            required: false,
        },
        FieldSchema {
            name: "max_prs",
            ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
            comment: "Max pull requests per sync for github_repo sources.",
            required: false,
        },
        FieldSchema {
            name: "query",
            ty: TypeSchema::Option(Box::new(TypeSchema::String)),
            comment: "Search query for twitter_query sources.",
            required: false,
        },
        FieldSchema {
            name: "since_days",
            ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
            comment: "Lookback window in days for twitter_query.",
            required: false,
        },
        FieldSchema {
            name: "max_items",
            ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
            comment: "Maximum items for rss_feed or composio sources.",
            required: false,
        },
        FieldSchema {
            name: "selector",
            ty: TypeSchema::Option(Box::new(TypeSchema::String)),
            comment: "CSS selector for web_page sources.",
            required: false,
        },
        FieldSchema {
            name: "max_tokens_per_sync",
            ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
            comment: "Max tokens per sync run.",
            required: false,
        },
        FieldSchema {
            name: "max_cost_per_sync_usd",
            ty: TypeSchema::Option(Box::new(TypeSchema::F64)),
            comment: "Max cost per sync run in USD.",
            required: false,
        },
        FieldSchema {
            name: "sync_depth_days",
            ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
            comment: "Only sync items from the last N days.",
            required: false,
        },
    ]
}

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("list"),
        schemas("get"),
        schemas("add"),
        schemas("update"),
        schemas("remove"),
        schemas("list_items"),
        schemas("read_item"),
        schemas("sync"),
        schemas("reconcile"),
        schemas("status_list"),
        schemas("supported_toolkits"),
        schemas("sync_audit_log"),
        schemas("estimate_sync_cost"),
        schemas("monthly_cost_summary"),
        schemas("apply_all_in"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("list"),
            handler: handle_list,
        },
        RegisteredController {
            schema: schemas("get"),
            handler: handle_get,
        },
        RegisteredController {
            schema: schemas("add"),
            handler: handle_add,
        },
        RegisteredController {
            schema: schemas("update"),
            handler: handle_update,
        },
        RegisteredController {
            schema: schemas("remove"),
            handler: handle_remove,
        },
        RegisteredController {
            schema: schemas("list_items"),
            handler: handle_list_items,
        },
        RegisteredController {
            schema: schemas("read_item"),
            handler: handle_read_item,
        },
        RegisteredController {
            schema: schemas("sync"),
            handler: handle_sync,
        },
        RegisteredController {
            schema: schemas("reconcile"),
            handler: handle_reconcile,
        },
        RegisteredController {
            schema: schemas("status_list"),
            handler: handle_status_list,
        },
        RegisteredController {
            schema: schemas("supported_toolkits"),
            handler: handle_supported_toolkits,
        },
        RegisteredController {
            schema: schemas("sync_audit_log"),
            handler: handle_sync_audit_log,
        },
        RegisteredController {
            schema: schemas("estimate_sync_cost"),
            handler: handle_estimate_sync_cost,
        },
        RegisteredController {
            schema: schemas("monthly_cost_summary"),
            handler: handle_monthly_cost_summary,
        },
        RegisteredController {
            schema: schemas("apply_all_in"),
            handler: handle_apply_all_in,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "list" => ControllerSchema {
            namespace: NAMESPACE,
            function: "list",
            description: "List all configured memory sources.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "sources",
                ty: TypeSchema::Array(Box::new(TypeSchema::Ref("MemorySourceEntry"))),
                comment: "All configured sources.",
                required: true,
            }],
        },
        "get" => ControllerSchema {
            namespace: NAMESPACE,
            function: "get",
            description: "Get a single memory source by id.",
            inputs: vec![FieldSchema {
                name: "id",
                ty: TypeSchema::String,
                comment: "Source id.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "source",
                ty: TypeSchema::Option(Box::new(TypeSchema::Ref("MemorySourceEntry"))),
                comment: "The source if found.",
                required: false,
            }],
        },
        "add" => {
            let mut inputs = vec![
                FieldSchema {
                    name: "kind",
                    ty: TypeSchema::Enum {
                        variants: vec![
                            "composio",
                            "conversation",
                            "folder",
                            "github_repo",
                            "twitter_query",
                            "rss_feed",
                            "web_page",
                        ],
                    },
                    comment: "Source kind.",
                    required: true,
                },
                FieldSchema {
                    name: "label",
                    ty: TypeSchema::String,
                    comment: "User-facing display name.",
                    required: true,
                },
                FieldSchema {
                    name: "enabled",
                    ty: TypeSchema::Bool,
                    comment: "Whether the source is active. Defaults to true.",
                    required: false,
                },
            ];
            inputs.extend(kind_specific_fields());
            ControllerSchema {
                namespace: NAMESPACE,
                function: "add",
                description:
                    "Add a new memory source. Kind-specific fields are flat on the request.",
                inputs,
                outputs: vec![FieldSchema {
                    name: "source",
                    ty: TypeSchema::Ref("MemorySourceEntry"),
                    comment: "The newly created source.",
                    required: true,
                }],
            }
        }
        "update" => {
            let mut inputs = vec![
                FieldSchema {
                    name: "id",
                    ty: TypeSchema::String,
                    comment: "Source id to update.",
                    required: true,
                },
                FieldSchema {
                    name: "label",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "New label.",
                    required: false,
                },
                FieldSchema {
                    name: "enabled",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Bool)),
                    comment: "Enable or disable.",
                    required: false,
                },
            ];
            inputs.extend(kind_specific_fields());
            ControllerSchema {
                namespace: NAMESPACE,
                function: "update",
                description: "Partial update of a memory source.",
                inputs,
                outputs: vec![FieldSchema {
                    name: "source",
                    ty: TypeSchema::Ref("MemorySourceEntry"),
                    comment: "The updated source.",
                    required: true,
                }],
            }
        }
        "remove" => ControllerSchema {
            namespace: NAMESPACE,
            function: "remove",
            description: "Remove a memory source.",
            inputs: vec![FieldSchema {
                name: "id",
                ty: TypeSchema::String,
                comment: "Source id to remove.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "removed",
                ty: TypeSchema::Bool,
                comment: "True if the source was found and removed.",
                required: true,
            }],
        },
        "list_items" => ControllerSchema {
            namespace: NAMESPACE,
            function: "list_items",
            description: "List readable items from a memory source via its reader.",
            inputs: vec![FieldSchema {
                name: "source_id",
                ty: TypeSchema::String,
                comment: "Source id to list items from.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "items",
                ty: TypeSchema::Array(Box::new(TypeSchema::Ref("SourceItem"))),
                comment: "Items available in the source.",
                required: true,
            }],
        },
        "read_item" => ControllerSchema {
            namespace: NAMESPACE,
            function: "read_item",
            description: "Read one item's content from a memory source.",
            inputs: vec![
                FieldSchema {
                    name: "source_id",
                    ty: TypeSchema::String,
                    comment: "Source id.",
                    required: true,
                },
                FieldSchema {
                    name: "item_id",
                    ty: TypeSchema::String,
                    comment: "Item id within the source.",
                    required: true,
                },
            ],
            outputs: vec![FieldSchema {
                name: "content",
                ty: TypeSchema::Ref("SourceContent"),
                comment: "The item's content.",
                required: true,
            }],
        },
        "sync" => ControllerSchema {
            namespace: NAMESPACE,
            function: "sync",
            description: "Trigger a sync for a memory source. Returns immediately; \
                          progress is published as MemorySyncStageChanged events.",
            inputs: vec![FieldSchema {
                name: "source_id",
                ty: TypeSchema::String,
                comment: "Source id to sync.",
                required: true,
            }],
            outputs: vec![
                FieldSchema {
                    name: "requested",
                    ty: TypeSchema::Bool,
                    comment: "True when the sync was queued.",
                    required: true,
                },
                FieldSchema {
                    name: "source_id",
                    ty: TypeSchema::String,
                    comment: "Echo of the requested source id.",
                    required: true,
                },
            ],
        },
        "reconcile" => ControllerSchema {
            namespace: NAMESPACE,
            function: "reconcile",
            description: "Report raw-archive vs memory-tree coverage per source scope; \
                          with execute=true, start a background incremental reconcile \
                          (summarise + ingest) for every scope with pending files. The \
                          same reconcile also runs automatically after each sync.",
            inputs: vec![
                FieldSchema {
                    name: "source_id",
                    ty: TypeSchema::String,
                    comment: "Restrict to one source id; omit for all enabled sources.",
                    required: false,
                },
                FieldSchema {
                    name: "execute",
                    ty: TypeSchema::Bool,
                    comment: "Start background reconcile for scopes with pending files \
                              (default false = report only).",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "scopes",
                ty: TypeSchema::Array(Box::new(TypeSchema::Ref("ReconcileScopeReport"))),
                comment: "Per-scope coverage: total raw files, covered, pending, started.",
                required: true,
            }],
        },
        "status_list" => ControllerSchema {
            namespace: NAMESPACE,
            function: "status_list",
            description: "Per-source sync status — chunks ingested, freshness label, \
                          last-chunk timestamp.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "statuses",
                ty: TypeSchema::Array(Box::new(TypeSchema::Ref("SourceStatus"))),
                comment: "One row per configured memory source.",
                required: true,
            }],
        },
        "supported_toolkits" => ControllerSchema {
            namespace: NAMESPACE,
            function: "supported_toolkits",
            description: "Toolkit slugs that ship a native memory-sync provider. \
                          The Add Source picker disables connections outside this set.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "toolkits",
                ty: TypeSchema::Array(Box::new(TypeSchema::String)),
                comment: "Sorted, de-duplicated supported toolkit slugs.",
                required: true,
            }],
        },
        "sync_audit_log" => ControllerSchema {
            namespace: NAMESPACE,
            function: "sync_audit_log",
            description:
                "Sync audit history — timestamp, tokens consumed, cost, duration for each sync run.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "entries",
                ty: TypeSchema::Array(Box::new(TypeSchema::Ref("SyncAuditEntry"))),
                comment: "Audit entries, most recent first.",
                required: true,
            }],
        },
        "estimate_sync_cost" => ControllerSchema {
            namespace: NAMESPACE,
            function: "estimate_sync_cost",
            description:
                "Estimate the cost of syncing a source before starting. Returns item count, \
                 estimated tokens, and estimated cost in USD.",
            inputs: vec![FieldSchema {
                name: "source_id",
                ty: TypeSchema::String,
                comment: "Source id to estimate.",
                required: true,
            }],
            outputs: vec![
                FieldSchema {
                    name: "source_id",
                    ty: TypeSchema::String,
                    comment: "Echo of source id.",
                    required: true,
                },
                FieldSchema {
                    name: "item_count",
                    ty: TypeSchema::U64,
                    comment: "Number of items to sync.",
                    required: true,
                },
                FieldSchema {
                    name: "estimated_tokens",
                    ty: TypeSchema::U64,
                    comment: "Estimated input tokens.",
                    required: true,
                },
                FieldSchema {
                    name: "estimated_cost_usd",
                    ty: TypeSchema::F64,
                    comment: "Estimated cost in USD.",
                    required: true,
                },
                FieldSchema {
                    name: "budget_max_cost_usd",
                    ty: TypeSchema::Option(Box::new(TypeSchema::F64)),
                    comment: "Configured cost cap if set.",
                    required: false,
                },
                FieldSchema {
                    name: "budget_max_tokens",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Configured token cap if set.",
                    required: false,
                },
            ],
        },
        "monthly_cost_summary" => ControllerSchema {
            namespace: NAMESPACE,
            function: "monthly_cost_summary",
            description: "Aggregate sync costs for the current calendar month.",
            inputs: vec![],
            outputs: vec![
                FieldSchema {
                    name: "month",
                    ty: TypeSchema::String,
                    comment: "YYYY-MM.",
                    required: true,
                },
                FieldSchema {
                    name: "total_cost_usd",
                    ty: TypeSchema::F64,
                    comment: "Total spend in USD.",
                    required: true,
                },
                FieldSchema {
                    name: "total_syncs",
                    ty: TypeSchema::U64,
                    comment: "Number of sync runs.",
                    required: true,
                },
                FieldSchema {
                    name: "total_items",
                    ty: TypeSchema::U64,
                    comment: "Total items fetched.",
                    required: true,
                },
                FieldSchema {
                    name: "total_input_tokens",
                    ty: TypeSchema::U64,
                    comment: "Total input tokens.",
                    required: true,
                },
                FieldSchema {
                    name: "total_output_tokens",
                    ty: TypeSchema::U64,
                    comment: "Total output tokens.",
                    required: true,
                },
            ],
        },
        "apply_all_in" => ControllerSchema {
            namespace: NAMESPACE,
            function: "apply_all_in",
            description: "Enable ALL memory sources, clear all per-source caps, \
                          and trigger a background sync for every source. \
                          Returns immediately with the updated source list and \
                          the count of sync tasks queued.",
            inputs: vec![],
            outputs: vec![
                FieldSchema {
                    name: "sources",
                    ty: TypeSchema::Array(Box::new(TypeSchema::Ref("MemorySourceEntry"))),
                    comment: "All memory sources after the all-in transformation.",
                    required: true,
                },
                FieldSchema {
                    name: "sync_triggered",
                    ty: TypeSchema::U64,
                    comment: "Number of sync tasks spawned.",
                    required: true,
                },
            ],
        },
        other => panic!("unknown memory_sources schema function: {other}"),
    }
}

fn handle_list(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { to_json(rpc::list_rpc().await?) })
}

fn handle_get(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let req = parse_value::<rpc::GetRequest>(Value::Object(params))?;
        to_json(rpc::get_rpc(req).await?)
    })
}

fn handle_add(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let req = parse_value::<rpc::AddRequest>(Value::Object(params))?;
        to_json(rpc::add_rpc(req).await?)
    })
}

fn handle_update(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let req = parse_value::<rpc::UpdateRequest>(Value::Object(params))?;
        to_json(rpc::update_rpc(req).await?)
    })
}

fn handle_remove(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let req = parse_value::<rpc::RemoveRequest>(Value::Object(params))?;
        to_json(rpc::remove_rpc(req).await?)
    })
}

fn handle_list_items(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let req = parse_value::<rpc::ListItemsRequest>(Value::Object(params))?;
        to_json(rpc::list_items_rpc(req).await?)
    })
}

fn handle_read_item(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let req = parse_value::<rpc::ReadItemRequest>(Value::Object(params))?;
        to_json(rpc::read_item_rpc(req).await?)
    })
}

fn handle_sync(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let req = parse_value::<rpc::SyncRequest>(Value::Object(params))?;
        to_json(rpc::sync_rpc(req).await?)
    })
}

fn handle_reconcile(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let req = parse_value::<rpc::ReconcileRequest>(Value::Object(params))?;
        to_json(rpc::reconcile_rpc(req).await?)
    })
}

fn handle_status_list(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { to_json(rpc::status_list_rpc().await?) })
}

fn handle_supported_toolkits(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { to_json(rpc::supported_toolkits_rpc().await?) })
}

fn handle_sync_audit_log(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { to_json(rpc::sync_audit_log_rpc().await?) })
}

fn handle_estimate_sync_cost(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let req = parse_value::<rpc::EstimateSyncCostRequest>(Value::Object(params))?;
        to_json(rpc::estimate_sync_cost_rpc(req).await?)
    })
}

fn handle_monthly_cost_summary(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { to_json(rpc::monthly_cost_summary_rpc().await?) })
}

fn handle_apply_all_in(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { to_json(rpc::apply_all_in_rpc().await?) })
}

fn parse_value<T: DeserializeOwned>(v: Value) -> Result<T, String> {
    serde_json::from_value(v).map_err(|e| format!("invalid params: {e}"))
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_controller_schemas_and_registered_controllers_stay_in_sync() {
        let schemas = all_controller_schemas();
        let controllers = all_registered_controllers();
        assert_eq!(schemas.len(), controllers.len());
        assert!(schemas.iter().all(|s| s.namespace == NAMESPACE));
    }

    #[test]
    #[should_panic(expected = "unknown memory_sources schema function")]
    fn schemas_panics_on_unknown_function() {
        schemas("nope");
    }
}
