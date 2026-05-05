//! Method dispatch for webview_apis requests.
//!
//! Maps a protocol method name to the Rust function that handles it.
//! Currently empty — the only consumer was the Gmail embedded-webview
//! bridge, which has been retired so the webview-account flow can stay
//! focused on social / messaging surfaces. Future connectors that want
//! to expose CDP-driven actions through the bridge plug their handlers
//! into [`dispatch_inner`] here.

use serde_json::{Map, Value};

/// Dispatch a single webview_apis request to its handler. Returns the
/// `result` JSON on success or a string error that the server relays
/// back as `{ ok: false, error }`.
///
/// Outcome logging lives here so the bridge has a single chokepoint
/// for success/failure traces — callers (tests, the WS server) keep
/// their own entry/exit logs but rely on this function to summarise
/// each dispatch decision.
pub async fn dispatch(method: &str, params: Map<String, Value>) -> Result<Value, String> {
    log::debug!("[webview_apis] dispatch method={method}");
    let out = dispatch_inner(method, params).await;
    match &out {
        Ok(_) => log::debug!("[webview_apis] dispatch ok method={method}"),
        Err(e) => log::warn!("[webview_apis] dispatch err method={method} error={e}"),
    }
    out
}

async fn dispatch_inner(method: &str, _params: Map<String, Value>) -> Result<Value, String> {
    Err(format!("unknown webview_apis method: {method}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unknown_method_is_rejected() {
        let err = dispatch("something.else", Map::new()).await.unwrap_err();
        assert!(err.contains("unknown webview_apis method"));
    }
}
