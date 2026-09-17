//! Pure complete-response Qwen normalizer.
//!
//! Normalizes Qwen model responses by enforcing single-action semantics,
//! recognizing canonical `<tool_call>` prompt-guided blocks when native calls
//! are absent, stripping raw tool/result/exit markup from visible text,
//! preserving thinking blocks separately, and classifying invalid calls.

use tinyinference::{
    message::{AssistantMessage, ContentBlock},
    model::ModelResponse,
    tool::{ToolCall, ToolSchema},
};

/// Classification of an invalid Qwen tool call.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum QwenInvalidCall {
    /// Payload is not valid JSON or violates canonical structure.
    Malformed { raw: String, reason: String },
    /// Tool name is not among the advertised schemas.
    Unknown { name: String, raw: Option<String> },
    /// Tool call object is missing the "name" field or it is not a valid string.
    MissingName { raw: String },
    /// Tool call object is missing the "arguments" field or it is not a JSON object.
    MissingArguments { name: Option<String>, raw: String },
    /// Arguments do not match the required fields or types of the advertised schema.
    SchemaMismatch {
        name: String,
        reason: String,
        raw: Option<String>,
    },
    /// Ambiguous call block or conflicting instructions.
    Ambiguous { reason: String, raw: Option<String> },
    /// Multiple tool calls were emitted when only a single action is permitted.
    MultipleCalls { count: usize, raw: Option<String> },
}

impl QwenInvalidCall {
    pub fn is_malformed(&self) -> bool {
        matches!(self, Self::Malformed { .. })
    }

    pub fn is_unknown(&self) -> bool {
        matches!(self, Self::Unknown { .. })
    }

    pub fn is_missing_name(&self) -> bool {
        matches!(self, Self::MissingName { .. })
    }

    pub fn is_missing_arguments(&self) -> bool {
        matches!(self, Self::MissingArguments { .. })
    }

    pub fn is_schema_mismatch(&self) -> bool {
        matches!(self, Self::SchemaMismatch { .. })
    }

    pub fn is_ambiguous(&self) -> bool {
        matches!(self, Self::Ambiguous { .. })
    }

    pub fn is_multiple(&self) -> bool {
        matches!(self, Self::MultipleCalls { .. })
    }

    pub fn tool_name(&self) -> Option<&str> {
        match self {
            Self::Unknown { name, .. }
            | Self::MissingArguments {
                name: Some(name), ..
            }
            | Self::SchemaMismatch { name, .. } => Some(name),
            _ => None,
        }
    }

    pub fn raw(&self) -> Option<&str> {
        match self {
            Self::Malformed { raw, .. }
            | Self::MissingName { raw }
            | Self::MissingArguments { raw, .. } => Some(raw),
            Self::Unknown { raw, .. }
            | Self::SchemaMismatch { raw, .. }
            | Self::Ambiguous { raw, .. }
            | Self::MultipleCalls { raw, .. } => raw.as_deref(),
        }
    }
}

impl std::fmt::Display for QwenInvalidCall {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Malformed { reason, .. } => write!(f, "malformed tool call: {reason}"),
            Self::Unknown { name, .. } => write!(f, "unknown tool: {name}"),
            Self::MissingName { .. } => write!(f, "missing tool call name"),
            Self::MissingArguments { name, .. } => {
                if let Some(name) = name {
                    write!(f, "missing arguments for tool '{name}'")
                } else {
                    write!(f, "missing tool call arguments")
                }
            }
            Self::SchemaMismatch { name, reason, .. } => {
                write!(f, "tool '{name}' schema mismatch: {reason}")
            }
            Self::Ambiguous { reason, .. } => write!(f, "ambiguous tool call: {reason}"),
            Self::MultipleCalls { count, .. } => {
                write!(
                    f,
                    "multiple tool calls emitted ({count}); only single action permitted"
                )
            }
        }
    }
}

impl std::error::Error for QwenInvalidCall {}

/// Result of Qwen response normalization.
#[derive(Debug, Clone)]
pub struct NormalizedQwenResponse {
    pub response: ModelResponse,
    pub invalid_call: Option<QwenInvalidCall>,
}

impl NormalizedQwenResponse {
    pub fn new(response: ModelResponse, invalid_call: Option<QwenInvalidCall>) -> Self {
        Self {
            response,
            invalid_call,
        }
    }

    pub fn into_parts(self) -> (ModelResponse, Option<QwenInvalidCall>) {
        (self.response, self.invalid_call)
    }
}

impl std::ops::Deref for NormalizedQwenResponse {
    type Target = ModelResponse;

