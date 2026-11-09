//! A stand-in for the TinyJuice module in host tests.
//!
//! [`compact_tool_output`](super::compact_tool_output) answers from [`STUB`]
//! instead of the loaded module when a test scopes one, so middleware tests can
//! drive the summary and compaction paths without a cdylib.

use std::sync::{Arc, Mutex};

use futures::future::BoxFuture;

use {
    tinyjuice_bus::wire::CompactRequest, tinyjuice_bus::wire::CompactResponse,
    tinyjuice_bus::wire::GenerateRequest,
};

pub(crate) type StubCompactor =
    Arc<dyn Fn(CompactRequest) -> BoxFuture<'static, CompactResponse> + Send + Sync>;

#[derive(Clone)]
pub(crate) struct RpcClientFixture {
    pub(crate) proxy: tinybus::Proxy,
    pub(crate) configs: Arc<Mutex<Vec<ObservedConfig>>>,
}

#[derive(Clone, Debug)]
pub(crate) struct ObservedConfig {
    pub(crate) workspace_dir: std::path::PathBuf,
    pub(crate) config_path: std::path::PathBuf,
    pub(crate) min_bytes_to_compress: usize,
}

tokio::task_local! {
    pub(crate) static STUB: StubCompactor;
    static RPC_CLIENT: RpcClientFixture;
}

/// Inject a TinyBus proxy for actual tokenjuice RPC operations and retain the
/// configuration each operation uses to construct its module client.
pub(crate) async fn with_rpc_client<F: std::future::Future>(
    fixture: RpcClientFixture,
    fut: F,
) -> F::Output {
    RPC_CLIENT.scope(fixture, fut).await
}

#[cfg(feature = "modules")]
pub(crate) fn rpc_client(config: &crate::config::Config) -> Option<tinybus::module::ModuleClient> {
    RPC_CLIENT
        .try_with(|fixture| {
            fixture.configs.lock().unwrap().push(ObservedConfig {
                workspace_dir: config.workspace_dir.clone(),
                config_path: config.config_path.clone(),
                min_bytes_to_compress: config.tokenjuice.min_bytes_to_compress,
            });
            crate::modules::ops::fixture_client(fixture.proxy.clone())
        })
        .ok()
}

/// Without `modules` there is no fixture client to build, so the caller
/// falls through to its own client.
#[cfg(not(feature = "modules"))]
pub(crate) fn rpc_client(_config: &crate::config::Config) -> Option<tinybus::module::ModuleClient> {
    None
}

/// Run `fut` with `stub` answering every compaction it makes.
pub(crate) async fn with_stub<F: std::future::Future>(stub: StubCompactor, fut: F) -> F::Output {
    STUB.scope(stub, fut).await
}

/// What a stubbed module response looks like when nothing changed.
pub(crate) fn passthrough(request: &CompactRequest) -> CompactResponse {
    let bytes = request.content.len();
    CompactResponse {
        text: request.content.clone(),
        original_bytes: bytes,
        compacted_bytes: bytes,
        rule_id: "none/plain_text".into(),
        applied: false,
        content_kind: "plain_text".into(),
        compressor: "none".into(),
        original_tokens: bytes.div_ceil(4) as u64,
        compacted_tokens: bytes.div_ceil(4) as u64,
        notice: None,
    }
}

/// The notice TinyJuice sends when the summary stage fails.
pub(crate) const FAILED_NOTICE: &str = "[summarization unavailable — stub]";

/// Emulate the module's summary stage: when the request carries a context
/// token, call back through `MlHost.Generate` the way the module does and
/// answer with the summary, a failure notice, or the content unchanged.
/// Every request is recorded.
pub(crate) fn summarizing(recorded: Arc<Mutex<Vec<CompactRequest>>>) -> StubCompactor {
    Arc::new(move |request: CompactRequest| {
        recorded.lock().unwrap().push(request.clone());
        Box::pin(async move {
            let Some(token) = request.context_token.clone() else {
                return passthrough(&request);
            };
            let reply = super::generate::serve(GenerateRequest {
                context_token: token,
                purpose: "tool_output_summary".into(),
                system: "stub system".into(),
                prompt: format!(
                    "focus={} content={}",
                    request.focus.clone().unwrap_or_default(),
                    request.content
                ),
                max_output_tokens: 64,
            })
            .await;
            match reply {
                Ok(Some(summary)) => CompactResponse {
                    compacted_bytes: summary.len(),
                    text: summary,
                    rule_id: "llm_summary".into(),
                    applied: true,
                    compressor: "llm_summary".into(),
                    ..passthrough(&request)
                },
                Ok(None) => passthrough(&request),
                Err(_) => CompactResponse {
                    notice: Some(FAILED_NOTICE.into()),
                    ..passthrough(&request)
                },
            }
        })
    })
}
