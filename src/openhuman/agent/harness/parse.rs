use crate::openhuman::inference::provider::ToolCall;
use crate::openhuman::tools::Tool;
use regex::Regex;
use std::sync::LazyLock;

#[derive(Debug)]
pub(crate) struct ParsedToolCall {
    pub name: String,
    pub arguments: serde_json::Value,
    /// Provider-assigned call id when the call came from a native
    /// tool-use response. `None` for prompt-guided (XML-parsed)
    /// tool calls — progress emitters synthesise a fallback id.
    pub id: Option<String>,
}

/// Find a tool by name in the registry.
pub(crate) fn find_tool<'a>(tools: &'a [Box<dyn Tool>], name: &str) -> Option<&'a dyn Tool> {
    tools.iter().find(|t| t.name() == name).map(|t| t.as_ref())
}
pub(crate) fn parse_arguments_value(raw: Option<&serde_json::Value>) -> serde_json::Value {
    match raw {
        Some(serde_json::Value::String(s)) => serde_json::from_str::<serde_json::Value>(s)
            .unwrap_or_else(|_| serde_json::Value::Object(serde_json::Map::new())),
        Some(value) => value.clone(),
        None => serde_json::Value::Object(serde_json::Map::new()),
    }
}

/// Object keys that may carry the tool **arguments**, in priority order.
/// Models drift from the canonical `arguments` to `args`/`parameters`/etc.;
/// accepting these recovers an otherwise well-formed call (with a correct
/// `name`) instead of dropping it and burning an agent iteration
/// (bug-report-2026-05-26 A3). The tool **name** is deliberately left
/// strict — widening it would risk misreading a plain JSON answer as a
/// tool call in the whole-response parse path.
const TOOL_ARG_KEYS: &[&str] = &["arguments", "args", "parameters", "params", "input"];

/// Normalized arguments for the first present key among [`TOOL_ARG_KEYS`]
/// (via [`parse_arguments_value`], which tolerates both stringified and
/// object JSON). Empty-object default when none are present.
fn first_args_by_keys(obj: &serde_json::Value) -> serde_json::Value {
    for key in TOOL_ARG_KEYS {
        if let Some(v) = obj.get(*key) {
            return parse_arguments_value(Some(v));
        }
    }
    parse_arguments_value(None)
}

pub(crate) fn parse_tool_call_value(value: &serde_json::Value) -> Option<ParsedToolCall> {
    // Default to the permissive (tagged) behaviour: callers that reach a
    // value through an explicit tool-call marker (`tool_calls` array,
    // `<tool_call>` tags, ```tool_call blocks) accept the arg-key aliases.
    parse_tool_call_value_aliased(value, true)
}

/// Parse a single JSON value as a tool call.
///
/// `allow_arg_aliases` controls whether the generic argument-key aliases in
/// [`TOOL_ARG_KEYS`] (notably the very generic `input`) are honoured for a
/// **bare** `{ "name": .., .. }` object. The whole-response fallback path
/// (`parse_tool_calls` on a top-level JSON object) passes `false`: there, a
/// normal model reply such as `{"name":"Alice","input":"hi"}` must not have
/// its `input` slurped into tool arguments and routed to execution
/// (bug-report-2026-05-26 A3 follow-up). The `function`-wrapped shape stays
/// permissive regardless — the `function` key is an unambiguous tool-call
/// marker.
fn parse_tool_call_value_aliased(
    value: &serde_json::Value,
    allow_arg_aliases: bool,
) -> Option<ParsedToolCall> {
    if let Some(function) = value.get("function") {
        let name = function
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if !name.is_empty() {
            let arguments = first_args_by_keys(function);
            return Some(ParsedToolCall {
                name,
                arguments,
                id: None,
            });
        }
    }

    let name = value
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();

    if name.is_empty() {
        return None;
    }

    let arguments = if allow_arg_aliases {
        first_args_by_keys(value)
    } else {
        // Whole-response bare-object fallback: require the canonical
        // `arguments` key as an explicit tool-call marker. A plain JSON reply
        // that merely carries a `name` (e.g. {"name":"Alice","input":…}) must
        // stay plain text, not be dispatched as a tool call just because its
        // name happens to match a registered tool (CodeRabbit, #2683). Tagged
        // contexts (`<tool_call>`/`<invoke>`, `tool_calls` array, `function`
        // wrapper) reach this fn with `allow_arg_aliases = true` and keep the
        // permissive behaviour.
        match value.get("arguments") {
            Some(args) => parse_arguments_value(Some(args)),
            None => return None,
        }
    };
    Some(ParsedToolCall {
        name,
        arguments,
        id: None,
    })
}