    fn deref(&self) -> &Self::Target {
        &self.response
    }
}

impl std::ops::DerefMut for NormalizedQwenResponse {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.response
    }
}

impl From<NormalizedQwenResponse> for (ModelResponse, Option<QwenInvalidCall>) {
    fn from(res: NormalizedQwenResponse) -> Self {
        (res.response, res.invalid_call)
    }
}

impl From<NormalizedQwenResponse> for ModelResponse {
    fn from(res: NormalizedQwenResponse) -> Self {
        res.response
    }
}

/// Normalize an assembled Qwen model response against advertised tool schemas.
///
/// Rules:
/// 1. Native structured calls always win and retain their IDs/order, retaining only
///    the first action.
/// 2. When native calls are absent, canonical `<tool_call>{"name":"...","arguments":{...}}</tool_call>`
///    blocks are recognized if the name is advertised and arguments are compatible.
/// 3. Synthesize a stable ID for recovered calls.
/// 4. Stop parsing at tool-result or exit/final boundaries.
/// 5. Strip all raw tool/result/exit markup from visible text whether valid or invalid.
/// 6. Preserve thinking blocks separately and never expose them as visible text.
/// 7. Preserve response usage/raw/resolved metadata.
/// 8. Classify malformed, unknown, ambiguous/multiple, or missing name/arguments as invalid.
/// 9. Only the first valid action survives.
pub fn normalize_qwen_response(
    response: ModelResponse,
    advertised_tools: &[ToolSchema],
) -> NormalizedQwenResponse {
    // 1. Preserve existing thinking blocks and extract any inline thinking blocks from text.
    let mut thinking_blocks = Vec::new();
    let mut raw_prose_parts = Vec::new();

    for block in &response.message.content {
        match block {
            ContentBlock::Thinking { .. } | ContentBlock::RedactedThinking { .. } => {
                thinking_blocks.push(block.clone());
            }
            ContentBlock::Text(text) => {
                let (thinks, prose) = extract_thinking_and_prose(text);
                for think in thinks {
                    thinking_blocks.push(ContentBlock::thinking(think));
                }
                if !prose.is_empty() {
                    raw_prose_parts.push(prose);
                }
            }
            other => {
                thinking_blocks.push(other.clone());
            }
        }
    }

    let full_text = raw_prose_parts.concat();

    // 2. Native structured calls check.
    if !response.message.tool_calls.is_empty() {
        let original_count = response.message.tool_calls.len();
        let mut first_valid = None;
        let mut first_invalid = None;

        for call in &response.message.tool_calls {
            let schema_opt = advertised_tools.iter().find(|t| t.name == call.name);
            if let Some(schema) = schema_opt {
                match validate_tool_arguments(schema, &call.name, &call.arguments) {
                    Ok(()) => {
                        if first_valid.is_none() {
                            first_valid = Some(call.clone());
                        }
                    }
                    Err(reason) => {
                        if first_invalid.is_none() {
                            first_invalid = Some(QwenInvalidCall::SchemaMismatch {
                                name: call.name.clone(),
                                reason,
                                raw: None,
                            });
                        }
                    }
                }
            } else if first_invalid.is_none() {
                first_invalid = Some(QwenInvalidCall::Unknown {
                    name: call.name.clone(),
                    raw: None,
                });
            }
        }

        let (surviving_calls, invalid_call) = if let Some(valid) = first_valid {
            let inv = if original_count > 1 {
                Some(QwenInvalidCall::MultipleCalls {
                    count: original_count,
                    raw: None,
                })
            } else {
                None
            };
            (vec![valid], inv)
        } else {
            (Vec::new(), first_invalid)
        };

        let visible_text = strip_markup_for_visible_text(&full_text);
        let mut content = thinking_blocks;
        if !visible_text.is_empty() {
            content.push(ContentBlock::Text(visible_text));
        }

        let finish_reason = if surviving_calls.is_empty() {
            response.finish_reason.or_else(|| Some("stop".to_string()))
        } else {
            Some("tool_calls".to_string())
        };

        let normalized = ModelResponse {
            message: AssistantMessage {
                id: response.message.id,
                content,
                tool_calls: surviving_calls,
                usage: response.message.usage,
            },
            usage: response.usage,
            finish_reason,
            raw: response.raw,
            resolved_model: response.resolved_model,
            continue_turn: response.continue_turn,
            served_from_cache: response.served_from_cache,
        };

        return NormalizedQwenResponse::new(normalized, invalid_call);
    }

    // 3. Native calls absent: parse prompt-guided Qwen tool calls.
    let boundary_cutoff = find_boundary_cutoff(&full_text);
    let parseable_text = &full_text[..boundary_cutoff];

    let mut valid_calls = Vec::new();
    let mut first_invalid = None;
    let mut total_blocks = 0;

    let mut rest = parseable_text;
    while let Some(start) = rest.find("<tool_call>") {
        total_blocks += 1;
        let after_open = &rest[start + "<tool_call>".len()..];
        if let Some(end) = after_open.find("</tool_call>") {
            let body = &after_open[..end];
            let raw_block = format!("<tool_call>{body}</tool_call>");

            match serde_json::from_str::<serde_json::Value>(body.trim()) {
                Ok(val) => {
                    if let Some(obj) = val.as_object() {
                        let name_opt = obj.get("name").and_then(|v| v.as_str());
                        match name_opt {
                            Some(name) if !name.trim().is_empty() => {
                                let name = name.trim();
                                if let Some(args_val) = obj.get("arguments") {
                                    if args_val.is_object() {
                                        if let Some(schema) =
                                            advertised_tools.iter().find(|t| t.name == name)
                                        {
                                            match validate_tool_arguments(schema, name, args_val) {
                                                Ok(()) => {
                                                    let call_id = obj
                                                        .get("id")
                                                        .and_then(|v| v.as_str())
                                                        .map(|s| s.to_string())
                                                        .unwrap_or_else(|| {
                                                            format!("call_{}", valid_calls.len())
                                                        });
                                                    valid_calls.push(ToolCall::new(
                                                        call_id,
                                                        name.to_string(),
                                                        args_val.clone(),
                                                    ));
                                                }
                                                Err(reason) => {
                                                    if first_invalid.is_none() {
                                                        first_invalid =
                                                            Some(QwenInvalidCall::SchemaMismatch {
                                                                name: name.to_string(),
                                                                reason,
                                                                raw: Some(raw_block),
                                                            });
                                                    }
                                                }
                                            }
                                        } else if first_invalid.is_none() {
                                            first_invalid = Some(QwenInvalidCall::Unknown {
                                                name: name.to_string(),
                                                raw: Some(raw_block),
                                            });
                                        }
                                    } else if first_invalid.is_none() {
                                        first_invalid = Some(QwenInvalidCall::MissingArguments {
                                            name: Some(name.to_string()),
                                            raw: raw_block,
                                        });
                                    }
                                } else if first_invalid.is_none() {
                                    first_invalid = Some(QwenInvalidCall::MissingArguments {
                                        name: Some(name.to_string()),
                                        raw: raw_block,
                                    });
                                }
                            }
                            _ => {
                                if first_invalid.is_none() {
                                    first_invalid =
                                        Some(QwenInvalidCall::MissingName { raw: raw_block });
                                }
                            }
                        }
                    } else if first_invalid.is_none() {
                        first_invalid = Some(QwenInvalidCall::Malformed {
                            raw: raw_block,
                            reason: "tool call body must be a JSON object".to_string(),
                        });
                    }
                }
                Err(err) => {
                    if first_invalid.is_none() {
                        first_invalid = Some(QwenInvalidCall::Malformed {
                            raw: raw_block,
                            reason: format!("invalid JSON: {err}"),
                        });
                    }
                }
            }

            rest = &after_open[end + "</tool_call>".len()..];
        } else {
            let raw_block = rest[start..].to_string();
            if first_invalid.is_none() {
                first_invalid = Some(QwenInvalidCall::Malformed {
                    raw: raw_block,
                    reason: "unclosed <tool_call> tag".to_string(),
                });
            }
            break;
        }
    }

    if total_blocks == 0 && parseable_text.contains("</tool_call>") && first_invalid.is_none() {
        first_invalid = Some(QwenInvalidCall::Malformed {
            raw: "</tool_call>".to_string(),
            reason: "unmatched </tool_call> tag".to_string(),
        });
    }

    let (surviving_calls, invalid_call) = if !valid_calls.is_empty() {
        let first = valid_calls.remove(0);
        let inv = if total_blocks > 1 {
            first_invalid.or(Some(QwenInvalidCall::MultipleCalls {
                count: total_blocks,
                raw: None,
            }))
        } else {
            None
        };
        (vec![first], inv)
    } else {
        (Vec::new(), first_invalid)
    };

    let visible_text = strip_markup_for_visible_text(&full_text);
    let mut content = thinking_blocks;
    if !visible_text.is_empty() {
        content.push(ContentBlock::Text(visible_text));
    }

    let finish_reason = if surviving_calls.is_empty() {
        response.finish_reason.or_else(|| Some("stop".to_string()))
    } else {
        Some("tool_calls".to_string())
    };

    let normalized = ModelResponse {
        message: AssistantMessage {
            id: response.message.id,
            content,
            tool_calls: surviving_calls,
            usage: response.message.usage,
        },
        usage: response.usage,
        finish_reason,
        raw: response.raw,
        resolved_model: response.resolved_model,
        continue_turn: response.continue_turn,
        served_from_cache: response.served_from_cache,
    };

    NormalizedQwenResponse::new(normalized, invalid_call)
}

