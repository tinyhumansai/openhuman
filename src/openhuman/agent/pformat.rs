//! P-Format ("Parameter-Format") tool calls — compact, positional,
//! pipe-delimited tool invocations designed to slash the token cost of
//! text-based tool calling.
//!
//! # Why
//!
//! Standard JSON tool calls are heavy on tokens for what's actually a
//! simple instruction:
//!
//! ```text
//! {"name": "get_weather", "arguments": {"location": "London", "unit": "metric"}}
//! ```
//!
//! That's roughly 25 tokens. The same call in P-Format:
//!
//! ```text
//! get_weather[London|metric]
//! ```
//!
//! is ~5 tokens — an 80% reduction. Across a long agent loop with many
//! tool calls per turn, that compounds dramatically.
//!
//! # Spec
//!
//! - One call per `<tool_call>...</tool_call>` tag body.
//! - Form: `name[arg1|arg2|...|argN]`.
//! - `name` is the tool's registered name (alphanumerics + `_`).
//! - Arguments are **positional**, with the order pinned to the
//!   **alphabetical** sort of the JSON-schema property names. The
//!   project's `serde_json` build does not enable `preserve_order`, so
//!   `Map` iterates as a `BTreeMap` — alphabetical iteration is the
//!   only order we can produce deterministically without flipping a
//!   crate-wide feature flag, and it is stable across rebuilds and
//!   workspaces.
//! - The renderer always exposes the order in the tool catalogue
//!   (e.g. `get_weather[location|unit]`, `math[verbose|x|y]`), so the
//!   model never has to guess which slot maps to which parameter — it
//!   reads the signature line and copies that order verbatim.
//! - Empty calls: `tool_name[]` for zero-arg tools.
//! - Empty arguments: `tool_name[||value]` is three args, the first two
//!   being empty strings.
//! - Escapes: `\|` → `|`, `\]` → `]`, `\\` → `\`. Other backslashes
//!   pass through verbatim so URLs and Windows paths remain readable.
//! - Type coercion: schema property `type: integer | number | boolean`
//!   triggers parsing the string into the matching JSON value. Failed
//!   coercion falls back to a string so the model still gets *something*
//!   useful into the tool argument.
//!
//! # Trade-offs
//!
//! - **Positional only** — nested objects or arrays can't be expressed
//!   directly. Tools that need rich payloads should either flatten their
//!   schema, accept a JSON-blob string parameter, or be invoked via the
//!   legacy JSON-in-tag fallback (which the dispatcher attempts when
//!   p-format parsing returns `None`).
//! - **Tool registry required at parse time** — without the schema we
//!   can't reconstruct named arguments. The dispatcher caches a
//!   pre-computed `name → params` map at construction time so this
//!   stays fast and avoids holding a reference to the live tool slice.

use crate::openhuman::tools::Tool;
use serde_json::{Map, Value};
use std::collections::HashMap;

/// JSON-schema primitive type used for argument coercion. Anything we
/// don't recognise (objects, arrays, custom types) is treated as
/// `Other`, which preserves the raw string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PFormatParamType {
    String,
    Integer,
    Number,
    Boolean,
    Other,
}

impl PFormatParamType {
    /// Map a JSON-schema `type` value to the coercion enum. Schemas may
    /// expose `type` as either a single string (`"integer"`) or an
    /// array (`["integer", "null"]`); we accept both and pick the first
    /// non-`null` entry.
    pub fn from_schema_type(value: Option<&Value>) -> Self {
        let label = match value {
            Some(Value::String(s)) => s.as_str(),
            Some(Value::Array(items)) => items
                .iter()
                .find_map(|v| v.as_str().filter(|s| *s != "null"))
                .unwrap_or(""),
            _ => "",
        };
        match label {
            "string" => Self::String,
            "integer" => Self::Integer,
            "number" => Self::Number,
            "boolean" => Self::Boolean,
            _ => Self::Other,
        }
    }
}

/// One tool's positional parameter list, as the dispatcher needs it
/// at parse time.
#[derive(Debug, Clone)]
pub struct PFormatToolParams {
    /// Parameter names in declaration order.
    pub names: Vec<String>,
    /// Parallel slice of JSON types for coercion.
    pub types: Vec<PFormatParamType>,
}

