//! Small utility functions used across archivist sub-modules.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::time::{SystemTime, UNIX_EPOCH};

/// Strip tool-call JSON blocks from an assistant response, leaving only the
/// prose text.
///
/// The archivist stores the full response (including `tool_calls_json`) in
/// the episodic log for diagnostic purposes. However, per the memory
/// ingestion policy, structured tool-call payloads must not reach the memory
/// tree — only the assistant's natural-language prose is ingested.
///
/// This function applies a lightweight heuristic: it removes any contiguous
/// spans of text that look like `<tool_call>…</tool_call>` XML/JSON blocks or
/// raw JSON objects that begin with `{"tool_calls":`. The output may be empty
/// if the entire response was tool-call markup — callers should handle that
/// case (empty text → no-op ingest).
pub(super) fn strip_tool_calls_from_response(response: &str) -> String {
    // Fast path: if the response contains no obvious tool-call markers, return
    // it unchanged to avoid unnecessary allocation.
    if !response.contains("<tool_call>")
        && !response.contains("{\"tool_calls\"")
        && !response.contains("\"tool_use\"")
    {
        return response.to_string();
    }

    // Remove XML-style tool-call blocks.
    let mut cleaned = response.to_string();

    // Strip <tool_call>…</tool_call> spans (may span multiple lines).
    while let Some(start) = cleaned.find("<tool_call>") {
        if let Some(end) = cleaned[start..].find("</tool_call>") {
            cleaned.drain(start..start + end + "</tool_call>".len());
        } else {
            // Unclosed tag — remove from the tag to end of string.
            cleaned.truncate(start);
            break;
        }
    }

    // Drop JSON / tool-use payload lines the XML strip above cannot catch
    // (evidence-vs-interpretation policy: tool-call payloads must never reach
    // tree ingest).
    cleaned = cleaned
        .lines()
        .filter(|line| {
            let l = line.trim();
            !(l.contains("\"tool_use\"")
                || l.starts_with("{\"tool_calls\"")
                || l.starts_with("\"tool_calls\""))
        })
        .collect::<Vec<_>>()
        .join("\n");

    // Trim and collapse runs of blank lines left by block removal.
    let trimmed = cleaned
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n");

    // Collapse more than two consecutive newlines to two.
    let mut result = String::with_capacity(trimmed.len());
    let mut blank_run = 0usize;
    for line in trimmed.lines() {
        if line.is_empty() {
            blank_run += 1;
            if blank_run <= 2 {
                result.push('\n');
            }
        } else {
            blank_run = 0;
            result.push_str(line);
            result.push('\n');
        }
    }

    result.trim().to_string()
}

/// Extract simple lessons from tool call outcomes (no LLM needed).
pub(super) fn extract_lesson_from_tools(
    tool_calls: &[crate::openhuman::agent::hooks::ToolCallRecord],
) -> Option<String> {
    let failures: Vec<&str> = tool_calls
        .iter()
        .filter(|tc| !tc.success)
        .map(|tc| tc.name.as_str())
        .collect();

    if failures.is_empty() {
        return None;
    }

    Some(format!(
        "Tools that failed in this turn: {}",
        failures.join(", ")
    ))
}

/// Extract a short profile key from event content (first few meaningful words).
pub(crate) fn extract_profile_key(content: &str, prefix: &str) -> String {
    let words: Vec<&str> = content
        .split_whitespace()
        .filter(|w| w.len() > 2)
        .take(4)
        .collect();
    let key = words.join("_").to_lowercase();
    let key = key
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect::<String>();
    if key.is_empty() {
        format!("{prefix}_unknown")
    } else {
        format!("{prefix}_{key}")
    }
}

/// Generate a simple UUID v4 (random).
pub(super) fn uuid_v4() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{:x}{:08x}", nanos, rand_u32())
}

/// Simple random u32 from system entropy.
fn rand_u32() -> u32 {
    let state = RandomState::new();
    let mut hasher = state.build_hasher();
    hasher.write_u64(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64,
    );
    hasher.finish() as u32
}