/// Alias for [`normalize_qwen_response`].
pub fn normalize(
    response: ModelResponse,
    advertised_tools: &[ToolSchema],
) -> NormalizedQwenResponse {
    normalize_qwen_response(response, advertised_tools)
}

/// Alias for [`normalize_qwen_response`] matching historical name.
pub fn normalize_qwen_tool_response(
    response: ModelResponse,
    advertised_tools: &[ToolSchema],
) -> NormalizedQwenResponse {
    normalize_qwen_response(response, advertised_tools)
}

/// Check whether a provider and model pair matches the exact local Qwen route.
pub fn is_local_qwen_route(provider_id: &str, model_name: &str) -> bool {
    let binding = format!("{provider_id}:{model_name}");
    binding == crate::agent::primary_orchestration::LOCAL_QWEN_PROVIDER_BINDING
}

/// Pure schema-informed correction builder accepting advertised tool schemas.
///
/// Instructs the model to return either direct final text or exactly one canonical
/// tool-call object containing exact `name` and object `arguments`, enumerating only
/// each advertised exact name and its parameter JSON schema, and stating that missing
/// names, arguments, or required values must not be invented. Never accepts or echoes
/// invalid model payload or user content.
pub fn build_protocol_correction_prompt(advertised_tools: &[ToolSchema]) -> String {
    let mut prompt = String::from(
        "Your previous response did not produce a valid tool call.\n\
Please return either direct final text answering the user or exactly one canonical tool-call object containing exact \"name\" and object \"arguments\".\n\
Do not invent missing tool names, arguments, or required values.\n\n\
Permitted tools and their parameter JSON schemas:\n",
    );

    if advertised_tools.is_empty() {
        prompt.push_str("(No tools are currently available. Please provide direct final text.)\n");
    } else {
        for tool in advertised_tools {
            let schema_str =
                serde_json::to_string(&tool.parameters).unwrap_or_else(|_| "{}".to_string());
            prompt.push_str(&format!(
                "- Tool name: \"{}\"\n  Parameters schema: {}\n",
                tool.name, schema_str
            ));
        }
    }

    prompt.push_str(
        "\nIf calling a tool, use the canonical format:\n\
{\"name\": \"<exact_tool_name>\", \"arguments\": { ... }}\n",
    );

    prompt
}

