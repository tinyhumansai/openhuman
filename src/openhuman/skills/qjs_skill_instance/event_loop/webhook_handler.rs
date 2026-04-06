//! Webhook request handler for the skill event loop.
//!
//! This module provides the logic for processing incoming HTTP webhooks targeted
//! at a specific skill. It bridges the system-level webhook request into a
//! call to the skill's `onWebhookRequest` JavaScript handler.

use std::collections::HashMap;

use crate::openhuman::webhooks::WebhookResponseData;

use crate::openhuman::skills::qjs_skill_instance::js_handlers::handle_js_call;
use crate::openhuman::skills::qjs_skill_instance::js_helpers::{
    restore_auth_credential, restore_client_key, restore_oauth_credential,
};

/// Handle an incoming webhook request.
///
/// This function performs the following steps:
/// 1. Restores the skill's authentication credentials into the JS context.
/// 2. Invokes the `onWebhookRequest` JavaScript function with the request details.
/// 3. Parses the JavaScript response (status code, headers, body) into `WebhookResponseData`.
pub(crate) async fn handle_webhook_request(
    rt: &rquickjs::AsyncRuntime,
    ctx: &rquickjs::AsyncContext,
    skill_id: &str,
    correlation_id: String,
    method: String,
    path: String,
    headers: HashMap<String, serde_json::Value>,
    query: HashMap<String, String>,
    body: String,
    tunnel_id: String,
    tunnel_name: String,
    data_dir: &std::path::Path,
) -> Result<WebhookResponseData, String> {
    log::info!(
        "[skill:{}] event_loop: WebhookRequest {} {} (tunnel={})",
        skill_id,
        method,
        path,
        tunnel_id,
    );

    // Restore credentials in case the handler needs to make authenticated API calls
    restore_oauth_credential(ctx, skill_id, data_dir).await;
    restore_auth_credential(ctx, skill_id, data_dir).await;
    restore_client_key(ctx, skill_id, data_dir).await;

    let args = serde_json::json!({
        "correlationId": correlation_id,
        "method": method,
        "path": path,
        "headers": headers,
        "query": query,
        "body": body,
        "tunnelId": tunnel_id,
        "tunnelName": tunnel_name,
    });

    // Call the JavaScript onWebhookRequest handler
    match handle_js_call(rt, ctx, "onWebhookRequest", &args.to_string()).await {
        Ok(response_val) => {
            // Extract HTTP status code from the JS response object
            let status_code = response_val
                .get("statusCode")
                .and_then(|v| v.as_u64())
                .unwrap_or(200) as u16;
            
            // Extract headers from the JS response object
            let resp_headers: HashMap<String, String> = response_val
                .get("headers")
                .map(|v| match serde_json::from_value(v.clone()) {
                    Ok(h) => h,
                    Err(e) => {
                        log::warn!(
                            "[skill] Failed to parse webhook response headers: {e}, raw: {v}"
                        );
                        HashMap::new()
                    }
                })
                .unwrap_or_default();
            
            // Extract response body (usually a string or serialized JSON)
            let resp_body = response_val
                .get("body")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            log::debug!(
                "[skill:{}] event_loop: WebhookRequest handled -> status {}",
                skill_id,
                status_code,
            );

            Ok(WebhookResponseData {
                correlation_id,
                status_code,
                headers: resp_headers,
                body: resp_body,
            })
        }
        Err(e) => {
            log::warn!(
                "[skill:{}] event_loop: onWebhookRequest failed: {}",
                skill_id,
                e,
            );
            Err(e)
        }
    }
}
