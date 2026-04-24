//! Shared helpers for authenticated calls from the Tauri host to the local core RPC.

use reqwest::RequestBuilder;

const DEFAULT_CORE_RPC_URL: &str = "http://127.0.0.1:7788/rpc";
const CORE_RPC_URL_ENV: &str = "OPENHUMAN_CORE_RPC_URL";
pub(crate) fn core_rpc_url_value() -> String {
    std::env::var(CORE_RPC_URL_ENV).unwrap_or_else(|_| DEFAULT_CORE_RPC_URL.to_string())
}

pub(crate) fn apply_auth(builder: RequestBuilder) -> Result<RequestBuilder, String> {
    let token = crate::core_process::current_rpc_token()
        .ok_or_else(|| "core RPC token is not initialized".to_string())?;
    Ok(builder.header("Authorization", format!("Bearer {token}")))
}
