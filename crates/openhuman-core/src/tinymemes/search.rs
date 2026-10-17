//! OpenHuman's web search as the module's `Search` callback.
//!
//! Runs the same checks the `web_search` tool does before calling the search
//! module: search enabled in settings, the LocalOnly egress block, and at
//! least one provider able to serve the search role.

use serde_json::json;

use crate::config::Config;
use crate::core::runtime::DispatchCtx;

const TOOL: &str = tinysearch_bus::tools::WEB_SEARCH;

pub(crate) struct OpenHumanSearch {
    ctx: DispatchCtx,
    config: Config,
}

impl OpenHumanSearch {
    /// A search backend, or `None` when OpenHuman cannot search right now.
    pub(crate) fn available(ctx: &DispatchCtx, config: &Config) -> Option<Self> {
        if !crate::search::is_active(config) {
            return None;
        }
        if crate::search::tools::local_only_search_block(TOOL).is_some() {
            return None;
        }
        let role = tinysearch_bus::role_for_tool(TOOL)?;
        let resolved = crate::search::providers::resolve(config);
        if crate::search::providers::effective_role_providers(&resolved, config, role).is_empty() {
            return None;
        }
        Some(Self {
            ctx: ctx.clone(),
            config: config.clone(),
        })
    }
}

impl OpenHumanSearch {
    /// Run one search through the search module.
    pub(crate) async fn search(
        &self,
        query: &str,
        max_results: usize,
    ) -> Result<Vec<tinymemes_bus::SearchResult>, String> {
        let request = tinysearch_bus::ExecuteToolRequest {
            name: TOOL.to_string(),
            arguments: json!({ "query": query, "max_results": max_results.clamp(1, 20) }),
        };
        let response = crate::modules::search::execute_tool(&self.ctx, &self.config, request)
            .await
            .map_err(|e| crate::search::tools::user_facing_error(&e))?;
        Ok(response
            .results
            .into_iter()
            .map(|r| tinymemes_bus::SearchResult {
                title: r.title,
                url: r.url,
                snippet: r.snippet,
            })
            .collect())
    }
}

#[cfg(test)]
#[path = "search_tests.rs"]
mod tests;
