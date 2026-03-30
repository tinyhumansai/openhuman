use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

const DEFAULT_CORE_RPC_URL: &str = "http://127.0.0.1:7788/rpc";

#[derive(Debug, Serialize)]
struct RpcRequest<'a> {
    jsonrpc: &'static str,
    id: u64,
    method: &'a str,
    params: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct RpcResponse<T> {
    #[allow(dead_code)]
    jsonrpc: String,
    #[allow(dead_code)]
    id: serde_json::Value,
    result: Option<T>,
    error: Option<RpcError>,
}

#[derive(Debug, Deserialize)]
struct RpcError {
    #[allow(dead_code)]
    code: i64,
    message: String,
    data: Option<serde_json::Value>,
}

fn rpc_url() -> String {
    std::env::var("OPENHUMAN_CORE_RPC_URL").unwrap_or_else(|_| DEFAULT_CORE_RPC_URL.to_string())
}

pub fn resolved_rpc_url() -> String {
    rpc_url()
}

pub async fn call<T: DeserializeOwned>(
    method: &str,
    params: serde_json::Value,
) -> Result<T, String> {
    let client = reqwest::Client::new();
    let req = RpcRequest {
        jsonrpc: "2.0",
        id: 1,
        method,
        params,
    };

    let resp = client
        .post(rpc_url())
        .json(&req)
        .send()
        .await
        .map_err(|e| format!("core rpc request failed: {e}"))?;

    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("core rpc http error {status}: {text}"));
    }

    let payload: RpcResponse<T> = resp
        .json()
        .await
        .map_err(|e| format!("core rpc decode failed: {e}"))?;

    if let Some(err) = payload.error {
        let data = err.data.map(|d| format!(" ({d})")).unwrap_or_default();
        return Err(format!("{}{}", err.message, data));
    }

    payload
        .result
        .ok_or_else(|| "core rpc missing result".to_string())
}

pub async fn ping() -> bool {
    #[derive(Deserialize)]
    struct Pong {
        ok: bool,
    }

    match call::<Pong>("core.ping", serde_json::json!({})).await {
        Ok(p) => p.ok,
        Err(_) => false,
    }
}
