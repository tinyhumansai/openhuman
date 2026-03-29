use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::openhuman::screen_intelligence::{
    InputActionParams, PermissionRequestParams, StartSessionParams, StopSessionParams,
};
use crate::rpc::RpcOutcome;

fn parse_params<T: DeserializeOwned>(params: serde_json::Value) -> Result<T, String> {
    serde_json::from_value(params).map_err(|e| format!("invalid params: {e}"))
}

fn rpc_json<T: Serialize>(outcome: RpcOutcome<T>) -> Result<serde_json::Value, String> {
    outcome.into_cli_compatible_json()
}

#[derive(Debug, Deserialize)]
struct AccessibilityVisionRecentParams {
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct MemoryDocListParams {
    namespace: Option<String>,
}

pub async fn try_dispatch(
    method: &str,
    params: serde_json::Value,
) -> Option<Result<serde_json::Value, String>> {
    match method {
        "memory.namespace.list" => Some(
            async move { rpc_json(crate::openhuman::memory::rpc::namespace_list().await?) }.await,
        ),

        "memory.doc.put" => Some(
            async move {
                let payload: crate::openhuman::memory::rpc::PutDocParams = parse_params(params)?;
                rpc_json(crate::openhuman::memory::rpc::doc_put(payload).await?)
            }
            .await,
        ),

        "memory.doc.list" => Some(
            async move {
                let payload: MemoryDocListParams = parse_params(params)?;
                let namespace_params = payload.namespace.map(|namespace| {
                    crate::openhuman::memory::rpc::NamespaceOnlyParams { namespace }
                });
                rpc_json(crate::openhuman::memory::rpc::doc_list(namespace_params).await?)
            }
            .await,
        ),

        "memory.doc.delete" => Some(
            async move {
                let payload: crate::openhuman::memory::rpc::DeleteDocParams = parse_params(params)?;
                rpc_json(crate::openhuman::memory::rpc::doc_delete(payload).await?)
            }
            .await,
        ),

        "memory.context.query" => Some(
            async move {
                let payload: crate::openhuman::memory::rpc::QueryNamespaceParams =
                    parse_params(params)?;
                rpc_json(crate::openhuman::memory::rpc::context_query(payload).await?)
            }
            .await,
        ),

        "memory.context.recall" => Some(
            async move {
                let payload: crate::openhuman::memory::rpc::RecallNamespaceParams =
                    parse_params(params)?;
                rpc_json(crate::openhuman::memory::rpc::context_recall(payload).await?)
            }
            .await,
        ),

        "memory.kv.set" => Some(
            async move {
                let payload: crate::openhuman::memory::rpc::KvSetParams = parse_params(params)?;
                rpc_json(crate::openhuman::memory::rpc::kv_set(payload).await?)
            }
            .await,
        ),

        "memory.kv.get" => Some(
            async move {
                let payload: crate::openhuman::memory::rpc::KvGetDeleteParams =
                    parse_params(params)?;
                rpc_json(crate::openhuman::memory::rpc::kv_get(payload).await?)
            }
            .await,
        ),

        "memory.kv.delete" => Some(
            async move {
                let payload: crate::openhuman::memory::rpc::KvGetDeleteParams =
                    parse_params(params)?;
                rpc_json(crate::openhuman::memory::rpc::kv_delete(payload).await?)
            }
            .await,
        ),

        "memory.kv.list_namespace" => Some(
            async move {
                let payload: crate::openhuman::memory::rpc::NamespaceOnlyParams =
                    parse_params(params)?;
                rpc_json(crate::openhuman::memory::rpc::kv_list_namespace(payload).await?)
            }
            .await,
        ),

        "memory.graph.upsert" => Some(
            async move {
                let payload: crate::openhuman::memory::rpc::GraphUpsertParams =
                    parse_params(params)?;
                rpc_json(crate::openhuman::memory::rpc::graph_upsert(payload).await?)
            }
            .await,
        ),

        "memory.graph.query" => Some(
            async move {
                let payload: crate::openhuman::memory::rpc::GraphQueryParams =
                    parse_params(params)?;
                rpc_json(crate::openhuman::memory::rpc::graph_query(payload).await?)
            }
            .await,
        ),

        "openhuman.security_policy_info" => Some(rpc_json(
            crate::openhuman::security::rpc::security_policy_info(),
        )),

        "openhuman.accessibility_status" => Some(
            async move {
                rpc_json(crate::openhuman::screen_intelligence::rpc::accessibility_status().await?)
            }
            .await,
        ),

        "openhuman.accessibility_request_permissions" => Some(
            async move {
                rpc_json(
                    crate::openhuman::screen_intelligence::rpc::accessibility_request_permissions()
                        .await?,
                )
            }
            .await,
        ),

        "openhuman.accessibility_request_permission" => Some(
            async move {
                let payload: PermissionRequestParams = parse_params(params)?;
                rpc_json(
                    crate::openhuman::screen_intelligence::rpc::accessibility_request_permission(
                        payload,
                    )
                    .await?,
                )
            }
            .await,
        ),

        "openhuman.accessibility_start_session" => Some(
            async move {
                let payload: StartSessionParams = parse_params(params)?;
                rpc_json(
                    crate::openhuman::screen_intelligence::rpc::accessibility_start_session(
                        payload,
                    )
                    .await?,
                )
            }
            .await,
        ),

        "openhuman.accessibility_stop_session" => Some(
            async move {
                let payload: StopSessionParams = parse_params(params)?;
                rpc_json(
                    crate::openhuman::screen_intelligence::rpc::accessibility_stop_session(payload)
                        .await?,
                )
            }
            .await,
        ),

        "openhuman.accessibility_capture_now" => Some(
            async move {
                rpc_json(
                    crate::openhuman::screen_intelligence::rpc::accessibility_capture_now().await?,
                )
            }
            .await,
        ),

        "openhuman.accessibility_capture_image_ref" => Some(
            async move {
                rpc_json(
                    crate::openhuman::screen_intelligence::rpc::accessibility_capture_image_ref()
                        .await?,
                )
            }
            .await,
        ),

        "openhuman.accessibility_input_action" => Some(
            async move {
                let payload: InputActionParams = parse_params(params)?;
                rpc_json(
                    crate::openhuman::screen_intelligence::rpc::accessibility_input_action(payload)
                        .await?,
                )
            }
            .await,
        ),

        "openhuman.accessibility_vision_recent" => Some(
            async move {
                let payload: AccessibilityVisionRecentParams = parse_params(params)?;
                rpc_json(
                    crate::openhuman::screen_intelligence::rpc::accessibility_vision_recent(
                        payload.limit,
                    )
                    .await?,
                )
            }
            .await,
        ),

        "openhuman.accessibility_vision_flush" => Some(
            async move {
                rpc_json(
                    crate::openhuman::screen_intelligence::rpc::accessibility_vision_flush()
                        .await?,
                )
            }
            .await,
        ),

        "openhuman.socket.connect" => Some(Err(
            "native skill runtime and socket manager are not available in this build".to_string(),
        )),

        "openhuman.socket.disconnect" => Some(Err(
            "native skill runtime and socket manager are not available in this build".to_string(),
        )),

        "openhuman.socket.state" => Some(Err(
            "native skill runtime and socket manager are not available in this build".to_string(),
        )),

        "openhuman.socket.emit" => Some(Err(
            "native skill runtime and socket manager are not available in this build".to_string(),
        )),

        _ => None,
    }
}
