//! Network ops: fetch, WebSocket, net bridge.

use parking_lot::RwLock;
use rquickjs::{function::Async, Ctx, Function, Object};
use std::collections::HashMap;
use std::sync::Arc;

use super::types::{js_err, WebSocketConnection, WebSocketState};

pub fn register(ctx: &Ctx<'_>, ops: &Object<'_>, ws_state: Arc<RwLock<WebSocketState>>) -> rquickjs::Result<()> {
    // ========================================================================
    // Fetch (1) - ASYNC
    // ========================================================================

    ops.set("fetch", Function::new(ctx.clone(),
        Async(move |url: String, options: String| async move {
            let opts: serde_json::Value =
                serde_json::from_str(&options).map_err(|e| js_err(e.to_string()))?;

            let method = opts["method"].as_str().unwrap_or("GET");
            let headers_obj = opts["headers"].as_object();
            let body = opts["body"].as_str();

            let client = reqwest::Client::new();
            let mut req = match method {
                "GET" => client.get(&url),
                "POST" => client.post(&url),
                "PUT" => client.put(&url),
                "DELETE" => client.delete(&url),
                _ => client.get(&url),
            };

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

            let response = req.send().await.map_err(|e| js_err(e.to_string()))?;

            let status = response.status().as_u16();
            let status_text = response.status().canonical_reason().unwrap_or("").to_string();
            let headers: HashMap<String, String> = response
                .headers()
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
                .collect();
            let body_text = response.text().await.map_err(|e| js_err(e.to_string()))?;

            let result = serde_json::json!({
                "status": status,
                "statusText": status_text,
                "headers": headers,
                "body": body_text,
            });

            Ok::<String, rquickjs::Error>(result.to_string())
        }),
    ))?;

    // ========================================================================
    // WebSocket (4) - placeholders
    // ========================================================================

    {
        let ws = ws_state.clone();
        ops.set("ws_connect", Function::new(ctx.clone(),
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
        ))?;
    }

    {
        let ws = ws_state.clone();
        ops.set("ws_send", Function::new(ctx.clone(), move |_id: u32, _data: String| {
            let _state = ws.read();
        }))?;
    }

    {
        let ws = ws_state.clone();
        ops.set("ws_recv", Function::new(ctx.clone(),
            Async(move |_id: u32| {
                let _ws = ws.clone();
                async move { Ok::<Option<String>, rquickjs::Error>(None) }
            }),
        ))?;
    }

    {
        let ws = ws_state;
        ops.set("ws_close", Function::new(ctx.clone(), move |id: u32, _code: u16, _reason: String| {
            let mut state = ws.write();
            state.connections.remove(&id);
        }))?;
    }

    // ========================================================================
    // Net Bridge (1)
    // ========================================================================

    ops.set("net_fetch", Function::new(ctx.clone(),
        |url: String, options_json: String| -> rquickjs::Result<String> {
            crate::runtime::bridge::net::http_fetch(&url, &options_json).map_err(|e| js_err(e))
        },
    ))?;

    Ok(())
}