pub(crate) fn parse_tool_calls_from_json_value(value: &serde_json::Value) -> Vec<ParsedToolCall> {
    // Tagged contexts (callers reach here via an explicit tool-call marker)
    // accept the argument-key aliases.
    parse_tool_calls_from_json_value_aliased(value, true)
}

/// Like [`parse_tool_calls_from_json_value`], but lets the caller forbid
/// generic arg-key aliases on a **bare** singleton/array object. The
/// `tool_calls`-keyed envelope always stays permissive — that key is an
/// unambiguous tool-call marker even on the whole-response path.
pub(crate) fn parse_tool_calls_from_json_value_aliased(
    value: &serde_json::Value,
    allow_arg_aliases: bool,
) -> Vec<ParsedToolCall> {
    let mut calls = Vec::new();

    if let Some(tool_calls) = value.get("tool_calls").and_then(|v| v.as_array()) {
        for call in tool_calls {
            // `tool_calls` entries are explicitly tool-call shaped → widen.
            if let Some(parsed) = parse_tool_call_value_aliased(call, true) {
                calls.push(parsed);
            }
        }

        if !calls.is_empty() {
            return calls;
        }
    }

    if let Some(array) = value.as_array() {
        for item in array {
            if let Some(parsed) = parse_tool_call_value_aliased(item, allow_arg_aliases) {
                calls.push(parsed);
            }
        }
        return calls;
    }

    if let Some(parsed) = parse_tool_call_value_aliased(value, allow_arg_aliases) {
        calls.push(parsed);
    }

    calls
}

const TOOL_CALL_OPEN_TAGS: [&str; 4] = ["<tool_call>", "<toolcall>", "<tool-call>", "<invoke>"];

pub(crate) fn find_first_tag<'a>(haystack: &str, tags: &'a [&'a str]) -> Option<(usize, &'a str)> {
    tags.iter()
        .filter_map(|tag| haystack.find(tag).map(|idx| (idx, *tag)))
        .min_by_key(|(idx, _)| *idx)
}

pub(crate) fn matching_tool_call_close_tag(open_tag: &str) -> Option<&'static str> {
    match open_tag {
        "<tool_call>" => Some("</tool_call>"),
        "<toolcall>" => Some("</toolcall>"),
        "<tool-call>" => Some("</tool-call>"),
        "<invoke>" => Some("</invoke>"),
        _ => None,
    }
}

/// `<invoke` prefix shared by the bare (`<invoke>`) and Claude-native
/// attribute (`<invoke name="…">`) forms.
const INVOKE_PREFIX: &str = "<invoke";

/// Locate the earliest Claude-native attribute-form `<invoke …>` open tag
/// (issue #3493). Matches `<invoke` only when the next character is whitespace
/// — i.e. attributes follow. The bare `<invoke>` form (next char `>`) is
/// intentionally skipped here; it is recognised as a literal tag with a JSON
/// body via [`TOOL_CALL_OPEN_TAGS`], preserving back-compat.
fn find_invoke_attr_tag(haystack: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(rel) = haystack[from..].find(INVOKE_PREFIX) {
        let idx = from + rel;
        let after = &haystack[idx + INVOKE_PREFIX.len()..];
        match after.chars().next() {
            Some(c) if c.is_whitespace() => return Some(idx),
            _ => from = idx + INVOKE_PREFIX.len(),
        }
    }
    None
}

