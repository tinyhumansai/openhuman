use crate::openhuman::config::Config;
use crate::openhuman::search::registry::SearchToolParams;
use crate::openhuman::tools::Tool;
use std::sync::Arc;

pub(crate) fn build(root_config: &Config, params: SearchToolParams) -> Vec<Box<dyn Tool>> {
    tracing::debug!(
        requested = %root_config.search.requested_engine_str(),
        "[search] active engine = managed (backend-proxied web_search)"
    );

    vec![Box::new(crate::openhuman::search::WebSearchTool::new(
        crate::openhuman::integrations::build_client(root_config),
        Some(Arc::new(root_config.clone())),
        params.max_results,
        params.timeout_secs,
    ))]
}
