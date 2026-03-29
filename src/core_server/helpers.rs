use serde::de::DeserializeOwned;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::OnceLock;

use crate::openhuman::config::Config;
use crate::openhuman::rpc::RpcOutcome;

#[cfg(feature = "tauri-host")]
#[allow(dead_code)]
static DESKTOP_APP_HANDLE: OnceLock<tauri::AppHandle> = OnceLock::new();

#[allow(dead_code)]
static DESKTOP_RESOURCE_DIR: OnceLock<PathBuf> = OnceLock::new();

#[cfg(feature = "tauri-host")]
#[allow(dead_code)]
pub fn init_desktop_app_handle(handle: tauri::AppHandle) {
    let _ = DESKTOP_APP_HANDLE.set(handle);
}

#[cfg(feature = "tauri-host")]
#[allow(dead_code)]
pub fn desktop_app_handle() -> Result<tauri::AppHandle, String> {
    DESKTOP_APP_HANDLE
        .get()
        .cloned()
        .ok_or_else(|| "desktop app handle not set".to_string())
}

#[allow(dead_code)]
pub fn init_desktop_resource_dir(dir: PathBuf) {
    let _ = DESKTOP_RESOURCE_DIR.set(dir);
}

#[allow(dead_code)]
pub fn desktop_resource_dir() -> Option<PathBuf> {
    DESKTOP_RESOURCE_DIR.get().cloned()
}

pub async fn load_openhuman_config() -> Result<Config, String> {
    crate::openhuman::config::rpc::load_config_with_timeout().await
}

pub fn parse_params<T: DeserializeOwned>(params: serde_json::Value) -> Result<T, String> {
    serde_json::from_value(params).map_err(|e| format!("invalid params: {e}"))
}

pub fn extract_namespaces_from_documents(payload: &serde_json::Value) -> Vec<String> {
    fn collect_from_value(value: &serde_json::Value, out: &mut std::collections::BTreeSet<String>) {
        match value {
            serde_json::Value::Object(map) => {
                if let Some(ns) = map.get("namespace").and_then(serde_json::Value::as_str) {
                    if !ns.trim().is_empty() {
                        out.insert(ns.to_string());
                    }
                }
                for nested in map.values() {
                    collect_from_value(nested, out);
                }
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    collect_from_value(item, out);
                }
            }
            _ => {}
        }
    }

    let mut namespaces = std::collections::BTreeSet::new();
    collect_from_value(payload, &mut namespaces);
    namespaces.into_iter().collect()
}

pub fn filter_documents_payload_by_namespace(
    payload: serde_json::Value,
    namespace: &str,
) -> serde_json::Value {
    fn filter_array(items: &mut Vec<serde_json::Value>, namespace: &str) {
        items.retain(|item| {
            item.as_object()
                .and_then(|obj| obj.get("namespace"))
                .and_then(serde_json::Value::as_str)
                .map(|ns| ns == namespace)
                .unwrap_or(false)
        });
    }

    match payload {
        serde_json::Value::Array(mut items) => {
            filter_array(&mut items, namespace);
            serde_json::Value::Array(items)
        }
        serde_json::Value::Object(mut root) => {
            for key in ["documents", "items", "results"] {
                if let Some(serde_json::Value::Array(items)) = root.get_mut(key) {
                    filter_array(items, namespace);
                    return serde_json::Value::Object(root);
                }
            }

            if let Some(serde_json::Value::Object(data)) = root.get_mut("data") {
                for key in ["documents", "items", "results"] {
                    if let Some(serde_json::Value::Array(items)) = data.get_mut(key) {
                        filter_array(items, namespace);
                        return serde_json::Value::Object(root);
                    }
                }
            }

            serde_json::Value::Object(root)
        }
        other => other,
    }
}

/// Maps a domain [`RpcOutcome`](crate::openhuman::rpc::RpcOutcome) into a JSON-RPC [`InvocationResult`].
pub fn rpc_invocation_from_outcome<T: Serialize>(
    o: RpcOutcome<T>,
) -> Result<super::types::InvocationResult, String> {
    super::types::InvocationResult::with_logs(o.value, o.logs)
}

/// Wraps a domain [`RpcOutcome`] the same way as JSON-RPC / [`super::types::invocation_to_rpc_json`] for CLI output.
pub fn rpc_outcome_to_cli_json<T: Serialize>(
    outcome: RpcOutcome<T>,
) -> Result<serde_json::Value, String> {
    Ok(super::types::invocation_to_rpc_json(
        rpc_invocation_from_outcome(outcome)?,
    ))
}

pub async fn rpc_outcome_fut_to_cli_json<T: Serialize>(
    fut: impl std::future::Future<Output = Result<RpcOutcome<T>, String>>,
) -> Result<serde_json::Value, String> {
    rpc_outcome_to_cli_json(fut.await?)
}
