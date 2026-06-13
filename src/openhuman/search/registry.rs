use crate::openhuman::config::{Config, SearchEngine};
use crate::openhuman::tools::Tool;
use std::sync::Arc;

use super::engines;

#[derive(Clone, Copy)]
pub(crate) struct SearchToolParams {
    pub(crate) max_results: usize,
    pub(crate) timeout_secs: u64,
}

/// Build the complete agent-facing search tool surface for the configured
/// search engine.
///
/// Exactly one engine owns the canonical `web_search_tool` slot. When search is
/// disabled, this returns an empty list so search tools are absent from both the
/// agent prompt context and the runtime tool map.
pub fn build_search_tools(root_config: &Config) -> Vec<Box<dyn Tool>> {
    let search = &root_config.search;
    let params = SearchToolParams {
        max_results: search.max_results.clamp(1, 20),
        timeout_secs: search.timeout_secs.max(1),
    };

    let engine = search.effective_engine();
    let mut tools = match engine {
        SearchEngine::Disabled => engines::disabled::build(root_config, params),
        SearchEngine::Managed => engines::managed::build(root_config, params),
        SearchEngine::Parallel => engines::parallel::build(root_config, params),
        SearchEngine::Brave => engines::brave::build(root_config, params),
        SearchEngine::Querit => engines::querit::build(root_config, params),
    };

    if engine != SearchEngine::Disabled {
        tools.extend(build_backend_search_tools(root_config));
    }

    tools
}

fn build_backend_search_tools(root_config: &Config) -> Vec<Box<dyn Tool>> {
    let Some(client) = crate::openhuman::integrations::build_client(root_config) else {
        tracing::debug!("[search] no integration client — backend search tools skipped");
        return Vec::new();
    };

    let mut tools: Vec<Box<dyn Tool>> = Vec::new();
    if root_config.integrations.tinyfish.is_active() {
        tools.push(Box::new(
            crate::openhuman::search::tools::TinyFishSearchTool::new(Arc::clone(&client)),
        ));
        tools.push(Box::new(
            crate::openhuman::search::tools::TinyFishFetchTool::new(Arc::clone(&client)),
        ));
        tools.push(Box::new(
            crate::openhuman::search::tools::TinyFishAgentRunTool::new(Arc::clone(&client)),
        ));
        tracing::debug!("[search] registered tinyfish tools");
    } else {
        tracing::debug!("[search] tinyfish disabled — skipping");
    }

    tools
}

#[cfg(test)]
mod tests {
    use crate::openhuman::config::Config;

    #[test]
    fn disabled_engine_registers_no_search_tools() {
        let mut cfg = Config::default();
        cfg.search.engine = "disabled".to_string();

        let tools = super::build_search_tools(&cfg);

        assert!(tools.is_empty());
    }

    #[test]
    fn managed_engine_registers_unified_web_search_tool() {
        let mut cfg = Config::default();
        cfg.search.engine = "managed".to_string();

        let tools = super::build_search_tools(&cfg);
        let names = tools.iter().map(|tool| tool.name()).collect::<Vec<_>>();

        assert_eq!(names, vec!["web_search_tool"]);
    }

    #[test]
    fn brave_engine_registers_brave_search_family() {
        let mut cfg = Config::default();
        cfg.search.engine = "brave".to_string();
        cfg.search.brave.api_key = Some("test-key".to_string());

        let tools = super::build_search_tools(&cfg);
        let names = tools.iter().map(|tool| tool.name()).collect::<Vec<_>>();

        assert_eq!(
            names,
            vec![
                "web_search_tool",
                "brave_news_search",
                "brave_image_search",
                "brave_video_search"
            ]
        );
    }
}
