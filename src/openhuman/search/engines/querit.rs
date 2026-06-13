use crate::openhuman::config::Config;
use crate::openhuman::search::registry::SearchToolParams;
use crate::openhuman::tools::Tool;

pub(crate) fn build(root_config: &Config, params: SearchToolParams) -> Vec<Box<dyn Tool>> {
    tracing::debug!("[search] active engine = querit (BYO direct API)");

    let api_key = root_config.search.querit.api_key.clone();
    vec![
        Box::new(
            crate::openhuman::search::tools::QueritSearchTool::new_web_search_tool(
                api_key.clone(),
                None,
                params.max_results,
                params.timeout_secs,
            ),
        ),
        Box::new(crate::openhuman::search::tools::QueritSearchTool::new(
            api_key,
            None,
            params.max_results,
            params.timeout_secs,
        )),
    ]
}
