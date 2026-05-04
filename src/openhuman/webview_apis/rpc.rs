//! Handler bodies for the webview_apis controllers.
//!
//! `schemas.rs` stays registry-only per project convention
//! (`src/openhuman/*/schemas.rs`: describe the schema and delegate to
//! `rpc.rs`). Each `handle_*` here validates params, issues the bridge
//! call via [`super::client::request`], and wraps the response in
//! [`RpcOutcome`].

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::all::ControllerFuture;
use crate::openhuman::webview_apis::client;
use crate::openhuman::webview_apis::types::{
    Ack, GmailLabel, GmailMessage, GmailSendRequest, SendAck,
};
use crate::rpc::RpcOutcome;

// ── handlers ────────────────────────────────────────────────────────────

pub fn handle_gmail_list_labels(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        require_string(&params, "account_id")?;
        let labels: Vec<GmailLabel> = client::request("gmail.list_labels", params).await?;
        finish(RpcOutcome::single_log(
            labels,
            "[webview_apis] gmail_list_labels ok",
        ))
    })
}

pub fn handle_gmail_list_messages(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        require_string(&params, "account_id")?;
        require_u32(&params, "limit")?;
        let messages: Vec<GmailMessage> = client::request("gmail.list_messages", params).await?;
        finish(RpcOutcome::single_log(
            messages,
            "[webview_apis] gmail_list_messages ok",
        ))
    })
}

pub fn handle_gmail_search(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        require_string(&params, "account_id")?;
        require_string(&params, "query")?;
        require_u32(&params, "limit")?;
        let messages: Vec<GmailMessage> = client::request("gmail.search", params).await?;
        finish(RpcOutcome::single_log(
            messages,
            "[webview_apis] gmail_search ok",
        ))
    })
}

pub fn handle_gmail_get_message(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        require_string(&params, "account_id")?;
        require_string(&params, "message_id")?;
        let msg: GmailMessage = client::request("gmail.get_message", params).await?;
        finish(RpcOutcome::single_log(
            msg,
            "[webview_apis] gmail_get_message ok",
        ))
    })
}

pub fn handle_gmail_send(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        require_string(&params, "account_id")?;
        let _: GmailSendRequest = read_required(&params, "request")?;
        let ack: SendAck = client::request("gmail.send", params).await?;
        finish(RpcOutcome::single_log(ack, "[webview_apis] gmail_send ok"))
    })
}

pub fn handle_gmail_trash(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        require_string(&params, "account_id")?;
        require_string(&params, "message_id")?;
        let ack: Ack = client::request("gmail.trash", params).await?;
        finish(RpcOutcome::single_log(ack, "[webview_apis] gmail_trash ok"))
    })
}

pub fn handle_gmail_add_label(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        require_string(&params, "account_id")?;
        require_string(&params, "message_id")?;
        require_string(&params, "label")?;
        let ack: Ack = client::request("gmail.add_label", params).await?;
        finish(RpcOutcome::single_log(
            ack,
            "[webview_apis] gmail_add_label ok",
        ))
    })
}

// ── helpers ─────────────────────────────────────────────────────────────

fn finish<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

fn require_string(params: &Map<String, Value>, key: &str) -> Result<(), String> {
    match params.get(key) {
        Some(Value::String(s)) if !s.trim().is_empty() => Ok(()),
        Some(Value::String(_)) => Err(format!("invalid '{key}': must be non-empty")),
        Some(_) => Err(format!("invalid '{key}': expected string")),
        None => Err(format!("missing required param '{key}'")),
    }
}

/// Tighten the numeric guard: the schema declares every `limit` input
/// as `TypeSchema::U64` and the Tauri-side router casts to `u32`, so
/// reject negatives, fractions, and values that overflow `u32` here
/// rather than letting them surface as confusing downstream errors.
fn require_u32(params: &Map<String, Value>, key: &str) -> Result<(), String> {
    match params.get(key) {
        Some(Value::Number(n)) => {
            let u = n
                .as_u64()
                .ok_or_else(|| format!("invalid '{key}': expected non-negative integer"))?;
            if u > u32::MAX as u64 {
                return Err(format!("invalid '{key}': exceeds u32 max"));
            }
            Ok(())
        }
        Some(_) => Err(format!("invalid '{key}': expected number")),
        None => Err(format!("missing required param '{key}'")),
    }
}

fn read_required<T: DeserializeOwned>(params: &Map<String, Value>, key: &str) -> Result<T, String> {
    let v = params
        .get(key)
        .cloned()
        .ok_or_else(|| format!("missing required param '{key}'"))?;
    serde_json::from_value(v).map_err(|e| format!("invalid '{key}': {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn require_string_rejects_missing_empty_and_whitespace() {
        let mut p = Map::new();
        assert!(require_string(&p, "account_id").is_err());
        p.insert("account_id".into(), Value::String(String::new()));
        assert!(require_string(&p, "account_id").is_err());
        p.insert("account_id".into(), Value::String("   ".into()));
        assert!(require_string(&p, "account_id").is_err());
        p.insert("account_id".into(), Value::String("gmail".into()));
        assert!(require_string(&p, "account_id").is_ok());
    }

    #[test]
    fn require_u32_rejects_negative_fraction_and_overflow() {
        let mut p = Map::new();
        assert!(require_u32(&p, "limit").is_err()); // missing
        p.insert("limit".into(), json!(-1));
        assert!(require_u32(&p, "limit").is_err());
        p.insert("limit".into(), json!(1.5));
        assert!(require_u32(&p, "limit").is_err());
        p.insert("limit".into(), json!(u64::from(u32::MAX) + 1));
        assert!(require_u32(&p, "limit").is_err());
        p.insert("limit".into(), json!(42));
        assert!(require_u32(&p, "limit").is_ok());
    }
}