/// Scalar policy for `<parameter>` values: a value that parses as JSON
/// (number, bool, null, array, object) is kept as that JSON type; anything
/// else — the common case of bare text — stays a string. Mirrors the tolerant
/// arg handling in [`parse_arguments_value`].
fn parameter_scalar_value(raw: &str) -> serde_json::Value {
    let trimmed = raw.trim();
    match serde_json::from_str::<serde_json::Value>(trimmed) {
        Ok(
            value @ (serde_json::Value::Number(_)
            | serde_json::Value::Bool(_)
            | serde_json::Value::Null
            | serde_json::Value::Array(_)
            | serde_json::Value::Object(_)),
        ) => value,
        _ => serde_json::Value::String(trimmed.to_string()),
    }
}

/// Parse a Claude-native attribute-form invoke block whose text begins
/// immediately after the `<invoke` prefix (at the attributes). Returns the
/// recovered call and the number of bytes consumed up to and including the
/// closing `</invoke>`. `None` when the `name` attribute or the closing tag is
/// missing — the caller then leaves the markup as text rather than dropping it.
fn parse_invoke_attribute_block(after_prefix: &str) -> Option<(ParsedToolCall, usize)> {
    static INVOKE_NAME_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"name\s*=\s*"([^"]*)""#).unwrap());
    static PARAMETER_RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?s)<parameter\s+name\s*=\s*"([^"]*)"\s*>(.*?)</parameter>"#).unwrap()
    });

    let open_end = after_prefix.find('>')?;
    let attrs = &after_prefix[..open_end];
    let name = INVOKE_NAME_RE
        .captures(attrs)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().trim().to_string())
        .filter(|n| !n.is_empty())?;

    let body = &after_prefix[open_end + 1..];
    let close_rel = body.find("</invoke>")?;
    let inner = &body[..close_rel];

    let mut arguments = serde_json::Map::new();
    for cap in PARAMETER_RE.captures_iter(inner) {
        // Groups 1 (name) and 2 (value) are mandatory in the pattern, so a
        // captured match always has both — index access is safe.
        let key = cap[1].trim();
        if key.is_empty() {
            continue;
        }
        arguments.insert(key.to_string(), parameter_scalar_value(&cap[2]));
    }

    let consumed = open_end + 1 + close_rel + "</invoke>".len();
    Some((
        ParsedToolCall {
            name,
            arguments: serde_json::Value::Object(arguments),
            id: None,
        },
        consumed,
    ))
}

pub(crate) fn extract_first_json_value_with_end(input: &str) -> Option<(serde_json::Value, usize)> {
    let trimmed = input.trim_start();
    let trim_offset = input.len().saturating_sub(trimmed.len());

    for (byte_idx, ch) in trimmed.char_indices() {
        if ch != '{' && ch != '[' {
            continue;
        }

        let slice = &trimmed[byte_idx..];
        let mut stream = serde_json::Deserializer::from_str(slice).into_iter::<serde_json::Value>();
        if let Some(Ok(value)) = stream.next() {
            let consumed = stream.byte_offset();
            if consumed > 0 {
                return Some((value, trim_offset + byte_idx + consumed));
            }
        }
    }

    None
}

pub(crate) fn strip_leading_close_tags(mut input: &str) -> &str {
    loop {
        let trimmed = input.trim_start();
        if !trimmed.starts_with("</") {
            return trimmed;
        }

        let Some(close_end) = trimmed.find('>') else {
            return "";
        };
        input = &trimmed[close_end + 1..];
    }
}

/// Extract JSON values from a string.
///
/// # Security Warning
///
/// This function extracts ANY JSON objects/arrays from the input. It MUST only
/// be used on content that is already trusted to be from the LLM, such as
/// content inside `<invoke>` tags where the LLM has explicitly indicated intent
/// to make a tool call. Do NOT use this on raw user input or content that
/// could contain prompt injection payloads.
pub(crate) fn extract_json_values(input: &str) -> Vec<serde_json::Value> {
    let mut values = Vec::new();
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return values;
    }

    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        values.push(value);
        return values;
    }

    let char_positions: Vec<(usize, char)> = trimmed.char_indices().collect();
    let mut idx = 0;
    while idx < char_positions.len() {
        let (byte_idx, ch) = char_positions[idx];
        if ch == '{' || ch == '[' {
            let slice = &trimmed[byte_idx..];
            let mut stream =
                serde_json::Deserializer::from_str(slice).into_iter::<serde_json::Value>();
            if let Some(Ok(value)) = stream.next() {
                let consumed = stream.byte_offset();
                if consumed > 0 {
                    values.push(value);
                    let next_byte = byte_idx + consumed;
                    while idx < char_positions.len() && char_positions[idx].0 < next_byte {
                        idx += 1;
                    }
                    continue;
                }
            }
        }
        idx += 1;
    }

    values
}

