use crate::openhuman::config::Config;
use crate::openhuman::search::registry::SearchToolParams;
use crate::openhuman::tools::Tool;

/// Exa BYOK: every tool here calls `https://api.exa.ai` directly with the
/// user's own key. Nothing in this surface routes through the managed backend.
pub(crate) fn build(root_config: &Config, params: SearchToolParams) -> Vec<Box<dyn Tool>> {
    tracing::debug!("[search] active engine = exa (BYO direct API)");

    let api_key = root_config.search.exa.api_key.clone();
    vec![
        Box::new(
            crate::openhuman::search::tools::ExaSearchTool::new_web_search_tool(
                api_key.clone(),
                None,
                params.max_results,
                params.timeout_secs,
            ),
        ),
        Box::new(crate::openhuman::search::tools::ExaSearchTool::new(
            api_key.clone(),
            None,
            params.max_results,
            params.timeout_secs,
        )),
        Box::new(crate::openhuman::search::tools::ExaFindSimilarTool::new(
            api_key.clone(),
            None,
            params.max_results,
            params.timeout_secs,
        )),
        Box::new(crate::openhuman::search::tools::ExaGetContentsTool::new(
            api_key,
            None,
            params.max_results,
            params.timeout_secs,
        )),
    ]
}
