use super::*;
use crate::core::runtime::DispatchCtx;

/// Best-effort authoring diagnostics delegated to TinyFlows' portable wiring
/// checker. OpenHuman contributes the request-scoped Composio contract lookup.
pub(crate) async fn graph_wiring_warnings(
    context: &DispatchCtx,
    config: &Config,
    graph: &Value,
) -> Result<Vec<String>, String> {
    wiring_warnings(context, config, graph.clone()).await
}

/// Author-time WARNs for a portable Copilot graph payload.
pub(crate) async fn wiring_warnings_json(
    context: &DispatchCtx,
    config: &Config,
    graph: Value,
) -> Result<Vec<String>, String> {
    wiring_warnings(context, config, graph).await
}

async fn wiring_warnings(
    context: &DispatchCtx,
    config: &Config,
    graph: Value,
) -> Result<Vec<String>, String> {
    let host = crate::flows::authoring_host::register(
        std::sync::Arc::new(config.clone()),
        context.clone(),
    )
    .await?;
    let proxy = crate::modules::tinyflows::authoring_proxy(config).await?;
    proxy
        .call::<Vec<String>>(
            tinyflows_bus::methods::WIRING_WARNINGS,
            &(tinyflows_bus::WiringWarningsRequest {
                graph,
                host: host.authoring.clone(),
            },),
        )
        .await
        .map_err(|error| error.wire_message())
}
