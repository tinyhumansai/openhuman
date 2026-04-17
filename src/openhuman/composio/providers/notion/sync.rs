//! Notion sync helpers — result extraction, pagination cursor,
//! page title extraction, and time utilities.

use serde_json::Value;

use super::pick_str;

/// Walk the Composio response envelope for Notion page results.
pub(crate) fn extract_results(data: &Value) -> Vec<Value> {
    let candidates = [
        data.pointer("/data/results"),
        data.pointer("/results"),
        data.pointer("/data/data/results"),
        data.pointer("/data/items"),
        data.pointer("/items"),
    ];
    for cand in candidates.into_iter().flatten() {
        if let Some(arr) = cand.as_array() {
            return arr.clone();
        }
    }
    Vec::new()
}

/// Extract the Notion pagination cursor (for `start_cursor` on the
/// next request).
pub(crate) fn extract_notion_cursor(data: &Value) -> Option<String> {
    let candidates = [
        data.pointer("/data/next_cursor"),
        data.pointer("/next_cursor"),
        data.pointer("/data/data/next_cursor"),
    ];
    for cand in candidates.into_iter().flatten() {
        if let Some(s) = cand.as_str() {
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

/// Try to extract a human-readable title from a Notion page object.
///
/// Notion pages store the title in `properties.title` or
/// `properties.Name.title[0].plain_text`. We try several shapes.
pub(crate) fn extract_page_title(page: &Value) -> Option<String> {
    // Try the common `properties.title.title[0].plain_text` shape.
    let props = page
        .get("properties")
        .or_else(|| page.get("data")?.get("properties"));
    if let Some(props) = props {
        // Walk all properties looking for a "title" type field.
        if let Some(obj) = props.as_object() {
            for (_key, val) in obj {
                if val.get("type").and_then(Value::as_str) == Some("title") {
                    if let Some(arr) = val.get("title").and_then(Value::as_array) {
                        let text: String = arr
                            .iter()
                            .filter_map(|t| t.get("plain_text").and_then(Value::as_str))
                            .collect::<Vec<_>>()
                            .join("");
                        if !text.is_empty() {
                            return Some(text);
                        }
                    }
                }
            }
        }
    }

    // Fallback: top-level "title" field (some Composio shapes).
    pick_str(page, &["title", "data.title", "name", "data.name"])
}

pub(crate) fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extract_results_from_data_results() {
        let data = json!({"data": {"results": [{"id": "page1"}]}});
        let results = extract_results(&data);
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn extract_results_from_top_level() {
        let data = json!({"results": [{"id": "a"}, {"id": "b"}]});
        let results = extract_results(&data);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn extract_results_from_data_items() {
        let data = json!({"data": {"items": [{"id": "x"}]}});
        let results = extract_results(&data);
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn extract_results_empty_when_no_match() {
        let data = json!({"foo": "bar"});
        assert!(extract_results(&data).is_empty());
    }

    #[test]
    fn extract_notion_cursor_from_data() {
        let data = json!({"data": {"next_cursor": "cur123"}});
        assert_eq!(extract_notion_cursor(&data), Some("cur123".into()));
    }

    #[test]
    fn extract_notion_cursor_from_top_level() {
        let data = json!({"next_cursor": "abc"});
        assert_eq!(extract_notion_cursor(&data), Some("abc".into()));
    }

    #[test]
    fn extract_notion_cursor_none_when_empty() {
        let data = json!({"data": {"next_cursor": "  "}});
        assert_eq!(extract_notion_cursor(&data), None);
    }

    #[test]
    fn extract_notion_cursor_none_when_missing() {
        assert_eq!(extract_notion_cursor(&json!({})), None);
    }

    #[test]
    fn extract_page_title_from_properties_title_type() {
        let page = json!({
            "properties": {
                "Name": {
                    "type": "title",
                    "title": [{"plain_text": "Hello"}, {"plain_text": " World"}]
                }
            }
        });
        assert_eq!(extract_page_title(&page), Some("Hello World".into()));
    }

    #[test]
    fn extract_page_title_from_nested_data_properties() {
        let page = json!({
            "data": {
                "properties": {
                    "Title": {
                        "type": "title",
                        "title": [{"plain_text": "My Page"}]
                    }
                }
            }
        });
        assert_eq!(extract_page_title(&page), Some("My Page".into()));
    }

    #[test]
    fn extract_page_title_fallback_to_top_level_title() {
        let page = json!({"title": "Fallback Title"});
        assert_eq!(extract_page_title(&page), Some("Fallback Title".into()));
    }

    #[test]
    fn extract_page_title_none_when_empty() {
        let page = json!({"properties": {"Name": {"type": "title", "title": []}}});
        // Empty title array means no text
        assert!(
            extract_page_title(&page).is_none() || extract_page_title(&page) == Some(String::new())
        );
    }

    #[test]
    fn extract_page_title_none_when_no_title_field() {
        let page = json!({"id": "123"});
        assert!(extract_page_title(&page).is_none());
    }

    #[test]
    fn now_ms_returns_nonzero() {
        assert!(now_ms() > 0);
    }
}