/// Find the end position of a JSON object by tracking balanced braces.
pub(crate) fn find_json_end(input: &str) -> Option<usize> {
    let trimmed = input.trim_start();
    let offset = input.len() - trimmed.len();

    if !trimmed.starts_with('{') {
        return None;
    }

    let mut depth = 0;
    let mut in_string = false;
    let mut escape_next = false;

    for (i, ch) in trimmed.char_indices() {
        if escape_next {
            escape_next = false;
            continue;
        }

        match ch {
            '\\' if in_string => escape_next = true,
            '"' => in_string = !in_string,
            '{' if !in_string => depth += 1,
            '}' if !in_string => {
                depth -= 1;
                if depth == 0 {
                    return Some(offset + i + ch.len_utf8());
                }
            }
            _ => {}
        }
    }

    None
}

/// Parse GLM-style tool calls from response text.
/// GLM uses proprietary formats like:
/// - `browser_open/url>https://example.com`
/// - `shell/command>ls -la`
/// - `http_request/url>https://api.example.com`
pub(crate) fn map_glm_tool_alias(tool_name: &str) -> &str {
    match tool_name {
        "browser_open" | "browser" | "web_search" | "shell" | "bash" => "shell",
        "http_request" | "http" => "http_request",
        _ => tool_name,
    }
}

pub(crate) fn build_curl_command(url: &str) -> Option<String> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return None;
    }

    if url.chars().any(char::is_whitespace) {
        return None;
    }

    let escaped = url.replace('\'', r#"'\\''"#);
    Some(format!("curl -s '{}'", escaped))
}

pub(crate) fn parse_glm_style_tool_calls(
    text: &str,
) -> Vec<(String, serde_json::Value, Option<String>)> {
    let mut calls = Vec::new();

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // Format: tool_name/param>value or tool_name/{json}
        if let Some(pos) = line.find('/') {
            let tool_part = &line[..pos];
            let rest = &line[pos + 1..];

            if tool_part.chars().all(|c| c.is_alphanumeric() || c == '_') {
                let tool_name = map_glm_tool_alias(tool_part);

                if let Some(gt_pos) = rest.find('>') {
                    let param_name = rest[..gt_pos].trim();
                    let value = rest[gt_pos + 1..].trim();

                    let arguments = match tool_name {
                        "shell" => {
                            if param_name == "url" {
                                let Some(command) = build_curl_command(value) else {
                                    continue;
                                };
                                serde_json::json!({"command": command})
                            } else if value.starts_with("http://") || value.starts_with("https://")
                            {
                                if let Some(command) = build_curl_command(value) {
                                    serde_json::json!({"command": command})
                                } else {
                                    serde_json::json!({"command": value})
                                }
                            } else {
                                serde_json::json!({"command": value})
                            }
                        }
                        "http_request" => {
                            serde_json::json!({"url": value, "method": "GET"})
                        }
                        _ => serde_json::json!({param_name: value}),
                    };

                    calls.push((tool_name.to_string(), arguments, Some(line.to_string())));
                    continue;
                }

                if rest.starts_with('{') {
                    if let Ok(json_args) = serde_json::from_str::<serde_json::Value>(rest) {
                        calls.push((tool_name.to_string(), json_args, Some(line.to_string())));
                    }
                }
            }
        }

        // Plain URL
        if let Some(command) = build_curl_command(line) {
            calls.push((
                "shell".to_string(),
                serde_json::json!({"command": command}),
                Some(line.to_string()),
            ));
        }
    }

    calls
}

