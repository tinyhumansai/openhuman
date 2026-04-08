//! Network native operations: fetch and WebSockets.

use parking_lot::RwLock;
use rquickjs::{function::Async, Ctx, Function, Object};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use super::types::{js_err, WebSocketConnection, WebSocketState};

/// Shared HTTP client — built once and reused across all fetch calls within the application.
///
/// Using a shared client enables connection pooling, persistent TLS sessions,
/// and prevents per-request TLS handshake overhead that can cause hangs or performance degradation.
static HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

/// Returns a reference to the global shared HTTP client, initializing it if necessary.
fn get_http_client() -> &'static reqwest::Client {
    HTTP_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .use_rustls_tls()
            .connect_timeout(std::time::Duration::from_secs(10))
            // Disable connection pooling entirely — reused connections hang on
            // consecutive POST requests through the staging proxy.
            .pool_idle_timeout(std::time::Duration::from_millis(0))
            .pool_max_idle_per_host(0)
            .build()
            .expect("failed to build shared HTTP client")
    })
}

/// Registers network operations (fetch, WebSocket) onto the provided JavaScript object.
pub fn register<'js>(
    ctx: &Ctx<'js>,
    ops: &Object<'js>,
    ws_state: Arc<RwLock<WebSocketState>>,
) -> rquickjs::Result<()> {
    // ========================================================================
    // Fetch
    // Asynchronous implementation of the browser `fetch` API.
    // ========================================================================

    ops.set(
        "fetch",
        Function::new(
            ctx.clone(),
            Async(move |url: String, options: String| async move {
                let opts: serde_json::Value =
                    serde_json::from_str(&options).map_err(|e| js_err(e.to_string()))?;

                let method = opts["method"].as_str().unwrap_or("GET");
                let headers_obj = opts["headers"].as_object();
                let body = opts["body"].as_str();
                let timeout_secs = opts["timeout"]
                    .as_u64()
                    .or_else(|| opts["timeout"].as_f64().map(|f| f as u64))
                    .unwrap_or(30);

                log::debug!(
                    "[net.fetch] {} {} (timeout={}s)",
                    method,
                    &url,
                    timeout_secs
                );

                let client = get_http_client();
                let mut req = match method {
                    "GET" => client.get(&url),
                    "POST" => client.post(&url),
                    "PUT" => client.put(&url),
                    "PATCH" => client.patch(&url),
                    "DELETE" => client.delete(&url),
                    _ => client.get(&url),
                };

                req = req.timeout(std::time::Duration::from_secs(timeout_secs));

                if let Some(h) = headers_obj {
                    for (k, v) in h {
                        if let Some(val_str) = v.as_str() {
                            req = req.header(k, val_str);
                        }
                    }
                }

                if let Some(b) = body {
                    req = req.body(b.to_string());
                }

                // Hard safety net: tokio timeout wraps send+body read so a stalled
                // connection cannot block the QuickJS event loop indefinitely even
                // if the per-request timeout on the reqwest builder fails to fire.
                let total_deadline = std::time::Duration::from_secs(timeout_secs + 5);
                let response = tokio::time::timeout(total_deadline, req.send())
                    .await
                    .map_err(|_| js_err(format!("request timed out after {}s", timeout_secs + 5)))?
                    .map_err(|e| {
                        let mut msg = e.to_string();
                        let mut source = std::error::Error::source(&e);
                        while let Some(cause) = source {
                            msg.push_str(&format!(" | caused by: {cause}"));
                            source = std::error::Error::source(cause);
                        }
                        js_err(msg)
                    })?;

                let status = response.status().as_u16();
                let status_text = response
                    .status()
                    .canonical_reason()
                    .unwrap_or("")
                    .to_string();
                let headers: HashMap<String, String> = response
                    .headers()
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
                    .collect();
                log::debug!("[net.fetch] response status={} from {}", status, &url);

                let body_text = tokio::time::timeout(
                    std::time::Duration::from_secs(timeout_secs + 5),
                    response.text(),
                )
                .await
                .map_err(|_| js_err(format!("body read timed out after {}s", timeout_secs + 5)))?
                .map_err(|e| js_err(e.to_string()))?;

                log::debug!("[net.fetch] body read complete (len={})", body_text.len());

                let result = serde_json::json!({
                    "status": status,
                    "statusText": status_text,
                    "headers": headers,
                    "body": body_text,
                });

                Ok::<String, rquickjs::Error>(result.to_string())
            }),
        ),
    )?;

    // ========================================================================
    // WebSocket (Placeholders)
    // Implementation of basic WebSocket management.
    // ========================================================================

    {
        let ws = ws_state.clone();
        ops.set(
            "ws_connect",
            Function::new(
                ctx.clone(),
                Async(move |url: String| {
                    let ws = ws.clone();
                    async move {
                        let mut state = ws.write();
                        let id = state.next_id;
                        state.next_id += 1;
                        state.connections.insert(id, WebSocketConnection { url });
                        Ok::<u32, rquickjs::Error>(id)
                    }
                }),
            ),
        )?;
    }

    {
        let ws = ws_state.clone();
        ops.set(
            "ws_send",
            Function::new(ctx.clone(), move |_id: u32, _data: String| {
                let _state = ws.read();
            }),
        )?;
    }

    {
        let ws = ws_state.clone();
        ops.set(
            "ws_recv",
            Function::new(
                ctx.clone(),
                Async(move |_id: u32| {
                    let _ws = ws.clone();
                    async move { Ok::<Option<String>, rquickjs::Error>(None) }
                }),
            ),
        )?;
    }

    {
        let ws = ws_state;
        ops.set(
            "ws_close",
            Function::new(ctx.clone(), move |id: u32, _code: u16, _reason: String| {
                let mut state = ws.write();
                state.connections.remove(&id);
            }),
        )?;
    }
    Ok(())
}
