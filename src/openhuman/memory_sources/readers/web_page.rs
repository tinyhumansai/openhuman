//! Web page source reader.
//!
//! Fetches a single URL and extracts its text content. When a CSS
//! `selector` is configured, only matching elements are included;
//! otherwise the full page body is returned.

use async_trait::async_trait;

use crate::openhuman::config::Config;
use crate::openhuman::memory_sources::types::{
    ContentType, MemorySourceEntry, SourceContent, SourceItem, SourceKind,
};

use super::SourceReader;

pub struct WebPageReader;

#[async_trait]
impl SourceReader for WebPageReader {
    fn kind(&self) -> SourceKind {
        SourceKind::WebPage
    }

    async fn list_items(
        &self,
        source: &MemorySourceEntry,
        _config: &Config,
    ) -> Result<Vec<SourceItem>, String> {
        let url = source
            .url
            .as_deref()
            .ok_or("web_page source requires a url")?;

        Ok(vec![SourceItem {
            id: url.to_string(),
            title: source.label.clone(),
            updated_at_ms: None,
        }])
    }

    async fn read_item(
        &self,
        source: &MemorySourceEntry,
        item_id: &str,
        _config: &Config,
    ) -> Result<SourceContent, String> {
        let url = if item_id.starts_with("http") {
            item_id.to_string()
        } else {
            source.url.clone().ok_or("web_page source requires a url")?
        };

        // SSRF guard: only allow http(s) — reject file://, data://, etc.
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(format!(
                "web_page source requires an http(s) URL, got: {}",
                url.chars().take(64).collect::<String>()
            ));
        }

        tracing::debug!(
            host = %url
                .trim_start_matches("https://")
                .trim_start_matches("http://")
                .split(['/', '?', '#'])
                .next()
                .unwrap_or(""),
            selector = ?source.selector,
            "[memory_sources:web_page] reading item"
        );

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .map_err(|e| format!("failed to build http client: {e}"))?;
        let resp = client
            .get(&url)
            .header("User-Agent", "openhuman")
            .send()
            .await
            .map_err(|e| format!("failed to fetch page: {e}"))?;

        if !resp.status().is_success() {
            return Err(format!("page returned {}", resp.status()));
        }

        // Cap response body to 10 MiB so a hostile/giant page can't OOM us.
        const MAX_BODY_BYTES: u64 = 10 * 1024 * 1024;
        if let Some(len) = resp.content_length() {
            if len > MAX_BODY_BYTES {
                return Err(format!(
                    "page body exceeds {MAX_BODY_BYTES}-byte limit (Content-Length={len})"
                ));
            }
        }

        let bytes = resp
            .bytes()
            .await
            .map_err(|e| format!("failed to read page body: {e}"))?;
        if bytes.len() as u64 > MAX_BODY_BYTES {
            return Err(format!(
                "page body exceeds {MAX_BODY_BYTES}-byte limit (read {} bytes)",
                bytes.len()
            ));
        }
        let body = String::from_utf8_lossy(&bytes).into_owned();

        let extracted = if let Some(selector) = source.selector.as_deref() {
            extract_by_selector(&body, selector)
        } else {
            strip_html_tags(&body)
        };

        Ok(SourceContent {
            id: url.clone(),
            title: extract_title(&body).unwrap_or_else(|| url.clone()),
            body: extracted,
            content_type: ContentType::Plaintext,
            metadata: serde_json::json!({ "url": url }),
        })
    }
}

fn extract_title(html: &str) -> Option<String> {
    let start = html.find("<title")?;
    let content_start = html[start..].find('>')? + start + 1;
    let end = html[content_start..].find("</title>")? + content_start;
    Some(html[content_start..end].trim().to_string())
}

fn extract_by_selector(html: &str, selector: &str) -> String {
    // Simple tag-name selector support (e.g. "article", "main", "div.content")
    // For full CSS selector support, the `scraper` crate would be needed.
    // This handles the common case of a single tag name.
    let tag = selector.split('.').next().unwrap_or(selector).trim();

    if tag.is_empty() {
        return strip_html_tags(html);
    }

    let open = format!("<{tag}");
    let close = format!("</{tag}>");

    let mut result = String::new();
    let mut offset = 0;

    while let Some(start) = html[offset..].find(&open) {
        let abs_start = offset + start;
        let content_start = match html[abs_start..].find('>') {
            Some(i) => abs_start + i + 1,
            None => break,
        };
        if let Some(end_offset) = html[content_start..].find(&close) {
            let content = &html[content_start..content_start + end_offset];
            if !result.is_empty() {
                result.push_str("\n\n");
            }
            result.push_str(&strip_html_tags(content));
            offset = content_start + end_offset + close.len();
        } else {
            break;
        }
    }

    if result.is_empty() {
        strip_html_tags(html)
    } else {
        result
    }
}

fn strip_html_tags(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;
    let mut last_was_space = false;

    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                if !last_was_space && !result.is_empty() {
                    result.push(' ');
                    last_was_space = true;
                }
            }
            _ if !in_tag => {
                if ch.is_whitespace() {
                    if !last_was_space {
                        result.push(' ');
                        last_was_space = true;
                    }
                } else {
                    result.push(ch);
                    last_was_space = false;
                }
            }
            _ => {}
        }
    }

    result.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_html_tags_removes_tags() {
        let html = "<p>Hello <b>world</b></p>";
        assert_eq!(strip_html_tags(html), "Hello world");
    }

    #[test]
    fn extract_title_finds_title_tag() {
        let html = "<html><head><title>My Page</title></head><body></body></html>";
        assert_eq!(extract_title(html).as_deref(), Some("My Page"));
    }

    #[test]
    fn extract_by_selector_finds_tag_content() {
        let html = "<html><body><article><p>Important content</p></article><footer>skip</footer></body></html>";
        let result = extract_by_selector(html, "article");
        assert!(result.contains("Important content"));
        assert!(!result.contains("skip"));
    }

    #[test]
    fn extract_by_selector_fallback_on_missing_tag() {
        let html = "<html><body>All the text</body></html>";
        let result = extract_by_selector(html, "article");
        assert!(result.contains("All the text"));
    }
}