impl PFormatToolParams {
    /// Pull the ordered parameter names + types out of a tool's
    /// JSON schema. Non-object schemas (rare, but possible for
    /// shell-style tools) return an empty list — the renderer falls
    /// back to `name[]`.
    ///
    /// Iteration order is alphabetical because `serde_json::Map` is
    /// a `BTreeMap` in this build (no `preserve_order` feature). The
    /// renderer always shows the resulting order in the tool catalogue
    /// so the model — and the parser — agree on the layout. See the
    /// module-level docs for the rationale.
    pub fn from_schema(schema: &Value) -> Self {
        let Some(props) = schema.get("properties").and_then(|p| p.as_object()) else {
            return Self {
                names: Vec::new(),
                types: Vec::new(),
            };
        };
        let mut names = Vec::with_capacity(props.len());
        let mut types = Vec::with_capacity(props.len());
        for (name, def) in props {
            names.push(name.clone());
            types.push(PFormatParamType::from_schema_type(def.get("type")));
        }
        Self { names, types }
    }
}

/// Pre-computed lookup of every tool's parameter list. Built once at
/// dispatcher construction time so the parser doesn't need to hold a
/// reference to the live `Vec<Box<dyn Tool>>` (which the agent owns).
///
/// The map preserves the spec contract: the parser refuses to invent
/// argument names for an unknown tool, so an LLM can't tunnel
/// arbitrary JSON in by guessing tool names that don't exist.
pub type PFormatRegistry = HashMap<String, PFormatToolParams>;

/// Build a [`PFormatRegistry`] from the agent's tool slice. Call this
/// once at construction time, before the tools are moved into the
/// agent — the result is owned and self-contained, so it survives the
/// move without keeping a reference back to the registry.
pub fn build_registry(tools: &[Box<dyn Tool>]) -> PFormatRegistry {
    tools
        .iter()
        .map(|t| {
            (
                t.name().to_string(),
                PFormatToolParams::from_schema(&t.parameters_schema()),
            )
        })
        .collect()
}

/// Render a single tool's p-format signature, e.g. `get_weather[location|unit]`.
/// Used when emitting the tool catalogue inside the system prompt so the
/// model sees the exact positional order it should produce.
pub fn render_signature(name: &str, params: &PFormatToolParams) -> String {
    if params.names.is_empty() {
        format!("{name}[]")
    } else {
        format!("{name}[{}]", params.names.join("|"))
    }
}

/// Convenience wrapper that renders a signature directly from a `Tool`.
/// Equivalent to building a `PFormatToolParams` first; cheaper for
/// one-off rendering paths that don't pre-compute a registry.
pub fn render_signature_from_tool(tool: &dyn Tool) -> String {
    let params = PFormatToolParams::from_schema(&tool.parameters_schema());
    render_signature(tool.name(), &params)
}

/// Parse a single p-format call body and return `(tool_name, args_json)`.
///
/// `body` is the inside of a `<tool_call>...</tool_call>` tag (after the
/// dispatcher has stripped the wrapping). The function expects exactly
/// one call — multi-call bodies should be split by the caller.
///
/// Returns `None` for any of:
/// - missing `[` or unbalanced `]`
/// - unknown tool name (defensive — refuses to invent argument names)
/// - non-identifier characters in the tool name
///
/// On a successful parse the returned JSON object is keyed by parameter
/// name (in declaration order), with values coerced to integers,
/// numbers, or booleans where the schema asks for it. Excess positional
/// arguments past the schema length are silently dropped — keeps the
/// parser permissive when a model adds a stray trailing pipe.
pub fn parse_call(body: &str, registry: &PFormatRegistry) -> Option<(String, Value)> {
    let trimmed = body.trim();

    // Locate the opening bracket. The closing bracket must be the
    // **last** character of the trimmed body — anything trailing it
    // (e.g. extra whitespace, JSON, prose) means this isn't a valid
    // p-format call and we leave it for the JSON fallback.
    let open = trimmed.find('[')?;
    if !trimmed.ends_with(']') {
        return None;
    }

    let name = trimmed[..open].trim();
    if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return None;
    }

    let inner = &trimmed[open + 1..trimmed.len() - 1];

    // Look up the parameter spec — required so we can map positional
    // values back to named JSON keys with the correct types.
    let params = registry.get(name)?;

    let raw_values = split_pipes(inner);
    let mut args = Map::with_capacity(params.names.len());
    for (i, raw) in raw_values.iter().enumerate() {
        let Some(param_name) = params.names.get(i) else {
            // Excess values: drop silently. The schema is the source
            // of truth for argument count.
            tracing::debug!(
                tool = name,
                index = i,
                "[pformat] dropping excess positional argument"
            );
            continue;
        };
        let coerced = coerce_value(raw, params.types.get(i).copied().unwrap_or(PFormatParamType::String));
        args.insert(param_name.clone(), coerced);
    }

    Some((name.to_string(), Value::Object(args)))
}

