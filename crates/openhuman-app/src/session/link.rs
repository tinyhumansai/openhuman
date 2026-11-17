//! `CoreLink` over the shell's own RPC path: the same `(url, token)` the
//! renderer uses, resolved per call so a gateway switch or a core restart is
//! picked up without re-wiring, and the same transport guard
//! (`core_rpc::post_json_rpc`) that refuses a bearer over plain HTTP off
//! loopback.

use async_trait::async_trait;
use openhuman_rpc::tinyhumans::CoreLink;
use serde_json::Value;

use crate::core_process::CoreProcessHandle;

pub struct HttpCoreLink {
    desktop: CoreProcessHandle,
}

impl HttpCoreLink {
    pub fn new(desktop: CoreProcessHandle) -> Self {
        Self { desktop }
    }
}

#[async_trait]
impl CoreLink for HttpCoreLink {
    async fn invoke(&self, method: &str, params: Value) -> Result<Value, String> {
        let (url, token) = crate::active_rpc_endpoint(&self.desktop).await;
        let body = openhuman_rpc::request_body(1, method, params).to_string();
        log::debug!(
            "[session][link] {method} -> {}",
            openhuman_rpc::redact_url_for_log(&url)
        );
        let token = (!token.is_empty()).then_some(token);
        let response = crate::core_rpc::post_json_rpc(&url, token.as_deref(), body).await?;
        openhuman_rpc::decode_response(response.status, &response.body)
    }
}
