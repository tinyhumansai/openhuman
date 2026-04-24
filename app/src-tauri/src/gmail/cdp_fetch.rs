//! CDP-driven authenticated HTTP fetch from within an attached session.
//!
//! Uses `Network.loadNetworkResource` + `IO.read` + `IO.close` to issue a
//! request that rides the session's cookie jar, without ever executing
//! page JavaScript. That lets us hit cookie-gated Gmail endpoints
//! (atom feed, print view) from Rust while still passing as the logged-in
//! user.
//!
//! Flow:
//!
//! 1. `Page.getFrameTree` → extract the main frameId.
//! 2. `Network.loadNetworkResource({ frameId, url, options: { includeCredentials: true } })`
//!    → returns `{ success, httpStatusCode, stream, headers }`.
//! 3. Loop `IO.read({ handle, size })` until `eof: true`, concatenating
//!    the (possibly base64-encoded) chunks.
//! 4. `IO.close({ handle })`.
//!
//! Returns the full response body as a `String`. Non-UTF-8 responses
//! are rejected rather than silently lossy-decoded — Gmail's feeds are
//! all UTF-8.

use base64::Engine;
use serde_json::{json, Value};

use crate::cdp::CdpConn;

/// Upper bound on per-`IO.read` chunk size. Gmail atom feeds top out at
/// a few hundred KB; a 256 KB read keeps the protocol round-trips low
/// without asking the browser to buffer the universe.
const READ_CHUNK: usize = 256 * 1024;

/// Issue an authenticated GET through the attached CDP session.
pub async fn fetch(cdp: &mut CdpConn, session: &str, url: &str) -> Result<String, String> {
    // `Network.loadNetworkResource` needs a frameId to charge the
    // request against — we use the main frame of the target.
    let frame_id = main_frame_id(cdp, session).await?;
    log::debug!("[gmail-cdp-fetch] session={session} frame={frame_id} url={url}");

    // Network must be enabled for loadNetworkResource to work on some
    // CDP builds. Enabling is idempotent — safe to call every fetch.
    let _ = cdp
        .call("Network.enable", json!({}), Some(session))
        .await
        .map_err(|e| format!("Network.enable: {e}"))?;

    let res = cdp
        .call(
            "Network.loadNetworkResource",
            json!({
                "frameId": frame_id,
                "url": url,
                "options": {
                    "disableCache": false,
                    "includeCredentials": true,
                },
            }),
            Some(session),
        )
        .await
        .map_err(|e| format!("Network.loadNetworkResource {url}: {e}"))?;

    let resource = res
        .get("resource")
        .ok_or_else(|| "loadNetworkResource: no `resource` in reply".to_string())?;
    let success = resource
        .get("success")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !success {
        let status = resource
            .get("httpStatusCode")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let net_error = resource
            .get("netErrorName")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        return Err(format!(
            "loadNetworkResource reported failure: status={status} netError={net_error}"
        ));
    }
    let stream_handle = resource
        .get("stream")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "loadNetworkResource: no `stream` handle in reply".to_string())?
        .to_string();

    let body = read_stream(cdp, session, &stream_handle).await;

    // Always close the stream, even if read failed. Otherwise the
    // browser keeps the response buffered.
    let _ = cdp
        .call(
            "IO.close",
            json!({ "handle": &stream_handle }),
            Some(session),
        )
        .await;

    body
}

async fn main_frame_id(cdp: &mut CdpConn, session: &str) -> Result<String, String> {
    let tree = cdp
        .call("Page.getFrameTree", json!({}), Some(session))
        .await
        .map_err(|e| format!("Page.getFrameTree: {e}"))?;
    tree.pointer("/frameTree/frame/id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "Page.getFrameTree: no frameTree.frame.id".to_string())
}

async fn read_stream(cdp: &mut CdpConn, session: &str, handle: &str) -> Result<String, String> {
    let mut out = String::new();
    loop {
        let chunk = cdp
            .call(
                "IO.read",
                json!({ "handle": handle, "size": READ_CHUNK }),
                Some(session),
            )
            .await
            .map_err(|e| format!("IO.read: {e}"))?;
        let data = chunk.get("data").and_then(|v| v.as_str()).unwrap_or("");
        let is_base64 = chunk
            .get("base64Encoded")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if is_base64 {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(data)
                .map_err(|e| format!("IO.read base64 decode: {e}"))?;
            let text =
                String::from_utf8(bytes).map_err(|e| format!("IO.read body not utf-8: {e}"))?;
            out.push_str(&text);
        } else {
            // CDP already returns `data` as a JSON string — tokio-tungstenite
            // parsed it into a `String` via serde_json, so the JSON escapes
            // (\uXXXX) are already resolved. We can push verbatim; no
            // byte-by-byte re-encoding.
            out.push_str(data);
        }
        let eof = chunk.get("eof").and_then(|v| v.as_bool()).unwrap_or(false);
        if eof {
            break;
        }
    }
    Ok(out)
}