/// Split a p-format argument body on unescaped `|`. Honours `\|`,
/// `\]`, and `\\` escapes. An empty body produces an empty `Vec` (NOT
/// `vec![""]`) so a tool with zero parameters parses cleanly.
fn split_pipes(input: &str) -> Vec<String> {
    if input.is_empty() {
        return Vec::new();
    }

    let mut out = Vec::new();
    let mut current = String::new();
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.peek() {
                Some('|') => {
                    current.push('|');
                    chars.next();
                }
                Some(']') => {
                    current.push(']');
                    chars.next();
                }
                Some('\\') => {
                    current.push('\\');
                    chars.next();
                }
                _ => current.push('\\'),
            }
        } else if c == '|' {
            out.push(std::mem::take(&mut current));
        } else {
            current.push(c);
        }
    }

    out.push(current);
    out
}

/// Coerce a raw string argument into the JSON type the schema expects.
/// Falls back to `Value::String` for any failed coercion so the model
/// still gets a usable value into the tool argument map.
fn coerce_value(raw: &str, ty: PFormatParamType) -> Value {
    match ty {
        PFormatParamType::Integer => raw
            .trim()
            .parse::<i64>()
            .map(|n| Value::Number(n.into()))
            .unwrap_or_else(|_| Value::String(raw.to_string())),
        PFormatParamType::Number => raw
            .trim()
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map(Value::Number)
            .unwrap_or_else(|| Value::String(raw.to_string())),
        PFormatParamType::Boolean => match raw.trim().to_ascii_lowercase().as_str() {
            "true" | "yes" | "1" => Value::Bool(true),
            "false" | "no" | "0" => Value::Bool(false),
            _ => Value::String(raw.to_string()),
        },
        PFormatParamType::String | PFormatParamType::Other => Value::String(raw.to_string()),
    }
}