/// Pure constant terminal-answer helper returning clear user-safe text.
pub fn protocol_failure_terminal_answer() -> &'static str {
    "The tool-call format remained invalid and no action was taken."
}

fn validate_tool_arguments(
    schema: &ToolSchema,
    name: &str,
    arguments: &serde_json::Value,
) -> Result<(), String> {
    let args_obj = arguments
        .as_object()
        .ok_or_else(|| format!("arguments for '{name}' must be a JSON object"))?;

    // Check required fields from schema parameters
    if let Some(params_obj) = schema.parameters.as_object() {
        if let Some(required) = params_obj.get("required").and_then(|r| r.as_array()) {
            for req in required {
                if let Some(field) = req.as_str() {
                    if !args_obj.contains_key(field)
                        || args_obj.get(field) == Some(&serde_json::Value::Null)
                    {
                        return Err(format!(
                            "missing required parameter '{field}' for tool '{name}'"
                        ));
                    }
                }
            }
        }

        // Check property types if specified in schema
        if let Some(properties) = params_obj.get("properties").and_then(|p| p.as_object()) {
            for (key, val) in args_obj {
                if let Some(prop_spec) = properties.get(key).and_then(|s| s.as_object()) {
                    if let Some(expected_type) = prop_spec.get("type").and_then(|t| t.as_str()) {
                        let type_matches = match expected_type {
                            "string" => val.is_string(),
                            "number" => val.is_number(),
                            "integer" => val.is_i64() || val.is_u64(),
                            "boolean" => val.is_boolean(),
                            "array" => val.is_array(),
                            "object" => val.is_object(),
                            "null" => val.is_null(),
                            _ => true,
                        };
                        if !type_matches {
                            return Err(format!(
                                "parameter '{key}' for tool '{name}' expected type '{expected_type}', got {}",
                                json_type_name(val)
                            ));
                        }
                    }
                }
            }
        }
    }

    let dummy_call = ToolCall::new("val", name, arguments.clone());
    if let Err(e) = schema.validate_call(&dummy_call) {
        return Err(e.to_string());
    }

    Ok(())
}

