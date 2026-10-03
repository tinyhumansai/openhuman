//! Recognising tool-call markup a sub-agent wrote as text instead of
//! executing (#6033), and stripping it to see whether any prose remains.

/// Whether `text` carries tool-call markup rather than (only) prose.
///
/// This is the single place the marker vocabulary is written down. The
/// delegation return path uses it to notice a sub-agent that emitted a tool
/// call instead of executing one (#6033).
pub(crate) fn looks_like_unexecuted_tool_call(text: &str) -> bool {
    contains_tool_call_payload(text) || text.contains("\"tool_use\"")
}

/// Whether `text` carries a **call-shaped** payload — an XML tool-call span
/// or a `tool_calls` JSON key.
///
/// Narrower than [`looks_like_unexecuted_tool_call`] on purpose: `"tool_use"`
/// alone appears in ordinary prose about the protocol, which is fine for
/// deciding whether stripping is worth attempting but not for deciding that
/// a sub-agent produced no answer.
///
/// The JSON key is matched without assuming it opens the object, so a
/// pretty-printed `{\n  "tool_calls": [...]\n}` is recognised too.
pub(crate) fn contains_tool_call_payload(text: &str) -> bool {
    text.contains("<tool_call>") || text.contains("\"tool_calls\"")
}

/// Strip tool-call JSON blocks from an assistant response, leaving only the
/// prose text.
///
/// This function applies a lightweight heuristic: it removes any contiguous
/// spans of text that look like `<tool_call>…</tool_call>` XML/JSON blocks or
/// raw JSON objects that begin with `{"tool_calls":`. The output may be empty
/// if the entire response was tool-call markup — callers should handle that
/// case (empty text → no-op ingest).
pub(crate) fn strip_tool_calls_from_response(response: &str) -> String {
    // Fast path: if the response contains no obvious tool-call markers, return
    // it unchanged to avoid unnecessary allocation.
    if !looks_like_unexecuted_tool_call(response) {
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

    // Drop JSON / tool-use payload lines the XML strip above cannot catch.
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

#[cfg(test)]
#[path = "tool_call_text_tests.rs"]
mod tests;