// ──────────────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_registry() -> PFormatRegistry {
        let mut reg = PFormatRegistry::new();
        reg.insert(
            "get_weather".to_string(),
            PFormatToolParams::from_schema(&json!({
                "type": "object",
                "properties": {
                    "location": { "type": "string" },
                    "unit": { "type": "string" }
                }
            })),
        );
        reg.insert(
            "shell".to_string(),
            PFormatToolParams::from_schema(&json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string" }
                }
            })),
        );
        reg.insert(
            "ping".to_string(),
            PFormatToolParams::from_schema(&json!({
                "type": "object",
                "properties": {}
            })),
        );
        reg.insert(
            "math".to_string(),
            PFormatToolParams::from_schema(&json!({
                "type": "object",
                "properties": {
                    "x": { "type": "integer" },
                    "y": { "type": "number" },
                    "verbose": { "type": "boolean" }
                }
            })),
        );
        reg
    }

    #[test]
    fn renders_zero_arg_signature() {
        let reg = make_registry();
        assert_eq!(render_signature("ping", &reg["ping"]), "ping[]");
    }

    #[test]
    fn renders_multi_arg_signature() {
        let reg = make_registry();
        assert_eq!(
            render_signature("get_weather", &reg["get_weather"]),
            "get_weather[location|unit]"
        );
    }

    #[test]
    fn parses_simple_call() {
        let reg = make_registry();
        let (name, args) = parse_call("get_weather[London|metric]", &reg).unwrap();
        assert_eq!(name, "get_weather");
        assert_eq!(args, json!({"location": "London", "unit": "metric"}));
    }

    #[test]
    fn parses_zero_arg_call() {
        let reg = make_registry();
        let (name, args) = parse_call("ping[]", &reg).unwrap();
        assert_eq!(name, "ping");
        assert_eq!(args, json!({}));
    }

    #[test]
    fn parses_single_arg_with_spaces() {
        let reg = make_registry();
        let (name, args) = parse_call("shell[ls -la /tmp]", &reg).unwrap();
        assert_eq!(name, "shell");
        assert_eq!(args, json!({"command": "ls -la /tmp"}));
    }

    #[test]
    fn handles_pipe_escape() {
        let reg = make_registry();
        let (_, args) = parse_call(r"shell[cat foo \| grep bar]", &reg).unwrap();
        assert_eq!(args, json!({"command": "cat foo | grep bar"}));
    }

    #[test]
    fn handles_bracket_escape() {
        let reg = make_registry();
        let (_, args) = parse_call(r"shell[echo \]done\]]", &reg).unwrap();
        assert_eq!(args, json!({"command": "echo ]done]"}));
    }

    #[test]
    fn handles_backslash_escape() {
        let reg = make_registry();
        let (_, args) = parse_call(r"shell[C:\\Users\\bob]", &reg).unwrap();
        assert_eq!(args, json!({"command": r"C:\Users\bob"}));
    }

    #[test]
    fn coerces_typed_arguments() {
        let reg = make_registry();
        // Alphabetical order: verbose, x, y. The signature the model
        // sees in the catalogue is `math[verbose|x|y]` so this is the
        // order it would emit.
        let (_, args) = parse_call("math[true|42|3.14]", &reg).unwrap();
        assert_eq!(args, json!({"verbose": true, "x": 42, "y": 3.14}));
    }

    #[test]
    fn coercion_falls_back_to_string_on_failure() {
        let reg = make_registry();
        let (_, args) = parse_call("math[maybe|notanumber|alsonotanumber]", &reg).unwrap();
        assert_eq!(
            args,
            json!({
                "verbose": "maybe",
                "x": "notanumber",
                "y": "alsonotanumber"
            })
        );
    }

    #[test]
    fn signature_uses_alphabetical_order() {
        let reg = make_registry();
        // `math` has properties (in source) {x, y, verbose} but
        // BTreeMap iteration sorts to {verbose, x, y}.
        assert_eq!(
            render_signature("math", &reg["math"]),
            "math[verbose|x|y]"
        );
    }

    #[test]
    fn rejects_unknown_tool() {
        let reg = make_registry();
        assert!(parse_call("nope[arg]", &reg).is_none());
    }

    #[test]
    fn rejects_missing_brackets() {
        let reg = make_registry();
        assert!(parse_call("get_weather London metric", &reg).is_none());
    }

    #[test]
    fn rejects_trailing_garbage() {
        let reg = make_registry();
        // Closing bracket isn't last char → invalid p-format, dispatcher
        // should try the JSON fallback path.
        assert!(parse_call("get_weather[London|metric] // comment", &reg).is_none());
    }

    #[test]
    fn drops_excess_positional_arguments() {
        let reg = make_registry();
        // get_weather only has 2 schema params; the third value is dropped.
        let (_, args) = parse_call("get_weather[London|metric|extra]", &reg).unwrap();
        assert_eq!(args, json!({"location": "London", "unit": "metric"}));
    }

    #[test]
    fn empty_body_pipes_produce_empty_strings() {
        let reg = make_registry();
        let (_, args) = parse_call("get_weather[||]", &reg).unwrap();
        // 3 raw values: "", "", "". get_weather has 2 params, third is dropped.
        assert_eq!(args, json!({"location": "", "unit": ""}));
    }

    #[test]
    fn signature_round_trips_with_parser() {
        let reg = make_registry();
        let sig = render_signature("get_weather", &reg["get_weather"]);
        // Render uses the same identifier the parser expects.
        assert!(sig.starts_with("get_weather["));
        let synthesised = "get_weather[Berlin|imperial]";
        let (name, args) = parse_call(synthesised, &reg).unwrap();
        assert_eq!(name, "get_weather");
        assert_eq!(args["location"], json!("Berlin"));
        assert_eq!(args["unit"], json!("imperial"));
    }
}