fn json_type_name(val: &serde_json::Value) -> &'static str {
    match val {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(n) => {
            if n.is_i64() || n.is_u64() {
                "integer"
            } else {
                "number"
            }
        }
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

fn extract_thinking_and_prose(text: &str) -> (Vec<String>, String) {
    const THINK_PAIRS: [(&str, &str); 3] = [
        ("<think>", "</think>"),
        ("<thought>", "</thought>"),
        ("<reasoning>", "</reasoning>"),
    ];

    let mut thinkings = Vec::new();
    let mut current_text = text.to_string();

    for &(open_tag, close_tag) in &THINK_PAIRS {
        let mut cleaned = String::with_capacity(current_text.len());
        let mut rest = current_text.as_str();

        while let Some(start) = rest.find(open_tag) {
            cleaned.push_str(&rest[..start]);
            let after_open = &rest[start + open_tag.len()..];
            if let Some(end) = after_open.find(close_tag) {
                let thinking = after_open[..end].trim();
                if !thinking.is_empty() {
                    thinkings.push(thinking.to_string());
                }
                rest = &after_open[end + close_tag.len()..];
            } else {
                let thinking = after_open.trim();
                if !thinking.is_empty() {
                    thinkings.push(thinking.to_string());
                }
                rest = "";
                break;
            }
        }
        cleaned.push_str(rest);
        current_text = cleaned;
    }

    (thinkings, current_text)
}

fn find_boundary_cutoff(text: &str) -> usize {
    const BOUNDARIES: [&str; 15] = [
        "<tool_result>",
        "<tool_result ",
        "<tool_results>",
        "<tool_results ",
        "<tool_response>",
        "<tool_response ",
        "<result>",
        "<result ",
        "<exit>",
        "<exit/>",
        "<final>",
        "<final/>",
        "<final_answer>",
        "<final_answer/>",
        "<|im_end|>",
    ];

    let mut earliest = text.len();
    for boundary in &BOUNDARIES {
        if let Some(pos) = text.find(boundary) {
            if pos < earliest {
                earliest = pos;
            }
        }
    }
    earliest
}

fn strip_markup_for_visible_text(text: &str) -> String {
    const RESULT_BOUNDARIES: [&str; 8] = [
        "<tool_result>",
        "<tool_result ",
        "<tool_results>",
        "<tool_results ",
        "<tool_response>",
        "<tool_response ",
        "<result>",
        "<result ",
    ];

    let mut truncated = text;
    for rb in &RESULT_BOUNDARIES {
        if let Some(pos) = truncated.find(rb) {
            truncated = &truncated[..pos];
        }
    }

    let mut cleaned = String::with_capacity(truncated.len());
    let mut rest = truncated;
    while let Some(start) = rest.find("<tool_call>") {
        cleaned.push_str(&rest[..start]);
        let after_open = &rest[start + "<tool_call>".len()..];
        if let Some(end) = after_open.find("</tool_call>") {
            rest = &after_open[end + "</tool_call>".len()..];
            if cleaned.ends_with('\n') {
                if let Some(stripped) = rest.strip_prefix("\r\n") {
                    rest = stripped;
                } else if let Some(stripped) = rest.strip_prefix('\n') {
                    rest = stripped;
                }
            }
        } else {
            rest = "";
            break;
        }
    }
    cleaned.push_str(rest);

    let mut no_tool = String::with_capacity(cleaned.len());
    let mut rest = cleaned.as_str();
    while let Some(start) = rest.find("<tool>") {
        no_tool.push_str(&rest[..start]);
        let after_open = &rest[start + "<tool>".len()..];
        if let Some(end) = after_open.find("</tool>") {
            rest = &after_open[end + "</tool>".len()..];
        } else {
            rest = "";
            break;
        }
    }
    no_tool.push_str(rest);

    let tags_to_strip = [
        "<exit>",
        "</exit>",
        "<exit/>",
        "<final>",
        "</final>",
        "<final/>",
        "<final_answer>",
        "</final_answer>",
        "<final_answer/>",
        "<|im_end|>",
        "<|im_start|>",
        "<|endoftext|>",
        "</tool_call>",
        "</tool_result>",
        "</tool_results>",
        "</tool_response>",
    ];

    let mut result = no_tool;
    for tag in tags_to_strip {
        result = result.replace(tag, "");
    }

    result.trim().to_string()
}
