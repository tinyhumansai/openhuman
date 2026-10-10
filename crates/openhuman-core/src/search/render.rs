//! Render a TinySearch response for the model and for the chat UI.
//!
//! The model gets plain text with a `(via <Provider>)` heading marker, which
//! the UI's `extractSearchProvider` also reads. The UI additionally gets a
//! host-only structured payload in `ToolResult::metadata`:
//! `{"kind":"web_search","query","provider","role"?,"answer"?,"citations"?,
//! "fallback_from"?,"results":[{"title","url","published"?,"excerpt"?}]}`.

use serde_json::{json, Value};
use tinysearch_bus::{ExecuteToolResponse, Role, SearchStatus};
use tinytools::ToolResult;

/// Excerpts in the structured payload are capped; the model-facing text keeps
/// its own, larger cap.
const METADATA_EXCERPT_CHARS: usize = 300;
const TEXT_EXCERPT_CHARS: usize = 500;

/// Display name for a provider id (`exa` → `Exa`).
pub fn provider_label(provider: &str) -> String {
    match provider {
        "exa" => "Exa".into(),
        "gemini" => "Gemini".into(),
        "gemini_deep_research" => "Gemini Deep Research".into(),
        "tinyfish" => "TinyFish".into(),
        "parallel" => "Parallel".into(),
        "brave" => "Brave".into(),
        "querit" => "Querit".into(),
        "tavily" => "Tavily".into(),
        "seltz" => "Seltz".into(),
        "searxng" => "SearXNG".into(),
        "keenable" => "Keenable".into(),
        other => other.to_string(),
    }
}

/// What the call was about: the query, or the URLs for a contents call.
pub fn subject(arguments: &Value) -> String {
    if let Some(query) = arguments.get("query").and_then(Value::as_str) {
        return query.trim().to_string();
    }
    if let Some(urls) = arguments.get("urls").and_then(Value::as_array) {
        return urls
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(", ");
    }
    arguments
        .get("objective")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// Build the tool result: text for the model, optional markdown, and the
/// structured payload for the UI.
pub fn render(
    response: &ExecuteToolResponse,
    subject: &str,
    max_results: usize,
    prefer_markdown: bool,
) -> ToolResult {
    let provider = provider_label(&response.provider);
    let mut result = ToolResult::success(render_text(response, subject, &provider, max_results));
    if prefer_markdown {
        result.markdown_formatted =
            Some(render_markdown(response, subject, &provider, max_results));
    }
    result.metadata = Some(metadata(response, subject, &provider, max_results));
    result
}

fn heading(response: &ExecuteToolResponse, subject: &str, provider: &str) -> String {
    let via = if response.fallback_from.is_empty() {
        format!("via {provider}")
    } else {
        let skipped: Vec<String> = response
            .fallback_from
            .iter()
            .map(|p| provider_label(p))
            .collect();
        format!("via {provider}, after {}", skipped.join(", "))
    };
    match (response.role, response.status) {
        (_, SearchStatus::InProgress) => format!("Research still running for: {subject} ({via})"),
        (Some(Role::Answer), _) => format!("Answer for: {subject} ({via})"),
        (Some(Role::Contents), _) => format!("Page contents for: {subject} ({via})"),
        _ if response.results.is_empty() && response.answer.is_none() => {
            format!("No results found for: {subject} ({via})")
        }
        _ => format!("Search results for: {subject} ({via})"),
    }
}

fn render_text(
    response: &ExecuteToolResponse,
    subject: &str,
    provider: &str,
    max_results: usize,
) -> String {
    let mut lines = vec![heading(response, subject, provider)];
    if let Some(answer) = response
        .answer
        .as_deref()
        .map(str::trim)
        .filter(|a| !a.is_empty())
    {
        lines.push(String::new());
        lines.push(answer.to_string());
    }
    for (index, item) in response.results.iter().take(max_results).enumerate() {
        let title = if item.title.trim().is_empty() {
            "No title"
        } else {
            item.title.trim()
        };
        lines.push(format!("{}. {}", index + 1, title));
        lines.push(format!("   {}", item.url.trim()));
        if let Some(date) = item
            .published
            .as_deref()
            .map(str::trim)
            .filter(|d| !d.is_empty())
        {
            lines.push(format!("   Published: {date}"));
        }
        if let Some(snippet) = item
            .snippet
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            lines.push(format!(
                "   {}",
                crate::util::truncate_with_suffix(snippet, TEXT_EXCERPT_CHARS, "…")
            ));
        }
    }
    if !response.citations.is_empty() {
        lines.push(String::new());
        lines.push("Sources:".into());
        for (index, citation) in response.citations.iter().enumerate() {
            match citation
                .title
                .as_deref()
                .map(str::trim)
                .filter(|t| !t.is_empty())
            {
                Some(title) => lines.push(format!("[{}] {} — {}", index + 1, title, citation.url)),
                None => lines.push(format!("[{}] {}", index + 1, citation.url)),
            }
        }
    }
    lines.join("\n")
}

fn render_markdown(
    response: &ExecuteToolResponse,
    subject: &str,
    provider: &str,
    max_results: usize,
) -> String {
    let mut out = format!("# {}\n", heading(response, subject, provider));
    if let Some(answer) = response
        .answer
        .as_deref()
        .map(str::trim)
        .filter(|a| !a.is_empty())
    {
        out.push_str(&format!("\n{answer}\n"));
    }
    for item in response.results.iter().take(max_results) {
        let title = if item.title.trim().is_empty() {
            "Untitled"
        } else {
            item.title.trim()
        };
        out.push_str(&format!("\n## [{title}]({})\n", item.url.trim()));
        if let Some(date) = item
            .published
            .as_deref()
            .map(str::trim)
            .filter(|d| !d.is_empty())
        {
            out.push_str(&format!("_Published: {date}_\n\n"));
        }
        if let Some(snippet) = item
            .snippet
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            out.push_str(&format!(
                "> {}\n",
                crate::util::truncate_with_suffix(snippet, TEXT_EXCERPT_CHARS, "…")
            ));
        }
    }
    if !response.citations.is_empty() {
        out.push_str("\n### Sources\n");
        for citation in &response.citations {
            let title = citation
                .title
                .as_deref()
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .unwrap_or(citation.url.as_str());
            out.push_str(&format!("- [{title}]({})\n", citation.url));
        }
    }
    out
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