/// Parse tool calls from an LLM response that uses XML-style function calling.
///
/// Expected format (common with system-prompt-guided tool use):
/// ```text
/// <tool_call>
/// {"name": "shell", "arguments": {"command": "ls"}}
/// </tool_call>
/// ```
///
/// Also accepts common tag variants (`<toolcall>`, `<tool-call>`) for model
/// compatibility.
///
/// Also supports JSON with `tool_calls` array from OpenAI-format responses.
pub(crate) fn parse_tool_calls(response: &str) -> (String, Vec<ParsedToolCall>) {
    let mut text_parts = Vec::new();
    let mut calls = Vec::new();
    let mut remaining = response;

    // First, try to parse as OpenAI-style JSON response with tool_calls array
    // This handles providers like Minimax that return tool_calls in native JSON format
    if let Ok(json_value) = serde_json::from_str::<serde_json::Value>(response.trim()) {
        // Whole-response parse: a bare top-level object/array is NOT an
        // explicit tool-call marker, so forbid the generic arg-key aliases
        // here (a plain `{"name":..,"input":..}` answer must stay text).
        // The `tool_calls`-keyed envelope is still honoured (it carries its
        // own marker) — handled inside the `_aliased` helper.
        calls = parse_tool_calls_from_json_value_aliased(&json_value, false);
        if !calls.is_empty() {
            // If we found tool_calls, extract any content field as text
            if let Some(content) = json_value.get("content").and_then(|v| v.as_str()) {
                if !content.trim().is_empty() {
                    text_parts.push(content.trim().to_string());
                }
            }
            return (text_parts.join("\n"), calls);
        }
    }

    // Fall back to XML-style tool-call tag parsing.
    loop {
        let literal = find_first_tag(remaining, &TOOL_CALL_OPEN_TAGS);
        let invoke_attr = find_invoke_attr_tag(remaining);

        // Choose the earliest-positioned recognised open tag. The bare
        // `<invoke>` literal and the attribute form `<invoke …>` never collide
        // at one offset (one is followed by `>`, the other by whitespace), so a
        // simple index comparison disambiguates them (issue #3493).
        let use_invoke_attr = match (invoke_attr, literal.as_ref()) {
            (Some(i), Some((l, _))) => i < *l,
            (Some(_), None) => true,
            _ => false,
        };

        if use_invoke_attr {
            let start = invoke_attr.expect("use_invoke_attr implies Some");
            let before = &remaining[..start];
            if !before.trim().is_empty() {
                text_parts.push(before.trim().to_string());
            }

            let after_prefix = &remaining[start + INVOKE_PREFIX.len()..];
            if let Some((parsed, consumed)) = parse_invoke_attribute_block(after_prefix) {
                calls.push(parsed);
                remaining = &after_prefix[consumed..];
                continue;
            }

            // Unparseable attribute-form block (no `name`/no close tag): leave
            // it and the rest as text instead of silently dropping content.
            tracing::warn!(
                body_chars = after_prefix.chars().count(),
                "[agent_parse] malformed <invoke> attribute block: missing name or close tag"
            );
            remaining = &remaining[start..];
            break;
        }

        let Some((start, open_tag)) = literal else {
            break;
        };

        // Everything before the tag is text.
        let before = &remaining[..start];
        if !before.trim().is_empty() {
            text_parts.push(before.trim().to_string());
        }

        let Some(close_tag) = matching_tool_call_close_tag(open_tag) else {
            break;
        };

        let after_open = &remaining[start + open_tag.len()..];
        if let Some(close_idx) = after_open.find(close_tag) {
            let inner = &after_open[..close_idx];
            let mut parsed_any = false;
            let json_values = extract_json_values(inner);
            for value in json_values {
                let parsed_calls = parse_tool_calls_from_json_value(&value);
                if !parsed_calls.is_empty() {
                    parsed_any = true;
                    calls.extend(parsed_calls);
                }
            }

            if !parsed_any {
                // body_chars only (never the body itself — it may carry tool
                // arguments with user data). Stable `[agent_parse]` prefix so
                // it aggregates with the other harness log families. Surfaces
                // how often the model emits an unparseable tool-call tag
                // (bug-report-2026-05-26 A3).
                tracing::warn!(
                    body_chars = inner.chars().count(),
                    "[agent_parse] malformed <tool_call> JSON: expected tool-call object in tag body"
                );
            }

            remaining = &after_open[close_idx + close_tag.len()..];
        } else {
            if let Some(json_end) = find_json_end(after_open) {
                if let Ok(value) =
                    serde_json::from_str::<serde_json::Value>(&after_open[..json_end])
                {
                    let parsed_calls = parse_tool_calls_from_json_value(&value);
                    if !parsed_calls.is_empty() {
                        calls.extend(parsed_calls);
                        remaining = strip_leading_close_tags(&after_open[json_end..]);
                        continue;
                    }
                }
            }

            if let Some((value, consumed_end)) = extract_first_json_value_with_end(after_open) {
                let parsed_calls = parse_tool_calls_from_json_value(&value);
                if !parsed_calls.is_empty() {
                    calls.extend(parsed_calls);
                    remaining = strip_leading_close_tags(&after_open[consumed_end..]);
                    continue;
                }
            }

            remaining = &remaining[start..];
            break;
        }
    }

    // If XML tags found nothing, try markdown code blocks with tool_call language.
    // Models behind OpenRouter sometimes output ```tool_call ... ``` or hybrid
    // ```tool_call ... </tool_call> instead of structured API calls or XML tags.
    if calls.is_empty() {
        static MD_TOOL_CALL_RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(
                r"(?s)```(?:tool[_-]?call|invoke)\s*\n(.*?)(?:```|</tool[_-]?call>|</toolcall>|</invoke>)",
            )
            .unwrap()
        });
        let mut md_text_parts: Vec<String> = Vec::new();
        let mut last_end = 0;

        for cap in MD_TOOL_CALL_RE.captures_iter(response) {
            let full_match = cap.get(0).unwrap();
            let before = &response[last_end..full_match.start()];
            if !before.trim().is_empty() {
                md_text_parts.push(before.trim().to_string());
            }
            let inner = &cap[1];
            let json_values = extract_json_values(inner);
            for value in json_values {
                let parsed_calls = parse_tool_calls_from_json_value(&value);
                calls.extend(parsed_calls);
            }
            last_end = full_match.end();
        }

        if !calls.is_empty() {
            let after = &response[last_end..];
            if !after.trim().is_empty() {
                md_text_parts.push(after.trim().to_string());
            }
            text_parts = md_text_parts;
            remaining = "";
        }
    }

    // GLM-style tool calls (browser_open/url>https://..., shell/command>ls, etc.)
    if calls.is_empty() {
        let glm_calls = parse_glm_style_tool_calls(remaining);
        if !glm_calls.is_empty() {
            let mut cleaned_text = remaining.to_string();
            for (name, args, raw) in &glm_calls {
                calls.push(ParsedToolCall {
                    name: name.clone(),
                    arguments: args.clone(),
                    id: None,
                });
                if let Some(r) = raw {
                    cleaned_text = cleaned_text.replace(r, "");
                }
            }
            if !cleaned_text.trim().is_empty() {
                text_parts.push(cleaned_text.trim().to_string());
            }
            remaining = "";
        }
    }

    // SECURITY: We do NOT fall back to extracting arbitrary JSON from the response
    // here. That would enable prompt injection attacks where malicious content
    // (e.g., in emails, files, or web pages) could include JSON that mimics a
    // tool call. Tool calls MUST be explicitly wrapped in either:
    // 1. OpenAI-style JSON with a "tool_calls" array
    // 2. OpenHuman tool-call tags (<tool_call>, <toolcall>, <tool-call>)
    // 3. Markdown code blocks with tool_call/toolcall/tool-call language
    // 4. Explicit GLM line-based call formats (e.g. `shell/command>...`)
    // This ensures only the LLM's intentional tool calls are executed.

    // Remaining text after last tool call
    if !remaining.trim().is_empty() {
        text_parts.push(remaining.trim().to_string());
    }

    (text_parts.join("\n"), calls)
}

