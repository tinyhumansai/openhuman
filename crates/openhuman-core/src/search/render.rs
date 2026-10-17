//! Render a TinySearch response for the model and for the chat UI.
//!
//! The model gets plain text (rendered by `tinysearch_bus::render`) with a `(via <Provider>)` heading marker, which
//! the UI's `extractSearchProvider` also reads. The UI additionally gets a
//! host-only structured payload in `ToolResult::metadata`:
//! `{"kind":"web_search","query","provider","role"?,"answer"?,"citations"?,
//! "fallback_from"?,"results":[{"title","url","published"?,"excerpt"?}]}`.

use serde_json::{json, Value};
use tinysearch_bus::labels::provider_label;
use tinysearch_bus::{render as bus_render, ExecuteToolResponse, SearchStatus};
use tinytools::ToolResult;

/// Excerpts in the structured payload are capped.
const METADATA_EXCERPT_CHARS: usize = 300;

pub use tinysearch_bus::render::subject;

/// Build the tool result: text for the model, optional markdown, and the
/// structured payload for the UI.
pub fn render(
    response: &ExecuteToolResponse,
    subject: &str,
    max_results: usize,
    prefer_markdown: bool,
) -> ToolResult {
    let provider = provider_label(&response.provider);
    let mut result = ToolResult::success(bus_render::render_text(response, subject, max_results));
    if prefer_markdown {
        result.markdown_formatted =
            Some(bus_render::render_markdown(response, subject, max_results));
    }
    result.metadata = Some(metadata(response, subject, &provider, max_results));
    result
}

fn metadata(
    response: &ExecuteToolResponse,
    subject: &str,
    provider: &str,
    max_results: usize,
) -> Value {
    let results: Vec<Value> = response
        .results
        .iter()
        .take(max_results)
        .map(|item| {
            let mut obj = json!({ "title": item.title, "url": item.url });
            if let Some(published) = item
                .published
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                obj["published"] = json!(published);
            }
            if let Some(snippet) = item
                .snippet
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                obj["excerpt"] = json!(crate::util::truncate_with_ellipsis(
                    snippet,
                    METADATA_EXCERPT_CHARS
                ));
            }
            obj
        })
        .collect();
    let mut payload = json!({
        "kind": "web_search",
        "query": subject,
        "provider": provider,
        "results": results,
    });
    if let Some(role) = response.role {
        payload["role"] = json!(role);
    }
    if let Some(answer) = response
        .answer
        .as_deref()
        .map(str::trim)
        .filter(|a| !a.is_empty())
    {
        payload["answer"] = json!(answer);
    }
    if !response.citations.is_empty() {
        payload["citations"] = json!(response
            .citations
            .iter()
            .map(|c| json!({ "url": c.url, "title": c.title }))
            .collect::<Vec<_>>());
    }
    if !response.fallback_from.is_empty() {
        payload["fallback_from"] = json!(response
            .fallback_from
            .iter()
            .map(|p| provider_label(p))
            .collect::<Vec<_>>());
    }
    if response.status == SearchStatus::InProgress {
        payload["in_progress"] = json!(true);
    }
    payload
}

#[cfg(test)]
#[path = "render_tests.rs"]
mod tests;
