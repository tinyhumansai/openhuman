//! Normalize and validate Composio action arguments before dispatch (#1797).

use serde_json::{json, Map, Value};

pub fn prepare_execute_arguments(tool: &str, arguments: Option<Value>) -> Result<Value, String> {
    let tool = tool.trim();
    let mut args = match arguments {
        Some(Value::Object(map)) => Value::Object(map),
        Some(Value::Null) | None => Value::Object(Map::new()),
        Some(other) => {
            return Err(format!(
                "composio: `{tool}` arguments must be a JSON object, got {}",
                other
            ));
        }
    };

    if tool.starts_with("GOOGLECALENDAR_") {
        normalize_calendar_time_bounds(&mut args)?;
    }
    if tool == "NOTION_FETCH_DATA" {
        ensure_notion_fetch_type(&mut args)?;
    }
    if tool == "GMAIL_SEND_EMAIL" {
        validate_gmail_send_email(&args)?;
    }
    if tool == "GMAIL_ADD_LABEL_TO_EMAIL" {
        validate_gmail_add_label(&args)?;
    }

    Ok(args)
}

fn normalize_calendar_time_bounds(args: &mut Value) -> Result<(), String> {
    let Some(obj) = args.as_object_mut() else {
        return Ok(());
    };
    for key in ["timeMin", "timeMax", "time_min", "time_max"] {
        if let Some(v) = obj.get(key).cloned() {
            if let Some(normalized) = normalize_rfc3339_bound(&v) {
                obj.insert(key.to_string(), Value::String(normalized));
            } else if v.is_string() {
                return Err(format!(
                    "GOOGLECALENDAR time bound `{key}` must be an RFC 3339 timestamp \
                     (e.g. 2026-05-14T00:00:00Z), not a bare date"
                ));
            }
        }
    }
    Ok(())
}

fn normalize_rfc3339_bound(value: &Value) -> Option<String> {
    let s = value.as_str()?.trim();
    if s.is_empty() {
        return None;
    }
    if s.contains('T') {
        return Some(s.to_string());
    }
    // A bare date like `2026-05-14` is promoted to RFC 3339 midnight UTC.
    // Parse explicitly so impossible dates such as `2026-99-99` are rejected
    // up front instead of being passed through to Google Calendar.
    if chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok() {
        return Some(format!("{s}T00:00:00Z"));
    }
    None
}

fn ensure_notion_fetch_type(args: &mut Value) -> Result<(), String> {
    let Some(obj) = args.as_object_mut() else {
        return Ok(());
    };
    let has_fetch_type = obj
        .get("fetch_type")
        .or_else(|| obj.get("fetchType"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .is_some_and(|s| !s.is_empty());
    if has_fetch_type {
        return Ok(());
    }
    let inferred = obj
        .get("filter")
        .and_then(|f| f.get("value").or_else(|| f.get("property")))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|v| match v {
            "page" | "pages" => "pages",
            "database" | "databases" => "databases",
            other => other,
        })
        .unwrap_or("pages");
    tracing::debug!(
        fetch_type = %inferred,
        "[composio][prepare] NOTION_FETCH_DATA: inferred fetch_type"
    );
    obj.insert("fetch_type".to_string(), json!(inferred));
    Ok(())
}

fn validate_gmail_send_email(args: &Value) -> Result<(), String> {
    let Some(obj) = args.as_object() else {
        return Err("GMAIL_SEND_EMAIL: arguments must be an object".to_string());
    };
    let recipient = obj
        .get("to")
        .or_else(|| obj.get("recipient_email"))
        .or_else(|| obj.get("recipientEmail"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if recipient.is_some() {
        return Ok(());
    }
    Err(
        "GMAIL_SEND_EMAIL: `to` (or `recipient_email`) is required — cannot send without a recipient"
            .to_string(),
    )
}

fn validate_gmail_add_label(args: &Value) -> Result<(), String> {
    let Some(obj) = args.as_object() else {
        return Err("GMAIL_ADD_LABEL_TO_EMAIL: arguments must be an object".to_string());
    };
    let message_id = obj
        .get("message_id")
        .or_else(|| obj.get("messageId"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if message_id.is_none() {
        return Err("GMAIL_ADD_LABEL_TO_EMAIL: `message_id` is required".to_string());
    }
    let add = non_empty_string_array(obj.get("add_label_ids").or_else(|| obj.get("addLabelIds")));
    let remove = non_empty_string_array(
        obj.get("remove_label_ids")
            .or_else(|| obj.get("removeLabelIds")),
    );
    if add || remove {
        return Ok(());
    }
    Err(
        "GMAIL_ADD_LABEL_TO_EMAIL: provide at least one non-empty label in `add_label_ids` or \
         `remove_label_ids`"
            .to_string(),
    )
}

fn non_empty_string_array(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Array(items)) => items
            .iter()
            .any(|v| v.as_str().map(str::trim).is_some_and(|s| !s.is_empty())),
        Some(Value::String(s)) => !s.trim().is_empty(),
        _ => false,
    }
}

#[cfg(test)]
#[path = "execute_prepare_tests.rs"]
mod tests;