pub(crate) fn parse_structured_tool_calls(tool_calls: &[ToolCall]) -> Vec<ParsedToolCall> {
    tool_calls
        .iter()
        .map(|call| ParsedToolCall {
            name: call.name.clone(),
            arguments: serde_json::from_str::<serde_json::Value>(&call.arguments)
                .unwrap_or_else(|_| serde_json::Value::Object(serde_json::Map::new())),
            id: Some(call.id.clone()),
        })
        .collect()
}

/// Build assistant history entry in JSON format for native tool-call APIs.
/// `convert_messages` in the OpenRouter provider parses this JSON to reconstruct
/// the proper `NativeMessage` with structured `tool_calls`.
///
/// `reasoning_content` carries the model's thinking output (when the provider
/// surfaced it). It is persisted so the next request can replay it: DeepSeek's
/// thinking mode rejects an `assistant` turn that carries `tool_calls` if its
/// `reasoning_content` is not passed back (Sentry TAURI-RUST-4KB). Omitted from
/// the JSON when empty, so non-reasoning models are unaffected.
pub(crate) fn build_native_assistant_history(
    text: &str,
    reasoning_content: Option<&str>,
    tool_calls: &[ToolCall],
) -> String {
    let calls_json: Vec<serde_json::Value> = tool_calls
        .iter()
        .map(|tc| {
            let mut call = serde_json::json!({
                "id": tc.id,
                "name": tc.name,
                "arguments": tc.arguments,
            });
            // Persist Gemini's per-call `thought_signature` (TAURI-RUST-4PK /
            // 4PJ) into the stored assistant turn. PR #3553 threaded the
            // signature through the live response→request hop and the
            // stored-history *parser* (`parse_provider_tool_call_from_value`),
            // but this writer — the single sink the agent loop persists every
            // native tool-call turn through (engine/core.rs) — dropped it. On a
            // history reload the rebuilt assistant turn therefore lacked
            // `extra_content`, so the echoed `functionCall` part went out with
            // no `thought_signature` and Gemini 400'd ("Function call is
            // missing a thought_signature in functionCall parts"). Write it
            // per-part so EVERY call in a parallel/multi-call turn round-trips,
            // not just the first; `skip_serializing_if = "Option::is_none"` on
            // `extra_content` keeps the stored JSON byte-identical for every
            // provider that doesn't emit it.
            if let Some(extra) = tc.extra_content.clone() {
                if let Some(obj) = call.as_object_mut() {
                    obj.insert("extra_content".to_string(), extra);
                }
            }
            call
        })
        .collect();

    let content = if text.trim().is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::Value::String(text.trim().to_string())
    };

    let mut entry = serde_json::json!({
        "content": content,
        "tool_calls": calls_json,
    });

    if let Some(reasoning) = reasoning_content.map(str::trim).filter(|r| !r.is_empty()) {
        entry["reasoning_content"] = serde_json::Value::String(reasoning.to_string());
    }

    entry.to_string()
}

pub(crate) fn build_assistant_history_with_tool_calls(
    text: &str,
    tool_calls: &[ToolCall],
) -> String {
    let mut parts = Vec::new();

    if !text.trim().is_empty() {
        parts.push(text.trim().to_string());
    }

    for call in tool_calls {
        let arguments = serde_json::from_str::<serde_json::Value>(&call.arguments)
            .unwrap_or_else(|_| serde_json::Value::String(call.arguments.clone()));
        let payload = serde_json::json!({
            "id": call.id,
            "name": call.name,
            "arguments": arguments,
        });
        parts.push(format!("<tool_call>\n{payload}\n</tool_call>"));
    }

    parts.join("\n")
}

/// Convert a tool registry to OpenAI function-calling format for native tool support.
pub(crate) fn tools_to_openai_format(tools_registry: &[Box<dyn Tool>]) -> Vec<serde_json::Value> {
    tools_registry
        .iter()
        .map(|tool| {
            serde_json::json!({
                "type": "function",
                "function": {
                    "name": tool.name(),
                    "description": tool.description(),
                    "parameters": tool.parameters_schema()
                }
            })
        })
        .collect()
}

#[cfg(test)]
#[path = "parse_tests.rs"]
mod tests;
