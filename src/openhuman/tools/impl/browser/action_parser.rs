use super::types::{BrowserAction, ResolvedBackend};
use serde_json::Value;

/// Parse a JSON `args` object into a typed `BrowserAction`.
pub(crate) fn parse_browser_action(
    action_str: &str,
    args: &Value,
) -> anyhow::Result<BrowserAction> {
    match action_str {
        "open" => {
            let url = args
                .get("url")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'url' for open action"))?;
            Ok(BrowserAction::Open { url: url.into() })
        }
        "snapshot" => Ok(BrowserAction::Snapshot {
            interactive_only: args
                .get("interactive_only")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true),
            compact: args
                .get("compact")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true),
            depth: args
                .get("depth")
                .and_then(serde_json::Value::as_u64)
                .map(|d| u32::try_from(d).unwrap_or(u32::MAX)),
        }),
        "click" => {
            let selector = args
                .get("selector")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'selector' for click"))?;
            Ok(BrowserAction::Click {
                selector: selector.into(),
            })
        }
        "fill" => {
            let selector = args
                .get("selector")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'selector' for fill"))?;
            let value = args
                .get("value")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'value' for fill"))?;
            Ok(BrowserAction::Fill {
                selector: selector.into(),
                value: value.into(),
            })
        }
        "type" => {
            let selector = args
                .get("selector")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'selector' for type"))?;
            let text = args
                .get("text")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'text' for type"))?;
            Ok(BrowserAction::Type {
                selector: selector.into(),
                text: text.into(),
            })
        }
        "get_text" => {
            let selector = args
                .get("selector")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'selector' for get_text"))?;
            Ok(BrowserAction::GetText {
                selector: selector.into(),
            })
        }
        "get_title" => Ok(BrowserAction::GetTitle),
        "get_url" => Ok(BrowserAction::GetUrl),
        "screenshot" => Ok(BrowserAction::Screenshot {
            path: args.get("path").and_then(|v| v.as_str()).map(String::from),
            full_page: args
                .get("full_page")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
        }),
        "wait" => Ok(BrowserAction::Wait {
            selector: args
                .get("selector")
                .and_then(|v| v.as_str())
                .map(String::from),
            ms: args.get("ms").and_then(serde_json::Value::as_u64),
            text: args.get("text").and_then(|v| v.as_str()).map(String::from),
        }),
        "press" => {
            let key = args
                .get("key")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'key' for press"))?;
            Ok(BrowserAction::Press { key: key.into() })
        }
        "hover" => {
            let selector = args
                .get("selector")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'selector' for hover"))?;
            Ok(BrowserAction::Hover {
                selector: selector.into(),
            })
        }
        "scroll" => {
            let direction = args
                .get("direction")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'direction' for scroll"))?;
            Ok(BrowserAction::Scroll {
                direction: direction.into(),
                pixels: args
                    .get("pixels")
                    .and_then(serde_json::Value::as_u64)
                    .map(|p| u32::try_from(p).unwrap_or(u32::MAX)),
            })
        }
        "is_visible" => {
            let selector = args
                .get("selector")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'selector' for is_visible"))?;
            Ok(BrowserAction::IsVisible {
                selector: selector.into(),
            })
        }
        "close" => Ok(BrowserAction::Close),
        "find" => {
            let by = args
                .get("by")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'by' for find"))?;
            let value = args
                .get("value")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'value' for find"))?;
            let action = args
                .get("find_action")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'find_action' for find"))?;
            Ok(BrowserAction::Find {
                by: by.into(),
                value: value.into(),
                action: action.into(),
                fill_value: args
                    .get("fill_value")
                    .and_then(|v| v.as_str())
                    .map(String::from),
            })
        }
        other => anyhow::bail!("Unsupported browser action: {other}"),
    }
}

pub(crate) fn is_supported_browser_action(action: &str) -> bool {
    matches!(
        action,
        "open"
            | "snapshot"
            | "click"
            | "fill"
            | "type"
            | "get_text"
            | "get_title"
            | "get_url"
            | "screenshot"
            | "wait"
            | "press"
            | "hover"
            | "scroll"
            | "is_visible"
            | "close"
            | "find"
            | "mouse_move"
            | "mouse_click"
            | "mouse_drag"
            | "key_type"
            | "key_press"
            | "screen_capture"
    )
}

pub(crate) fn is_computer_use_only_action(action: &str) -> bool {
    matches!(
        action,
        "mouse_move" | "mouse_click" | "mouse_drag" | "key_type" | "key_press" | "screen_capture"
    )
}

pub(crate) fn backend_name(backend: ResolvedBackend) -> &'static str {
    match backend {
        ResolvedBackend::AgentBrowser => "agent_browser",
        ResolvedBackend::RustNative => "rust_native",
        ResolvedBackend::ComputerUse => "computer_use",
    }
}

pub(crate) fn unavailable_action_for_backend_error(
    action: &str,
    backend: ResolvedBackend,
) -> String {
    format!(
        "Action '{action}' is unavailable for backend '{}'",
        backend_name(backend)
    )
}
