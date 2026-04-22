//! RPC handler functions for the Gmail domain.
//!
//! Each handler loads the config, delegates to `ops.rs`, and converts the
//! `RpcOutcome` to a JSON `Value` as expected by the controller registry.

use serde_json::{Map, Value};

use crate::core::all::ControllerFuture;
use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::gmail::ops;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn read_required_string(params: &Map<String, Value>, key: &str) -> Result<String, String> {
    params
        .get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| format!("missing required param '{key}'"))
}

fn to_value<T: serde::Serialize>(outcome: crate::rpc::RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

// ---------------------------------------------------------------------------
// Handlers (called by schemas.rs controller registry entries)
// ---------------------------------------------------------------------------

pub fn handle_list_accounts(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_value(ops::list_accounts(&config).await?)
    })
}

pub fn handle_connect_account(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let account_id = read_required_string(&params, "account_id")?;
        let email = read_required_string(&params, "email")?;
        to_value(ops::connect_account(&config, &account_id, &email).await?)
    })
}

pub fn handle_disconnect_account(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let account_id = read_required_string(&params, "account_id")?;
        to_value(ops::disconnect_account(&config, &account_id).await?)
    })
}

pub fn handle_sync_now(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let account_id = read_required_string(&params, "account_id")?;
        to_value(ops::sync_now(&config, &account_id).await?)
    })
}

pub fn handle_get_stats(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let account_id = read_required_string(&params, "account_id")?;
        to_value(ops::get_stats(&config, &account_id).await?)
    })
}

pub fn handle_ingest_raw_response(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let account_id = read_required_string(&params, "account_id")?;
        let url = read_required_string(&params, "url")?;
        let body = read_required_string(&params, "body")?;
        to_value(ops::ingest_raw_response(&config, &account_id, &url, &body).await?)
    })
}
